# cnverc

cnverc is a live interpreter for two people who don't share a language. One person speaks,
for example in Spanish. cnverc writes down what they said, translates it into English, shows
both on screen, and says the English out loud. It works the other way round too.


Everything happens on your own computer. cnverc **never uses the internet**: it doesn't
download anything, check for updates or send anything anywhere. Once it's set up, you can
unplug the network and it keeps working.

| You want to… | Go to |
|---|---|
| Set it up on Windows | [Set up on Windows](#set-up-on-windows) |
| Set it up on Linux | [Set up on Linux](#set-up-on-linux) |
| Learn the four ways of using it | [Using cnverc](#using-cnverc) |
| Find the right setup for your situation | [USE-CASES.md](USE-CASES.md) |
| Fix something | [If something goes wrong](#if-something-goes-wrong) |
| Understand or change the code | [ARCHITECTURE.md](ARCHITECTURE.md) |

---

## What you need

- **A PC** running Windows 10 or 11, or Ubuntu 25.04 or later.
- **8 GB of memory**, or 16 GB to use Shared machine mode with the best recognizer.
- **About 12 GB of free disk space.**
- **A microphone and speakers,** or a headset.
- **Internet, during setup only:** about 2.5 GB of downloads, plus the tools.
- **About an hour** for the first setup, mostly waiting. On Linux, add an hour.

You'll type a few commands into a terminal. Every command is given in full: copy it, paste it
and press Enter. Each step ends with a ✅ **check**, so you know it worked before moving on.

---

## Set up on Windows

### 1. Install Git

Download **Git for Windows** from <https://git-scm.com/download/win> and run the installer,
keeping every default option. It also installs **Git Bash**, the terminal every command below
is typed into.

✅ **Check:** open the Start menu, type `Git Bash`, and open it. A window with a `$` prompt
appears. Keep it open.

### 2. Install the C++ build tools

1. Download **Build Tools for Visual Studio** from
   <https://visualstudio.microsoft.com/downloads/>. Scroll to **Tools for Visual Studio**;
   it's free.
2. Run it. When it asks what to install, tick **Desktop development with C++**.
3. In the **Installation details** list on the right, make sure **C++ CMake tools for
   Windows** is ticked too.
4. Click **Install**. It downloads several gigabytes; let it finish.

### 3. Install LLVM

1. Open <https://github.com/llvm/llvm-project/releases/latest>. Under **Assets**, download the
   file named `LLVM-<version>-win64.exe`.
2. Run it. When it asks, choose **Add LLVM to the system PATH for all users**. Keep the
   default folder.

### 4. Install Rust

Download `rustup-init.exe` from <https://rustup.rs> and run it. When it asks, press **Enter**
to accept the default installation.

Then **close Git Bash and open it again**, so it finds everything you just installed.

### 5. Get cnverc

Put cnverc in a folder with a **short path**, such as `C:\cnverc`. Long folder paths break the
build, and the check in the next step refuses them. In Git Bash:

```bash
cd /c/
```

```bash
git clone https://github.com/Cvio/cnverc.git
```

```bash
cd cnverc
```

Every command from now on is run from inside this `cnverc` folder. (If you close Git Bash,
open it again and type `cd /c/cnverc` first.)

### 6. Check your tools

```bash
./check-setup.sh
```

✅ **Check:** every line says `OK`, and it ends with `Everything cnverc needs to build is
here.` If a line says `MISSING`, do what the line below it says, then run the check again.

### 7. Build cnverc and download its models

```bash
./setup.sh
```

This builds cnverc (10–20 minutes the first time) and downloads its models (about 2.4 GB). It
checks everything as it goes, and stops with a plain message if something needs doing. If it
stops partway, for example because the internet dropped, just run it again: it picks up where
it left off.

✅ **Check:** it ends with `Everything is in place. Start cnverc with:`

### 8. Start cnverc

```bash
./target/release/cnverc.exe
```

Or double-click `cnverc.exe` in the `C:\cnverc\target\release` folder. You can make a desktop
shortcut to it: right-click it, then **Send to › Desktop**.

A second black window opens behind the main one. It's the log, and you can ignore it.

Now go to [First run](#first-run).

---

## Set up on Linux

These steps are tested on **Ubuntu 26.04**. Any Ubuntu from 25.04 on should work, because it
needs Ubuntu's `libonnxruntime-dev` package. On other distributions, see
[TECHNICAL.md → Linux: shared sherpa-onnx](TECHNICAL.md#linux-shared-sherpa-onnx) first.

### 1. Install the tools

Open a terminal and run:

```bash
sudo apt install git curl build-essential cmake pkg-config libclang-dev libasound2-dev libonnxruntime-dev
```

### 2. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Press **Enter** to accept the defaults. Then **close the terminal and open a new one.**

### 3. Get cnverc

```bash
cd ~ && git clone https://github.com/Cvio/cnverc.git && cd cnverc
```

Every command from now on is run from inside this `cnverc` folder. (In a new terminal, type
`cd ~/cnverc` first.)

### 4. Build the speech library (once, about an hour)

On Linux, one of cnverc's libraries has to be built from source. This script does it and
checks the result:

```bash
./build-sherpa-linux.sh
```

✅ **Check:** it ends with `The speech library is built`. Running it again later just says
it's already done.

### 5. Check your tools

```bash
./check-setup.sh
```

✅ **Check:** every line says `OK`. If a line says `MISSING`, do what the line below it says.

### 6. Build cnverc and download its models

```bash
./setup.sh
```

✅ **Check:** it ends with `Everything is in place. Start cnverc with:`. If it stops partway,
run it again.

### 7. Start cnverc

```bash
./target/release/cnverc
```

cnverc needs a desktop session to open its window. Now go to [First run](#first-run).

---

## First run

1. **Choose your microphone and output** in the settings on the left. "System default" is
   usually right. If you're unsure which is which, close cnverc and run
   `./target/release/cnverc.exe --devices` (Linux: `./target/release/cnverc --devices`) to
   list them.
2. **Choose the languages.** *Speaker's language* is what the person talking speaks;
   *Translate into* is what the listener hears.
3. **Choose a recognizer.** *Whisper large-v3-turbo* is the safe choice, and the only one that
   works in every mode.
4. Press **Start**, and wait for the loading to finish (up to a minute the first time).
5. **Try it:** press **Space**, say "Good morning, how are you?", and press **Space** again.
   You should see the sentence, its translation, and hear it spoken.

Your choices are saved next to the program (in `cnverc.toml`) as you make them, so you only do
this once.

---

## Using cnverc

cnverc has four modes. Choose one under **Mode** in the settings. For which to use when, see
[USE-CASES.md](USE-CASES.md).

### Take turns: one PC, one person talking at a time

Press **Space** to start talking and **Space** again when you've finished. The large banner
shows what's happening: **READY** (grey-blue), **RECORDING** (red), **PROCESSING** (amber) and
**SPEAKING**. Prefer holding the key down while you talk? Choose **Hold Space while
speaking**.

### Listen continuously: one PC, no keys

cnverc listens all the time and translates each time you pause. With speakers, keep **Mute
the microphone while speaking** ticked, so cnverc doesn't translate its own voice.

### Shared machine: two people, one PC


Two people sit at one PC, each with their own key: **Left arrow** for the person on the left,
**Right arrow** for the person on the right.

1. Choose **Shared machine**, and the **Whisper** recognizer. Parakeet can't be used here: it
   can't be told which language to expect.
2. Above the two columns, set each person's language, and the voice their words are spoken in.
3. Press **Start**. To talk, press **your** key, speak, and press it again.

Only one person talks at a time. While one is talking, the other key does nothing, and while
the PC is speaking, neither does. The columns say why. **Escape** cancels a turn: nothing from
it is spoken.

### Paired: two PCs, one conversation


Each person has their own PC. Each PC translates what its own person says and sends the other
PC only the text, which the other PC shows and speaks. It works over Wi-Fi, a router with no
internet, or a single Ethernet cable, and never uses the internet.

1. On each PC, set the languages **for that PC's person**. On the Spanish speaker's: *Spanish*
   into *English*. On the English speaker's: *English* into *Spanish*.
2. On both, tick **Pair with another PC** and press **Start**. The first time, Windows asks
   about the firewall: tick **Private networks** and click **Allow**.
3. On either PC, type the address shown under **This PC** on the other one, or click it under
   **Found on this network**, and press **Connect**.

✅ **Check:** both PCs show **Paired with** and the other PC's name.

Then press **Space** to talk, as in Take turns. Only the person talking has a live microphone,
so both PCs can use speakers. With **Listen continuously** instead, both people **must wear
headsets**, or the two PCs translate each other in a loop.

If it won't connect, the pairing panel says why. See
[If something goes wrong](#if-something-goes-wrong).

### Other settings

- **Speak translations:** untick this to see captions only.
- **Mute the microphone while speaking:** leave it on with speakers. Turn it off only with a
  headset.
- **Compare recognizers:** runs every installed recognizer on what you say, side by side, to
  find the one that hears you best. Nothing is translated while it's on.

---

## Updating

In Git Bash (Windows) or a terminal (Linux), from the `cnverc` folder:

```bash
git pull
```

```bash
./setup.sh
```

Your settings are kept: `setup.sh` never overwrites your `cnverc.toml`. Close cnverc before
updating.

---

## Moving it to a PC with no internet (Windows)

Only the PC you set up on needs internet. To run cnverc on another Windows PC, copy these from
`C:\cnverc\target\release\` into one folder, on a USB stick for example:

```
cnverc.exe
cnverc.toml
models\
```

On the other PC, put that folder anywhere and double-click `cnverc.exe`. Nothing needs
installing.

On Linux, this doesn't work yet: each Linux PC needs the setup done on it.

---

## If something goes wrong

**First, run the check:** `./check-setup.sh` finds most setup problems and says how to fix
them.

| What you see | What to do |
|---|---|
| `check-setup.sh` says `MISSING` | Do what the line under it says, then run the check again |
| "This folder's path is too long for the build" | Move the `cnverc` folder somewhere short, such as `C:\cnverc`, and run `./setup.sh` there |
| `setup.sh` says cnverc is running | Close the cnverc window, then run `./setup.sh` again |
| A download failed | Run `./setup.sh` again; it retries only what's missing |
| `Access is denied (os error 5)` while building | cnverc is still running. Close it and build again |
| The build fails mentioning `libclang` or `clang` | Install LLVM (Windows step 3) and open a new Git Bash |
| The window doesn't open | Run cnverc from Git Bash and read the last lines it prints. Include them if you ask for help |
| **Linux:** `free(): invalid pointer` when starting | The speech library was built wrongly. Run `./build-sherpa-linux.sh --rebuild`, then `./setup.sh` |
| **Linux:** `ALSA lib … Unknown PCM` lines | Harmless; ignore them. If no microphone is listed at all: `sudo apt install libasound2-plugins` |
| No microphone or speakers listed | Plug them in and click **Rescan models and devices** |
| The level meter doesn't move when you talk | The wrong microphone is chosen, or it's muted in Windows' sound settings |
| "Nothing recognised in … ms of speech" | The microphone heard sound but no words. Speak closer, or choose another microphone |
| The first word goes missing | Pause for a moment after pressing the key, before you speak |
| cnverc translates its own voice | Tick **Mute the microphone while speaking**, or use a headset |
| "No voice installed for …" | Run `./setup.sh` again; it downloads missing voices |
| "Not translated: …" | cnverc refused a translation it didn't trust. Say it again, more simply |
| Shared machine: a key does nothing | Read that person's column: it says why. If blank, click outside any text box |
| Shared machine: a column says it "cannot hear this side" | Choose the **Whisper large-v3-turbo** recognizer |
| Paired: "Nothing is listening at …" | On the other PC, tick **Pair with another PC**, press **Start**, and check the address |
| Paired: "No answer from … within 4 s" | A firewall is blocking. On the PC not being reached: **Settings › Network & internet**, choose the network, set **Network profile type** to **Private** |
| Paired: "There is no route to …" | The PCs aren't on the same network, or the address is mistyped |
| A sentence comes out wrong | See [USE-CASES.md → Why was that wrong?](USE-CASES.md#8-which-recognizer-hears-me-best-and-why-was-that-wrong) |

Asking for help? Include what you did, what you saw, and the newest file in
`target/release/logs/`.

---

## Words used in cnverc

| Word | Meaning |
|---|---|
| **Recognizer** | The model that turns speech into text. cnverc comes with two: Whisper and Parakeet |
| **Translator** | The model that translates the text (Qwen3) |
| **Voice** | The model that speaks the translation aloud (Piper voices) |
| **Turn** | One stretch of talking, from pressing your key to pressing it again |
| **Floor** | In paired mode, the right to talk. Only one PC has it at a time |
| **Mute the microphone while speaking** | Also called half-duplex: the microphone is ignored while cnverc speaks, so it doesn't hear itself |
| **`cnverc.toml`** | The settings file next to the program. The window saves it for you |
| **`models/`** | The folder next to the program that holds every model, one folder each |
| **`engine.toml`** | A small file in each model's folder saying what the model is and which files it uses |
| **`--report`** | `cnverc --report` lists every model cnverc found, and anything missing |

---

## The models

`setup.sh` downloads all of these. The table is for when you want to fetch or replace one by
hand.

| Job | Model | Folder | Where it comes from |
|---|---|---|---|
| Notice speech | Silero VAD | `models/vad/` | [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) (`silero_vad.onnx`) |
| Speech → text | Whisper large-v3-turbo (int8) | `models/asr/whisper-large-v3-turbo/` | [sherpa-onnx ASR releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) (`sherpa-onnx-whisper-turbo.tar.bz2`) |
| Speech → text | NVIDIA Parakeet TDT 0.6B v3 (int8) | `models/asr/parakeet-tdt-0.6b-v3-int8/` | [sherpa-onnx ASR releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) (`sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2`) |
| Translate | Qwen3 1.7B (Q4_K_M) | `models/mt/` | [unsloth/Qwen3-1.7B-GGUF](https://huggingface.co/unsloth/Qwen3-1.7B-GGUF) (`Qwen3-1.7B-Q4_K_M.gguf`) |
| Speak English | Piper en_US lessac (medium) | `models/tts/vits-piper-en_US-lessac-medium/` | [sherpa-onnx TTS releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) |
| Speak Spanish | Piper es_ES carlfm (x_low) | `models/tts/vits-piper-es_ES-carlfm-x_low/` | [sherpa-onnx TTS releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) |
| Speak Spanish (Mexico) | Piper es_MX claude (high) | `models/tts/vits-piper-es_MX-claude-high/` | [sherpa-onnx TTS releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) |

- **Translator:** `models/mt/` must hold exactly **one** `.gguf` file, and it must be a
  **Qwen3** model.
- **Voices:** for a new language, add a Piper voice in its own folder with an `engine.toml`;
  see [ARCHITECTURE.md → Add a voice](ARCHITECTURE.md#add-a-voice).
- **Your own Whisper model:** the separate
  [model-converter](https://github.com/Cvio/model-converter) project converts one from Hugging
  Face.

Model files come with their own licences; check each one before you redistribute it.

---

## Other documents

| File | What's in it |
|---|---|
| [USE-CASES.md](USE-CASES.md) | Situations (a front desk, two PCs, no internet…) and how to run each |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the code fits together, for developers |
| [TECHNICAL.md](TECHNICAL.md) | Why things are built the way they are: design decisions and build details |
| [SPEC.md](SPEC.md) | The original specification, and the milestones |
| [DIALECTS.md](DIALECTS.md), [lora.md](lora.md) | Tuning the models for a regional dialect |
| [HANDOFF.md](HANDOFF.md) | Where the project stands, for whoever works on it next |

## Licence

Unpublished.
