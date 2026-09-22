# Dialect training pipeline: recognition + translation

As of 2026-09-21.

## Bottom line

One pipeline trains both models — a LoRA on Whisper large-v3-turbo for recognition and a LoRA on
Qwen3 1.7B for translation — and every dialect-specific choice lives in a single **dialect
profile** file. Starting with Mexican Spanish means writing `es-MX.yaml`. Moving to Iraqi Arabic
means writing `ar-IQ.yaml` and running the same seven steps.

| Step | What it does | Output |
| --- | --- | --- |
| 1. Prepare | Load, filter and split the datasets named in the profile | Clean train and test sets |
| 2. Baseline | Score stock Whisper and stock Qwen | The numbers to beat, and a go/no-go gate |
| 3. Recognition LoRA | Tune Whisper on the dialect's speech | A tuned Whisper |
| 4. Translation pairs | Turn the dialect transcripts into translation pairs with a larger model | Pairs in both directions |
| 5. Translation LoRA | Tune Qwen on those pairs | A tuned Qwen |
| 6. Export | Convert both into cnverc's formats | A model folder and a `.gguf` |
| 7. Verify | Re-measure inside cnverc | The acceptance result |

Three rules make it swappable, and they hold for every dialect:

1. **Nothing dialect-specific is written into the scripts.** Dataset names, column names,
   language codes, the scoring normalizer and the output names all come from the profile.
2. **Every score is compared to a baseline and to a general regression set.** The baseline says
   whether tuning helped; the regression set says whether it broke something else.
3. **The final score is taken inside cnverc, after export.** Compressing to int8 and Q4 can erase
   a gain that looked real in training.

## The dialect profile

This is the only file that changes between dialects. Datasets name their columns differently, so
each dataset entry maps its own column names onto the three the pipeline needs: audio, text and
speaker.

```yaml
# profiles/es-MX.yaml
id: es-MX
name: Mexican Spanish
whisper_language: es          # Whisper's language token; there are no dialect tokens

asr:
  train:
    - hf: ciempiess/ciempiess_light
      split: train
      columns: { audio: audio, text: normalized_text, speaker: speaker_id }
  test:
    - hf: ciempiess/ciempiess_test
      split: test
      columns: { audio: audio, text: normalized_text, speaker: speaker_id }  # confirm names
  regression: { hf: google/fleurs, config: es_419, split: test }   # general Latin American Spanish
  max_seconds: 30             # Whisper's window; longer clips are dropped
  normalizer: spanish         # how text is compared when scoring

mt:
  source: asr.train           # dialect sentences come from the recognition transcripts
  min_words: 5                # fragments like "para que sea" teach nothing
  teacher: <largest model that fits the 5090>
  directions: [dialect-en, en-dialect]
  regression: { hf: openlanguagedata/flores_plus, config: spa_Latn }

cnverc:
  asr_folder: whisper-large-v3-turbo-es-mx
  mt_file: qwen3-1.7b-es-mx-q4_k_m.gguf
```

The **normalizer** matters more than it looks. Word error rate compares words, so both the
model's output and the reference are cleaned the same way first — lowercase, punctuation removed,
numbers written out. For Spanish that is simple. For Arabic it is most of the work; see the
swapping section.

The **regression sets** are the same kind for every language. FLEURS has read speech in about a
hundred languages and FLORES+ has professional translations in over two hundred, so every profile
can point at its own language in both.

## Where it lives and the environment

**A separate repository, `cnverc-tune`.** Training is Python, and cnverc's own rules keep its
repository free of other toolchains. The two meet at one point: `cnverc-tune` produces a model
folder and a `.gguf`, and those are copied into cnverc's `models/`. cnverc never learns how they
were made.

```
cnverc-tune/
  profiles/            es-MX.yaml, later ar-IQ.yaml
  steps/               one script per step, each taking --profile
  runs/<id>/           everything a run produces: data, scores, checkpoints, exports
  results.md           one line per run: baseline, tuned, regression
```

**The machine is the 5090 desktop.** Laptop GPUs throttle on long runs.

The setup trap is PyTorch. The 5090 is a Blackwell GPU and needs a PyTorch built for CUDA 12.8 or
later; older builds install without complaint and then fail on the card. Around it: Hugging Face
`transformers`, `datasets` and `peft` for training, `jiwer` for word error rate, `sacrebleu` for
translation scores, and llama.cpp for the GGUF conversion.

**Check:** `torch.cuda.is_available()` is true, and a ten-step dummy LoRA run on Whisper finishes
without error. Run it before downloading anything large.

Internet is needed here, for downloading datasets and models. That doesn't touch cnverc's
no-internet rule, which is about the app at runtime.

## Step 1: prepare the data

Load every dataset in the profile, map its columns, resample to 16 kHz mono, and drop clips longer
than `max_seconds` — CIEMPIESS has clips up to 57 seconds, and Whisper only sees 30. Then restore
capitalization and punctuation to the transcripts (see Decisions, below), once, and save the
result so every later step uses the same text.

The split rule is by speaker. No speaker in the test set may appear in training, or the score
measures a memorised voice instead of the dialect.

**Check:** a report showing hours and speaker count per split, clips dropped for length, and a
speaker overlap of **zero**. If the published test set shares speakers with training, move those
speakers out of training rather than trusting the split.

## Step 2: baselines, and the go/no-go gate

Score the stock models before training anything. Four numbers:

| Score | On | Metric |
| --- | --- | --- |
| Stock Whisper, dialect | The profile's test set | Word error rate |
| Stock Whisper, general | FLEURS for the language | Word error rate |
| Stock Qwen, dialect | The translation test set built in Step 4 | chrF |
| Stock Qwen, general | FLORES+ for the language | chrF |

chrF compares translations character by character; it is steadier than BLEU on short sentences
and on languages with rich word endings, which covers both Spanish and Arabic.

**The gate** compares the two Whisper numbers. If the dialect error rate is close to the general
one, the dialect is not what's hurting recognition — skip Step 3 and put the effort into
translation. A dialect rate well above the general one is the case for tuning. Using the gap
rather than a fixed threshold is what keeps the gate meaningful across languages, since Arabic's
general error rate starts far higher than Spanish's.

Run the Qwen baseline with **cnverc's translation prompt, copied verbatim from `translate.rs`**.
A score taken with a different prompt measures a different system.

**Check:** all four numbers written to `results.md`, and a decision recorded: tune recognition, or
skip it.

## Step 3: the recognition LoRA

Tune `openai/whisper-large-v3-turbo` with LoRA in full precision — the 5090 has room, so no 8-bit
loading. Starting settings, to adjust only if the check fails:

| Setting | Start at | Why |
| --- | --- | --- |
| Rank | 32 | Room for an accent; the Iraqi adapters seen earlier also used 32 |
| Alpha | 64 | Twice the rank, the usual starting ratio |
| Target layers | Attention query, key, value and output, in encoder and decoder | Accent lives mostly in how sounds are heard, so the encoder must be included |
| Learning rate | 1e-4, warmup 50 steps | A standard LoRA rate; lower it if the regression check fails |
| Epochs | 2–3 | 18 hours is small; more epochs mostly memorise |
| Language token | From the profile, `es` | Whisper has no dialect tokens |

Hold back about 5% of training speakers as a validation set, and keep the checkpoint with the
lowest validation error rather than the last one.

On 18 hours of audio this is a few hours on the 5090.

**Check:** two numbers against the Step 2 baselines. The dialect error rate on the test set must
drop. The FLEURS error rate must **not rise by more than about one point** — if it does, the
adapter has traded general Spanish for radio Spanish. Lower the learning rate or the epochs and
run again before going further.

Keep the adapter separate at this point; merging happens in Step 6.

## Step 4: build the translation pairs

There is no ready dialect-to-English corpus, so make one. The recognition transcripts are
authentic dialect in exactly the style cnverc's translator will receive; a much larger
**teacher** model translates them into English, and the small model learns from those
translations. This is distillation.

1. Take the transcripts named by `mt.source`, drop anything under `min_words`, and remove
   duplicates.
2. Have the teacher translate each into English. Run it locally on the 5090 — the largest model
   that fits — so the data never leaves the machine.
3. **Dialect → English pairs:** transcript as the source, the teacher's English as the target.
4. **English → dialect pairs:** the same pairs reversed, so the target is real dialect written by
   real speakers. Clean that side first — have the teacher remove false starts and repeated words
   (*y y*, *que que*) while keeping every dialect word — or Qwen learns to stammer.
5. Build the **translation test set** from the test speakers' transcripts only, the same way, so
   no sentence in it was trained on.

The teacher's mistakes become Qwen's lessons, so checking it is part of the step, not an extra.

**Check:** a fluent speaker reads 50 random teacher translations from the test set and marks each
acceptable or not. Below about 90% acceptable, change the teacher before training anything; the
whole downstream result is capped by this number. Record the pair counts per direction in
`results.md`.

## Step 5: the translation LoRA

Tune `Qwen/Qwen3-1.7B` with LoRA in full precision, rank 16, alpha 32, on all linear layers, for
2–3 epochs. Two things matter more than the settings:

- **Train in cnverc's exact prompt**, copied from `translate.rs`, including its chat format and
  how it handles Qwen3's thinking block. A model trained on one prompt format and run with another
  loses most of what it learned. Done this way, the first dialect needs no change to cnverc's code
  at all.
- **Mix in general translation pairs**, around a fifth of the data, so the model keeps its
  breadth. FLORES+ **dev** can supply them; **devtest** stays untouched for scoring.

This is well under an hour on the 5090.

**Check:** chrF on the translation test set rises in **both** directions over the Step 2 baseline,
and FLORES+ devtest chrF falls by no more than about one point. Then read 20 translations by eye.
A higher score with stiffer output is a warning sign that the teacher's style, not the dialect, is
what got learned.

## Step 6: export into cnverc

**Recognition**, the four steps from `DIALECTS.md`: merge the adapter into Whisper; convert the
Hugging Face checkpoint to OpenAI's original format; run sherpa-onnx's
`scripts/whisper/export-onnx.py`, adapted to take a local checkpoint and with `dynamo=False` on
PyTorch 2.9 or later; quantize to int8. Put the encoder, decoder and tokens file in
`models/asr/<asr_folder>/` with an `engine.toml` — `backend = "whisper"`, `languages` from the
profile. It appears in the recognizer picker beside the stock Whisper, which stays installed.

**Translation**: merge the adapter into Qwen; convert with llama.cpp's `convert_hf_to_gguf.py`;
quantize to Q4_K_M. `models/mt/` holds exactly one `.gguf`, so move the stock file somewhere safe
rather than deleting it — comparing the two means swapping them by hand.

**Check:** `cnverc --report` shows the new recognizer as `ok` and finds the new `.gguf`.

## Step 7: re-measure inside cnverc

This is the acceptance test. Everything before it was measured in PyTorch at full precision;
cnverc runs int8 and Q4, and compression can quietly take a gain back.

- **Recognition:** play test-set clips through `cnverc --listen --compare` with stock and tuned
  Whisper both installed, and score both against the references with the same normalizer.
- **Translation:** re-run `bench_both_directions` and the translation test set with the tuned
  `.gguf`, then with the stock one.
- **Live:** a real conversation. Numbers can improve while the experience doesn't.

**Check:** the tuned models still beat stock inside cnverc, on both the dialect and the regression
sets. If a gain survives training but not export, the compression is eating it — try Q5_K_M or
Q8_0 for Qwen before retraining anything. Record the final numbers in `results.md`: that line is
the result of the whole run.

## Swapping to another dialect: Iraqi Arabic

The seven steps stay the same. What changes is the profile, and for Arabic, three things the
profile can't solve on its own.

```yaml
# profiles/ar-IQ.yaml — the parts that differ
id: ar-IQ
name: Iraqi Arabic
whisper_language: ar
asr:
  train: [ ... ]              # no open Iraqi speech corpus exists; see below
  regression: { hf: google/fleurs, config: ar_eg, split: test }
  normalizer: arabic
mt:
  regression: { hf: openlanguagedata/flores_plus, config: acm_Arab }
```

**1. The data doesn't exist openly.** This is the real blocker, not the pipeline. The substantial
Iraqi speech sets are paid — Appen's 50-hour conversational corpus, the TRANSTAC data — and are
8 kHz telephone audio. The options are to buy one, find Iraqi speech inside a multi-dialect set
with dialect labels, or record and transcribe your own. The translation side is better off:
FLORES+ has **Mesopotamian Arabic**, `acm_Arab`, which is Iraqi — about 2,000 professionally
translated sentences. It is news and travel register, not street speech, so it makes a good
regression and test set and a thin training set.

**2. The normalizer does real work.** Arabic has no standard spelling for dialect, so the same
spoken word appears written several ways. Before scoring, the `arabic` normalizer must strip
diacritics, unify the alef variants, and fold *ta marbuta* into *ha* and *alef maqsura* into
*ya*. Even then, report character error rate alongside word error rate; it forgives spelling
variants that word error rate punishes.

**3. cnverc needs a code change first.** `language_name` in `translate.rs` doesn't know Arabic, so
the prompt would say "into ar". Fix that before Step 2, or the baseline is measuring a prompt bug.

The gate in Step 2 behaves differently here, and it should: expect the dialect error rate to sit
far above FLEURS, which is the case for doing Step 3 at all.

## Decisions made, and what is still open

**Decided:**

- **Restore capitalization and punctuation before training.** CIEMPIESS transcripts are lowercase
  with no punctuation. Trained on them as-is, Whisper would stop punctuating, and cnverc's
  captions would lose it. Having the teacher restore case and punctuation once, in Step 1, changes
  no words — colloquial spellings like *pus* and *namás* stay — and the scores are unaffected
  because the normalizer strips both anyway. The fallback, if the teacher punctuates badly, is to
  train as-is and accept unpunctuated captions.
- **Train the translator on the reference transcripts,** not on what the recognizer produced.
  Clean input first; robustness to recognition errors is a later refinement.
- **Keep the stock models installed** beside the tuned ones, so every comparison can be repeated.
- **Merge, don't swap adapters at runtime,** for this first dialect. Runtime adapters only pay off
  once a second dialect's translator exists.

**Open:**

- **Which teacher model.** It sets the ceiling on translation quality. Candidates are the largest
  general or translation-specialised model that fits 32 GB; pick by running the Step 4 check on
  two of them.
- **Who checks the teacher.** Step 4's check needs a fluent reader of the dialect. For Mexican
  Spanish that's worth arranging before Step 4, not during it.
- **Whether `ciempiess_test` shares speakers with `ciempiess_light`.** Step 1 answers it; if they
  overlap, the published test set can't be used as-is.
- **Whether Step 3 runs at all.** The Step 2 gate decides, and for Mexican Spanish the honest
  expectation is a small recognition gain — much of it from the radio-conversation style rather
  than the accent.
