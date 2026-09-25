#!/usr/bin/env bash
#
# setup-models.sh — download every model cnverc needs.
#
# Run it once, from the cnverc folder, on a PC with internet:
#
#   ./setup-models.sh
#
# By default it fills target/release/models. To fill the development copy
# instead, pass the folder the executable is in:
#
#   ./setup-models.sh target/debug
#
# It is safe to run again: anything already downloaded is skipped, so if it
# stops partway through, run it again and it picks up where it left off.
#
# About 2.4 GB in total. Works in Git Bash on Windows and in a Linux shell.

set -u

DEST="${1:-target/release}"
MODELS="$DEST/models"
DL="downloads"

BASE_ASR="https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"
BASE_TTS="https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models"
BASE_MT="https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main"

failed=0

say()  { printf '\n=== %s\n' "$1"; }
warn() { printf '!!! %s\n' "$1" >&2; failed=1; }

# fetch <url> <output path>   — skips a file that is already there
fetch() {
  if [ -s "$2" ]; then
    printf '    already have %s\n' "$2"
    return 0
  fi
  curl -L --fail --retry 3 -o "$2.part" "$1" || { warn "download failed: $1"; rm -f "$2.part"; return 1; }
  mv "$2.part" "$2"
}

# unpack <archive> <destination folder>
unpack() {
  tar -xjf "$1" -C "$2" || { warn "could not extract $1"; return 1; }
}

if [ ! -d "$DEST" ]; then
  warn "$DEST does not exist. Build cnverc first (cargo build --release)."
  exit 1
fi

if [ ! -d "$MODELS" ]; then
  warn "$MODELS does not exist. Run this from the cnverc folder, after:  cp -r models cnverc.toml $DEST/"
  exit 1
fi

mkdir -p "$MODELS/vad" "$MODELS/mt" "$MODELS/tts" "$DL"

say "Voice detector (2 MB) — notices when someone starts and stops talking"
fetch "$BASE_ASR/silero_vad.onnx" "$MODELS/vad/silero_vad.onnx"

say "Translator (1.1 GB) — Qwen3 1.7B"
fetch "$BASE_MT/Qwen3-1.7B-Q4_K_M.gguf" "$MODELS/mt/qwen3-1.7b-q4_k_m.gguf"

say "Speech recognizer: Whisper (564 MB)"
if fetch "$BASE_ASR/sherpa-onnx-whisper-turbo.tar.bz2" "$DL/whisper.tar.bz2"; then
  unpack "$DL/whisper.tar.bz2" "$DL" &&
  cp "$DL/sherpa-onnx-whisper-turbo/turbo-encoder.int8.onnx" \
     "$DL/sherpa-onnx-whisper-turbo/turbo-decoder.int8.onnx" \
     "$DL/sherpa-onnx-whisper-turbo/turbo-tokens.txt" \
     "$MODELS/asr/whisper-large-v3-turbo/" || warn "could not copy the Whisper files"
fi

say "Speech recognizer: Parakeet (487 MB)"
if fetch "$BASE_ASR/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2" "$DL/parakeet.tar.bz2"; then
  unpack "$DL/parakeet.tar.bz2" "$DL" &&
  cp "$DL/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/encoder.int8.onnx" \
     "$DL/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/decoder.int8.onnx" \
     "$DL/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/joiner.int8.onnx" \
     "$DL/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/tokens.txt" \
     "$MODELS/asr/parakeet-tdt-0.6b-v3-int8/" || warn "could not copy the Parakeet files"
fi

say "English voice (64 MB)"
if [ -d "$MODELS/tts/vits-piper-en_US-lessac-medium" ] &&
   [ -s "$MODELS/tts/vits-piper-en_US-lessac-medium/en_US-lessac-medium.onnx" ]; then
  printf '    already have the English voice\n'
else
  fetch "$BASE_TTS/vits-piper-en_US-lessac-medium.tar.bz2" "$DL/voice-en.tar.bz2" &&
  unpack "$DL/voice-en.tar.bz2" "$MODELS/tts"
fi

say "Spanish voice (25 MB)"
if [ -d "$MODELS/tts/vits-piper-es_ES-carlfm-x_low" ] &&
   [ -s "$MODELS/tts/vits-piper-es_ES-carlfm-x_low/es_ES-carlfm-x_low.onnx" ]; then
  printf '    already have the Spanish voice\n'
else
  fetch "$BASE_TTS/vits-piper-es_ES-carlfm-x_low.tar.bz2" "$DL/voice-es.tar.bz2" &&
  unpack "$DL/voice-es.tar.bz2" "$MODELS/tts"
fi

say "Mexican Spanish voice (67 MB) — a second Spanish voice, higher quality"
if [ -d "$MODELS/tts/vits-piper-es_MX-claude-high" ] &&
   [ -s "$MODELS/tts/vits-piper-es_MX-claude-high/es_MX-claude-high.onnx" ]; then
  printf '    already have the Mexican Spanish voice\n'
else
  fetch "$BASE_TTS/vits-piper-es_MX-claude-high.tar.bz2" "$DL/voice-es-mx.tar.bz2" &&
  unpack "$DL/voice-es-mx.tar.bz2" "$MODELS/tts"
fi

printf '\n'
if [ "$failed" -ne 0 ]; then
  printf 'Some downloads did not finish. Run this script again to retry just those.\n'
  exit 1
fi

printf 'All seven downloaded into %s\n' "$MODELS"
printf 'Now check them with:   %s/cnverc --report\n' "$DEST"
printf '(on Windows: %s/cnverc.exe --report)\n' "$DEST"
printf 'When the report is clean you can delete the %s folder.\n' "$DL"
