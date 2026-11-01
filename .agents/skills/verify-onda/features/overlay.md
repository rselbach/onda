# Recording and download overlays

Floating, non-activating panels at the bottom center of the screen with the
pointer: 400x104 during a session, 400x72 for model downloads and loads.
prd/07 "Recording overlay" and "Download overlay".

## Sub-features

- `ovl-waveform`: level meter, target app icon, clock, cancel button
  (OVL-6..8).
- `ovl-preview`: live preview text above a small waveform (OVL-9).
- `ovl-processing`: "Transcribing..." sweep after stop (OVL-10).
- `ovl-silent-mic`: "No sound from <device>" warning.
- `ovl-notice`: 4 s red-triangle notice, e.g. "Still Transcribing" (OVL-12..14).
- `ovl-no-focus-steal`: the target app stays frontmost (OVL-3).
- `ovl-download`: progress line and bar, Hide button (OVL-16..19).

## How to get to it (user POV)

Start a dictation; the overlay appears once the provisional 300 ms passes.
The download overlay appears while a model downloads or loads outside a
session (for example right after launch with a fresh binary).

## Driving it with verify-onda

- Recording and preview: run the hold recipe from
  [dictation-hotkeys.md](./dictation-hotkeys.md) and take
  `screenshot RUN_ID overlay-recording --window overlay` 2-4 s into speech.
  The preview text must match the spoken words so far.
- Processing: take a second overlay screenshot right after the hotkey is
  released (within about 300 ms; parakeet is fast, so this is timing-luck;
  long dictations of 20 s or more widen the window).
- Cancel button: during a Toggle session, `verify-onda windows RUN_ID`
  lists the overlay (layer 3, 400x104) with its global `x`/`y` in points.
  The circled X sits at about `x+378, y+52`; `verify-onda click RUN_ID X Y`
  there ends the session with no text and no history entry.
- Focus: after the dictation, `textedit RUN_ID read` proves the text reached
  TextEdit, so the overlay did not take focus.
- Download overlay: launch with a fresh build (`launch` without `--reuse`
  after a code change) and screenshot `--window overlay` during the load, or
  see [models-downloads.md](./models-downloads.md).

## Gotchas

- The overlay is on screen only during a session or load; `screenshot`
  retries for 3 s and then lists the PID's windows on stderr.
- It follows the pointer's screen. Leave the pointer on the main display.
- Fade-in is 0.2 s; a screenshot right at start can be translucent.
