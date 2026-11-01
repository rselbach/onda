# Status menu and tray

The menu-bar icon and its menu, rebuilt from the core's `MenuState` each time
it opens. See shell/macos/FFI.md.

## Sub-features

- `menu-header`: disabled header per state, e.g. `Ready to Dictate — ⌥ Space`,
  `Recording…`, `Finish Setup to Dictate` (UI-7). With typing access
  missing it is `Accessibility Access Needed` on macOS 26 and
  `Device Control and Data Access Needed` on macOS 27.
- `menu-tray-label`: icon accessibility label, e.g. `Onda, ready to dictate`
  (`Onda, accessibility access needed` on macOS 26 and
  `Onda, device control and data access needed` on macOS 27 when typing
  access is missing).
- `menu-dictation`: Start / Stop / Cancel Dictation (UI-9).
- `menu-last-transcript`: Paste Last Transcript with preview subtitle.
- `menu-recent`: Recent Transcripts submenu with "Show All…" (history on).
- `menu-language`: Language submenu, subtitle is the current language.
- `menu-open`: "Open Onda" opens Getting Started; "Settings…" opens General
  (UI-11).
- `menu-fix-permissions`: shown only when a permission is missing (UI-8).
- `menu-updates`: "Check for Updates…" (disabled until Sparkle starts).

## How to get to it (user POV)

Click the Onda icon in the menu bar.

## Driving it with verify-onda

- `menu RUN_ID list` prints items in order (`----` separators, `(disabled)`
  flags) and closes the menu. Compare against the order of `StatusMenu`
  in crates/onda-domain/src/status_menu.rs.
- `menu RUN_ID open`, `screenshot RUN_ID status-menu --window menu`,
  `menu RUN_ID close` for visual proof.
- `menu RUN_ID press "Open Onda"` then `screenshot RUN_ID ... --window main`:
  the window title is `Getting Started`. `press "Settings…"` lands on
  `General`.
- `menu RUN_ID press "Start Dictation"` with TextEdit focused, `say`, then
  `press "Stop Dictation"`; the transcript arrives as with the hotkey.
- `doctor` and `wait-ready` print the tray label.
- Submenus (Language, Recent Transcripts): `menu RUN_ID open`, then
  `llama-cu get-app com.rselbach.onda` and `get-ax-state`; the open menu
  appears under the menu extra.

## Gotchas

- `ONDA_DATA_DIR` only stops Sparkle's background checks. "Check for
  Updates…" is enabled a moment after launch and pressing it fetches the
  real appcast and can offer to install a release over the run. Do not press
  it unless the update UI is the subject, and then never accept an install.
- The header text right after launch can still be `Preparing Speech Model…`;
  run `wait-ready` first.
- `Paste Last Transcript` types into the frontmost app; focus the run's
  TextEdit document before pressing it.
