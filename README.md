# cnverc

cnverc is a live interpreter for two people who don't share a language. One person speaks, for
example in Spanish. cnverc writes down what they said, translates it into English, shows both on
screen, and says the English out loud. It works the other way round too.

Everything happens on your own computer. cnverc **never uses the internet**. Once it's set up,
you can unplug the network and it keeps working.

**This page gets you from nothing to talking.** Each step ends with ✅ what you should see, so
you know it worked before moving on.

---

## What you need

- A PC with **Windows 10 or 11**, or **Ubuntu 25.04 or later**.
- **8 GB of memory** (16 GB is better for two people at one PC), and **12 GB of free disk**.
- A **microphone and speakers**, or a headset.
- **Internet during setup only**, and about an hour, mostly waiting.

You'll paste a few commands into a terminal. Copy each one, paste it, and press Enter.

---

## Set up on Windows

1. **Install Git.** Download it from <https://git-scm.com/download/win> and run it, keeping every
   default. ✅ In the Start menu, **Git Bash** now exists. Open it: every command below is typed
   there.
2. **Install the C++ build tools.** Download **Build Tools for Visual Studio** from
   <https://visualstudio.microsoft.com/downloads/> (under *Tools for Visual Studio*). Run it,
   tick **Desktop development with C++**, check that **C++ CMake tools for Windows** is ticked
   on the right, and click **Install**.
3. **Install LLVM.** From <https://github.com/llvm/llvm-project/releases/latest>, download
   `LLVM-<version>-win64.exe`. Run it and choose **Add LLVM to the system PATH for all users**.
4. **Install Rust.** Download `rustup-init.exe` from <https://rustup.rs>, run it, and press
   **Enter** for the defaults. Then **close Git Bash and open it again.**
5. **Get cnverc** into a short folder (long paths break the build):

   ```bash
   cd /c/ && git clone https://github.com/Cvio/cnverc.git && cd cnverc
   ```

   In a new Git Bash window later, type `cd /c/cnverc` first.
6. **Check your tools:**

   ```bash
   ./check-setup.sh
   ```

   ✅ Every line says `OK`. If one says `MISSING`, do what the line under it says and run it
   again.
7. **Build cnverc and download its models** (10–20 minutes, plus about 2.4 GB of downloads):

   ```bash
   ./setup.sh
   ```

   ✅ It ends with `Everything is in place`. If it stops partway, run it again: it carries on
   where it left off.
8. **Start cnverc:**

   ```bash
   ./target/release/cnverc.exe
   ```

   Or double-click `cnverc.exe` in `C:\cnverc\target\release` (right-click it › **Send to ›
   Desktop** makes a shortcut). A black log window opens behind it; you can ignore it.

Now go to [First run](#first-run).

---

## Set up on Linux

Tested on Ubuntu 26.04; any Ubuntu from 25.04 on should work.

1. **Install the tools:**

   ```bash
   sudo apt install git curl build-essential cmake pkg-config libclang-dev libasound2-dev libonnxruntime-dev
   ```

2. **Install Rust,** pressing **Enter** for the defaults, then **open a new terminal**:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. **Get cnverc:**

   ```bash
   cd ~ && git clone https://github.com/Cvio/cnverc.git && cd cnverc
   ```

4. **Build the speech library** (once; about an hour):

   ```bash
   ./build-sherpa-linux.sh
   ```

   ✅ It ends with `The speech library is built`.
5. **Check your tools:** `./check-setup.sh`. ✅ Every line says `OK`.
6. **Build cnverc and download its models:** `./setup.sh`. ✅ It ends with
   `Everything is in place`.
7. **Start cnverc:** `./target/release/cnverc` (it needs a desktop session).

---

## First run

1. In the settings on the left, choose your **microphone** and **output**. "System default" is
   usually right.
2. Choose **Speaker's language** (the person talking) and **Translate into** (the listener).
3. For **Recognizer**, choose **Whisper large-v3-turbo**. It's the safe choice.
4. Press **Start** and wait for loading to finish (up to a minute).
5. Press **Space**, say "Good morning, how are you?", and press **Space** again.

✅ You see your sentence and its translation, and hear the translation spoken.

cnverc remembers your choices, so you only do this once.

---

## The four ways to use it

Choose one under **Mode**. [USE-CASES.md](USE-CASES.md) explains which suits your situation.

| Mode | Who | How to talk |
|---|---|---|
| **Take turns** | One person talking to a listener, one PC | **Space** to start, **Space** to stop. The banner goes red while recording, amber while working. |
| **Listen continuously** | One PC, no keys | Just talk and pause after each sentence. Keep **Mute the microphone while speaking** ticked if you use speakers. |
| **Shared machine** | Two people at one PC | Set each person's language above their column. Left person: **Left arrow**. Right person: **Right arrow**. Press, speak, press again. **Escape** cancels a turn. |
| **Paired** | Two people, a PC each, same network | On each PC, set that person's language. Tick **Pair with another PC**, press **Start**, then type or click the other PC's address and press **Connect**. Talk with **Space**. |

Under each language there's an optional **Variety**, such as *Spanish (Mexico)*. It picks the
matching voice and asks the translator for that dialect.

Untick **Speak translations** for captions only.

---

## Updating

Close cnverc, then from the `cnverc` folder:

```bash
git pull && ./setup.sh
```

Your settings are kept.

---

## If something goes wrong

| What you see | What to do |
|---|---|
| A `MISSING` line from `check-setup.sh` or `setup.sh` | Do what the line under it says, then run it again |
| `setup.sh` stopped partway, or a download failed | Run `./setup.sh` again |
| `Access is denied` while building | cnverc is still open. Close it and try again |
| The level meter doesn't move when you talk | Choose another microphone, or unmute it in your system's sound settings |
| cnverc translates its own voice | Tick **Mute the microphone while speaking**, or use a headset |

Anything else: the full table is in
[USE-CASES.md → When something goes wrong](USE-CASES.md#when-something-goes-wrong). Asking for
help? Include the newest file in `target/release/logs/`.

---

## Read more

| File | For |
|---|---|
| [USE-CASES.md](USE-CASES.md) | Situations and how to run each; working with no internet; full troubleshooting; glossary |
| [MODELS.md](MODELS.md) | How the models work, explained simply, and how to add a voice, language or model |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How the code fits together, module by module, for developers |
| [TECHNICAL.md](TECHNICAL.md) | Why it's built this way: design decisions and build internals |
| [SPEC.md](SPEC.md) | The original specification and milestones |
| [DIALECTS.md](DIALECTS.md), [lora.md](lora.md) | Training models for a regional dialect |
| [HANDOFF.md](HANDOFF.md) | Where the project stands, for whoever works on it next |

## Licence

Unpublished. Model files come with their own licences; check each before redistributing.
