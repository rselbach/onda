# Onda

Onda is a macOS menu bar app for dictation: hold or toggle a hotkey, speak,
and the transcript is typed into the frontmost app. Speech recognition runs on
the device (Parakeet on CoreML, or Apple's System Speech); optional transcript
cleanup goes through OpenRouter. It is a Rust rewrite of Jabber: the core is a
Rust workspace, and a thin Swift shell supplies the native UI and system
integration.

The product requirements (`PRD.md`, `prd/`) are kept outside version control
and are not part of this repository.

## Requirements

- macOS 27 or later on Apple Silicon (arm64 only)
- Rust stable 1.99 or later (`rustup`), with `rustfmt` and `clippy`
- Xcode 27
- [SwiftFormat](https://github.com/nicklockwood/SwiftFormat) for the Swift
  format check

## Layout

- `crates/`: the Rust workspace.
  - `onda-domain`: pure rules. `onda-app`: orchestration and the session
    engine.
  - `onda-audio`, `onda-asr`, `onda-asr-coreml`, `onda-models`,
    `onda-cleanup`, `onda-history`, `onda-settings`, `onda-adapters`: capture,
    speech engines, model management, cleanup, history, settings, and the
    core-side ports.
  - `onda-ffi`: the macOS composition root and the UniFFI bindings the shell
    calls. `uniffi-bindgen` runs the binding generator.
  - `onda-cli`: a developer CLI that drives the core from a terminal.
- `shell/macos/`: the Swift package for the app (AppKit/SwiftUI, Sparkle),
  its `Info.plist`, and its entitlements.
- `scripts/`: `build-ffi.sh` (core and Swift bindings), `bundle-app.sh` (the
  signed app bundle), `release.sh` (DMG, notarization), and
  `gen-model-manifest.py`.
- `assets/`: app icon and cue sounds.

## Build and run

```sh
scripts/bundle-app.sh            # debug build of build/Onda.app
scripts/bundle-app.sh release    # release build
open build/Onda.app
```

`bundle-app.sh` signs with the keychain's "Developer ID Application" identity
when there is one, so macOS privacy grants survive rebuilds; otherwise it
signs ad hoc. Set `SIGNING_IDENTITY` to override it (`-` for ad hoc).

To run against a scratch data folder instead of
`~/Library/Application Support/Onda`, set `ONDA_DATA_DIR`. Such runs also skip
background update checks:

```sh
ONDA_DATA_DIR="$(mktemp -d)" build/Onda.app/Contents/MacOS/Onda
```

The CLI exercises the core without the app (devices, recording, models,
transcription, dictation, settings). `--data-dir` selects the data folder:

```sh
cargo run -p onda-cli -- --help
cargo run -p onda-cli -- --data-dir /tmp/onda models list
```

## Test

These are the checks CI runs on every pull request and on `main`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
swiftformat --lint .
scripts/build-ffi.sh debug && swift build --package-path shell/macos --arch arm64
SIGNING_IDENTITY=- scripts/bundle-app.sh release
```

The Parakeet tests that load a real model are ignored by default; see
`crates/onda-asr-coreml/tests/real_model.rs` for how to run them.

## Release

Push a `v*` tag (for example `v1.2.0`). The release workflow builds the app
with that version and the run number as the build number, signs it with
Developer ID, notarizes and staples the DMG, creates the GitHub release with
generated notes, and publishes a signed Sparkle appcast to
`https://rselbach.github.io/onda/appcast.xml`. The secrets it needs are listed
at the top of `.github/workflows/release.yml`.

To build a DMG locally:

```sh
scripts/release.sh --skip-notarize   # build/Onda-<version>.dmg, not notarized
scripts/release.sh --version 1.2.0 --build 7
```

Notarizing locally uses the notarytool keychain profile `onda-notary`
(override with `NOTARY_PROFILE`), or `APPLE_ID`, `APPLE_TEAM_ID`, and
`APPLE_APP_PASSWORD`. Create the profile once with:

```sh
xcrun notarytool store-credentials onda-notary \
  --apple-id <apple-id> --team-id <team-id> --password <app-specific-password>
```
