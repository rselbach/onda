# Onda verification map

The maintained list of Onda's user-facing features and how to prove each one
in the running app. Read this index, then the feature file.

## Baseline preconditions

- `verify-onda launch <new-run-id>` then `wait-ready`; `doctor` reports
  `healthy` and tray `Onda, ready to dictate` (or `finish setup to dictate`
  for `--fresh`).
- Nothing else named Onda runs. If `pgrep -x Onda` shows a process that is
  not your run, wait for it to exit; never stop it.
- The data directory is scratch; the defaults domain, Keychain, TCC grants,
  login items, pasteboard, and audio devices are shared with the user.
- Room noise reaches the microphone. If a transcript contains words nobody
  spoke through `say`, it came from the room; repeat the dictation rather
  than accepting it.

## Driving conventions

- Global input through `verify-onda hotkey`, `say`, `textedit`, and `click`.
- The status menu through `verify-onda menu`.
- Windows through `llama-cu` selected with `get-app com.rselbach.onda`, with a
  fresh `get-ax-state` before each action. Prefer element names over
  coordinates.
- Preconditions through `--setting` / `--model` at launch, never as the proof.

## Proof and skip reporting

- A screenshot (and `.ax.txt` for windows) before the action and after the
  result; an `actions.tsv` row per action via `verify-onda record`.
- Side effects verified separately: target-app text (`textedit read`),
  `history`, `log`, or `onda-cli --data-dir <data> settings get KEY`.
- Report anything unreachable with the attempted action, the tray label, and
  the missing precondition (permission, model, network).

## On Linux

The sections above describe macOS. On Linux the same features are proven in
a private session (see [../LINUX.md](../LINUX.md)).

- Baseline: `verify-onda launch <new-run-id>` then `wait-ready`. `doctor`
  reports `healthy` and status `Onda, ready to dictate`, or
  `finish setup to dictate` for `--fresh`. Nothing is shared with the user
  or with another run, so several runs can go at once.
- Driving: the tray through `menu`, dictation through `hotkey`, `ctl`,
  `say`, and `target`, and windows through `ui tree`, `ui press`, `click`,
  `key`, and `type`. `exec RUN_ID -- COMMAND` covers the rest.
- Proof: a screenshot and a `ui tree --save` for a window, `menu list` and
  `status` for the tray, and the side effect checked separately:
  `target read`, `history`, `log`, `<data>/settings.json`, `requests.log`,
  or `opened.log`.
- Report what a run cannot prove: portal shortcuts, the layer-shell
  overlay, `wtype` paste, and whatever a desktop adds. Name the session
  the claim would need.

## Feature entry contract

Each file has five H2s: `Sub-features` (stable IDs, requirement IDs in
parentheses), `How to get to it (user POV)`, `Driving it with verify-onda`,
`Gotchas`, and `On Linux`. The first four describe macOS. `On Linux` says
what differs there, how to drive it with the Linux helper, and what a run
cannot prove.

## Features

- [Dictation and hotkeys](./dictation-hotkeys.md): Hold, Toggle, Automatic,
  lone modifiers, Escape, delivery by typing or clipboard.
- [Recording and download overlays](./overlay.md): waveform, preview,
  processing, notices, download progress.
- [Status menu and tray](./menu.md): header, dictation item, last
  transcript, language, open/settings.
- [Main window pages](./main-window.md): Getting Started, General, Hotkey,
  Speech, Replacements, About.
- [Onboarding](./onboarding.md): Welcome through Ready.
- [Models and downloads](./models-downloads.md): select, download, delete.
- [History](./history.md): saving, page, search, copy, clear, recent menu.
- [Post-processing](./cleanup.md): UI only, no real requests.
