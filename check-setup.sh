#!/usr/bin/env bash
#
# check-setup.sh — is this PC ready to build cnverc?
#
# Checks the tools the build needs and says, for anything missing, exactly
# what to do about it. It changes nothing on your PC.
#
#   ./check-setup.sh
#
# Works in Git Bash on Windows and in a terminal on Linux. setup.sh runs it
# first, so you only need to run it yourself to see where you stand.

set -u

missing=0
ok()   { printf '  OK       %s\n' "$1"; }
bad()  { printf '  MISSING  %s\n' "$1"; printf '           -> %s\n' "$2"; missing=1; }
note() { printf '  NOTE     %s\n' "$1"; [ -n "${2:-}" ] && printf '           -> %s\n' "$2"; }

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  Linux)                os=linux ;;
  *) echo "This script supports Windows (Git Bash) and Linux; this is $(uname -s)."; exit 1 ;;
esac

# The Visual Studio installer's locator, which finds the Build Tools wherever
# they are installed and whichever version they are.
VSWHERE="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"

# cmake_dir: the folder holding the Build Tools' cmake.exe, or nothing.
cmake_dir() {
  [ -x "$VSWHERE" ] || return 0
  local found
  found=$("$VSWHERE" -latest -products '*' -find '**/CMake/bin/cmake.exe' 2>/dev/null | head -1 | tr -d '\r')
  [ -n "$found" ] && dirname "$(cygpath -u "$found")"
}

echo "Checking what cnverc needs to build ($os)"
echo

# --- Both ---------------------------------------------------------------------
if command -v git >/dev/null 2>&1; then
  ok "Git"
else
  if [ "$os" = windows ]; then
    bad "Git" "Install Git for Windows from https://git-scm.com/download/win"
  else
    bad "Git" "sudo apt install git"
  fi
fi

if command -v cargo >/dev/null 2>&1; then
  version=$(cargo --version | awk '{print $2}')
  major=${version%%.*}; rest=${version#*.}; minor=${rest%%.*}
  if [ "$major" -gt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -ge 95 ]; }; then
    ok "Rust (cargo $version)"
  else
    bad "Rust is $version; cnverc needs 1.95 or newer" "rustup update"
  fi
else
  if [ "$os" = windows ]; then
    bad "Rust" "Install it from https://rustup.rs (rustup-init.exe), then close and reopen Git Bash"
  else
    bad "Rust" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   then open a new terminal"
  fi
fi

# --- Windows ------------------------------------------------------------------
if [ "$os" = windows ]; then
  if [ ! -x "$VSWHERE" ]; then
    bad "Visual Studio Build Tools" \
        "Install 'Build Tools for Visual Studio' with 'Desktop development with C++' (README, Windows step 2)"
  else
    cpp=$("$VSWHERE" -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
          -property installationPath 2>/dev/null | head -1 | tr -d '\r')
    if [ -n "$cpp" ]; then
      ok "C++ compiler (Build Tools)"
    else
      bad "C++ compiler" \
          "Open 'Visual Studio Installer', choose Modify on Build Tools, tick 'Desktop development with C++'"
    fi
    if [ -n "$(cmake_dir)" ]; then
      ok "CMake (found inside the Build Tools; setup.sh uses it automatically)"
    elif command -v cmake >/dev/null 2>&1; then
      ok "CMake"
    else
      bad "CMake" \
          "Open 'Visual Studio Installer', choose Modify, and tick 'C++ CMake tools for Windows'"
    fi
  fi

  libclang=""
  for dir in "${LIBCLANG_PATH:-}" "/c/Program Files/LLVM/bin"; do
    [ -n "$dir" ] && [ -f "$dir/libclang.dll" ] && libclang="$dir" && break
  done
  if [ -n "$libclang" ]; then
    ok "LLVM (libclang in $libclang)"
  else
    bad "LLVM" \
        "Install LLVM-<version>-win64.exe from https://github.com/llvm/llvm-project/releases/latest, choosing 'Add LLVM to the system PATH'"
  fi

  if [ -n "${SHERPA_ONNX_LIB_DIR:-}" ] && [ ! -d "${SHERPA_ONNX_LIB_DIR}" ]; then
    note "SHERPA_ONNX_LIB_DIR is set to a folder that doesn't exist: $SHERPA_ONNX_LIB_DIR" \
         "setup.sh ignores it. To clear it yourself: unset SHERPA_ONNX_LIB_DIR"
  fi
fi

# --- Linux --------------------------------------------------------------------
if [ "$os" = linux ]; then
  apt_line="sudo apt install git curl build-essential cmake pkg-config libclang-dev libasound2-dev libonnxruntime-dev"
  command -v c++ >/dev/null 2>&1   && ok "C++ compiler"  || bad "C++ compiler" "$apt_line"
  command -v cmake >/dev/null 2>&1 && ok "CMake"         || bad "CMake" "$apt_line"
  command -v pkg-config >/dev/null 2>&1 && ok "pkg-config" || bad "pkg-config" "$apt_line"

  if pkg-config --exists alsa 2>/dev/null; then
    ok "ALSA sound headers"
  else
    bad "ALSA sound headers" "$apt_line"
  fi

  if ls /usr/lib/*/libclang*.so* /usr/lib/llvm-*/lib/libclang*.so* >/dev/null 2>&1; then
    ok "libclang"
  else
    bad "libclang" "$apt_line"
  fi

  if ls /usr/lib/*/libonnxruntime.so* /usr/lib/libonnxruntime.so* >/dev/null 2>&1; then
    ok "onnxruntime (system package)"
  else
    bad "onnxruntime" "$apt_line   (Ubuntu 25.04 or later packages it)"
  fi

  sherpa="${SHERPA_ONNX_LIB_DIR:-$HOME/sherpa-onnx/install/lib}"
  if [ -f "$sherpa/libsherpa-onnx-c-api.so" ]; then
    ok "sherpa-onnx speech library ($sherpa)"
  else
    bad "sherpa-onnx speech library (not in $sherpa)" "./build-sherpa-linux.sh   (once; takes about an hour)"
  fi
fi

# --- Room to work ---------------------------------------------------------------
free_kb=$(df -Pk . | awk 'NR==2 {print $4}')
if [ -n "$free_kb" ]; then
  free_gb=$((free_kb / 1024 / 1024))
  if [ "$free_gb" -ge 12 ]; then
    ok "Disk space (${free_gb} GB free here)"
  else
    note "Only ${free_gb} GB free here. The build and models need about 12 GB." \
         "Free some space, or clone cnverc onto a drive with more room"
  fi
fi

mem_gb=""
if [ "$os" = linux ] && [ -r /proc/meminfo ]; then
  mem_gb=$(awk '/MemTotal/ {printf "%d", $2 / 1024 / 1024 + 0.5}' /proc/meminfo)
elif [ "$os" = windows ]; then
  mem_gb=$(powershell.exe -NoProfile -Command \
    "[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)" 2>/dev/null | tr -d '\r')
fi
if [ -n "$mem_gb" ]; then
  if [ "$mem_gb" -ge 16 ]; then
    ok "Memory (${mem_gb} GB)"
  elif [ "$mem_gb" -ge 8 ]; then
    ok "Memory (${mem_gb} GB; enough, though Shared machine mode with Whisper turbo is happier with 16)"
  else
    note "Memory is ${mem_gb} GB. cnverc needs about 8 GB to run all its models."
  fi
fi

echo
if [ "$missing" -ne 0 ]; then
  echo "Something is missing. Do what each MISSING line says, then run this again."
  exit 1
fi
echo "Everything cnverc needs to build is here."
