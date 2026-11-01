# Onda

Onda is a macOS menu bar app for dictation: hold or toggle a hotkey, speak,
and the transcript is typed into the frontmost app. Speech recognition runs on
the device (Parakeet on CoreML, or Apple's System Speech); optional transcript
cleanup goes through OpenRouter. It is a Rust rewrite of Jabber: the core is a
Rust workspace, and a thin Swift shell supplies the native UI and system
integration.

## Requirements

- macOS 26 or later on Apple Silicon (arm64 only)
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
  - `onda-update`: the self-updater (signed feed, verified download, macOS
    bundle swap) and `onda-update-tool`, which signs and checks the feeds at
    release.
  - `onda-ffi`: the macOS composition root and the UniFFI bindings the shell
    calls. `uniffi-bindgen` runs the binding generator.
  - `onda-cli`: a developer CLI that drives the core from a terminal.
- `shell/macos/`: the Swift package for the app (AppKit/SwiftUI),
  its `Info.plist`, and its entitlements.
- `scripts/`: `build-ffi.sh` (core and Swift bindings), `bundle-app.sh` (the
  signed app bundle), `release.sh` (update ZIP, DMG, notarization), and
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
Developer ID, notarizes and staples the app, and packages that bundle as
`Onda-<version>-macos-arm64.zip` (what the in-app updater installs) and as
a notarized, stapled `Onda-<version>.dmg`. It creates the GitHub release
with both and generated notes, signs the update feed, downloads and
verifies the published assets against it, and only then deploys GitHub
Pages with two feeds: `https://rselbach.github.io/onda/update.json`, which
Onda reads, and the legacy Sparkle appcast
`https://rselbach.github.io/onda/appcast.xml`, which builds from before the
switch read to receive the release that replaces Sparkle. The secrets it
needs are listed at the top of `.github/workflows/release.yml`.

Existing release assets are never overwritten: a rerun whose built bytes
differ fails verification and leaves the feeds unchanged. Publish a new
version for changed bytes.

### Updates

`update.json` is one JSON envelope: the base64 release JSON (bundle id,
channel, version, build number, and per OS and architecture the minimum OS,
size, SHA-256, and GitHub Releases URL of the ZIP and DMG) and an Ed25519
signature over `onda-update-feed-v1\n` followed by that JSON. Onda pins the
public key (`onda_update::PUBLIC_KEY`, also `SUPublicEDKey` in Info.plist)
and only installs a strictly higher build. It refuses a feed older than one
it already verified, so to withdraw a release publish a newer build rather
than an older feed. The release workflow's `SPARKLE_EDDSA_PRIVATE_KEY`
(a base64 32-byte Ed25519 seed, as `generate_keys -x` exports it) signs
both the feed and the legacy appcast's DMG; `onda-update-tool` refuses any
key whose public half is not the pinned one. Key rotation needs a bridge
release that existing clients can verify and that trusts the replacement
key before publishing feeds signed with it.

A downloaded update is unpacked into a new `.onda-update-…` folder beside
the installed app and must be `com.rselbach.onda` at the signed version and
build, arm64, runnable on this macOS, and signed by Onda's notarized
Developer ID (`crates/onda-update/update-requirement.txt`). The folder holds
a receipt naming the installed app's real path, locked by the process that
staged it. Onda only ever deletes staging folders whose receipt names its
own copy and that no running Onda holds, so other copies in the same folder
and look-alike names are left alone; a staging folder orphaned by renaming
or moving the app stays until removed by hand.

Checks continue while an update waits: a newer release replaces the staged
one once it is verified, and a failed check leaves the staged one ready.
"Restart to Update" is refused while dictating, and once accepted no
dictation starts until Onda quits or the restart is abandoned. It first
re-reads the feed for up to 5 seconds: a newer installable release is
downloaded instead and the user restarts again when it is ready. A verified
feed that removes this platform drops the staged update; offline, the
staged update installs. It then re-verifies the staged app and that
the installed app is still the running build, swaps the two bundles in one
rename, and relaunches; the previous bundle is deleted on the next launch.
If the relaunch cannot be scheduled the swap is undone. If undoing also
fails, the new version stays installed, the previous bundle stays in the
staging folder (its path is logged), and Onda asks the user to quit and
reopen it. There is no automatic rollback once the new version is running.
A copy Onda cannot replace (read-only volume, App Translocation, or a folder
this user cannot write) offers the DMG instead.

To build the ZIP and DMG locally:

```sh
scripts/release.sh --skip-notarize   # not notarized
scripts/release.sh --version 1.2.0 --build 7
```

Notarizing locally uses the notarytool keychain profile `onda-notary`
(override with `NOTARY_PROFILE`), or `APPLE_ID`, `APPLE_TEAM_ID`, and
`APPLE_APP_PASSWORD`. Create the profile once with:

```sh
xcrun notarytool store-credentials onda-notary \
  --apple-id <apple-id> --team-id <team-id> --password <app-specific-password>
```
