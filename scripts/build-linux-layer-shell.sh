#!/usr/bin/env bash
# Build the private Layer Shell library used by Linux release packages.
set -euo pipefail

main() {
  local root work source prefix archive
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  work="${root}/build/linux-layer-shell"
  source="${work}/gtk4-layer-shell-1.3.0"
  prefix="${work}/install"
  archive="${work}/source.tar.gz"
  mkdir -p "${work}"
  curl --fail --location --retry 3 \
    https://codeload.github.com/wmww/gtk4-layer-shell/tar.gz/refs/tags/v1.3.0 \
    --output "${archive}"
  local checksum
  checksum=1ebb01ab14e98afd1727f68f64981c37bd23305b1f131f5667c02b94cf593192
  echo "${checksum}  ${archive}" | sha256sum --check
  tar -xzf "${archive}" -C "${work}"
  meson setup --wipe "${work}/build" "${source}" \
    --prefix "${prefix}" --libdir lib --buildtype release \
    -Dexamples=false -Ddocs=false -Dtests=false \
    -Dintrospection=false -Dvapi=false
  meson compile -C "${work}/build"
  meson install -C "${work}/build"
  install -Dm644 "${source}/LICENSE" \
    "${prefix}/share/licenses/gtk4-layer-shell/LICENSE"
  echo "Layer Shell installed in ${prefix}"
}

main "$@"
