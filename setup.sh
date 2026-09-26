#!/usr/bin/env bash
#
# setup.sh — build cnverc and get it ready to run.
#
# Run it from the cnverc folder, on a PC with internet:
#
#   ./setup.sh
#
# It checks your tools, builds cnverc, puts its settings file and model
# folders next to the program, downloads the models, and checks the result.
#
# Safe to run again at any time, and it is how you update: after `git pull`,
# run ./setup.sh again. Your own settings (cnverc.toml) are never overwritten.
#
# Works in Git Bash on Windows and in a terminal on Linux.

set -u
cd "$(dirname "$0")" || exit 1

DEST="target/release"

say()  { printf '\n=== %s\n' "$1"; }
stop() { printf '\n!!! %s\n' "$1" >&2; [ -n "${2:-}" ] && printf '    %s\n' "$2" >&2; exit 1; }

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) os=windows; exe="$DEST/cnverc.exe" ;;
  Linux)                os=linux;   exe="$DEST/cnverc" ;;
  *) stop "This script supports Windows (Git Bash) and Linux; this is $(uname -s)." ;;
esac

# --- 1. Tools ---------------------------------------------------------------------
say "1/5  Checking your tools"
./check-setup.sh || stop "Install what check-setup.sh listed as MISSING, then run ./setup.sh again."

# --- 2. Build ----------------------------------------------------------------------
say "2/5  Building cnverc (the first time takes 10-20 minutes)"
jobs=()
if [ "$os" = windows ]; then
  # A running cnverc holds its own file, and the build fails with "Access is
  # denied" when it tries to replace it.
  if tasklist //FI "IMAGENAME eq cnverc.exe" 2>/dev/null | grep -qi cnverc.exe; then
    stop "cnverc is running. Close its window, then run ./setup.sh again."
  fi
  # The Build Tools' CMake isn't on PATH; find it, whichever version is installed.
  vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
  cmake_exe=$("$vswhere" -latest -products '*' -find '**/CMake/bin/cmake.exe' 2>/dev/null | head -1 | tr -d '\r')
  if [ -n "$cmake_exe" ]; then
    PATH="$PATH:$(dirname "$(cygpath -u "$cmake_exe")")"
    export PATH
  fi
  # On Windows the build downloads the speech library itself. A leftover
  # SHERPA_ONNX_LIB_DIR pointing nowhere would stop it.
  if [ -n "${SHERPA_ONNX_LIB_DIR:-}" ] && [ ! -d "$SHERPA_ONNX_LIB_DIR" ]; then
    echo "    ignoring SHERPA_ONNX_LIB_DIR ($SHERPA_ONNX_LIB_DIR): that folder doesn't exist"
    unset SHERPA_ONNX_LIB_DIR
  fi
else
  # The speech library built by build-sherpa-linux.sh.
  export SHERPA_ONNX_LIB_DIR="${SHERPA_ONNX_LIB_DIR:-$HOME/sherpa-onnx/install/lib}"
  # A fully parallel build of the translator can run out of memory.
  jobs=(-j2)
fi

cargo build --release "${jobs[@]}" ||
  stop "The build failed. The last lines above say why." \
       "The table 'If something goes wrong' in README.md covers the usual causes."
[ -f "$exe" ] || stop "The build finished but $exe isn't there."

# --- 3. Settings file and model folders ---------------------------------------------
say "3/5  Putting the settings file and model folders next to cnverc"
# File by file, never replacing one that is already there: that would throw
# away your settings, or a model description you edited.
added=0
kept_different=()
while IFS= read -r -d '' source; do
  target="$DEST/$source"
  if [ ! -e "$target" ]; then
    mkdir -p "$(dirname "$target")"
    cp "$source" "$target"
    added=$((added + 1))
  elif ! cmp -s "$source" "$target"; then
    kept_different+=("$source")
  fi
done < <(find models cnverc.toml -type f -print0)
echo "    added $added file(s)"
if [ "${#kept_different[@]}" -gt 0 ]; then
  echo "    kept yours, although the version in the cnverc folder is different:"
  for file in "${kept_different[@]}"; do
    echo "      $DEST/$file"
  done
  echo "    (Your cnverc.toml differing is normal: it holds your choices. To take a"
  echo "     new version of any other file, copy it over yourself.)"
fi

# --- 4. Models ------------------------------------------------------------------------
say "4/5  Downloading the models (about 2.4 GB the first time; skipped if already here)"
./setup-models.sh "$DEST" || stop "Some models didn't download. Run ./setup.sh again to retry them."

# --- 5. Check -----------------------------------------------------------------------
say "5/5  Checking that cnverc finds everything"
report=$("$exe" --report 2>&1)
status=$?
echo "$report" | sed -n '/^summary:/,$p'
if [ "$status" -ne 0 ] || echo "$report" | grep -q -e 'DISABLED' -e '\[!\]'; then
  echo "$report"
  stop "cnverc is missing something (see DISABLED or [!] above)." \
       "Run ./setup.sh again. If it keeps happening, see 'If something goes wrong' in README.md."
fi

echo
echo "Everything is in place. Start cnverc with:"
echo
echo "    ./$exe"
echo
if [ "$os" = windows ]; then
  echo "or double-click cnverc.exe in the $DEST folder."
fi
