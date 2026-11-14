#!/usr/bin/env python3
"""Report what AT-SPI says about the focused application and text field.

Lists every application on the accessibility bus with its toolkit, process
and windows. For a window in the ACTIVE state it finds the accessible in the
FOCUSED state and prints its role, its interfaces, and the length and caret
of its text. It reads only: it registers no listeners and prints no text.
Run it with `sleep 3; scripts/report-linux-focus.py` and focus the
application to examine while it waits.
"""

import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402

# a browser document can hold tens of thousands of accessibles
NODE_BUDGET = 4000


def states(accessible):
    held = accessible.get_state_set()
    return [
        name
        for name, state in (
            ("active", Atspi.StateType.ACTIVE),
            ("focused", Atspi.StateType.FOCUSED),
            ("showing", Atspi.StateType.SHOWING),
        )
        if held.contains(state)
    ]


def focused(window):
    """The FOCUSED accessible under `window`, and how many were visited."""
    stack = [window]
    visited = 0
    while stack and visited < NODE_BUDGET:
        visited += 1
        current = stack.pop()
        try:
            if current.get_state_set().contains(Atspi.StateType.FOCUSED):
                return current, visited
            for index in range(current.get_child_count()):
                child = current.get_child_at_index(index)
                if child is not None:
                    stack.append(child)
        except GLib.Error:
            # an accessible that vanished or whose application did not answer
            continue
    return None, visited


def describe(accessible):
    interfaces = accessible.get_interfaces()
    line = f"role={accessible.get_role_name()} interfaces={','.join(interfaces)}"
    if "Text" in interfaces:
        text = accessible.get_text_iface()
        line += f" chars={text.get_character_count()} caret={text.get_caret_offset()}"
    return line


def report_window(window):
    held = states(window)
    print(f"  window role={window.get_role_name()} states={held}")
    if "active" not in held:
        return
    found, visited = focused(window)
    if found is None:
        print(f"    focused: none among {visited} accessibles")
        return
    print(f"    focused after {visited} accessibles: {describe(found)}")


def main():
    started = time.monotonic()
    Atspi.init()
    Atspi.set_timeout(250, 1000)
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        try:
            app = desktop.get_child_at_index(index)
            toolkit = f"{app.get_toolkit_name()} {app.get_toolkit_version()}"
            print(
                f"app {app.get_name()!r} toolkit={toolkit}"
                f" pid={app.get_process_id()} windows={app.get_child_count()}"
            )
            for position in range(app.get_child_count()):
                report_window(app.get_child_at_index(position))
        except GLib.Error as error:
            print(f"app #{index}: unreadable ({error.message})", file=sys.stderr)
    print(f"report took {1000 * (time.monotonic() - started):.0f} ms")


if __name__ == "__main__":
    main()
