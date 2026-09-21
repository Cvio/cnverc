# cnverc: technical notes

This is the maintainer's companion to [README.md](README.md). The README explains what cnverc
is and how to set it up. This file explains how it's built and why. The build specification
is [SPEC.md](SPEC.md); the constraints every change must respect are restated in
[CLAUDE.md](CLAUDE.md). How to tune models for a regional dialect, and which ones, is in
[DIALECTS.md](DIALECTS.md).

## Status

Milestones 0–7 of 9 are complete. Milestones are defined in `SPEC.md` §13 and built in order;
each one's check must pass before the next starts.

Windows is the primary platform. Linux builds and runs the full pipeline too, using a shared
sherpa-onnx build; see "Linux: shared sherpa-onnx" under Building.

| Milestone | What it added |
|---|---|
| M0 | Skeleton, path resolution, config load, model discovery, `--report` |
| M1 | cpal capture, resampling to 16 kHz mono at the capture boundary, Silero VAD cutting utterances |
| M2 | Segment ASR for Parakeet and Whisper, the utterance ring buffer, `--compare` |
| M3 | In-process translation through llama.cpp, on its own thread |
| M4 | Synthesis through sherpa-onnx, playback through cpal, the half-duplex gate |
| M5 | The egui window; the pipeline reports only through `PipelineMsg` |
| M6 | Continuous and turn-based modes, switchable while running; the turn key in toggle and hold styles |
| M7 | Paired mode: listener, dialler, `Hello`, `Utterance`, the floor token, the peer panel, the firewall diagnostic, the headset warning, discovery |

## Architecture

```
mic ─► capture ─► VAD ─► ASR ─► translate ─┬─► speak ─► playback ─► speakers
       (cpal)   (Silero)  │     (llama.cpp) │   (Piper)   (cpal)
                          │                 └─► peer ◄──► the other PC (text only)
                          └──────► PipelineMsg ──────► window / terminal
```

Threads, connected by bounded channels:

- **Capture** runs the cpal input callback. It resamples to 16 kHz mono once, at the capture
  boundary. Nothing downstream resamples again.
- **Pipeline** (`pipeline.rs`) runs the VAD and ASR, and takes `PipelineCmd`s (mode changes,
  turn start and end).
- **Translate** runs MT, so a slow token stream can't stall recognition. It hands each
  translation to the speaker (solo) or to the peer (paired).
- **Speak** runs TTS for everything this PC says: its own translations when solo, and the
  other PC's utterances when paired. The voice is chosen per utterance by its language.
- **Playback** runs the cpal output and the half-duplex `Gate`.
- **Peer** (`peer.rs`), only when paired, owns the connection, the handshake and the floor.
  It has helpers: an acceptor, one dialler per Connect, and a reader per connection. A stalled
  or dead peer can't block capture, recognition or the window.

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
- **The voice is chosen by language**, not by a config key: §7 defines none, and §9 says the
  language decides which voice speaks. Solo, that is the target language. Paired, it is each
  received utterance's own `lang`, never inferred.
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

### Linux: shared sherpa-onnx

Linux doesn't use the static build. The prebuilt `sherpa-onnx-v1.13.8-linux-x64-static-lib`
archive bundles an onnxruntime (`1.28.2-glibc2_17`) that aborts with `free(): invalid pointer`
the moment a model session is created, on Ubuntu 26.04 with GCC 15 and glibc 2.43. It isn't
cnverc's code: sherpa-onnx's own `sherpa-onnx-offline`, built from source with the same bundled
onnxruntime, crashed identically at "Creating recognizer", for Parakeet and Whisper alike. Built
as a shared library against Ubuntu's `libonnxruntime` 1.23, every model loads and runs: Silero
VAD, both recognizers and both Piper voices.

How it's wired, all of it Linux-only so the Windows build is untouched:

- `Cargo.toml` declares `sherpa-onnx` twice: the default (static) under
  `cfg(not(target_os = "linux"))`, and `default-features = false, features = ["shared"]` under
  `cfg(target_os = "linux")`. It must be split this way. `sherpa-onnx-sys` refuses to build
  with both `static` and `shared` on, and adding `shared` on top of the default would turn on
  both.
- `SHERPA_ONNX_LIB_DIR` must point at the shared build's `lib/` (README, "Setting up on Linux").
  Without it, the build script downloads the official `linux-x64-shared-lib` archive instead,
  which hasn't been tested.
- The build script copies `libsherpa-onnx-c-api.so` next to the executable, but a dependency's
  `rustc-link-arg` never reaches the final binary, so the rpath it asks for is lost.
  `.cargo/config.toml` adds `-Wl,-rpath,$ORIGIN` under `[target.'cfg(target_os = "linux")']` so
  `cnverc` finds the library in its own folder.
- The shared link asks for `-lonnxruntime`. If the `SHERPA_ONNX_LIB_DIR` folder still holds a
  `libonnxruntime.a`, left over from an earlier static build of sherpa-onnx, the linker finds it
  there before the system `.so` and links the crashing library again. Move it out.

Check the result with:

```bash
ldd target/release/cnverc | grep -E "onnx|sherpa|not found"
```

`libsherpa-onnx-c-api.so` should resolve to `target/release/`, and `libonnxruntime.so.1.23` to
`/usr/lib/x86_64-linux-gnu/`. `readelf -d target/release/cnverc | grep RUNPATH` should show
`$ORIGIN`.

This means the Linux build depends on the system `libonnxruntime` package, so it isn't
copy-to-run the way the Windows build is. §3 makes Linux a nice-to-have, and the Milestone 9
acceptance test is Windows-only. At startup, Ubuntu's onnxruntime prints one harmless line,
`Schema error: ... TreeEnsembleClassifier ... already registered`.

**Distributions without an onnxruntime package.** Ubuntu 26.04 has `libonnxruntime-dev` 1.23.
Ubuntu 24.04 LTS and Fedora don't package onnxruntime at all, which is why the README's Fedora
line can't name it. On such a machine sherpa-onnx's cmake falls back to downloading the same
static `1.28.2-glibc2_17` build that crashes, **the configure and build both succeed**, and the
failure only appears when a model session is created. That is why the README's Linux step 3
asks you to check the configure output for `location_onnxruntime_lib: /usr/lib/...` and to stop
if it says `Downloading pre-compiled onnxruntime` instead. Without a distribution package the
options are to build onnxruntime from source, or to try sherpa-onnx's own
`linux-x64-shared-lib` release archive, which bundles a matching onnxruntime and would remove
the distribution dependency entirely — untested here, and the obvious next experiment if Linux
is to be supported properly.

**Version skew is untested.** The pairing verified here is sherpa-onnx 1.13.8 (released against
onnxruntime 1.28) with Ubuntu's 1.23. An older packaged onnxruntime — Debian trixie ships
roughly 1.16 — may fail to compile, or compile and then fail on an operator used by the Whisper
or Parakeet graphs. Nobody has tried it.

Build on a small machine with `-j2` (for both `cargo` and sherpa-onnx's `cmake --build`). A fully
parallel build of llama.cpp and sherpa-onnx ran a 10 GB PC out of memory.

### Build-time internet

The `sherpa-onnx` crate's build script downloads a matching prebuilt `-lib` archive from GitHub
releases unless `SHERPA_ONNX_LIB_DIR` is set, so the first build on a new machine needs
internet. The binary never does. For offline rebuilds, keep the extracted archive and point at
it:

```bash
export SHERPA_ONNX_LIB_DIR="$PWD/target/sherpa-onnx-prebuilt/sherpa-onnx-v1.13.8-win-x64-static-MT-Release-lib/lib"
cargo build --release
```

Leave `SHERPA_ONNX_LIB_DIR` unset otherwise: if it names a folder that doesn't exist, the build
fails. On Linux it's always set, to the shared build (above).

On Windows, disable any Vulkan feature flag that appears in a dependency rather than debugging
it: Vulkan-backed whisper builds have failed on this platform before.

There is no JavaScript toolchain, no `package.json` and no webview. The GUI is native egui
compiled into the binary.

## Development workflow

During development cnverc runs from `target/debug/`, so copy the models and config there once,
then point the download script at the same folder:

```bash
cp -r models cnverc.toml target/debug/
```

```bash
./setup-models.sh target/debug
```

`setup-models.sh` takes the folder the executable is in and defaults to `target/release`. It
skips anything already present, so it is safe to re-run, and it is the only place the model
URLs are written down: the README's model table names the archives but does not repeat the
commands.

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

### Testing without a person talking

A Piper voice can supply the speech. This is how the Linux build was checked end to end
without anyone at the microphone. Make a clip with sherpa-onnx's TTS program (from the shared
build's `bin/`), then play it into a running `--listen` through the speakers:

```bash
V=target/release/models/tts/vits-piper-en_US-lessac-medium
sherpa-onnx-offline-tts --vits-model=$V/en_US-lessac-medium.onnx --vits-tokens=$V/tokens.txt --vits-data-dir=$V/espeak-ng-data --output-filename=en.wav "Where is the train station?"
```

```bash
./target/release/cnverc --listen --seconds 60 &
sleep 30; aplay en.wav; wait
```

The model load takes about 20 s, which is why the clip plays after 30. Piper writes 22050 Hz
(the `x_low` Spanish voice writes 16 kHz). `sherpa-onnx-offline` and cnverc's capture path both
resample, but `sherpa-onnx-vad` and the `#[ignore]`d tests need 16 kHz, so convert first. The
same clip also makes a quick check that sherpa-onnx itself works, with no cnverc involved:
run it through `sherpa-onnx-offline` with the recognizer's files.

### Logs

Logs are written to `logs/` next to the exe. `--listen --wav` also writes each utterance to
`logs/segments/`, which is how a misheard sentence gets diagnosed.

## Paired mode

Two cnverc instances as the two ends of one conversation (SPEC §9). How to use it, and the
networking notes §14 requires, are in the README's "Talking between two PCs" section. This is
how it works.

**What crosses the wire.** Text only: `wire.rs` is newline-delimited JSON over TCP, exactly
the messages in §9, so a connection can be faked with netcat. Each machine runs its whole
pipeline; the sender's translation is the receiver's caption and speech. `Bye` may carry an
optional `reason`, which is how a refused second connection learns why.

**Untrusted input.** `wire::decode` refuses a line over 16 KiB, a line that is not UTF-8, and
any message with an unknown `proto`, the last one shown on screen by name. Every string is
bounded, too long is refused rather than truncated, control characters become spaces, and a
language code must be letters and hyphens. Received text is only ever shown or spoken, and
`source_text` is never translated again.

**No names, only addresses.** The address field takes an IP address, with an optional port
(47800 by default). A hostname would go to the system resolver, which can mean a public DNS
server, so names are refused rather than looked up.

**The floor** is `floor.rs`, a state machine with no sockets or clock, tested case by case:

- The turn key sends `FloorRequest`. `Pipeline::send` routes `BeginTurn` to the peer thread
  when paired, and the peer thread sends `BeginTurn` to the pipeline only when `FloorGrant`
  arrives. The microphone never opens on a timeout: after 2 s the turn fails visibly.
- Simultaneous requests go to the name that sorts first. Two PCs with the same name fall back
  to their addresses, which both ends see the same way round.
- The floor goes back after the turn's utterance has been sent. The translate thread sends
  `ReleaseFloor` after handing the utterance to the peer thread, so the release follows it on
  the wire. A turn with nothing recognised releases the floor at once.
- A grant nobody is waiting for (cancelled, late or stale) is answered with `FloorRelease`, so
  the two ends never disagree about who holds the floor.

**Noticing a dead link.** A pulled cable sends nothing, not even a reset. Each end pings every
2 s, and a reader that hears nothing for 6 s declares the link gone: the floor is
force-released and both windows show the disconnected state. Verified on 2026-09-20 between
the Windows PC and `ubox` over Wi-Fi, by disabling the adapter on one machine mid-session:
both ends reported the loss and the floor was released. A disabled adapter and a pulled cable
are the same case here — nothing arrives either way, and only the missed pings reveal it.

Pairing was checked again the same day over a **direct Ethernet cable between the two PCs**,
with Wi-Fi switched off on both: no router, no DHCP server and nothing upstream on the segment.
Both ends fell back to link-local addressing (`169.254.x.x`) after about thirty seconds,
discovery found the other PC, and the conversation worked. That is SPEC §2's "would it still
work with the WAN cable unplugged and no DNS server anywhere on the segment?" answered on real
hardware rather than by inspection. Three cosmetic defects in the peer panel surfaced during
that run; see HANDOFF.md, section 8.

**One peer at a time.** A second connection from a different PC is sent `Bye` with the reason
and refused. Two connections between the same pair, from both PCs pressing Connect at once,
settle on the one with the lower dialling address; both ends compute the same answer.

**The firewall diagnostic** (§10). A refused dial means nothing is listening. A dial that
times out means packets are being dropped. That message names the Windows firewall and the
network profile, and adds that this PC's own firewall is suspect too if its listener has
never accepted a connection.

**Discovery** (`discovery.rs`) is a UDP broadcast on 47801 to every local network's broadcast
address, plus 255.255.255.255, every 2 s. Windows sends the all-networks broadcast out of one
interface only, so the per-network addresses are what reach a second network card. It only
fills a pick-list. Typing the address always works, and nothing depends on discovery.

**Testing on one PC.** `peer.rs`'s tests run two or three peers over loopback: a handshake, an
utterance each way, the floor, a refused second connection, simultaneous dials, a hand-typed
netcat session, and a dropped connection. To try the window with two instances on one PC,
give the second copy its own folder and a different `[peer].listen_addr` port. Only one of the
two can use discovery, since only one program can hold UDP port 47801.
