"""Motion files (`.omf`): reading, and adding the motions one file has to another of the same skeleton.

A motion file is two chunks. The parameters (15): a version; the partitions, each a name and its bones (name,
track index); the motion definitions, each a name, flags, the partition of a cycle or the bone of an FX, a
motion number, four numbers, and from version 4 the marks. The motions (14): a count, then the i-th motion - a
name, the frame count and one track per bone in track order (a flags byte; the rotation: 8 bytes when it has no
keys, else a checksum and 8 bytes a frame; the translation: a checksum and 6 or 3 bytes a frame with 24 more
when it has keys, else 12 bytes).

`merge(mine, theirs)` adds to `mine` every motion of `theirs` that `mine` lacks by name, its tracks laid into
mine's track order by bone name: a mod's copy of a file keeps what today's game has in the same file, the mod's
own motion winning where both have one name."""
import struct

FL_T_PRESENT, FL_R_ABSENT, FL_T16 = 1, 2, 4
ESM_FX = 1
MAX_MOTIONS = 0x3FFF


def chunks(data):
    off, out = 0, []
    while off + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, off)
        if size > len(data) - off - 8:
            raise ValueError("a chunk runs past the end of the file")
        out.append((cid, data[off + 8:off + 8 + size]))
        off += 8 + size
    return out


def cstr(b, off):
    e = b.index(b"\0", off)
    return b[off:e], e + 1


def line(b, off):
    e = b.find(b"\n", off)
    if e < 0:
        return b[off:], len(b)
    s = b[off:e]
    return (s[:-1] if s.endswith(b"\r") else s), e + 1


class Omf:
    def __init__(self, data):
        self.order = []
        self.other = {}
        self.version = None
        self.parts = []      # [(name bytes, [(bone name bytes, track index)])]
        self.defs = []       # dicts: name, flags, part, motion, params (16 bytes), marks (raw bytes, v4)
        self.motions = []    # subchunk bodies
        for cid, body in chunks(data):
            self.order.append(cid)
            c = cid & 0x7FFFFFFF
            if c == 15:
                self._smparams(body)
            elif c == 14:
                self._motions(body)
            else:
                self.other[cid] = body
        if self.version is None:
            raise ValueError("no motion parameters")

    def _smparams(self, b):
        self.version, nparts = struct.unpack_from("<HH", b, 0)
        off = 4
        for _ in range(nparts):
            pname, off = cstr(b, off)
            nb = struct.unpack_from("<H", b, off)[0]
            off += 2
            bones = []
            for _ in range(nb):
                bn, off = cstr(b, off)
                idx = struct.unpack_from("<I", b, off)[0]
                off += 4
                bones.append((bn, idx))
            self.parts.append((pname, bones))
        nmot = struct.unpack_from("<H", b, off)[0]
        off += 2
        for _ in range(nmot):
            name, off = cstr(b, off)
            flags, part, motion = struct.unpack_from("<IHH", b, off)
            off += 8
            params = b[off:off + 16]
            off += 16
            marks = b""
            if self.version >= 4:
                m0 = off
                cnt = struct.unpack_from("<I", b, off)[0]
                off += 4
                for _ in range(cnt):
                    _, off = line(b, off)
                    n = struct.unpack_from("<I", b, off)[0]
                    off += 4 + 8 * n
                marks = b[m0:off]
            self.defs.append({"name": name, "flags": flags, "part": part, "motion": motion, "params": params, "marks": marks})
        if off != len(b):
            raise ValueError(f"motion parameters: {len(b) - off} bytes left over")

    def _motions(self, b):
        count, byid = None, {}
        for cid, body in chunks(b):
            if cid == 0:
                count = struct.unpack_from("<I", body, 0)[0]
            else:
                byid[cid - 1] = body
        if count is None or any(i not in byid for i in range(count)):
            raise ValueError("the motions are not all there")
        self.motions = [byid[i] for i in range(count)]

    # ---- views
    def names(self):
        return [d["name"].decode("cp1251").lower() for d in self.defs]

    def bone_track(self):
        """bone name (lower) -> track index"""
        return {bn.decode("cp1251").lower(): idx for _, bones in self.parts for bn, idx in bones}

    def track_bone(self):
        return {idx: bn.decode("cp1251").lower() for _, bones in self.parts for bn, idx in bones}

    def part_index(self):
        return {p.decode("cp1251").lower(): i for i, (p, _) in enumerate(self.parts)}

    def split_tracks(self, i):
        """(name, frames, [track bytes]) of motion i; None when the tracks do not read whole."""
        body = self.motions[i]
        name, off = cstr(body, 0)
        frames = struct.unpack_from("<I", body, off)[0]
        off += 4
        n = len(self.bone_track())
        tracks = []
        for _ in range(n):
            if off >= len(body):
                return None  # an old file that leaves trailing tracks out
            fl = body[off]
            size = 1
            size += 8 if fl & FL_R_ABSENT else 4 + frames * 8
            if fl & FL_T_PRESENT:
                size += 4 + frames * (6 if fl & FL_T16 else 3) + 24
            else:
                size += 12
            if off + size > len(body):
                return None
            tracks.append(body[off:off + size])
            off += size
        if off != len(body):
            return None
        return name, frames, tracks

    # ---- writing
    def build(self):
        mot = bytearray(struct.pack("<III", 0, 4, len(self.motions)))
        for i, body in enumerate(self.motions):
            mot += struct.pack("<II", i + 1, len(body)) + body
        sm = bytearray(struct.pack("<HH", self.version, len(self.parts)))
        for pname, bones in self.parts:
            sm += pname + b"\0" + struct.pack("<H", len(bones))
            for bn, idx in bones:
                sm += bn + b"\0" + struct.pack("<I", idx)
        sm += struct.pack("<H", len(self.defs))
        for d in self.defs:
            sm += d["name"] + b"\0" + struct.pack("<IHH", d["flags"], d["part"], d["motion"]) + d["params"]
            if self.version >= 4:
                sm += d["marks"] if d["marks"] else struct.pack("<I", 0)
        body = {14: bytes(mot), 15: bytes(sm)}
        out = bytearray()
        for cid in self.order:
            c = cid & 0x7FFFFFFF
            data = body.get(c, self.other.get(cid))
            out += struct.pack("<II", cid, len(data)) + data
        return bytes(out)


def merge(mine: bytes, theirs: bytes):
    """(merged bytes or None, added names, skipped {name: why}). The files are of one skeleton."""
    a, b = Omf(mine), Omf(theirs)
    a_bt, b_tb = a.bone_track(), b.track_bone()
    if set(a_bt) != set(b.bone_track()):
        return None, [], {"*": "the two files are of different skeletons"}
    a_parts, b_parts = a.part_index(), [p.decode("cp1251").lower() for p, _ in b.parts]
    same_layout = all(a_bt[b_tb[i]] == i for i in b_tb)
    have = set(a.names())
    added, skipped = [], {}
    # the i-th definition goes with the i-th motion (a definition's own motion number is stale in many files)
    for i, d in enumerate(b.defs):
        n = d["name"].decode("cp1251").lower()
        if n in have:
            continue
        if i >= len(b.motions):
            skipped[n] = "it has a definition and no motion"
            continue
        if d["part"] == 0xFFFF:
            # of no one partition: every partition of the model
            part = 0xFFFF
        elif d["flags"] & ESM_FX:
            bone = b_tb.get(d["part"])
            if bone is None:
                skipped[n] = "an FX of a bone the file does not name"
                continue
            part = a_bt[bone]
        else:
            pname = b_parts[d["part"]] if d["part"] < len(b_parts) else None
            if pname not in a_parts:
                skipped[n] = "a partition the module's file does not have"
                continue
            part = a_parts[pname]
        if same_layout:
            body = b.motions[i]
        else:
            split = b.split_tracks(i)
            if split is None:
                skipped[n] = "its tracks do not read whole (an old file)"
                continue
            name, frames, tracks = split
            ordered = [None] * len(tracks)
            for j, t in enumerate(tracks):
                ordered[a_bt[b_tb[j]]] = t
            body = name + b"\0" + struct.pack("<I", frames) + b"".join(ordered)
        a.motions.append(body)
        a.defs.append({"name": d["name"], "flags": d["flags"], "part": part, "motion": len(a.motions) - 1,
                       "params": d["params"], "marks": d["marks"] if a.version >= 4 else b""})
        have.add(n)
        added.append(n)
    if not added:
        return None, [], skipped
    if len(a.motions) >= MAX_MOTIONS:
        return None, [], {"*": f"{len(a.motions)} motions: more than a file may hold"}
    return a.build(), added, skipped
