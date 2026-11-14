# Main window pages

The settings window with a grouped sidebar: Setup (Getting Started),
Configure (General, Hotkey, Speech, Post-Processing, Replacements), Activity
(History), Help (About). Page contracts are in
shell/macos/FFI.md. History, Post-Processing, and model controls have their
own files.

## Sub-features

- `win-open`: "Open Onda" opens Getting Started, "Settings…" and Cmd-, open
  General, re-selecting the page if already open (UI-3, UI-4, UI-11).
- `win-presence`: Onda becomes a regular app (Dock icon, app menu) while the
  window is open and returns to accessory when it closes (UI-2).
- `win-getting-started`: setup checklist (`Onda is ready`, Microphone,
  Accessibility, Speech Model), "Run Setup Again", Try It box. On macOS 27
  the Accessibility row reads "Device Control and Data Access", the name
  System Settings shows there.
- `win-general`: Permissions (Microphone, and Accessibility or, on macOS 27,
  "Device Control and Data Access"), Microphone input picker, Output, Media,
  Feedback (sounds), Startup (Launch at Login), Updates sections.
- `win-hotkey`: Shortcut keycaps, recorder, "Reset to ⌥ Space", Activation
  picker with description.
- `win-speech`: Transcription Language picker, compatibility note, Speech
  Models rows.
- `win-replacements`: rule rows (triggers, replacement), Add Rule, trash.
- `win-about`: version, links, licenses.

## How to get to it (user POV)

Status menu "Open Onda" or "Settings…"; Cmd-, while Onda is active; "Fix
Permissions…" lands on General.

## Driving it with verify-onda

1. `menu RUN_ID press "Open Onda"`, then
   `screenshot RUN_ID main-getting-started --window main`.
2. `export LLAMA_CU_SESSION="verify-onda-${RUN_ID}"`,
   `llama-cu get-app com.rselbach.onda`, `llama-cu get-ax-state`. Sidebar
   rows are `outline row` elements containing `text "General"` etc.; click
   the row's id. The window title follows the page.
3. Per page, capture `screenshot RUN_ID <page> --window main` and save the
   tree as `<page>.ax.txt`.
4. Changing a setting: act on the control by name (`check box`,
   `pop up button`), screenshot, then prove persistence with
   `onda-cli --data-dir <data> settings get KEY` (data dir from `doctor`), or
   by switching pages and back.
5. Hotkey: choose Toggle in the Activation pop-up, verify
   `settings get hotkeyActivationMode` is `"toggle"`, then exercise it with
   `hotkey RUN_ID tap`. "Reset to ⌥ Space" restores the default.
6. Replacements: Add Rule, type a trigger (`Greendale`) and replacement
   (`Greendale Community College`) with `llama-cu type-text`, wait 1 s
   (debounced save), dictate the trigger, read TextEdit.

## Gotchas

- The window frame autosaves into the shared `com.rselbach.onda` defaults
  domain; earlier runs may have left it full-screen sized. Harmless, but do
  not delete the user's defaults to "fix" it.
- Launch at Login registers the real bundle with the system. Do not toggle
  it outside a task that is about it, and turn it back off afterwards.
- Picking an input device in General writes `inputDeviceUID` for this run
  only; the system default input is not changed.
- Appearance proof (light and dark) means changing System Settings >
  Appearance; switch it back and record both changes.

## On Linux

One GTK window named `Onda`, 1060x780, with the same sidebar groups. The
header shows the page name over `Onda` and holds a `Quit` button. Closing
the window hides it and Onda keeps running.

What differs:

- `win-presence` does not apply. A GTK application has no Dock presence to
  switch.
- `win-getting-started` shows the status line, `Start / Stop` and `Cancel`,
  the shortcut, and a "Set up dictation" group with three rows that jump to
  Speech, Hotkey, and General. It has no permission rows.
- `win-general` has Microphone, Output ("After transcription" and the
  Linux-only "Terminal paste shortcut"), Media, Feedback, Startup, and
  Updates. It has no Permissions section.
- `win-hotkey` shows the shortcut as keycaps, the shortcut the desktop
  registered, `Record Shortcut`, "Reset to Ctrl+Alt+Space", the Activation
  choice, and a note on binding `onda --toggle` in the desktop's own
  keyboard settings.
- `win-speech` lists the GGUF models (see
  [models-downloads.md](./models-downloads.md)).

Driving it:

1. `menu RUN_ID press "Open Onda"` or `launch --show-window`, then
   `wait-window RUN_ID main`.
2. `ui RUN_ID tree --save NAME` and `screenshot RUN_ID NAME` per page. The
   `[selected]` sidebar row and the header label name the page, and the
   page's own panel lists its groups, rows, and controls with their states.
3. Change pages with `click RUN_ID X Y` on a sidebar row. Read the row's
   position from the screenshot. GTK reports no action for a list row.
4. `ui RUN_ID press "<title>"` flips a switch row or presses a button. Two
   controls with one name need `--index N`.
5. A combo row and a text field need `click`, then `key` or `type` once
   `exec RUN_ID -- xdotool windowfocus <id>` gave the window the keyboard.
6. Record a shortcut: `ui RUN_ID press "Record Shortcut"`, give the window
   the keyboard with `exec RUN_ID -- xdotool windowfocus <id>`, then
   `key RUN_ID ctrl+alt+d`. The tree reads `Current shortcut: Ctrl+Alt+D`,
   `hotkey` in `<data>/settings.json` holds `KeyD`, and
   `hotkey RUN_ID hold 2 ctrl+alt+d` shows the overlay.
7. Prove a setting twice: the control's state in `ui tree`, and the key in
   `<data>/settings.json`. `ctl RUN_ID quit` then `relaunch RUN_ID` proves
   that it survives a restart.

Gotchas:

- "Open Onda at login" is safe in a run. It writes
  `autostart/com.rselbach.onda.desktop` under the run's scratch
  `~/.config`, and that file is the proof. No login happens.
- Every window opens at 0,0. The paste target moves itself to 1080,0, so
  it does not cover Onda's window.
- The alert dialogs are drawn inside the window. `ui tree` shows one as
  `alert "<title>"` with its buttons.
- A run uses the default theme of an empty home. Light and dark appearance
  need a desktop.
