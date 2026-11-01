# Onda

Multiplatform speech-to-text app.

## Requirements

- macOS 26 or later on Apple Silicon (arm64 only)
- Rust stable 1.99 or later (`rustup`), with `rustfmt` and `clippy`
- Xcode 27
- [SwiftFormat](https://github.com/nicklockwood/SwiftFormat) for the Swift
  format check

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
