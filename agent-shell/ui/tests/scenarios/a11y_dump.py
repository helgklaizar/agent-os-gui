#!/usr/bin/env python3
"""Launches agent-shell and prints its accessibility tree from AT-SPI: `depth<TAB>role<TAB>name`.

Usage: dbus-run-session -- python3 a11y_dump.py <path to agent-shell> [--menu]. Exit code 3: the application did not appear in AT-SPI.
With `--menu` the script waits for bar data, presses the "Agent OS menu" button (an AT-SPI action) and only then prints the tree.
"""
import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402


def find_app(pid):
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is not None and app.get_process_id() == pid:
            return app
    return None


def retry(call, default=None, attempts=4):
    """AT-SPI under load sometimes does not answer in time: retry the call instead of failing the scenario."""
    for _ in range(attempts):
        try:
            return call()
        except GLib.GError:
            time.sleep(0.5)
    return default


def states_of(node):
    """AT-SPI state names of a node, comma separated (`expanded`, `selected`, `focusable`…)."""
    states = retry(lambda: node.get_state_set().get_states(), [])
    return ",".join(sorted(s.value_nick for s in states))


def walk(node, depth, out, limit=2000, with_states=False):
    if len(out) >= limit or depth > 30:
        return
    role = retry(lambda: node.get_role().value_nick, "unknown")
    name = retry(lambda: node.get_name() or "", "")
    out.append((depth, role, name, states_of(node) if with_states else None))
    count = retry(node.get_child_count, 0)
    for i in range(count):
        child = retry(lambda: node.get_child_at_index(i))
        if child is not None:
            walk(child, depth + 1, out, limit, with_states)


def find_pressable(node, name, depth=0):
    """A node with this name and at least one action (for a MenuButton the action is on the inner button)."""
    if depth > 30:
        return None
    if retry(lambda: node.get_name() or "", "") == name and retry(lambda: node.get_role().value_nick, "") == "push-button":
        action = retry(node.get_action_iface)
        if action is not None and retry(action.get_n_actions, 0) > 0:
            return action
    for i in range(retry(node.get_child_count, 0)):
        child = retry(lambda: node.get_child_at_index(i))
        if child is not None:
            found = find_pressable(child, name, depth + 1)
            if found is not None:
                return found
    return None


def main():
    menu = "--menu" in sys.argv[2:]
    with_states = "--states" in sys.argv[2:]
    Atspi.init()
    proc = subprocess.Popen([sys.argv[1]])
    try:
        deadline = time.time() + 20
        app = None
        while time.time() < deadline and app is None:
            if proc.poll() is not None:
                print("application exited with code %s" % proc.returncode, file=sys.stderr)
                return 4
            app = find_app(proc.pid)
            time.sleep(0.3)
        if app is None:
            print("application did not appear in AT-SPI within 20 s", file=sys.stderr)
            return 3
        time.sleep(4.0 if menu else 1.5)  # the window tree is completed after the first frame; with the menu we also wait for bar frames
        if menu:
            action = find_pressable(app, "Agent OS menu")
            if action is None:
                print("menu button with an action not found", file=sys.stderr)
            else:
                done = retry(lambda: action.do_action(0), False)
                print("do_action(0) -> %s" % done, file=sys.stderr)
                time.sleep(1.0)
        out = []
        walk(app, 0, out, 2000, with_states)
        for depth, role, name, states in out:
            line = "%d\t%s\t%s" % (depth, role, name.replace("\t", " ").replace("\n", " "))
            print(line + ("\t" + states if states is not None else ""))
        return 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


sys.exit(main())
