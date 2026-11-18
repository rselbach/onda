# Dictation and hotkeys

The global hotkey records speech, the selected model transcribes it, and the
text is typed into the app that was focused at start (or left on the
clipboard).

## Sub-features

- `dict-hold`: Hold mode, Option+Space down records, up stops (HOT-25).
- `dict-provisional`: a press shorter than 300 ms leaves no trace (HOT-29..31).
- `dict-toggle`: tap starts, tap stops (HOT-26).
- `dict-automatic`: short tap toggles, hold of 400 ms or more is push-to-talk
  (HOT-27).
- `dict-lone-modifier`: Right Option, Fn, and other lone modifiers with the
  150 ms debounce (HOT-21..23), discard when another key joins (HOT-33).
- `dict-escape`: Escape (and Option+Escape) cancels a session (HOT-34).
- `dict-type-output`: typed into the target app with the joiner (OUT-*).
- `dict-clipboard-output`: transcript on the pasteboard (`outputMode`
  `clipboard`).
- `dict-menu-start`: "Start Dictation" from the status menu (UI-9).

## How to get to it (user POV)

- Focus a text field and use the hotkey (default Option+Space, Hold).
- Status menu "Start Dictation" / "Stop Dictation".
- The Try It box on Getting Started and on the onboarding Ready step.

## Driving it with verify-onda

1. `launch RUN_ID --setting 'inputDeviceUID="BuiltInMicrophoneDevice"'`
   (add `hotkey=` / `hotkeyActivationMode=` / `outputMode=` as needed), then
   `wait-ready`.
2. `textedit RUN_ID new`, then `screenshot RUN_ID target-empty --window screen`.
3. Hold: `hotkey RUN_ID hold 7 &`, `say RUN_ID "<phrase>" --after 1.2 &`,
   `sleep 3.5`, `screenshot RUN_ID overlay-recording --window overlay`, `wait`.
   Toggle/Automatic: `hotkey RUN_ID tap`, `say`, then `tap` again. Lone
   modifier: launch with `hotkey={"key":"AltRight","modifiers":[]}` and use
   `hotkey RUN_ID lone right-option SECONDS`.
4. `sleep 3`, then `textedit RUN_ID read`: the phrase is there, appended with
   a single space after earlier text. `history RUN_ID` shows the entry with
   app `TextEdit`; `log RUN_ID` shows `dictation pipeline`.
5. Escape: start a session (Toggle is easiest), confirm `menu RUN_ID list`
   shows `Recording…`, run `hotkey RUN_ID escape`, and confirm the header is
   back to `Ready to Dictate` and the log says `Escape cancelled the session`;
   the document is unchanged.
6. Provisional: `hotkey RUN_ID hold 0.15`; no overlay, no history entry.

## Gotchas

- If text does not arrive and history has app `-` or an unexpected app, some
  app stole focus: `textedit RUN_ID focus` and repeat. The first dictation
  right after launch can lose its target; wait a few seconds after focusing.
- Room speech gets transcribed too. Accept only the phrase you spoke.
- AirPods as default input: set `inputDeviceUID` to
  `BuiltInMicrophoneDevice`; `say` plays through the default output (pass
  `--device` with an id from `say -a '?'` for the built-in speakers).
- On a US-International layout a stuck Option leaves TextEdit in a dead-key
  state where typed text vanishes; quit and reopen TextEdit (after closing
  the run's document) if that happens.
- Fn as a hotkey may also trigger the system Globe action
  (`AppleFnUsageType`); the Hotkey page shows the warning.
- Clipboard output replaces the user's pasteboard. Note what you overwrite.

## On Linux

The default shortcut is Ctrl+Alt+Space in Hold mode. Linux offers no lone
modifier and no Fn shortcut, so `dict-lone-modifier` has nothing to prove.

What differs:

- A dictation has four entry points: the shortcut, the tray, the control
  commands (`onda --start`, `--stop`, `--toggle`, `--cancel`,
  `--paste-last`), and the `Start / Stop` and `Cancel` buttons on Getting
  Started. Prove each one you claim.
- `dict-escape` works on X11 only. Onda claims Escape for the length of a
  session and logs `claimed Escape for the session`, then
  `Escape cancelled the session`.
- `dict-type-output` copies the transcript and sends Ctrl+V through XTEST.
  General has a Linux-only "Terminal paste shortcut" switch that sends
  Ctrl+Shift+V. If Onda cannot send the shortcut safely on the active layout,
  the transcript stays on the clipboard for manual paste. After an automatic paste, Onda
  tries to restore the previous clipboard.
- The joiner reads the focused field over the accessibility bus, which a
  run has. A password field is never read.
- General > Media pauses MPRIS players and mutes other audio while
  recording. Drive it with the `mpris-player.py` recipe in
  [../LINUX.md](../LINUX.md) and `pactl get-sink-mute onda-speakers`.

Driving it:

1. `launch RUN_ID` (add `--setting 'hotkeyActivationMode="toggle"'` or
   `--setting 'outputMode="clipboard"'` as needed), then `wait-ready`.
2. `target RUN_ID new`.
3. Hold: `hotkey RUN_ID hold 14 &`, `say RUN_ID --after 1`,
   `screenshot RUN_ID overlay-recording --window overlay`, `wait`.
   Toggle: `hotkey RUN_ID tap`, `say RUN_ID`, `hotkey RUN_ID tap`.
4. `wait-window RUN_ID overlay --gone --timeout 60`, then
   `target RUN_ID read`. The JFK phrase is there. `history RUN_ID` shows
   the entry with app `Greendale Dictation Target`.
5. Escape: `ctl RUN_ID start`, `wait-window RUN_ID overlay`,
   `hotkey RUN_ID escape`, `wait-window RUN_ID overlay --gone`. `status`
   is back to `Onda, ready to dictate`, `log` has
   `Escape cancelled the session`, and `history` gained no row.
6. Provisional: `hotkey RUN_ID hold 0.15` leaves no overlay and no history
   row.
7. Clipboard output: `exec RUN_ID -- xclip -selection clipboard -out`.
8. Joiner: `target RUN_ID clear`, `type RUN_ID "Troy Barnes joined the"`,
   then dictate. `target RUN_ID read` starts
   `Troy Barnes joined the and so,` with one space and the capital lowered.
   For a password field, `target RUN_ID close`, `target RUN_ID new
   --password`, `type RUN_ID "Greendale7"`, then dictate. It reads
   `Greendale7And so,` because Onda did not read the field.

Gotchas:

- The debug build transcribes the 11 s recording in about 10 s. Wait for
  the overlay to go, not for a fixed time.
- Nothing but `say` reaches the microphone, so a take never needs a repeat
  for room noise.
- A click in Onda's window does not move keyboard focus off the target,
  because no window manager runs. `target RUN_ID focus` puts it back after
  `xdotool windowfocus` moved it.
- A run cannot prove the Wayland paths: the GlobalShortcuts portal, `wtype`
  paste, the portal's keyboard prompt, and the Wayland clipboard.
