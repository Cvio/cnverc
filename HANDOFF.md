# Handoff: where Volis stands, for the next session

Last updated 2026-09-27. Read this first, then `CLAUDE.md` (the hard constraints, the
working rules, and which document holds what), then `SPEC.md` (the build specification).

## 1. What Volis is

An offline speech-to-speech translator in Rust. Microphone → Silero VAD → speech recognition
(Parakeet or Whisper, via sherpa-onnx) → translation (Qwen3 1.7B GGUF, via llama.cpp) → speech
(Piper voices, via sherpa-onnx) → speakers, with captions in a native egui window. It
**never touches the internet**. Everything resolves from the folder the executable is in.
The project's acceptance test (Milestone 9) is: zip the folder, unzip it on a freshly imaged
Windows PC with no internet, double-click, and it works.

Windows is the primary target. SPEC §3 calls Linux "a nice-to-have; do not let it complicate
the Windows path." Any Linux fix must leave the Windows build and its static-CRT setup exactly
as they are.

## 2. Status

- **M0–M7 are complete** and checked by the user: capture, VAD, both recognizers,
  translation, speech, the window, continuous and turn-based modes, the Space turn key, and
  paired mode.
- **M7's check passed on 2026-09-20** between the Windows PC and `ubox`: a turn-based
  Spanish/English conversation with translations spoken on the far side, and an abruptly
  killed link force-releasing the floor with a clear message on both ends. It was run over
  Wi-Fi, disabling the adapter rather than pulling a cable; the detection is the same either
  way, since both are silent failures caught only by missed pings.
- **M7.5 (shared-machine mode), M7.6 (a recognizer per side) and M7.7 (varieties/dialects)
  are built and merged to `main`.** Their checks are by hand and haven't all been run; see
  section 5. How they work is in `ARCHITECTURE.md`, "Shared machine mode, in depth", and
  `MODELS.md`.
- **Linux runs**, including the window. The Start crash is fixed with a Linux-only shared
  sherpa-onnx build; see TECHNICAL.md.
- M8 (streaming ASR) and M9 (portability acceptance) haven't started.

## 3. Where everything else is

- How the code works, module by module, with the Shared and Paired internals and the dev
  workflow: [ARCHITECTURE.md](ARCHITECTURE.md).
- The models and how to add one: [MODELS.md](MODELS.md).
- Build internals (static CRT, the Linux shared sherpa-onnx build and its crash story,
  build-time internet) and design decisions: [TECHNICAL.md](TECHNICAL.md).
- Setup for users: [README.md](README.md); troubleshooting: [USE-CASES.md](USE-CASES.md#when-something-goes-wrong).

Things not written down elsewhere:

- **`ubox`** (the user's Linux PC): Ubuntu 26.04, 10 GB RAM, repo at
  `~/Desktop/projects/cnverc`. Build with `-j2`: a fully parallel build ran it out of memory and
  took Claude Code down with it. If ufw is on, paired mode needs `sudo ufw allow 47800/tcp` and
  `sudo ufw allow 47801/udp`.
- **Not a bug:** a Spanish clip captioned `[en]` and then refused by the echo guard. Volis
  doesn't detect language; the clip didn't match the configured source, and the guard caught
  the Spanish-to-Spanish "translation" as designed.
- **For an agent's shell:** long heredocs in the Bash tool break with "unexpected EOF". Write
  multi-line edit scripts to a file first.

## 4. Working with this user

- When they **ask a question, answer it and wait.** Don't run commands or change their
  `volis.toml` unasked; they have said so explicitly.
- They test live and report back. Give them a concrete test to run, then read the logs in
  `logs/` next to the exe.
- Commit at milestone boundaries, or when asked. Keep `cargo fmt` and
  `cargo clippy --all-targets -- -D warnings` clean. The user pushes, or asks for a push.
- Build in milestone order, and don't start M8 before M7's check passes.

## 5. Next steps

- **M7.8 and M7.9** are on the `model-bench-work` branch: `--print-prompt`, `--translate`,
  `--transcribe`, Persian, and Whisper listing Arabic and Persian. Built and checked on the build
  PC; the user will test on another machine before merging. The branch also removes 14 stray
  spaces from the dialect sentence of the translation prompt (a mangled line continuation from
  M7.7), which had to be fixed before any translator is trained or benchmarked on the prompt.

0. **The rename to Volis (2026-09-27)** is new: program `volis.exe`, settings `volis.toml`, logs
   `volis.log.<date>`, Rust package `volis`. `setup.sh` renames an existing `cnverc.toml` and
   removes the old `cnverc.exe`. The GitHub repo is still `Cvio/cnverc`; the README clones it
   into a `volis` folder. If the user renames the repo, update the two clone lines in README.md.
   If the rename causes trouble, it is one branch (`rename-volis`) to revert; an install that
   already ran `setup.sh` would then need `volis.toml` renamed back to `cnverc.toml` by hand.

1. Run the M7.5, M7.6 and M7.7 checks by hand (`SPEC.md` §13): both directions heard in the
   right language with speakers up, the other key dead during a turn, Escape cancelling;
   Parakeet on one side and a Spanish Whisper on the other; a side set to Spanish (Mexico)
   offering the Mexico-tuned voice first; two PCs pairing on protocol 2; and turn-based,
   continuous and paired mode re-tested. The log's `transcribing as "<lang>"` lines must match
   the keys pressed. Then mark them complete in `CLAUDE.md`.
2. Wait for the go-ahead on M8 (streaming ASR). Don't start it unasked.

Open from M7.5, not blocking: Shared mode keeps two copies of the Whisper model in memory,
one per language, because the in-place language change sherpa-onnx has isn't bound by its Rust
crates.

Open, small: `setup-models.sh` re-extracts the Whisper and Parakeet archives on every run when
they're already downloaded (slow, harmless). It should skip them when the installed model files
are present, as it does for voices.

Nothing is outstanding from M0–M7. The `[en]`-captioned Spanish clip in section 3 is **not** a
bug and needs no work: Volis doesn't detect language, the clip simply didn't match the
configured `source`, and the echo guard behaved correctly.

**Closed (M7.7): the voice picker.** Two Spanish voices both declaring `languages = ["es"]`
had no tiebreak. Voices now carry `varieties` in `engine.toml`, and `models::rank` orders them
(tuned for the variety, then general, then other varieties), so the choice follows the language
setting's variety; Shared mode also has a per-side voice picker. See MODELS.md.

**Open from M7.6–M7.7:**
- A dialect-tuned translator: the dialect prompt alone barely changes Qwen3 1.7B's output (the
  `dialect_pairs` test).
- The translator folder holds a bare `.gguf` with no `engine.toml`, so a translator can't yet
  declare `varieties` or be labelled in a dropdown. That needs a descriptor for `models/mt/`,
  deliberately left for when a dialect-tuned translator exists.

Also filed, not started, none of them urgent - three defects in the **peer panel**, found on
2026-09-20 while pairing the Windows PC and `ubox` over a direct Ethernet cable. The pairing
itself worked throughout; none of these stop a connection.

1. **The address list omits an interface that works.** On the Windows PC, `peer::local_addresses()`
   returned the Wi-Fi address and two IPv6 addresses but never the Ethernet adapter's
   `169.254.49.46`, while `discovery.rs` was receiving broadcasts over that same interface and
   listing `ubox` correctly. So `if_addrs::get_if_addrs()` is returning a partial result on
   Windows rather than failing. Without discovery the user would have had no way to learn the
   address to type. That machine has two ExpressVPN adapters in a "Not Present" state, which is
   the obvious suspect but unconfirmed. Related but separate: line ~1057's `.unwrap_or_default()`
   turns a failed enumeration into an empty list, so a real error becomes indistinguishable from
   "no interfaces" - latent here, since the call succeeded.

2. **"No network connection" is asserted while connected.** The `addresses.is_empty()` branch in
   `gui::peer_panel` prints "No network connection. Plug in a cable or join a network, then
   Rescan" - and did so with a live pairing shown two lines above it. An empty list means Volis
   found no addresses, not that the machine has no network; the wording should say that, and the
   message should be suppressed entirely when `session.peer` is connected. As written it would
   send someone to re-seat a working cable.

3. **IPv6 addresses are offered as something to type.** The panel listed
   `2600:4040:273b:b600:2909:d8fe:c14e:93b2` under "The other PC types one of these." They are
   global, not link-local, so they pass the existing filter, but nobody is going to type one.
   Show IPv4 only, or sort it first and de-emphasise the rest.

Also worth knowing for any future two-machine test: with Wi-Fi left on, discovery finds the
other PC on **both** paths (`169.254.x.x` over the cable and `192.168.1.x` over Wi-Fi) and
clicking the wrong one silently pairs over Wi-Fi, so a cable test proves nothing. Turn Wi-Fi
off first.

Done on 2026-09-19: the Linux crash (TECHNICAL.md), and the README's `SHERPA_ONNX_LIB_DIR` note.
Done on 2026-09-20: M7's check, over Wi-Fi and again over a direct Ethernet cable with no
router, no DHCP and nothing upstream (SPEC §2's "WAN cable unplugged" case, proven); the README
rewritten around `setup-models.sh` with a troubleshooting table; the ARM64 rpath fix.
