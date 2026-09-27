# Instructions: per-side recognizers and dialect ("variety") support

For Claude Code. Two changes to cnverc, in order:

- **Part A:** in shared mode, each person gets their own recognizer instead of one for both.
- **Part B:** add a dialect setting, called a **variety**, that flows through all three
  stages: recognition, translation and voice.

Read this whole file before writing any code. Finish and commit Part A before starting Part B.

---

## Why

Dialect accuracy is the point of cnverc. It's what makes it better than a general translation
app. Today the app only understands *languages*: `es`, `ar`, `en`. There's no way to say
"Mexican Spanish" or "Iraqi Arabic", so:

- Shared mode forces one recognizer to cover both people. A Spanish-tuned Whisper can't be
  used, because the English side needs a recognizer too.
- The translation prompt says "Arabic", so Qwen answers in formal written Arabic, not Iraqi.
- The voice picker can't tell two Spanish voices apart. That's the open issue in `HANDOFF.md`:
  two voices declare `languages = ["es"]` and nothing decides between them.

Part A fixes the first. Part B fixes all three the same way.

---

## Before writing code

Read these and summarise them back before changing anything:

- The shared-mode code: how the recognizer is chosen and loaded, and where the check "the
  recognizer must cover both languages" lives.
- `models.rs`: how `engine.toml` is parsed (it rejects unknown keys), and `for_language`.
- `config.rs`: the `[shared]` section and the settings for the other modes, and how
  `save_selections` writes changes back.
- `translate.rs`: `system_prompt`, `language_name`, `leaks_the_prompt`, and their tests.
- `asr.rs`: where the language is passed to Whisper (near line 261).
- `peer.rs`: whether a language code is sent between machines in paired mode, and in what form.

---

# Part A — one recognizer per side

## What changes

In shared mode, each column gets a **Recognizer** dropdown under its language. The dropdown
lists only recognizers whose `languages` include that side's language.

Example: English on the left with Parakeet, Spanish on the right with the Spanish-tuned
Whisper.

## Decisions

1. **Both stay loaded.** Load each side's recognizer when shared mode starts, or when a
   dropdown changes, and keep it loaded. Only one person talks at a time, so only one runs at a
   time. Loading on every turn would add seconds of delay per turn.
2. **Same model on both sides loads once.** If both sides pick the same recognizer, share one
   loaded copy.
3. **Memory.** Print GPU memory after each load. If the second recognizer fails to load, show
   the error in that column and let the other side keep working. Don't crash, and don't unload
   the working side.
4. **Remove the old check.** "The recognizer must cover both languages" is replaced by a check
   per side: "this side's recognizer must cover this side's language".

## Config

Add to `[shared]`, declared in `config.rs` with defaults so older config files still load:

```toml
left_asr  = ""   # folder name under models/asr/, empty = best match
right_asr = ""
```

## Steps

**A1 — config.** Add the two keys. Nothing uses them yet.
Check: the app starts with an old `cnverc.toml` and with a new one.

**A2 — load and route.** Load a recognizer per side, and send each turn to its own side's
recognizer.
Check, by hand: Parakeet on the left, Spanish-tuned Whisper on the right. English on the left
and Spanish on the right both transcribe correctly. The log shows which recognizer handled each
turn.

**A3 — dropdowns.** One recognizer dropdown per column, filtered by that side's language.
Check: the warning in the screenshot ("does not list 'en' among its languages") no longer
appears when each side has a suitable model.

Commit after each step.

---

# Part B — varieties

## What a variety is

A variety is a language plus a region, written like `es-MX`, `ar-IQ`, `ar-JO`, `en-US`. This is
a standard format for naming language varieties, called a BCP 47 tag. The part before the
hyphen is the language. A plain `es` still works and means "Spanish, no particular dialect".

## Decisions

**1. One setting, not two.** Wherever cnverc stores a language today (shared mode's two sides,
and the source and target languages for the other modes), it now stores a tag that may carry a
variety: `es` or `es-MX`. Old config files with plain codes keep working unchanged.

**2. A table of known varieties, in its own file.** Create `src/varieties.rs` with one entry
per variety:

| tag | display name | prompt name |
|---|---|---|
| `en` | English | English |
| `en-US` | English (US) | American English |
| `es` | Spanish | Spanish |
| `es-MX` | Spanish (Mexico) | Mexican Spanish |
| `ar` | Arabic | Arabic |
| `ar-IQ` | Arabic (Iraq) | Iraqi Arabic |
| `ar-JO` | Arabic (Jordan) | Jordanian Arabic |

The display name is for the GUI. The prompt name is what goes into the translation prompt.
Adding a dialect later means adding one row, not code. `language_name` in `translate.rs` is
replaced by lookups into this table. That also fixes the known gap that `language_name` doesn't
know Arabic.

A tag that's not in the table is an error shown to the user, not a silent fallback.

**3. Two dropdowns in the GUI: language, then variety.** Pick "Spanish", then pick "Mexico" or
"(any)". The second dropdown lists only varieties of the chosen language from the table.

**4. Models declare varieties in `engine.toml`.** Add an optional key:

```toml
languages = ["es"]
varieties = ["es-MX"]   # optional: what this model is tuned for
```

Every entry in `varieties` must belong to a language in `languages`. If it doesn't, the model
shows as broken in `--report`, with the reason.

**5. How models are matched to a side.** For a side set to `es-MX`, each model dropdown
(recognizer, voice) lists models in this order:

1. Tuned for exactly `es-MX`. Labelled "tuned for Spanish (Mexico)".
2. Covering `es` with no variety declared. Labelled "general".
3. Tuned for another Spanish variety, e.g. `es-ES`. Labelled with that variety, so nobody picks
   it by accident.

The default selection is the first entry. Models that don't cover the language at all aren't
listed. This replaces `for_language`'s "first match wins", and it resolves the two-Spanish-voices
issue in `HANDOFF.md`. Close that issue in `HANDOFF.md` when done.

## Through the three stages

**Recognition (`asr.rs`).** Whisper only accepts plain language codes. Pass it the language
part only (`es`, never `es-MX`). This is easy to get wrong without noticing: an unrecognised code
may make Whisper guess the language, and it will usually guess right, which hides the bug. Log
the exact code passed to Whisper on every turn. The variety still matters here, but through
*which model* is chosen, not through what Whisper is told.

**Translation (`translate.rs`).** Build the prompt from the prompt names in the table. When the
target has a variety, add one sentence:

> Write it the way a {target prompt name} speaker would say it aloud, using everyday spoken
> wording rather than the formal written standard.

When the source has a variety, the existing sentence already reads "Translate the user's Iraqi
Arabic text", which helps Qwen understand dialect words. No extra sentence is needed.

Keep `leaks_the_prompt` working. It checks whether Qwen recited the prompt back. It must
recognise the new sentence too. Add a test that builds a prompt with a variety and confirms that
reciting it back is caught.

The translator is shared by both directions, so it keeps one dropdown. But translation models
may also declare `varieties` in `engine.toml`, for a later dialect-tuned Qwen. Show that in the
dropdown label.

**Voice (`tts.rs`).** Filter and order voices by the matching rule above, using the variety of
the language being *spoken*: for shared mode's left column, that's the right person's variety.

## Paired mode

If `peer.rs` sends language codes between machines, send the full tag. If that changes the
message format, bump the protocol version so two machines on different versions refuse to pair
with a clear message, rather than mistranslating quietly. If languages aren't sent over the
wire, say so in the summary and change nothing there.

## Steps

**B1 — the table.** Add `src/varieties.rs` with the table and lookup functions, plus unit tests.
Replace `language_name` with lookups into the table.
Check: `cargo test` passes. The Spanish→English ignored test still passes.

**B2 — engine.toml.** Add the optional `varieties` key to the descriptor, with the validation in
decision 4. Update `--report` to print each model's varieties.
Check: every existing model still loads. A test descriptor with `varieties = ["fr-FR"]` and
`languages = ["es"]` is reported as broken, with the reason.

**B3 — label the user's models.** List every model folder and **propose** a `varieties` line for
each, based on its name and model card. For example, the Piper `es_MX claude` voice gets
`["es-MX"]`, and the Jordanian voice gets `["ar-JO"]`. Show the list to the user and wait for
confirmation before editing any `engine.toml`. Models with no clear dialect get no `varieties`
line.

**B4 — config and matching.** Accept tags in every language setting. Implement the matching
order from decision 5 for recognizers and voices.
Check: unit tests for the matching order, including a side set to `es-MX` with a `es-MX` voice,
a general `es` voice and an `es-ES` voice. They must come out in that order.

**B5 — recognition.** Pass only the language part to Whisper. Log it.
Check: a turn set to `es-MX` logs `es` going to Whisper.

**B6 — translation.** The prompt change and its tests.
Check: run these through the translator and print the outputs for the user to read. Don't judge
them automatically.
- English → `es` and English → `es-MX`: "Hey man, what's up? Want to grab a bite?"
- English → `ar` and English → `ar-IQ`: "What are you doing right now?"

Print each pair side by side. The user decides whether the dialect prompt changed anything.

**B7 — GUI.** Language and variety dropdowns, model labels ("tuned for…", "general"),
everywhere a language is chosen.
Check, by hand: set the right side to Spanish (Mexico). The recognizer and voice dropdowns show
the Mexico-tuned models first, labelled as such.

**B8 — paired mode**, per the section above.
Check: two machines pair, and the translation arrives in the right variety.

**B9 — documents.** Update `README.md` (how to pick a variety, how to label a model) and
`DIALECTS.md` (the `varieties` key and the matching rule). Update `HANDOFF.md` (close the voice
picker issue, record anything left open). Add Parts A and B to `SPEC.md` as their own milestones.
Don't renumber M8 and M9.

Commit after each step. Re-run turn-based, continuous and paired mode once after B7.

---

## One limit to state plainly in the docs

The translation prompt can *ask* for Iraqi Arabic. Whether a 1.7-billion-parameter Qwen can
*produce* convincing Iraqi Arabic is a separate question, and the answer may be "only
somewhat". Step B6 is there so the user sees the answer with their own eyes. If the dialect
prompt barely changes the output, the fix is a translator tuned on dialect text, which is
already planned. It isn't a bigger prompt.
