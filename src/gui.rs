//! The window (Milestone 5).
//!
//! Everything from the earlier milestones is reachable from here without
//! touching `convers.toml` or the command line: the recognizer (populated from
//! discovery, disabled entries shown with what is missing), both languages,
//! both audio devices, speech on or off, the half-duplex gate, the comparison
//! harness, and a caption pane with a latency readout underneath.
//!
//! The window never talks to a model or a device. It starts a [`Pipeline`],
//! reads [`PipelineMsg`]s (SPEC §11), and draws them. What the messages do to
//! the window's state lives in [`Session`], which has no egui in it and is
//! tested on its own.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use anyhow::{anyhow, Result};
use eframe::egui::{self, Color32, RichText};

use crate::audio::{self, InputDevice};
use crate::compare::Comparison;
use crate::config::Config;
use crate::models::{self, AsrBackend, Backend, Engine, Entry, Role};
use crate::paths;
use crate::pipeline::{self, Options, Pipeline, PipelineMsg};
use crate::translate::language_name;
use crate::tts;

/// How often the window wakes to read messages while the pipeline runs.
const REFRESH: Duration = Duration::from_millis(100);

/// Captions kept on screen. Older ones scroll off; the log keeps everything.
const MAX_LINES: usize = 200;

/// The quietest level the meter shows, in dBFS.
const METER_FLOOR_DB: f32 = -60.0;

pub fn run(root: PathBuf, config: Config) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("convers")
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "convers",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(root, config)))),
    )
    .map_err(|e| anyhow!("the window could not be opened: {e}"))
}

// ---------------------------------------------------------------------------
// What the messages do. No egui below this line until `App`.
// ---------------------------------------------------------------------------

/// Where the pipeline is in its life.
#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
    Stopped,
    /// Loading models; carries what is being loaded.
    Starting(String),
    Listening,
}

/// One recognised utterance and everything that happened to it afterwards.
#[derive(Debug, Clone, Default)]
pub struct Caption {
    pub index: usize,
    pub source: String,
    pub target: Option<String>,
    /// Why there is no translation, when there is not going to be one.
    pub problem: Option<String>,
    pub speech_ms: u64,
    pub asr_ms: u128,
    pub translate_ms: Option<u128>,
    pub first_audio_ms: Option<u128>,
}

/// A line in the caption pane.
#[derive(Debug, Clone)]
pub enum Line {
    Caption(Caption),
    Comparison(Comparison),
}

/// The window's state, as changed by pipeline messages.
#[derive(Debug)]
pub struct Session {
    pub state: RunState,
    pub speaking: bool,
    pub level_db: Option<f32>,
    pub lines: Vec<Line>,
    pub last_error: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            state: RunState::Stopped,
            speaking: false,
            level_db: None,
            lines: Vec::new(),
            last_error: None,
        }
    }
}

impl Session {
    pub fn apply(&mut self, msg: PipelineMsg) {
        match msg {
            PipelineMsg::Loading(what) => self.state = RunState::Starting(what),
            PipelineMsg::Listening => {
                self.state = RunState::Listening;
                self.last_error = None;
            }
            PipelineMsg::Level(db) => self.level_db = Some(db),
            PipelineMsg::SpeechStarted => {}
            PipelineMsg::Final {
                index,
                text,
                speech_ms,
                asr_ms,
                ..
            } => self.push(Line::Caption(Caption {
                index,
                source: text,
                speech_ms,
                asr_ms,
                ..Default::default()
            })),
            PipelineMsg::Translated {
                index,
                target,
                translate_ms,
                ..
            } => {
                if let Some(caption) = self.caption_mut(index) {
                    caption.target = Some(target);
                    caption.translate_ms = Some(translate_ms);
                }
            }
            PipelineMsg::NotTranslated { index, reason } => {
                if let Some(caption) = self.caption_mut(index) {
                    caption.problem = Some(reason);
                }
            }
            PipelineMsg::SpeakingStarted {
                index,
                first_audio_ms,
                ..
            } => {
                self.speaking = true;
                if let Some(caption) = self.caption_mut(index) {
                    caption.first_audio_ms = Some(first_audio_ms);
                }
            }
            PipelineMsg::SpeakingEnded => self.speaking = false,
            PipelineMsg::Comparison(comparison) => self.push(Line::Comparison(comparison)),
            PipelineMsg::Error(e) => self.last_error = Some(e),
            PipelineMsg::Stopped => {
                self.state = RunState::Stopped;
                self.speaking = false;
                self.level_db = None;
            }
        }
    }

    /// The most recent caption with every stage it went through, for the
    /// latency readout.
    pub fn latest_timed(&self) -> Option<&Caption> {
        self.lines.iter().rev().find_map(|line| match line {
            Line::Caption(c) if c.translate_ms.is_some() => Some(c),
            _ => None,
        })
    }

    fn push(&mut self, line: Line) {
        self.lines.push(line);
        if self.lines.len() > MAX_LINES {
            let excess = self.lines.len() - MAX_LINES;
            self.lines.drain(..excess);
        }
    }

    fn caption_mut(&mut self, index: usize) -> Option<&mut Caption> {
        self.lines.iter_mut().rev().find_map(|line| match line {
            Line::Caption(c) if c.index == index => Some(c),
            _ => None,
        })
    }
}

// ---------------------------------------------------------------------------
// The window.
// ---------------------------------------------------------------------------

struct App {
    root: PathBuf,
    config: Config,
    config_path: PathBuf,
    /// Every ASR directory with an engine.toml, including broken ones, because
    /// "why isn't my model in the list" must be answered on screen (SPEC §6).
    recognizers: Vec<Entry>,
    voices: Vec<Engine>,
    inputs: Vec<InputDevice>,
    outputs: Vec<InputDevice>,
    compare: bool,
    session: Session,
    pipeline: Option<Pipeline>,
    events: Receiver<PipelineMsg>,
    events_tx: Sender<PipelineMsg>,
    /// Problems the window itself has, as opposed to the pipeline's.
    notice: Option<String>,
}

impl App {
    fn new(root: PathBuf, config: Config) -> Self {
        let (events_tx, events) = channel();
        let mut app = Self {
            config_path: paths::config_file(&root),
            recognizers: Vec::new(),
            voices: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            compare: false,
            session: Session::default(),
            pipeline: None,
            events,
            events_tx,
            notice: None,
            root,
            config,
        };
        app.rediscover();
        app
    }

    /// Read the model tree and the device lists again. Adding a model means
    /// dropping a folder in (SPEC §6); this saves restarting to see it.
    fn rediscover(&mut self) {
        self.recognizers = models::discover(&paths::asr_dir(&self.root), Role::Asr);
        self.voices = pipeline::discovered_voices(&self.root);
        self.inputs = audio::list_input_devices().unwrap_or_default();
        self.outputs = audio::list_output_devices().unwrap_or_default();
    }

    fn running(&self) -> bool {
        self.pipeline.is_some()
    }

    fn start(&mut self) {
        self.session.last_error = None;
        self.session.state = RunState::Starting("models".to_string());
        let options = Options {
            write_wav: false,
            compare: self.compare,
        };
        match Pipeline::start(
            self.root.clone(),
            self.config.clone(),
            options,
            self.events_tx.clone(),
        ) {
            Ok(pipeline) => self.pipeline = Some(pipeline),
            Err(e) => {
                self.session.state = RunState::Stopped;
                self.session.last_error = Some(format!("{e:#}"));
            }
        }
    }

    fn stop(&mut self) {
        if let Some(pipeline) = &self.pipeline {
            pipeline.stop();
        }
    }

    /// Persist a changed selection. A failure is shown, not swallowed: the
    /// user would otherwise find their choice gone on the next start.
    fn save(&mut self) {
        self.notice = self
            .config
            .save_selections(&self.config_path)
            .err()
            .map(|e| format!("{e:#}"));
    }

    fn selected_recognizer(&self) -> Option<&Engine> {
        self.recognizers.iter().find_map(|entry| match entry {
            Entry::Loaded(e) if e.dir_name == self.config.asr.engine => Some(e),
            _ => None,
        })
    }

    /// Every language any installed recognizer or voice declares.
    fn known_languages(&self) -> Vec<String> {
        let mut languages: Vec<String> = self
            .recognizers
            .iter()
            .filter_map(|entry| match entry {
                Entry::Loaded(e) => Some(e.languages.clone()),
                Entry::Failed { .. } => None,
            })
            .flatten()
            .chain(self.voices.iter().flat_map(|v| v.languages.clone()))
            .filter(|l| l != "...")
            .collect();
        languages.sort();
        languages.dedup();
        languages
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add_space(6.0);

        let editable = !self.running();
        let mut changed = false;

        ui.add_enabled_ui(editable, |ui| {
            changed |= self.recognizer_picker(ui);
            ui.add_space(8.0);
            changed |= self.language_pickers(ui);
            ui.add_space(8.0);
            changed |= self.device_pickers(ui);
            ui.add_space(8.0);
            changed |= self.speech_options(ui);
            ui.add_space(8.0);
            ui.separator();
            ui.checkbox(&mut self.compare, "Compare recognizers")
                .on_hover_text(
                    "Run every installed recognizer on each utterance and show the transcripts \
                     side by side with their timings (SPEC §12). Translation and speech are off \
                     while comparing.",
                );
        });

        if !editable {
            ui.add_space(6.0);
            ui.weak("Stop to change settings.");
        }

        ui.add_space(12.0);
        if ui
            .add_enabled(editable, egui::Button::new("Rescan models and devices"))
            .clicked()
        {
            self.rediscover();
        }

        if changed {
            self.save();
        }
    }

    /// The recognizer list, straight from discovery. Nothing is hidden: a
    /// disabled model says what it is missing, a broken one says why.
    fn recognizer_picker(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.label("Recognizer");
        let current = self
            .selected_recognizer()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| {
                if self.config.asr.engine.is_empty() {
                    "(none selected)".to_string()
                } else {
                    format!("{} (not found)", self.config.asr.engine)
                }
            });

        egui::ComboBox::from_id_salt("recognizer")
            .width(ui.available_width())
            .selected_text(current)
            .show_ui(ui, |ui| {
                for entry in &self.recognizers {
                    match entry {
                        Entry::Loaded(engine) if engine.enabled() => {
                            let response = ui.selectable_value(
                                &mut self.config.asr.engine,
                                engine.dir_name.clone(),
                                &engine.name,
                            );
                            changed |= response.changed();
                        }
                        Entry::Loaded(engine) => {
                            ui.add_enabled(
                                false,
                                egui::Button::selectable(
                                    false,
                                    format!(
                                        "{} — missing: {}",
                                        engine.name,
                                        engine.missing_files().join(", ")
                                    ),
                                ),
                            )
                            .on_disabled_hover_text(format!(
                                "Put the missing files in {}",
                                engine.dir.display()
                            ));
                        }
                        Entry::Failed { dir_name, error } => {
                            ui.add_enabled(
                                false,
                                egui::Button::selectable(false, format!("{dir_name} — broken")),
                            )
                            .on_disabled_hover_text(error.to_string());
                        }
                    }
                }
            });
        changed
    }

    fn language_pickers(&mut self, ui: &mut egui::Ui) -> bool {
        let languages = self.known_languages();
        let mut changed = false;

        for (label, id, value) in [
            (
                "Speaker's language",
                "source",
                &mut self.config.languages.source,
            ),
            (
                "Translate into",
                "target",
                &mut self.config.languages.target,
            ),
        ] {
            ui.label(label);
            egui::ComboBox::from_id_salt(id)
                .width(ui.available_width())
                .selected_text(describe_language(value))
                .show_ui(ui, |ui| {
                    for code in &languages {
                        changed |= ui
                            .selectable_value(value, code.clone(), describe_language(code))
                            .changed();
                    }
                });
        }

        if self.config.languages.source == self.config.languages.target {
            ui.colored_label(
                Color32::from_rgb(220, 160, 40),
                "Both languages are the same.",
            );
        }
        changed
    }

    fn device_pickers(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        for (label, id, devices, value) in [
            (
                "Microphone",
                "input",
                &self.inputs,
                &mut self.config.audio.input_device,
            ),
            (
                "Output",
                "output",
                &self.outputs,
                &mut self.config.audio.output_device,
            ),
        ] {
            ui.label(label);
            let shown = if value.is_empty() {
                let default = devices
                    .iter()
                    .find(|d| d.is_default)
                    .map(|d| d.name.as_str())
                    .unwrap_or("none");
                format!("System default ({default})")
            } else {
                value.clone()
            };
            egui::ComboBox::from_id_salt(id)
                .width(ui.available_width())
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(value, String::new(), "System default")
                        .changed();
                    for device in devices {
                        changed |= ui
                            .selectable_value(value, device.name.clone(), &device.name)
                            .changed();
                    }
                });
            // A saved device that has since been unplugged is shown, not
            // silently replaced (SPEC §15).
            if !value.is_empty() && !devices.iter().any(|d| &d.name == value) {
                ui.colored_label(Color32::from_rgb(220, 90, 70), "Not connected.");
            }
        }
        changed
    }

    fn speech_options(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        changed |= ui
            .checkbox(&mut self.config.tts.enabled, "Speak translations")
            .changed();

        ui.add_enabled_ui(self.config.tts.enabled, |ui| {
            changed |= ui
                .checkbox(
                    &mut self.config.tts.half_duplex,
                    "Mute the microphone while speaking",
                )
                .on_hover_text(
                    "Half-duplex. Turn this off only when using headphones: with speakers, \
                     convers hears its own voice and transcribes it.",
                )
                .changed();

            match tts::for_language(&self.voices, &self.config.languages.target) {
                Ok(voice) => {
                    ui.weak(format!("Voice: {}", voice.name));
                }
                Err(_) => {
                    ui.colored_label(
                        Color32::from_rgb(220, 90, 70),
                        format!(
                            "No voice installed for {}.",
                            describe_language(&self.config.languages.target)
                        ),
                    );
                }
            }
        });
        changed
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("convers");
            ui.add_space(16.0);

            let (label, stopping) = match (&self.session.state, self.running()) {
                (RunState::Stopped, false) => ("Start", false),
                (_, true) => ("Stop", false),
                (_, false) => ("Stop", true),
            };
            let button =
                egui::Button::new(RichText::new(label).size(18.0)).min_size(egui::vec2(96.0, 32.0));
            if ui.add_enabled(!stopping, button).clicked() {
                if self.running() {
                    self.stop();
                } else {
                    self.start();
                }
            }

            ui.add_space(12.0);
            let status = match &self.session.state {
                RunState::Stopped => RichText::new("Stopped").weak(),
                RunState::Starting(what) => RichText::new(format!("Loading {what}…")),
                RunState::Listening if self.session.speaking => {
                    RichText::new("Speaking").color(Color32::from_rgb(90, 160, 230))
                }
                RunState::Listening => {
                    RichText::new("Listening").color(Color32::from_rgb(90, 190, 110))
                }
            };
            ui.label(status.size(16.0));

            if let Some(db) = self.session.level_db {
                ui.add_space(12.0);
                let fraction = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(fraction).desired_width(140.0).text(
                    if db.is_finite() {
                        format!("{db:.0} dBFS")
                    } else {
                        "silence".to_string()
                    },
                ))
                .on_hover_text("Microphone level. Nothing moving here means nothing is heard.");
            }
        });
    }

    fn latency_readout(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            match self.session.latest_timed() {
                Some(c) => {
                    ui.label(format!(
                        "Last utterance: {} ms of speech · recognised in {} ms · translated in \
                         {} ms",
                        c.speech_ms,
                        c.asr_ms,
                        c.translate_ms.unwrap_or_default()
                    ));
                    if let Some(first) = c.first_audio_ms {
                        ui.label(RichText::new(format!("· {first} ms to first audio")).strong());
                    }
                }
                None => {
                    ui.weak("Timings appear after the first translated utterance.");
                }
            }
            // The segment duration always travels with the timing, and Whisper
            // gets a note, because its cost barely depends on length (SPEC §10).
            if matches!(
                self.selected_recognizer().map(|e| e.backend),
                Some(Backend::Asr(AsrBackend::Whisper))
            ) {
                ui.weak("· Whisper pads every utterance to 30 s, so short ones cost nearly as much as long ones.");
            }
        });

        for problem in [&self.session.last_error, &self.notice]
            .into_iter()
            .flatten()
        {
            ui.colored_label(Color32::from_rgb(220, 90, 70), problem);
        }
    }

    fn captions(&self, ui: &mut egui::Ui) {
        if self.session.lines.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.weak(match self.session.state {
                    RunState::Listening => "Listening. Speak, and captions appear here.",
                    _ => "Press Start, then speak.",
                });
            });
            return;
        }

        let source = self.config.languages.source.clone();
        let target = self.config.languages.target.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.session.lines {
                    match line {
                        Line::Caption(c) => caption_card(ui, c, &source, &target),
                        Line::Comparison(c) => comparison_card(ui, c, &self.config.asr.engine),
                    }
                    ui.add_space(6.0);
                }
            });
    }
}

impl eframe::App for App {
    /// Read the pipeline's messages. No drawing happens here.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.events.try_recv() {
            let stopped = matches!(msg, PipelineMsg::Stopped);
            self.session.apply(msg);
            if stopped {
                // The thread has finished, so dropping the handle joins at
                // once and the window never waits on it.
                self.pipeline = None;
            }
        }
        if self.running() {
            ctx.request_repaint_after(REFRESH);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("bottom").show(ui, |ui| {
            ui.add_space(4.0);
            self.latency_readout(ui);
            ui.add_space(4.0);
        });
        egui::Panel::left("settings")
            .resizable(true)
            .default_size(300.0)
            .min_size(240.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| self.captions(ui));
    }
}

fn describe_language(code: &str) -> String {
    let name = language_name(code);
    if name == code {
        code.to_string()
    } else {
        format!("{name} ({code})")
    }
}

/// One utterance: the translation large, the original beneath it, and the
/// timings small. The translation is what the listener came for.
fn caption_card(ui: &mut egui::Ui, c: &Caption, source: &str, target: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match (&c.target, &c.problem) {
            (Some(text), _) => {
                ui.label(RichText::new(text).size(22.0).strong());
            }
            (None, Some(problem)) => {
                ui.label(
                    RichText::new(format!("Not translated: {problem}"))
                        .color(Color32::from_rgb(220, 150, 60)),
                );
            }
            (None, None) => {
                ui.label(RichText::new("…").size(22.0).weak());
            }
        }
        ui.label(RichText::new(&c.source).size(16.0).italics());
        let mut timing = format!(
            "#{} · {source} → {target} · {} ms of speech · recognised {} ms",
            c.index, c.speech_ms, c.asr_ms
        );
        if let Some(ms) = c.translate_ms {
            timing.push_str(&format!(" · translated {ms} ms"));
        }
        if let Some(ms) = c.first_audio_ms {
            timing.push_str(&format!(" · first audio {ms} ms"));
        }
        ui.small(timing);
    });
}

/// Every recognizer's transcript of one utterance, side by side (SPEC §12),
/// always with the length of the audio beside the timing.
fn comparison_card(ui: &mut egui::Ui, c: &Comparison, selected: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(format!(
                "Utterance #{} — {} ms of audio",
                c.utterance_index, c.segment_ms
            ))
            .strong(),
        );
        egui::Grid::new(("comparison", c.utterance_index))
            .num_columns(3)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for run in &c.runs {
                    let name = if run.engine == selected {
                        format!("{} (selected)", run.engine)
                    } else {
                        run.engine.clone()
                    };
                    ui.monospace(name);
                    ui.monospace(format!("{} ms", run.elapsed_ms));
                    if run.ok {
                        ui.label(if run.text.is_empty() {
                            "(no text)"
                        } else {
                            &run.text
                        });
                    } else {
                        ui.colored_label(Color32::from_rgb(220, 90, 70), &run.text);
                    }
                    ui.end_row();
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::Run;

    fn final_msg(index: usize, text: &str) -> PipelineMsg {
        PipelineMsg::Final {
            index,
            text: text.to_string(),
            speech_ms: 1500,
            asr_ms: 120,
        }
    }

    #[test]
    fn an_utterance_collects_its_translation_and_timings() {
        let mut s = Session::default();
        s.apply(PipelineMsg::Listening);
        s.apply(final_msg(1, "¿Dónde está la estación?"));
        s.apply(PipelineMsg::Translated {
            index: 1,
            target: "Where is the station?".to_string(),
            translate_ms: 340,
        });
        s.apply(PipelineMsg::SpeakingStarted {
            index: 1,
            first_audio_ms: 650,
        });

        let c = s.latest_timed().expect("a timed caption");
        assert_eq!(c.target.as_deref(), Some("Where is the station?"));
        assert_eq!(c.translate_ms, Some(340));
        assert_eq!(c.first_audio_ms, Some(650));
        assert!(s.speaking);

        s.apply(PipelineMsg::SpeakingEnded);
        assert!(!s.speaking);
    }

    #[test]
    fn a_refused_translation_is_shown_on_its_own_caption() {
        let mut s = Session::default();
        s.apply(final_msg(1, "Hola."));
        s.apply(final_msg(2, "Me scables."));
        s.apply(PipelineMsg::NotTranslated {
            index: 2,
            reason: "the model recited its own instructions".to_string(),
        });

        let Line::Caption(first) = &s.lines[0] else {
            panic!("a caption")
        };
        let Line::Caption(second) = &s.lines[1] else {
            panic!("a caption")
        };
        assert!(first.problem.is_none(), "the wrong caption was marked");
        assert!(second.problem.as_deref().unwrap_or("").contains("recited"));
        assert!(s.latest_timed().is_none(), "nothing was translated");
    }

    #[test]
    fn loading_listening_and_stopping_move_the_state() {
        let mut s = Session::default();
        s.apply(PipelineMsg::Loading("the translation model".to_string()));
        assert_eq!(
            s.state,
            RunState::Starting("the translation model".to_string())
        );
        s.apply(PipelineMsg::Listening);
        assert_eq!(s.state, RunState::Listening);
        s.apply(PipelineMsg::Level(-20.0));
        s.apply(PipelineMsg::SpeakingStarted {
            index: 9,
            first_audio_ms: 1,
        });
        s.apply(PipelineMsg::Stopped);
        assert_eq!(s.state, RunState::Stopped);
        assert!(!s.speaking, "stopping ends speech");
        assert!(s.level_db.is_none(), "no meter once stopped");
    }

    #[test]
    fn an_error_is_kept_until_the_next_start_listens() {
        let mut s = Session::default();
        s.apply(PipelineMsg::Error(
            "no installed voice speaks \"ja\"".to_string(),
        ));
        s.apply(PipelineMsg::Stopped);
        assert!(s.last_error.is_some(), "a failed start must stay on screen");
        s.apply(PipelineMsg::Listening);
        assert!(s.last_error.is_none());
    }

    #[test]
    fn comparisons_become_their_own_lines() {
        let mut s = Session::default();
        s.apply(PipelineMsg::Comparison(Comparison {
            utterance_index: 1,
            start_ms: 0,
            segment_ms: 1000,
            runs: vec![Run {
                engine: "parakeet".to_string(),
                text: "hola".to_string(),
                elapsed_ms: 80,
                ok: true,
            }],
        }));
        assert!(matches!(s.lines[0], Line::Comparison(_)));
    }

    #[test]
    fn the_pane_keeps_a_bounded_history() {
        let mut s = Session::default();
        for i in 0..(MAX_LINES + 25) {
            s.apply(final_msg(i, "hola"));
        }
        assert_eq!(s.lines.len(), MAX_LINES);
        let Line::Caption(oldest) = &s.lines[0] else {
            panic!("a caption")
        };
        assert_eq!(oldest.index, 25, "the oldest lines go first");
    }
}
