#!/usr/bin/env bash
# Run every ignored test of the GTK shell, the tests that drive the real
# window, in a private desktop session with its own display, bus, and
# keyring. Each test runs in its own process and data directory. A local
# proxy refuses all network traffic, and the run fails if a test tried to
# reach the host of the speech models.
set -euo pipefail

readonly MODEL_HOST='huggingface.co'

cleanup() {
  local status=$?
  if [[ -n "${proxy_pid:-}" ]]; then kill "${proxy_pid}" 2>/dev/null || true; fi
  if [[ -n "${keyring_pid:-}" ]]; then kill "${keyring_pid}" 2>/dev/null || true; fi
  if [[ -n "${display_pid:-}" ]]; then kill "${display_pid}" 2>/dev/null || true; fi
  if (( status != 0 )); then
    cat "${work}/keyring.log" "${work}/requests" >&2 2> /dev/null || true
  fi
  rm -rf "${work}"
  exit "${status}"
}

wait_for_file() {
  local attempt
  for ((attempt = 0; attempt < 50; attempt++)); do
    [[ -s "$1" ]] && return 0
    sleep 0.1
  done
  echo "Timed out waiting for $1" >&2
  return 1
}

# Passes its arguments to the test harness of the GTK shell.
gui_tests() {
  if [[ -n "${ONDA_GUI_TEST_BINARY:-}" ]]; then
    "${ONDA_GUI_TEST_BINARY}" "$@"
    return
  fi
  cargo test --locked --manifest-path "${root}/Cargo.toml" -p onda-linux \
    -- "$@"
}

# Adds a line to the request log through the proxy. The proxy answers in
# order, so every request made before this call is in the log on return.
mark_requests() {
  local reply
  exec 4<> "/dev/tcp/127.0.0.1/${proxy_port}"
  printf '# %s\r\n' "$1" >&4
  reply="$(cat <&4)"
  exec 4>&-
  if [[ "${reply}" != *502* ]]; then
    echo 'The proxy did not answer' >&2
    return 1
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
  # the harness prints one "<name>: test" line for each test
  local listing
  listing="$(gui_tests --list --ignored)"
  local -a tests
  mapfile -t tests < <(sed -n 's/: test$//p' <<< "${listing}")
  if (( ${#tests[@]} == 0 )); then
    echo "Found no ignored test in the GTK shell" >&2
    exit 1
  fi
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
  wait_for_file "${work}/display"
  DISPLAY=":$(cat "${work}/display")"
  export DISPLAY
  dbus-update-activation-environment DISPLAY XDG_RUNTIME_DIR XDG_CONFIG_HOME \
    XDG_DATA_HOME GDK_BACKEND
  gnome-keyring-daemon --foreground --unlock --components=secrets \
    --control-directory="${work}/keyring" <<< 'Greendale' \
    > "${work}/keyring.log" 2>&1 &
  keyring_pid=$!
  local attempt ready
  for ((attempt = 0; attempt < 50; attempt++)); do
    ready="$(gdbus call --session --dest org.freedesktop.DBus \
      --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner org.freedesktop.secrets)"
    [[ "${ready}" == *true* ]] && break
    sleep 0.1
  done
  [[ "${ready}" == *true* ]]
  python3 "${root}/shell/linux/tests/refusing-proxy.py" "${work}/requests" \
    "${work}/proxy-port" > "${work}/proxy.log" 2>&1 &
  proxy_pid=$!
  wait_for_file "${work}/proxy-port"
  proxy_port="$(cat "${work}/proxy-port")"
  # cargo has fetched and built everything by now, so only the tests meet
  # the proxy
  local proxy="http://127.0.0.1:${proxy_port}"
  export HTTPS_PROXY="${proxy}" https_proxy="${proxy}" HTTP_PROXY="${proxy}"
  export http_proxy="${proxy}" ALL_PROXY="${proxy}" all_proxy="${proxy}"
  unset NO_PROXY no_proxy
  local name
  for name in "${tests[@]}"; do
    ONDA_DATA_DIR="$(mktemp -d "${work}/onda.XXXXXX")"
    export ONDA_DATA_DIR
    gui_tests "${name}" --exact --ignored --test-threads=1
    mark_requests "end of ${name}"
    if grep -qF "${MODEL_HOST}" "${work}/requests"; then
      echo "${name} tried to reach ${MODEL_HOST}, the host of the speech" \
        "models. Start it from settings that begin no download." >&2
      exit 1
    fi
  done
  echo 'Requests of each test, above its line. The proxy refused them all:'
  cat "${work}/requests"
  # a log without requests would mean the tests bypass the proxy, and the
  # check above would prove nothing
  if ! grep -qv '^#' "${work}/requests"; then
    echo 'No test request reached the proxy, so the run cannot show that' \
      "no test reached ${MODEL_HOST}." >&2
    exit 1
  fi
}

main "$@"
