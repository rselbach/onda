# Onboarding

The first-run wizard "Set Up Onda": Welcome, Language, Permissions, Hotkey,
Speech Model, Ready (ONB-1..13).

## Sub-features

- `onb-auto-open`: appears 750 ms after launch while onboarding is pending
  (ONB-13); tray label `Onda, finish setup to dictate`.
- `onb-welcome`: "Welcome to Onda", "On-device dictation for any app.",
  `Get Started`.
- `onb-language`: search, Show All / Popular Only, selection sets the
  recommended model (ONB-4, ONB-5).
- `onb-permissions`: Microphone and Typing Access cards, "Use the clipboard
  instead" (ONB-6).
- `onb-hotkey`: preset cards and the "Another shortcut" recorder (ONB-7).
- `onb-model`: model cards, Download / Use This Model / Selected (ONB-8).
- `onb-ready`: "You're all set", Try It box, `Start Dictating` (ONB-9, ONB-10).
- `onb-abandon`: closing early keeps first-run pending (ONB-11).
- `onb-run-again`: Getting Started "Run Setup Again" reopens it.

## How to get to it (user POV)

First launch, or Getting Started > "Run Setup Again".

## Driving it with verify-onda

1. `launch RUN_ID --fresh --model parakeet-tdt-v2`. The clone means the
   Speech Model step shows the model as downloaded, with no network.
   Use plain `--fresh` only to test the Download path.
2. `sleep 2`, `screenshot RUN_ID onboarding-welcome --window onboarding`,
   `menu RUN_ID list` (header `Finish Setup to Dictate`).
3. `llama-cu get-app com.rselbach.onda`, `get-ax-state`: window
   `"Set Up Onda"`, `button "Get Started"`. Click it, screenshot each step,
   and continue with the named buttons (`Continue`, `Back`,
   `Start Dictating`).
4. Ready step: focus the Try It box with `llama-cu click`, then dictate with
   `hotkey RUN_ID hold` + `say`; screenshot the box with the text.
5. After `Start Dictating`: the window closes, `wait-ready` succeeds, and
   `onda-cli --data-dir <data> settings get selectedModel` matches the step's
   choice.
6. Abandon: close the window on an early step with `llama-cu` (`close
   button`). The tray label stays `Onda, finish setup to dictate` and
   `ls <data>/Models` shows no download started. The helper cannot relaunch
   the same data dir, so the "shows again at next launch" half of ONB-11 is
   out of reach.

## Gotchas

- Permissions show as granted: TCC grants belong to the real bundle id.
  The denied and never-asked paths cannot be produced without changing the
  user's grants; do not reset them (`tccutil`) without explicit permission.
- The Language step changes `selectedModel` to the language's
  recommendation; a non-English choice may want a model that is not cloned
  (`--model parakeet-ultra`, `parakeet-ja`, `parakeet-unified`).
- The Hotkey step saves and re-registers immediately; picking a lone
  modifier changes which `verify-onda hotkey` gesture works.

## On Linux

The wizard is its own window, `Set Up Onda`, 720x560, with the steps
Welcome, Language, Hotkey, Speech Model, and Ready. Linux has no permission
grants, so there is no Permissions step.

Driving it:

1. `launch RUN_ID --fresh --model parakeet-tdt-v2`. Skip `wait-ready`.
2. `status RUN_ID` reads `Onda, finish setup to dictate` with the icon
   `dialog-warning-symbolic`, and `menu RUN_ID list` starts with
   `[Finish Setup to Dictate]`.
3. `wait-window RUN_ID onboarding`, then `ui RUN_ID tree` and
   `screenshot RUN_ID onboarding-welcome --window onboarding`. The tree
   names the step in its first `grouping`.
4. Walk the steps by name: `ui RUN_ID press "Get Started"`, then
   `ui RUN_ID press "Continue"` on each step and `ui RUN_ID press "Back"`
   to return. Take a tree and a screenshot per step.
5. Hotkey: the row `list item "Another shortcut"` holds the recorder of
   the Hotkey page. `ui RUN_ID press "Record Shortcut"`, then
   `key RUN_ID ctrl+alt+g` records Ctrl+Alt+G. The row then reads
   `list item "Ctrl Alt G" [selected]`, `hotkey` in `<data>/settings.json`
   names the key, and the log says `registered the dictation hotkey
   hotkey=Ctrl+Alt+G`. From then on pass the keys to `hotkey`, as in
   `hotkey RUN_ID hold 14 ctrl+alt+g`. `key RUN_ID Escape` and
   `ui RUN_ID press "Back"`, `"Continue"`, or `"Close"` each end a
   recording: the button reads `Record Shortcut` again and the log gains
   another `registered the dictation hotkey` line.
6. Speech Model: the step shows the model recommended for the language.
   `ui RUN_ID press "Show other models"` lists the rest, and the linked
   model reads `Selected`.
7. Ready: the step has a "Click here and try it." field. `ui RUN_ID press
   "Start Dictating"` closes the window. `status` then reads
   `Onda, ready to dictate` and `onboardingCompleted` in
   `<data>/settings.json` is `true`.
8. `ctl RUN_ID quit` and `relaunch RUN_ID` prove what a second launch
   shows, such as the wizard returning after it was closed early. The
   macOS helper cannot do this.

Gotchas:

- The wizard starts a download by itself when the recommended model is not
  on disk. With English that is Parakeet Ultra, 740 MB. In a run the proxy
  refuses it: `requests.log` gains `CONNECT huggingface.co:443`, and the
  step shows "Download failed" beside a `Download` button. Link the
  recommended model with `--model` to see the step without the failure.
- A language row and a hotkey preset are list rows, so they need `click`
  with a position read from a screenshot.
