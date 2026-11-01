# Models and downloads

Choosing, downloading, loading, and deleting speech models.

## Sub-features

- `mdl-select`: Speech page rows `Select` / `Selected` (DL selection).
- `mdl-download-and-use`: "Download & Use" with a progress bar, Cancel, then
  the load (DL-*, OVL-16..19).
- `mdl-delete`: trash button, confirmation alert.
- `mdl-load`: first load compiles CoreML (~12 s on a fresh binary); tray
  shows progress; log `compiled model` lines then `model loaded`.
- `mdl-compatibility`: language vs model note with its action button.
- `mdl-failure`: "Retry Loading Model" / "Re-download Model" menu items after
  a load failure.
- `mdl-system-speech`: the System Speech engine (`system-speech`).

## How to get to it (user POV)

Main window > Speech; onboarding's Speech Model step; the status menu while a
model is loading or failed.

## Driving it with verify-onda

- Selection and load: `launch RUN_ID --model parakeet-tdt-v2 --model
  parakeet-ultra`; on Speech, select the other row with `llama-cu`, screenshot
  the tray/overlay during the load, `wait-ready`, and confirm with
  `log RUN_ID` (`model loaded ... model="..."`) and a dictation.
- Download: `launch RUN_ID --fresh` or select an uncloned model and press
  `Download & Use`. This fetches from the network into the run's data dir
  only. Screenshot `--window overlay` for the download overlay and
  `menu RUN_ID list` for `Preparing Speech Model… N%`.
- Delete: trash on a cloned, non-selected row, confirm the alert, then
  `ls <data>/Models` shows it gone. Only ever delete inside the run's data
  dir.
- Failure: corrupt a cloned model inside the run's data dir before launch
  (for example truncate one file under `<data>/Models/<id>/`), launch, and
  capture the error header and the Retry items.

## Gotchas

- `cp -c` clones are copy-on-write; editing a clone never touches
  `target/dev-models`. Never edit `target/dev-models` itself.
- Real downloads take minutes and use bandwidth; prefer clones unless the
  download path is the subject.
- Size hints are decimal MB, unlike Jabber.
