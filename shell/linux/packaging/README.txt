Onda for Linux

Install the .deb with `sudo apt install ./Onda-*.deb`, the .rpm with
`sudo dnf install ./Onda-*.rpm`, or the Arch package with
`sudo pacman -U ./Onda-*.pkg.tar.zst`. Select the file for your architecture.

For this archive, run ./install.sh to install into ~/.local. Ensure ~/.local/bin
is in your desktop session's PATH, then launch Onda from the application menu.
An optional prefix is supported: ./install.sh /path/to/prefix. You can also run
./bin/onda directly; install the desktop entry before using desktop portals.
Keep bin/ and lib/ together: Onda loads its private GTK4 Layer Shell library
relative to the executable. Do not copy just the binary.

Official builds target Ubuntu 24.04 or newer, or a comparably recent Linux
desktop with GTK 4.14+, libadwaita 1.5+, GLib 2.80+, and glibc 2.39+.
x86_64 requires AVX2, FMA, and F16C; ARM64 requires ARMv8.0-A.
The exact library versions used by each build are in share/doc/onda/build.json.
Packages made locally on newer distributions require those newer libraries.

Archive runtime dependencies on Ubuntu:
  sudo apt install libgtk-4-1 libadwaita-1-0 libasound2t64 libstdc++6 \
    libsecret-tools wl-clipboard wtype xclip xdotool xdg-utils xdg-desktop-portal

Archive runtime dependencies on Fedora:
  sudo dnf install gtk4 libadwaita alsa-lib libstdc++ libsecret \
    wl-clipboard wtype xclip xdotool xdg-utils xdg-desktop-portal

Archive runtime dependencies on Arch Linux:
  sudo pacman -S --needed gtk4 libadwaita alsa-lib gcc-libs libsecret \
    wl-clipboard wtype xclip xdotool xdg-utils xdg-desktop-portal

Muting other audio while you dictate also needs pactl, from pulseaudio-utils
on Ubuntu and Fedora and from libpulse on Arch Linux.

Use your desktop's portal backend and an unlocked Secret Service keyring
(such as GNOME Keyring) for global shortcuts, keyboard permission, and API keys.
Keep your existing PipeWire or PulseAudio setup; no PulseAudio daemon is needed.
Audio must be available through ALSA, normally via the desktop's ALSA plugin.

Choose Speech > Download & Use on first launch to download a speech model.
Models are not included in these packages. Speech recognition runs locally on
the CPU. Optional post-processing sends transcript text to the chosen provider.

Close Onda before upgrading, then install the newer package or archive and
restart it. General > Updates checks the signed release feed automatically and
offers a download matching this package's format and architecture. Automatic
checks can be disabled; Check for Updates remains available. Downloads open in
your browser, and installation stays under your control.
User data is in ${XDG_DATA_HOME:-~/.local/share}/onda and survives upgrades.
Do not mix package-manager and per-user installations; ~/.local/bin/onda can
shadow /usr/bin/onda. Quit an older running instance before testing an upgrade.

Verify downloads with `sha256sum -c Onda-VERSION-linux-ARCH.sha256` in the
directory containing all four release artifacts for that architecture.

Documentation: https://github.com/rselbach/onda/tree/main/shell/linux
Licenses and third-party notices: share/doc/onda/
