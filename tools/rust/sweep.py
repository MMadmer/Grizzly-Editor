#!/usr/bin/env python3
"""Keeps cargo's target folder (Intermediate/cargo) from growing without end.

    python tools/rust/sweep.py            after a cargo run (cg.sh, test.sh and Build.ps1 call it)
    python tools/rust/sweep.py --report   says what it would remove and removes nothing

Cargo never removes an artifact it no longer needs: every change of a crate's features, flags,
dependencies or toolchain writes a new copy under a new hash beside the old one, and the
incremental caches pile up the same way. Left alone the debug folder reached 400 GB.

A unit is the files of one compilation (`<name>-<hash>.*` in `deps`, a folder in `build`,
`incremental` and `.fingerprint`). Units of the same target - one crate's library, one test or
program by its root source, one build script, its output - are siblings; only one of them is the
one the next build uses, and the others are what cargo left behind. So:

- a sibling older than the newest one by more than KEEP_DAYS goes;
- when a profile folder is still larger than its cap (GRIZZLY_TARGET_CAP_GB, 60 by default),
  every sibling but the newest goes, and then the incremental caches, newest kept last.

What goes by mistake is only rebuilt. The sweep takes cargo's own lock of the profile folder
first, so it never meets a build of another author half way: a folder whose lock is held is left
for the next sweep.
"""
from __future__ import annotations

import msvcrt
import os
import re
import shutil
import sys
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2] / "Intermediate" / "cargo"
KEEP_DAYS = 2
CAP = float(os.environ.get("GRIZZLY_TARGET_CAP_GB", "60")) * (1 << 30)
# deps, build and .fingerprint name a unit by 16 hex digits, incremental a crate by 13 base-32 ones
UNIT = re.compile(r"^(?:lib)?(.+)-([0-9a-f]{16}|[0-9a-v]{13})(\..+)?$")


def size_of(p: Path) -> int:
    if p.is_file():
        return p.stat().st_size
    total = 0
    for dirpath, _, files in os.walk(p):
        for f in files:
            try:
                total += os.stat(os.path.join(dirpath, f)).st_size
            except OSError:
                pass
    return total


def newest(paths: list[Path]) -> float:
    t = 0.0
    for p in paths:
        try:
            t = max(t, p.stat().st_mtime)
        except OSError:
            pass
    return t


def root_source(dfile: Path) -> str:
    # the first prerequisite of the dep-info file is the target's root source (src/lib.rs, src/main.rs, tests/x.rs)
    try:
        with dfile.open(encoding="utf-8", errors="replace") as f:
            line = f.readline()
    except OSError:
        return ""
    _, _, rest = line.partition(": ")
    first = rest.split(" ")[0].strip()
    return first.replace("\\", "/")


def units(profile: Path) -> dict[tuple, list[tuple[float, list[Path]]]]:
    """Sibling key -> [(time, files of one unit)]."""
    groups: dict[tuple, list[tuple[float, list[Path]]]] = defaultdict(list)

    deps = profile / "deps"
    if deps.is_dir():
        by_unit: dict[tuple[str, str], list[Path]] = defaultdict(list)
        for f in deps.iterdir():
            m = UNIT.match(f.name)
            if m:
                by_unit[(m.group(1), m.group(2))].append(f)
        for (name, h), files in by_unit.items():
            exts = {f.suffix for f in files}
            if ".exe" in exts:
                d = next((f for f in files if f.suffix == ".d"), None)
                kind = ("exe", root_source(d) if d else "")
            elif ".dll" in exts:
                kind = ("dll",)
            elif ".rlib" in exts or ".rmeta" in exts:
                kind = ("lib",)
            else:
                kind = ("other", "".join(sorted(exts)))
            groups[("deps", name) + kind].append((newest(files), files))

    for sub in ("build", "incremental", ".fingerprint"):
        d = profile / sub
        if not d.is_dir():
            continue
        for u in d.iterdir():
            m = UNIT.match(u.name)
            if not m or not u.is_dir():
                continue
            kind: tuple = ()
            if sub == "build":
                kind = ("output",) if (u / "output").exists() else ("script",)
            elif sub == ".fingerprint":
                # one folder per unit, named like its deps files: it follows them
                continue
            groups[(sub, m.group(1)) + kind].append((newest([u, *u.iterdir()]), [u]))
    return groups


def remove(paths: list[Path], report: bool) -> int:
    freed = 0
    for p in paths:
        freed += size_of(p)
        if report:
            continue
        try:
            if p.is_dir():
                shutil.rmtree(p, ignore_errors=True)
            else:
                p.unlink()
        except OSError:
            pass
    return freed


def fingerprints_of(profile: Path, gone: set[str]) -> list[Path]:
    fp = profile / ".fingerprint"
    if not fp.is_dir() or not gone:
        return []
    return [u for u in fp.iterdir() if UNIT.match(u.name) and UNIT.match(u.name).group(2) in gone]


def sweep(profile: Path, report: bool) -> int:
    groups = units(profile)
    now = time.time()
    freed = 0
    gone: set[str] = set()

    def drop(entry: tuple[float, list[Path]]) -> None:
        nonlocal freed
        freed += remove(entry[1], report)
        for f in entry[1]:
            m = UNIT.match(f.name)
            if m:
                gone.add(m.group(2))

    kept: dict[tuple, list[tuple[float, list[Path]]]] = {}
    for key, entries in groups.items():
        entries.sort(key=lambda e: e[0], reverse=True)
        top = entries[0][0]
        keep = [entries[0]]
        for e in entries[1:]:
            if top - e[0] > KEEP_DAYS * 86400:
                drop(e)
            else:
                keep.append(e)
        kept[key] = keep

    # the size before this sweep, less what it took: the same in a report and in a real run
    before = size_of(profile) + (0 if report else freed)
    if before - freed > CAP:
        for keep in kept.values():
            for e in keep[1:]:
                drop(e)
            del keep[1:]
        inc = sorted((e for k, v in kept.items() if k[0] == "incremental" for e in v), key=lambda e: e[0])
        for e in inc:
            if before - freed <= CAP * 0.75:
                break
            drop(e)

    freed += remove(fingerprints_of(profile, gone), report)
    return freed


def locked(profile: Path):
    """Cargo's lock of the profile folder, taken without waiting; None when a build holds it."""
    for name in (".cargo-build-lock", ".cargo-lock"):
        p = profile / name
        if not p.exists():
            continue
        try:
            f = open(p, "r+b")
        except OSError:
            return None
        try:
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
        except OSError:
            f.close()
            return None
        return f
    return open(os.devnull, "rb")


def main() -> int:
    report = "--report" in sys.argv[1:]
    if not ROOT.is_dir():
        return 0
    total = 0
    for profile in sorted(p for p in ROOT.iterdir() if p.is_dir() and (p / "deps").is_dir()):
        lock = locked(profile)
        if lock is None:
            print(f"sweep: {profile.name} is being built, left for the next sweep")
            continue
        try:
            total += sweep(profile, report)
        finally:
            lock.close()
    if total >= 1 << 30 or report:
        verb = "would free" if report else "freed"
        print(f"sweep: {verb} {total / (1 << 30):.1f} GB of stale build artifacts")
    return 0


if __name__ == "__main__":
    sys.exit(main())
