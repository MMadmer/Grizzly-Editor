"""particles.xr: effects and groups as the runtime reads them, and loose `.pe` / `.pg` files the game's loader takes.

The binary reader follows `CPEDef::Load` / `CPGDef::Load` (xrRender), the loose reader `CPEDef::Load2` /
`CPGDef::Load2` of a game build (`CPSLibrary::LoadLooseOverrides`). The game build's `Load2` reads no action
list: the compiled actions (`PED_CHUNK_ACTIONLIST`, what emits and moves the particles) are only built from
`action_NNNN` sections in the SDK editor. So the loose files carry them in an `[actions]` section the game
does not read today, byte for byte, and the effect is inert in the game until it does.
"""
from __future__ import annotations

import math
import os
import struct

import numpy as np

from ltx import Config, Reader

DF_SPRITE, DF_FRAMED, DF_TIMELIMIT, DF_ALIGN, DF_COLLISION, DF_VELSCALE = 1 << 0, 1 << 10, 1 << 14, 1 << 15, 1 << 16, 1 << 18
HEX_LINE = 1024  # bytes per `[actions]` line: an ini line holds 4096 characters


def chunks(data: bytes, pos=0, end=None):
    end = len(data) if end is None else end
    while pos + 8 <= end:
        cid, size = struct.unpack_from("<II", data, pos)
        yield cid, data[pos + 8:pos + 8 + size]
        pos += 8 + size


def find(data: bytes, cid: int):
    for c, body in chunks(data):
        if c & 0x7FFFFFFF == cid:
            if c & 0x80000000:
                raise ValueError("a compressed chunk")
            return body
    return None


def _z(body: bytes, pos=0):
    end = body.index(b"\0", pos)
    return body[pos:end].decode("cp1251", "surrogateescape"), end + 1


def f32(x: float) -> float:
    return struct.unpack("<f", struct.pack("<f", x))[0]


def read_effect(body: bytes) -> dict:
    if struct.unpack_from("<H", find(body, 1))[0] != 1:
        raise ValueError("effect version")
    name = _z(find(body, 2))[0]
    e = {"name": name, "max_particles": struct.unpack_from("<I", find(body, 3))[0]}
    al = find(body, 4)
    e["actions"] = bytes(al) if al is not None else b""
    fl = find(body, 5)
    e["flags"] = struct.unpack_from("<I", fl)[0] if fl and len(fl) >= 4 else 0
    f = e["flags"]
    if f & DF_SPRITE:
        sp = find(body, 7)
        e["shader"], p = _z(sp)
        e["texture"], _ = _z(sp, p)
    if f & DF_FRAMED:
        fr = find(body, 6)
        tx, ty, rx, ry, dim, cnt, speed = struct.unpack_from("<4f2if", fr)
        e["frame"] = (tx, ty, rx, ry, dim, cnt, speed)
    if f & DF_TIMELIMIT:
        e["timelimit"] = struct.unpack_from("<f", find(body, 8))[0]
    if f & DF_COLLISION:
        e["collision"] = struct.unpack_from("<3f", find(body, 0x21))
    if f & DF_VELSCALE:
        e["velocity_scale"] = struct.unpack_from("<3f", find(body, 0x22))
    if f & DF_ALIGN:
        ap = find(body, 0x25)
        e["align_to_path"] = struct.unpack_from("<3f", ap) if ap is not None else (f32(-math.pi / 2), 0.0, 0.0)
    return e


def read_group(body: bytes) -> dict:
    if struct.unpack_from("<H", find(body, 1))[0] != 3:
        raise ValueError("group version")
    g = {"name": _z(find(body, 2))[0]}
    fl = find(body, 3)
    g["flags"] = struct.unpack_from("<I", fl)[0] if fl and len(fl) >= 4 else 0
    tl = find(body, 5)
    limit = struct.unpack_from("<f", tl)[0] if tl is not None else 0.0
    keep = limit > 0.0
    effects = []
    ef = find(body, 4)
    if ef is not None:
        n = struct.unpack_from("<I", ef)[0]
        p = 4
        for _ in range(n):
            name, p = _z(ef, p)
            play, p = _z(ef, p)
            birth, p = _z(ef, p)
            dead, p = _z(ef, p)
            t0, t1, flags = struct.unpack_from("<2fI", ef, p)
            p += 12
            effects.append((name, play, birth, dead, t0, t1, flags))
            if not keep:
                limit = max(limit, t1)
    g["effects"] = effects
    g["timelimit"] = limit
    return g


def read_library(data: bytes) -> tuple[dict, dict]:
    if struct.unpack_from("<H", find(data, 1))[0] != 1:
        raise ValueError("particles.xr version")
    effects, groups = {}, {}
    for cid, reader, target in ((3, read_effect, effects), (4, read_group, groups)):
        obj = find(data, cid)
        if obj is None:
            continue
        expect = 0
        for c, body in chunks(obj):
            if c != expect:
                break
            d = reader(body)
            target[d["name"]] = d
            expect += 1
    return effects, groups


def same(a: dict, b: dict) -> bool:
    return {k: v for k, v in a.items() if k != "name"} == {k: v for k, v in b.items() if k != "name"}


# ---- writing ---------------------------------------------------------------------------------------------
def ftext(x: float) -> str:
    if math.isnan(x):
        return "nan"
    if math.isinf(x):
        return "inf" if x > 0 else "-inf"
    t = np.format_float_positional(np.float32(x), unique=True, trim="-")
    if f32(float(t)) == x:
        return t
    return "%.9g" % x


def itext(u: int) -> str:
    return str(u - (1 << 32) if u >= 1 << 31 else u)


def write_effect(e: dict) -> bytes:
    L = ["[_effect]", "version = 1", f"max_particles = {itext(e['max_particles'])}", f"flags = {itext(e['flags'])}"]
    if "shader" in e:
        L += ["", "[sprite]", f"shader = {e['shader']}", f"texture = {e['texture']}"]
    if "frame" in e:
        tx, ty, rx, ry, dim, cnt, speed = e["frame"]
        L += ["", "[frame]", f"tex_size = {ftext(tx)},{ftext(ty)}", f"reserved = {ftext(rx)},{ftext(ry)}",
              f"dim_x = {dim}", f"frame_count = {cnt}", f"speed = {ftext(speed)}"]
    if "timelimit" in e:
        L += ["", "[timelimit]", f"value = {ftext(e['timelimit'])}"]
    if "collision" in e:
        a, b, c = e["collision"]
        L += ["", "[collision]", f"one_minus_friction = {ftext(a)}", f"collide_resilence = {ftext(b)}", f"collide_sqr_cutoff = {ftext(c)}"]
    if "velocity_scale" in e:
        L += ["", "[velocity_scale]", "value = " + ",".join(ftext(x) for x in e["velocity_scale"])]
    if "align_to_path" in e:
        L += ["", "[align_to_path]", "default_rotation = " + ",".join(ftext(x) for x in e["align_to_path"])]
    act = e["actions"]
    L += ["", "[actions]", f"size = {len(act)}"]
    for i in range(0, len(act), HEX_LINE):
        L.append(f"data_{i // HEX_LINE:04d} = {act[i:i + HEX_LINE].hex()}")
    return ("\r\n".join(L) + "\r\n").encode("cp1251", "surrogateescape")


def write_group(g: dict) -> bytes:
    L = ["[_group]", "version = 3", f"flags = {itext(g['flags'])}", f"effects_count = {len(g['effects'])}",
         f"timelimit = {ftext(g['timelimit'])}"]
    for i, (name, play, birth, dead, t0, t1, flags) in enumerate(g["effects"]):
        L += ["", f"[effect_{i:04d}]", f"effect_name = {name}", f"on_play_child = {play}", f"on_birth_child = {birth}",
              f"on_death_child = {dead}", f"time0 = {ftext(t0)}", f"time1 = {ftext(t1)}", f"flags = {itext(flags)}"]
    return ("\r\n".join(L) + "\r\n").encode("cp1251", "surrogateescape")


# ---- reading a loose file the way the game does -----------------------------------------------------------
class _One:
    def __init__(self, data):
        self.data = data
        self.names = {"x"}

    def exists(self, rel):
        return rel == "x"

    def read(self, rel):
        return self.data


def _ini(data: bytes) -> Config:
    c = Config()
    Reader(_One(data), c).load("x")
    return c


def _r(c: Config, s, k):
    sect = c.sections.get(s)
    if sect is None or k not in sect.own:
        raise KeyError(f"[{s}] {k}")
    return sect.own[k]


def _atoi(t):
    t = (t or "").strip()
    sign = -1 if t.startswith("-") else 1
    digits = ""
    for ch in t.lstrip("+-"):
        if not ch.isdigit():
            break
        digits += ch
    v = sign * int(digits or "0")
    v = max(min(v, 2**31 - 1), -2**31)
    return v & 0xFFFFFFFF


def _atof(t):
    try:
        return f32(float(t))
    except (TypeError, ValueError):
        return 0.0


def _vec(t, n):
    parts = (t or "").split(",")
    return tuple(_atof(parts[i]) if i < len(parts) else 0.0 for i in range(n))


def load2_effect(data: bytes, name: str) -> dict:
    c = _ini(data)
    e = {"name": name, "max_particles": _atoi(_r(c, "_effect", "max_particles")), "flags": _atoi(_r(c, "_effect", "flags"))}
    f = e["flags"]
    if f & DF_SPRITE:
        e["shader"] = _r(c, "sprite", "shader") or ""
        e["texture"] = _r(c, "sprite", "texture") or ""
    if f & DF_FRAMED:
        tx, ty = _vec(_r(c, "frame", "tex_size"), 2)
        rx, ry = _vec(_r(c, "frame", "reserved"), 2)
        dim = _atoi(_r(c, "frame", "dim_x"))
        cnt = _atoi(_r(c, "frame", "frame_count"))
        e["frame"] = (tx, ty, rx, ry, dim - (1 << 32) if dim >= 1 << 31 else dim, cnt - (1 << 32) if cnt >= 1 << 31 else cnt,
                      _atof(_r(c, "frame", "speed")))
    if f & DF_TIMELIMIT:
        e["timelimit"] = _atof(_r(c, "timelimit", "value"))
    if f & DF_COLLISION:
        e["collision"] = (_atof(_r(c, "collision", "one_minus_friction")), _atof(_r(c, "collision", "collide_resilence")),
                          _atof(_r(c, "collision", "collide_sqr_cutoff")))
    if f & DF_VELSCALE:
        e["velocity_scale"] = _vec(_r(c, "velocity_scale", "value"), 3)
    if f & DF_ALIGN:
        e["align_to_path"] = _vec(_r(c, "align_to_path", "default_rotation"), 3)
    # not read by the game build: the carried action list
    size = int(_r(c, "actions", "size") or 0)
    hexes = "".join(_r(c, "actions", f"data_{i:04d}") or "" for i in range((size + HEX_LINE - 1) // HEX_LINE))
    e["actions"] = bytes.fromhex(hexes)
    return e


def load2_group(data: bytes, name: str) -> dict:
    c = _ini(data)
    g = {"name": name, "flags": _atoi(_r(c, "_group", "flags"))}
    effects = []
    for i in range(_atoi(_r(c, "_group", "effects_count"))):
        s = f"effect_{i:04d}"
        effects.append((_r(c, s, "effect_name") or "", _r(c, s, "on_play_child") or "", _r(c, s, "on_birth_child") or "",
                        _r(c, s, "on_death_child") or "", _atof(_r(c, s, "time0")), _atof(_r(c, s, "time1")), _atoi(_r(c, s, "flags"))))
    g["effects"] = effects
    g["timelimit"] = _atof(_r(c, "_group", "timelimit"))
    return g


def equal_fields(a: dict, b: dict) -> list[str]:
    """Fields that differ; NaN equals NaN."""
    def eq(x, y):
        if isinstance(x, float) and isinstance(y, float):
            return (math.isnan(x) and math.isnan(y)) or struct.pack("<f", x) == struct.pack("<f", y)
        if isinstance(x, (tuple, list)) and isinstance(y, (tuple, list)):
            return len(x) == len(y) and all(eq(p, q) for p, q in zip(x, y))
        return x == y
    return [k for k in set(a) | set(b) if k != "name" and not eq(a.get(k), b.get(k))]


def loose_path(name: str, kind: str) -> str:
    return os.path.join("gamedata", "particles", name.replace("/", "\\") + kind)
