# cnverc: technical notes

Why cnverc is built the way it is: design decisions, measurements, and the build internals
that are easy to break. Read this before changing the build or reversing a decision.

- How the code fits together: [ARCHITECTURE.md](ARCHITECTURE.md).
- The models, and how to add one: [MODELS.md](MODELS.md).
- The build specification: [SPEC.md](SPEC.md). Its hard constraints are restated in
  [CLAUDE.md](CLAUDE.md), and project status is kept there and in [HANDOFF.md](HANDOFF.md).

## Design decisions

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
  received utterance's own `lang`, never inferred. When several voices fit, `models::rank`
  decides, by the variety declared in each voice's `engine.toml` (M7.7). Shared mode alone has
  per-side voice pickers.
- **Varieties are BCP 47 tags from a fixed table** (`src/varieties.rs`), not free text: a tag
  the table doesn't know is refused, because a prompt saying "into xx-YY" translates worse than
  one naming the dialect, and a typo would otherwise pass silently. Whisper is told only the
  language part; its language token has no regions.
- **Whisper is this user's recognizer.** In live `--compare` testing, Parakeet dropped words
  where Whisper didn't. Parakeet stays installed and is allowed everywhere, including Shared
  mode, where it carries a note because it ignores the language it's told.
- **Recognition and translation each use 6 threads.**
- The window renders with **DirectX 12** on Windows. The Vulkan backend logged loader errors
  at startup. It uses **FXC**, the shader compiler built into Windows: wgpu's default takes any
  `dxcompiler.dll` on PATH first, and Wireshark's old copy made the window fail with "Parent
  device is lost". With FXC, no DLL has to sit beside `cnverc.exe`.

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
- `SHERPA_ONNX_LIB_DIR` must point at the shared build's `lib/`. `build-sherpa-linux.sh`
  builds it into `~/sherpa-onnx/install`, and `setup.sh` sets the variable.
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
Ubuntu 24.04 LTS and Fedora don't package onnxruntime at all. On such a machine sherpa-onnx's cmake falls back to downloading the same
static `1.28.2-glibc2_17` build that crashes, **the configure and build both succeed**, and the
failure only appears when a model session is created. That is why `build-sherpa-linux.sh`
checks the configure output for `location_onnxruntime_lib: /usr/lib/...` and stops if it says
`Downloading pre-compiled onnxruntime` instead. Without a distribution package the
options are to build onnxruntime from source, or to try sherpa-onnx's own
`linux-x64-shared-lib` release archive, which bundles a matching onnxruntime and would remove
the distribution dependency entirely - untested here, and the obvious next experiment if Linux
is to be supported properly.

**Version skew is untested.** The pairing verified here is sherpa-onnx 1.13.8 (released against
onnxruntime 1.28) with Ubuntu's 1.23. An older packaged onnxruntime - Debian trixie ships
roughly 1.16 - may fail to compile, or compile and then fail on an operator used by the Whisper
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
