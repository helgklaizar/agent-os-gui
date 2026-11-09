#!/usr/bin/env python3
"""Prints the accessibility tree of an already running process from AT-SPI: `depth<TAB>role<TAB>name<TAB>states`.

Usage: python3 -I a11y_pid.py <pid>. Exit code 3: the process is not in AT-SPI. The process must run the GTK main loop while the script works.
"""
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402


def retry(call, default=None, attempts=4):
    for _ in range(attempts):
        try:
            return call()
        except GLib.GError:
            time.sleep(0.5)
    return default


def walk(node, depth, out):
    if len(out) >= 2000 or depth > 30:
        return
    role = retry(lambda: node.get_role().value_nick, "unknown")
    name = retry(lambda: node.get_name() or "", "")
    states = retry(lambda: node.get_state_set().get_states(), [])
    out.append((depth, role, name, ",".join(sorted(s.value_nick for s in states))))
    for i in range(retry(node.get_child_count, 0)):
        child = retry(lambda: node.get_child_at_index(i))
        if child is not None:
            walk(child, depth + 1, out)


def main():
    pid = int(sys.argv[1])
    Atspi.init()
    deadline = time.time() + 20
    app = None
    while time.time() < deadline and app is None:
        desktop = Atspi.get_desktop(0)
        for i in range(desktop.get_child_count()):
            candidate = desktop.get_child_at_index(i)
            if candidate is not None and candidate.get_process_id() == pid:
                app = candidate
        time.sleep(0.3)
    if app is None:
        print("process did not appear in AT-SPI within 20 s", file=sys.stderr)
        return 3
    time.sleep(1.0)
    out = []
    walk(app, 0, out)
    for depth, role, name, states in out:
        print("%d\t%s\t%s\t%s" % (depth, role, name.replace("\t", " ").replace("\n", " "), states))
    return 0


sys.exit(main())
