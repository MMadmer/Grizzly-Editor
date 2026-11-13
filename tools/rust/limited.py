#!/usr/bin/env python3
"""Runs a program with a time limit and never leaves it behind.

    python tools/rust/limited.py <seconds> <program> [arguments ...]

The program's output and exit code pass through. When the limit runs out the program is ended WITH
EVERYTHING IT STARTED (its whole process tree - an editor a launcher started included), one line
says so on stderr and the exit code is 124. No script of this repository starts a program that
could wait for ever any other way: a check that hangs is worse than a check that fails.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys

TIMED_OUT = 124


def end_tree(pid: int) -> None:
    """Ends `pid` and every process below it. Only that tree: never another editor on the machine."""
    subprocess.run(
        ["taskkill", "/T", "/F", "/PID", str(pid)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    try:
        limit = float(sys.argv[1])
    except ValueError:
        print(f"limited: '{sys.argv[1]}' is not a number of seconds", file=sys.stderr)
        return 2
    command = sys.argv[2:]
    # by PATH, as the calling shell would find it: Windows itself looks in its system folder first,
    # where another program of the same name may live (`bash`)
    if os.path.dirname(command[0]):
        # a path as a shell writes it (forward slashes, relative): Windows wants it whole
        command[0] = os.path.abspath(command[0])
    else:
        command[0] = shutil.which(command[0]) or command[0]
    try:
        child = subprocess.Popen(command)
    except OSError as e:
        print(f"limited: {command[0]} could not be started: {e}", file=sys.stderr)
        return 2
    try:
        return child.wait(timeout=limit)
    except subprocess.TimeoutExpired:
        end_tree(child.pid)
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        print(f"limited: {command[0]} did not end within {limit:g} s and was stopped", file=sys.stderr)
        return TIMED_OUT
    except KeyboardInterrupt:
        end_tree(child.pid)
        raise


if __name__ == "__main__":
    sys.exit(main())
