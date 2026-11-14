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
transcribe.cpp. Inference uses the CPU. GPU inference is not enabled in this
shell, and the CoreML Japanese model is unavailable.

Parakeet Unified shows your words in the recording overlay while you speak.
The preview needs at least 4 CPU cores: it costs a little over 2 CPU-seconds
for each second of speech, so two cores fall behind the speaker. The text
trails your voice by 1 to 3 seconds. The typed transcript is the same with or
without the preview. Parakeet TDT v2 and Ultra have no live preview on Linux,
because transcribe.cpp cannot stream them.

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

- **Recording a shortcut:** click **Record Shortcut** on the Hotkey page, or
  under **Another shortcut** in the first-run wizard, and press a key with
  Ctrl, Alt, or Super. Escape cancels. Modifier-only gestures are unavailable.
  The desktop keeps some combinations for itself, such as many Super
  shortcuts, and those never reach Onda.
- **Wayland shortcuts:** GlobalShortcuts portal, with the desktop's granted
  shortcut displayed in the window. The initial preference is Ctrl+Alt+Space.
- **X11 shortcuts:** native global key grabs. Onda claims the key by its place
  on the keyboard, so a shortcut stays on the keys you pressed in every
  keyboard layout.
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
  notice, and silent-microphone states, and a cancel button. Outside a
  dictation, the same panel shows a model download or load: its status line, a
  progress bar, and a button that hides the panel until that download or load
  ends. It is a layer-shell window, or an X11 window (including XWayland on
  GNOME). It never takes keyboard focus, and only its button takes clicks.
  Without either window, use the main window's status.
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
- **Cancellation:** the window, the overlay's cancel button, the tray, or
  `onda --cancel`. On X11, Escape also cancels: Onda claims the key when a
  dictation starts and gives it back when the dictation ends, so Escape works
  as usual in every application the rest of the time. Escape with the
  shortcut's own modifiers cancels too, and Num Lock, Caps Lock, and Scroll
  Lock make no difference. A combination that another application has already
  claimed stays with it. On Wayland, Onda does not claim Escape. The desktop
  offers no claim that lasts only for a dictation, and a permanent one would
  take Escape from every application.
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
- **Text before the caret:** Onda reads the focused text field through the
  accessibility bus (AT-SPI), in both session types, and fits the dictation
  to what precedes it. Mid-sentence it adds a space and lowers a word such as
  "And". After a sentence it adds the space and keeps the capital. This works
  in GTK 3 and GTK 4 text fields. A password field is never read, and a field
  that does not answer within 250 ms, or that is not on the accessibility
  bus, gets the transcript as the model wrote it (see
  [Text before the caret](#text-before-the-caret)).

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

**Getting Started** lists what is left to set up, with a button for each open
step, and **Run Setup Again** reopens the first-run wizard. Click the **Try
It** field and dictate to see the result without leaving Onda. Below it, the
page shows the dictation status and microphone level, **Start / Stop** and
**Cancel**, and the shortcut the desktop granted. A click on either button
leaves the caret in the field.

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
session and keyring. CI runs the script on every push. The tests check the
first-run wizard, the Getting Started checklist with a model missing and
installed, a paste into its try-it field, provider switching, key storage
and deletion, custom model
selection, missing-key feedback, prompt editing, replacement editing,
shortcuts, history search, confirmation dialogs, the model update notice, and
critical alerts. They never use your credentials or normal Onda data. The
script also runs the overlay tests described below, and the test of the text
field reader. That test serves a stand-in accessibility bus on the private
session bus, with fields, a password field, and applications that never
answer.

The script needs no list of tests. Mark a new real-GUI test `#[ignore]`, and
start the app with `launch` in `src/linux/ui/integration_tests.rs`. `launch`
writes the settings the test starts from, and those settings begin no model
download. The script routes the tests through a local proxy that refuses every
request, prints the requests each test made, and fails when one names
`huggingface.co`, the host of the speech models.

The first-run wizard starts the download of the recommended model when the
user leaves its Language step. The test that walks the wizard therefore puts
that model's file in its data directory first, with the right size and no
content, so the wizard finds the model downloaded. The Getting Started test
with a model installed starts from the same file. To have both tests also load
the model, name a real copy of the file:

```sh
ONDA_TEST_SPEECH_MODEL=/path/to/parakeet-tdt-0.6b-v2-Q8_0.gguf \
  scripts/test-linux-gui.sh
```

Set `ONDA_TEST_SCREENSHOT_DIR` to capture the pages in light and dark appearances.
Run it as a regular user:

```sh
sudo dnf install xorg-x11-server-Xvfb dbus-daemon dbus-tools gnome-keyring \
  libsecret xclip python3
scripts/test-linux-gui.sh
```

The overlay tests show the overlay's X11 window in Xvfb, above a focused
window. They check that each recording state and the download overlay are on
screen as drawn, that only the button takes a press, that the press cancels a
recording and hides a download, that a recording takes the panel from a
download, and that the focus never moves. Set `ONDA_TEST_SCREENSHOT_DIR` to
capture each state. The layer-shell window needs a compositor that offers the
protocol, so these tests do not cover it.

```sh
sudo dnf install xorg-x11-server-Xvfb
scripts/test-linux-overlay.sh
```

The real dictation smoke test uses an isolated D-Bus session, Xvfb, a PipeWire
virtual microphone, and a GTK text field. Its private PipeWire and WirePlumber
processes use temporary sockets and disable physical-device discovery. The
desktop audio session stays running. Supply the v2 GGUF file above and
whisper.cpp's `samples/jfk.wav` (the expected transcript contains “country”).
Add the Unified GGUF file as a fourth argument to also check live preview:
preview text appears before the recording stops on a host with 4 CPUs or
more, and no preview runs when Onda is held to 2 CPUs.
It checks cancellation, the overlay's cancel button, Escape, command controls,
global push-to-talk, CPU inference, paste, clipboard restoration, and
single-instance activation. It starts a private accessibility bus and checks
that a dictation after "Troy Barnes joined the" continues with one space and
a lowercase "and", that one after a full sentence keeps its capital, that a
dictation into a password field arrives unchanged, and that the log holds
neither the fields' text nor the transcript. It checks that history records the text field's application, and
that a dictation whose window lost the focus before delivery is copied instead
of pasted. It clicks the try-it field of the Getting Started page and checks
that a dictation started with the shortcut, and one started with the page's
Start / Stop button, both arrive in the field. A stand-in tray host reads the
tray's menu over D-Bus and clicks
its items. It never uses your normal Onda data or microphone. It also checks
microphone release after
standby and a fresh recording after capture has stopped. A stand-in MPRIS
player and a `pw-play` stream on a second virtual sink check media pause and
resume, the play/pause fallback, the output mute, and its recovery after Onda
is killed. For Escape, it checks that the key cancels a recording with speech
in it and delivers nothing, that it still cancels with Num Lock or Caps Lock
on, and that it reaches the text field again after a cancel, after a delivered
dictation, and after Onda is killed while recording. With sounds on, it records that sink and checks that the start and
stop cues play in full and that the start cue alone does not mute the output.

```sh
sudo dnf install xorg-x11-server-Xvfb pipewire pipewire-pulseaudio \
  pipewire-alsa wireplumber pulseaudio-utils dbus-daemon gcc ripgrep \
  xclip xdotool xprop python3 python3-dbus python3-gobject at-spi2-core
scripts/test-linux-dictation.sh \
  target/debug/onda /path/to/parakeet-tdt-0.6b-v2-Q8_0.gguf /path/to/jfk.wav \
  /path/to/parakeet-unified-en-0.6b-Q8_0.gguf
```

The speech engine has tests of its own against the real models. They check
that the catalog offers live preview for exactly the models that stream, and
that the final transcript of `jfk.wav` is the same with and without a preview.
Put the three GGUF files and `jfk.wav` in one folder:

```sh
ONDA_TEST_MODEL_DIR=/path/to/that/folder \
  cargo test --locked -p onda-linux speech:: -- --test-threads=1
```

Without the variable these tests pass without running.

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

## Text before the caret

Onda reads the focused field over AT-SPI with the D-Bus access GTK already
gives it. It needs no library or package of its own. The read works for an
application that is on the accessibility bus and keeps its focus there. For
any other application Onda finds no field and delivers the transcript
unchanged, as it did before it could read fields.

The route is the one described above. On X11 Onda asks only the target
application, which it finds by process id. On Wayland it has no target, so it
asks every application for its active window. It reads the role, the text,
and the selection of the focused accessible. The dictation follows the start
of the selection, or the caret when nothing is selected. AT-SPI counts in
characters and Onda's core in UTF-16 units, so Onda converts.

Each row is a real dictation on GNOME Shell 50.1 with `org.a11y.Status
IsEnabled` false, the default. "Joined" means the transcript arrived fitted
to the text before the caret:

| Field | Toolkit | Session | Result |
| --- | --- | --- | --- |
| Text view | GTK 4.22 | X11, Wayland | joined |
| Text view | GTK 3.24 | Wayland | joined |
| Text view, caret after emoji | GTK 4.22 | Wayland | joined, with the caret converted |
| GNOME Text Editor 50 | GTK 4, GtkSourceView | Wayland | joined, 31 accessibles in 24 ms |
| `GtkPasswordEntry` | GTK 4.22 | X11, Wayland | not read: role `password text` |
| `GtkEntry` with hidden text | GTK 4.22 | Wayland | not joined: see below |
| Button with the focus | GTK 4.22 | Wayland | no text, unchanged |
| Text view started with `GTK_A11Y=none` | none | X11 | not on the bus, unchanged |
| Text view, beside a stopped application | GTK 4.22 | Wayland | joined, after 100 ms |

The X11 rows come from `scripts/test-linux-dictation.sh`. The Wayland rows
come from a private headless GNOME Shell, where Onda cannot paste, so the
transcript was read from the clipboard it was left on.

Limits found by measuring:

- **Password fields.** GTK 3 entries with hidden text, `GtkPasswordEntry`, and
  libadwaita's `AdwPasswordEntryRow` report the role `password text`, and Onda
  does not ask them for their text. A GTK 4 `GtkEntry` with hidden text
  reports as an ordinary field whose text is its mask, such as `•••`. Onda
  treats a field that holds nothing but one repeated mask character as
  hidden. A field that really holds only `***` is therefore not joined either.
- **Applications that do not answer.** One call may take 100 ms and the whole
  read 250 ms. On Wayland a stopped application ahead of the focused one
  costs 100 ms, and three of them use up the read.
- **Terminals.** Not established. In the test session GNOME Terminal and
  Ptyxis each had the focus on a button, not on the terminal, and nothing was
  joined.
- **Other toolkits.** Not measured, because the test machine has no Qt
  application, no KDE Plasma, and a Firefox that does not start in a private
  session. An Electron application was listed without window states (see
  [Focused application on Wayland](#focused-application-on-wayland)), which
  Onda reads as no field. Onda does not switch on the desktop's accessibility
  (`org.a11y.Status IsEnabled`) to make an application report more.

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
