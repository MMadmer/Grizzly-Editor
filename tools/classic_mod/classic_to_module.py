#!/usr/bin/env python3
"""Makes a classic gamedata mod, already copied into a project's gamedata, follow the rules of a module.

A classic mod replaces whole files. A module adds to the game: everything of the game the mod does
not change has to survive it, a game update included. So nothing of the game or the editor bends here,
the mod does:

  * what the game itself writes (the player's options) leaves the module;
  * sources, backups and archives leave it, and so does every file identical to the game's;
  * the .ltx files of the system.ltx tree become one configs\\xms\\<module id>.ltx holding only what the
    mod really changes, section by section, key by key, and a patch\\<module id>_removed.ltxp taking away
    the animation and sound keys of a section the mod rewrote without them: the game's own tree loads whole;
  * the string tables become one file per language, holding the ids the mod adds or changes.

Run it once on a fresh copy (it refuses a project it has already adapted):

    python tools/classic_mod/classic_to_module.py --project <project> --game <game root>

Needs python-lzo and the game's tools\\stalker-tools (LzHuf), and nothing of what it unpacks stays behind.
"""
from __future__ import annotations

import argparse
import fnmatch
import glob
import hashlib
import os
import re
import shutil
import struct
import sys
import tempfile
import zlib

COMPRESSED = 0x80000000
STATE_FILES = {r"configs\axr_options.ltx", r"configs\atmosfear_options.ltx"}
# keys the engine walks as a series or looks a motion up by: one left over from the game plays over the mod's
MOD_OWNED_KEYS = ("anm_", "snd_")
JUNK_EXT = {".psd", ".7z", ".rar", ".zip", ".bak", ".wav", ".skl", ".skls"}
FOLDER_JUNK = re.compile(r"(^|\\)(_backup|[^\\]* — копия)(\\|$)", re.I)


# ---- the game's files ---------------------------------------------------------
def read_tables(database: str, lzhuf) -> dict[str, tuple[str, int, int, int, int]]:
    files: dict[str, tuple[str, int, int, int, int]] = {}
    for path in sorted(glob.glob(os.path.join(database, "*.xdb*"))):
        size = os.path.getsize(path)
        table = None
        with open(path, "rb") as f:
            pos = 0
            while pos + 8 <= size:
                f.seek(pos)
                cid, csize = struct.unpack("<II", f.read(8))
                if cid & ~COMPRESSED == 1:
                    raw = f.read(csize)
                    table = bytes(lzhuf().Decode(raw)) if cid & COMPRESSED else raw
                pos += 8 + csize
        if table is None:
            continue
        pos = 0
        while pos + 2 <= len(table):
            entry = struct.unpack_from("<H", table, pos)[0]
            real, packed, crc = struct.unpack_from("<III", table, pos + 2)
            rel = table[pos + 14:pos + 2 + entry - 4].rstrip(b"\0").decode("cp1251", "replace").replace("/", "\\")
            ptr = struct.unpack_from("<I", table, pos + 2 + entry - 4)[0]
            pos += 2 + entry
            if real:
                files[rel.lower()] = (path, ptr, real, packed, crc)
    return files


def unpack_configs(tables, loose: str, out: str, lzo) -> None:
    handles = {}
    for rel, (path, ptr, real, packed, _crc) in tables.items():
        if not rel.startswith(("configs\\", "scripts\\")):
            continue
        f = handles.setdefault(path, open(path, "rb"))
        f.seek(ptr)
        raw = f.read(packed)
        data = raw if real == packed else lzo.decompress(raw, False, real, algorithm="LZO1X")
        dst = os.path.join(out, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst, "wb") as w:
            w.write(data)
    for f in handles.values():
        f.close()
    for top in ("configs", "scripts"):
        for dp, _, fs in os.walk(os.path.join(loose, top)):
            for name in fs:
                src = os.path.join(dp, name)
                dst = os.path.join(out, os.path.relpath(src, loose).lower())
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                shutil.copyfile(src, dst)


def tree(root: str) -> dict[str, str]:
    out = {}
    for dp, _, fs in os.walk(root):
        for name in fs:
            p = os.path.join(dp, name)
            out[os.path.relpath(p, root).lower()] = p
    return out


# ---- ltx, the way CInifile reads it ----------------------------------------------
def strip_comment(line: str) -> str:
    quoted = False
    for i, c in enumerate(line):
        if c == '"':
            quoted = not quoted
        elif not quoted and (c == ";" or (c == "/" and line[i + 1:i + 2] == "/")):
            return line[:i]
    return line


def clean_value(value: str) -> str:
    """CInifile drops the blanks of a value, outside quotes."""
    out, quoted = [], False
    for c in value:
        if c == '"':
            quoted = not quoted
        if quoted or c not in " \t":
            out.append(c)
    return "".join(out)


class Ltx:
    def __init__(self, view: dict[str, str], fallback: dict[str, dict] | None = None):
        self.view = view
        self.fallback = fallback or {}
        self.sections: dict[str, dict] = {}
        self.files: list[str] = []
        self.problems: list[str] = []

    def parent(self, name: str):
        return self.sections.get(name, self.fallback.get(name))

    def load(self, rel: str, stack=()) -> None:
        path = self.view.get(rel)
        if not path or rel in stack:
            self.problems.append(("include " if not path else "loop ") + rel)
            return
        self.files.append(rel)
        folder = os.path.dirname(rel)
        lines = open(path, "rb").read().decode("cp1251", "surrogateescape").splitlines()
        current = None
        i = 0
        while i < len(lines):
            raw = lines[i].strip()
            i += 1
            m = re.match(r'#include\s*\??\s+"([^"]+)"', raw)
            if m:
                inc = os.path.normpath(os.path.join(folder, m.group(1))).lower()
                if "*" in inc:
                    d, mask = os.path.split(inc)
                    for name in sorted(k for k in self.view if os.path.dirname(k) == d and fnmatch.fnmatch(os.path.basename(k), mask)):
                        self.load(name, stack + (rel,))
                else:
                    self.load(inc, stack + (rel,))
                continue
            line = strip_comment(raw).strip()
            if not line:
                continue
            if line[0] == "[":
                close = line.index("]")
                name = line[1:close].strip().lower()
                current = {}
                rest = line[close + 1:].strip()
                if rest.startswith(":"):
                    for parent in (x.strip().lower() for x in rest[1:].split(",")):
                        if not parent:
                            continue
                        got = self.parent(parent)
                        if got is None:
                            self.problems.append("parent " + parent + " of " + name + " in " + rel)
                        else:
                            current.update(got)
                self.sections[name] = current
                continue
            if current is None:
                continue
            if "=" in line:
                key, value = line.split("=", 1)
                key, value = key.strip(), value.strip()
                while value.count('"') % 2 and i < len(lines):
                    value += "\r\n" + lines[i].strip()
                    i += 1
                current[key] = clean_value(value)
            else:
                current[line] = None


def write_overlay(path: str, sections: dict[str, dict]) -> None:
    out = ["; What this module changes in the game's configs, key by key. Everything else stays the game's."]
    for name in sorted(sections):
        out.append("")
        out.append("[" + name + "]")
        for key in sorted(sections[name]):
            value = sections[name][key]
            out.append(key if value is None else key + " = " + value)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(("\r\n".join(out) + "\r\n").encode("cp1251", "surrogateescape"))


# ---- string tables -------------------------------------------------------------------
STRING = re.compile(rb'<string\s+id\s*=\s*"([^"]+)"\s*>(.*?)</string>', re.S)
TEXT = re.compile(rb"<text>(.*?)</text>", re.S)


def strings_of(path: str) -> dict[bytes, bytes]:
    out = {}
    for sid, body in STRING.findall(open(path, "rb").read()):
        t = TEXT.search(body)
        out[sid] = t.group(1) if t else b""
    return out


# ---- the steps -----------------------------------------------------------------------
def drop_noise(gamedata: str, tables, loose: str) -> list[str]:
    removed = []
    files = tree(gamedata)
    dds = set(files)
    for rel, path in files.items():
        ext = os.path.splitext(rel)[1]
        name_ok = True
        try:
            os.path.relpath(path, gamedata).encode("cp1251")
        except UnicodeEncodeError:
            name_ok = False
        why = None
        if rel in STATE_FILES:
            why = "the game writes it"
        elif os.path.basename(rel) in ("desktop.ini", "thumbs.db") or ext in JUNK_EXT or FOLDER_JUNK.search(rel) or (ext == ".tga" and rel[:-4] + ".dds" in dds) or not name_ok:
            why = "not a game file"
        else:
            size = os.path.getsize(path)
            real = os.path.join(loose, rel)
            if os.path.isfile(real):
                same = os.path.getsize(real) == size and open(real, "rb").read() == open(path, "rb").read()
            else:
                entry = tables.get(rel)
                same = bool(entry) and entry[2] == size and zlib.crc32(open(path, "rb").read()) == entry[4]
            if same:
                why = "identical to the game's"
        if why:
            os.remove(path)
            removed.append(rel)
    return removed


def script_readers(view: dict[str, str]) -> set[str]:
    """Config files scripts open by name: those have to keep existing as files."""
    named: set[str] = set()
    for rel, path in view.items():
        if rel.endswith(".script"):
            text = open(path, "rb").read().decode("cp1251", "replace")
            for m in re.finditer(r'ini_file(?:_ex)?\(\s*"([^"]+)"', text):
                named.add(os.path.normpath("configs\\" + m.group(1).replace("\\\\", "\\")).lower())
    return named


def adapt(project: str, game: str) -> int:
    sys.path.insert(0, os.path.join(game, "tools", "stalker-tools"))
    import lzo
    from LzHuf import LzHuf

    gamedata = os.path.join(project, "gamedata")
    manifest = open(os.path.join(project, "mod.ltx"), encoding="utf-8").read()
    module = re.search(r"(?m)^id\s*=\s*(\S+)", manifest).group(1)
    overlay = os.path.join(gamedata, "configs", "xms", module + ".ltx")
    if os.path.exists(overlay):
        print("already adapted: " + overlay)
        return 1

    work = tempfile.mkdtemp(prefix="classic_mod_")
    try:
        tables = read_tables(os.path.join(game, "database"), LzHuf)
        loose = os.path.join(game, "gamedata")
        print("noise removed:", len(drop_noise(gamedata, tables, loose)))
        unpack_configs(tables, loose, work, lzo)
        base = tree(work)
        mod = {k: v for k, v in tree(gamedata).items() if k.startswith(("configs\\", "scripts\\"))}

        g = Ltx(base)
        g.load("configs\\system.ltx")
        view = dict(base)
        view.update(mod)
        m = Ltx(view, g.sections)
        m.load("configs\\system.ltx")

        readers = script_readers(view)
        shared = sorted(f for f in set(m.files) if f in mod and f in readers)
        for f in shared:
            print("kept as a file too, a script opens it: " + f)

        changed: dict[str, dict] = {}
        for name, keys in m.sections.items():
            have = g.sections.get(name)
            delta = dict(keys) if have is None else {k: v for k, v in keys.items() if k not in have or have[k] != v}
            if delta:
                changed[name] = delta
        write_overlay(overlay, changed)
        # a motion the mod's model does not have stops the game and a sound of the game's plays over the mod's: only those keys go
        dropped = {n: sorted(k for k in g.sections[n] if k not in keys and k.startswith(MOD_OWNED_KEYS)) for n, keys in m.sections.items()
                   if n in g.sections and any(k not in keys and k.startswith(MOD_OWNED_KEYS) for k in g.sections[n])}
        patch = [b"; Animation and sound keys of the game's sections this module rewrites without them."]
        for name in sorted(dropped):
            patch.append(("[" + name + "]").encode("cp1251", "surrogateescape"))
            patch.extend(("!" + k).encode("cp1251", "surrogateescape") for k in dropped[name])
        if dropped:
            os.makedirs(os.path.join(project, "patch"), exist_ok=True)
            with open(os.path.join(project, "patch", module + "_removed.ltxp"), "wb") as f:
                f.write(b"\r\n".join(patch) + b"\r\n")
        print(f"keys taken away: {sum(len(v) for v in dropped.values())} in {len(dropped)} sections")
        print(f"config overlay: {len(changed)} sections, {sum(len(v) for v in changed.values())} keys; "
              f"sections of the game a whole-file copy would have lost: {len([k for k in g.sections if k not in m.sections])}")
        removed = 0
        for f in set(m.files):
            if f in mod and f not in shared:
                os.remove(mod[f])
                removed += 1
        print("whole-file configs replaced by it:", removed)

        for lang_dir in sorted({os.path.dirname(k) for k in mod if k.startswith("configs\\text\\")}):
            lang = os.path.basename(lang_dir)
            ours = {k: v for k, v in mod.items() if os.path.dirname(k) == lang_dir and k.endswith(".xml")}
            overriding = [k for k in ours if k in base]
            if not overriding:
                continue
            stock: dict[bytes, bytes] = {}
            for k in sorted(k for k in base if os.path.dirname(k) == lang_dir and k.endswith(".xml")):
                stock.update(strings_of(base[k]))
            merged: dict[bytes, bytes] = {}
            for k in sorted(ours):
                merged.update(strings_of(ours[k]))
            wanted = {sid: text for sid, text in merged.items() if stock.get(sid) != text}
            target = os.path.join(gamedata, lang_dir, module + "_strings.xml")
            body = [b'<?xml version="1.0" encoding="windows-1251"?>', b"<string_table>"]
            for sid in sorted(wanted):
                body.append(b'\t<string id="' + sid + b'">\r\n\t\t<text>' + wanted[sid] + b"</text>\r\n\t</string>")
            body.append(b"</string_table>\r\n")
            for k in overriding:
                os.remove(ours[k])
            with open(target, "wb") as f:
                f.write(b"\r\n".join(body))
            print(f"strings {lang}: {len(wanted)} ids instead of {len(overriding)} whole files")

        # the result must load: the game's tree whole, with what is left of the mod on top
        after = dict(base)
        after.update({k: v for k, v in tree(gamedata).items() if k.startswith("configs\\")})
        check = Ltx(after, g.sections)
        check.load("configs\\system.ltx")
        extra = [p for p in check.problems if p not in g.problems]
        print("problems left:", extra[:20])
        return 1 if extra else 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True)
    ap.add_argument("--game", required=True)
    args = ap.parse_args()
    return adapt(os.path.abspath(args.project), os.path.abspath(args.game))


if __name__ == "__main__":
    sys.exit(main())
