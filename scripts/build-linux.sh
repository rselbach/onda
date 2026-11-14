#!/usr/bin/env bash
# Build the GTK4/libadwaita Linux shell.
set -euo pipefail

# The speech library compiles with -mcpu=native unless told otherwise. GCC
# rejects that flag in ARM64 virtual machines on Apple Silicon, where the CPU
# reports SME without SVE2.
readonly PORTABLE_ARM_CMAKE_ARGS="-DGGML_NATIVE=OFF -DGGML_CPU_ARM_ARCH=armv8-a"

compiler_rejects_native_cpu() {
  local compiler
  read -ra compiler <<< "${CC:-cc}"
  [[ "$(uname -m)" == "aarch64" ]] \
    && ! "${compiler[@]}" -mcpu=native -E - < /dev/null > /dev/null 2>&1
}

main() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  local flags=()
  case "${1:-debug}" in
    debug) ;;
    release) flags+=(--release) ;;
    *) echo "usage: $0 [debug|release]" >&2; return 2 ;;
  esac
  if [[ -z "${TRANSCRIBE_CMAKE_ARGS+set}" ]] \
    && compiler_rejects_native_cpu; then
    export TRANSCRIBE_CMAKE_ARGS="${PORTABLE_ARM_CMAKE_ARGS}"
    echo "$0: the compiler rejects -mcpu=native, so the speech library" \
      "builds with TRANSCRIBE_CMAKE_ARGS=\"${TRANSCRIBE_CMAKE_ARGS}\"" >&2
  fi
  cargo build --locked --manifest-path "${root}/Cargo.toml" \
    -p onda-linux "${flags[@]}"
}

main "$@"
