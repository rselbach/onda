#!/usr/bin/env bash
# Build the GTK4/libadwaita Linux shell.
set -euo pipefail

main() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  local flags=()
  case "${1:-debug}" in
    debug) ;;
    release) flags+=(--release) ;;
    *) echo "usage: $0 [debug|release]" >&2; return 2 ;;
  esac
  cargo build --locked --manifest-path "${root}/Cargo.toml" \
    -p onda-linux "${flags[@]}"
}

main "$@"
