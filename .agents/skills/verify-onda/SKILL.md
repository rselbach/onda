---
name: verify-onda
description: Launch, drive, and capture proof from the real Onda.app (macOS menu-bar dictation app; Swift shell over the Rust core) against a scratch data directory. Use after changing dictation, hotkeys, the overlays, the status menu, onboarding, main-window pages, models, or history, when the proof must come from the running app rather than Rust or Swift tests.
---

# Verify Onda

Onda is a background (`LSUIElement`) menu-bar app. A verification run copies
`build/Onda.app`, points it at a scratch data directory with `ONDA_DATA_DIR`,
drives it with synthetic HID input, the status menu, and `llama-cu`, and keeps
screenshots, logs, and a TSV of actions as evidence.

Read [features/README.md](features/README.md) and the file for the feature
under test before driving. Run `cargo test` and the Swift tests separately.

## Rules

- One Onda at a time. `launch` refuses while `pgrep -x Onda` finds anything.
  That process belongs to another agent or to the user: wait and retry, never
  stop it. Hotkeys, microphone, TCC grants, and the pasteboard are global.
- Never read or write `~/Library/Application Support/Onda`. Every run uses
  `/tmp/onda-verify-runs/<run-id>/data`.
- Never enter a real OpenRouter API key, press "Test" on Post-Processing, or
  enable post-processing with a key: the Keychain item is shared with the
  user's real install and requests cost money.
- Leave Launch at Login, automatic updates, and "Check for Updates…" alone
  unless they are the feature under test.
- Close test documents unsaved (`textedit close`; `cleanup` does it too).
- Restore any system setting you change (appearance, volume, default input
  or output device, keyboard settings) before cleanup, and record the
  restore in `actions.tsv`.
- Use Community fixtures: `Troy Barnes studies at Greendale Community
  College.`
- The microphone hears the room. A transcript with words nobody passed to
  `say` came from people or media nearby; repeat the take instead of
  accepting it, and record the bad take in `actions.tsv`.

Isolation is by data directory, not bundle id. The Microphone and
Accessibility grants (System Settings calls Accessibility "Device Control
and Data Access" on macOS 27) belong to `com.rselbach.onda` and its
Developer ID designated requirement; a unique id per run would lose them. Consequences:
the `com.rselbach.onda` defaults domain (main window frame), Keychain
items, TCC, and login items are shared with the user's real Onda. The
updater keeps its state in the run's data directory and makes no
background checks whenever `ONDA_DATA_DIR` is set.

## Launch

Run from the repository root with a new run ID per attempt:

```bash
VERIFY=".agents/skills/verify-onda/scripts/verify-onda"
RUN_ID="dictation-$(date +%H%M%S)"
"${VERIFY}" launch "${RUN_ID}"
"${VERIFY}" wait-ready "${RUN_ID}"
```

`launch` runs `scripts/bundle-app.sh debug` and `cargo build -p onda-cli`
(log in `<evidence>/build.log`; SwiftPM waits if another agent holds its
lock), copies the bundle to `/tmp/onda-verify-runs/<run-id>/Onda.app` so a
concurrent rebuild cannot swap it, creates the data directory, completes
onboarding with `onda-cli`, clones `parakeet-tdt-v2` from
`target/dev-models` with `cp -c -R` (instant), applies settings, launches
with `open -n --env ONDA_DATA_DIR=...`, and records the PID and executable.

Options:

- `--reuse` skips both builds and uses the current `build/Onda.app`. Use it
  for a second run when nothing changed.
- `--fresh` leaves onboarding pending and clones no default model; the
  onboarding window appears 750 ms after launch.
- `--model ID` (repeatable) clones a dev model (`parakeet-tdt-v2`,
  `parakeet-ultra`, `parakeet-ja`, `parakeet-unified`) and selects the first
  one; `system-speech` needs no files.
- `--setting KEY=JSON` (repeatable) runs `onda-cli settings set` before
  launch. Keys: `selectedModel`, `selectedLanguage`, `recentLanguages`,
  `historyRetention`, `hotkey`, `hotkeyActivationMode`, `outputMode`,
  `inputDeviceUID`, `soundFeedbackEnabled`, `pauseMediaDuringRecording`,
  `historyEnabled`, `historyKeepsAudio`, `postProcessingEnabled`. Examples:
  `hotkey={"key":"AltRight","modifiers":[]}`, `hotkeyActivationMode="toggle"`,
  `outputMode="clipboard"`, `inputDeviceUID="BuiltInMicrophoneDevice"`.

Ready means the tray label is `Onda, ready to dictate`; `wait-ready` polls it
(default 90 s). A fresh binary compiles the CoreML encoder on first load for
about 12 s; the log line `model loaded` marks the end. With `--fresh` the
label stays `Onda, finish setup to dictate`, so skip `wait-ready`.

## Doctor

Run it first whenever anything looks off:

```bash
"${VERIFY}" doctor "${RUN_ID}"
```

It is read-only: the recorded PID is alive and runs this run's executable,
the executable hash matches launch, no other Onda runs, the copy still
verifies, and it prints the data dir and the tray label (which names a
missing permission or model). It notes when `build/Onda.app` changed since
launch (the run is stale). A failing doctor means stop and clean up.

## Drive

Global input (hotkey, speech, focus):

```bash
"${VERIFY}" textedit "${RUN_ID}" new          # new unsaved doc, focused by a real click
"${VERIFY}" hotkey "${RUN_ID}" hold 7 &       # Option+Space held 7 s (Hold mode)
"${VERIFY}" say "${RUN_ID}" "Troy Barnes studies at Greendale Community College." --after 1.2 &
sleep 3.5
"${VERIFY}" screenshot "${RUN_ID}" overlay-recording --window overlay
wait
sleep 3                                       # transcription and typing
"${VERIFY}" textedit "${RUN_ID}" read
```

- `hotkey RUN_ID hold SECONDS`: Option+Space down, wait, up.
- `hotkey RUN_ID tap`: an 80 ms Option+Space press (Toggle, Automatic).
- `hotkey RUN_ID lone KEY SECONDS`: a lone modifier with the right side bit;
  `KEY` is `option`, `right-option`, `control`, `right-control`, `shift`,
  `right-shift`, `command`, `right-command`, or `fn`. The run must use a
  matching `hotkey` setting.
- `hotkey RUN_ID escape [--with-modifiers]`: plain Escape, or Option+Escape.
- `say RUN_ID TEXT [--voice V] [--after S] [--device ID]`: speaks through the
  speakers for the microphone (`say -a '?'` lists devices; logged to
  `spoken.tsv`).
- `textedit RUN_ID new|focus|read|close`: `focus` re-clicks inside the run's
  document when another app stole focus; `read` prints the text and logs it
  to `textedit.tsv`.
- `click RUN_ID X Y`: one real left click at global screen points.
- `windows RUN_ID`: the PID's on-screen windows (id, layer, x, y, width,
  height, alpha, title). Layer 0 is the main or onboarding window, 3 the
  overlays, 101 the open status menu.

Status menu, read and pressed through accessibility (no coordinates):

```bash
"${VERIFY}" menu "${RUN_ID}" list               # titles, ---- separators, (disabled)
"${VERIFY}" menu "${RUN_ID}" press "Open Onda"  # or "Settings…", "Start Dictation", ...
"${VERIFY}" menu "${RUN_ID}" open               # leave it open for a screenshot
"${VERIFY}" screenshot "${RUN_ID}" status-menu --window menu
"${VERIFY}" menu "${RUN_ID}" close
```

`press` handles top-level items only. For Language and Recent Transcripts
submenus, use `llama-cu` on the open menu.

Windows (main window, onboarding): load the `llama-cu` skill and select the
running app by bundle id, never by path (a path can launch a second copy):

```bash
export LLAMA_CU_SESSION="verify-onda-${RUN_ID}"
llama-cu get-app com.rselbach.onda
llama-cu get-ax-state                    # sidebar rows: outline row > text "History"
llama-cu click --element <id of the outline row>
```

Sidebar rows, toggles (`check box "Save transcripts"`), pop-ups, buttons
(`button "Get Started"`, `button "Run Setup Again"`), and the search field
(`search field ... placeholder="Search transcripts"`) all expose names. Take a
fresh `get-ax-state` before each action; ids change with the UI.

## Evidence

Everything lands in `/tmp/onda-verify-evidence/<run-id>/` (also printed by
`"${VERIFY}" evidence-dir RUN_ID`); cleanup never touches it.

- `screenshot RUN_ID NAME [--window main|onboarding|overlay|menu|screen]`
  writes `NAME.png` of that Onda window (default `main`), or the whole
  screen. The overlay only exists mid-session; take it while the hotkey
  helper runs in the background.
- `log RUN_ID [--since 5m]` prints this PID's `com.rselbach.onda` unified
  log and saves `log-HHMMSS.txt`. (`log` is a zsh builtin; the helper calls
  `/usr/bin/log`.) Useful lines: `model loaded`, `saved history entry`,
  `dictation pipeline`, `Escape cancelled the session`.
- `history RUN_ID` prints saved entries (timestamp, app, model, duration,
  transcript) from `<data>/History/*/metadata.json` and saves `history.tsv`.
- Save `llama-cu get-ax-state` output as `NAME.ax.txt` beside screenshots.
- `record RUN_ID FEATURE ACTION RESULT` appends to `actions.tsv`.

Proof standards: drive the user path (hotkey, menu, clicks), not `onda-cli`
writes or core commands; `onda-cli` only prepares preconditions. Capture the
action and the result: the overlay or menu state during the session plus the
delivered text, and for settings the changed control plus a second
observation (reopen the page, or `onda-cli --data-dir <data> settings get
KEY`). Verify side effects separately: text in the target app, the history
entry, the log line. Record every entry point exercised; do not claim one
proved through another.

## Cleanup

After every success and every failed attempt:

```bash
"${VERIFY}" cleanup "${RUN_ID}"
ls "/tmp/onda-verify-evidence/${RUN_ID}"
```

Cleanup closes the run's TextEdit document unsaved, sends `TERM` only to an
Onda process whose command is this run's copied executable, and removes
`/tmp/onda-verify-runs/<run-id>` (app copy and data). It never kills by name
and never removes evidence. The compiled input helper stays cached in
`/tmp/onda-verify-tools`.

## Helpers

```text
verify-onda launch RUN_ID [--fresh] [--reuse] [--model ID]... [--setting KEY=JSON]...
verify-onda doctor RUN_ID
verify-onda wait-ready RUN_ID [--timeout SECONDS]
verify-onda evidence-dir RUN_ID
verify-onda record RUN_ID FEATURE ACTION RESULT
verify-onda hotkey RUN_ID hold SECONDS | tap | lone KEY SECONDS | escape [--with-modifiers]
verify-onda say RUN_ID TEXT [--voice VOICE] [--after SECONDS] [--device ID]
verify-onda textedit RUN_ID new|focus|read|close
verify-onda menu RUN_ID list|open|close|press TITLE
verify-onda click RUN_ID X Y
verify-onda windows RUN_ID
verify-onda screenshot RUN_ID NAME [--window main|onboarding|overlay|menu|screen]
verify-onda log RUN_ID [--since 5m]
verify-onda history RUN_ID
verify-onda cleanup RUN_ID
```

`scripts/onda-driver.swift` is the CGEvent and accessibility helper behind
`hotkey`, `click`, `menu`, `screenshot`, and the tray reads; `verify-onda`
compiles it once per source hash. It needs Accessibility and Screen
Recording for the app running the agent (`llama-cu doctor` reports both).
