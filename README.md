# convers

Offline speech-to-speech translation. Microphone → VAD → ASR → translation → TTS → speakers,
with captions, running as a single folder you can copy to a machine that has never been
online.

`convers` **never touches the internet**. It does not download models, check for updates,
phone home, or talk to any service. If a model file is missing it prints the absolute path it
expected and exits non-zero. See `CLAUDE.md` for the constraints in full, and `SPEC.md` for
the build specification.

Local network sockets are a different matter and are used deliberately: paired mode (two
machines, one conversation) is plain TCP to an address you type in, and works on a switch with
no uplink and no DNS anywhere.

## Status

**Milestone 3 of 9.** The binary resolves its own directory, reads `convers.toml`, enumerates
the model tree, captures from the microphone, cuts utterances with Silero VAD, transcribes them
with either Parakeet or Whisper, and translates the result with a GGUF model running in
process. Nothing speaks yet. Milestones are listed in `SPEC.md` §13 and are built in order.

```bash
convers                       # what models are installed
convers --devices             # what microphones are available
convers --listen              # listen and transcribe with the configured engine
convers --listen --wav        # also write each utterance to logs/segments/
convers --listen --compare    # run every engine on each utterance, side by side
convers --listen --seconds 20 # stop cleanly after 20 s
```

`models/mt/` holds exactly one `.gguf`. There is no key in `convers.toml` naming it — §7 does
not define one — so the filesystem is the index here too; two files is an error asking you to
remove one rather than convers choosing for you.

`--compare` exists because published word error rates are measured on read speech, not on your
microphone and your accent (`SPEC.md` §12). It prints each engine's transcript with the wall
clock time and the duration of the audio, always together: Whisper pads every utterance to a
30-second window internally, so a 1-second utterance costs it about what a 20-second one does.

## Layout

Everything resolves from the directory containing `convers.exe` — never from the working
directory, never from `%APPDATA%`. Move the folder anywhere, including another drive, and
nothing changes.

```
convers/
  convers.exe
  convers.toml
  models/
    vad/silero_vad.onnx
    asr/<engine dir>/engine.toml + model files
    mt/<translation>.gguf
    tts/<voice dir>/engine.toml + model files
  logs/
```

Each ASR and TTS directory describes itself in an `engine.toml`. Adding a model means dropping
a folder in and restarting: there is no registry, no cache, and no download UI. A directory
without an `engine.toml` is skipped with a warning; a directory whose `engine.toml` points at
a file that is not there is listed but **disabled**, with the missing filename on screen.

## Building

```bash
cargo build --release
```

Rust stable, 2021 edition. There is no JavaScript toolchain, no `package.json`, and no
webview; the GUI (from Milestone 5) is native `egui` compiled into the binary.

**Build prerequisites on Windows:** MSVC (Visual Studio Build Tools) and **cmake**, because
`llama-cpp-2` compiles llama.cpp from source. Build Tools ships a cmake that is not on `PATH`
by default; either install cmake separately or add the bundled one for the build:

```bash
export PATH="$PATH:/c/Program Files (x86)/Microsoft Visual Studio/18/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin"
```

### Everything links against the static CRT

`.cargo/config.toml` sets `LLAMA_STATIC_CRT`, `CMAKE_MSVC_RUNTIME_LIBRARY` and
`-C target-feature=+crt-static`. This is not a preference. sherpa-onnx ships its prebuilt
static library built against the static CRT, llama.cpp's cmake build defaults to the dynamic
one, and MSVC refuses to link the two together:

```
error LNK2038: mismatch detected for 'RuntimeLibrary': value 'MT_StaticRelease'
doesn't match value 'MD_DynamicRelease'
```

Matching everything to the static CRT is also what §2.6 needs: the executable then depends on
no Visual C++ redistributable. `llama-cpp-2`'s default `openmp` feature is off for the same
reason — it links `VCOMP140.DLL`, which a freshly imaged machine may not have. The result
depends on Windows system DLLs only:

```
kernel32.dll  advapi32.dll  ole32.dll  oleaut32.dll  dbghelp.dll  setupapi.dll  dxgi.dll  ntdll.dll
```

Worth re-checking with `dumpbin -dependents convers.exe` whenever a dependency is added.

### Build-time internet caveat (applies from Milestone 1)

The `sherpa-onnx` crate's build script downloads a matching prebuilt `-lib` archive from
GitHub releases unless `SHERPA_ONNX_LIB_DIR` is set. **The first build on a new development
machine therefore needs internet**, even though the resulting binary never does. For offline
rebuilds, keep the extracted archive and point at it:

```bash
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx-vX.Y.Z-win-x64-static/lib
cargo build --release
```

On Windows, disable any Vulkan feature flag that appears in a dependency rather than debugging
it — Vulkan-backed whisper builds have failed on this platform before.

### Running during development

`convers` looks for `models/` and `convers.toml` next to the executable, which during
development means `target/debug/`. Copy them once:

```bash
cp -r models convers.toml target/debug/
```

Checks:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Two tests need files that are not in the repository — the VAD model, and a 16 kHz mono
recording of someone talking — so they are `#[ignore]`d by default:

```bash
CONVERS_TEST_VAD_MODEL=/abs/path/silero_vad.onnx CONVERS_TEST_WAV=/abs/path/speech.wav CONVERS_TEST_MODELS=/abs/path/models CONVERS_TEST_WAV_ES=/abs/path/spanish-16k.wav cargo test --release -- --ignored --nocapture
```

Both recordings must be 16 kHz mono: the pipeline resamples at the capture boundary and
nowhere else, and the test reader refuses to introduce a second resampling path. The Parakeet
archive ships `test_wavs/es.wav`, which is 22050 Hz — convert it once with
`ffmpeg -i es.wav -ar 16000 -ac 1 es-16k.wav`.

## Models

`convers` does not fetch any of these. Download them on a machine that has internet, extract
them into the layout above, and copy the folder across. The application never uses the URLs
below — they are here for you, not for it.

Nearly everything comes from the sherpa-onnx model releases, which are the authoritative
listings. Asset filenames change between releases, so take the exact name from the release
page rather than assuming the one quoted here:

- ASR: <https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models>
- TTS: <https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models>

| What | Where it goes | Source |
|---|---|---|
| Silero VAD | `models/vad/silero_vad.onnx` | `silero_vad.onnx` from <https://github.com/snakers4/silero-vad> (`files/silero_vad.onnx`), or the copy in the sherpa-onnx VAD release assets |
| Parakeet TDT 0.6B v3, int8 | `models/asr/parakeet-tdt-0.6b-v3-int8/` | `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2` (487 MB) from the ASR release listing |
| Whisper large-v3-turbo, int8 | `models/asr/whisper-large-v3-turbo/` | `sherpa-onnx-whisper-turbo.tar.bz2` (564 MB) from the ASR release listing — the release calls it "turbo", and the files inside are named `turbo-*` |
| Qwen3 0.6B, Q4_K_M | `models/mt/qwen3-0.6b-q4_k_m.gguf` | `Qwen3-0.6B-Q4_K_M.gguf` (397 MB) from <https://huggingface.co/unsloth/Qwen3-0.6B-GGUF> — Qwen's own GGUF repo publishes only Q8_0 |
| Spanish Piper voice | `models/tts/vits-piper-es_ES-carlfm-x_low/` | `vits-piper-es_ES-carlfm-x_low.tar.bz2` from the TTS release listing |

Extract each archive so the model files sit **directly** in the directory named above, beside
its `engine.toml` — not in a nested folder from the archive. Keep the published filenames. If
the names in an archive differ from what the shipped `engine.toml` declares, edit the
`engine.toml`; do not rename the model files.

The `engine.toml` files in this repository are committed; the weights are not. Run the binary
after extracting and read the table: it names every file it looked for and where.

All four models are expected to be resident simultaneously and to fit in 8GB of VRAM alongside
a desktop session.

## Paired mode networking

Two machines each run their own complete pipeline. **Only text crosses the wire** — never
audio, never models. Laptop A captures Spanish, transcribes and translates locally, and sends
the English text; laptop B displays it and speaks it in its own voice. An utterance costs a few
hundred bytes, so the link can be terrible and it still works.

Any transport that presents to the OS as an IP interface works, and the socket code is
identical across all of them:

| Transport | Works | Notes |
|---|---|---|
| Unmanaged Ethernet switch | Yes | No uplink needed |
| Router with the WAN unplugged | Yes | Gives you DHCP, which is convenient |
| Ethernet cable laptop-to-laptop | Yes | No crossover cable needed |
| Wi-Fi hotspot from one laptop | Yes | No upstream required |
| Existing Wi-Fi LAN | Yes | Guest-network client isolation will block it |
| Thunderbolt / USB4 networking | Yes | Virtual Ethernet adapter; fastest option |
| USB bridge/transfer cable | Yes | Presents as a NIC to each side |
| Two USB-C-to-Ethernet dongles | Yes | Ordinary cable between them |
| **Plain USB-C cable between two laptops** | **No** | Both ends are USB hosts; there is no network |

**Addressing.** A dumb switch or a direct cable means no DHCP, so Windows falls back to
link-local addressing (169.254.x.x) after roughly thirty seconds. It works, but the addresses
are ugly and can change between sessions. For a rig you use repeatedly, set static IPs on that
interface once — 192.168.50.1 and 192.168.50.2 — and forget about it.

**Firewall.** A cable or uplink-less switch produces a network Windows cannot identify, and it
is frequently classified as **Public**, which blocks inbound connections. The listener binds,
the peer connects to nothing, and no useful error appears. Set that interface's network
profile to Private. From Milestone 7, `convers` detects the bound-but-never-accepted state and
says so specifically rather than showing a generic timeout.

## Licence

Unpublished. Model files carry their own licences; check each one.
