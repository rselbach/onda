#!/usr/bin/env bash
# Install a built Onda binary and its desktop identity for the current user.
set -euo pipefail

main() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  local binary="${1:-${root}/target/release/onda}"
  local data="${XDG_DATA_HOME:-${HOME}/.local/share}"
  local destination="${HOME}/.local/bin/onda"
  if [[ ! -x "${binary}" ]]; then
    echo "Build first: scripts/build-linux.sh release" >&2
    return 1
  fi
  install -Dm755 "${binary}" "${destination}.next"
  mv "${destination}.next" "${destination}"
  install -Dm644 "${root}/shell/linux/com.rselbach.onda.desktop" \
    "${data}/applications/com.rselbach.onda.desktop"
  install -Dm644 \
    "${root}/assets/AppIcon.appiconset/icon_256x256.png" \
    "${data}/icons/hicolor/256x256/apps/com.rselbach.onda.png"
  install -Dm644 -t "${data}/onda/sounds" "${root}"/assets/sounds/*.wav
  install -Dm644 "${root}/shell/linux/licenses/hex.txt" \
    "${data}/doc/onda/hex-license.txt"
  echo "Installed ${destination}; ensure ${HOME}/.local/bin is in PATH."
}

main "$@"
