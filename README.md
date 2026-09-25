# cnverc

cnverc is a live interpreter for two people who don't share a language. One person speaks
Spanish into the microphone. cnverc writes down what they said, translates it into English,
shows both on screen, and says the English out loud. It works the other way round too.

Everything happens on your own computer. cnverc **never uses the internet**: it doesn't
download anything, check for updates or send anything anywhere. Once it's set up you can
unplug the network and it keeps working.

> On **Windows**, you can also copy the finished folder to a PC that has never been online and
> it will run there. On **Linux** you can't yet: the Linux build uses a system package that
> has to be installed on each PC. See [Setting up on Linux](#setting-up-on-linux).

Inside, it runs five steps in order:

1. **Listen.** Wait for someone to start talking, and cut the recording off when they stop.
2. **Transcribe.** Turn the speech into text.
3. **Translate.** Translate that text into the other language.
4. **Speak.** Read the translation aloud.
5. **Show.** Put both sentences on screen as captions.

Each step uses a model file that you download once during setup.

### Where to go next

| You want to… | Go to |
|---|---|
| Use a copy someone already set up | [Using cnverc](#using-cnverc) |
| Use two PCs together | [Talking between two PCs](#talking-between-two-pcs) |
| Set it up on a new Windows PC | [Setting up](#setting-up-on-a-new-pc) |
| Set it up on a new Linux PC | [Setting up on Linux](#setting-up-on-linux) |
| Fix something that went wrong | [If something goes wrong](#if-something-goes-wrong) |
| Understand how it works inside | [TECHNICAL.md](TECHNICAL.md) |

---

## Using cnverc

Double-click `cnverc.exe` (on Linux, `cnverc`). The first time, choose your microphone and
speakers in the window; your choices are saved next to the program, so you only do it once.

A second black window opens behind the main one. It's the log, and you can ignore it.

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
  - **Shared machine:** two people, one PC, a key each. See
    [Two people, one PC](#two-people-one-pc).
- **Speak translations:** untick this to see captions only, with no voice.
- **Mute the microphone while speaking:** leave this on when using speakers, so cnverc doesn't
  hear its own voice and translate it again. Turn it off only if you're wearing headphones.
- **Pair with another PC:** two people, two PCs, one conversation. See
  [Talking between two PCs](#talking-between-two-pcs).

### Command-line options

Optional, and mostly for troubleshooting:

```bash
cnverc --report              # which models are installed, and where it looked
cnverc --devices             # which microphones and speakers it can see
cnverc --listen              # listen and translate in the terminal, no window
cnverc --listen --seconds 20 # the same, stopping after 20 seconds
cnverc --listen --wav        # also save each utterance heard, into logs/segments/
cnverc --listen --compare    # run every installed recognizer and show them side by side
```

`--wav` is the quickest way to find out why a sentence came out wrong: play back what cnverc
actually recorded. `--compare` tells you which recognizer is better **on your voice**, which
is not the same as which one scores better in published tests.

---

## Moving it to a PC with no internet

**Windows only** — see the note at the top of this file for why.

Only the PC you build on needs internet. To run cnverc on another Windows PC, copy these three
things from `target/release/`:

```
cnverc.exe
cnverc.toml
models/
```

Put them in one folder anywhere — a USB stick, the desktop, another drive — and double-click
`cnverc.exe`. Nothing needs installing on that PC.

---

## Setting up on a new PC

The setup has to be done on a PC **with** internet, because you download the tools and the
models. These steps are for Windows 10 or 11; for Linux, see
[Setting up on Linux](#setting-up-on-linux).

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

It should print `cargo 1.95` or a later version. If the number is lower, run `rustup update`.

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
CMake is. **You need this line every time you open a new Git Bash window to build:**

```bash
export PATH="$PATH:$(dirname "$("/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe" -latest -find '**/CMake/bin/cmake.exe' | head -1)")"
```

That finds CMake whichever version of the Build Tools you have. If it prints nothing or the
build still can't find CMake, set the path by hand instead — look inside
`C:\Program Files (x86)\Microsoft Visual Studio\` to see which version number you have, and
use it in place of the `18` here:

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

### Step 6: Download the models

cnverc looks for everything in the folder `cnverc.exe` is in. First copy the settings file and
the empty model folders there:

```bash
cp -r models cnverc.toml target/release/
```

Then run the download script. It fetches all six models, about 2.3 GB, and skips anything you
already have — so if it stops partway, just run it again.

```bash
./setup-models.sh
```

✅ **Check:** it ends with `All six downloaded into target/release/models`.

Each model and where it comes from is listed under [The models](#the-models) below, if you
ever need to fetch one by hand.

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

```bash
./target/release/cnverc.exe
```

Then see [Using cnverc](#using-cnverc).

---

## Setting up on Linux

The same steps as Windows, with different tools, plus one extra: on Linux you build the speech
library yourself, which adds about an hour. These commands are for Ubuntu or Debian; package
names on other distributions are similar.

These steps have been followed on **Ubuntu 26.04**, and that is the only Linux they have been
tried on. Step 3 needs a distribution that packages onnxruntime (Ubuntu 25.04 and later do),
and only Ubuntu's version 1.23 has been tested. Linux support is new, so if a step fails, note
exactly what it printed.

**1. Install the tools.**

```bash
sudo apt install git curl build-essential cmake pkg-config libclang-dev libasound2-dev libonnxruntime-dev
```

On Fedora: `sudo dnf install git curl gcc-c++ cmake pkgconf clang-devel alsa-lib-devel`. Note
that Fedora has no onnxruntime package; see
[TECHNICAL.md → Linux: shared sherpa-onnx](TECHNICAL.md#linux-shared-sherpa-onnx) before you
start.

**2. Install Rust** with the command from <https://rustup.rs>, accepting the defaults, then
close the terminal and open a new one:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**3. Build the speech library.** On Windows the build downloads this ready-made. The ready-made
Linux one crashes as soon as it loads a model, so build it yourself. Do this once, anywhere;
the version must be exactly `v1.13.8`.

```bash
git clone https://github.com/k2-fsa/sherpa-onnx && cd sherpa-onnx && git checkout v1.13.8
```

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON -DSHERPA_ONNX_ENABLE_PYTHON=OFF -DSHERPA_ONNX_ENABLE_TESTS=OFF -DCMAKE_INSTALL_PREFIX="$HOME/sherpa-onnx/install"
```

✅ **Check:** that command printed `location_onnxruntime_lib: /usr/lib/...`. If it said
`Downloading pre-compiled onnxruntime` instead, stop — `libonnxruntime-dev` isn't installed,
and continuing gives you a build that crashes later.

```bash
cmake --build build -j2 --target install
```

`-j2` matters: a fully parallel build can run a 10 GB PC out of memory.

**4. Get the code and build it**, as in steps 4 and 5 above but with no `export PATH` line.
Tell the build where the speech library is, in every new terminal you build from:

```bash
export SHERPA_ONNX_LIB_DIR="$HOME/sherpa-onnx/install/lib"
```

```bash
cargo build --release -j2
```

**5. Download the models**, the same as step 6 above:

```bash
cp -r models cnverc.toml target/release/ && ./setup-models.sh
```

**6. Check and run**, as in steps 7 and 8. The program is `cnverc`, with no `.exe`:

```bash
./target/release/cnverc --report
```

```bash
./target/release/cnverc
```

cnverc needs a desktop session to open its window. The window lists microphones and speakers by
their ALSA names; if you're unsure which to pick, "System default" is usually right.

**If a model load ends in `free(): invalid pointer`,** the build picked up the wrong
onnxruntime. The cause and the fix are in
[TECHNICAL.md → Linux: shared sherpa-onnx](TECHNICAL.md#linux-shared-sherpa-onnx).

---

## Two people, one PC

With **Shared machine** mode, two people who speak different languages sit at the same PC, one
on the left and one on the right. Each has their own key: **Left arrow** for the person on the
left, **Right arrow** for the person on the right.

### Setting it up

1. Under **Mode**, choose **Shared machine**. (It's greyed out while **Pair with another PC**
   is ticked: untick that first.)
2. Choose a **Whisper** recognizer. Shared mode tells the recognizer which language to expect,
   and only Whisper takes that instruction; Parakeet decides the language itself, so the
   columns say so and won't start a turn.
3. Above the two columns, choose each person's language. Under each, choose the voice their
   words are spoken in. That's a voice in the *other* person's language, so the list only shows
   those. Leave it on "The first installed voice" if you only have one.
4. Press **Start**.

### Talking

- Press **your** key, speak, and press it again when you've finished. Your column turns red
  while it listens, amber while it works, and blue while it speaks your words in the other
  person's language.
- Only one person at a time. While you're talking, the other key does nothing. While the PC is
  working or speaking, neither key does, and the microphone is off, so it never translates its
  own voice. The other column says **Speaking — wait** or **Wait — the other person has the
  turn**, so a key that does nothing doesn't look broken.
- **Escape** cancels: it throws away a turn you're recording, or stops a translation being
  worked on or spoken. Nothing from a cancelled turn is spoken.
- Each column keeps that person's history: what they said, and what was spoken for them.

The keys only work while no text box has the cursor. If they seem dead, click anywhere outside
a text box. To use different keys, for a foot pedal for example, set `left_key` and `right_key`
under `[shared]` in `cnverc.toml`.

Keep the speakers at a normal volume; you don't need headphones in this mode.

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
  key won't start a turn until you've finished. When you press Space again, your words appear
  on the other PC in their language and are spoken there.

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

## If something goes wrong

| What you see | What it means | What to do |
|---|---|---|
| `cmake not found`, or the build stops mentioning CMake | Git Bash can't see the Build Tools' CMake | Run the `export PATH=…` line from [step 5](#step-5-build-it) again, in this window |
| The build mentions `libclang` or `clang` | LLVM is missing or somewhere else | Install it ([step 2](#step-2-install-the-c-build-tools-and-llvm)), or `export LIBCLANG_PATH="/c/Program Files/LLVM/bin"` |
| `Access is denied (os error 5)` when building | cnverc is still running and holding its own file | Close the cnverc window, then build again |
| `SHERPA_ONNX_LIB_DIR does not exist` | That variable points at a folder that isn't there | `unset SHERPA_ONNX_LIB_DIR` and build again (Windows), or fix the path (Linux) |
| `free(): invalid pointer` when a model loads — **Linux** | The wrong onnxruntime got linked in | [TECHNICAL.md → Linux: shared sherpa-onnx](TECHNICAL.md#linux-shared-sherpa-onnx) |
| `ALSA lib pcm.c … Unknown PCM pulse/jack/oss` — **Linux** | Harmless: cnverc is probing plug-ins you don't have | Ignore it. If no microphone shows up at all, `sudo apt install libasound2-plugins` |
| `Schema error: … already registered` — **Linux** | Harmless startup message from onnxruntime | Ignore it |
| A model shows `DISABLED - missing: …` in `--report` | A file isn't where the report says it looked | Put it at exactly that path, or run `./setup-models.sh` again |
| A sentence comes out wrong | Could be mishearing or mistranslation | Run `--listen --wav` and play back `logs/segments/` to see which |

Building again without internet, and every other build detail, is in
[TECHNICAL.md → Building](TECHNICAL.md#building).

---

## The models

Every model lives in its own folder under `models/`, keeping the name it was published with.
To see what a file is, look at the folder it's in. `./setup-models.sh` downloads all of these;
the table is here for when you want to fetch or replace one by hand.

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

Why these models and not others — including the translator sizes that were tried and rejected
— is in [TECHNICAL.md](TECHNICAL.md).

---

## Licence

Unpublished. Model files come with their own licences; check each one before you redistribute
it.
