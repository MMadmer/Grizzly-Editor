#!/usr/bin/env python3
"""Runs a measurement while nothing else measures: one at a time on this machine, each under a time limit.

    python tools/rust/alone.py <seconds> <program> [arguments...]

Several authors measure on one machine; two measurements at once tell nothing about either. This takes a lock
(a folder under the system temp folder, made exclusively), waits for it at most five minutes, runs the program
under tools/rust/limited.py with the given limit, and gives the lock back. A lock older than twenty minutes is a
run that died: it is taken over. The exit code is the program's (124 when the limit ended it, 125 when the lock
could not be had).
"""
from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import time

LOCK = os.path.join(tempfile.gettempdir(), "gz_bench_alone.lock")
WAIT = 300
STALE = 1200


def take() -> bool:
    end = time.monotonic() + WAIT
    while True:
        try:
            os.mkdir(LOCK)
            return True
        except FileExistsError:
            try:
                if time.time() - os.path.getmtime(LOCK) > STALE:
                    os.rmdir(LOCK)
                    continue
            except OSError:
                pass
        if time.monotonic() >= end:
            return False
        time.sleep(2)


def main() -> int:
    if len(sys.argv) < 3 or not sys.argv[1].isdigit():
        print(__doc__)
        return 2
    limited = os.path.join(os.path.dirname(os.path.abspath(__file__)), "limited.py")
    if not take():
        print("alone.py: another measurement held the machine for %d s; nothing was run" % WAIT, flush=True)
        return 125
    try:
        return subprocess.call([sys.executable, limited, sys.argv[1]] + sys.argv[2:])
    finally:
        try:
            os.rmdir(LOCK)
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
