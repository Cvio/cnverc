# SPEC.md — `cnverc`

**Read this entire file before writing any code.** This is a build specification, not a
suggestion list. The constraints in §2 are the reason the project exists; violating one of
them makes the deliverable worthless even if it compiles and runs.

---

## 1. What we are building

`cnverc` is a desktop application that listens to speech in one language and produces text
and spoken audio in another, entirely offline, running as a single self-contained folder that
can be copied to a machine with no internet connection, no Node, no Python, no Ollama, and no
toolchain of any kind, and will run.

Pipeline: **microphone → VAD → ASR → translation → TTS → speakers**, with on-screen captions
alongside.

Two operating modes (§8) and an optional paired mode (§9) in which two instances of `cnverc`
on two machines on a local network act as the two ends of a conversation.

Primary language pair for development and testing: **Spanish ⇄ English**. The design must not
hardcode that pair, but it also should not be generalized past what this spec requires.

### Prior art in this repo family

There is an existing project at `D:\AI_Data\projects\live-captions` (Rust/Tauri) with a
`pipeline.rs` containing four message types and a working Spanish-to-English captioning path.
**Read it for the message-passing shape before designing ours.** Do not copy its Tauri
scaffolding or its model loading.

There is also `D:\AI_Data\projects\sokuji` (Electron/React). It is the thing we are reacting
against. Its model storage is content-addressed in `AppData/Roaming` with the human-readable
names held only in IndexedDB, which makes the models impossible to identify, curate, or move
by hand. Do not replicate any part of that approach.

---

## 2. Hard constraints

These are non-negotiable. If a design choice conflicts with one of these, the design choice
loses. If you believe one of these is wrong, stop and say so rather than working around it.

1. **The application must never require, attempt, or depend on internet access.** No model
   downloads, no telemetry, no update checks, no license checks, no cloud inference, no
   public DNS lookups, no CDN fetches, no crash reporting. If a required model file is absent,
   `cnverc` prints the exact absolute path it expected and exits non-zero. It does not offer
   to fetch it.

   **Local network sockets are explicitly permitted and required** for paired mode (§9):
   TCP and UDP to a peer address the user has entered or that was discovered on the local
   link. This is not an exception to the rule above — it introduces no internet dependency,
   and paired mode must work on a network with no gateway and no upstream at all. What is
   forbidden is *reaching the internet*, not *opening a socket*.

   Practical test for any network code added to this project: **would it still work with the
   WAN cable unplugged and no DNS server anywhere on the segment?** If yes, it is allowed. If
   no, it does not belong here. This rules out any hosted signalling, STUN/TURN, relay
   service, or hostname that must resolve through a public resolver.

2. **No separate services.** No Ollama, no `localhost:11434`, no sidecar process, no Docker,
   no external inference server. Every model runs in-process. (The peer listener in §9 is part
   of `cnverc` itself, not a separate service.)

3. **No JavaScript toolchain.** No `package.json`, no `node_modules`, no npm, no bundler,
   anywhere in the repository. The GUI is native Rust (§4).

4. **No OS-managed application directories.** Never write to or read models from `%APPDATA%`,
   `%LOCALAPPDATA%`, `~/.cache`, or anything returned by the `dirs`/`directories` crates. All
   model and configuration paths resolve relative to the executable's own location.

5. **No content-addressed or opaque storage.** Every model file on disk keeps the name it was
   published with, in a directory named after the model. A human opening the folder in
   Explorer must be able to tell what each file is without running anything.

6. **Copy-to-run portability.** The success criterion for the whole project: zip the output
   folder, move it to a freshly imaged Windows machine with no internet, unzip, double-click,
   and it works. This is verified in Milestone 9 and it is the acceptance test.

---

## 3. Target environment

- Windows 10/11 x64, primary. Developer shell is Git Bash.
- NVIDIA RTX 4070 Laptop GPU, **8GB VRAM**. 32GB system RAM.
- Model selection must fit comfortably in 8GB alongside a desktop session. Assume all four
  models are resident simultaneously.
- Linux support is a nice-to-have; do not let it complicate the Windows path.

Known build hazard from previous work in this family: on Windows, Vulkan-backed whisper builds
have caused failures. If a Vulkan feature flag appears in a dependency, disable it for the
Windows target rather than debugging it.

---

## 4. Stack — decided, not open

| Concern | Choice | Notes |
|---|---|---|
| Language | Rust (stable, 2021 edition) | Single static binary is the whole point |
| VAD, ASR, TTS | `sherpa-onnx` crate | One dependency covers all three |
| Translation | `llama-cpp-2` | GGUF file loaded in-process |
| GUI | `eframe` / `egui` | Immediate-mode, compiled in, no webview |
| Audio I/O | `cpal` | Capture and playback |
| Peer transport | `std::net` / `tokio` TCP | Plain sockets, no framework |
| Peer discovery | `mdns-sd` *or* raw UDP broadcast | Convenience only — never the sole path in |
| Config | `toml` + `serde` | One file, §7 |
| Errors | `anyhow` in app code, `thiserror` at library boundaries | |
| Logging | `tracing` + `tracing-subscriber` | stdout and `./logs/` |

### On sherpa-onnx specifically

`sherpa-onnx` wraps the sherpa-onnx C API with RAII Rust types. It provides
`VoiceActivityDetector` (Silero), `OfflineRecognizer` (whole-utterance ASR — both Whisper and
Parakeet live here), `OnlineRecognizer` (true streaming ASR), and `OfflineTts`. It **links
statically by default**, which is why we chose it: no `onnxruntime.dll` to ship beside the
binary.

**Do not guess at its API.** Before writing binding code, read `https://docs.rs/sherpa-onnx`
and the upstream `rust-api-examples/examples/` directory. Directly relevant examples:
`nemo_parakeet.rs`, `streaming_zipformer.rs`, `silero_vad_remove_silence.rs`, `pocket_tts.rs`.
Mirror their configuration struct usage rather than inventing it.

**Build-time internet caveat.** The `sherpa-onnx` build script downloads a matching prebuilt
`-lib` archive from GitHub releases unless `SHERPA_ONNX_LIB_DIR` is set. The *first build on a
new dev machine therefore needs internet*, even though the resulting binary does not. Document
this in the README, and document setting `SHERPA_ONNX_LIB_DIR` for offline rebuilds.

### On egui rather than Tauri

The UI is a handful of selectors, a start/stop control, a caption pane, a latency readout, and
a pairing panel. That does not justify a webview, a JS build step, or a WebView2 runtime
dependency on the target machine. Use `egui`. Do not propose switching to Tauri, Iced, Slint,
or a web UI.

---

## 5. Directory layout

```
cnverc/
  cnverc.exe
  cnverc.toml
  models/
    vad/
      silero_vad.onnx
    asr/
      parakeet-tdt-0.6b-v3-int8/
        engine.toml
        encoder.int8.onnx
        decoder.int8.onnx
        joiner.int8.onnx
        tokens.txt
      whisper-large-v3-turbo/
        engine.toml
        <whisper onnx files>
        tokens.txt
    mt/
      qwen3-0.6b-q4_k_m.gguf
    tts/
      <tts model dir>/
        engine.toml
        <model files>
  logs/
```

### Path resolution rule

All of the above resolve from `std::env::current_exe()`, canonicalized, parent directory.
**Never from the current working directory** — double-clicking from Explorer and launching
from a shell must behave identically. Exactly one such function exists, and no other path
derivation appears anywhere in the codebase:

```rust
fn app_root() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    Ok(exe.parent().context("exe has no parent")?.to_path_buf())
}
```

---

## 6. Model discovery — the filesystem is the index

Each ASR and TTS model directory contains an `engine.toml` describing itself. `cnverc`
enumerates subdirectories of `models/asr/` at startup, parses each `engine.toml`, and
populates the selector from the results. Adding a model means dropping a folder in and
restarting. There is no registry, no database, no cache, and no download UI.

`models/asr/parakeet-tdt-0.6b-v3-int8/engine.toml`:

```toml
name = "Parakeet TDT 0.6B v3 (int8)"
kind = "segment"                 # "segment" | "stream"
backend = "nemo_transducer"      # maps to a sherpa-onnx model config variant
languages = ["es", "en", "de", "fr", "it", "..."]

[files]
encoder = "encoder.int8.onnx"
decoder = "decoder.int8.onnx"
joiner  = "joiner.int8.onnx"
tokens  = "tokens.txt"
```

`models/asr/whisper-large-v3-turbo/engine.toml`:

```toml
name = "Whisper large-v3-turbo"
kind = "segment"
backend = "whisper"
languages = ["es", "en", "..."]

[files]
encoder = "..."
decoder = "..."
tokens  = "tokens.txt"
```

Validation rules:

- A directory missing `engine.toml` is skipped, with a warning naming the directory.
- A directory whose `engine.toml` references a file that is not present is listed in the UI but
  **disabled**, with the missing filename shown. Do not silently hide it — the user's most
  likely question is "why isn't my model in the list", and the answer must be on screen.
- Unknown `backend` values are a hard error for that entry at load time, not a panic.

---

## 7. Configuration

`cnverc.toml` at the app root holds only user selections, by directory name.

```toml
[asr]
engine = "parakeet-tdt-0.6b-v3-int8"

[languages]
source = "es"
target = "en"

[mode]
kind = "turn"            # "continuous" | "turn"
turn_key = "Space"
turn_style = "toggle"    # "toggle" | "hold"

[audio]
input_device = ""        # empty = system default
output_device = ""

[vad]
threshold = 0.5
min_silence_ms = 500
min_speech_ms = 250

[tts]
enabled = true
half_duplex = true       # §10

[peer]
enabled = false
listen_addr = "0.0.0.0:47800"
peer_addr = ""           # e.g. "192.168.50.2:47800"
display_name = ""        # empty = hostname
discovery = true         # mDNS/broadcast convenience; manual entry always available

[shared]                 # M7.5: two people, one machine, a key each
left_language  = "en"
right_language = "es"
left_voice     = ""      # speaks what the LEFT person said: a voice in the RIGHT
right_voice    = ""      # person's language; folder under models/tts/, "" = first
left_key       = "ArrowLeft"
right_key      = "ArrowRight"
```

Do not add configuration keys beyond what a milestone actually requires.

---

## 8. Operating modes

Mode is a first-class runtime setting, changeable from the UI without restart.

### Continuous mode

VAD runs freely. Every detected speech segment is transcribed, translated, and (if enabled)
spoken. This is the hands-free behaviour. Both ASR engine kinds (§11) work here.

### Turn-based mode

The microphone is closed until the user takes a turn. Taking a turn opens capture; ending the
turn closes it and flushes whatever was captured as a single utterance.

- **Key:** default `Space`, configurable via `mode.turn_key`.
- **`turn_style = "toggle"`** (default): press to start the turn, press again to end it. This
  is what you want for anything longer than a sentence.
- **`turn_style = "hold"`**: capture only while the key is held. Better for short exchanges.
- The key must work while the `cnverc` window has focus. Do **not** install a global/system-wide
  hotkey; it is a permissions and antivirus problem on Windows and is out of scope.
- Spacebar must not also activate whatever egui widget currently has keyboard focus. Handle the
  turn key before widget input and consume the event.
- Visual state is mandatory and unambiguous: an idle indicator, a recording indicator, and a
  processing indicator, each distinguishable at a glance from across a desk.

In turn-based mode, VAD is used only for trimming leading and trailing silence from the captured
turn, not for deciding utterance boundaries. The user decided the boundary — that is the point of
the mode.

---

## 9. Paired mode — two machines, one conversation

Two instances of `cnverc` on a local network act as the two ends of a conversation. Person A
speaks Spanish into laptop A; laptop B displays and speaks the English.

### What crosses the wire

**Text only. Never audio, never models, never PCM.**

Each machine runs its own complete pipeline. Laptop A captures Spanish, runs its own VAD, ASR
and translation, and sends the resulting **English text** to laptop B. Laptop B displays it and
speaks it with laptop B's own TTS model and voice.

Consequences that make this the right design, and which must not be traded away:

- An utterance costs a few hundred bytes. The link can be terrible and it still works.
- The two machines may run entirely different ASR and TTS models, at different quality levels,
  suited to their own hardware.
- There is no shared state to synchronize, no clock to align, and no stream to keep alive.
- A dropped connection loses the current utterance and nothing else.

Do not implement audio streaming between peers. Do not implement a client/server split where one
machine does the inference for both.

### Wire protocol

Newline-delimited JSON over TCP. Chosen so it can be inspected and faked with `netcat` during
development.

```jsonc
{"t":"Hello","name":"laptop-a","speaks":"es","sends":"en","proto":1}
{"t":"Utterance","seq":17,"lang":"en","text":"Where is the station?",
 "source_lang":"es","source_text":"¿Dónde está la estación?"}
{"t":"FloorRequest","seq":18}
{"t":"FloorGrant","seq":18}
{"t":"FloorRelease","seq":18}
{"t":"Ping"} {"t":"Pong"}
{"t":"Bye"}
```

- `lang` on `Utterance` tells the receiver which TTS voice to use. Do not infer it.
- `source_text` is carried so the receiving side can display the original alongside the
  translation. It is display-only; never re-translate it.
- Reject any message with an unknown `proto`, and say so on screen rather than failing silently.
- Treat every inbound field as untrusted: bound the text length, reject non-UTF-8, and never
  interpret received text as a command, a path, or a prompt instruction.

### Connection model

Each instance listens on `peer.listen_addr`. To pair, the user enters the other machine's
address in `peer.peer_addr` and connects. Once established the link is symmetric — there is no
client and no server role beyond who dialled. Exactly one peer connection at a time; reject a
second inbound connection with a clear message rather than queueing it.

### The floor token

In **turn-based paired** mode, exactly one peer holds the floor at any moment, and **the
microphone is live only for the peer holding the floor.**

- Pressing the turn key sends `FloorRequest`. Capture does not begin until `FloorGrant` arrives.
- Ending the turn sends `FloorRelease` after the utterance has been sent.
- If both request simultaneously, the peer whose `name` sorts lexicographically first wins;
  the loser's UI shows that the other party has the floor. Deterministic, no negotiation.
- If a `FloorGrant` does not arrive within 2 seconds, fail the turn visibly. Never open the
  microphone on a timeout — that is how both ends end up talking at once.
- The floor is force-released if the connection drops.

This is not only turn-taking etiquette. It closes the acoustic feedback path (§10) for free:
if your microphone is only live while you hold the floor, and the remote machine only speaks
when it holds the floor, neither machine can ever hear the other's speakers.

### Continuous mode while paired

Allowed, but both microphones are live and both sets of speakers are playing, so there is no
floor token gating anything. In a shared room the two machines will transcribe each other's TTS
output in a loop.

`cnverc` must **display a persistent warning** when continuous mode and paired mode are both
active, stating that headsets are required. Do not silently degrade, and do not try to solve
this with echo cancellation.

### Discovery

mDNS (service type `_cnverc._tcp.local`) or a UDP broadcast on port 47801 may be used to find
peers on the local link and populate a pick-list. This is **convenience only**. Manual IP entry
must always be available and must always work, because managed switches and VLAN configurations
drop multicast and the user must never be locked out of the feature by network policy.

---

## 10. Three problems that will bite

### Acoustic feedback (single machine)

If TTS output plays through speakers while the microphone is live, `cnverc` hears its own
translated speech, the VAD triggers on it, and the ASR transcribes it. This does not appear in a
captions-only build, so it arrives as a surprise the moment TTS is switched on.

Implement **half-duplex gating** as the default: the capture path discards audio, and the VAD is
held reset, from `SpeakingStarted` until `SpeakingEnded` plus a 150ms tail. Expose it as
`tts.half_duplex`. Setting it false is for headphone use only; say so in the tooltip. Do not
attempt acoustic echo cancellation.

### Whisper's fixed window

Whisper pads every input to a 30-second window internally regardless of how short the utterance
is. A 1.2-second segment therefore costs roughly what a 20-second one does. This will make the
Milestone 2 timing comparison look bizarre until it is accounted for. Report timings with the
segment duration alongside, and note it in the latency readout.

### Windows firewall on an unidentified network (paired mode)

A direct cable or an uplink-less switch produces a network Windows cannot identify, and Windows
frequently classifies it as **Public**, which blocks inbound connections by default. The
listener binds successfully, the peer connects to nothing, and no useful error surfaces. This
will consume an hour and look like a bug in our code.

Mitigate in the app, not only in the README: if the listener has bound but has never accepted a
connection while a peer connection is being attempted, show a specific diagnostic naming the
Windows firewall and the network profile, not a generic timeout.

---

## 11. Architecture

### The ASR abstraction is two-shaped, deliberately

Whisper and Parakeet TDT are both whole-utterance recognizers: a segment goes in complete, text
comes out. Streaming recognizers (streaming Zipformer, streaming FastConformer) are fed
continuously and polled for a revisable partial hypothesis. These are genuinely different
interaction shapes, and collapsing them into one `transcribe()` call would wall us off from the
low-latency path in Milestone 8.

```rust
pub enum AsrEngine {
    Segment(Box<dyn SegmentAsr + Send>),
    Stream(Box<dyn StreamAsr + Send>),
}

pub trait SegmentAsr {
    /// `pcm` is a complete utterance, 16 kHz mono f32 in [-1.0, 1.0].
    fn transcribe(&mut self, pcm: &[f32], language: &str) -> anyhow::Result<String>;
}

pub trait StreamAsr {
    fn feed(&mut self, pcm: &[f32]) -> anyhow::Result<()>;
    /// Current best guess. May be revised on subsequent calls.
    fn partial(&self) -> &str;
    /// True when the recognizer considers the utterance complete.
    fn endpoint(&mut self) -> anyhow::Result<bool>;
    fn reset(&mut self) -> anyhow::Result<()>;
}
```

Implement `SegmentAsr` for both Whisper and Parakeet first. `StreamAsr` has no implementation
before Milestone 8, but the enum and both traits exist from Milestone 2 so downstream stages are
written against the right shape from the start.

### Pipeline messages

Stages communicate over channels. Downstream stages must not know which engine kind is upstream,
nor whether an utterance originated locally or arrived from a peer.

```rust
pub enum PipelineMsg {
    TurnStarted { at: Instant },
    SpeechStarted { at: Instant },
    Partial { text: String, at: Instant },          // stream engines only
    Final { text: String, pcm: Arc<Vec<f32>>, at: Instant },
    Translated { source: String, target: String, at: Instant },
    Remote { source: String, target: String, lang: String, from: String },
    SpeakingStarted,
    SpeakingEnded,
    FloorChanged { holder: Option<String> },
    PeerState(PeerState),
    Error(String),
}
```

A segment engine emits `SpeechStarted` then exactly one `Final`. A stream engine emits
`SpeechStarted`, then zero or more `Partial`, then one `Final`. The `pcm` on `Final` is the raw
audio of the utterance, retained for the dual-run harness (§12) behind an `Arc`, dropped when
the ring buffer evicts it.

### Threading

- Audio capture thread (cpal callback) → bounded channel of PCM chunks. Never block here.
- Pipeline thread owns VAD and ASR.
- Translation and TTS on a separate thread, so a slow LLM token stream cannot stall recognition.
- Peer I/O on its own thread. A stalled or dead peer must never block local capture, local
  recognition, or the UI.
- GUI thread owns egui and receives `PipelineMsg` only.

Resample to 16 kHz mono at the capture boundary, once. Every stage downstream assumes 16 kHz
mono f32.

---

## 12. Dual-run comparison harness

Keep the last N (default 20) utterances' raw PCM in a ring buffer keyed by timestamp. Provide a
debug mode — CLI flag `--compare`, and a toggle in the UI — that runs a captured utterance
through every enabled `segment`-kind engine and displays, side by side: the transcript, wall
clock milliseconds, and segment duration.

This is not a nice-to-have. Parakeet's multilingual variant is newer than Whisper large-v3 and
leaderboard word error rates are measured on read speech, not on a specific person's microphone
and accent. The harness is how we get real numbers for this use case.

---

## 13. Build order

Each milestone is independently runnable and independently verifiable. **Do not start the next
one until the stated check passes.** Commit at each boundary.

**M0 — Skeleton and model discovery.** Cargo project, `app_root()`, config load, model directory
enumeration and `engine.toml` parsing. No audio, no models loaded.
*Check:* the binary prints a table of every discovered ASR and TTS model, its kind, its backend,
and whether all declared files are present. Moving the binary and `models/` to another drive
changes nothing.

**M1 — Capture and VAD.** cpal input, device enumeration, resample to 16 kHz mono, Silero VAD via
sherpa-onnx.
*Check:* speaking logs segment start and end with durations matching reality; silence produces
nothing; segment PCM written to a `.wav` plays back as a clean utterance.

**M2 — Segment ASR, both engines, and the harness.** `SegmentAsr` for Parakeet and Whisper,
driven by M1 segments. Engine selection from config. Ring buffer and `--compare`.
*Check:* Spanish speech produces Spanish transcripts from both engines; `--compare` prints both
transcripts with timings for the same audio.

**M3 — Translation.** `llama-cpp-2` loading the GGUF, behind a `Translator` trait. Prompt
constrained to translation only — the model must not answer the sentence. Strip any reasoning or
preamble from the output.
*Check:* Spanish in, English out, on the console, end to end from the microphone.

**M4 — TTS and half-duplex.** `OfflineTts`, playback via cpal, `SpeakingStarted`/`Ended`, the
half-duplex gate.
*Check:* speak Spanish with speakers on at normal volume; the system does not transcribe its own
output. Measure and log time-to-first-audio.

**M5 — GUI.** egui: engine selector populated from discovery, language selectors, device pickers,
start/stop, scrolling caption pane showing source and target, latency readout, compare toggle.
*Check:* everything from M0–M4 is reachable without touching the config file or the command line.

**M6 — Modes and the turn key.** Continuous/turn-based selector, spacebar handling with both
toggle and hold styles, the three-state visual indicator, VAD demoted to silence-trimming in
turn mode.
*Check:* in turn mode the microphone is provably closed between turns (speak while idle, nothing
happens); a toggled turn captures a full multi-sentence utterance; spacebar never activates a
focused widget.

**M7 — Paired mode.** Listener, dialler, `Hello` handshake, `Utterance` exchange, floor token,
peer panel in the UI showing local interface addresses and their IPs, the firewall diagnostic,
the continuous-mode headset warning. Discovery last, and only after manual entry works.
*Check:* two laptops on an isolated switch with no internet hold a turn-based Spanish/English
conversation; unplugging the WAN uplink changes nothing; pulling the cable mid-session
force-releases the floor and shows a disconnected state on both ends.

**M7.5 — Shared-machine mode.** Added after M7 and built before M8; the numbering of M8 and
M9 is unchanged. Two people who speak different languages use **one** machine, each with a
key (the arrows by default, `[shared]` above). The key says who is talking and so which
language they speak: that language goes to the recognizer explicitly, and nothing is detected,
so Shared mode requires Whisper (Parakeet decides the language itself). One person at a time:
while one side records, the other key does nothing, and while a translation is worked on or
spoken, neither does, and the microphone is closed. Escape cancels a turn, or what it is
producing, and nothing from it is spoken. Each side has its own language and its own voice,
chosen by folder name. The window shows a column per person with an unmistakable highlight on
the active one. Shared mode and paired mode exclude each other. Specified in full in
`shared-machine-mode.md`.
*Check:* by hand, with speakers at normal volume: English spoken on the left is heard in
Spanish, Spanish on the right in English, never in the wrong language, and the log records the
language used for every turn; the other key does nothing during a turn; Escape during a turn
means nothing is spoken; cnverc never translates its own voice; from across a table you can
tell whose turn it is; turn-based, continuous and paired mode behave as before.

**M8 — Streaming ASR.** `StreamAsr` against `OnlineRecognizer` with a streaming model. `Partial`
messages flowing to the caption pane. Commit policy: hold a prefix until it is unchanged across N
successive updates before passing it downstream (LocalAgreement).
*Check:* captions appear while the speaker is still talking; measured end-to-end latency is
materially below the M2 segment path on the same utterances.

**M9 — Portability acceptance.** Produce the release folder. Zip it.
*Check:* on a machine with no internet, no Rust, no Node, and no prior installation, unzip and
run. Everything works, paired mode included, over a direct cable to a second such machine.
**This is the acceptance test for the project.** If it fails, the project has failed regardless
of the state of M0–M8.

---

## 14. Conventions

- `cargo fmt` and `cargo clippy -- -D warnings` clean at every commit.
- No `unwrap()` or `expect()` in any code path that runs after startup. Startup-time `expect()`
  with a message naming the missing file is fine, and preferred to a silent default.
- Every error message concerning a file names the **absolute path** that was tried.
- `README.md` lists each required model, the exact URL to obtain it, and where to extract it, plus
  the networking setup notes in Appendix A. The app itself never uses those URLs.
- `CLAUDE.md` at the repo root restates §2 in full, so the constraints survive into future sessions.

---

## 15. Do not

- Do not add a model download button, a "get models" helper, or a first-run wizard that fetches
  anything.
- Do not add Ollama, an OpenAI-compatible client, or any HTTP client — including to localhost.
- Do not add STUN, TURN, a relay, a rendezvous server, a hosted signalling service, or any peer
  discovery that depends on a name resolving through a public DNS resolver.
- Do not stream audio between peers, and do not build a client/server split where one machine
  infers for both.
- Do not introduce a JS toolchain or a webview.
- Do not store anything in `%APPDATA%` or use the `dirs` crate.
- Do not add a plugin system, a scripting layer, or a generalized "provider" abstraction beyond
  the two ASR traits in §11.
- Do not register a global system hotkey.
- Do not add settings that no milestone needs.
- Do not silently substitute a different model when the configured one is missing.
- Do not restructure the milestones or work ahead. If a milestone seems to require something from
  a later one, stop and raise it.

---

## Appendix A — Physical transports for paired mode

All of these present to the OS as an IP interface. **The socket code is identical across every
one of them** and must not special-case any transport.

| Transport | Works | Notes |
|---|---|---|
| Ethernet switch, unmanaged | Yes | No uplink needed |
| Router with WAN unplugged | Yes | Gives you DHCP, which is convenient |
| Ethernet cable laptop-to-laptop | Yes | No crossover cable needed; gigabit auto-negotiates |
| Wi-Fi hotspot from one laptop | Yes | No upstream required |
| Existing Wi-Fi LAN | Yes | Client isolation on guest networks will block it |
| Thunderbolt / USB4 networking | Yes | Creates a virtual Ethernet adapter; fastest option |
| USB "bridge"/"transfer" cable | Yes | Has a chipset presenting as a NIC to each side |
| Two USB-C-to-Ethernet dongles | Yes | Ordinary cable between them |
| **Plain USB-C cable between two laptops** | **No** | Both ends are USB hosts. No network, not even a broken one |

### Addressing

A dumb switch or a direct cable means no DHCP server, so Windows falls back to link-local
addressing (169.254.x.x) after roughly thirty seconds of negotiation. It works, but the addresses
are ugly and may change between sessions. For a repeatedly-used rig, set static IPs on that
interface once — 192.168.50.1 and 192.168.50.2 — and never think about it again.

The pairing panel must **list every local interface with its current IP address**, so the user can
read one off laptop A and type it into laptop B without hunting through `ipconfig`.

### Firewall

See §10. Set the interface's network profile to Private. The app must produce a specific
diagnostic for the bound-but-never-accepted state rather than a generic timeout.
