# Status menu and tray

The menu-bar icon and its menu, rebuilt from the core's `MenuState` each time
it opens. See shell/macos/FFI.md.

## Sub-features

- `menu-header`: disabled header per state, e.g. `Ready to Dictate — ⌥ Space`,
  `Recording…`, `Finish Setup to Dictate` (UI-7). With typing access
  missing it is `Accessibility Access Needed` on macOS 26 and
  `Device Control and Data Access Needed` on macOS 27.
- `menu-tray-label`: icon accessibility label, e.g. `Onda, ready to dictate`
  (`Onda, accessibility access needed` on macOS 26 and
  `Onda, device control and data access needed` on macOS 27 when typing
  access is missing).
- `menu-dictation`: Start / Stop / Cancel Dictation (UI-9).
- `menu-last-transcript`: Paste Last Transcript with preview subtitle.
- `menu-recent`: Recent Transcripts submenu with "Show All…" (history on).
- `menu-language`: Language submenu, subtitle is the current language.
- `menu-open`: "Open Onda" opens Getting Started; "Settings…" opens General
  (UI-11).
- `menu-fix-permissions`: shown only when a permission is missing (UI-8).
- `menu-updates`: the update item: "Check for Updates…", "Checking for
  Updates…" and "Downloading Update…" (disabled), "Restart to Update"
  (disabled while dictating, UPD-8), "Restarting…" (disabled; no
  dictation starts until Onda quits or the restart is abandoned), or
  "Download Onda <version>…" for a copy that cannot replace itself. A
  failed check while an update is staged keeps "Restart to Update".

## How to get to it (user POV)

Click the Onda icon in the menu bar.

## Driving it with verify-onda

- `menu RUN_ID list` prints items in order (`----` separators, `(disabled)`
  flags) and closes the menu. Compare against the order of `StatusMenu`
  in crates/onda-domain/src/status_menu.rs.
- `menu RUN_ID open`, `screenshot RUN_ID status-menu --window menu`,
  `menu RUN_ID close` for visual proof.
- `menu RUN_ID press "Open Onda"` then `screenshot RUN_ID ... --window main`:
  the window title is `Getting Started`. `press "Settings…"` lands on
  `General`.
- `menu RUN_ID press "Start Dictation"` with TextEdit focused, `say`, then
  `press "Stop Dictation"`; the transcript arrives as with the hotkey.
- `doctor` and `wait-ready` print the tray label.
- Submenus (Language, Recent Transcripts): `menu RUN_ID open`, then
  `llama-cu get-app com.rselbach.onda` and `get-ax-state`; the open menu
  appears under the menu extra.

## Gotchas

- `ONDA_DATA_DIR` only stops background update checks. "Check for
  Updates…" is enabled at launch and pressing it fetches the real
  `update.json`; a newer release downloads and stages beside the run's copy
  in `/tmp/onda-verify-runs/<run-id>/`, never over the user's install. Do
  not press it unless the update UI is the subject, and never press
  "Restart to Update" during a run: it quits the run and relaunches the
  copy (with the run's `ONDA_DATA_DIR`) outside the driver's control.
- The header text right after launch can still be `Preparing Speech Model…`;
  run `wait-ready` first.
- `Paste Last Transcript` types into the frontmost app; focus the run's
  TextEdit document before pressing it.

## On Linux

The tray is a StatusNotifierItem, built from the same core `StatusMenu`. A
run reads and clicks it over D-Bus (see [../LINUX.md](../LINUX.md)).

What differs:

- `menu-header` names the Linux shortcut: `Ready to Dictate — Ctrl+Alt+Space`.
- `menu-tray-label` is the item's `Title` and `ToolTip`, and
  `onda --status` answers the same text. The icon is a themed name, not a
  drawing: `audio-input-microphone-symbolic` (ready),
  `media-record-symbolic` (recording), `content-loading-symbolic`
  (transcribing), `folder-download-symbolic` (preparing a model), and
  `dialog-warning-symbolic` (warning). A download's percentage is in the
  title.
- `menu-dictation` adds a `Cancel Dictation` row while recording.
- `menu-language` has no subtitle, so its parent reads
  `Language: <current>`.
- `menu-recent` ends with `Show All History…`.
- `menu-fix-permissions` never shows. The core reports no missing
  permission on Linux.
- `menu-updates` offers a download, never `Restart to Update`. Onda does
  not replace a package-managed install.

Driving it:

- `status RUN_ID` prints the status, the icon, the title, and the tooltip.
- `menu RUN_ID list` prints the rows: a submenu's rows indented, a disabled
  row in brackets, `✓` and `·` on checkable rows, `---` for a separator.
- `menu RUN_ID press "Settings…"`, then `wait-window RUN_ID main` and
  `ui RUN_ID tree`. The `[selected]` sidebar row and the header label read
  `General`. `press "Open Onda"` lands on `Getting Started`.
- `menu RUN_ID press "Language: Automatic" "Other Language…"` opens Speech.
  Launch with `--setting 'recentLanguages="en,pt"'` to get language rows.
  `press "Language: Automatic" "Portuguese"` then sets `selectedLanguage`
  to `pt` in `<data>/settings.json` and the parent reads
  `Language: Portuguese`.
- `menu RUN_ID press "Start Dictation"`, `say`, `press "Stop Dictation"`.
- After a dictation, `menu RUN_ID press "Recent Transcripts" "<row>"`
  pastes that transcript into the focused target.
- `menu RUN_ID press "Quit Onda"` ends the process. `status` then fails
  with `Onda is not running`, and `relaunch` starts it again.
- `Retry Loading Model` and `Re-download Model` show after a failed load.
  See `mdl-failure` in [models-downloads.md](models-downloads.md).
- `Retry Failed Dictation` and `Discard Failed Recording` show after a
  decode fails, which no user action causes with a loaded model. Make the
  first decode of a run fail for lack of memory instead: `menu press
  "Start Dictation"`, `say`, read `VmSize` from `/proc/<pid>/status`, run
  `prlimit --pid <pid> --as=$(( (VmSize_kB + 220000) * 1024 )):`, then
  `menu press "Stop Dictation"`. The log reads `transcription failed` with
  `out of memory`, and both rows appear. Lift the cap with
  `prlimit --pid <pid> --as=unlimited:` before pressing either row. Retry
  pastes the kept recording's transcript into the focused target. Discard
  removes both rows.

Gotchas:

- `Check for Updates…` is safe to press. The proxy refuses the request,
  `requests.log` gains `CONNECT rselbach.github.io:443`, and General shows
  "Couldn't check for updates."
- Labels double every underscore, because DBusMenu reads one as an
  access-key marker.
- A run proves what Onda publishes on the bus. How a desktop's tray host
  draws the icon and the menu needs that desktop.
- The memory cap works for the first decode after a launch, with jfk.wav
  and a 220 MB margin. A smaller margin, or a later decode, can fail an
  allocation that the speech library does not survive, and Onda aborts.
  `relaunch` and try again.
