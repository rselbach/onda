#!/usr/bin/env python3
"""Stand-in tray host for a private test bus.

    tray-host.py watch                 own org.kde.StatusNotifierWatcher
    tray-host.py show                  print the icon, the tooltip, and the menu
    tray-host.py click LABEL [LABEL]   open the menu, then click the item

`show` prints one menu row per line: a submenu's rows are indented, a disabled
row is in brackets, and a checkable row starts with its mark. `click` takes
the labels down to the item, without marks or brackets.
"""

import sys
import time

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

WATCHER = "org.kde.StatusNotifierWatcher"
WATCHER_PATH = "/StatusNotifierWatcher"
ITEM = "org.kde.StatusNotifierItem"
MENU = "com.canonical.dbusmenu"
PROPERTIES = "org.freedesktop.DBus.Properties"


class Watcher(dbus.service.Object):
    """Accepts every item and claims a host, so the item publishes its menu."""

    def __init__(self, bus):
        super().__init__(bus, WATCHER_PATH)
        self.items = dbus.Array([], signature="s")

    @dbus.service.method(WATCHER, in_signature="s")
    def RegisterStatusNotifierItem(self, service):
        self.items.append(service)

    @dbus.service.method(PROPERTIES, in_signature="ss", out_signature="v")
    def Get(self, _interface, name):
        return self.GetAll(WATCHER)[name]

    @dbus.service.method(PROPERTIES, in_signature="s", out_signature="a{sv}")
    def GetAll(self, _interface):
        return {
            "IsStatusNotifierHostRegistered": True,
            "ProtocolVersion": dbus.Int32(0),
            "RegisteredStatusNotifierItems": self.items,
        }


def watch():
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    name = dbus.service.BusName(WATCHER, bus, do_not_queue=True)
    watcher = Watcher(bus)
    GLib.MainLoop().run()
    del name, watcher


def tray(bus):
    watcher = bus.get_object(WATCHER, WATCHER_PATH)
    items = watcher.Get(WATCHER, "RegisteredStatusNotifierItems", dbus_interface=PROPERTIES)
    items = [item for item in items if bus.name_has_owner(item)]
    if len(items) != 1:
        sys.exit(f"expected one tray item, found {len(items)}")
    return str(items[0])


def rows(node, path=()):
    """Every row below `node` as (id, labels down to the row, properties)."""
    for child in node[2]:
        number, properties, _ = child
        if properties.get("type") == "separator":
            yield int(number), path + ("---",), properties
            continue
        here = path + (str(properties.get("label", "")),)
        yield int(number), here, properties
        yield from rows(child, here)


def layout(bus, name):
    menu = dbus.Interface(bus.get_object(name, "/MenuBar"), MENU)
    _, root = menu.GetLayout(0, -1, [])
    return menu, list(rows(root))


def show(bus, name):
    item = bus.get_object(name, "/StatusNotifierItem")
    properties = item.GetAll(ITEM, dbus_interface=PROPERTIES)
    print(f"icon {properties['IconName']}")
    print(f"tooltip {properties['ToolTip'][2]}")
    for _, path, properties in layout(bus, name)[1]:
        label = path[-1]
        if properties.get("toggle-type") == "checkmark":
            label = f"{'✓' if properties.get('toggle-state') == 1 else '·'} {label}"
        if not properties.get("enabled", True):
            label = f"[{label}]"
        print("  " * (len(path) - 1) + label)


def click(bus, name, labels):
    menu = dbus.Interface(bus.get_object(name, "/MenuBar"), MENU)
    menu.AboutToShow(0)
    # the app answers an opening menu with a new one, which renumbers the items
    time.sleep(0.3)
    found = layout(bus, name)[1]
    matches = [number for number, path, _ in found if path == tuple(labels)]
    if len(matches) != 1:
        sys.exit(f"expected one item at {' > '.join(labels)}, found {len(matches)}")
    menu.Event(matches[0], "clicked", dbus.Int32(0, variant_level=1), dbus.UInt32(0))


def main():
    command, *labels = sys.argv[1:] or [""]
    if command == "watch" and not labels:
        return watch()
    bus = dbus.SessionBus()
    if command == "show" and not labels:
        return show(bus, tray(bus))
    if command == "click" and labels:
        return click(bus, tray(bus), labels)
    sys.exit(__doc__)


if __name__ == "__main__":
    main()
