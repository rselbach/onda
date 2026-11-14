#!/usr/bin/env bash
# Exercise the recording overlay's X11 window in a private Xvfb display.
set -euo pipefail

cleanup() {
  local status=$?
  if [[ -n "${display_pid:-}" ]]; then kill "${display_pid}" 2>/dev/null || true; fi
  rm -rf "${work}"
  exit "${status}"
}

main() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  work="$(mktemp -d)"
  trap cleanup EXIT
  export XDG_RUNTIME_DIR="${work}/runtime" XDG_CONFIG_HOME="${work}/config"
  export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
  unset WAYLAND_DISPLAY
  mkdir -m 700 "${XDG_RUNTIME_DIR}"
  mkdir -p "${XDG_CONFIG_HOME}"
  Xvfb -displayfd 3 -screen 0 1024x768x24 3> "${work}/display" \
    > "${work}/xvfb.log" 2>&1 &
  display_pid=$!
  local attempt
  for ((attempt = 0; attempt < 50; attempt++)); do
    [[ -s "${work}/display" ]] && break
    sleep 0.1
  done
  DISPLAY=":$(cat "${work}/display")"
  export DISPLAY
  local test_name=x11_overlay_shows_each_state_and_cancels_without_taking_focus
  if [[ -n "${ONDA_GUI_TEST_BINARY:-}" ]]; then
    "${ONDA_GUI_TEST_BINARY}" "${test_name}" --ignored --test-threads=1
  else
    cargo test --locked --manifest-path "${root}/Cargo.toml" -p onda-linux \
      "${test_name}" -- --ignored --test-threads=1
  fi
}

main "$@"
