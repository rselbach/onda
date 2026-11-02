#!/usr/bin/env bash
# Exercise the GTK post-processing controls with a private desktop keyring.
set -euo pipefail

cleanup() {
  local status=$?
  if [[ -n "${keyring_pid:-}" ]]; then kill "${keyring_pid}" 2>/dev/null || true; fi
  if [[ -n "${display_pid:-}" ]]; then kill "${display_pid}" 2>/dev/null || true; fi
  if (( status != 0 )); then cat "${work}/keyring.log" >&2 || true; fi
  rm -rf "${work}"
  exit "${status}"
}

main() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  if [[ "${ONDA_TEST_SESSION:-}" != 1 ]]; then
    exec dbus-run-session \
      --config-file="${root}/shell/linux/tests/dbus-session.conf" \
      -- env ONDA_TEST_SESSION=1 "$0" "$@"
  fi
  work="$(mktemp -d)"
  trap cleanup EXIT
  export XDG_RUNTIME_DIR="${work}/runtime" XDG_CONFIG_HOME="${work}/config"
  export XDG_DATA_HOME="${work}/data" ONDA_DATA_DIR="${work}/onda"
  export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
  unset WAYLAND_DISPLAY
  mkdir -m 700 "${XDG_RUNTIME_DIR}" "${work}/keyring"
  mkdir -p "${XDG_CONFIG_HOME}" "${XDG_DATA_HOME}" "${ONDA_DATA_DIR}"
  cat > "${ONDA_DATA_DIR}/settings.json" <<'JSON'
{"onboardingCompleted":true,"didShowFirstRunSetup":true}
JSON
  Xvfb -displayfd 3 -screen 0 1440x1000x24 3> "${work}/display" \
    > "${work}/xvfb.log" 2>&1 &
  display_pid=$!
  local attempt
  for ((attempt = 0; attempt < 50; attempt++)); do
    [[ -s "${work}/display" ]] && break
    sleep 0.1
  done
  DISPLAY=":$(cat "${work}/display")"
  export DISPLAY
  dbus-update-activation-environment DISPLAY XDG_RUNTIME_DIR XDG_CONFIG_HOME \
    XDG_DATA_HOME GDK_BACKEND
  gnome-keyring-daemon --foreground --unlock --components=secrets \
    --control-directory="${work}/keyring" <<< 'Greendale' \
    > "${work}/keyring.log" 2>&1 &
  keyring_pid=$!
  local ready
  for ((attempt = 0; attempt < 50; attempt++)); do
    ready="$(gdbus call --session --dest org.freedesktop.DBus \
      --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner org.freedesktop.secrets)"
    [[ "${ready}" == *true* ]] && break
    sleep 0.1
  done
  [[ "${ready}" == *true* ]]
  local test_name=real_gui_preserves_provider_keys_and_model_selection
  if [[ -n "${ONDA_GUI_TEST_BINARY:-}" ]]; then
    "${ONDA_GUI_TEST_BINARY}" "${test_name}" --ignored --test-threads=1
  else
    cargo test --locked --manifest-path "${root}/Cargo.toml" -p onda-linux \
      "${test_name}" -- --ignored --test-threads=1
  fi
}

main "$@"
