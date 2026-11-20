"""The converter's ledger of a project (`temp\\classic_import.files`): every file the conversion wrote, with its
size and SHA-256, so that a rerun removes exactly that and nothing a person added or changed.

A step of the conversion run by hand on a project that exists (`motion_files.py`, `media_assets.py`) keeps the
ledger in step through `follow`: what it removed leaves the ledger, what it renamed keeps its row under the new
name, what it rewrote is said as it is now, what it made joins it. A project without a ledger is left alone.
"""
from __future__ import annotations

import hashlib
import os

LEDGER = os.path.join("temp", "classic_import.files")


def _rel(rel: str) -> str:
    return os.path.normpath(rel.replace("/", os.sep))


def follow(project: str, gone=(), renamed=None, rewritten=(), added=()) -> bool:
    """Keeps the ledger of `project` in step with a step run by hand; paths are project-relative, either slash.
    `rewritten` files are said again only when the ledger lists them: a file a person brought stays theirs."""
    path = os.path.join(project, LEDGER)
    if not os.path.isfile(path):
        return False
    rows: dict[str, tuple[str, str, str]] = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) == 3:
                rows[parts[2].lower()] = (parts[0], parts[1], parts[2])

    def said(rel: str) -> tuple[str, str, str]:
        with open(os.path.join(project, rel), "rb") as f:
            data = f.read()
        return hashlib.sha256(data).hexdigest(), str(len(data)), rel

    for rel in gone:
        rows.pop(_rel(rel).lower(), None)
    for was, now in (renamed or {}).items():
        if rows.pop(_rel(was).lower(), None) is not None:
            rows[_rel(now).lower()] = said(_rel(now))
    for rel in rewritten:
        if _rel(rel).lower() in rows:
            rows[_rel(rel).lower()] = said(_rel(rel))
    for rel in added:
        rows[_rel(rel).lower()] = said(_rel(rel))
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        for _, (digest, size, rel) in sorted(rows.items()):
            f.write(f"{digest}\t{size}\t{rel}\n")
    return True
