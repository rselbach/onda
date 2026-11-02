# Onda on Linux

The Linux shell uses GTK4 and libadwaita. Application state, microphone capture,
model downloads, history, and dictation processing come from the shared Rust
core. The macOS shell continues to use Swift and CoreML.

Hex informed the portal, clipboard, and compositor integration. The `wtype`
keymap workaround is adapted under its [MIT license](licenses/hex.txt).

## Build

Requires Rust 1.99+, a C++ compiler, CMake, GTK 4.12+, libadwaita 1.5+,
GLib 2.80+, GTK4 Layer Shell, ALSA development files, and pkg-config.

On Fedora:

```sh
sudo dnf install rust cargo rustfmt clippy gcc-c++ cmake clang-devel pkgconf-pkg-config \
  gtk4-devel libadwaita-devel gtk4-layer-shell-devel alsa-lib-devel \
  openssl-devel libX11-devel libXtst-devel libxkbcommon-devel wayland-devel \
  wl-clipboard wtype xclip xdotool libsecret
scripts/build-linux.sh release
scripts/install-linux.sh
onda
```

The desktop entry is also the portal identity. Install it before testing global
shortcuts or keyboard permission prompts. The launcher must find `onda` in PATH.
The XDG desktop session bus must be available.

Choose **Speech**, then **Download & Use**. Linux offers pinned,
SHA-256-verified GGUF versions of Parakeet TDT v2, Ultra, and Unified through
transcribe.cpp. Inference uses the CPU; live preview and the CoreML Japanese
model are unavailable. GPU inference is not enabled in this shell.

## Post-processing

Open **Post-processing** and enable **Post-process transcripts**. Click **Model**
to choose OpenRouter or Fireworks, enter the provider's API key, and select a
model. Search the catalog or enter a model ID directly. Click **Done**, then
**Test with Sample Text** to check the configuration before dictating.

Post-processing sends the transcript to the selected cloud provider. API keys
are stored in the desktop's Secret Service keyring through `secret-tool`
(Fedora's `libsecret` package), rather than in Onda's settings file. A compatible
keyring, such as GNOME Keyring, must be available and unlocked. Keys save when
you leave the field, switch providers, or close the chooser.

## Desktop integration

- **Wayland shortcuts:** GlobalShortcuts portal, with the desktop's granted
  shortcut displayed in the window. The initial preference is Ctrl+Alt+Space.
  Configure a key with modifiers; modifier-only gestures are unavailable.
- **X11 shortcuts:** native global key grabs.
- **Paste:** `wl-copy` plus `wtype` when the compositor advertises virtual
  keyboards, otherwise a keyboard-only RemoteDesktop portal session. X11 uses
  `xclip` and `xdotool`. Refused or failed injection leaves the transcript on
  the clipboard. Terminal paste selects Ctrl+Shift+V for the current session.
- **Permission lifetime:** portal restore tokens are kept with owner-only
  permissions. A restorable keyboard session closes after 15 idle seconds.
  The desktop may show a remote-control indicator while it is open.
- **Clipboard preservation:** bounded to 32 MiB and one second of reads.
  Wayland preserves offered formats when data-control is available; X11
  preserves text-only selections. Unsupported or unreadable selections leave
  the transcript on the clipboard. Restoration waits 500 ms and checks the
  current text; this is not an acknowledgement from the target application.
- **Indicator:** a passive layer-shell window, or a click-through X11 window
  (including XWayland on GNOME). Without either, use the main window's status.
- **Tray:** optional StatusNotifierItem. Closing the main window keeps Onda
  running; the launcher reopens the existing process. Quit is always available
  in the window and through the CLI.
- **Cancellation:** the window, tray, or `onda --cancel`. Onda does not claim
  Escape globally on Linux.
- **Target context:** Linux currently pastes into the focused application at
  delivery time. Target-app identification and surrounding text are unavailable.

## Desktop command bindings

```sh
onda --background
onda --toggle
onda --start
onda --stop
onda --cancel
onda --paste-last
onda --quit
```

Control commands require a running Onda. Start/stop are idempotent. On desktops
without a shortcut portal, bind `onda --toggle` in keyboard settings. A desktop
with separate press/release commands can bind `--start` and `--stop`.

`ONDA_DATA_DIR` selects scratch data, a separate application instance, and a
separate keyring namespace:

```sh
ONDA_DATA_DIR=/tmp/onda-demo target/debug/onda
ONDA_DATA_DIR=/tmp/onda-demo target/debug/onda --quit
```

Normal data lives in `${XDG_DATA_HOME:-~/.local/share}/onda`. No system-wide
daemon, input-device permissions, or privileged input injection is required.
Updates are installed by rebuilding/reinstalling this version. The sidebar
groups Getting Started, General, Hotkey, Speech, Post-Processing, Replacements,
History, and About. Settings use native libadwaita groups and follow the desktop's
light or dark appearance. Apple Intelligence and macOS-specific permissions and
update controls are unavailable.

**Replacements** edits phrases after transcription and post-processing. Changes
save as you type and when you leave the page. **History** includes transcript
search, date groups, retention, optional recordings, and a confirmation before
clearing saved history.

## Checks

```sh
cargo fmt --all --check
cargo clippy -p onda-linux --all-targets -- -D warnings
cargo test -p onda-domain -p onda-app -p onda-adapters -p onda-audio -p onda-linux -- --test-threads=1
```

The post-processing GUI test uses Xvfb and a private D-Bus session and keyring.
It checks provider switching, key storage and deletion, custom model selection,
missing-key feedback, replacement editing, shortcuts, history search, and
confirmation dialogs without using your credentials or normal Onda data.
Set `ONDA_TEST_SCREENSHOT_DIR` to capture the pages in light and dark appearances.
Run it as a regular user:

```sh
sudo dnf install xorg-x11-server-Xvfb dbus-daemon gnome-keyring libsecret
scripts/test-linux-post-processing.sh
```

The real dictation smoke test uses an isolated D-Bus session, Xvfb, a PipeWire
virtual microphone, and a GTK text field. Its private PipeWire and WirePlumber
processes use temporary sockets and disable physical-device discovery. The
desktop audio session stays running. Supply the v2 GGUF file above and
whisper.cpp's `samples/jfk.wav` (the expected transcript contains “country”).
It checks cancellation, command controls, global push-to-talk, CPU inference,
paste, clipboard restoration, and single-instance activation. It never uses
your normal Onda data or microphone. It also checks microphone release after
standby and a fresh recording after capture has stopped.

```sh
sudo dnf install xorg-x11-server-Xvfb pipewire pipewire-pulseaudio \
  pipewire-alsa wireplumber pulseaudio-utils dbus-daemon gcc ripgrep \
  xclip xdotool python3
scripts/test-linux-dictation.sh \
  target/debug/onda /path/to/parakeet-tdt-0.6b-v2-Q8_0.gguf /path/to/jfk.wav
```

`pulseaudio-utils` supplies client tools compatible with PipeWire. The
`pulseaudio` daemon is not needed and conflicts with Fedora's
`pipewire-pulseaudio` package.

Desktop testing must include physical portal shortcuts and permission dialogs
on the supported GNOME/KDE versions. Headless X11 tests do not establish that
Wayland grants, focus, or paste work on every compositor.

## Blank GNOME shortcut dialog

Fedora's `gnome-control-center-50.0-1.fc44` can show the **Add Keyboard
Shortcuts** dialog with buttons and icons but no text. Its shortcut helper
maps the text scaling factor directly to GTK's font DPI, producing zero-size
fonts. Verified fixed with `gnome-control-center-50.4-1.fc44`, whose bundled
[libgxdp converts the units correctly](https://github.com/GNOME/libgxdp/blob/1fe6de799f39b3126e8d0db66fa4f16c10fd9bd6/src/gxdp-wayland.c).

```sh
sudo dnf upgrade gnome-control-center
```

Close the old dialog and restart Onda after updating. Changing Onda's GTK
renderer does not change this separately running GNOME helper.
