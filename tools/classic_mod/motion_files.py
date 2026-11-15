#!/usr/bin/env python3
"""A motion file of another skeleton never replaces the game's.

A classic mod ships its own models with their motion files (`.omf`) under the game's paths. Where the mod's
model has another skeleton than the game's, its motion file is another file under the old name - and mounted
over the game's it takes the motions away from every model of TODAY's game that still names that path and
that the mod does not replace (a medkit the game added after the mod was made, for one): the first scene
that plays one of those motions stops the game.

For every motion file of the project that stands on a path the game has, with another set of bones, and that
some model of the game the project does not replace still names, this gives the project's file a name of its
own (`<name>_<module id>.omf`) and points the project's own models at it. The game's file is the game's again.

    python tools/classic_mod/motion_files.py --project <module project> --game <game root> [--dry]

It only ever renames a file of the project and rewrites the motion references of the project's models; a second
run changes nothing. Needs python-lzo and the game's tools\\stalker-tools (the converter's archive reader).
"""
from __future__ import annotations

import argparse
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import archives  # noqa: E402

MOTIONS = 15       # the motion definitions of a motion file, with the bones by name
REFS = 19          # a model's motion files: one string, names joined by commas
REFS_LIST = 24     # the same as a counted list of strings


def chunks(data: bytes):
    pos, out = 0, []
    while pos + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, pos)
        if size > len(data) - pos - 8:
            break
        out.append((cid, data[pos + 8:pos + 8 + size]))
        pos += 8 + size
    return out


def cstr(b: bytes, off: int):
    e = b.index(b"\0", off)
    return b[off:e], e + 1


def bones_of(data: bytes) -> frozenset | None:
    """The bones a motion file is written for, by name; None when the file does not read."""
    try:
        for cid, body in chunks(data):
            if cid & 0x7FFFFFFF != MOTIONS:
                continue
            _, parts = struct.unpack_from("<HH", body, 0)
            off, names = 4, set()
            for _ in range(parts):
                _, off = cstr(body, off)
                count = struct.unpack_from("<H", body, off)[0]
                off += 2
                for _ in range(count):
                    name, off = cstr(body, off)
                    off += 4
                    names.add(name.decode("cp1251", "replace").lower())
            return frozenset(names)
    except (ValueError, struct.error):
        return None
    return None


def key(name: str) -> str:
    n = name.strip().replace("/", "\\").lower()
    return n[:-4] if n.endswith(".omf") else n


def refs_of(data: bytes) -> list[str]:
    """The motion files a model names, as written."""
    out = []
    for cid, body in chunks(data):
        if cid == REFS:
            text, _ = cstr(body + b"\0", 0)
            out += [x.strip() for x in text.decode("cp1251", "replace").split(",") if x.strip()]
        elif cid == REFS_LIST and len(body) >= 4:
            count, p = struct.unpack_from("<I", body)[0], 4
            for _ in range(min(count, 4096)):
                text, p = cstr(body + b"\0", p)
                if text:
                    out.append(text.decode("cp1251", "replace"))
    return out


def with_refs(data: bytes, rename: dict) -> bytes | None:
    """The model with its motion references renamed (`rename`: old key -> new name); None when none changes."""
    out, changed = [], False
    for cid, body in chunks(data):
        if cid == REFS:
            text, _ = cstr(body + b"\0", 0)
            names = [x.strip() for x in text.decode("cp1251", "replace").split(",") if x.strip()]
            new = [rename.get(key(n), n) for n in names]
            if new != names:
                changed = True
                body = ",".join(new).encode("cp1251") + b"\0"
        elif cid == REFS_LIST and len(body) >= 4:
            count, p = struct.unpack_from("<I", body)[0], 4
            names = []
            for _ in range(min(count, 4096)):
                text, p = cstr(body + b"\0", p)
                names.append(text.decode("cp1251", "replace"))
            new = [rename.get(key(n), n) for n in names]
            if new != names:
                changed = True
                body = struct.pack("<I", len(new)) + b"".join(n.encode("cp1251") + b"\0" for n in new)
        out.append(struct.pack("<II", cid, len(body)) + body)
    return b"".join(out) if changed else None


def module_id(project: str) -> str:
    with open(os.path.join(project, "mod.ltx"), encoding="cp1251", errors="replace") as f:
        for line in f:
            left, _, right = line.partition("=")
            if left.strip() == "id":
                return right.split(";")[0].strip()
    raise SystemExit("mod.ltx names no id")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True, help="the module project")
    ap.add_argument("--game", required=True, help="the game's root")
    ap.add_argument("--dry", action="store_true", help="say what would change and change nothing")
    a = ap.parse_args()
    game = archives.Game(a.game)
    root = os.path.join(a.project, "gamedata")
    meshes = os.path.join(root, "meshes")
    suffix = "_" + module_id(a.project)

    mine = {}
    for folder, _, files in os.walk(meshes):
        for name in files:
            path = os.path.join(folder, name)
            mine[os.path.relpath(path, root).replace("/", "\\").lower()] = path

    # the project's motion files that stand on a game path with another set of bones
    foreign = {}
    for rel, path in mine.items():
        if not rel.endswith(".omf") or rel[:-4].endswith(suffix):
            continue
        v = game.effective(rel)
        if v is None:
            continue
        with open(path, "rb") as f:
            a_bones = bones_of(f.read())
        b_bones = bones_of(game.read(v))
        if a_bones is not None and b_bones is not None and a_bones != b_bones:
            foreign[rel[len("meshes\\"):-4]] = rel
    if not foreign:
        print("no motion file of another skeleton stands on a game path")
        return

    # the game's models the project does not replace that still name one of them
    users = {k: [] for k in foreign}
    for rel in sorted(game.versions):
        if not (rel.startswith("meshes\\") and rel.endswith(".ogf")) or rel in mine:
            continue
        v = game.effective(rel)
        for name in refs_of(game.read(v)):
            if key(name) in users:
                users[key(name)].append(rel)
    rename = {k: k + suffix for k, models in users.items() if models}
    for k in sorted(foreign):
        if k in rename:
            print(f"renamed\tmeshes\\{k}.omf\t-> {os.path.basename(rename[k])}.omf\t{len(users[k])} model(s) of the game keep the game's file, "
                  f"first: {users[k][0]}")
        else:
            print(f"kept\tmeshes\\{k}.omf\tno model of the game outside the project names it")
    if not rename:
        return

    pointed = 0
    for rel, path in sorted(mine.items()):
        if not rel.endswith(".ogf"):
            continue
        with open(path, "rb") as f:
            data = f.read()
        new = with_refs(data, rename)
        if new is None:
            continue
        pointed += 1
        print(f"pointed\t{rel}")
        if not a.dry:
            with open(path, "wb") as f:
                f.write(new)
    if not a.dry:
        for k, to in rename.items():
            src = mine[foreign[k]]
            os.replace(src, os.path.join(os.path.dirname(src), os.path.basename(to) + ".omf"))
    print(f"{len(rename)} motion file(s) renamed, {pointed} model(s) of the project pointed at them" + (" (dry run)" if a.dry else ""))


main()
