#!/usr/bin/env bash
#
# Builds the Rust core as a static library, generates its Swift bindings, and
# packages both as shell/macos/Generated/OndaFFI.xcframework for SwiftPM.
#
# Usage: scripts/build-ffi.sh [debug|release]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly SHELL_DIR="${ROOT}/shell/macos"
readonly GENERATED_DIR="${SHELL_DIR}/Generated"
readonly SWIFT_OUT="${SHELL_DIR}/Sources/OndaCore/Generated"
readonly TARGET="aarch64-apple-darwin"
# keep the static library's objects, including cc-built C code, at the app's
# minimum macOS (Info.plist LSMinimumSystemVersion)
MACOSX_DEPLOYMENT_TARGET="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' \
  "${SHELL_DIR}/Info.plist")"
export MACOSX_DEPLOYMENT_TARGET
STAGING=""

# keeps the failing status: bash 3.2 (macOS's /bin/bash) otherwise exits with
# the trap's last status, so a failed build would report success
cleanup() {
  local status=$?
  if [[ -n "${STAGING}" ]]; then
    rm -rf "${STAGING}"
  fi
  exit "${status}"
}

main() {
  local profile="${1:-debug}"
  local cargo_flags=()
  case "${profile}" in
    debug) ;;
    release) cargo_flags+=(--release) ;;
    *)
      echo "usage: $0 [debug|release]" >&2
      exit 2
      ;;
  esac

  local lib_dir="${ROOT}/target/${profile}"
  if [[ "$(uname -m)" != "arm64" ]]; then
    cargo_flags+=(--target "${TARGET}")
    lib_dir="${ROOT}/target/${TARGET}/${profile}"
  fi
  STAGING="$(mktemp -d)"
  trap cleanup EXIT

  cargo build --manifest-path "${ROOT}/Cargo.toml" -p onda-ffi \
    ${cargo_flags[@]+"${cargo_flags[@]}"}

  cargo run --quiet --manifest-path "${ROOT}/Cargo.toml" -p uniffi-bindgen -- \
    generate --library "${lib_dir}/libonda_ffi.dylib" \
    --language swift --out-dir "${STAGING}"

  mkdir -p "${STAGING}/headers" "${SWIFT_OUT}" "${GENERATED_DIR}"
  mv "${STAGING}/onda_ffiFFI.h" "${STAGING}/headers/"
  mv "${STAGING}/onda_ffiFFI.modulemap" "${STAGING}/headers/module.modulemap"
  if ! cmp -s "${STAGING}/onda_ffi.swift" "${SWIFT_OUT}/onda_ffi.swift"; then
    mv "${STAGING}/onda_ffi.swift" "${SWIFT_OUT}/onda_ffi.swift"
  fi

  rm -rf "${GENERATED_DIR}/OndaFFI.xcframework"
  xcodebuild -create-xcframework \
    -library "${lib_dir}/libonda_ffi.a" \
    -headers "${STAGING}/headers" \
    -output "${GENERATED_DIR}/OndaFFI.xcframework" >/dev/null
  echo "built ${GENERATED_DIR}/OndaFFI.xcframework (${profile})"
}

main "$@"
