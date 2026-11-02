#!/usr/bin/env bash
# Install the release archive for the current user or into a chosen prefix.
set -euo pipefail

main() {
  if (( $# > 1 )); then
    echo "usage: $0 [prefix (default: ~/.local)]" >&2
    return 2
  fi
  local source prefix data file relative
  source="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  prefix="${1:-${HOME}/.local}"
  data="${prefix}/share"
  if (( $# == 0 )); then
    data="${XDG_DATA_HOME:-${data}}"
  fi
  mkdir -p "${prefix}/bin" "${prefix}/lib/onda" "${data}"
  # Rename executable files so upgrades also work while Onda is running.
  install -m755 "${source}/lib/onda/libgtk4-layer-shell.so.0" \
    "${prefix}/lib/onda/libgtk4-layer-shell.so.0.next"
  mv "${prefix}/lib/onda/libgtk4-layer-shell.so.0.next" \
    "${prefix}/lib/onda/libgtk4-layer-shell.so.0"
  install -m755 "${source}/bin/onda" "${prefix}/bin/onda.next"
  mv "${prefix}/bin/onda.next" "${prefix}/bin/onda"
  while IFS= read -r -d '' file; do
    relative="${file#"${source}/share/"}"
    install -Dm644 "${file}" "${data}/${relative}"
  done < <(find "${source}/share" -type f -print0)
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database "${data}/applications"
  fi
  echo "Installed ${prefix}/bin/onda; ensure ${prefix}/bin is in PATH."
  echo "Restart Onda to use the new version."
}

main "$@"
