# CLAUDE.md — `convers`

`SPEC.md` is the build specification. Read it before changing anything. This file exists so
the constraints below survive into sessions that have not read it.

## Hard constraints (SPEC §2, restated in full)

These are non-negotiable. If a design choice conflicts with one of these, the design choice
loses. If you believe one of these is wrong, stop and say so rather than working around it.

1. **The application must never require, attempt, or depend on internet access.** No model
   downloads, no telemetry, no update checks, no license checks, no cloud inference, no
   public DNS lookups, no CDN fetches, no crash reporting. If a required model file is absent,
   `convers` prints the exact absolute path it expected and exits non-zero. It does not offer
   to fetch it.

   **Local network sockets are explicitly permitted and required** for paired mode (SPEC §9):
   TCP and UDP to a peer address the user has entered or that was discovered on the local
   link. This is not an exception to the rule above — it introduces no internet dependency,
   and paired mode must work on a network with no gateway and no upstream at all. What is
   forbidden is *reaching the internet*, not *opening a socket*.

   Practical test for any network code added to this project: **would it still work with the
   WAN cable unplugged and no DNS server anywhere on the segment?** If yes, it is allowed. If
   no, it does not belong here. This rules out any hosted signalling, STUN/TURN, relay
   service, or hostname that must resolve through a public resolver.

2. **No separate services.** No Ollama, no `localhost:11434`, no sidecar process, no Docker,
   no external inference server. Every model runs in-process. (The peer listener in SPEC §9 is
   part of `convers` itself, not a separate service.)

3. **No JavaScript toolchain.** No `package.json`, no `node_modules`, no npm, no bundler,
   anywhere in the repository. The GUI is native Rust (SPEC §4).

4. **No OS-managed application directories.** Never write to or read models from `%APPDATA%`,
   `%LOCALAPPDATA%`, `~/.cache`, or anything returned by the `dirs`/`directories` crates. All
   model and configuration paths resolve relative to the executable's own location.

5. **No content-addressed or opaque storage.** Every model file on disk keeps the name it was
   published with, in a directory named after the model. A human opening the folder in
   Explorer must be able to tell what each file is without running anything.

6. **Copy-to-run portability.** The success criterion for the whole project: zip the output
   folder, move it to a freshly imaged Windows machine with no internet, unzip, double-click,
   and it works. This is verified in Milestone 9 and it is the acceptance test.

## Working rules

- Build in the milestone order of SPEC §13. Do not start the next milestone until the stated
  check passes, and do not work ahead. If a milestone seems to require something from a later
  one, stop and raise it.
- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` clean at every commit.
- No `unwrap()` or `expect()` after startup. Startup-time `expect()` naming the missing file
  is fine, and preferred to a silent default.
- Every error message concerning a file names the **absolute path** that was tried.
- `paths::app_root()` is the only path-derivation function. Everything else joins off it.
- Do not add configuration keys, settings, or abstractions that no milestone needs.
- See SPEC §15 for the full "do not" list.

## Status

Milestone 2 complete.

- M0: skeleton, path resolution, config load, model discovery, report.
- M1: cpal capture with device enumeration, resampling to 16 kHz mono at the capture
  boundary, and Silero VAD through sherpa-onnx cutting utterances.
- M2: `SegmentAsr` for Parakeet (nemo_transducer) and Whisper, both traits and the
  `AsrEngine` enum from SPEC §11 in place, the utterance ring buffer, and `--compare`.
  `StreamAsr` deliberately has no implementation until M8.

- M3: translation through `llama-cpp-2`, behind a `Translator` trait, on its own thread so a
  slow token stream cannot stall recognition. The prompt is constrained to translation and
  the output is stripped of reasoning, labels and quotes.

- M4: synthesis through sherpa-onnx `OfflineTts`, playback through cpal, and the half-duplex
  gate. The voice is chosen by language, not by a config key, because §7 defines no key for it
  and §9 says the language decides which voice speaks.

Next is M5, the egui GUI.

## What the translation stage refuses

A 0.6B model fails in ways that are worse than failing visibly, so three
guards in `translate.rs` reject an output rather than let it be captioned and
spoken: an echo of the source, a recitation of the system prompt, and an
output far longer than its input. All three were found in live sessions, not
imagined. Do not remove one without a replacement — the failure it prevents
is a caption and a voice asserting something the speaker never said.

## Build note

Everything links against the **static CRT** (`.cargo/config.toml`). sherpa-onnx's prebuilt
library is `/MT` and llama.cpp defaults to `/MD`; MSVC will not link both. Do not "fix" a
RuntimeLibrary mismatch by switching sherpa to the dynamic build — the static CRT is also what
keeps the executable free of any Visual C++ redistributable dependency, which §2.6 requires.
For the same reason `llama-cpp-2` is built without its default `openmp` feature.
