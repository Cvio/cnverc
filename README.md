# cnverc

cnverc is a live interpreter for two people who don't share a language. One person speaks
Spanish into the microphone. cnverc writes down what they said, translates it into English,
shows both on screen, and says the English out loud. It works the other way round too.

Everything happens on your own computer. cnverc **never uses the internet**: it doesn't
download anything, check for updates or send anything anywhere. Once it's set up, you can
unplug the network, or copy the whole folder to a PC that has never been online, and it still
works.

Inside, it runs five steps in order:

1. **Listen.** Wait for someone to start talking, and cut the recording off when they stop.
2. **Transcribe.** Turn the speech into text.
3. **Translate.** Translate that text into the other language.
4. **Speak.** Read the translation aloud.
5. **Show.** Put both sentences on screen as captions.

Each step uses a model file that you download once during setup (step 6 below).

Technical details, design notes and the development workflow are in
[TECHNICAL.md](TECHNICAL.md).

---

## Setting up on a new PC

The setup has to be done on a PC **with** internet, because you download the tools and the
models. The finished folder doesn't need internet. These steps are for Windows 10 or 11.

Allow about an hour, most of it spent waiting for downloads and the first build.

### Step 1: Install Git

Download **Git for Windows** from <https://git-scm.com/download/win> and install it with the
default options. It also installs **Git Bash**, the terminal every command below is typed into.

✅ **Check:** open the Start menu, type `Git Bash`, and open it. A black window with a `$`
prompt appears.

### Step 2: Install the Visual Studio Build Tools

cnverc is written in Rust, and parts of it are compiled with Microsoft's C++ compiler.

1. Download **Build Tools for Visual Studio** from
   <https://visualstudio.microsoft.com/downloads/>. It's under "Tools for Visual Studio", and
   it's free.
2. Run the installer. When it asks what to install, tick **Desktop development with C++**.
3. In the list on the right, make sure **C++ CMake tools for Windows** is also ticked.
4. Click **Install** and wait. It's several gigabytes.

Do this step before step 3: the Rust installer looks for these tools.

### Step 3: Install Rust

Download `rustup-init.exe` from <https://rustup.rs> and run it. When it asks, press **Enter**
to accept the default installation.

Then **close Git Bash and open it again**, so it can find Rust.

✅ **Check:** in Git Bash, type:

```bash
cargo --version
```

It should print `cargo 1.95` or a later version. If the number is lower, update Rust with:

```bash
rustup update
```

### Step 4: Get the code

In Git Bash, move to the folder you want cnverc in, then download it. For example:

```bash
cd /d/projects
```

```bash
git clone https://github.com/Cvio/cnverc.git
```

```bash
cd cnverc
```

Every command after this one is run from inside this `cnverc` folder.

### Step 5: Build it

Git Bash can't find the CMake that came with the Build Tools on its own, so tell it where
CMake is. **You need to do this every time you open a new Git Bash window to build:**

```bash
export PATH="$PATH:/c/Program Files (x86)/Microsoft Visual Studio/18/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin"
```

Now build:

```bash
cargo build --release
```

The first build takes a while, often 10 to 20 minutes, and it needs internet (it downloads
some libraries). Later builds are much faster.

✅ **Check:** the last line says `Finished`, and the file `target/release/cnverc.exe` exists.

> **Building again without internet:** only the first build downloads anything: the speech
> library, which it saves in `target/sherpa-onnx-prebuilt/`. If you ever delete `target/` and
> need to rebuild with no internet, copy that folder somewhere safe first, then point the
> build at the `lib` folder inside it before running `cargo build`:
>
> ```bash
> export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx-v1.13.8-win-x64-static-MT-Release-lib/lib
> ```

> **If the build fails with "cmake not found" or similar:** run the `export PATH=...` line
> again and rebuild. If your Build Tools are a different version, the `18` in that path will
> be different; look inside `C:\Program Files (x86)\Microsoft Visual Studio\` to see which
> number you have.

### Step 6: Download the models

cnverc looks for everything in the folder `cnverc.exe` is in, which is `target/release/`.
First copy the settings file and the empty model folders there:

```bash
cp -r models cnverc.toml target/release/
```

```bash
mkdir -p target/release/models/vad target/release/models/mt downloads
```

Now download each model. There are six, about 2.3 GB in total. Paste each block into Git Bash
and wait for it to finish before pasting the next.

**Voice detector.** It notices when someone starts and stops talking.

```bash
curl -L -o target/release/models/vad/silero_vad.onnx https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx
```

**Translator** (1.1 GB). It must be saved under the lower-case name shown here.

```bash
curl -L -o target/release/models/mt/qwen3-1.7b-q4_k_m.gguf https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf
```

**Speech recognizer: Whisper** (564 MB). It turns speech into text.

```bash
curl -L -o downloads/whisper.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-turbo.tar.bz2
tar -xjf downloads/whisper.tar.bz2 -C downloads
cp downloads/sherpa-onnx-whisper-turbo/turbo-encoder.int8.onnx downloads/sherpa-onnx-whisper-turbo/turbo-decoder.int8.onnx downloads/sherpa-onnx-whisper-turbo/turbo-tokens.txt target/release/models/asr/whisper-large-v3-turbo/
```

**Speech recognizer: Parakeet** (487 MB). This is a second recognizer, so you can choose
between them.

```bash
curl -L -o downloads/parakeet.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2
tar -xjf downloads/parakeet.tar.bz2 -C downloads
cp downloads/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/{encoder.int8.onnx,decoder.int8.onnx,joiner.int8.onnx,tokens.txt} target/release/models/asr/parakeet-tdt-0.6b-v3-int8/
```

**English voice** (64 MB). This voice speaks English translations.

```bash
curl -L -o downloads/voice-en.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-lessac-medium.tar.bz2
tar -xjf downloads/voice-en.tar.bz2 -C target/release/models/tts
```

**Spanish voice** (25 MB). This voice speaks Spanish translations.

```bash
curl -L -o downloads/voice-es.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-es_ES-carlfm-x_low.tar.bz2
tar -xjf downloads/voice-es.tar.bz2 -C target/release/models/tts
```

When all six are done, you can delete the `downloads` folder. Nothing uses it any more.

### Step 7: Check that everything is in place

```bash
./target/release/cnverc.exe --report
```

This lists every model cnverc can find, with the full path of each file it looked for.

- Every file should have a **`[+]`** next to it. A **`[!]`** means that file is missing.
- Every recognizer and voice should say **`ok`**. **`DISABLED - missing: ...`** names the file
  it couldn't find.

To fix a missing file, put it at the path shown, spelled exactly the same way, and run the
report again.

### Step 8: Run it

Double-click `target/release/cnverc.exe` in File Explorer, or run it from Git Bash:

```bash
./target/release/cnverc.exe
```

The first time it opens, choose your microphone and speakers in the window. Your choices are
saved in `cnverc.toml`, next to the exe, so you only do this once.

---

## Using cnverc

On the left side of the window you choose:

- **Languages:** who is speaking which language. Swap them to translate the other way.
- **Recognizer:** Whisper or Parakeet. Try both and keep whichever hears your voice better.
- **Microphone and output:** which devices to use.
- **Mode:**
  - **Take turns:** press **Space** to start talking and again when you've finished. A large
    banner shows **READY**, **RECORDING** or **PROCESSING**. If you prefer, choose **Hold
    Space while speaking** instead: hold the key down while you talk and let go when you're
    done.
  - **Listen continuously:** cnverc listens all the time and translates each time you pause.
- **Speak translations:** untick this to see captions only, with no voice.
- **Mute the microphone while speaking:** leave this on when using speakers, so cnverc doesn't
  hear its own voice and translate it again. Turn it off only if you're wearing headphones.

A second black window opens behind the main one. It's the log, and you can ignore it.

### Command-line options

These are optional and mostly useful for troubleshooting:

```bash
cnverc --report              # which models are installed, and where it looked
cnverc --devices             # which microphones and speakers it can see
cnverc --listen              # listen and translate in the terminal, no window
cnverc --listen --seconds 20 # the same, stopping after 20 seconds
```

---

## Moving it to a PC with no internet

Only the PC you build on needs internet. To run cnverc on any other Windows PC, copy these from
`target/release/`:

```
cnverc.exe
cnverc.toml
models/
```

Put them in one folder anywhere, for example on a USB stick, the desktop or another drive,
and double-click `cnverc.exe`. Nothing needs installing on that PC.

---

## The models

Every model lives in its own folder under `models/`, keeping the name it was published with.
To see what a file is, look at the folder it's in.

| Job | Model used | Folder | Where it comes from |
|---|---|---|---|
| Notice speech | Silero VAD | `models/vad/` | [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) (`silero_vad.onnx`), or [silero-vad](https://github.com/snakers4/silero-vad) |
| Speech → text | Whisper large-v3-turbo (int8) | `models/asr/whisper-large-v3-turbo/` | [sherpa-onnx ASR releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) (`sherpa-onnx-whisper-turbo.tar.bz2`) |
| Speech → text | NVIDIA Parakeet TDT 0.6B v3 (int8) | `models/asr/parakeet-tdt-0.6b-v3-int8/` | [sherpa-onnx ASR releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) (`sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2`) |
| Translate | Qwen3 1.7B (Q4_K_M) | `models/mt/` | [unsloth/Qwen3-1.7B-GGUF](https://huggingface.co/unsloth/Qwen3-1.7B-GGUF) (`Qwen3-1.7B-Q4_K_M.gguf`) |
| Speak English | Piper en_US lessac (medium) | `models/tts/vits-piper-en_US-lessac-medium/` | [sherpa-onnx TTS releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) (`vits-piper-en_US-lessac-medium.tar.bz2`) |
| Speak Spanish | Piper es_ES carlfm (x_low) | `models/tts/vits-piper-es_ES-carlfm-x_low/` | [sherpa-onnx TTS releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) (`vits-piper-es_ES-carlfm-x_low.tar.bz2`) |

Each recognizer and voice folder has a small `engine.toml` that comes with the code. It tells
cnverc what the model is and which files belong to it. The model files themselves are
downloaded separately and are never stored in the repository.

**Swapping models:**

- **Translator:** `models/mt/` must hold exactly **one** `.gguf` file, and it must be a
  **Qwen3** model. To try a different size, move the old file out and put the new one in.
- **Voices:** cnverc picks the voice that matches the language it's translating *into*. To
  speak another language, add a Piper voice for it, with its own folder and `engine.toml`.
- **Anything else:** a folder with no `engine.toml` is skipped. If a file named in the
  `engine.toml` is missing, that model shows as disabled in the report, with the missing
  file's name.

---

## Connecting two PCs (paired mode, coming in Milestone 7)

Paired mode isn't built yet. These notes are here so you can plan the hardware.

Two machines each run their own complete pipeline. **Only text crosses the wire**, never audio
and never models. Laptop A captures Spanish, transcribes and translates it locally, and sends
the English text; laptop B displays it and speaks it in its own voice. An utterance costs a few
hundred bytes, so even a terrible link works.

Any transport that appears to the OS as an IP interface works, and the socket code is the same
for all of them:

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
link-local addressing (169.254.x.x) after about thirty seconds. That works, but the addresses
are ugly and can change between sessions. For a rig you use repeatedly, set static IPs on that
interface once, for example 192.168.50.1 and 192.168.50.2.

**Firewall.** A network Windows can't identify is often classified as **Public**, which blocks
inbound connections: the listener binds, the peer connects to nothing, and no useful error
appears. Set that interface's network profile to Private. From Milestone 7, cnverc detects the
bound-but-never-accepted state and says so specifically, rather than showing a generic
timeout.

---

## Licence

Unpublished. Model files come with their own licences; check each one before you redistribute
it.
