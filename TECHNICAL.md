# cnverc: technical notes

This is the maintainer's companion to [README.md](README.md). The README explains what cnverc
is and how to set it up. This file explains how it's built and why. The build specification
is [SPEC.md](SPEC.md); the constraints every change must respect are restated in
[CLAUDE.md](CLAUDE.md).

## Status

Milestones 0–6 of 9 are complete. Milestones are defined in `SPEC.md` §13 and built in order;
each one's check must pass before the next starts.

| Milestone | What it added |
|---|---|
| M0 | Skeleton, path resolution, config load, model discovery, `--report` |
| M1 | cpal capture, resampling to 16 kHz mono at the capture boundary, Silero VAD cutting utterances |
| M2 | Segment ASR for Parakeet and Whisper, the utterance ring buffer, `--compare` |
| M3 | In-process translation through llama.cpp, on its own thread |
| M4 | Synthesis through sherpa-onnx, playback through cpal, the half-duplex gate |
| M5 | The egui window; the pipeline reports only through `PipelineMsg` |
| M6 | Continuous and turn-based modes, switchable while running; the turn key in toggle and hold styles |
| M7 | Paired mode (next) |

## Architecture

```
mic ─► capture ─► VAD ─► ASR ─► translate ─► TTS ─► playback ─► speakers
       (cpal)   (Silero)  │     (llama.cpp)  (Piper)   (cpal)
                          └──────► PipelineMsg ──────► window / terminal
```

Four threads, connected by bounded channels:

- **Capture** runs the cpal input callback. It resamples to 16 kHz mono once, at the capture
  boundary. Nothing downstream resamples again.
- **Pipeline** (`pipeline.rs`) runs the VAD and ASR, and takes `PipelineCmd`s (mode changes,
  turn start and end).
- **Translate** runs MT and TTS, so a slow token stream can't stall recognition.
- **Playback** runs the cpal output and the half-duplex `Gate`.

The front ends, `gui.rs` (the window) and `listen.rs` (`--listen`), receive `PipelineMsg`
only. What those messages do to the window lives in `gui::Session`, which contains no egui
and is unit-tested directly.

Details worth knowing:

- **VAD pre-roll is 600 ms.** Shorter pre-roll lost first words in live tests. Segment
  timestamps are on the capture timeline (`Segmenter::reset(origin)`).
- **Turn mode closes the microphone device between turns**, not merely ignores it. A turn is
  one utterance, trimmed by the VAD and split at pauses only past 25 s, because Whisper hears
  30 s. Taking a turn stops any reply mid-word and holds replies that arrive during it, so the
  half-duplex gate never eats a turn.
- **The turn key** is removed from the frame's events before any widget runs
  (`gui::take_turn_key`), so Space never also presses a focused button. egui's `keys_down` is
  left alone, because egui uses it to mark auto-repeats. cnverc also keeps its own key state
  (`gui::TurnKey`), so a held key counts as one press.
- **The voice is chosen by target language**, not by a config key: §7 defines none, and §9
  says the language decides which voice speaks.
- **Recognition and translation each use 6 threads.**
- The window renders with **DirectX 12** on Windows. The Vulkan backend logged loader errors
  at startup.

## What the translation stage refuses

A small translation model fails in ways that are worse than failing visibly, so `translate.rs`
rejects an output rather than let it be captioned and spoken when it is:

1. **An echo of the source.** Names, numbers and single words that translate to themselves
   are exempt.
2. **A recitation of the system prompt.**
3. **Far longer than its input.**

All three failures were found in live sessions. Don't remove a guard without a replacement: the
failure it prevents is a caption and a voice asserting something the speaker never said.

## Choosing the translation model

The recommended model is **Qwen3 1.7B**. It started as 0.6B, which is smaller and faster but
not good enough. On the same twenty test sentences (the `bench_both_directions` test):

| Model (Q4_K_M) | English→Spanish | Spanish→English | Per sentence (CPU) |
|---|---|---|---|
| Qwen3 0.6B | 1 of 10 correct; 7 handed back untranslated | 9 of 10 | ~0.35 s |
| Qwen3 1.7B | 10 of 10 usable | 10 of 10 | ~0.9 s |

The 0.6B fails by handing the source straight back, or sometimes by producing a confident
wrong answer. No prompt wording fixed that: wording that stopped the echo produced wrong
translations instead. The echo guard catches the first failure. Only a better model fixes the
second.

The prompt is written for Qwen's chat format, so the model in `models/mt/` must be a Qwen3.
`models/mt/` holds exactly one `.gguf`. Two files is an error that asks you to remove one,
rather than cnverc choosing for you. There is no config key naming the file, because §7
defines none; the filesystem is the index.

Qwen's own GGUF repositories publish only Q8_0, which is why the Q4_K_M comes from unsloth.

## Comparing recognizers

`--compare` (or **Compare recognizers** in the window) runs every installed recognizer on each
utterance and prints the results side by side. It exists because published word error rates
are measured on read speech, not on your microphone and your accent (`SPEC.md` §12).

It always prints each engine's wall-clock time together with the length of the audio. Whisper
pads every utterance to a 30-second window internally, so a 1-second utterance costs it about
what a 20-second one does.

In live testing, Parakeet dropped words for this user where Whisper didn't, so Whisper is the
one in use.

## Model layout and discovery

Everything resolves from the directory containing `cnverc.exe`, never from the working
directory or `%APPDATA%`. `paths::app_root()` is the only path derivation.

```
cnverc/
  cnverc.exe
  cnverc.toml
  models/
    vad/silero_vad.onnx
    asr/<engine dir>/engine.toml + model files
    mt/<translation>.gguf
    tts/<voice dir>/engine.toml + model files
  logs/
```

Each ASR and TTS directory describes itself in an `engine.toml`. Adding a model means dropping
a folder in and restarting: there is no registry, no cache and no download UI.

- A directory without an `engine.toml` is skipped with a warning.
- A directory whose `engine.toml` names a missing file is listed but **disabled**, with the
  missing filename shown.
- Piper voices declare their `espeak-ng-data` directory with `data_dir`, because a directory
  can't be listed under `[files]`.
- Model files keep their published names. If an archive's filenames differ from what the
  shipped `engine.toml` declares, edit the `engine.toml`; don't rename the model files.

The `engine.toml` files are committed; the weights never are (see `.gitignore`). All models
are expected to be resident at the same time and to fit in 8 GB of VRAM alongside a desktop
session.

## Building

### The static CRT

`.cargo/config.toml` sets `LLAMA_STATIC_CRT`, `CMAKE_MSVC_RUNTIME_LIBRARY` and
`-C target-feature=+crt-static`. This isn't a preference. sherpa-onnx's prebuilt static
library uses the static CRT, llama.cpp's cmake build defaults to the dynamic one, and MSVC
refuses to link the two:

```
error LNK2038: mismatch detected for 'RuntimeLibrary': value 'MT_StaticRelease'
doesn't match value 'MD_DynamicRelease'
```

Matching everything to the static CRT is also what §2.6 needs: the executable depends on no
Visual C++ redistributable. `llama-cpp-2`'s default `openmp` feature is off for the same
reason: it links `VCOMP140.DLL`, which a freshly imaged machine may not have. The result
depends on Windows system DLLs only:

```
kernel32.dll  advapi32.dll  ole32.dll  oleaut32.dll  dbghelp.dll  setupapi.dll  dxgi.dll  ntdll.dll
```

Re-check with `dumpbin -dependents cnverc.exe` whenever a dependency is added. Don't fix a
RuntimeLibrary mismatch by switching sherpa to its dynamic build.

### Build-time internet

The `sherpa-onnx` crate's build script downloads a matching prebuilt `-lib` archive from GitHub
releases unless `SHERPA_ONNX_LIB_DIR` is set, so the first build on a new machine needs
internet. The binary never does. For offline rebuilds, keep the extracted archive and point at
it:

```bash
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx-vX.Y.Z-win-x64-static/lib
cargo build --release
```

On Windows, disable any Vulkan feature flag that appears in a dependency rather than debugging
it: Vulkan-backed whisper builds have failed on this platform before.

There is no JavaScript toolchain, no `package.json` and no webview. The GUI is native egui
compiled into the binary.

## Development workflow

During development cnverc runs from `target/debug/`, so copy the models and config there once:

```bash
cp -r models cnverc.toml target/debug/
```

The model files themselves go in the same places as the README's setup steps, under
`target/debug/models/` instead of `target/release/models/`.

Before every commit:

```bash
cargo fmt --check
```

```bash
cargo clippy --all-targets -- -D warnings
```

```bash
cargo test
```

A running `cnverc.exe` locks its own file, so close the window before rebuilding or the build
fails with "Access is denied".

### Tests that need real audio

Some tests need files that aren't in the repository: the models and 16 kHz mono recordings of
someone talking. They are `#[ignore]`d by default:

```bash
CNVERC_TEST_VAD_MODEL=/abs/path/silero_vad.onnx CNVERC_TEST_WAV=/abs/path/speech.wav CNVERC_TEST_MODELS=/abs/path/models CNVERC_TEST_WAV_ES=/abs/path/spanish-16k.wav cargo test --release -- --ignored --nocapture
```

The recordings must be 16 kHz mono. The pipeline resamples at the capture boundary and nowhere
else, and the test reader refuses to add a second resampling path. The Parakeet archive ships
`test_wavs/es.wav` at 22050 Hz; convert it once with:

```bash
ffmpeg -i es.wav -ar 16000 -ac 1 es-16k.wav
```

### Logs

Logs are written to `logs/` next to the exe. `--listen --wav` also writes each utterance to
`logs/segments/`, which is how a misheard sentence gets diagnosed.

## Paired mode networking (Milestone 7)

The networking notes (which cables and networks work, addressing, and the Windows firewall
trap) are in the README's "Connecting two PCs" section, because §14 puts them there.
