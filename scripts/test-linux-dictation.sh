#!/usr/bin/env bash
# Exercise a real GTK window, virtual microphone, inference, X11 paste, the
# tray menu, media pause and mute, and sound cues against a private bus and
# audio server.
set -euo pipefail

cleanup() {
  local status=$?
  if [[ -n "${app_pid:-}" ]]; then kill "${app_pid}" 2>/dev/null || true; fi
  if [[ -n "${target_pid:-}" ]]; then kill "${target_pid}" 2>/dev/null || true; fi
  if [[ -n "${other_pid:-}" ]]; then kill "${other_pid}" 2>/dev/null || true; fi
  if [[ -n "${music_pid:-}" ]]; then kill "${music_pid}" 2>/dev/null || true; fi
  if [[ -n "${player_pid:-}" ]]; then kill "${player_pid}" 2>/dev/null || true; fi
  if [[ -n "${toggle_pid:-}" ]]; then kill "${toggle_pid}" 2>/dev/null || true; fi
  if [[ -n "${tray_pid:-}" ]]; then kill "${tray_pid}" 2>/dev/null || true; fi
  if [[ -n "${recorder_pid:-}" ]]; then kill "${recorder_pid}" 2>/dev/null || true; fi
  if [[ -n "${pulse_pid:-}" ]]; then kill "${pulse_pid}" 2>/dev/null || true; fi
  if [[ -n "${wireplumber_pid:-}" ]]; then kill "${wireplumber_pid}" 2>/dev/null || true; fi
  if [[ -n "${pipewire_pid:-}" ]]; then kill "${pipewire_pid}" 2>/dev/null || true; fi
  if [[ -n "${display_pid:-}" ]]; then kill "${display_pid}" 2>/dev/null || true; fi
  if (( status != 0 )); then
    cat "${work}/onda.log" >&2 || true
    cat "${work}/tray" >&2 || true
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

wait_until() {
  local what="$1"
  shift
  local attempt
  for ((attempt = 0; attempt < 100; attempt++)); do
    if "$@"; then return 0; fi
    sleep 0.1
  done
  echo "Timed out waiting for ${what}" >&2
  return 1
}

write_settings() {
  cat > "${ONDA_DATA_DIR}/settings.json" <<JSON
{"onboardingCompleted":true,"didShowFirstRunSetup":true,"selectedModel":"parakeet-tdt-v2","selectedLanguage":"en","recentLanguages":"en,pt","soundFeedbackEnabled":${2:-false},"pauseMediaDuringRecording":$1}
JSON
}

start_onda() {
  PIPEWIRE_PROPS='{ stream.capture.sink = true }' \
    "${binary}" "$@" >> "${work}/onda.log" 2>&1 &
  app_pid=$!
  wait_ready
}

tray() {
  python3 "${tray_host}" "$@"
}

tray_shows() {
  tray show > "${work}/tray"
  rg -qx -- "$1" "${work}/tray"
}

tray_is() {
  tray show > "${work}/tray"
  [[ "$(cat "${work}/tray")" == "$1" ]]
}

tray_watcher_ready() {
  gdbus call --session --dest org.freedesktop.DBus \
    --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner org.kde.StatusNotifierWatcher \
    | rg -q true
}

target_is_empty() {
  [[ ! -s "${work}/transcript" ]]
}

# No window manager runs here, so the test names the active window as one
# would.
activate() {
  xdotool windowfocus --sync "$1"
  xprop -root -f _NET_ACTIVE_WINDOW 32x -set _NET_ACTIVE_WINDOW "$1"
}

transcript_copied() {
  xclip -selection clipboard -out 2> /dev/null | rg -qi 'ask not|country'
}

onda_window_visible() {
  [[ -n "$(xdotool search --onlyvisible --name '^Onda$' 2> /dev/null)" ]]
}

player() {
  gdbus call --session --dest "org.mpris.MediaPlayer2.$1" \
    --object-path /org/mpris/MediaPlayer2 --method "$2" "${@:3}"
}

player_is() {
  local status
  status="$(player "$1" org.freedesktop.DBus.Properties.Get \
    org.mpris.MediaPlayer2.Player PlaybackStatus)"
  [[ "${status}" == *"'$2'"* ]]
}

calls() {
  tr '\n' ' ' < "$1"
}

speakers_are_default() {
  pactl set-default-sink onda-speakers 2> /dev/null \
    && [[ "$(pactl get-default-sink)" == 'onda-speakers' ]]
}

speakers_muted() {
  [[ "$(LC_ALL=C pactl get-sink-mute onda-speakers)" == 'Mute: yes' ]]
}

speakers_audible() {
  [[ "$(LC_ALL=C pactl get-sink-mute onda-speakers)" == 'Mute: no' ]]
}

overlay_window() {
  local windows
  windows="$(xdotool search --onlyvisible --name '^onda-recording$' \
    2> /dev/null || true)"
  printf '%s\n' "${windows%%$'\n'*}"
}

overlay_shown() {
  [[ -n "$(overlay_window)" ]]
}

overlay_hidden() {
  [[ -z "$(overlay_window)" ]]
}

target_received() {
  rg -q "$1" "${work}/transcript"
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
  # an unpackaged binary finds its cue files in the user's data folder
  install -Dm644 -t "${XDG_DATA_HOME}/onda/sounds" \
    "${root}"/assets/sounds/*.wav
  mkdir -p "${XDG_DATA_HOME}/applications"
  cat > "${XDG_DATA_HOME}/applications/com.rselbach.onda.TestTarget.desktop" <<'ENTRY'
[Desktop Entry]
Type=Application
Name=Greendale Dictation Target
Exec=true
Icon=accessories-text-editor
ENTRY
  write_settings true
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
  # media plays through its own default sink, so a mute never reaches the
  # virtual microphone; the default can be set once WirePlumber is up
  pactl load-module module-null-sink sink_name=onda-speakers > /dev/null
  wait_until 'the default sink' speakers_are_default
  python3 - "${work}/music.wav" <<'PY'
import math
import struct
import sys
import wave

second = b"".join(
    struct.pack("<h", int(8000 * math.sin(math.tau * 400 * n / 8000)))
    for n in range(8000)
)
with wave.open(sys.argv[1], "wb") as tone:
    tone.setnchannels(1)
    tone.setsampwidth(2)
    tone.setframerate(8000)
    tone.writeframes(second * 900)
PY
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
  tray_host="${root}/shell/linux/tests/tray-host.py"
  tray watch &
  tray_pid=$!
  wait_until 'the tray watcher' tray_watcher_ready
  start_onda
  local menu_tail
  menu_tail='---
Open Onda
Settings…
---
Check for Updates…
---
Quit Onda'
  wait_until 'the idle tray menu' tray_is "icon audio-input-microphone-symbolic
tooltip Onda, ready to dictate
[Ready to Dictate — Ctrl+Alt+Space]
Start Dictation
[Paste Last Transcript]
---
Language: English
  ✓ English
  · Portuguese
  ---
  Other Language…
${menu_tail}"
  tray click 'Language: English' 'Portuguese'
  wait_until 'Portuguese' tray_shows '  ✓ Portuguese'
  tray_shows 'Language: Portuguese'
  rg -q '"selectedLanguage": "pt"' "${ONDA_DATA_DIR}/settings.json"
  tray click 'Language: Portuguese' 'English'
  wait_until 'English' tray_shows 'Language: English'
  tray click 'Start Dictation'
  "${binary}" --start
  wait_until 'the recording tray menu' tray_is "icon media-record-symbolic
tooltip Onda, recording
[Recording…]
Stop Dictation
Cancel Dictation
[Paste Last Transcript]
---
Language: English
  [✓ English]
  [· Portuguese]
  ---
  Other Language…
${menu_tail}"
  tray click 'Cancel Dictation'
  wait_ready
  "${work}/target" "${work}/transcript" > "${work}/target.log" 2>&1 &
  target_pid=$!
  local target_window
  target_window="$(xdotool search --sync --name 'Greendale dictation target' | head -1)"
  xdotool windowfocus --sync "${target_window}"
  printf 'Greendale clipboard' | xclip -selection clipboard -in
  "${binary}" --start
  wait_until 'the recording overlay' overlay_shown
  # the cancel button, in the overlay's own coordinates
  xdotool mousemove --window "$(overlay_window)" 378 52 click 1
  wait_until 'the cancel button to end the recording' overlay_hidden
  wait_ready
  [[ "$(xdotool getwindowfocus)" == "${target_window}" ]]
  xdotool type 'Troy Barnes'
  wait_until 'typing to reach the target' target_received 'Troy Barnes'
  xdotool key ctrl+a BackSpace
  python3 "${root}/shell/linux/tests/mpris-player.py" \
    greendale "${work}/greendale.log" &
  player_pid=$!
  wait_until 'the media player' player_is greendale Playing
  pw-play "${work}/music.wav" > "${work}/music.log" 2>&1 &
  music_pid=$!
  "${binary}" --start
  wait_until 'the player to pause' player_is greendale Paused
  sleep 0.5
  paplay --device=onda-test "${sample}"
  sleep 0.3
  speakers_audible
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
  rg -q '"appName": "Greendale Dictation Target"' \
    "${ONDA_DATA_DIR}"/History/*/metadata.json
  rg -q '"appBundleID": "com.rselbach.onda.TestTarget"' \
    "${ONDA_DATA_DIR}"/History/*/metadata.json
  wait_until 'the player to resume' player_is greendale Playing
  [[ "$(calls "${work}/greendale.log")" == 'Pause Play ' ]]
  sleep 9
  check_microphone_released
  player greendale org.mpris.MediaPlayer2.Player.Pause > /dev/null
  : > "${work}/greendale.log"
  tray_shows 'Paste Last Transcript'
  local recent
  recent="$(awk '/^Recent Transcripts$/ { getline; sub(/^  /, ""); print }' \
    "${work}/tray")"
  [[ -n "${recent}" ]]
  xdotool key ctrl+a BackSpace
  wait_until 'the cleared target' target_is_empty
  tray click 'Recent Transcripts' "${recent}"
  wait_until 'the recent transcript' rg -qi 'ask not|country' "${work}/transcript"
  xdotool key ctrl+a BackSpace
  wait_until 'the cleared target' target_is_empty
  "${work}/target" "${work}/elsewhere" > "${work}/elsewhere.log" 2>&1 &
  other_pid=$!
  local other_window
  other_window="$(xdotool search --sync --all --pid "${other_pid}" \
    --name 'Greendale dictation target' | head -1)"
  activate "${target_window}"
  "${binary}" --start
  wait_until 'the mute' speakers_muted
  sleep 0.5
  paplay --device=onda-test "${sample}"
  "${binary}" --stop
  activate "${other_window}"
  for ((attempt = 0; attempt < 600; attempt++)); do
    if transcript_copied; then break; fi
    sleep 0.2
  done
  transcript_copied
  wait_ready
  rg -q 'the focus left the target' "${work}/onda.log"
  target_is_empty
  [[ ! -s "${work}/elsewhere" ]]
  kill "${other_pid}"
  wait "${other_pid}" || true
  other_pid=""
  xprop -root -remove _NET_ACTIVE_WINDOW
  xdotool windowfocus --sync "${target_window}"
  xdotool keydown ctrl+alt+space
  wait_until 'the mute' speakers_muted
  sleep 0.5
  paplay --device=onda-test "${sample}"
  speakers_muted
  xdotool keyup ctrl+alt+space
  wait_until 'the unmute' speakers_audible
  for ((attempt = 0; attempt < 600; attempt++)); do
    if [[ -s "${work}/transcript" ]] && \
      rg -qi 'ask not|country' "${work}/transcript"; then break; fi
    sleep 0.2
  done
  rg -qi 'ask not|country' "${work}/transcript"
  wait_ready
  player_is greendale Paused
  [[ ! -s "${work}/greendale.log" ]]
  sleep 9
  check_microphone_released
  "${binary}"
  kill -0 "${app_pid}"
  if [[ -n "${ONDA_TEST_SCREENSHOT:-}" ]]; then
    magick import -window root "${ONDA_TEST_SCREENSHOT}"
  fi
  "${binary}" --quit
  wait "${app_pid}"
  start_onda
  python3 "${root}/shell/linux/tests/mpris-player.py" --ignore-pause \
    troy "${work}/troy.log" &
  toggle_pid=$!
  wait_until 'the second media player' player_is troy Playing
  "${binary}" --start
  wait_until 'the toggle to pause' player_is troy Paused
  "${binary}" --cancel
  wait_until 'the second player to resume' player_is troy Playing
  [[ "$(calls "${work}/troy.log")" == 'Pause PlayPause Play ' ]]
  player_is greendale Paused
  [[ ! -s "${work}/greendale.log" ]]
  kill "${toggle_pid}"
  wait "${toggle_pid}" || true
  toggle_pid=""
  wait_ready
  "${binary}" --start
  wait_until 'the mute' speakers_muted
  [[ "$(cat "${ONDA_DATA_DIR}/muted-output")" == 'onda-speakers' ]]
  kill -KILL "${app_pid}"
  wait "${app_pid}" || true
  speakers_muted
  write_settings false
  start_onda --background
  speakers_audible
  [[ ! -e "${ONDA_DATA_DIR}/muted-output" ]]
  player greendale org.mpris.MediaPlayer2.Player.Play > /dev/null
  : > "${work}/greendale.log"
  "${binary}" --start
  sleep 3
  player_is greendale Playing
  [[ ! -s "${work}/greendale.log" ]]
  speakers_audible
  "${binary}" --cancel
  wait_ready
  if onda_window_visible; then
    echo "Onda showed its window on a background start" >&2
    return 1
  fi
  tray click 'Settings…'
  wait_until 'the Onda window' onda_window_visible
  tray click 'Quit Onda'
  wait "${app_pid}"
  kill "${music_pid}"
  wait "${music_pid}" || true
  music_pid=""
  player greendale org.mpris.MediaPlayer2.Player.Pause > /dev/null
  write_settings true true
  start_onda
  parecord --device=onda-speakers.monitor --rate=44100 --channels=1 \
    --format=s16le --file-format=wav --latency-msec=20 "${work}/heard.wav" &
  recorder_pid=$!
  sleep 1
  "${binary}" --start
  sleep 1
  # the start cue is Onda's own sound, not another application to mute
  speakers_audible
  paplay --device=onda-test "${sample}"
  "${binary}" --stop
  wait_ready
  sleep 2
  kill "${recorder_pid}"
  wait "${recorder_pid}" || true
  recorder_pid=""
  python3 "${root}/shell/linux/tests/cues-heard.py" "${work}/heard.wav" \
    "${root}/assets/sounds/dictation_start.wav" \
    "${root}/assets/sounds/dictation_stop.wav"
  "${binary}" --quit
  wait "${app_pid}"
  app_pid=""
  echo "PASS: commands, cancellation, overlay cancel button, global shortcut, CPU transcription, GTK paste, clipboard restoration, target application, target left, tray menu, media pause, output mute, mute recovery, sound cues."
}

main "$@"
