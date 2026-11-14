# History

Saved transcripts on disk (`<data>/History/<entry>/metadata.json`, optional
`audio.wav`), the History page, and Recent Transcripts in the status menu.

## Sub-features

- `his-save`: each finished dictation saves an entry when "Save transcripts"
  is on (default on) (HIS-*).
- `his-page`: day sections, rows `<time> · <app> · <duration>`, transcript
  text, Copy button.
- `his-search`: "Search transcripts" field and the no-results view.
- `his-row-menu`: Copy, Copy Original, Play/Show Recording, Delete.
- `his-retention`: Keep for 1 Week / 1 Month / Forever; Keep audio recordings.
- `his-clear`: "Clear History…" toolbar button and confirmation.
- `his-recent-menu`: Recent Transcripts submenu and "Show All…".
- `his-off`: with history off, nothing is written and the page offers
  "Turn On History".

## How to get to it (user POV)

Main window > History; status menu > Recent Transcripts.

## Driving it with verify-onda

1. Dictate once or twice (see [dictation-hotkeys.md](./dictation-hotkeys.md)).
2. `history RUN_ID`: one row per dictation with app `TextEdit`, the model id,
   and the transcript that reached TextEdit.
3. `menu RUN_ID press "Open Onda"`, select the `History` outline row with
   `llama-cu`, `screenshot RUN_ID main-history --window main`, and save the
   tree: rows show `text "9:17 PM · TextEdit · 0:07"` and the transcript.
4. Search: `llama-cu set-value --element <search field> Greendale`, then
   screenshot; a nonsense query shows the no-results view.
5. Delete or Clear: confirm in the dialog, then `history RUN_ID` shows the
   rows gone.
6. Audio: launch with `--setting historyKeepsAudio=true`; after a dictation,
   `ls <data>/History/*/audio.wav` exists.
7. Off: launch with `--setting historyEnabled=false`; a dictation adds no
   directory under `<data>/History`.

## Gotchas

- History lives in the run's data dir and is removed by cleanup; `history`
  saves `history.tsv` in the evidence dir first.
- Copy actions overwrite the user's pasteboard.
- "Show Recording" opens Finder at the run's data dir; close that window.

## On Linux

The same files on disk, a History page, and the tray's Recent Transcripts.

What differs:

- The page has a "History settings" group (`Save transcripts`, `Keep for`,
  `Keep audio recordings`), a search field, `Clear History…`, and day
  groups. A row reads `<time> · <app> · <duration>` with the transcript, a
  `Copy transcript` button, and a `More actions` menu button.
- The app of a run's dictation is `Greendale Dictation Target`.
- With no entries the page shows "No Dictations Yet" and `Clear History…`
  is disabled.

Driving it:

1. Dictate once (see [dictation-hotkeys.md](./dictation-hotkeys.md)).
2. `history RUN_ID` prints one row per dictation and saves `history.tsv`.
3. `menu RUN_ID press "Recent Transcripts" "Show All History…"` opens the
   page. `ui RUN_ID tree --save history` lists the rows.
4. `ui RUN_ID press "Save transcripts"` turns history off. A dictation
   then adds no folder under `<data>/History`.
5. `ui RUN_ID press "Keep audio recordings"`, dictate, and
   `ls <data>/History/*/audio.wav`.
6. `ui RUN_ID press "Copy transcript"`, then
   `exec RUN_ID -- xclip -selection clipboard -out`.

Gotchas:

- The search field and the `Keep for` choice need `click` and `type`.
  `More actions` opens a popover, which is its own X11 window named `onda`.
- An action that opens a file or a folder writes its target to
  `opened.log` instead of starting a player or a file manager.
- The clipboard is the run's own. Nothing a run copies reaches the
  desktop.
