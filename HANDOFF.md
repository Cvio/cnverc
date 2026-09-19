# Handoff: where cnverc stands, for the next session

Written 2026-09-19 at the end of a long Windows session, for a new session starting on the
user's Linux PC. Read this first, then `CLAUDE.md` (the hard constraints and the working
rules), then `SPEC.md` (the build specification). `README.md` is the user's setup guide, and
`TECHNICAL.md` covers design and internals.

## 1. What cnverc is

An offline speech-to-speech translator in Rust. Microphone → Silero VAD → speech recognition
(Parakeet or Whisper, via sherpa-onnx) → translation (Qwen3 1.7B GGUF, via llama.cpp) → speech
(Piper voices, via sherpa-onnx) → speakers, with captions in a native egui window. It
**never touches the internet**. Everything resolves from the folder the executable is in.
The project's acceptance test (Milestone 9) is: zip the folder, unzip it on a freshly imaged
Windows PC with no internet, double-click, and it works.

Windows is the primary target. SPEC §3 calls Linux "a nice-to-have; do not let it complicate
the Windows path." Any Linux fix must leave the Windows build and its static-CRT setup exactly
as they are.

## 2. Status

- **M0–M6 are complete** and checked by the user on Windows: capture, VAD, both recognizers,
  translation, speech, the window, continuous and turn-based modes, and the Space turn key.
- **M7 (paired mode) is built and committed** (`434fa60`, pushed) but **its check has not
  run**. The check needs two PCs on an isolated switch: a turn-based Spanish/English
  conversation, then a pulled cable force-releasing the floor on both ends. The second PC is
  the user's Linux box, `ubox`.
- **Linux is where things are stuck.** See section 5.
- M8 (streaming ASR) and M9 (portability acceptance) haven't started. Don't work ahead of
  M7's check.

## 3. How the code works

`src/`, one line each:

| File | What it does |
|---|---|
| `main.rs` / `cli.rs` | No arguments opens the window. `--report`, `--devices`, `--listen [--seconds N] [--wav] [--compare]` |
| `paths.rs` | `app_root()`: the exe's folder. The only path derivation. |
| `config.rs` | `cnverc.toml` (SPEC §7). `save_selections` edits the file in place with toml_edit and keeps its comments |
| `models.rs` / `report.rs` | Discovery: each model folder's `engine.toml`, the filesystem as the index |
| `audio.rs` | cpal capture, resampled to 16 kHz mono at the capture boundary, and only there |
| `vad.rs` | Silero VAD segmenter, with a 600 ms pre-roll |
| `asr.rs` | `SegmentAsr` for Parakeet and Whisper. `StreamAsr` is for M8 |
| `translate.rs` | `LlamaTranslator`, the Qwen prompt, and three guards (echo, prompt recital, too long). Don't remove a guard |
| `tts.rs` | Piper voices; `for_language` picks the voice by language |
| `playback.rs` | cpal output, the half-duplex `Gate`, and `PlaybackControl` (a turn stops or holds replies) |
| `pipeline.rs` | The threads, `PipelineMsg` (everything a front end hears), `PipelineCmd`, `SpeakJob`. The speaker thread speaks both local and received utterances |
| `gui.rs` | The egui window. `Session` holds the message-driven state with no egui in it, and is unit-tested. Also the turn key handling and the peer panel |
| `listen.rs` | The `--listen` front end. Always continuous and never paired |
| `wire.rs` | The M7 protocol: newline-delimited JSON; every inbound line is untrusted |
| `floor.rs` | The M7 floor token as a pure state machine |
| `peer.rs` | The M7 connection: acceptor, dialler, readers, Hello, pings, one peer at a time, the firewall diagnostic |
| `discovery.rs` | The M7 UDP broadcast pick-list on port 47801 |

Threads: capture → pipeline (VAD, ASR, commands) → translate → speak → playback, plus peer
when paired. In paired mode, `Pipeline::send` routes the turn key to the peer thread. The
peer thread opens the microphone only on `FloorGrant`. Translations go to the other PC, and
what arrives is spoken locally in the language each utterance declares.

Tests: `cargo test` runs 107, all passing on Windows as of `434fa60`, including loopback
tests with two or three peers. 14 more are `#[ignore]`d because they need models, recordings
or a loopback audio device; see `TECHNICAL.md` for how to run them.

## 4. Setup, and what goes wrong

The README's steps have been followed on a fresh clone on Windows, and every model command
worked. Things learned the hard way:

**Windows**
- Close any running `cnverc.exe` before rebuilding, or the build fails with "Access is denied
  (os error 5)".
- Every new Git Bash window needs the CMake `export PATH=...` line from README step 5.
- **LLVM (libclang) is required**, because `llama-cpp-2` runs bindgen. It was missing from the
  README until M7; this PC had it installed already, which hid the gap.
- The README's "Building again without internet" note shows
  `export SHERPA_ONNX_LIB_DIR=/path/to/...` as a placeholder. The user ran it literally, and
  the build failed looking for that path; `unset SHERPA_ONNX_LIB_DIR` fixed it. **That note
  should be reworded**: make it clearly optional, and give the real folder,
  `target/sherpa-onnx-prebuilt/<archive>/lib`. The user has asked for this and it isn't done
  yet.
- Everything links the **static CRT** (`.cargo/config.toml`), and `llama-cpp-2` is built
  without `openmp`. Both are required on Windows (see CLAUDE.md's build note). Don't change
  either to fix a Linux problem; use `[target.'cfg(target_os = "linux")'...]` sections if
  Linux needs something different.
- The window renders with DirectX 12 on Windows only (`gui::graphics_setup`); elsewhere wgpu
  chooses.

**Both**
- cnverc reads `models/` and `cnverc.toml` from the exe's folder: `target/release/` for the
  user, `target/debug/` during development. Each folder has its own `cnverc.toml` with its own
  device choices.
- `--report` should show `[+]` for every file and `ok` for every recognizer and voice.
- Two instances on one PC need separate folders and different `[peer].listen_addr` ports, and
  only one of them can use discovery (UDP 47801).

## 5. The open problem: Linux crashes when Start is pressed

**Machine:** `ubox`, Ubuntu with Mesa 26 and an NVIDIA GT 1030 plus Intel UHD 630. The repo is
at `~/Desktop/projects/cnverc`.

**What works:** it builds, `--report` passes, the window opens (on Wayland; wgpu picks Vulkan),
and "Pair with another PC" is shown.

**What fails:** with pairing ticked, pressing Start closes the program:

```
INFO cnverc::peer: paired mode: listening on port 47800 as "ubox"
INFO cnverc::discovery: discovery: announcing "ubox" on UDP port 47801
free(): invalid pointer
Aborted (core dumped)
```

It is a native (C/C++) abort, not a Rust panic. **Not yet known:** whether Start *without*
pairing also crashes. The last two log lines come from the peer thread, but the pipeline
thread starts loading the recognizer at the same moment. That is sherpa-onnx's C++ code, and
this was its first run on Linux.

**Suspects, most likely first:**
1. **Loading a model on Linux.** sherpa-onnx's prebuilt Linux static archive
   (`sherpa-onnx-v1.13.8-linux-x64-static-lib`, with onnxruntime, espeak-ng and
   piper_phonemize inside) and llama.cpp, compiled locally with Ubuntu's GCC, end up in one
   binary. A clash between those C++ libraries, or a memory allocated by one and freed by the
   other, gives exactly `free(): invalid pointer`. If so, one experiment is sherpa-onnx's
   `shared` feature on Linux only; its build script supports `linux-x64-shared-lib`. Do it as
   a Linux-only dependency override so Windows keeps the static build. A shared library also
   has to ship next to the exe, which affects portability on Linux only.
2. **Something in paired mode:** `if_addrs` (getifaddrs/freeifaddrs) in `peer::local_addresses`
   and `discovery::broadcast_targets`, or `gethostname`. Less likely, but cheap to rule out.

**Diagnose in this order:**
1. `./target/release/cnverc --listen --seconds 10`. This loads every model with pairing
   forced off. A crash here means model loading (suspect 1); a clean run means paired mode
   (suspect 2).
2. Get a backtrace from a debug build, which keeps its symbols; the release profile has
   `strip = true`:
   ```bash
   sudo apt install gdb
   cargo build
   ln -s ../release/models target/debug/models
   cp target/release/cnverc.toml target/debug/
   gdb -batch -ex run -ex bt --args ./target/debug/cnverc
   ```
   Reproduce the crash, then read which library's frames call `free`.
3. The `#[ignore]`d tests load each model on its own (VAD, recognizers, translator, voice).
   Running them one at a time narrows it to one library. See `TECHNICAL.md`, "Tests that need
   real audio".

**Linux noise to ignore:** `ALSA lib pcm.c ... Unknown PCM pulse/jack/oss` is cpal probing
ALSA plugins that aren't installed. It's harmless, unless no microphone or speaker shows up in
the window, in which case install `pipewire-alsa` or `libasound2-plugins`.
`sctk_adwaita: Ignoring unknown button type` is harmless too.

**Linux firewall for M7:** `sudo ufw allow 47800/tcp` and `sudo ufw allow 47801/udp`, if ufw
is on.

## 6. Pitfalls already paid for

- **egui key repeat:** don't remove the turn key from `input.keys_down`. egui uses it to mark
  auto-repeats, and removing it made a held Space flip turns on and off (fixed in `7b95843`).
- **Translation:** Qwen3 0.6B scored 1/10 English→Spanish, so 1.7B is installed. The prompt is
  Qwen-specific, and `models/mt/` must hold exactly one `.gguf`.
- **Recognizer:** Whisper is this user's choice; Parakeet dropped words for them. Whisper pads
  every utterance to 30 s, so a turn is split at pauses beyond 25 s.
- **Half-duplex:** with speakers, the gate stops cnverc hearing itself. Taking a turn stops
  and holds replies, so the gate can't eat the start of a turn.
- **Paired mode:** addresses are IP only, and hostnames are refused, because resolving one
  could reach a public DNS server (§2). A dead link is noticed by pings stopping for 6 s, not
  by TCP.
- **For an agent's shell:** in this environment, long heredocs in the Bash tool broke with
  "unexpected EOF", and escape sequences like `` in tool input were turned into real
  characters. Write multi-line edit scripts to a file first.

## 7. Working with this user

- When they **ask a question, answer it and wait.** Don't run commands or change their
  `cnverc.toml` unasked; they have said so explicitly.
- They test live and report back. Give them a concrete test to run, then read the logs in
  `logs/` next to the exe.
- Commit at milestone boundaries, or when asked. Keep `cargo fmt` and
  `cargo clippy --all-targets -- -D warnings` clean. The user pushes, or asks for a push.
- Build in milestone order, and don't start M8 before M7's check passes.

## 8. Next steps

1. Find and fix the Linux crash (section 5) without changing the Windows build.
2. Reword the README's `SHERPA_ONNX_LIB_DIR` note (section 4).
3. Run the M7 check between the Windows PC and `ubox`: README, "Talking between two PCs".
4. When it passes, mark M7 complete in `CLAUDE.md` and `TECHNICAL.md`, commit, and wait for the
   go-ahead on M8.
