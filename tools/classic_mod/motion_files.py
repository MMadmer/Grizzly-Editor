#!/usr/bin/env python3
"""A classic mod's motion files against the game's of the same paths.

A classic mod ships its own models with their motion files (`.omf`) under the game's paths, as whole copies made
over the game of its day. Mounted over today's game such a copy is wrong in one of two ways, and this settles
both for any classic mod:

  * **The same skeleton, fewer motions.** Today's game has grown the file since - new weapons and devices put
    their hands motions into the files the hands models already name - and the mod's copy takes those motions
    away: the game stops with a missing motion the moment one is played. The project's file keeps everything it
    has and gets every motion of the game's file it lacks (by name: the mod's own motion wins), its bone tracks
    laid into the file's own track order by bone name.
  * **Another skeleton.** The mod's model has other bones than the game's, so its motion file is another file
    under the old name, and every model that still names that path - the game's own that the mod does not
    replace, a model the game adds later, another module's - loses its motions. The project's file gets a name
    of its own (`<name>_<module id>.omf`) and the project's models are pointed at it. The game's file is the
    game's again.

    python tools/classic_mod/motion_files.py --project <module project> --game <game root> [--dry]

The converter runs it as one of its steps; run by hand it does the same to a project that exists. It only ever
changes files of the project - a motion file's contents or name, the motion references of the project's own
models - and a second run changes nothing. Needs python-lzo and the game's tools\\stalker-tools (the converter's
archive reader).
"""
from __future__ import annotations

import argparse
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import archives  # noqa: E402
import omf  # noqa: E402

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


class Done:
    """What a run changed, by the project's own paths (`gamedata\\meshes\\...`)."""

    def __init__(self):
        self.merged: dict[str, int] = {}     # motion file -> the game's motions it got
        self.renamed: dict[str, str] = {}    # motion file -> its new path
        self.pointed: list[str] = []         # models whose motion references were rewritten
        self.kept: dict[str, str] = {}       # motion file -> why it was left as it is


def process(project: str, game, module: str, dry: bool = False, say=print) -> Done:
    """Settles every motion file of `project` that stands on a path the game has. `game` is the converter's
    archive reader (`archives.Game`)."""
    done = Done()
    root = os.path.join(project, "gamedata")
    meshes = os.path.join(root, "meshes")
    suffix = "_" + module

    def own(path: str) -> str:
        return os.path.relpath(path, project)

    mine = {}
    for folder, _, files in os.walk(meshes):
        for name in files:
            path = os.path.join(folder, name)
            mine[os.path.relpath(path, root).replace("/", "\\").lower()] = path

    # every motion file of the project on a game path: of the game's skeleton, or of another
    foreign = {}
    for rel, path in sorted(mine.items()):
        if not rel.endswith(".omf") or rel[:-4].endswith(suffix):
            continue
        v = game.effective(rel)
        if v is None:
            continue
        with open(path, "rb") as f:
            data = f.read()
        theirs = game.read(v)
        a_bones, b_bones = bones_of(data), bones_of(theirs)
        if a_bones is None or b_bones is None:
            done.kept[own(path)] = "it does not read as a motion file" if a_bones is None else "the game's file does not read"
            say(f"kept\t{rel}\t{done.kept[own(path)]}")
            continue
        if a_bones != b_bones:
            foreign[rel[len("meshes\\"):-4]] = rel
            continue
        try:
            merged, added, skipped = omf.merge(data, theirs)
        except (ValueError, struct.error, IndexError, KeyError, UnicodeDecodeError) as e:
            done.kept[own(path)] = f"it does not read whole ({e})"
            say(f"kept\t{rel}\t{done.kept[own(path)]}")
            continue
        if merged is None:
            if "*" in skipped:
                done.kept[own(path)] = skipped["*"]
                say(f"kept\t{rel}\t{skipped['*']}")
            continue
        done.merged[own(path)] = len(added)
        say(f"merged\t{rel}\t+{len(added)} of the game's motions" + (f", {len(skipped)} left out" if skipped else ""))
        if not dry:
            with open(path, "wb") as f:
                f.write(merged)

    # a file of another skeleton never stands on the game's path, whoever names that path today
    rename = {k: k + suffix for k in foreign}
    for k in sorted(foreign):
        say(f"renamed\tmeshes\\{k}.omf\t-> {os.path.basename(rename[k])}.omf\tanother skeleton than the game's file")
    if rename:
        for rel, path in sorted(mine.items()):
            if not rel.endswith(".ogf"):
                continue
            with open(path, "rb") as f:
                data = f.read()
            new = with_refs(data, rename)
            if new is None:
                continue
            done.pointed.append(own(path))
            say(f"pointed\t{rel}")
            if not dry:
                with open(path, "wb") as f:
                    f.write(new)
        for k, to in rename.items():
            src = mine[foreign[k]]
            dst = os.path.join(os.path.dirname(src), os.path.basename(to) + ".omf")
            done.renamed[own(src)] = own(dst)
            if not dry:
                os.replace(src, dst)
    say(f"{len(done.merged)} motion file(s) got the game's motions, {len(done.renamed)} renamed, "
        f"{len(done.pointed)} model(s) of the project pointed at them" + (" (dry run)" if dry else ""))
    return done


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True, help="the module project")
    ap.add_argument("--game", required=True, help="the game's root")
    ap.add_argument("--dry", action="store_true", help="say what would change and change nothing")
    a = ap.parse_args()
    game = archives.Game(a.game)
    try:
        process(a.project, game, module_id(a.project), a.dry)
    finally:
        game.close()


if __name__ == "__main__":
    main()
