#!/usr/bin/env python3
"""File, screen, and accessibility helper behind verify-onda-linux.

    onda-driver.py settings FILE KEY=JSON...   merge settings into FILE
    onda-driver.py history DATA_DIR            saved entries, one TSV row each
    onda-driver.py screenshot OUT [X Y W H]    the display, or one region of it
    onda-driver.py tree PID [--all]            the accessible tree of a process
    onda-driver.py press PID NAME [--role ROLE] [--index N]

`screenshot`, `tree`, and `press` act on the display and the accessibility
bus of the environment they run in, so verify-onda-linux starts them only
inside a run. `tree` prints one accessible per line as `role "name"` with
its states and, for an editable field, its text. It leaves out what is not
showing and what has neither a name nor a state unless `--all` is given, and
it never prints a password field.
`press` takes the name as `tree` prints it and runs the action of the control
it names: `click` on a button, `toggle` on a switch. Of two matches where one
holds the other it takes the outer one, and for a row it runs the one control
inside. GTK reports no usable positions and no action for a list row, a combo
row, or a text field, so `press` refuses those.
"""

import argparse
import json
import pathlib
import sys

TEXT_ROLES = ("text", "entry", "terminal", "paragraph")
# GTK also lists clipboard and selection actions; only these activate
ACTIVATIONS = ("click", "toggle", "activate")
HISTORY_FIELDS = ("timestamp", "appName", "modelID", "duration", "transcript")


def settings(args):
    path = pathlib.Path(args.file)
    values = json.loads(path.read_text()) if path.exists() else {}
    for pair in args.pairs:
        key, separator, value = pair.partition("=")
        if not key or not separator:
            sys.exit(f"expected KEY=JSON, got {pair}")
        try:
            values[key] = json.loads(value)
        except json.JSONDecodeError as error:
            sys.exit(f"{key} is not JSON ({error}); quote a string as '\"text\"'")
    path.write_text(json.dumps(values, indent=2) + "\n")


def history(args):
    rows = []
    for metadata in pathlib.Path(args.data_dir).glob("History/*/metadata.json"):
        entry = json.loads(metadata.read_text())
        rows.append(
            [str(entry.get(field, "-")).replace("\t", " ").replace("\n", " ") for field in HISTORY_FIELDS]
        )
    for row in sorted(rows):
        print("\t".join(row))


def screenshot(args):
    from PIL import ImageGrab

    image = ImageGrab.grab(xdisplay="")
    if args.region:
        x, y, width, height = args.region
        image = image.crop((max(x, 0), max(y, 0), min(x + width, image.width), min(y + height, image.height)))
    image.save(args.out)


def atspi():
    import warnings

    import gi

    # libatspi 2.60 marks Action.get_action_name deprecated, and the Python
    # binding offers nothing else that reads an action's name
    warnings.simplefilter("ignore", DeprecationWarning)

    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi

    Atspi.init()
    return Atspi


def application(Atspi, pid):
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(index)
        if app is not None and app.get_process_id() == pid:
            return app
    sys.exit(f"process {pid} is not on the accessibility bus")


def walk(Atspi, node, everything, depth=0):
    """Every accessible below `node` as (depth, accessible), parents first."""
    for index in range(node.get_child_count()):
        child = node.get_child_at_index(index)
        if child is None:
            continue
        if not everything and not child.get_state_set().contains(Atspi.StateType.SHOWING):
            continue
        yield depth, child
        yield from walk(Atspi, child, everything, depth + 1)


def describe(Atspi, node):
    held = node.get_state_set()
    role = node.get_role_name()
    line = f'{role} "{node.get_name()}"'
    marks = [
        mark
        for mark, state in (
            ("focused", Atspi.StateType.FOCUSED),
            ("selected", Atspi.StateType.SELECTED),
            ("checked", Atspi.StateType.CHECKED),
            ("expanded", Atspi.StateType.EXPANDED),
        )
        if held.contains(state)
    ]
    if not held.contains(Atspi.StateType.SENSITIVE):
        marks.append("disabled")
    if marks:
        line += f" [{', '.join(marks)}]"
    # an interface's methods are called unbound: bound, `get_text` and
    # `get_name` resolve to the accessible's own
    if role in TEXT_ROLES and node.get_text_iface() is not None:
        text = Atspi.Text.get_text(node, 0, Atspi.Text.get_character_count(node))
        line += f" text={json.dumps(text, ensure_ascii=False)}"
    return line


def tree(args):
    Atspi = atspi()
    # a container with no name and no state says nothing, so its children
    # take its place
    depths = []
    for depth, node in walk(Atspi, application(Atspi, args.pid), args.all):
        del depths[depth:]
        line = describe(Atspi, node)
        silent = not args.all and line == f'{node.get_role_name()} ""'
        depths.append(0 if silent else 1)
        if not silent:
            print("  " * sum(depths[:-1]) + line)


def activation(Atspi, node):
    """The index of the action that activates `node`, or None."""
    if node.get_action_iface() is None:
        return None
    for index in range(Atspi.Action.get_n_actions(node)):
        if Atspi.Action.get_action_name(node, index) in ACTIVATIONS:
            return index
    return None


def press(args):
    Atspi = atspi()
    # a row repeats its name on its label and its control, so a match inside
    # another match is the same thing to the user
    matches = []
    inside = []
    for depth, node in walk(Atspi, application(Atspi, args.pid), False):
        del inside[depth:]
        matched = node.get_name() == args.name and args.role in (None, node.get_role_name())
        if matched and not any(inside):
            matches.append(node)
        inside.append(matched)
    if args.index is not None:
        if not 1 <= args.index <= len(matches):
            sys.exit(f"--index {args.index} is outside the {len(matches)} matches")
        matches = [matches[args.index - 1]]
    if len(matches) != 1:
        found = "".join(f"\n  {describe(Atspi, node)}" for node in matches)
        sys.exit(f'expected one showing "{args.name}", found {len(matches)}{found}')
    # a row has no action; the one control inside it has
    controls = [matches[0]]
    if activation(Atspi, matches[0]) is None:
        controls = [
            node for _, node in walk(Atspi, matches[0], False) if activation(Atspi, node) is not None
        ]
    if len(controls) != 1:
        sys.exit(
            f"{describe(Atspi, matches[0])} offers no single action to run."
            " Click it instead: read its position from a screenshot."
        )
    control = controls[0]
    if not control.get_state_set().contains(Atspi.StateType.SENSITIVE):
        sys.exit(f"{describe(Atspi, control)} is disabled")
    index = activation(Atspi, control)
    pressed = f"{describe(Atspi, control)}: {Atspi.Action.get_action_name(control, index)}"
    if not Atspi.Action.do_action(control, index):
        sys.exit(f"refused, {pressed}")
    print(pressed)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    commands = parser.add_subparsers(required=True)
    command = commands.add_parser("settings")
    command.add_argument("file")
    command.add_argument("pairs", nargs="+", metavar="KEY=JSON")
    command.set_defaults(run=settings)
    command = commands.add_parser("history")
    command.add_argument("data_dir")
    command.set_defaults(run=history)
    command = commands.add_parser("screenshot")
    command.add_argument("out")
    command.add_argument("region", nargs="*", type=int, metavar="X Y W H")
    command.set_defaults(run=screenshot)
    command = commands.add_parser("tree")
    command.add_argument("pid", type=int)
    command.add_argument("--all", action="store_true")
    command.set_defaults(run=tree)
    command = commands.add_parser("press")
    command.add_argument("pid", type=int)
    command.add_argument("name")
    command.add_argument("--role")
    command.add_argument("--index", type=int)
    command.set_defaults(run=press)
    args = parser.parse_args()
    if args.run is screenshot and len(args.region) not in (0, 4):
        parser.error("a region is X Y W H")
    args.run(args)


if __name__ == "__main__":
    main()
