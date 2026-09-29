# Instructions: two command-line additions to volis, for training

> **Completed** (M7.8, with M7.9 added for model-bench). Kept for history. What was built
> is in ARCHITECTURE.md (`cli.rs`). Built with three improvements this file didn't ask
> for: logs go to stderr for all three commands, stdin and stdout are UTF-8 whatever the
> console, and the prompt's stray spaces are gone.

For Claude Code, in the **volis** repo. Read `CLAUDE.md` and `SPEC.md` first. Their rules apply
in full.

## Why

A separate project, `model-converter`, trains the Qwen translator on dialect sentence pairs and
converts it to the `.gguf` volis runs. It needs two things only volis can provide:

1. **The exact prompt volis sends.** A model trained on one prompt layout and run with another
   loses most of what it learned. The prompt is built in `src/translate.rs` (`prompt_for`,
   `system_prompt`) and changes with the source and target varieties. Copying it into Python by
   hand would drift the next time `translate.rs` changes.
2. **A way to translate a file through volis itself,** so a newly trained model is scored the way
   volis will actually run it: the same compression, the same decoding, and the same three
   refusal guards.

## This is a milestone

volis's `CLAUDE.md` says to build in milestone order and add nothing no milestone needs. The
user has approved this as a new milestone. Add it to `SPEC.md` §13 as **M7.8 — Training
support commands**, after M7.7, in the same style as M7.5 to M7.7, with this file as its reason.
Don't renumber M8 or M9. Record it in `HANDOFF.md` and the status list in `CLAUDE.md`. When done,
move this file to `docs/plans/`, like the other finished build instructions.

## What to add

Both go in `src/cli.rs` as new commands, next to `--report` and `--devices`, and in its `HELP`
text.

### `volis --print-prompt <source> <target>`

Prints exactly what `prompt_for` would send for that source and target, with the user's text
replaced by the literal placeholder `{text}`, then exits.

- It must call `prompt_for` itself (make it `pub(crate)` if needed). **Don't** build a second
  copy of the prompt. The whole point is that there's only one.
- Tags go through the same validation `translate` uses. An unknown tag is an error naming the
  tag, and the exit code is not zero.
- Print nothing else to stdout, so a script can capture it as-is. Logs go to stderr.

### `volis --translate <source> <target>`

Reads UTF-8 text from stdin, one sentence per line. Translates each line with the model in
`models/mt/`, exactly as a live session would. Prints one line to stdout per input line, in the
same order.

- **All three refusal guards stay on:** echo, reciting the prompt, and implausibly long output.
  A refused line prints as an **empty line**, so the line numbers still match, and the reason
  goes to stderr with the line number.
- An empty input line gives an empty output line.
- Load the model once, not per line.
- A missing model follows the existing rule: print the absolute path that was expected, and exit
  with a non-zero code.

## Rules that apply

- No network access of any kind. Neither command needs it.
- No new config keys, settings or abstractions. `cargo fmt` and
  `cargo clippy --all-targets -- -D warnings` clean.
- No `unwrap()` or `expect()` on these paths.
- Tests: `--print-prompt` output equals `prompt_for`'s for a plain language (`en` → `es`) and a
  variety (`en` → `es-MX`). CLI parsing tests for both commands, including a missing or unknown
  tag.

## Check

- `volis --print-prompt en es-MX` prints the prompt, including the dialect sentence and the empty
  `<think></think>` block, and nothing else.
- `echo "My phone died." | volis --translate en es-MX` prints one Spanish line.
- A file of five lines, one of them empty, gives five output lines.
- `volis --print-prompt en xx-XX` fails with a clear message and a non-zero exit code.
