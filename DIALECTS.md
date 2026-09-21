# Getting a dialect working in cnverc

As of 2026-09-21. A living copy of this guide is kept as a Claude Doc; this file is the
repository's snapshot, for anyone (or any Claude Code session) working on dialect support.

## Bottom line

Tune the recognizer first, and only if a measured baseline says it needs it. Dialect lives at
three stages of cnverc, and they differ in payoff, in the data they need, and in how hard the
result is to get back into the app.

| Stage | What dialect breaks | Payoff from tuning | Data it needs | Model to tune |
| --- | --- | --- | --- | --- |
| Recognition | Mishearing: wrong words, dropped words | High for Arabic dialects, modest for Spanish | Audio + transcripts in the dialect | Whisper large-v3-turbo, with LoRA |
| Translation | Stiff or wrong register; dialect words mistranslated | Medium; try the prompt first | Parallel sentences: dialect ↔ English | Qwen3 1.7B, with LoRA |
| Speech | The voice has the wrong accent | Low; comprehension rarely suffers | Hours of one clean speaker | A Piper voice, fine-tuned |

The order to work in is: measure, then recognition, then translation, then speech — and stop at
the first stage that is good enough. Every tuned model has to come back as a drop-in folder under
`models/`, so each section below ends with the export path, which is where most of the real effort
goes.

## Step 0: measure before tuning

Set aside a test set before you train anything, and never let it touch training. Without it you
cannot tell whether a tuned model is better, the same, or worse — and fine-tuning makes models
worse more often than people expect.

- **Size:** 100–200 utterances is enough to see a real difference. More is better; fewer than 50
  is noise.
- **Split by speaker, not by clip.** If the same person is in both training and test, the score
  measures memorised voice, not dialect.
- **Two references per utterance:** the correct transcript, and a correct English translation.
- **Real conditions:** record some through the microphone cnverc will actually use. Clean studio
  audio flatters every model.

Then score what you already have. `cnverc --listen --compare --wav` runs both installed
recognizers on the same audio and saves each utterance. Word error rate is the recognizer's
number; for translation, a fixed set of 20–50 sentences judged by a fluent speaker is more honest
than any automatic score at this scale.

If stock Whisper already gets the dialect mostly right, stop — recognition does not need tuning,
and the rest of this guide starts at translation.

## Sort the datasets first

The kind of data decides which model it can train; the dialect does not. A dataset can only train
the stage whose input and output it contains.

| What the dataset contains | Trains | Useful amount |
| --- | --- | --- |
| Audio + a transcript of the same audio | Recognition (Whisper) | 10+ hours helps; 50+ hours is solid |
| Sentences in the dialect + their English translation | Translation (Qwen3) | 5,000+ pairs; 20,000+ is solid |
| Many hours of one speaker, clean, with transcripts | Speech (Piper) | 1–2 hours minimum from an existing checkpoint |
| Dialect text alone, no translation | Nothing directly | Can be turned into pairs with a larger model |
| Audio alone, no transcript | Nothing directly | Needs transcribing first |

For each dataset, check four things before spending compute on it. Its **licence** must allow
training and your use. The **transcription convention** must be consistent: for Arabic, whether it
carries diacritics, uses Arabic script or Latin "Arabizi", and spells dialect words consistently.
The **speaker count** matters more than the hours — 50 hours from three people teaches three
voices. And whether it is **genuinely the dialect** or mostly the standard language: many "Iraqi"
sets drift heavily into Modern Standard Arabic.

One transcription choice matters for the whole pipeline. Recognition output is translation's
input, so the translation model should be trained on text that looks like what the recognizer
produces — not polished written text.

## Recognition: tune Whisper large-v3-turbo

Tune `openai/whisper-large-v3-turbo`, the model cnverc already runs, with LoRA. Tuning a smaller
Whisper is easier, but it throws away most of the capacity you are trying to adapt; a small model
that knows the dialect often loses to a large one that half-knows it.

**Not Parakeet.** Parakeet v3 covers 25 European languages and no Arabic, and tuning it means
NVIDIA's NeMo toolkit, a heavier lift than Whisper for no gain. For a Spanish dialect it is
possible, but Whisper is the one path that works for every language.

**LoRA, not a full fine-tune.** LoRA trains a small set of add-on weights rather than the whole
model, so it is much less likely to erase what the model already knew. The 5090 has room for a
full fine-tune, but start with LoRA and only go further if it plateaus. Reasonable starting
settings: rank 16–32, alpha twice the rank, targeting the attention projections, learning rate
around 1e-4. Train with the language token set to the base language — Whisper has `ar` and `es`,
no dialect tokens — and let the data carry the dialect.

**The export path is the real work.** sherpa-onnx cannot load a Hugging Face checkpoint directly.
Others who have done it report the same four steps:

1. Merge the LoRA adapter into the base model, so it is one ordinary Whisper checkpoint again.
2. Convert the Hugging Face checkpoint to OpenAI's original Whisper format — a file holding `dims`
   and `model_state_dict`. The export script only reads that format.
3. Run sherpa-onnx's `scripts/whisper/export-onnx.py`, adapted to accept a local checkpoint; as
   shipped it downloads named models and does not support new ones.
4. Quantize to int8, giving `encoder.int8.onnx`, `decoder.int8.onnx` and `tokens.txt` — the three
   files cnverc's `engine.toml` expects.

One trap: on PyTorch 2.9 or later the export needs `dynamo=False` passed to `torch.onnx.export`,
because the newer exporter fails on Whisper's positional-embedding indexing.

The result goes in `models/asr/<name>/` with `backend = "whisper"`, and appears in the recognizer
picker beside the stock model. Run `--compare` on your held-out set with both installed: that is
the whole test.

Sources: [export-onnx.py](https://github.com/k2-fsa/sherpa-onnx/blob/master/scripts/whisper/export-onnx.py),
[sherpa-onnx issue #3636](https://github.com/k2-fsa/sherpa-onnx/issues/3636),
[indic-asr-onnx export notes](https://huggingface.co/ippocode/indic-asr-onnx),
[OffVox language packs](https://huggingface.co/Dpn82/offvox-language-packs).

## Translation: prompt first, then tune Qwen3 1.7B

Change the prompt before you train anything. The translator is told "translate into Spanish" and
produces whatever its training favours; telling it "into Mexican Spanish, keeping common English
loanwords" is one line in `translate.rs` and costs nothing to measure. For a Spanish dialect this
may be all you need. For Iraqi Arabic it probably is not — a 1.7B model's grip on a regional Arabic
dialect is weak, and dialect *input* is harder than dialect output.

**If the prompt falls short, tune `Qwen/Qwen3-1.7B` with LoRA.** It must stay a Qwen3 model:
cnverc checks for one, because it relies on Qwen3's chat format and how it handles the thinking
block. On a 5090, a 1.7B model trains with LoRA in full precision; there is no need for 4-bit
tricks.

You cannot train the file cnverc uses. `qwen3-1.7b-q4_k_m.gguf` is a compressed inference copy;
training happens on the full-precision weights. The round trip is:

1. Train LoRA on the Hugging Face model.
2. Merge the adapter into the weights.
3. Convert with llama.cpp's `convert_hf_to_gguf.py`.
4. Quantize to Q4_K_M.
5. Replace the file in `models/mt/`. It must hold exactly one `.gguf`.

**Train both directions.** Translation runs dialect→English and English→dialect, and tuning hard on
one can degrade the other. Include pairs in both directions, mix in some general translation data
so the model keeps its breadth, and re-run `bench_both_directions` — plus a larger held-out set,
since 20 sentences will not catch a small regression.

**Distillation if you lack pairs.** If a dataset has dialect text but no translations, a much
larger model can translate it and the small one can learn from those outputs. Check the large
model's translations with a fluent speaker first; you are teaching its mistakes too.

## Speech: usually skip it

Leave the voice until last, and probably leave it alone. A voice in the wrong accent is
understood; a recognizer that mishears is not. Tuning speech improves how cnverc sounds, not
whether it works.

Check what exists before training anything. Piper's only Arabic voice is Jordanian
(`ar_JO-kareem`, low and medium quality), which sherpa-onnx already packages. Levantine Arabic is
broadly intelligible to Iraqi listeners, so it is a reasonable stand-in. For Spanish,
`es_MX-claude-high` is already installed.

**If you do train one**, fine-tune an existing Piper checkpoint rather than starting from scratch.
You need at least one to two hours of a single speaker — clean, consistent microphone, no
background noise — with accurate transcripts. Piper pronounces through espeak-ng, so check that
espeak-ng handles your dialect's spelling tolerably before recording anything. The trained voice
exports to ONNX, but sherpa-onnx also needs metadata added to the file before it will load it;
sherpa-onnx provides a script for that.

The result goes in `models/tts/<name>/` with `backend = "vits"` and
`data_dir = "espeak-ng-data"`, like the voices you already have.

## Hardware and where to train

Train on the RTX 5090, laptop or desktop. Every job here fits on either, which removes the memory
workarounds and leaves the choice of method to what gives the best model.

| Job | 5090 laptop, 24 GB | 5090 desktop, 32 GB |
| --- | --- | --- |
| LoRA on Qwen3 1.7B | Easy, in full precision | Easy |
| Full fine-tune of Qwen3 1.7B | Tight; needs 8-bit optimizer states | Feasible |
| LoRA on Whisper large-v3-turbo | Easy, in full precision, real batch sizes | Easy |
| Full fine-tune of Whisper large-v3-turbo | Feasible with gradient checkpointing | Feasible |
| Piper voice fine-tune | Easy | Easy |

Prefer the desktop for long runs: laptop GPUs throttle under sustained load, and a tuning run can
take many hours.

**LoRA is still the default, even with room for a full fine-tune.** The reason was never only
memory. LoRA changes far less of the model, so it is much less likely to erase what the model
already knew — English, other languages, the opposite translation direction. Reach for a full
fine-tune only if LoRA plateaus on your held-out set and you have enough data to justify it.

**One setup trap.** The 5090 is a Blackwell GPU and needs a recent PyTorch built for CUDA 12.8 or
later. Older PyTorch wheels install fine and then fail on the card, so check
`torch.cuda.is_available()` and run a tiny training step before starting a real run.

`ubox` and the 4070 laptop are not needed for any of this. The trained models become ordinary
files in `models/`, and run on whatever machine cnverc runs on.

## Code changes cnverc will need

A tuned model drops in as a folder, but three small code changes stand between that and a dialect
that actually works.

- **Language names in the translation prompt.** `language_name` in `translate.rs` only knows
  German, English, Spanish, French, Italian and Portuguese. Anything else falls through to the bare
  code, so the translator is asked to translate "into ar". Add Arabic — and the dialect name, once
  the prompt names one — before judging any Arabic translation.
- **A variety in the prompt.** The prompt names a language, not a dialect. Asking for "Iraqi
  Arabic" or "Mexican Spanish" needs the chosen variety passed into `translate.rs`.
- **The voice picker already filed in `HANDOFF.md`.** Two voices for one language have no tiebreak
  today. The descriptor parser rejects unknown fields, so `models.rs` has to declare a `variety`
  key before any `engine.toml` can carry one.

None of these are milestone work, and SPEC says build in milestone order. They are small, though,
and without the first one any Arabic measurement is testing a prompt bug rather than the model.

## Worked example: Iraqi Arabic vs Mexican Spanish

The two dialects need almost opposite work, which is why measuring first matters.

| Stage | Mexican Spanish | Iraqi Arabic |
| --- | --- | --- |
| Recognition | Stock Whisper is likely fine; measure to confirm | Likely the weak point; the stage most worth tuning |
| Translation | Prompt change probably enough | Prompt change, then likely a LoRA tune |
| Speech | Done: `es_MX-claude-high` installed | Jordanian voice as a stand-in; tuning optional |
| Code changes | Variety in the prompt; voice picker | Arabic in `language_name`; variety in the prompt |
| Rough effort | Days | Weeks |

For Spanish, the work is mostly configuration: the voice is installed, and a prompt change plus a
measurement may finish it. For Iraqi Arabic, recognition is where the payoff is, and it is also
the hardest export — so it is worth confirming the baseline is actually poor before building that
pipeline.

The existing Iraqi LoRA adapters on Hugging Face are no shortcut. They are built on
`whisper-small`, a much weaker base than the model cnverc runs, and their model cards give no
data, test set or error rate to judge them by.
