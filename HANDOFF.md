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

- **M0–M7 are complete** and checked by the user: capture, VAD, both recognizers,
  translation, speech, the window, continuous and turn-based modes, the Space turn key, and
  paired mode.
- **M7's check passed on 2026-09-20** between the Windows PC and `ubox`: a turn-based
  Spanish/English conversation with translations spoken on the far side, and an abruptly
  killed link force-releasing the floor with a clear message on both ends. It was run over
  Wi-Fi, disabling the adapter rather than pulling a cable; the detection is the same either
  way, since both are silent failures caught only by missed pings.
- **M7.5 (shared-machine mode) is built** on the `shared-machine-mode` branch: two people,
  one machine, a key each. Its check is by hand and hasn't been run yet; see section 8. How it
  works is in `TECHNICAL.md`, "Shared-machine mode".
- **Linux runs**, including the window. The Start crash is fixed with a Linux-only shared
  sherpa-onnx build; see section 5.
- M8 (streaming ASR) and M9 (portability acceptance) haven't started.

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
  the build failed looking for that path; `unset SHERPA_ONNX_LIB_DIR` fixed it. The note
  has since been reworded (2026-09-19): it's marked optional, gives the real folder, and says
  to `unset` it if the path is wrong.
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

## 5. Linux: the Start crash, found and fixed (2026-09-19)

**Machine:** `ubox`, Ubuntu 26.04 (GCC 15, glibc 2.43, 10 GB RAM), Mesa 26 with an NVIDIA
GT 1030 and an Intel UHD 630. The repo is at `~/Desktop/projects/cnverc`.

**What it was:** pressing Start ended in `free(): invalid pointer` / `Aborted (core dumped)`.
The handoff's two suspects were a clash between llama.cpp and sherpa-onnx, and paired mode.
It was neither:
- `--listen` (pairing forced off) crashed the same way, which ruled out paired mode.
- sherpa-onnx's own `sherpa-onnx-offline`, built from source (no Rust, no llama.cpp), crashed
  at "Creating recognizer" for Parakeet and Whisper alike, which ruled out cnverc.
- The common factor was the **prebuilt static onnxruntime** (`1.28.2-glibc2_17`) that
  sherpa-onnx bundles, both in the crate's prebuilt archive and in a from-source build.

**The fix**, all Linux-only (the Windows build is unchanged; see TECHNICAL.md, "Linux: shared
sherpa-onnx"):
- sherpa-onnx v1.13.8 built from source with `BUILD_SHARED_LIBS=ON` against Ubuntu's
  `libonnxruntime-dev` (1.23), installed to `~/sherpa-onnx/install`.
- `Cargo.toml` splits `sherpa-onnx` by target: static elsewhere, `features = ["shared"]` on
  Linux.
- `.cargo/config.toml` adds a Linux-only `-Wl,-rpath,$ORIGIN`, because the crate's own rpath
  never reaches the final binary.
- Build with `export SHERPA_ONNX_LIB_DIR="$HOME/sherpa-onnx/install/lib"`, and `-j2`.
- A stale static `libonnxruntime.a` in that `lib/` folder would be linked instead of the
  system `.so`. It was moved to `lib/stale-static/`.

**Verified:** `ldd` shows the `.so` from `target/release/` and the system onnxruntime.
`--report` is all `ok`. A 60 s `--listen` run, with Piper-generated clips played through the
speakers, went capture → VAD → Parakeet → Qwen → Spanish voice → playback, then exited
cleanly. `cargo test`: 107 passed, 14 ignored. fmt and clippy clean. The user has since
confirmed the window opens and runs on `ubox`, unpaired and paired, as part of M7's check.

**Not yet verified:** nothing outstanding from the Linux work.

**A log line that looks like a bug and isn't:** a Spanish clip was transcribed correctly by
Parakeet but captioned `[en]`, and the echo guard then rejected its "translation". That is
correct. cnverc does not detect language: `[languages] source`/`target` in `cnverc.toml` fix
it, and Parakeet is multilingual so `asr.rs` ignores the per-utterance language argument. The
clip didn't match `source = "en"`, so the translator was asked to turn Spanish into Spanish,
and the guard caught the echo exactly as designed.

**Also on this machine:** a fully parallel build ran it out of memory and took Claude Code
down with it. Use `-j2` for `cargo` and `CMAKE_BUILD_PARALLEL_LEVEL=2`.

**Linux noise to ignore:** `ALSA lib pcm.c ... Unknown PCM pulse/jack/oss` is cpal probing
ALSA plugins that aren't installed. It's harmless, unless no microphone or speaker shows up in
the window, in which case install `pipewire-alsa` or `libasound2-plugins`.
`sctk_adwaita: Ignoring unknown button type` and onnxruntime's `Schema error: ...
TreeEnsembleClassifier ... already registered` are harmless too.

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

1. Run M7.5's check by hand (`SPEC.md`, M7.5): both directions heard in the right language with
   speakers up, the other key dead during a turn, Escape cancelling, and turn-based,
   continuous and paired mode re-tested. The log's `transcribing as "<lang>"` lines must match
   the keys pressed. Then merge the branch and mark M7.5 complete in `CLAUDE.md`.
2. Wait for the go-ahead on M8 (streaming ASR). Don't start it unasked.

Open from M7.5, not blocking: Shared mode keeps two copies of the Whisper model in memory,
one per language, because the in-place language change sherpa-onnx has isn't bound by its Rust
crates. The voice-picker issue below is unchanged; Shared mode works around it by choosing
voices by folder name.

Nothing is outstanding from M0–M7. The `[en]`-captioned Spanish clip in section 5 is **not** a
bug and needs no work: cnverc doesn't detect language, the clip simply didn't match the
configured `source`, and the echo guard behaved correctly.

Filed, not started, and not part of any milestone: a **voice picker**. Installing a second
Spanish voice (`vits-piper-es_MX-claude-high`, 2026-09-20) means two TTS entries declare
`languages = ["es"]`, and `for_language` has no tiebreak and no config key — whichever it
picks is arbitrary and the user can't change it. The likely shape is the same as the
recognizer picker: list installed voices for the target language by their `engine.toml`
`name`, save the choice in `cnverc.toml`. Note that the descriptor parser **rejects unknown
fields**, so a `variety = "es-MX"` key cannot be added to an `engine.toml` until `models.rs`
declares it.

Also filed, not started, none of them urgent — three defects in the **peer panel**, found on
2026-09-20 while pairing the Windows PC and `ubox` over a direct Ethernet cable. The pairing
itself worked throughout; none of these stop a connection.

1. **The address list omits an interface that works.** On the Windows PC, `peer::local_addresses()`
   returned the Wi-Fi address and two IPv6 addresses but never the Ethernet adapter's
   `169.254.49.46`, while `discovery.rs` was receiving broadcasts over that same interface and
   listing `ubox` correctly. So `if_addrs::get_if_addrs()` is returning a partial result on
   Windows rather than failing. Without discovery the user would have had no way to learn the
   address to type. That machine has two ExpressVPN adapters in a "Not Present" state, which is
   the obvious suspect but unconfirmed. Related but separate: line ~1057's `.unwrap_or_default()`
   turns a failed enumeration into an empty list, so a real error becomes indistinguishable from
   "no interfaces" — latent here, since the call succeeded.

2. **"No network connection" is asserted while connected.** The `addresses.is_empty()` branch in
   `gui::peer_panel` prints "No network connection. Plug in a cable or join a network, then
   Rescan" — and did so with a live pairing shown two lines above it. An empty list means cnverc
   found no addresses, not that the machine has no network; the wording should say that, and the
   message should be suppressed entirely when `session.peer` is connected. As written it would
   send someone to re-seat a working cable.

3. **IPv6 addresses are offered as something to type.** The panel listed
   `2600:4040:273b:b600:2909:d8fe:c14e:93b2` under "The other PC types one of these." They are
   global, not link-local, so they pass the existing filter, but nobody is going to type one.
   Show IPv4 only, or sort it first and de-emphasise the rest.

Also worth knowing for any future two-machine test: with Wi-Fi left on, discovery finds the
other PC on **both** paths (`169.254.x.x` over the cable and `192.168.1.x` over Wi-Fi) and
clicking the wrong one silently pairs over Wi-Fi, so a cable test proves nothing. Turn Wi-Fi
off first.

Done on 2026-09-19: the Linux crash (section 5), and the README's `SHERPA_ONNX_LIB_DIR` note.
Done on 2026-09-20: M7's check, over Wi-Fi and again over a direct Ethernet cable with no
router, no DHCP and nothing upstream (SPEC §2's "WAN cable unplugged" case, proven); the README
rewritten around `setup-models.sh` with a troubleshooting table; the ARM64 rpath fix.
