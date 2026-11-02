#!/usr/bin/env bash
# Exercise a real GTK window, virtual microphone, inference, and X11 paste.
set -euo pipefail

cleanup() {
  local status=$?
  if [[ -n "${app_pid:-}" ]]; then kill "${app_pid}" 2>/dev/null || true; fi
  if [[ -n "${target_pid:-}" ]]; then kill "${target_pid}" 2>/dev/null || true; fi
  if [[ -n "${pulse_pid:-}" ]]; then kill "${pulse_pid}" 2>/dev/null || true; fi
  if [[ -n "${wireplumber_pid:-}" ]]; then kill "${wireplumber_pid}" 2>/dev/null || true; fi
  if [[ -n "${pipewire_pid:-}" ]]; then kill "${pipewire_pid}" 2>/dev/null || true; fi
  if [[ -n "${display_pid:-}" ]]; then kill "${display_pid}" 2>/dev/null || true; fi
  if (( status != 0 )); then
    cat "${work}/onda.log" >&2 || true
    cat "${work}/pulse.log" >&2 || true
    cat "${work}/pipewire.log" >&2 || true
    cat "${work}/wireplumber.log" >&2 || true
  fi
  rm -rf "${work}"
  exit "${status}"
}

wait_ready() {
  local status
  for ((attempt = 0; attempt < 300; attempt++)); do
    status="$("${binary}" --status 2>/dev/null || true)"
    if [[ "${status}" == *ready* || "${status}" == "Ready" ]]; then
      return 0
    fi
    sleep 0.2
  done
  echo "Onda did not become ready" >&2
  return 1
}

check_microphone_released() {
  pw-dump > "${work}/audio-state.json"
  python3 - "${work}/audio-state.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as source:
    nodes = json.load(source)
for node in nodes:
    info = node.get("info", {})
    if info.get("props", {}).get("media.class") == "Stream/Input/Audio":
        if info.get("state") not in ("idle", "suspended"):
            sys.exit("Onda kept the microphone stream active")
PY
}

main() {
  if (( $# != 3 )); then
    echo "usage: $0 /path/to/onda /path/to/v2-Q8_0.gguf /path/to/jfk.wav" >&2
    return 2
  fi
  local root binary model sample
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  if [[ "${ONDA_TEST_SESSION:-}" != 1 ]]; then
    exec dbus-run-session \
      --config-file="${root}/shell/linux/tests/dbus-session.conf" \
      -- env ONDA_TEST_SESSION=1 "$0" "$@"
  fi
  binary="$(realpath "$1")"
  model="$(realpath "$2")"
  sample="$(realpath "$3")"
  work="$(mktemp -d)"
  trap cleanup EXIT
  export XDG_RUNTIME_DIR="${work}/runtime" XDG_CONFIG_HOME="${work}/config"
  export XDG_DATA_HOME="${work}/data" ONDA_DATA_DIR="${work}/onda"
  export XDG_STATE_HOME="${work}/state"
  export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
  unset WAYLAND_DISPLAY
  mkdir -m 700 "${XDG_RUNTIME_DIR}"
  mkdir -p "${XDG_CONFIG_HOME}" "${XDG_DATA_HOME}" \
    "${ONDA_DATA_DIR}/Models/parakeet-tdt-v2"
  ln -s "${model}" \
    "${ONDA_DATA_DIR}/Models/parakeet-tdt-v2/parakeet-tdt-0.6b-v2-Q8_0.gguf"
  cat > "${ONDA_DATA_DIR}/settings.json" <<'JSON'
{"onboardingCompleted":true,"didShowFirstRunSetup":true,"selectedModel":"parakeet-tdt-v2","soundFeedbackEnabled":false,"pauseMediaDuringRecording":false}
JSON
  Xvfb -displayfd 3 -screen 0 1024x768x24 3> "${work}/display" \
    > "${work}/xvfb.log" 2>&1 &
  display_pid=$!
  for ((attempt = 0; attempt < 50; attempt++)); do
    [[ -s "${work}/display" ]] && break
    sleep 0.1
  done
  DISPLAY=":$(cat "${work}/display")"
  export DISPLAY
  dbus-update-activation-environment DISPLAY XDG_RUNTIME_DIR XDG_CONFIG_HOME \
    XDG_DATA_HOME GDK_BACKEND
  export PIPEWIRE_RUNTIME_DIR="${XDG_RUNTIME_DIR}" PIPEWIRE_REMOTE=pipewire-0
  export PIPEWIRE_CONFIG_DIR="${work}/pipewire"
  unset PIPEWIRE_CONFIG_PREFIX PIPEWIRE_CONFIG_NAME
  export PULSE_RUNTIME_PATH="${XDG_RUNTIME_DIR}/pulse"
  export PULSE_SERVER="unix:${PULSE_RUNTIME_PATH}/native"
  mkdir -p "${PIPEWIRE_CONFIG_DIR}"
  local config
  for config in pipewire pipewire-pulse client; do
    cp "/usr/share/pipewire/${config}.conf" "${PIPEWIRE_CONFIG_DIR}/"
  done
  mkdir -p "${XDG_CONFIG_HOME}/wireplumber/wireplumber.conf.d"
  cat > "${XDG_CONFIG_HOME}/wireplumber/wireplumber.conf.d/onda-test.conf" <<'CONF'
wireplumber.profiles = {
  main = {
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez-midi = disabled
    monitor.v4l2 = disabled
    monitor.libcamera = disabled
  }
}
CONF
  pipewire > "${work}/pipewire.log" 2>&1 &
  pipewire_pid=$!
  for ((attempt = 0; attempt < 50; attempt++)); do
    [[ -S "${PIPEWIRE_RUNTIME_DIR}/pipewire-0" ]] && break
    sleep 0.1
  done
  [[ -S "${PIPEWIRE_RUNTIME_DIR}/pipewire-0" ]]
  wireplumber > "${work}/wireplumber.log" 2>&1 &
  wireplumber_pid=$!
  pipewire-pulse > "${work}/pulse.log" 2>&1 &
  pulse_pid=$!
  for ((attempt = 0; attempt < 50; attempt++)); do
    pactl info > /dev/null 2>&1 && break
    sleep 0.1
  done
  pactl load-module module-null-sink sink_name=onda-test \
    rate=16000 channels=1 > /dev/null
  export ALSA_CONFIG_PATH="${work}/alsa.conf"
  cat > "${ALSA_CONFIG_PATH}" <<'CONF'
pcm.!default {
  type pipewire
  server "pipewire-0"
  capture_node "onda-test"
}
CONF
  local gtk_flags
  gtk_flags="$(pkg-config --cflags --libs gtk4)"
  local flags=()
  read -r -a flags <<< "${gtk_flags}"
  cc "${root}/shell/linux/tests/paste-target.c" -o "${work}/target" \
    "${flags[@]}"
  if "${binary}" --stop > /dev/null 2>&1; then
    echo "A control command incorrectly started Onda" >&2
    return 1
  fi
  PIPEWIRE_PROPS='{ stream.capture.sink = true }' \
    "${binary}" > "${work}/onda.log" 2>&1 &
  app_pid=$!
  wait_ready
  "${binary}" --start
  "${binary}" --start
  sleep 0.4
  "${binary}" --cancel
  wait_ready
  "${work}/target" "${work}/transcript" > "${work}/target.log" 2>&1 &
  target_pid=$!
  local target_window
  target_window="$(xdotool search --sync --name 'Greendale dictation target' | head -1)"
  xdotool windowfocus --sync "${target_window}"
  printf 'Greendale clipboard' | xclip -selection clipboard -in
  "${binary}" --start
  sleep 0.5
  paplay --device=onda-test "${sample}"
  sleep 0.3
  "${binary}" --stop
  "${binary}" --stop
  for ((attempt = 0; attempt < 600; attempt++)); do
    if [[ -s "${work}/transcript" ]] && \
      rg -qi 'ask not|country' "${work}/transcript"; then break; fi
    sleep 0.2
  done
  rg -qi 'ask not|country' "${work}/transcript"
  wait_ready
  [[ "$(xclip -selection clipboard -out)" == 'Greendale clipboard' ]]
  sleep 9
  check_microphone_released
  xdotool key ctrl+a BackSpace
  xdotool keydown alt+space
  sleep 0.5
  paplay --device=onda-test "${sample}"
  xdotool keyup alt+space
  for ((attempt = 0; attempt < 600; attempt++)); do
    if [[ -s "${work}/transcript" ]] && \
      rg -qi 'ask not|country' "${work}/transcript"; then break; fi
    sleep 0.2
  done
  rg -qi 'ask not|country' "${work}/transcript"
  wait_ready
  sleep 9
  check_microphone_released
  "${binary}"
  kill -0 "${app_pid}"
  if [[ -n "${ONDA_TEST_SCREENSHOT:-}" ]]; then
    magick import -window root "${ONDA_TEST_SCREENSHOT}"
  fi
  "${binary}" --quit
  wait "${app_pid}"
  app_pid=""
  echo "PASS: commands, cancellation, global shortcut, CPU transcription, GTK paste, clipboard restoration."
}

main "$@"
