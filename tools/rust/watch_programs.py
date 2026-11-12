"""The run-time net of `tools/rust/test.sh`: no test writes a program file.

test.sh runs it twice beside the test binaries. Once over a temp folder of their own (TEMP / TMP):

    python tools/rust/watch_programs.py <folder> <report> [--ready <file>] --remove

and once over the repository's own tree, which holds programs and scripts of its own and where cargo
writes its programs (`Intermediate\cargo`, left out; cargo does not build while the binaries run):

    python tools/rust/watch_programs.py <repo> <report> --ready <file> --marks Intermediate/test-logs
        --exclude Intermediate/cargo --since-start

It reads the folder's change notifications (ReadDirectoryChangesW through ctypes, the whole tree, a
1 MB buffer), so a file is seen even when the test deletes it a millisecond later: every file
created, renamed into place or written there is a finding when its name ends in a program's extension
(.exe .dll .com .scr .bat .cmd .ps1 .msi) or its first two bytes are `MZ` (read where the file still
exists: opened with every sharing flag, two bytes, closed at once: a test's own delete is never
blocked). With `--since-start` only a file made after the watch began counts (its creation time), so
the tree's own `Build.ps1` or a release build edited meanwhile is no finding. A buffer the system
could not fill in time (it says so with an empty answer) makes it walk the folder once instead (the
excluded paths left out), and the report says so.

What it cannot see: a file whose NAME is harmless and that is gone before it can be opened - its `MZ`
head is never read (a program's name is seen however short the file lives); in a `--since-start` tree,
a file older than the watch that a test rewrites in place or renames to a program's name; anything
under an excluded path; and anything written outside the two watched trees.
`tools/rust/check_test_files.py` reads the sources for all of them.

The report has one line per finding, `<unix time>|<path relative to the folder>|<why>`, then
`events <n>`, `overflows <n>` and `cpu <seconds>` (what the watcher cost). It stops when a file
named `.stop` appears in the marks folder (`--marks`, the folder's top by default) - the mark that
proves the watch armed (`.watch_armed`) is written there too and removed at the end; with `--remove`
it then removes the folder - a link inside is unlinked, never followed - and writes the report last.
"""
from __future__ import annotations

import argparse
import ctypes
import os
import struct
import sys
import threading
import time
from ctypes import wintypes

EXTENSIONS = (".exe", ".dll", ".com", ".scr", ".bat", ".cmd", ".ps1", ".msi")
STOP = ".stop"
READY = ".watch_armed"

FILE_LIST_DIRECTORY = 0x0001
GENERIC_READ = 0x80000000
SHARE_ALL = 0x1 | 0x2 | 0x4
OPEN_EXISTING = 3
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
NOTIFY = 0x1 | 0x2 | 0x8 | 0x10  # file name, dir name, size, last write
ACTION_ADDED, ACTION_MODIFIED, ACTION_RENAMED_NEW = 1, 3, 5
INVALID = wintypes.HANDLE(-1).value

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.CreateFileW.restype = wintypes.HANDLE
k32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD,
                            wintypes.DWORD, wintypes.HANDLE]
k32.ReadDirectoryChangesW.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD, wintypes.BOOL,
                                      wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID,
                                      wintypes.LPVOID]
k32.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
                         wintypes.LPVOID]
k32.CloseHandle.argtypes = [wintypes.HANDLE]


def head(path: str) -> bytes:
    """The first two bytes of a file, without keeping anyone from deleting or renaming it."""
    h = k32.CreateFileW("\\\\?\\" + path, GENERIC_READ, SHARE_ALL, None, OPEN_EXISTING,
                        FILE_FLAG_OPEN_REPARSE_POINT, None)
    if h == INVALID or h is None:
        return b""
    try:
        buf = ctypes.create_string_buffer(2)
        got = wintypes.DWORD(0)
        if not k32.ReadFile(h, buf, 2, ctypes.byref(got), None):
            return b""
        return buf.raw[:got.value]
    finally:
        k32.CloseHandle(h)


def why(top: str, rel: str, since: float | None = None) -> str | None:
    path = os.path.join(top, rel)
    if os.path.isdir(path):
        return None
    if since is not None:
        # a tree with programs of its own: only what was made after the watch began
        try:
            if os.stat(path).st_ctime < since:
                return None
        except OSError:
            # gone already: its name alone can still say what it was
            pass
    if rel.lower().endswith(EXTENSIONS):
        return "a program's name"
    try:
        if os.path.getsize(path) < 2:
            return None
    except OSError:
        return None
    return "starts with MZ" if head(path) == b"MZ" else None


def excluded(rel: str, excludes: list[str]) -> bool:
    low = rel.lower()
    return any(low == e or low.startswith(e + "\\") for e in excludes)


def sweep(top: str, excludes: list[str], since: float | None) -> list[tuple[str, str]]:
    """Every program file the folder holds now (what an overflowed buffer could not name)."""
    out = []
    for d, dirs, files in os.walk(top):
        dirs[:] = [n for n in dirs if not os.path.islink(os.path.join(d, n))
                   and not getattr(os.lstat(os.path.join(d, n)), "st_reparse_tag", 0)
                   and not excluded(os.path.relpath(os.path.join(d, n), top), excludes)]
        for n in files:
            rel = os.path.relpath(os.path.join(d, n), top)
            w = why(top, rel, since)
            if w:
                out.append((rel, w))
    return out


def remove(top: str) -> None:
    """The folder removed; a junction or a link inside goes as a link, never followed."""
    for d, dirs, files in os.walk(top, topdown=True):
        keep = []
        for n in dirs:
            p = os.path.join(d, n)
            if os.path.islink(p) or getattr(os.lstat(p), "st_reparse_tag", 0):
                try:
                    os.rmdir(p)
                except OSError:
                    try:
                        os.unlink(p)
                    except OSError:
                        pass
            else:
                keep.append(n)
        dirs[:] = keep
    for d, dirs, files in os.walk(top, topdown=False):
        for n in files:
            p = os.path.join(d, n)
            try:
                os.chmod(p, 0o666)
                os.unlink(p)
            except OSError:
                pass
        for n in dirs:
            try:
                os.rmdir(os.path.join(d, n))
            except OSError:
                pass
    try:
        os.rmdir(top)
    except OSError:
        pass


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("folder")
    ap.add_argument("report")
    ap.add_argument("--ready")
    ap.add_argument("--remove", action="store_true")
    ap.add_argument("--marks", default="", help="the folder of the .stop and armed marks, relative to the watched one")
    ap.add_argument("--exclude", action="append", default=[], help="a path relative to the folder that is not judged")
    ap.add_argument("--since-start", action="store_true", help="only files made after the watch began count")
    args = ap.parse_args()
    top = os.path.abspath(args.folder)
    since = time.time() - 1.0 if args.since_start else None
    excludes = [os.path.normpath(e).lower().rstrip("\\") for e in args.exclude]
    marks = os.path.normpath(args.marks) if args.marks else ""
    stop_rel = os.path.join(marks, STOP) if marks else STOP
    ready_rel = os.path.join(marks, READY) if marks else READY
    h = k32.CreateFileW(top, FILE_LIST_DIRECTORY, SHARE_ALL, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)
    if h == INVALID or h is None:
        print("the folder cannot be watched: %s (error %d)" % (top, ctypes.get_last_error()), file=sys.stderr)
        return 2
    size = 1 << 20
    buf = ctypes.create_string_buffer(size)
    got = wintypes.DWORD(0)
    found: list[tuple[float, str, str]] = []
    seen: set[str] = set()
    events = overflows = 0

    def note(rel: str, w: str) -> None:
        if rel not in seen:
            seen.add(rel)
            found.append((time.time(), rel, w))

    # the watch is armed by the first call: a mark written inside the folder until its event arrives proves it,
    # and only then the ready file test.sh waits for is written
    armed = threading.Event()

    def arm() -> None:
        while not armed.is_set():
            with open(os.path.join(top, ready_rel), "w") as f:
                f.write("armed\n")
            armed.wait(0.05)
    threading.Thread(target=arm, daemon=True).start()
    stop = False
    while not stop:
        ok = k32.ReadDirectoryChangesW(h, buf, size, True, NOTIFY, ctypes.byref(got), None, None)
        if not ok:
            print("ReadDirectoryChangesW failed: error %d" % ctypes.get_last_error(), file=sys.stderr)
            break
        if got.value == 0:
            overflows += 1
            for rel, w in sweep(top, excludes, since):
                note(rel, w + " (found by a walk after an overflow)")
            stop = os.path.exists(os.path.join(top, stop_rel))
            continue
        at = 0
        while True:
            nxt, action, length = struct.unpack_from("<III", buf.raw, at)
            rel = buf.raw[at + 12:at + 12 + length].decode("utf-16-le")
            events += 1
            if rel == stop_rel:
                stop = True
            elif rel == ready_rel:
                if not armed.is_set():
                    armed.set()
                    if args.ready:
                        with open(args.ready, "w") as f:
                            f.write("ready\n")
            elif action in (ACTION_ADDED, ACTION_MODIFIED, ACTION_RENAMED_NEW) and not excluded(rel, excludes):
                w = why(top, rel, since)
                if w:
                    note(rel, w)
            if not nxt:
                break
            at += nxt
    k32.CloseHandle(h)
    cpu = sum(os.times()[:2])
    for mark in (ready_rel, stop_rel):
        try:
            os.unlink(os.path.join(top, mark))
        except OSError:
            pass
    if args.remove:
        remove(top)
    # the report last: whoever waits for it finds the folder gone already
    with open(args.report + ".part", "w", encoding="utf-8") as f:
        for t, rel, w in found:
            f.write("%.3f|%s|%s\n" % (t, rel, w))
        f.write("events %d\noverflows %d\ncpu %.2f\n" % (events, overflows, cpu))
    os.replace(args.report + ".part", args.report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
