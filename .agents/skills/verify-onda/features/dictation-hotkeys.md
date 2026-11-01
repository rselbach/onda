# Dictation and hotkeys

The global hotkey records speech, the selected model transcribes it, and the
text is typed into the app that was focused at start (or left on the
clipboard). prd/03.

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
