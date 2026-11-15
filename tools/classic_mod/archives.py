"""The game's files: every archived version of a path, the loose copy over them, and the views built on them."""
from __future__ import annotations

import glob
import os
import struct
import sys
import zlib

COMPRESSED = 0x80000000


def crc_of_file(path: str) -> int:
    crc = 0
    with open(path, "rb") as f:
        while True:
            block = f.read(1 << 22)
            if not block:
                return crc & 0xFFFFFFFF
            crc = zlib.crc32(block, crc)


def walk(root: str) -> dict[str, str]:
    """Relative lower-case path -> absolute path of every file under root."""
    out = {}
    for dp, _, fs in os.walk(root):
        for name in fs:
            p = os.path.join(dp, name)
            out[os.path.relpath(p, root).lower()] = p
    return out


class Version:
    """One copy of a game path: an archive entry or the loose file."""

    __slots__ = ("label", "order", "size", "crc", "archive", "ptr", "packed", "path")

    def __init__(self, label, order, size, crc, archive=None, ptr=0, packed=0, path=None):
        self.label, self.order, self.size, self.crc = label, order, size, crc
        self.archive, self.ptr, self.packed, self.path = archive, ptr, packed, path

    def __repr__(self):
        return f"Version({self.label}, {self.size}, {self.crc:08x})"


class Game:
    """The archives of `database\\*.xdb*` mounted in name order (a later one wins) and loose `gamedata` over all."""

    def __init__(self, root: str):
        self.root = root
        sys.path.insert(0, os.path.join(root, "tools", "stalker-tools"))
        import lzo  # noqa: F401  (python-lzo)
        from LzHuf import LzHuf

        self._lzo = lzo
        self.archives = sorted(glob.glob(os.path.join(root, "database", "*.xdb*")))
        self.versions: dict[str, list[Version]] = {}
        for order, path in enumerate(self.archives):
            label = os.path.basename(path)
            for rel, ptr, real, packed, crc in self._table(path, LzHuf):
                self.versions.setdefault(rel, []).append(Version(label, order, real, crc, path, ptr, packed))
        self.loose_root = os.path.join(root, "gamedata")
        self.loose = walk(self.loose_root) if os.path.isdir(self.loose_root) else {}
        self._loose_crc: dict[str, int] = {}
        for rel, path in self.loose.items():
            self.versions.setdefault(rel, []).append(Version("loose", len(self.archives), os.path.getsize(path), -1, path=path))
        self._handles: dict[str, object] = {}
        self._cache: dict[tuple, bytes] = {}

    @staticmethod
    def _table(path, lzhuf):
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
            return
        pos = 0
        while pos + 2 <= len(table):
            entry = struct.unpack_from("<H", table, pos)[0]
            real, packed, crc = struct.unpack_from("<III", table, pos + 2)
            rel = table[pos + 14:pos + 2 + entry - 4].rstrip(b"\0").decode("cp1251", "replace").replace("/", "\\").lower()
            ptr = struct.unpack_from("<I", table, pos + 2 + entry - 4)[0]
            pos += 2 + entry
            if real:
                yield rel, ptr, real, packed, crc

    def crc(self, v: Version) -> int:
        if v.crc < 0:
            v.crc = crc_of_file(v.path)
        return v.crc

    def read(self, v: Version) -> bytes:
        if v.path:
            with open(v.path, "rb") as f:
                return f.read()
        key = (v.archive, v.ptr)
        hit = self._cache.get(key)
        if hit is not None:
            return hit
        f = self._handles.get(v.archive)
        if f is None:
            f = self._handles[v.archive] = open(v.archive, "rb")
        f.seek(v.ptr)
        raw = f.read(v.packed)
        data = raw if v.size == v.packed else self._lzo.decompress(raw, False, v.size, algorithm="LZO1X")
        if v.size < (1 << 20):
            self._cache[key] = data
        return data

    def effective(self, rel: str) -> Version | None:
        vs = self.versions.get(rel)
        return vs[-1] if vs else None

    def distinct(self, rel: str) -> list[Version]:
        """Every different copy of a path, oldest first (one per content)."""
        seen, out = set(), []
        for v in self.versions.get(rel, []):
            key = (v.size, self.crc(v))
            if key not in seen:
                seen.add(key)
                out.append(v)
        return out

    def same_as(self, rel: str, size: int, crc: int) -> list[Version]:
        return [v for v in self.versions.get(rel, []) if v.size == size and self.crc(v) == crc]

    def close(self):
        for f in self._handles.values():
            f.close()
        self._handles.clear()
        self._cache.clear()


class View:
    """A set of files by game path: the effective game, with some paths replaced by other bytes."""

    def __init__(self, game: Game, prefixes=("configs\\", "scripts\\")):
        self.game = game
        self.over: dict[str, tuple] = {}  # rel -> ("file", path) | ("version", Version)
        self.names = {rel for rel in game.versions if rel.startswith(prefixes)}

    def put_file(self, rel: str, path: str):
        self.over[rel] = ("file", path)
        self.names.add(rel)

    def put_version(self, rel: str, v: Version):
        self.over[rel] = ("version", v)
        self.names.add(rel)

    def exists(self, rel: str) -> bool:
        return rel in self.names

    def read(self, rel: str) -> bytes | None:
        o = self.over.get(rel)
        if o:
            if o[0] == "file":
                with open(o[1], "rb") as f:
                    return f.read()
            return self.game.read(o[1])
        v = self.game.effective(rel)
        return self.game.read(v) if v else None

    def listdir(self, folder: str) -> list[str]:
        folder = folder.rstrip("\\") + "\\"
        return sorted(n for n in self.names if n.startswith(folder) and "\\" not in n[len(folder):])
