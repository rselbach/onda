# Main window pages

The settings window with a grouped sidebar: Setup (Getting Started),
Configure (General, Hotkey, Speech, Post-Processing, Replacements), Activity
(History), Help (About). prd/07 "Main window"; page contracts in
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
  System Settings shows there (08 "Permissions").
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
