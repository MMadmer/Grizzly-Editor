"""The author's files of the repository root, safe while a QA tool has them aside.

A tool that takes them aside (the project list, the settings, the C++ editor's layout) keeps them in memory - and
here on disk, under Intermediate/qa/kept, so that a run that dies before it puts them back loses nothing: the next
tool that starts puts them back first (`recover`). A tool run inside another that holds the copies already (a suite
under run_all.py) writes and removes nothing: the outer run's copies are the ones to put back. Importing this module
does nothing.
"""
from __future__ import annotations

import ctypes
import os

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
KEPT_DIR = os.path.join(REPO, "Intermediate", "qa", "kept")
_OWNER = os.path.join(KEPT_DIR, "owner.pid")


def _alive(pid: int) -> bool:
    handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return False
    code = ctypes.c_ulong()
    ok = ctypes.windll.kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
    ctypes.windll.kernel32.CloseHandle(handle)
    return bool(ok) and code.value == 259  # STILL_ACTIVE


def _owner() -> int:
    try:
        return int(open(_OWNER, encoding="ascii").read().strip() or "0")
    except (OSError, ValueError):
        return 0


def held_by_another() -> bool:
    """Whether a run that is alive, and is not this one, holds the copies."""
    owner = _owner()
    return bool(owner) and owner != os.getpid() and _alive(owner)


def recover() -> list[str]:
    """Puts back what a dead run left aside and returns the names; nothing while the run that owns the copies is
    alive."""
    if not os.path.isfile(_OWNER) or held_by_another():
        return []
    back = []
    for entry in sorted(os.listdir(KEPT_DIR)):
        side = os.path.join(KEPT_DIR, entry)
        if entry == "owner.pid" or not os.path.isfile(side):
            continue
        absent = entry.endswith(".absent")
        name = entry[:-len(".absent")] if absent else entry
        target = os.path.join(REPO, name)
        if absent:
            if os.path.isfile(target):
                os.remove(target)
        else:
            with open(side, "rb") as f, open(target, "wb") as g:
                g.write(f.read())
        back.append(name)
        os.remove(side)
    os.remove(_OWNER)
    return back


def to_disk(saved: dict) -> None:
    """Writes a snapshot ({file name in the repository root: bytes, or None for a file that is not there}) before
    anything of it is taken aside. Inside a run that holds copies already, nothing is written."""
    if held_by_another():
        return
    os.makedirs(KEPT_DIR, exist_ok=True)
    for name, data in saved.items():
        side = os.path.join(KEPT_DIR, os.path.basename(name))
        for stale in (side, side + ".absent"):
            if os.path.exists(stale):
                os.remove(stale)
        with open(side if data is not None else side + ".absent", "wb") as f:
            f.write(data or b"")
    with open(_OWNER, "w", encoding="ascii") as f:
        f.write(str(os.getpid()))


def done() -> None:
    """The run put the files back itself: the copies on disk go. Inside a run that holds them, they stay."""
    if not os.path.isdir(KEPT_DIR) or held_by_another():
        return
    for entry in os.listdir(KEPT_DIR):
        side = os.path.join(KEPT_DIR, entry)
        if os.path.isfile(side):
            os.remove(side)
