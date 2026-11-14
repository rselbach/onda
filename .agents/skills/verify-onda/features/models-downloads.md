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

## On Linux

Linux runs GGUF models on the CPU: `parakeet-tdt-v2`, `parakeet-ultra`, and
`parakeet-unified`. `mdl-system-speech` and the CoreML compile of
`mdl-load` do not exist. Only Parakeet Unified shows a live preview, and
it needs four CPUs.

Driving it:

- `launch RUN_ID --model parakeet-tdt-v2 --model parakeet-ultra` links both
  from `~/.cache/onda-dev-models` and selects the first.
- On Speech, `ui RUN_ID tree` shows each model as a row: the selected one
  with `image "Selected model"`, a linked one with `button "Select"`, and
  one that is not on disk with `button "Download & Use"`.
- `mdl-select`: `ui RUN_ID press "Select"`, `wait-ready`, and
  `selectedModel` in `<data>/settings.json`.
- `mdl-delete`: `ui RUN_ID press "Delete model" --index N`. The tree then
  shows `alert "Delete model"`. `ui RUN_ID press "Delete"` confirms, and
  `ls <data>/Models` shows the folder gone.
- `mdl-download-and-use`: `ui RUN_ID press "Download & Use" --index N`. The
  tray reads `Onda, preparing speech model, 0 percent`, the overlay shows,
  and `requests.log` gains `CONNECT huggingface.co:443` for each retry.
  Nothing is written, and the status returns to ready on the old model.
- `mdl-load`: after launch `wait-ready` takes a few seconds, and the first
  load writes `.onda-verified` beside the model once its SHA-256 matched.

Gotchas:

- A completed download cannot be proven in a run. The proxy refuses it by
  design, and the Rust tests of `onda-models` cover the transfer.
- A linked model is a symlink. Deleting it in the UI removes the link and
  leaves the shared file alone. Never edit the files in
  `~/.cache/onda-dev-models`.
- If a dev model is missing, download it once from the pinned URL in
  `shell/linux/src/linux/speech.rs` into that directory and check its
  SHA-256. The helper will not do it for you.
