"""The mod's binary files: which are its own work, which are the game's bytes (today's or an older version's), junk."""
from __future__ import annotations

import os
import struct

JUNK_EXT = {".psd", ".7z", ".rar", ".zip", ".bak", ".wav", ".skl", ".skls"}
JUNK_NAMES = {"desktop.ini", "thumbs.db"}


def junk(rel: str, mod_names: set) -> str | None:
    base = os.path.basename(rel)
    ext = os.path.splitext(rel)[1]
    if base in JUNK_NAMES:
        return "Windows folder settings"
    if ext in JUNK_EXT:
        return {".psd": "a Photoshop source", ".bak": "a backup", ".wav": "an uncompressed sound source",
                ".skl": "an SDK motion source", ".skls": "an SDK motion source"}.get(ext, "an archive")
    if ext == ".tga" and rel[:-4] + ".dds" in mod_names:
        return "the source of a .dds beside it"
    if ext == ".tga" and rel.startswith("textures\\"):
        return "a picture source: the game reads only .dds there"
    if ext == ".txt" and rel.startswith("textures\\"):
        return "an author's note"
    try:
        rel.encode("cp1251")
    except UnicodeEncodeError:
        return "a name the game cannot open (characters its code page lacks)"
    return None


def _chunks(data: bytes, pos=0, end=None):
    end = len(data) if end is None else end
    while pos + 8 <= end:
        cid, size = struct.unpack_from("<II", data, pos)
        if size > end - pos - 8:
            return
        yield cid & 0x7FFFFFFF, data[pos + 8:pos + 8 + size]
        pos += 8 + size


def _z(b: bytes, p=0):
    e = b.find(b"\0", p)
    e = len(b) if e < 0 else e
    return b[p:e].decode("cp1251", "replace").lower().replace("/", "\\"), e + 1


def ogf_refs(data: bytes, depth=0):
    """Textures, motion files and models an .ogf names (OGF_TEXTURE, OGF_S_MOTION_REFS*, OGF_S_LODS, children)."""
    tex, mot, mdl = set(), set(), set()
    if depth > 8:
        return tex, mot, mdl
    for cid, body in _chunks(data):
        if cid == 2:
            t, _ = _z(body)
            tex.update(x.strip() for x in t.split(",") if x.strip())
        elif cid == 9:
            for _, child in _chunks(body):
                a, b, c = ogf_refs(child, depth + 1)
                tex |= a
                mot |= b
                mdl |= c
        elif cid == 19:
            t, _ = _z(body)
            mot.update(x.strip() for x in t.split(",") if x.strip())
        elif cid == 24 and len(body) >= 4:
            n, p = struct.unpack_from("<I", body)[0], 4
            for _ in range(min(n, 4096)):
                t, p = _z(body, p)
                if t:
                    mot.add(t)
        elif cid == 23:
            t, _ = _z(body)
            if t:
                mdl.add(t)
    return tex, mot, mdl


def texture_files(name: str) -> list[str]:
    n = name.lower()
    return [f"textures\\{n}.dds", f"textures\\{n}.thm", f"textures\\{n}_bump.dds", f"textures\\{n}_bump#.dds"]


def companions(rel: str) -> list[str]:
    """Files that go with a texture: its description and its bump pair."""
    if not rel.startswith("textures\\"):
        return []
    stem, ext = os.path.splitext(rel)
    if ext not in (".dds", ".thm"):
        return []
    if stem.endswith("_bump#") or stem.endswith("_bump"):
        return []
    return [stem + ".dds", stem + ".thm", stem + "_bump.dds", stem + "_bump#.dds"]


def value_paths(value: str) -> list[str]:
    """Game paths a config value may name (a model, a sound, a texture, an animation)."""
    out = []
    for tok in value.replace(";", ",").split(","):
        t = tok.strip().strip('"').lower().replace("/", "\\")
        if not t or " " in t or len(t) > 200:
            continue
        out += [f"meshes\\{t}.ogf", f"meshes\\{t}.omf", f"sounds\\{t}.ogg", f"anims\\{t}", f"anims\\{t}.anm",
                f"anims\\{t}.ppe"] + texture_files(t)
        if t.endswith((".ogf", ".omf", ".ogg", ".anm", ".ppe", ".dds")):
            top = {"ogf": "meshes", "omf": "meshes", "ogg": "sounds", "anm": "anims", "ppe": "anims", "dds": "textures"}[t[-3:]]
            out.append(f"{top}\\{t}")
    return out
