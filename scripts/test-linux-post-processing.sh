#!/usr/bin/env bash
# Run the real-GUI tests of the GTK shell in a private desktop session with
# its own keyring. Each test starts from a fresh data directory.
set -euo pipefail

cleanup() {
  local status=$?
  if [[ -n "${keyring_pid:-}" ]]; then kill "${keyring_pid}" 2>/dev/null || true; fi
  if [[ -n "${display_pid:-}" ]]; then kill "${display_pid}" 2>/dev/null || true; fi
  if (( status != 0 )); then cat "${work}/keyring.log" >&2 || true; fi
  rm -rf "${work}"
  exit "${status}"
}

# Runs one ignored test from a new data directory that holds only the given
# settings.
run_gui_test() {
  local test_name="$1"
  local settings="$2"
  ONDA_DATA_DIR="$(mktemp -d "${work}/onda.XXXXXX")"
  export ONDA_DATA_DIR
  printf '%s\n' "${settings}" > "${ONDA_DATA_DIR}/settings.json"
  if [[ -n "${ONDA_GUI_TEST_BINARY:-}" ]]; then
    "${ONDA_GUI_TEST_BINARY}" "${test_name}" --ignored --test-threads=1
  else
    cargo test --locked --manifest-path "${root}/Cargo.toml" -p onda-linux \
      "${test_name}" -- --ignored --test-threads=1
  fi
}

main() {
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  if [[ "${ONDA_TEST_SESSION:-}" != 1 ]]; then
    exec dbus-run-session \
      --config-file="${root}/shell/linux/tests/dbus-session.conf" \
      -- env ONDA_TEST_SESSION=1 "$0" "$@"
  fi
  work="$(mktemp -d)"
  trap cleanup EXIT
  export XDG_RUNTIME_DIR="${work}/runtime" XDG_CONFIG_HOME="${work}/config"
  export XDG_DATA_HOME="${work}/data"
  export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
  # the model that replaces a removed one depends on the system language;
  # without one it is Parakeet Ultra
  export LANG=C.UTF-8
  unset WAYLAND_DISPLAY
  mkdir -m 700 "${XDG_RUNTIME_DIR}" "${work}/keyring"
  mkdir -p "${XDG_CONFIG_HOME}" "${XDG_DATA_HOME}"
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
  local set_up='"onboardingCompleted":true,"didShowFirstRunSetup":true'
  local removed='"selectedModel":"parakeet-ja"'
  local declined='"declinedModelMigrationNoticeKey":"parakeet-ja->parakeet-ultra"'
  local shown='"lastModelMigrationNoticeKey":"parakeet-ja->parakeet-ultra"'
  run_gui_test real_gui_preserves_provider_keys_and_model_selection \
    "{${set_up}}"
  run_gui_test real_gui_closing_the_update_notice_declines_the_download \
    "{${set_up},${removed}}"
  run_gui_test real_gui_choose_another_opens_the_speech_page \
    "{${set_up},${removed}}"
  # a declined update keeps the app quiet: no notice and no download
  run_gui_test real_gui_shows_a_critical_alert_as_a_modal_dialog \
    "{${set_up},${removed},${declined},${shown}}"
  run_gui_test real_gui_update_notice_sends_each_answer_once "{${set_up}}"
}

main "$@"
