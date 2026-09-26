# Using cnverc: situations and how to handle them

The [README](README.md) tells you how to set cnverc up and what each control does. This guide
starts from the other end: **you're in a situation, so what do you set up, and how does the
conversation go?**

Each use case follows the same pattern: who it's for, what you need, how to set it up, how the
conversation goes, what good looks like, what usually goes wrong, and its limits.

| Situation | Mode | PCs | Use case |
|---|---|---|---|
| Two people at one desk or counter | Shared machine | 1 | [1](#1-two-people-one-desk) |
| One person speaking to a listener | Take turns | 1 | [2](#2-one-speaker-one-listener) |
| Two people, a PC each, same room | Paired + Take turns | 2 | [3](#3-two-pcs-in-the-same-room) |
| Two people in different rooms | Paired | 2 | [4](#4-two-pcs-in-different-rooms) |
| A fast, natural conversation | Paired + Listen continuously | 2 | [5](#5-a-flowing-conversation-with-headsets) |
| Reading, not listening | Any, with speech off | 1 or 2 | [6](#6-captions-only) |
| No internet anywhere | Any | 1 or 2 | [7](#7-working-with-no-internet) |
| Checking or fixing recognition | `--compare`, `--wav` | 1 | [8](#8-which-recognizer-hears-me-best-and-why-was-that-wrong) |
| Using a better or tuned model | — | 1 | [9](#9-adding-a-better-model) |

A few things are true in every case:

- **cnverc translates between two languages at a time.** Out of the box those are English and
  Spanish. Other languages need a voice and a recognizer that know them; see
  [ARCHITECTURE.md → Add a language](ARCHITECTURE.md#add-a-language).
- **Short, complete sentences work best.** Say one or two sentences per turn, and pause
  briefly before you start.
- **It takes a second or three** from the end of a sentence to hearing the translation. Leave
  that gap, and it feels like talking through an interpreter.
- **Check the result before relying on it.** cnverc shows what it heard and what it
  translated. For anything important (a dose, an address, a price), read the screen, not just
  the voice.

---

## 1. Two people, one desk

**Who and where:** a reception desk, a counter, a table at a clinic, a school office. One
person speaks English and the other Spanish, and they sit either side of, or next to, one
laptop.

**What you need:**
- One PC with speakers. A laptop's built-in speakers are fine in a quiet room.
- A microphone both people can reach: the laptop's own, or a small USB desk microphone
  between them.
- **Shared machine** mode, with a **Whisper** recognizer (Parakeet can't be used here).

**Setting up** (once):
1. Start cnverc. Under **Mode**, choose **Shared machine**.
2. Set **Recognizer** to *Whisper large-v3-turbo*.
3. Above the columns, set **Left person** to the language of whoever sits on the left, and
   **Right person** to the other. Match the screen to where people actually sit: left is left.
4. Choose microphone and speakers in the settings. Leave **Mute the microphone while
   speaking** ticked.
5. Press **Start** and wait for both columns to say **Ready**.

**The conversation:**
1. The left person presses **Left arrow**. Their column turns red and says **Listening**.
2. They say a sentence or two, then press **Left arrow** again.
3. The column turns amber (**Working**), then blue (**Speaking**), and the translation is
   spoken aloud. Both what they said and the translation appear in their column.
4. When the column says **Ready** again, the right person presses **Right arrow** and does the
   same.
5. Made a mistake mid-sentence? Press **Escape**: that turn is thrown away and nothing is
   spoken. Start again.

**What good looks like:** each person's words appear correctly in their column, the
translation is spoken in the other language, and neither column ever speaks in the wrong
language.

**When it goes wrong:**
- **A key does nothing.** Read that person's column. It says why: the other person is
  talking, the PC is still speaking, or their language can't be heard by the chosen
  recognizer. If nothing is shown, click somewhere outside any text box and try again.
- **One column says it "cannot hear this side".** The recognizer doesn't know that
  language. Choose *Whisper large-v3-turbo*.
- **It translates its own voice.** Make sure **Mute the microphone while speaking** is
  ticked.

**Limits:** one person at a time, by design. It isn't meant for two people talking over each
other.

---

## 2. One speaker, one listener

**Who and where:** one person needs to say things to someone who speaks another language: a
short announcement, instructions, questions at a doorway. The listener mostly listens.

**What you need:** one PC with speakers and a microphone, **Take turns** mode, and either
recognizer.

**Setting up:**
1. **Mode:** *Take turns*, *Press Space to start, again to stop*.
2. **Speaker's language:** the language you speak. **Translate into:** the listener's.
3. Press **Start**.

**The conversation:**
1. Press **Space**. The banner turns red: **RECORDING**.
2. Speak one or two sentences, then press **Space** again.
3. The banner shows **PROCESSING**, then **SPEAKING**, and the translation is spoken. It's also
   shown large on screen, which you can turn towards the listener.
4. To let the listener answer, swap the two languages (stop, swap, start), or use
   [use case 1](#1-two-people-one-desk) instead, which handles both directions.

Prefer holding the key down while you talk? Choose *Hold Space while speaking*.

**What good looks like:** the large text on screen matches what you meant, and it's spoken
clearly.

**When it goes wrong:**
- **"Nothing recognised".** The microphone didn't hear you. Check the level meter at the top
  moves when you speak, and pick another microphone if it doesn't.
- **The first word is missing.** Leave a short pause after pressing Space before you speak.

**Limits:** one direction at a time. For a two-way conversation at one PC, use Shared machine.

---

## 3. Two PCs in the same room

**Who and where:** two people each with their own laptop, a meeting across a table. Each
person hears the other's words in their own language from their own laptop.

**What you need:**
- Two PCs with cnverc, on the same network: Wi-Fi, a router with no internet, or one Ethernet
  cable between them (see the README's network table).
- Speakers on both.
- **Pair with another PC**, and **Take turns** on both.

**Setting up:**
1. On each PC, set the languages **for that PC's person**. On Ana's (Spanish): Speaker's
   language *Spanish*, Translate into *English*. On Ben's (English): the other way round.
2. On both: tick **Pair with another PC**, choose **Take turns**, press **Start**. The first
   time, allow cnverc through the Windows firewall on **Private networks**.
3. On either PC: type the other PC's address (shown under **This PC** on it), or click it
   under **Found on this network**, and press **Connect**.
4. Both show **Paired with …** at the top.

**The conversation:**
1. Ana presses **Space**. For a moment her banner says **ASKING FOR THE FLOOR…**, then
   **RECORDING**. Ben's screen shows Ana's PC's name followed by **IS TALKING**, and his Space
   key waits.
2. Ana speaks, then presses **Space**. Her words appear on Ben's PC, in English, and are
   spoken there.
3. Ben presses **Space** and answers the same way.

**What good looks like:** only one microphone is live at a time, so both PCs can use
speakers, and nobody's words come back round in a loop.

**When it goes wrong:**
- **"No answer from … within 4 s".** A firewall is blocking. Set that network to **Private**
  in Windows settings, on the PC that isn't being reached.
- **Pressed Space and it says the other person has the floor.** Wait for them to finish; their
  turn has to end first.
- **The connection drops** (Wi-Fi gone, cable pulled): both PCs say **Not paired** within a few
  seconds. Press **Connect** again.

**Limits:** turn-taking only. Talking over each other needs use case 5.

---

## 4. Two PCs in different rooms

**Who and where:** the two people can't hear each other directly: an intercom between rooms,
or someone in another part of a building on the same network.

**What you need:** as in use case 3, but a **headset on each PC** is better, since each person
hears only their PC. The same network is needed, since cnverc never uses the internet.

**Setting up and the conversation:** exactly as in [use case 3](#3-two-pcs-in-the-same-room).
Because the rooms are separate, you can also try **Listen continuously** (use case 5).

**When it goes wrong:**
- **The other PC isn't found.** Guest Wi-Fi networks often stop devices seeing each other.
  Use the main network, or type the address shown on the other PC.

**Limits:** both PCs must be on the same local network. cnverc doesn't work across the
internet, by design.

---

## 5. A flowing conversation with headsets

**Who and where:** two people who want to talk naturally, without pressing keys, each wearing
a headset.

**What you need:** two paired PCs, as in use case 3, a **headset on each**, and **Listen
continuously** on both.

**Setting up:** as in use case 3, but choose **Listen continuously**. cnverc shows a red
**Headsets required** warning the whole time. That's expected.

**The conversation:** just talk, pausing briefly at the end of each sentence. Each pause sends
what you said.

**Why headsets are required:** with speakers, each PC's microphone would hear the other PC's
translation and translate it back, round and round. With headsets, each microphone only hears
its own person.

**When it goes wrong:**
- **Sentences cut in half.** You paused mid-sentence. Pause only at the end, or raise
  `min_silence_ms` under `[vad]` in `cnverc.toml` (for example, to 800).
- **Background noise becomes words.** Continuous mode translates anything it hears. Use a
  headset with a close microphone, or switch back to Take turns.

**Limits:** less control than Take turns, and it depends on everyone wearing headsets.

---

## 6. Captions only

**Who and where:** a noisy room where speech wouldn't be heard, a hard-of-hearing listener,
or anyone who'd rather read.

**What you need:** any of the setups above, with **Speak translations** unticked.

**The conversation:** as in the use case you based it on. The translation appears large on
screen, and nothing is spoken. Turn the screen towards the reader.

**Tip:** with speech off there's nothing to hear, so **Listen continuously** works well even
without headsets on a single PC.

---

## 7. Working with no internet

**Who and where:** field work, a site with no connection, or a policy of no internet at all.

**What you need:** cnverc set up on a PC **with** internet first (the README's setup). After
that, it never needs internet.

**On Windows, to use another PC with no internet:**
1. On the PC where you set cnverc up, copy these from its `target\release\` folder onto a USB
   stick: `cnverc.exe`, `cnverc.toml` and the `models` folder.
2. On the other PC, copy them into any folder and double-click `cnverc.exe`. Nothing needs
   installing.

**Two laptops, no network at all:** connect them with an ordinary Ethernet cable, pair them as
in use case 3, and type the addresses shown. They'll start with `169.254.` after about thirty
seconds, which is normal. Set that network to **Private** on both, if Windows asks or pairing
times out.

**Limits:** on Linux, the offline copy doesn't work yet: each Linux PC needs the setup done
on it. See the README.

---

## 8. "Which recognizer hears me best?" and "Why was that wrong?"

**Who and where:** you're setting cnverc up for a particular person, or something keeps coming
out wrong.

**Which recognizer suits a voice:**
1. In the window, tick **Compare recognizers** and press **Start**. Or, in a terminal:
   `./target/release/cnverc.exe --listen --compare`.
2. Speak normally for a minute. Each installed recognizer writes down the same audio, side by
   side, with how long it took.
3. Pick the one that gets that person's words right most often, and set it as the
   **Recognizer**. Published scores don't predict this: accents, microphones and rooms all
   differ.

**Why a sentence came out wrong:**
1. Run `./target/release/cnverc.exe --listen --wav --seconds 60` and say the sentence again.
2. Play back the file it saved in `target\release\logs\segments\`.
   - **The recording itself is bad** (cut off, quiet, noisy): it's the microphone or where you
     paused. Try another microphone, or leave a pause before speaking.
   - **The recording is fine but the words written are wrong:** it's the recognizer. Compare
     recognizers (above).
   - **The words are right but the translation is wrong:** it's the translator. The log shows
     both texts; rephrasing usually helps.
3. The log (`target\release\logs\`) records every sentence with its timings. Send it along if
   you ask for help.

---

## 9. Adding a better model

**Who and where:** you've found or trained a better Whisper model for your language or
dialect, or want to try a different translator.

**A different Whisper model** (from Hugging Face, or tuned yourself): use the separate
[model-converter](https://github.com/Cvio/model-converter) project. It turns a Hugging Face
Whisper model into a folder cnverc can use, checks the conversion, and installs it. Then
compare it with the stock one (use case 8) before switching.

**A different translator:** `models\mt\` must hold exactly one Qwen3 `.gguf` file. Move the
current one somewhere safe, put the new one in, and restart cnverc. Keep the old file, so you
can switch back.

**Tuning for a dialect:** see [DIALECTS.md](DIALECTS.md) and [lora.md](lora.md).
