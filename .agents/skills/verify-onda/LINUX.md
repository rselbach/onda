# Verify Onda on Linux

A Linux run is the real `onda` binary inside a private desktop session that
`verify-onda` builds and tears down. The session has its own D-Bus session
bus, Xvfb display, keyring, PipeWire server with a virtual microphone,
accessibility bus, tray watcher, and scratch home and data directories. A
proxy refuses every network request. The run keeps screenshots, logs,
accessible trees, and a TSV of actions as evidence.

Read [features/README.md](features/README.md) and the file for the feature
under test before driving. Each feature file ends with an `On Linux` section.
Run the Rust tests and `scripts/test-linux-gui.sh` separately.

## One sequence, start to finish

```bash
VERIFY=".agents/skills/verify-onda/scripts/verify-onda"
RUN_ID="tray-$(date +%H%M%S)"
"${VERIFY}" launch "${RUN_ID}"
"${VERIFY}" wait-ready "${RUN_ID}"
"${VERIFY}" menu "${RUN_ID}" list
"${VERIFY}" wait-window "${RUN_ID}" main --gone
"${VERIFY}" menu "${RUN_ID}" press "Settings…"
"${VERIFY}" wait-window "${RUN_ID}" main
"${VERIFY}" ui "${RUN_ID}" tree --save settings
"${VERIFY}" screenshot "${RUN_ID}" settings
"${VERIFY}" cleanup "${RUN_ID}"
```

It launches Onda in isolation, prints the tray menu, shows that no window
is open, presses "Settings…", confirms that the window mapped on General,
and removes the run.

## What a run cannot touch

The helper is safe by construction. These hold without any care from you:

- **The desktop session.** Every command reaches the run through one
  function that starts from an empty environment and adds only the run's own.
  The helper also drops `DISPLAY`, `WAYLAND_DISPLAY`,
  `DBUS_SESSION_BUS_ADDRESS`, and `XDG_RUNTIME_DIR` from its own process
  first. The run's bus starts no desktop service.
- **Your data.** `HOME`, every `XDG_*` directory, and `ONDA_DATA_DIR` point
  under `/tmp/onda-verify-runs/<run-id>/`.
- **The real keyring.** `org.freedesktop.secrets` on the run's bus is a
  keyring daemon that the run started, with an empty store in the scratch
  home.
- **Real audio devices.** The run's PipeWire loads no hardware monitor, and
  the only ALSA device is the virtual microphone. Onda's log shows
  `Invalid CTL hw:0` when it looks for hardware.
- **The network.** Model downloads, update checks, model listings, and
  cleanup requests all fail at the proxy, and `requests.log` names each one.
  Onda begins a 740 MB download by itself in two states: started with
  onboarding done and no model, and at the wizard's Speech Model step. Both
  die at the proxy with nothing written.
- **Browsers and file managers.** "Open" actions land in `opened.log`.

`doctor` proves these from the running process, not from what `launch` meant
to pass.

## Rules

- Drive a run only through the helper. Never start `onda` by hand, and never
  export a display or bus address to reach a run. `exec RUN_ID -- COMMAND`
  runs any command inside the run instead.
- Runs do not collide. Each has its own display, bus, and audio server, so
  several agents can verify at once. Each run holds a 370 MB copy of the
  binary in `/tmp`, which is memory, so clean up when you finish.
- Never type a real API key. The key would land in the scratch keyring and
  the request would be refused, so a real key proves nothing. Use
  `sk-greendale-human-being`.
- Dev models are linked from `~/.cache/onda-dev-models`, not copied. Never
  edit or delete a file there. The helper never downloads a model.
- Use Community fixtures for anything you type: `Troy Barnes studies at
  Greendale Community College.`

## Launch

Run from the repository root with a new run ID per attempt:

```bash
VERIFY=".agents/skills/verify-onda/scripts/verify-onda"
RUN_ID="tray-$(date +%H%M%S)"
"${VERIFY}" launch "${RUN_ID}"
"${VERIFY}" wait-ready "${RUN_ID}"
```

`launch` runs `scripts/build-linux.sh debug` (log in `<evidence>/build.log`),
copies the binary to the run so a rebuild cannot swap it, writes
`settings.json` with onboarding completed, links `parakeet-tdt-v2`, starts
the session, and starts `onda --background`. A background start shows no
window, as a login start does. It prints the PID, the data directory, the
evidence directory, the display, and a `not_covered` line.

Options:

- `--reuse` skips the build and uses the current `target/debug/onda`.
- `--fresh` leaves onboarding pending and links no model. The tray then
  reads `Onda, finish setup to dictate` and the `Set Up Onda` wizard opens
  by itself, so skip `wait-ready`. This is the launch for a host with no
  dev model.
- `--show-window` starts plain `onda`, as the application launcher does, and
  the window opens at once.
- `--model ID` (repeatable) links a dev model (`parakeet-tdt-v2`,
  `parakeet-ultra`, `parakeet-ja`, `parakeet-unified`) and selects the first
  one.
- `--setting KEY=JSON` (repeatable) writes a key of `settings.json` before
  the start. Keys are the ones the app writes, such as `selectedLanguage`,
  `hotkeyActivationMode`, `outputMode`, `soundFeedbackEnabled`,
  `pauseMediaDuringRecording`, `historyEnabled`, and `historyKeepsAudio`.
  Examples: `selectedLanguage='"en"'`, `hotkeyActivationMode='"toggle"'`.

Ready means the tray title is `Onda, ready to dictate`. `wait-ready` polls
it for 90 s by default. The debug build loads the model on the CPU in a few
seconds.

`relaunch RUN_ID [--show-window]` starts Onda again in the same session and
data directory after it quit. Use it to prove that a setting, a history
entry, or a pending onboarding survives a restart.

## Doctor

Run it first whenever anything looks off:

```bash
"${VERIFY}" doctor "${RUN_ID}"
```

It is read-only. It checks that the recorded PID runs this run's binary and
that the binary still has its launch hash. It reads Onda's environment from
`/proc` and compares the display, the bus, the runtime, home, and data
directories, the audio server, and the proxy with the run's own. It checks
that the bus, the keyring, the tray watcher, and the accessibility bus all
belong to processes of this run, and that the default sink is the virtual
one with no hardware sink loaded. Then it prints the status, the count of
refused requests, a note when `target/debug/onda` changed since launch, and
the `not_covered` line. A failing doctor means stop and clean up.

## Drive

The tray, over D-Bus. The run owns `org.kde.StatusNotifierWatcher`, because
an isolated session has no tray host. `menu` reads
`com.canonical.dbusmenu.GetLayout` on the item's `/MenuBar` and clicks with
`AboutToShow` and `Event`:

```bash
"${VERIFY}" status "${RUN_ID}"                # --status, then icon, title, tooltip
"${VERIFY}" menu "${RUN_ID}" list             # rows; [disabled], ✓ and · marks
"${VERIFY}" menu "${RUN_ID}" press "Settings…"
"${VERIFY}" menu "${RUN_ID}" press "Language: Automatic" "Other Language…"
```

`list` indents the rows of a submenu, and `press` takes the labels down to
the item. `status` prints what `onda --status` answers and the item's
`IconName`, `Title`, and `ToolTip`.

A dictation, with the virtual microphone and a paste target:

```bash
"${VERIFY}" target "${RUN_ID}" new            # a GTK text view, focused
"${VERIFY}" hotkey "${RUN_ID}" hold 14 &      # Ctrl+Alt+Space held 14 s
"${VERIFY}" say "${RUN_ID}" --after 1         # jfk.wav into the microphone
"${VERIFY}" screenshot "${RUN_ID}" overlay-recording --window overlay
wait
"${VERIFY}" wait-window "${RUN_ID}" overlay --gone --timeout 60
"${VERIFY}" target "${RUN_ID}" read
```

- `hotkey RUN_ID hold SECONDS [KEYS]`, `tap [KEYS]`, and `escape` send real
  X11 key events. `KEYS` is an `xdotool` name and defaults to
  `ctrl+alt+space`, the Linux default shortcut.
- `ctl RUN_ID start|stop|toggle|cancel|paste-last|quit` runs Onda's own
  control commands (`onda --start` and so on). They are a separate entry
  point. A dictation started with `ctl` proves nothing about the shortcut.
- `say RUN_ID [WAV] [--after S]` plays a recording into the virtual
  microphone and returns when it ends. The default is
  `~/.cache/onda-dev-models/jfk.wav`, which transcribes to `And so, my fellow
  Americans, ask not what your country can do for you, ask what you can do
  for your country.` The host has no speech synthesizer, so a new phrase
  needs a new recording. Nothing else reaches the microphone, so a
  transcript holds only what you played.
- `target RUN_ID new|focus|read|clear|close` is the stand-in for TextEdit:
  `shell/linux/tests/paste-target.c`, compiled once. `read` prints its text
  and logs it to `target.tsv`. History records it as
  `Greendale Dictation Target`. `new --password` makes it a password
  field.

Windows:

```bash
"${VERIFY}" windows "${RUN_ID}"               # id, kind, geometry, name
"${VERIFY}" wait-window "${RUN_ID}" main      # or onboarding, overlay, target
"${VERIFY}" ui "${RUN_ID}" tree --save general
"${VERIFY}" ui "${RUN_ID}" press "Pause media while recording"
"${VERIFY}" screenshot "${RUN_ID}" general
"${VERIFY}" click "${RUN_ID}" 110 237
```

- `ui tree` prints what the window shows as the accessibility bus reports
  it: one `role "name"` per line with `[selected]`, `[checked]`,
  `[disabled]`, and the text of an editable field. The selected sidebar row
  and the header label name the page. `--all` adds what is hidden.
- `ui press NAME [--role ROLE] [--index N]` runs the action of a button or
  a switch, named as `tree` prints it. For a row it runs the one control
  inside.
- GTK 4 reports no usable widget positions and no action for a sidebar row,
  a combo row, or a text field. Reach those with `click RUN_ID X Y`. The
  window sits at 0,0 with no decoration offset, so a point in a
  `--window main` screenshot is the point `click` takes. Take the
  screenshot first, read the position, click, and confirm with `ui tree`.
- `key RUN_ID KEYS...` and `type RUN_ID TEXT` send key events to the
  focused window. No window manager runs, so focus is where the helper or
  `xdotool windowfocus` put it. A click does not move it.

Anything else goes through `exec`, which is as isolated as the rest:

```bash
EVIDENCE="$("${VERIFY}" evidence-dir "${RUN_ID}")"
# a media player that logs Pause, Play, and PlayPause
"${VERIFY}" exec "${RUN_ID}" -- python3 shell/linux/tests/mpris-player.py \
  greendale "${EVIDENCE}/greendale.calls" &
"${VERIFY}" exec "${RUN_ID}" -- gdbus call --session \
  --dest org.mpris.MediaPlayer2.greendale --object-path /org/mpris/MediaPlayer2 \
  --method org.freedesktop.DBus.Properties.Get \
  org.mpris.MediaPlayer2.Player PlaybackStatus
"${VERIFY}" exec "${RUN_ID}" -- pactl get-sink-mute onda-speakers
"${VERIFY}" exec "${RUN_ID}" -- xclip -selection clipboard -out
"${VERIFY}" exec "${RUN_ID}" -- secret-tool search --all account openRouterApiKey
```

The virtual microphone is the sink `onda-test`, and media plays to the
default sink `onda-speakers`. `cleanup` stops whatever `exec` left running.

## Evidence

Everything lands in `/tmp/onda-verify-evidence/<run-id>/` (also printed by
`"${VERIFY}" evidence-dir RUN_ID`). Cleanup never touches it.

- `screenshot RUN_ID NAME
  [--window main|onboarding|overlay|target|screen|PATTERN]` writes
  `NAME.png`. The default is `main`. A value that is not a kind is a
  pattern for the X11 window name. It waits 3 s for the window and then
  lists the visible ones.
- `ui RUN_ID tree --save NAME` writes `NAME.tree.txt`.
- `log RUN_ID` prints `onda.log`, the app's own output. It works after
  cleanup too. Useful lines: `claimed Escape for the session`,
  `Escape cancelled the session`, `the focus left the target`,
  `download started`, `preview decode`, `media player`.
- `history RUN_ID` prints saved entries (timestamp, app, model, duration,
  transcript) from `<data>/History/*/metadata.json` and saves `history.tsv`.
- `requests.log` holds every request the proxy refused. `opened.log` holds
  every URL or file Onda asked the desktop to open.
- `session/` holds the logs of the display, the keyring, the audio server,
  and the other session services.
- `record RUN_ID FEATURE ACTION RESULT` appends to `actions.tsv`.

## Proof standards

What counts:

- The user path. A tray row pressed over D-Bus, a key event, a click, or a
  control's own action. `--setting`, `--model`, and edits under the data
  directory only prepare preconditions.
- The action and its result, observed separately. For a dictation that is
  the overlay or the tray state during the session, then the text in the
  target, the history row, and the log line. For a setting it is the
  control in `ui tree` and the key in `<data>/settings.json`, or the state
  after `relaunch`.
- Every entry point by itself. The shortcut, the tray, `ctl`, and the
  window's buttons each start a dictation. One does not prove another.
- A refusal, for network features. `requests.log` naming the host proves
  that Onda asked. The UI state after the refusal proves how it fails.

What does not count:

- A unit test, `cargo test`, or `scripts/test-linux-gui.sh`. They are
  required and they are not this.
- A row in `menu list` as proof that a desktop draws it. It proves that
  Onda publishes it.
- A screenshot alone for a page's identity or a control's state. Pair it
  with `ui tree`.

What a run cannot prove, because it is X11 in Xvfb with no window manager:

- **Portal global shortcuts.** On Wayland the shortcut comes from the
  GlobalShortcuts portal. A run registers an X11 grab.
- **The layer-shell overlay.** On Wayland the overlay is a layer surface. A
  run shows the X11 window `onda-recording`.
- **`wtype` paste.** On Wayland Onda types with `wtype` or asks the portal
  for a keyboard. A run pastes with `xdotool`.
- **The Wayland clipboard.** A run uses the X11 selection.
- **What a desktop adds.** Focus policy, stacking, the tray host's drawing of
  the icon and menu, notifications, the file manager, the browser, login
  autostart, real microphones, and device changes.
- **A completed download or update.** The proxy refuses both.

Say so in the report when a claim falls in this list. Name the session it
would need, such as GNOME on Wayland, and do not present the X11 result as
covering it.

## Cleanup

After every success and every failed attempt:

```bash
"${VERIFY}" cleanup "${RUN_ID}"
ls "/tmp/onda-verify-evidence/${RUN_ID}"
```

Cleanup sends `TERM` to Onda, then to every process that carries the run's
marker in its environment, and `KILL` to any that remain after 5 s. It
never kills by name. Then it removes `/tmp/onda-verify-runs/<run-id>` and
keeps the evidence. A launch that fails cleans up after itself. The compiled
paste target stays cached in `/tmp/onda-verify-tools`.

## Helpers

```text
verify-onda launch RUN_ID [--fresh] [--reuse] [--show-window] [--model ID]... [--setting KEY=JSON]...
verify-onda relaunch RUN_ID [--show-window]
verify-onda doctor RUN_ID
verify-onda wait-ready RUN_ID [--timeout SECONDS]
verify-onda status RUN_ID
verify-onda evidence-dir RUN_ID
verify-onda record RUN_ID FEATURE ACTION RESULT
verify-onda menu RUN_ID list | press LABEL [LABEL]...
verify-onda ctl RUN_ID start|stop|toggle|cancel|paste-last|quit
verify-onda hotkey RUN_ID hold SECONDS [KEYS] | tap [KEYS] | escape
verify-onda say RUN_ID [WAV] [--after SECONDS]
verify-onda target RUN_ID new [--password] | focus|read|clear|close
verify-onda ui RUN_ID tree [--all] [--save NAME] | press NAME [--role ROLE] [--index N]
verify-onda click RUN_ID X Y
verify-onda key RUN_ID KEYS...
verify-onda type RUN_ID TEXT
verify-onda windows RUN_ID
verify-onda wait-window RUN_ID KIND [--gone] [--timeout SECONDS]
verify-onda screenshot RUN_ID NAME [--window main|onboarding|overlay|target|screen|PATTERN]
verify-onda log RUN_ID
verify-onda history RUN_ID
verify-onda exec RUN_ID -- COMMAND [ARGS]
verify-onda cleanup RUN_ID
```

On Linux `scripts/verify-onda` hands every command to
`scripts/verify-onda-linux`. `scripts/onda-driver.py` is the screenshot and
accessibility helper behind `screenshot` and `ui`. The session reuses the
fixtures in `shell/linux/tests`: `tray-host.py`, `refusing-proxy.py`,
`paste-target.c`, `mpris-player.py`, and `dbus-session.conf`. The helper
needs `dbus-run-session`, `Xvfb`, `xdotool`, `xprop`, `gdbus`,
`gnome-keyring-daemon`, PipeWire with WirePlumber and `pipewire-pulse`,
`pactl`, `paplay`, `at-spi-bus-launcher`, and Python 3 with `dbus`, `gi`,
`Atspi`, and `PIL`. `launch` names any that are missing.
