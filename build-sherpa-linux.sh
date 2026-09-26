#!/usr/bin/env bash
#
# build-sherpa-linux.sh — build the speech library cnverc needs on Linux. Once.
#
#   ./build-sherpa-linux.sh
#
# On Windows the cnverc build downloads this library ready-made. The
# ready-made Linux one crashes as soon as it loads a model ("free(): invalid
# pointer"), so on Linux it is built here from source, against the system's
# onnxruntime, as a shared library. It takes about an hour and is only needed
# once per PC. TECHNICAL.md, "Linux: shared sherpa-onnx", explains why.
#
# It installs to ~/sherpa-onnx/install, which is where setup.sh looks.
# Already built? It says so and stops. To build again: --rebuild.

set -u

VERSION="v1.13.8"   # must match the sherpa-onnx crate in Cargo.toml
SRC="$HOME/sherpa-onnx"
PREFIX="$HOME/sherpa-onnx/install"
LIB="$PREFIX/lib/libsherpa-onnx-c-api.so"

say()  { printf '\n=== %s\n' "$1"; }
stop() { printf '\n!!! %s\n' "$1" >&2; [ -n "${2:-}" ] && printf '    %s\n' "$2" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || stop "This is only for Linux. On Windows, run ./setup.sh: the build fetches this library itself."

if [ -f "$LIB" ] && [ "${1:-}" != "--rebuild" ]; then
  echo "The speech library is already built: $LIB"
  echo "Next: ./setup.sh   (To build it again anyway: ./build-sherpa-linux.sh --rebuild)"
  exit 0
fi

apt_line="sudo apt install git cmake build-essential libonnxruntime-dev"
for tool in git cmake c++; do
  command -v "$tool" >/dev/null 2>&1 || stop "$tool is not installed." "$apt_line"
done
ls /usr/lib/*/libonnxruntime.so* /usr/lib/libonnxruntime.so* >/dev/null 2>&1 ||
  stop "The system onnxruntime isn't installed; building without it produces a library that crashes." \
       "$apt_line   (Ubuntu 25.04 or later packages it)"

say "1/3  Getting sherpa-onnx $VERSION into $SRC"
if [ -d "$SRC/.git" ]; then
  have=$(git -C "$SRC" describe --tags --exact-match 2>/dev/null || echo "something else")
  if [ "$have" != "$VERSION" ]; then
    git -C "$SRC" fetch --depth 1 origin tag "$VERSION" &&
    git -C "$SRC" checkout -q "$VERSION" ||
      stop "$SRC holds sherpa-onnx at $have and couldn't be switched to $VERSION." \
           "Move $SRC out of the way and run this again."
  fi
else
  [ -e "$SRC" ] && [ ! -d "$SRC/install" ] &&
    stop "$SRC exists but isn't a sherpa-onnx checkout." "Move it out of the way and run this again."
  if [ -d "$SRC" ]; then
    # Only an install folder from an earlier build is there; clone around it.
    git -C "$SRC" init -q && git -C "$SRC" remote add origin https://github.com/k2-fsa/sherpa-onnx.git &&
    git -C "$SRC" fetch --depth 1 origin tag "$VERSION" && git -C "$SRC" checkout -q "$VERSION" ||
      stop "Couldn't download sherpa-onnx. Check the internet connection and run this again."
  else
    git clone --depth 1 --branch "$VERSION" https://github.com/k2-fsa/sherpa-onnx.git "$SRC" ||
      stop "Couldn't download sherpa-onnx. Check the internet connection and run this again."
  fi
fi

say "2/3  Configuring"
log="$SRC/cnverc-configure.log"
cmake -S "$SRC" -B "$SRC/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DSHERPA_ONNX_ENABLE_PYTHON=OFF \
  -DSHERPA_ONNX_ENABLE_TESTS=OFF \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" 2>&1 | tee "$log"
[ "${PIPESTATUS[0]}" -eq 0 ] || stop "cmake failed; the lines above say why."

# The check that matters: it must use the system onnxruntime. If cmake fell
# back to downloading its own, the result crashes later, so stop now.
if grep -q "Downloading pre-compiled onnxruntime" "$log" ||
   ! grep -q "location_onnxruntime_lib: /usr/lib" "$log"; then
  stop "sherpa-onnx is not using the system onnxruntime, and would build a library that crashes." \
       "$apt_line   then: ./build-sherpa-linux.sh --rebuild"
fi
echo "    using the system onnxruntime: good"

say "3/3  Building (about an hour; two jobs at a time, so it doesn't run out of memory)"
cmake --build "$SRC/build" -j2 --target install || stop "The build failed; the lines above say why."
[ -f "$LIB" ] || stop "The build finished but $LIB isn't there."

echo
echo "The speech library is built: $LIB"
echo "Next: ./setup.sh"
