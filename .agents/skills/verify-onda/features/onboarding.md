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
