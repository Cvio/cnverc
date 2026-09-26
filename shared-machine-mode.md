# Instructions: shared-machine mode for cnverc

For Claude Code. Add a mode where two people who speak different languages use **one**
machine. Each person has their own key. Press your key, speak, press it again, and the machine
speaks your words in the other person's language.

Read this whole file before writing any code.

---

## What the user sees

Example: an English speaker sits on the left, a Spanish speaker on the right.

1. The left person presses **Left Arrow**. The left side of the window lights up: "Listening —
   English".
2. They speak, then press **Left Arrow** again to finish.
3. cnverc transcribes the speech as English, translates it into Spanish, and speaks the Spanish
   through the speakers. The left panel shows the English text and the Spanish translation.
4. The right person presses **Right Arrow** and does the same thing in the other direction.

The key tells cnverc two things at once: who is talking, and what language they are speaking.
No language detection is needed, and none should be used.

---

## Before writing code

Read these and summarise them back before changing anything:

- How turn-based mode works now: where the spacebar is handled, how a turn starts and ends,
  and how the audio gets to the recognizer.
- How the source and target languages are chosen and passed through `pipeline.rs`.
- How `tts.rs` and `models.rs` pick a voice for a language (`for_language`). There is a filed
  issue in `HANDOFF.md`: two Spanish voices both declare `languages = ["es"]` and nothing
  decides between them. This mode makes that worse, so read it.
- How paired mode's floor token works in `peer.rs`. Shared mode needs the same rule, "only one
  person talks at a time", on a single machine.

This is new work. It must not break turn-based mode, continuous mode or paired mode. If it
touches code those modes share, re-test them.

---

## Decisions already made

Build it this way. Don't change these without asking.

**1. It is its own mode.** Add "Shared" next to the existing modes. Don't bolt it onto
turn-based mode behind a checkbox. Shared mode can't be on while paired. If the app is paired,
the Shared option is greyed out and says why.

**2. Press to start, press to stop.** This matches how the spacebar already works in
turn-based mode. It is not hold-to-talk.

**3. One person at a time.** While one side is recording, the other key does nothing. While
the machine is speaking a translation, both keys do nothing, and the microphone is off. That
stops cnverc from hearing its own output and translating it. Show the reason on screen
("Speaking — wait") so a key press that does nothing doesn't look like a bug.

**4. Escape cancels.** Escape throws away a recording in progress, or stops playback early.
Nothing is translated or spoken for a cancelled turn.

**5. Each side has its own settings.**
- Language: a dropdown per side.
- Voice: a dropdown per side, listing only voices for the *other* side's language. That's the
  voice this side's words will be spoken in. Pick by folder name, so two Spanish voices can be
  told apart. This works around the voice picker issue without fixing it. Don't fix it here.
- Recognizer: shared by both sides. It must support both languages. If the selected
  recognizer only covers one of them (Parakeet is English-only), show that plainly and don't
  let a turn start in the language it can't handle.

**6. The key sets the recognizer language.** When the left key starts a turn, pass the left
language to Whisper explicitly. Never let it guess. A short Spanish sentence with an English
name in it gets misdetected often enough to matter.

**7. Keys only work when no text box has focus.** Arrow keys also move the cursor in text
boxes. If a text field has focus, the arrow keys belong to it. Clicking anywhere outside a
text field gives the keys back.

---

## Config

Add a section to `cnverc.toml`:

```toml
[shared]
left_language  = "en"
right_language = "es"
left_voice     = ""    # folder name under models/tts/, empty = first match
right_voice    = ""
left_key       = "ArrowLeft"
right_key      = "ArrowRight"
```

`left_voice` is the voice used to speak what the *left* person said, so it's a voice in the
right person's language. Put that in a comment in the file, because it will confuse someone
otherwise.

Two things about the config code:

- `config.rs` uses `deny_unknown_fields`. Declare every new key in the struct, with defaults,
  so an older `cnverc.toml` without a `[shared]` section still loads.
- Changes made in the GUI are saved through `save_selections`, which uses `toml_edit` so the
  user's comments survive. Use the same path. Don't rewrite the file.

The keys are configurable because some keyboards and foot pedals send other keys. The defaults
are the arrows.

---

## Window layout in Shared mode

Two columns, left and right, matching where the people sit.

Each column shows:
- The language and the key, in large text: "English — ←".
- A status: Ready / Listening / Working / Speaking — wait.
- A running history of that side's turns: what was said, and the translation.

The column whose turn it is gets a strong, obvious highlight. Someone glancing at the screen
from a metre away should be able to tell whose turn it is.

The settings (languages, voices, keys) sit above the columns and are locked while a turn is in
progress.

---

## Steps

Do these in order. Commit after each one.

**Step 1 — config and mode switch.** Add the `[shared]` section and the Shared mode option.
Nothing happens yet when it's selected.
Check: the app starts with an old `cnverc.toml` that has no `[shared]` section, and with a new
one. `cnverc --report` still runs.

**Step 2 — direction logic, no GUI.** Write the function that takes "which side pressed" and
returns source language, target language and voice. Write unit tests for it: left→right,
right→left, a missing voice, and a recognizer that doesn't cover one of the languages.
Check: `cargo test` passes.

**Step 3 — keys and turn states.** Handle the two keys and Escape, with the one-at-a-time
rules from decision 3. Log each state change.
Check, by hand: press Left, press Right during the left turn (nothing happens), press Left to
end. Press Escape during a turn (nothing is spoken).

**Step 4 — run the pipeline per turn.** Send each finished turn through the recognizer,
translator and voice, using the direction from step 2.
Check, by hand: speak English on the left, hear Spanish. Speak Spanish on the right, hear
English. Neither ever comes out in the wrong language.

**Step 5 — the two-column window.** Build the layout above.
Check: from across a table, you can tell whose turn it is.

**Step 6 — re-test the other modes.** Run turn-based, continuous and paired mode once each.
Check: all three behave as before.

**Step 7 — documents.** Update `README.md` with how to use Shared mode, and `HANDOFF.md` with
what was built and anything left open. Add it to `SPEC.md` as its own milestone. Don't renumber
the existing ones: M8 and M9 are already planned.

---

## Things that will go wrong quietly

- **Wrong language going into Whisper.** If the language from the key isn't actually passed
  through, Whisper falls back to guessing, and it'll be right most of the time. That makes the
  bug hard to see. Log the language used for every turn, and check the log in step 4.
- **Arrow keys eaten by the window.** The GUI library may use arrow keys to move focus between
  widgets. If the keys don't reach the handler, that's the first thing to check.
- **The microphone left on during playback.** The machine then translates its own voice. Test
  this with the speakers turned up, not with headphones.
- **A voice that was deleted.** If `left_voice` names a folder that no longer exists, show an
  error in that column and fall back to the first matching voice. Don't crash.
