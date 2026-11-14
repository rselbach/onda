#!/usr/bin/env python3
"""MPRIS player double for scripts/test-linux-dictation.sh.

Owns org.mpris.MediaPlayer2.<name> on the session bus, reports
PlaybackStatus, and appends each Pause, Play, and PlayPause call to a log.
With --ignore-pause it accepts Pause without pausing, as a player that only
honors the toggle does.
"""

import argparse

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

PLAYER = "org.mpris.MediaPlayer2.Player"


class Player(dbus.service.Object):
    def __init__(self, bus_name, log, ignore_pause):
        super().__init__(bus_name, "/org/mpris/MediaPlayer2")
        self.log = log
        self.ignore_pause = ignore_pause
        self.status = "Playing"

    def record(self, call):
        with open(self.log, "a") as log:
            log.write(f"{call}\n")

    @dbus.service.method(PLAYER)
    def Pause(self):
        self.record("Pause")
        if not self.ignore_pause:
            self.status = "Paused"

    @dbus.service.method(PLAYER)
    def Play(self):
        self.record("Play")
        self.status = "Playing"

    @dbus.service.method(PLAYER)
    def PlayPause(self):
        self.record("PlayPause")
        self.status = "Paused" if self.status == "Playing" else "Playing"

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, _interface, _name):
        return self.status


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("name")
    parser.add_argument("log")
    parser.add_argument("--ignore-pause", action="store_true")
    args = parser.parse_args()
    DBusGMainLoop(set_as_default=True)
    name = dbus.service.BusName(
        f"org.mpris.MediaPlayer2.{args.name}", dbus.SessionBus()
    )
    Player(name, args.log, args.ignore_pause)
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
