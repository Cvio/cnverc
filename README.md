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
models. The finished folder doesn't need internet. These steps are for Windows 10 or 11; for
Linux, see [Setting up on Linux](#setting-up-on-linux) after them.

Allow about an hour, most of it spent waiting for downloads and the first build.

### Step 1: Install Git

Download **Git for Windows** from <https://git-scm.com/download/win> and install it with the
default options. It also installs **Git Bash**, the terminal every command below is typed into.

✅ **Check:** open the Start menu, type `Git Bash`, and open it. A black window with a `$`
prompt appears.

### Step 2: Install the C++ build tools and LLVM

cnverc is written in Rust, and parts of it are compiled with Microsoft's C++ compiler. The
translator's build also needs **LLVM**, which reads its C++ headers.

1. Download **Build Tools for Visual Studio** from
   <https://visualstudio.microsoft.com/downloads/>. It's under "Tools for Visual Studio", and
   it's free.
2. Run the installer. When it asks what to install, tick **Desktop development with C++**.
3. In the list on the right, make sure **C++ CMake tools for Windows** is also ticked.
4. Click **Install** and wait. It's several gigabytes.
5. Download LLVM from <https://github.com/llvm/llvm-project/releases/latest>: the file named
   `LLVM-<version>-win64.exe`. Run it, and when it asks, choose **Add LLVM to the system PATH
   for all users**. Keep the default install folder, `C:\Program Files\LLVM`.

Do this step before step 3: the Rust installer looks for these tools.

✅ **Check:** the folder `C:\Program Files\LLVM\bin` exists and contains `libclang.dll`.

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

> **If the build fails mentioning "libclang" or "clang":** LLVM from step 2 is missing, or
> was installed somewhere else. Install it, or tell the build where it is, then rebuild:
>
> ```bash
> export LIBCLANG_PATH="/c/Program Files/LLVM/bin"
> ```

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

### Setting up on Linux

The steps are the same as for Windows, with different tools. These are for Ubuntu or Debian;
the package names on other distributions are similar. Linux support is new, so if a step
fails, note exactly what it printed.

1. **Install the tools** in a terminal:

   ```bash
   sudo apt install git curl build-essential cmake pkg-config libclang-dev libasound2-dev
   ```

   On Fedora the equivalent is
   `sudo dnf install git curl gcc-c++ cmake pkgconf clang-devel alsa-lib-devel`.

2. **Install Rust** with the command from <https://rustup.rs>, accepting the defaults, then
   close the terminal and open a new one:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. **Build the speech library.** On Windows the build downloads it ready-made. On Linux the
   ready-made one crashes as soon as it loads a model (`free(): invalid pointer`), so build it
   yourself, as a shared library using Ubuntu's own onnxruntime. Do this once, next to the
   cnverc folder. The version must be exactly `v1.13.8`:

   ```bash
   sudo apt install libonnxruntime-dev
   ```

   ```bash
   git clone https://github.com/k2-fsa/sherpa-onnx && cd sherpa-onnx && git checkout v1.13.8
   ```

   ```bash
   cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON -DSHERPA_ONNX_ENABLE_PYTHON=OFF -DSHERPA_ONNX_ENABLE_TESTS=OFF -DCMAKE_INSTALL_PREFIX="$HOME/sherpa-onnx/install"
   ```

   ```bash
   cmake --build build -j2 --target install
   ```

   Check that the configure step printed `location_onnxruntime_lib: /usr/lib/...`. If it says
   `Downloading pre-compiled onnxruntime` instead, `libonnxruntime-dev` isn't installed.
   `-j2` keeps the build within about 8 GB of memory; a larger number can run out.

4. **Get the code and build it**, as in steps 4 and 5 above, but with no `export PATH` line.
   Tell the build where the speech library is, in every new terminal you build from:

   ```bash
   export SHERPA_ONNX_LIB_DIR="$HOME/sherpa-onnx/install/lib"
   ```

   ```bash
   cargo build --release -j2
   ```

   The build copies `libsherpa-onnx-c-api.so` into `target/release/` next to `cnverc`, which
   finds it there. It also needs Ubuntu's `libonnxruntime` package, so a Linux build isn't
   copy-to-run like the Windows one. If `lib` in that folder contains a `libonnxruntime.a`
   (left over from building sherpa-onnx without `BUILD_SHARED_LIBS=ON`), move it out;
   otherwise the build links it again, and the crash comes back.

5. **Download the models** with exactly the commands in step 6. They work unchanged.

6. **Check and run** as in steps 7 and 8. The program is `cnverc`, with no `.exe`:

   ```bash
   ./target/release/cnverc --report
   ```

   ```bash
   ./target/release/cnverc
   ```

cnverc needs a desktop session to open its window. On Linux, the window lists microphones and
speakers by their ALSA names; if you're unsure which to pick, "System default" is usually right.

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
- **Pair with another PC:** two people, two PCs, one conversation. See
  [Talking between two PCs](#talking-between-two-pcs) below.

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

## Talking between two PCs

With paired mode, each person has their own PC, and each PC translates what its own person
says. It sends the other PC **only the translated text**, which the other PC shows and says
aloud in its own voice. No sound is sent, so even a slow or poor connection works, and neither
PC ever needs the internet.

### Setting it up

1. **Set each PC's languages for its own person.** If Ana speaks Spanish and Ben speaks
   English:
   - Ana's PC: Speaker's language **Spanish**, Translate into **English**.
   - Ben's PC: Speaker's language **English**, Translate into **Spanish**.

   Each PC needs the voice for the language its person *hears*: Ana's needs the Spanish voice,
   Ben's the English one.

2. **Connect the two PCs to the same network.** Any of the options in the table below works,
   including a plain Ethernet cable between them.

3. **On both PCs,** tick **Pair with another PC** and press **Start**.

   The first time, Windows asks whether to allow cnverc through the firewall. Tick **Private
   networks** and click **Allow**. On Linux with a firewall turned on, allow cnverc's ports
   once:

   ```bash
   sudo ufw allow 47800/tcp
   ```

   ```bash
   sudo ufw allow 47801/udp
   ```

4. **Connect them.** Under **This PC**, each PC shows its name and address, like
   `192.168.50.2`. On **either** PC, type the other PC's address into **Other PC's address**
   and press **Connect**. If the other PC appears under **Found on this network**, you can
   click it instead of typing.

   ✅ **Check:** both PCs show **Paired with** and the other PC's name, at the top of the
   window.

### Talking

- **Take turns** (the usual way): press **Space** to talk, as on one PC. Only one person can
  talk at a time. Your banner shows **ASKING FOR THE FLOOR…** for a moment, then
  **RECORDING**. The other PC shows your PC's name followed by **IS TALKING**, and its Space
  key won't start a turn until you've finished. When you press Space again, your
  words appear on the other PC in their language and are spoken there.

  Because only the person talking has a live microphone, you can both use speakers.

- **Listen continuously:** both microphones listen all the time. **Both people must wear
  headsets**, or each PC hears the other's speakers and translates it back, round and round.
  cnverc shows a red warning for as long as this combination is on.

If the connection drops, for example because a cable is pulled, both PCs show **Not paired**
within a few seconds, and whoever was talking loses the floor. Press **Connect** again once
the connection is back.

To change the name the other PC sees, set `display_name` under `[peer]` in `cnverc.toml`.

### If it won't connect

cnverc says what went wrong under **Not connected** in the pairing panel:

- **"Nothing is listening at …"**: the other PC isn't ready. Check that it has **Pair with
  another PC** ticked and **Start** pressed, and that the address was typed exactly.
- **"No answer from … within 4 s"**: something between the PCs is silently dropping the
  connection, almost always a firewall. See **Firewall** below.
- **"There is no route to …"**: the PCs aren't on the same network, or the address is wrong.

### Networks that work

Any connection that gives each PC a network address works, and cnverc treats them all the
same way:

| Connection | Works | Notes |
|---|---|---|
| Unmanaged Ethernet switch | Yes | No router or internet needed |
| Router with its internet cable unplugged | Yes | Hands out addresses automatically, which is convenient |
| Ethernet cable from one laptop to the other | Yes | Any ordinary cable |
| Wi-Fi hotspot from one laptop | Yes | No internet needed |
| Existing Wi-Fi network | Yes | A guest network usually stops devices seeing each other |
| Thunderbolt / USB4 networking | Yes | The fastest option |
| USB bridge or transfer cable | Yes | Appears as a network adapter on each side |
| Two USB-C-to-Ethernet adapters | Yes | Ordinary cable between them |
| **Plain USB-C cable between two laptops** | **No** | Both ends are USB hosts; there is no network |

**Addresses.** With a plain cable or a switch, there's no router to hand out addresses, so
after about thirty seconds Windows picks one starting `169.254.`. cnverc marks these "(no
router)". They work, but they can change each time. If you use the same two PCs often, give
that network adapter a fixed address on each, for example `192.168.50.1` and `192.168.50.2`.

**Firewall.** Windows often calls a network it can't identify, like a plain cable or a switch
with no router, **Public**, and blocks incoming connections on it. cnverc can listen, but
nothing reaches it, and the other PC just waits. cnverc recognises this and says so. The fix is
on the PC that isn't being reached: open **Settings › Network & internet**, choose that
network's adapter, and set **Network profile type** to **Private**.

---

## Licence

Unpublished. Model files come with their own licences; check each one before you redistribute
it.
