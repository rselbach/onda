# Onda on Linux

The Linux shell uses GTK4 and libadwaita. Application state, microphone capture,
model downloads, history, and dictation processing come from the shared Rust
core. The macOS shell continues to use Swift and CoreML.

Hex informed the portal, clipboard, and compositor integration. The `wtype`
keymap workaround is adapted under its [MIT license](licenses/hex.txt).

## Release packages

Linux releases provide `.deb`, `.rpm`, `.pkg.tar.zst` (Arch), and `.tar.gz`
artifacts for **x86_64** and **ARM64**, with a SHA-256 checksum file for each
architecture. Use the package matching your architecture:

- Debian/Ubuntu: `sudo apt install ./Onda-VERSION-linux-ARCH.deb`
- Fedora: `sudo dnf install ./Onda-VERSION-linux-ARCH.rpm`
- Arch Linux/Arch Linux ARM: `sudo pacman -U ./Onda-VERSION-linux-ARCH.pkg.tar.zst`

The package manager installs the runtime dependencies. Arch packages are release
downloads; they do not add a pacman repository or an AUR entry.

For the archive, extract it and run `./install.sh` to install into `~/.local`.
Ensure `~/.local/bin` is on your desktop session's PATH. The archive's README
lists runtime dependencies and supports installation to another prefix.
Keep the private `lib/onda` directory alongside `bin`; copying only the binary
is insufficient. The app downloads speech models separately on first use.

Official packages build natively on Ubuntu 24.04 runners for each architecture.
They require Ubuntu 24.04 or a similarly recent desktop; see each package's
`share/doc/onda/build.json` for its library versions. x86_64 requires AVX2,
FMA, and F16C. ARM64 targets ARMv8.0-A. GTK4 Layer Shell is bundled privately;
GTK, libadwaita, ALSA, and desktop services come from the system. Keep the
desktop's existing PipeWire/PulseAudio setup.

Quit Onda before upgrading and restart afterward. Avoid mixing a per-user
installation with a system package: `~/.local/bin/onda` may shadow
`/usr/bin/onda`.

### Update checks

General → Updates offers automatic checks, Check for Updates, the installed
version, and the time of the last successful check. Automatic checks run shortly
after launch and daily while Onda is running. The preference persists across
launches. The tray menu also offers the current update action.

Onda verifies the signed release feed with the same pinned key as macOS. A newer
version offers the download matching the installed package format and CPU
architecture. The download opens in your browser; install it with your package
manager or the archive's installer, then restart Onda. Onda does not overwrite
package-managed files or request administrator privileges. Equal or older
versions and feeds older than one already verified are rejected as updates.

The first release publishing Linux artifacts in the signed feed enables this
path. Manual checks still work in scratch runs; `ONDA_DATA_DIR` disables
background network checks for testing.

### Building packages

With the native build dependencies below and Python 3.12+, binutils, `curl`,
and `tar` installed:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
python3 scripts/package-linux.py 3.2.0
```

This builds the app with the requested version, stages its desktop files,
library, and license notices, then writes all four formats and checksums to
`build/linux/`. The script downloads a pinned, checksum-verified nFPM binary;
neither root, Go, Docker, nor Podman is needed. Build each architecture natively.
Release compilation has its own `build/linux-target` directory so CPU settings
cannot leak in from a development build. Local builds record their host's
library requirements; Fedora builds are not Ubuntu-baseline release binaries.

On systems without GTK4 Layer Shell, install Meson, Ninja, Wayland development
files, and `wayland-protocols`, then build the pinned private copy first:

```sh
scripts/build-linux-layer-shell.sh
export PKG_CONFIG_PATH="$PWD/build/linux-layer-shell/install/lib/pkgconfig"
python3 scripts/package-linux.py 3.2.0
```

The **Linux packages** GitHub Actions workflow can also be run manually with a
version. It produces downloadable artifacts without publishing a release.
CI builds and checks all payloads on both architectures, installs the DEB on
Ubuntu and the RPM in Fedora containers, and installs the x86_64 Arch package
in the official Arch Linux container. It launches the installed app in isolated
desktop sessions. A `v*` release waits for those checks and attaches the packages
alongside the macOS artifacts, then verifies the published downloads.

To validate local payloads and installation, install `rpm`, `cpio`, `zstd`, and
`desktop-file-utils`, Xvfb, and `dbus-daemon`, then run:

```sh
python3 scripts/test-linux-packages.py 3.2.0
```

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

### ARM64 virtual machines on Apple Silicon

The speech library compiles for the host CPU with `-mcpu=native`. In an ARM64
virtual machine on Apple Silicon the virtual CPU reports SME without SVE2, and
GCC rejects the flag:

```
cc1: sorry, unimplemented: no support for 'sme' without 'sve2'
```

`scripts/build-linux.sh` detects this and builds the speech library for the
ARMv8.0-A baseline. Cargo cannot set an environment variable for one kind of
host, so `cargo build`, `cargo clippy`, and `cargo test` need the same value
exported first:

```sh
export TRANSCRIBE_CMAKE_ARGS="-DGGML_NATIVE=OFF -DGGML_CPU_ARM_ARCH=armv8-a"
```

A host needs it when `cc -mcpu=native -E - < /dev/null` fails. The script
leaves `TRANSCRIBE_CMAKE_ARGS` alone when it is already set, even to an empty
value.

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
you leave the field, switch providers, or close the chooser. The chooser also
states where each provider sends your transcript, links to the provider's page
for creating a key, and names the model in use.

Click the row under **Prompt** to read or replace the instructions Onda sends
with each transcript. One prompt serves every provider and model. Edit the text
and click **Save**. Click **Cancel** or press Escape to discard your changes.
**Restore Default** puts Onda's own prompt back in the editor, and **Save** then
removes your custom prompt. Onda does not accept a blank prompt.

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
- **Indicator:** a floating panel with the waveform, a clock, the transcribing,
  notice, and silent-microphone states, and a cancel button. It is a
  layer-shell window, or an X11 window (including XWayland on GNOME). It never
  takes keyboard focus, and only the cancel button takes clicks. Without
  either window, use the main window's status.
- **Tray:** optional StatusNotifierItem. Its menu starts and stops dictation,
  cancels a recording, pastes the last or a recent transcript, retries or
  discards a failed dictation, retries or re-downloads a failed model, switches
  language, and opens Onda or its settings. The icon and its tooltip follow the
  state: ready, recording, transcribing, preparing a model, or needing
  attention. Closing the main window keeps Onda running; the launcher reopens
  the existing process. Quit is always available in the window and through the
  CLI.
- **Media:** with **Pause media while recording** on, Onda pauses the MPRIS
  players that are playing and resumes only those. When none is playing, it
  mutes the default output while another application plays through it, and
  unmutes when the recording stops. Muting uses `pactl` (`pulseaudio-utils`,
  or `libpulse` on Arch Linux), which works with PipeWire and PulseAudio. A
  mute left behind by a crash is undone at the next launch.
- **Sounds:** the start and stop cues are the macOS ones, converted to WAV
  and installed in `share/onda/sounds`. They play through ALSA, so they need
  no sound theme and no package beyond the ALSA library. A cue file that is
  missing is logged at launch and stays silent.
- **Cancellation:** the window, tray, or `onda --cancel`. Onda does not claim
  Escape globally on Linux.
- **Target application on X11:** Onda identifies the application that has the
  keyboard focus when a dictation starts. It reads the window from
  `_NET_ACTIVE_WINDOW`, or from the input focus when no window manager
  publishes one, and the process from the X server. The name and icon come
  from the desktop entry that matches the window's `_GTK_APPLICATION_ID`,
  `StartupWMClass`, or `WM_CLASS`. A window without a desktop entry is named
  by its `WM_CLASS`. History shows the name and icon, the recording indicator
  shows the icon, and post-processing receives the name. If another
  application has the focus when the transcript is ready, Onda copies the
  transcript instead of pasting it and says so.
- **Target application on Wayland:** none is identified. No Wayland protocol
  tells an ordinary client which application has the focus, and AT-SPI does
  not answer for every application (see
  [Focused application on Wayland](#focused-application-on-wayland)). History
  rows and the indicator carry no application, post-processing gets no
  application name, and Onda pastes into whatever has the focus at delivery
  time.
- **Surrounding text:** unavailable in both session types.

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
Updates are installed through release packages or by rebuilding. The sidebar
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

The `onda-linux` tests include `tests/core_parity.rs`. It fails when a public
command of the core's `App` has neither a call site under `shell/linux/src` nor
a line in `tests/uncalled-core-commands.txt`, and when a listed command gains a
call site. Delete a command's line in the same change that starts calling it.

The real-GUI tests drive the GTK window and are marked `#[ignore]`.
`scripts/test-linux-gui.sh` finds every ignored test of the shell and runs each
one in its own process and data directory, under Xvfb with a private D-Bus
session and keyring. CI runs the script on every push. The tests check provider
switching, key storage and deletion, custom model selection, missing-key
feedback, prompt editing, replacement editing, shortcuts, history search,
confirmation dialogs, the model update notice, and critical alerts. They never
use your credentials or normal Onda data. The script also runs the overlay
test described below.

The script needs no list of tests. Mark a new real-GUI test `#[ignore]`, and
start the app with `launch` in `src/linux/ui/integration_tests.rs`. `launch`
writes the settings the test starts from, and those settings begin no model
download. The script routes the tests through a local proxy that refuses every
request, prints the requests each test made, and fails when one names
`huggingface.co`, the host of the speech models.

Set `ONDA_TEST_SCREENSHOT_DIR` to capture the pages in light and dark appearances.
Run it as a regular user:

```sh
sudo dnf install xorg-x11-server-Xvfb dbus-daemon dbus-tools gnome-keyring \
  libsecret xclip python3
scripts/test-linux-gui.sh
```

The overlay test shows the recording overlay's X11 window in Xvfb, above a
focused window. It checks that each state is on screen as drawn, that only the
cancel button takes a press, and that the focus never moves. Set
`ONDA_TEST_SCREENSHOT_DIR` to capture each state. The layer-shell window needs
a compositor that offers the protocol, so this test does not cover it.

```sh
sudo dnf install xorg-x11-server-Xvfb
scripts/test-linux-overlay.sh
```

The real dictation smoke test uses an isolated D-Bus session, Xvfb, a PipeWire
virtual microphone, and a GTK text field. Its private PipeWire and WirePlumber
processes use temporary sockets and disable physical-device discovery. The
desktop audio session stays running. Supply the v2 GGUF file above and
whisper.cpp's `samples/jfk.wav` (the expected transcript contains “country”).
It checks cancellation, the overlay's cancel button, command controls, global
push-to-talk, CPU inference, paste, clipboard restoration, and single-instance
activation. It checks that history records the text field's application, and
that a dictation whose window lost the focus before delivery is copied instead
of pasted. A stand-in tray host reads the tray's menu over D-Bus and clicks
its items. It never uses your normal Onda data or microphone. It also checks
microphone release after
standby and a fresh recording after capture has stopped. A stand-in MPRIS
player and a `pw-play` stream on a second virtual sink check media pause and
resume, the play/pause fallback, the output mute, and its recovery after Onda
is killed. With sounds on, it records that sink and checks that the start and
stop cues play in full and that the start cue alone does not mute the output.

```sh
sudo dnf install xorg-x11-server-Xvfb pipewire pipewire-pulseaudio \
  pipewire-alsa wireplumber pulseaudio-utils dbus-daemon gcc ripgrep \
  xclip xdotool xprop python3 python3-dbus python3-gobject
scripts/test-linux-dictation.sh \
  target/debug/onda /path/to/parakeet-tdt-0.6b-v2-Q8_0.gguf /path/to/jfk.wav
```

`pulseaudio-utils` supplies client tools compatible with PipeWire. The
`pulseaudio` daemon is not needed and conflicts with Fedora's
`pipewire-pulseaudio` package.

Desktop testing must include physical portal shortcuts and permission dialogs
on the supported GNOME/KDE versions. Headless X11 tests do not establish that
Wayland grants, focus, or paste work on every compositor.

## Focused application on Wayland

Onda identifies no target application in a Wayland session. AT-SPI was
evaluated as the route and rejected for now. It names the focused application
only when that application is on the accessibility bus and keeps its window
states there, and "no window is active" reads the same whether an unlisted
application has the focus or nothing has.

Measured on GNOME Shell 50.1 with `org.a11y.Status IsEnabled` false, the
default. Each row is what AT-SPI reported while that window had the focus:

| Application | Toolkit | Active window | Focused accessible |
| --- | --- | --- | --- |
| Python text view | GTK 4.22 | found | `text`, with `Text` and `EditableText` |
| Python text view | GTK 3.24 | found | `text`, with `Text` and `EditableText` |
| GNOME Terminal | GTK 3.24, VTE | found | `terminal`, with `Text` only |
| Ptyxis | GTK 4.22, VTE | found | a `button`, without `Text` |
| GTK 4 window started with `GTK_A11Y=none` | none | not listed | none |
| T3 Code (Electron) | Chromium | listed without any state, not even `showing` | none |

The GTK and terminal rows come from a private headless GNOME Shell session,
where opening and closing windows moved the focus. The Electron row is a
read-only query of a desktop session. Qt applications, Firefox, and KDE Plasma
were not available on the test machine and are not measured. A report took 21
to 59 ms with four applications on the bus.

`scripts/report-linux-focus.py` prints the same report for the session it runs
in, without changing it. It needs `python3-gobject` and the AT-SPI
introspection data (`at-spi2-core`). To measure another desktop or toolkit,
run it after a delay and focus the application meanwhile:

```sh
sleep 3; scripts/report-linux-focus.py
```

The route to the focused text field is the application's window in the
`active` state, then its descendant in the `focused` state, then that
accessible's `Text` interface for the text and the caret. The report's `pid`
is the process the accessibility bus saw, which is the process id Onda records
for an X11 target.

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
