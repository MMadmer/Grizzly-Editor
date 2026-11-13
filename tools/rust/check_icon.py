"""The executables carry the editor's icon and are the programs they should be.

Reads each executable's first icon group (RT_GROUP_ICON, the one Explorer shows) from its resource
section and wants its largest image byte for byte equal to the largest image of
assets/grizzly-editor-icon.ico, and as many images as the icon file holds.

It also reads the PE subsystem: the editor is a window program (2), so Explorer starts it without
a console window that would end it on its close; its twin for a command line (Grizzly-Cmd.exe)
and the MCP bridge, a stdio server, are console programs (3).

    python tools/rust/check_icon.py Bin/x64/Release/Grizzly.exe Bin/x64/Release/Grizzly-Cmd.exe \n        Bin/x64/Release/grizzly-mcp.exe

Exit code 1 when an executable carries no icon or another one, or has the other subsystem.
"""
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ICO = os.path.join(HERE, "..", "..", "assets", "grizzly-editor-icon.ico")
RT_ICON, RT_GROUP_ICON = 3, 14
SUBSYSTEM_WINDOWS, SUBSYSTEM_CONSOLE = 2, 3
SUBSYSTEM_NAMES = {SUBSYSTEM_WINDOWS: "a window program", SUBSYSTEM_CONSOLE: "a console program"}


def wanted_subsystem(path):
    """The bridge speaks over stdio and the twin serves a shell; every other executable here is the editor."""
    name = os.path.basename(path).lower()
    return SUBSYSTEM_CONSOLE if name.startswith(("grizzly-mcp", "grizzly-cmd")) else SUBSYSTEM_WINDOWS


def side(b):
    return 256 if b == 0 else b


def ico_images(data):
    _, kind, count = struct.unpack_from("<HHH", data, 0)
    if kind != 1:
        raise ValueError("not an icon file")
    out = []
    for i in range(count):
        w, _h, _c, _r, _p, _b, size, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16 * i)
        out.append((side(w), data[offset:offset + size]))
    return out


class Resources:
    def __init__(self, exe):
        if exe[:2] != b"MZ":
            raise ValueError("not a Windows executable")
        pe = struct.unpack_from("<I", exe, 0x3C)[0]
        if exe[pe:pe + 4] != b"PE\0\0":
            raise ValueError("no PE header")
        sections, = struct.unpack_from("<H", exe, pe + 6)
        opt_size, = struct.unpack_from("<H", exe, pe + 20)
        opt = pe + 24
        magic, = struct.unpack_from("<H", exe, opt)
        # the same offset in PE32 and PE32+
        self.subsystem, = struct.unpack_from("<H", exe, opt + 68)
        dirs = opt + (112 if magic == 0x20B else 96)
        self.rva, _size = struct.unpack_from("<II", exe, dirs + 2 * 8)
        if not self.rva:
            raise ValueError("no resource section")
        self.sections = []
        for s in range(sections):
            at = opt + opt_size + 40 * s
            vsize, va, raw_size, raw_at = struct.unpack_from("<IIII", exe, at + 8)
            self.sections.append((va, max(vsize, raw_size), raw_at))
        self.exe = exe

    def at(self, rva):
        for va, size, raw in self.sections:
            if va <= rva < va + size:
                return raw + rva - va
        raise ValueError(f"RVA {rva:#x} lies in no section")

    def entries(self, offset):
        """(id, target) of a resource directory at `offset` from the section's root."""
        base = self.at(self.rva) + offset
        named, ids = struct.unpack_from("<HH", self.exe, base + 12)
        return [struct.unpack_from("<II", self.exe, base + 16 + 8 * i) for i in range(named + ids)]

    def find(self, kind, name=None):
        """The bytes of resource `kind` / `name` (the first name when None), first language."""
        types = [t for i, t in self.entries(0) if i == kind]
        if not types:
            return None
        names = [t for i, t in self.entries(types[0] & 0x7FFFFFFF) if name is None or i == name]
        if not names:
            return None
        langs = self.entries(names[0] & 0x7FFFFFFF)
        leaf = self.at(self.rva) + langs[0][1]
        data_rva, size = struct.unpack_from("<II", self.exe, leaf)
        start = self.at(data_rva)
        return self.exe[start:start + size]


def check(path, want_count, want):
    try:
        with open(path, "rb") as f:
            res = Resources(f.read())
    except OSError as e:
        return f"cannot be read: {e.strerror}"
    except ValueError as e:
        return f"carries no icon: {e}"
    want_sub = wanted_subsystem(path)
    if res.subsystem != want_sub:
        have = SUBSYSTEM_NAMES.get(res.subsystem, f"subsystem {res.subsystem}")
        return f"is {have} (subsystem {res.subsystem}), it must be {SUBSYSTEM_NAMES[want_sub]} ({want_sub})"
    group = res.find(RT_GROUP_ICON)
    if group is None:
        return "carries no icon"
    _, kind, count = struct.unpack_from("<HHH", group, 0)
    if kind != 1 or count != want_count:
        return f"carries an icon group of {count} images, the icon file holds {want_count}"
    entries = [struct.unpack_from("<BBBBHHIH", group, 6 + 14 * i) for i in range(count)]
    w, *_rest, icon_id = max(entries, key=lambda e: side(e[0]))
    image = res.find(RT_ICON, icon_id)
    if image != want:
        return f"its largest image ({side(w)} px, {len(image or b'')} bytes) is not the icon file's"
    return None


def main():
    with open(ICO, "rb") as f:
        images = ico_images(f.read())
    width, largest = max(images, key=lambda im: im[0])
    bad = 0
    for path in sys.argv[1:]:
        problem = check(path, len(images), largest)
        name = os.path.basename(path)
        if problem:
            print(f"{name}: {problem}")
            bad = 1
        else:
            sub = wanted_subsystem(path)
            print(f"{name}: {SUBSYSTEM_NAMES[sub]} (subsystem {sub}), the editor's icon "
                  f"({len(images)} images, the {width} px one equal to the .ico's)")
    return bad


if __name__ == "__main__":
    sys.exit(main())
