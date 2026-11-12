"""The comparison of diff_compile.py: what the Rust compiler wrote against what the C++ compilers wrote.

Rules (one per chunk; anything else is byte for byte):
  build.lights        byte for byte.
  build.cform         chunk 0: header and vertices byte for byte, each triangle's last word left out (the C++ writes
                      the low half of a face pointer, the Rust the triangle's index); chunk 1: per face (b_rc_face,
                      36 bytes) the material, the game material, the texture coordinates and `reserved`, the
                      padding after the material and after `reserved` left out (uninitialised in the C++).
  level.cform         byte for byte, else the triangles as a multiset; else (QSlim re-rates the edges around a
                      contraction in the order of their heap addresses, MxQSlim.cpp apply_contraction: the two C++
                      runs differ too) the header and the triangle count equal, the triangles one side alone has no
                      more than between cpp1 and cpp2 (at least 1e-4 of all), every material word's area within 1e-4.
  level               chunks 1, 2, 4, 6, 7, 8 byte for byte; chunk 3 visual by visual, each sub-chunk byte for byte
                      but a progressive visual's fast path (22), whose sliding windows' four reserved words are left
                      out (never set in the C++: cpp1 and cpp2 differ there).
  level.geom/geomx    the header's version byte for byte, its quality left out (uninitialised in the C++); the vertex
                      buffers byte for byte but the light bytes (normal's w, the colour) under the light bounds below;
                      the lightmap coordinates (tc1) left out of that: a deflector the light finds flat shrinks to a
                      texel or two (compress_*), so a light difference moves the deflectors after it in the page;
                      the light is compared through each side's own coordinates (below); index buffers
                      and sliding windows byte for byte, but in level.geomx the fast path of a visual that is not
                      progressive (D3DX9's D3DXOptimizeFaces, closed code; here Forsyth's linear-speed vertex cache
                      order) as a multiset of triangles by corner positions, with the same vertex positions.
  lightmaps           each lit texel of every lightmapped triangle sampled in both pages through the triangle's own
                      lightmap coordinates (deflectors may sit elsewhere in a page): mean absolute error and the 99th
                      percentile per channel, against bounds set from the C++ run-to-run floor (cpp1 against cpp2).
  implicit textures   texel by texel (the layout is the texture's own).
  recalculation_data_slots.details   the C++ detail compiler's state for its partial relight, read by nothing but
                      its own next run: not written (the resumed light is not ported).
  level.details       slot by slot: heights, ids and palette byte for byte, the five 4-bit colours within the bounds.
"""
from __future__ import annotations

import io
import json
import os
import struct
from collections import Counter

import numpy as np

VERSION = 4

# The bounds of the light, per channel in 0..255 steps (mean absolute error, 99th percentile). The C++ compiler's
# floor (cpp1 against cpp2) is zero; the bounds are what the Rust compiler is allowed to differ in: the DXT5 encoder
# (the C++ NVTT, here texpresso - a flat 128 decodes as 126 against 128, a third of a 5-bit step, 2.7), another
# ray-triangle test than Opcode's on grazing and coincident faces, the transparencies multiplied nearest first,
# Rust's sin/cos/pow. Measured values are in STATE.md.
LIGHT_MEAN = 3.0
LIGHT_P99 = 16.0
VERTEX_MEAN = 2.0
VERTEX_P99 = 16.0
DETAIL_MEAN = 0.5
DETAIL_P99 = 2.0


def chunks(data: bytes) -> list[tuple[int, bytes]]:
    out, at = [], 0
    while at + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, at)
        out.append((cid & 0x7fffffff, data[at + 8: at + 8 + size]))
        at += 8 + size
    return out


def chunk_map(data: bytes) -> dict[int, bytes]:
    return dict(chunks(data))


def read(d: str, name: str) -> bytes | None:
    p = os.path.join(d, *name.split("/"))
    try:
        with open(p, "rb") as f:
            return f.read()
    except OSError:
        return None


# ---------------------------------------------------------------------------------------- geometry

def cmp_build_cform(a: bytes, b: bytes) -> list[str]:
    ca, cb = chunk_map(a), chunk_map(b)
    out = []
    h0, h1 = ca.get(0, b""), cb.get(0, b"")
    if h0[:36] != h1[:36]:
        out.append("build.cform: header differs")
    _, vc, fc = struct.unpack_from("<III", h0, 0)
    if h0[36:36 + vc * 12] != h1[36:36 + vc * 12]:
        out.append("build.cform: vertices differ")
    ta = np.frombuffer(h0[36 + vc * 12:], dtype="<u4").reshape(-1, 4)[:, :3]
    tb = np.frombuffer(h1[36 + vc * 12:], dtype="<u4").reshape(-1, 4)[:, :3] if len(h1) >= 36 + vc * 12 else None
    if tb is None or ta.shape != tb.shape or (ta != tb).any():
        out.append("build.cform: triangle corners differ")
    fa = np.frombuffer(ca.get(1, b""), dtype=np.uint8).reshape(-1, 36)
    fb = np.frombuffer(cb.get(1, b""), dtype=np.uint8).reshape(-1, 36)
    keep = np.r_[0:2, 4:34]
    if fa.shape != fb.shape or (fa[:, keep] != fb[:, keep]).any():
        out.append("build.cform: face records differ")
    return out


def tri_multiset(data: bytes):
    vc, fc = struct.unpack_from("<II", data, 4)
    verts = np.frombuffer(data, dtype="<f4", count=vc * 3, offset=36).reshape(-1, 3)
    tris = np.frombuffer(data, dtype="<u4", count=fc * 4, offset=36 + vc * 12).reshape(-1, 4)
    rows = []
    for t in tris:
        corners = sorted(tuple(verts[i].tolist()) for i in t[:3])
        rows.append((tuple(corners), int(t[3])))
    rows.sort()
    return rows


def tri_area(t) -> float:
    p = np.array(t, dtype=np.float64)
    return 0.5 * float(np.linalg.norm(np.cross(p[1] - p[0], p[2] - p[0])))


def cmp_level_cform(a: bytes, b: bytes) -> tuple[list[str], str, dict]:
    """The triangles each side alone has and every material word's area; the judgement against the C++ runs' own
    difference is `judge`'s."""
    if a == b:
        return [], "byte for byte", {}
    if a[:4] != b[:4]:
        return ["level.cform: version differs"], "", {}
    ma, mb = tri_multiset(a), tri_multiset(b)
    if ma == mb:
        return [], "as a multiset of triangles", {}
    problems = [f"level.cform: {len(ma)} / {len(mb)} triangles"] if len(ma) != len(mb) else []
    ca, cb = Counter(ma), Counter(mb)
    only_a, only_b = ca - cb, cb - ca
    words: dict = {}
    for rows, k in ((ma, 0), (mb, 1)):
        for t, w in rows:
            words.setdefault(w, [0.0, 0.0])[k] += tri_area(t)
    return problems, "the C++ runs' own difference", {
        "triangles": len(ma), "one_sided": max(sum(only_a.values()), sum(only_b.values())),
        "area": {str(w): v for w, v in words.items()}}


OGF_HEADER = 1


OGF_FASTPATH = 22
OGF_SWIDATA = 6


def same_fast_path(x: bytes, y: bytes) -> bool:
    """A progressive visual's fast path: its sliding windows' four reserved words left out (OGF_Face_Save.cpp writes
    `swi.reserved`, never set: cpp1 and cpp2 differ there), every other byte equal."""
    ca, cb = chunks(x), chunks(y)
    if [c for c, _ in ca] != [c for c, _ in cb]:
        return False
    for (c, p), (_, q) in zip(ca, cb):
        if c == OGF_SWIDATA:
            if p[16:] != q[16:]:
                return False
        elif p != q:
            return False
    return True


def cmp_visuals(a: bytes, b: bytes, out: list[str]) -> dict:
    va, vb = chunks(a), chunks(b)
    stats = {"visuals": len(va), "equal": 0, "swi_rule": 0, "differ": 0}
    if len(va) != len(vb):
        out.append(f"level: {len(va)} visuals in the C++, {len(vb)} in the Rust")
    for (ia, da), (ib, db) in zip(va, vb):
        if da == db:
            stats["equal"] += 1
            continue
        sa, sb = chunks(da), chunks(db)
        bad = []
        if [c for c, _ in sa] != [c for c, _ in sb]:
            bad.append("sub-chunks " + str([c for c, _ in sa]) + " / " + str([c for c, _ in sb]))
        else:
            for (c, x), (_, y) in zip(sa, sb):
                if x == y or (c == OGF_FASTPATH and same_fast_path(x, y)):
                    continue
                bad.append(f"chunk {c} ({len(x)} / {len(y)} bytes)")
            if not bad:
                stats["swi_rule"] += 1
                continue
        stats["differ"] += 1
        if stats["differ"] <= 8:
            out.append(f"level: visual {ia}: " + "; ".join(bad))
    return stats


def cmp_level(a: bytes, b: bytes) -> tuple[list[str], dict]:
    out: list[str] = []
    ca, cb = chunks(a), chunks(b)
    ids_a, ids_b = [c for c, _ in ca], [c for c, _ in cb]
    if ids_a != ids_b:
        out.append(f"level: chunks {ids_a} / {ids_b}")
    ma, mb = dict(ca), dict(cb)
    stats = {}
    for cid in ids_a:
        x, y = ma.get(cid), mb.get(cid)
        if y is None:
            continue
        if cid == 3:
            stats = cmp_visuals(x, y, out)
        elif x != y:
            names = {1: "header", 2: "shaders", 4: "portals", 6: "lights", 7: "glows", 8: "sectors"}
            what = names.get(cid, str(cid))
            if cid == 2:
                sa = x[4:].split(b"\0")
                sb = y[4:].split(b"\0")
                extra = f" ({len(sa)} / {len(sb)} names; first different: " + next(
                    (f"{p!r} / {q!r}" for p, q in zip(sa, sb) if p != q), "-") + ")"
            else:
                extra = f" ({len(x)} / {len(y)} bytes)"
            out.append(f"level: chunk {cid} {what} differs{extra}")
    return out, stats


def decl_of(blob: bytes, at: int) -> tuple[list[tuple], int]:
    el = []
    while True:
        stream, off, ty, meth, usage, idx = struct.unpack_from("<HHBBBB", blob, at)
        at += 8
        if stream == 0xff:
            break
        el.append((off, ty, usage, idx))
    return el, at


SIZE = {2: 12, 4: 4, 6: 4, 7: 8}


def vbs(data: bytes) -> list[tuple[list[tuple], np.ndarray]]:
    c = chunk_map(data).get(9, b"")
    n = struct.unpack_from("<I", c, 0)[0]
    at = 4
    out = []
    for _ in range(n):
        el, at = decl_of(c, at)
        stride = max((o + SIZE.get(t, 4)) for o, t, _, _ in el)
        cnt = struct.unpack_from("<I", c, at)[0]
        at += 4
        raw = np.frombuffer(c, dtype=np.uint8, count=cnt * stride, offset=at).reshape(cnt, stride)
        at += cnt * stride
        out.append((el, raw))
    return out


def light_columns(el) -> list[int]:
    """The bytes of a vertex that carry light: the normal's w (hemisphere), a colour's four bytes."""
    cols = []
    for off, ty, usage, idx in el:
        if usage == 3 and ty == 4:
            cols.append(off + 3)
        if usage == 10 and ty == 4:
            cols += [off, off + 1, off + 2, off + 3]
    return cols


def err_stats(d: np.ndarray) -> tuple[float, float, float]:
    if d.size == 0:
        return 0.0, 0.0, 0.0
    d = np.abs(d.astype(np.int32)).ravel()
    return float(d.mean()), float(np.percentile(d, 99)), float(d.max())


def cmp_geom(name: str, a: bytes, b: bytes, out: list[str], light: dict, fast_sets=None) -> None:
    ma, mb = chunk_map(a), chunk_map(b)
    if ma.get(1, b"")[:2] != mb.get(1, b"")[:2]:
        out.append(f"{name}: header version differs")
    va, vb = vbs(a), vbs(b)
    if name == "level.geomx" and fast_sets:
        # the fast path of a visual that is not progressive goes through D3DXOptimizeFaces / -Vertices, D3DX's own
        # closed code (here XrETools's port of the same calls): its triangles as a multiset of corner positions
        bad = fast_path_diffs(a, b, fast_sets)
        out += bad
        return
    for cid, what in ((10, "indices"), (11, "sliding windows")):
        if ma.get(cid) != mb.get(cid):
            out.append(f"{name}: {what} differ ({len(ma.get(cid, b''))} / {len(mb.get(cid, b''))} bytes)")
    if len(va) != len(vb):
        out.append(f"{name}: {len(va)} / {len(vb)} vertex buffers")
        return
    diffs = []
    for k, ((ea, ra), (eb, rb)) in enumerate(zip(va, vb)):
        if ea != eb or ra.shape != rb.shape:
            out.append(f"{name}: vertex buffer {k} has another format or size {ra.shape} / {rb.shape}")
            continue
        cols = light_columns(ea)
        lm = [o for o, t, u, i in ea if u == 5 and i == 1]
        skip = cols + [c for o in lm for c in range(o, o + 4)]
        rest = [i for i in range(ra.shape[1]) if i not in skip]
        neq = (ra[:, rest] != rb[:, rest]).any(axis=1)
        if neq.any():
            first = int(np.argmax(neq))
            cols_bad = [i for i in rest if (ra[:, i] != rb[:, i]).any()]
            out.append(f"{name}: vertex buffer {k}: {int(neq.sum())} of {len(ra)} vertices differ outside the light "
                       f"(first {first}, bytes {cols_bad[:12]})")
        if cols:
            diffs.append(ra[:, cols].astype(np.int32) - rb[:, cols].astype(np.int32))
    if diffs:
        m, p, x = err_stats(np.concatenate([d.ravel() for d in diffs]))
        light[name] = {"mean": m, "p99": p, "max": x}


# ---------------------------------------------------------------------------------------- light

def ibs_of(geom: bytes) -> list[np.ndarray]:
    ib = chunk_map(geom).get(10, b"")
    nib = struct.unpack_from("<I", ib, 0)[0] if ib else 0
    out, at = [], 4
    for _ in range(nib):
        cnt = struct.unpack_from("<I", ib, at)[0]
        out.append(np.frombuffer(ib, dtype="<u2", count=cnt, offset=at + 4))
        at += 4 + cnt * 2
    return out


def fast_containers(level: bytes) -> list:
    """Per visual: (its fast path's container, whether it is progressive)."""
    out = []
    for _, v in chunks(chunk_map(level).get(3, b"")):
        sub = chunk_map(v)
        fp = sub.get(22)
        h = sub.get(1)
        if fp and h:
            g = chunk_map(fp).get(21)
            out.append((struct.unpack_from("<6I", g, 0), h[1] == 2))
        else:
            out.append(None)
    return out


def fast_path_diffs(a: bytes, b: bytes, sets) -> list[str]:
    la, lb = sets
    va, vb = vbs(a), vbs(b)
    ia, ib = ibs_of(a), ibs_of(b)
    out = []
    for k, (ca, cb) in enumerate(zip(fast_containers(la), fast_containers(lb))):
        if ca is None and cb is None:
            continue
        if ca is None or cb is None or ca[1] != cb[1]:
            out.append(f"level.geomx: visual {k} has a fast path on one side only")
            continue
        def tris(c, v, i):
            (vbid, vs, vcnt, ibid, is_, icnt), _ = c
            raw = v[vbid][1][vs:vs + vcnt].copy().view("<f4").reshape(-1, 3)
            idx = i[ibid][is_:is_ + icnt].reshape(-1, 3)
            return raw, idx
        ra, xa = tris(ca, va, ia)
        rb, xb = tris(cb, vb, ib)
        if ca[1]:
            if not (np.array_equal(ra, rb) and np.array_equal(xa, xb)):
                out.append(f"level.geomx: the progressive fast path of visual {k} differs")
            continue
        ta = sorted(tuple(sorted(tuple(ra[j].tolist()) for j in t)) for t in xa)
        tb = sorted(tuple(sorted(tuple(rb[j].tolist()) for j in t)) for t in xb)
        if ta != tb or sorted(map(tuple, ra.tolist())) != sorted(map(tuple, rb.tolist())):
            out.append(f"level.geomx: the fast path of visual {k} holds other triangles")
    return out


def dds_rgba(data: bytes) -> np.ndarray:
    from PIL import Image
    im = Image.open(io.BytesIO(data))
    im.load()
    return np.asarray(im.convert("RGBA"))


def lmap_triangles(level: bytes, geom: bytes) -> list[tuple[str, np.ndarray]]:
    """Per lightmapped visual: its page name and its triangles' lightmap coordinates (n, 3, 2)."""
    shaders = chunk_map(level).get(2, b"")
    names = [s.decode("cp1251", "replace") for s in shaders[4:].split(b"\0")]
    buffers = vbs(geom)
    ib = chunk_map(geom).get(10, b"")
    nib = struct.unpack_from("<I", ib, 0)[0]
    ibs, at = [], 4
    for _ in range(nib):
        cnt = struct.unpack_from("<I", ib, at)[0]
        ibs.append(np.frombuffer(ib, dtype="<u2", count=cnt, offset=at + 4))
        at += 4 + cnt * 2
    out = []
    for _, v in chunks(chunk_map(level).get(3, b"")):
        sub = chunk_map(v)
        h = sub.get(1)
        g = sub.get(21)
        if not h or not g or h[1] not in (0, 2):
            continue
        shader = names[struct.unpack_from("<H", h, 2)[0]] if struct.unpack_from("<H", h, 2)[0] < len(names) else ""
        tex = shader.split("/", 1)[-1].split(",")
        page = next((t[:-2] for t in tex if t.startswith("lmap#") and t.endswith("_1")), None)
        if not page:
            continue
        vb_id, vs, vc, ib_id, is_, ic = struct.unpack_from("<6I", g, 0)
        el, raw = buffers[vb_id]
        lm = next((o for o, t, u, i in el if u == 5 and i == 1), None)
        if lm is None:
            continue
        uv = raw[vs:vs + vc, lm:lm + 4].copy().view("<i2").reshape(-1, 2).astype(np.float64) / 32768.0
        idx = ibs[ib_id][is_:is_ + ic].reshape(-1, 3)
        out.append((page, uv[idx]))
    return out


def sample_pages(tris_a, tris_b, pages_a, pages_b):
    """Every texel centre inside each C++ triangle, looked up in the C++ page and, through the same barycentrics, in
    the Rust one. Answers the two value arrays (n, 8) and the samples' C++ page positions."""
    va, vb, where = [], [], []
    for (pa, ta), (pb, tb) in zip(tris_a, tris_b):
        if pa not in pages_a or pb not in pages_b or ta.shape != tb.shape:
            continue
        A, B = pages_a[pa], pages_b[pb]
        h, w = A.shape[:2]
        for t1, t2 in zip(ta, tb):
            p = t1 * np.array([w, h]) - 0.5
            lo = np.floor(p.min(axis=0)).astype(int)
            hi = np.ceil(p.max(axis=0)).astype(int)
            xs, ys = np.meshgrid(np.arange(lo[0], hi[0] + 1), np.arange(lo[1], hi[1] + 1))
            pts = np.stack([xs.ravel(), ys.ravel()], axis=1).astype(np.float64)
            v0, v1 = p[1] - p[0], p[2] - p[0]
            den = v0[0] * v1[1] - v1[0] * v0[1]
            if abs(den) < 1e-9:
                continue
            d = pts - p[0]
            l1 = (d[:, 0] * v1[1] - v1[0] * d[:, 1]) / den
            l2 = (v0[0] * d[:, 1] - d[:, 0] * v0[1]) / den
            l0 = 1 - l1 - l2
            inside = (l0 >= 0) & (l1 >= 0) & (l2 >= 0)
            if not inside.any():
                continue
            pts, l0, l1, l2 = pts[inside].astype(int), l0[inside], l1[inside], l2[inside]
            ok = (pts[:, 0] >= 0) & (pts[:, 0] < w) & (pts[:, 1] >= 0) & (pts[:, 1] < h)
            pts, l0, l1, l2 = pts[ok], l0[ok], l1[ok], l2[ok]
            q = (t2[0][None] * l0[:, None] + t2[1][None] * l1[:, None] + t2[2][None] * l2[:, None])
            hb, wb = B.shape[:2]
            qx = np.clip(np.floor(q[:, 0] * wb).astype(int), 0, wb - 1)
            qy = np.clip(np.floor(q[:, 1] * hb).astype(int), 0, hb - 1)
            va.append(A[pts[:, 1], pts[:, 0]])
            vb.append(B[qy, qx])
            where.append((pa, pts))
    if not va:
        return np.zeros((0, 8)), np.zeros((0, 8)), []
    return np.concatenate(va), np.concatenate(vb), where


def load_pages(d: str) -> dict[str, np.ndarray]:
    pages = {}
    for n in os.listdir(d):
        if n.startswith("lmap#") and n.endswith("_1.dds"):
            base = n[:-6]
            b2 = read(d, base + "_2.dds")
            if b2 is None:
                continue
            a1 = dds_rgba(read(d, n))
            a2 = dds_rgba(b2)
            pages[base] = np.concatenate([a1, a2], axis=2)
    return pages


def light_compare(da: str, db: str):
    la, ga = read(da, "level"), read(da, "level.geom")
    lb, gb = read(db, "level"), read(db, "level.geom")
    ta, tb = lmap_triangles(la, ga), lmap_triangles(lb, gb)
    pa, pb = load_pages(da), load_pages(db)
    if len(ta) != len(tb):
        return None, f"{len(ta)} / {len(tb)} lightmapped visuals", pa, pb, []
    a, b, where = sample_pages(ta, tb, pa, pb)
    return (a, b), "", pa, pb, where


def channel_stats(a: np.ndarray, b: np.ndarray) -> dict:
    names = ["1.r", "1.g", "1.b", "1.a(sun)", "2.r", "2.g", "2.b", "2.a(hemi)"]
    out = {}
    d = np.abs(a.astype(np.int32) - b.astype(np.int32))
    for i, n in enumerate(names):
        if d.shape[0] == 0:
            out[n] = {"mean": 0.0, "p99": 0.0, "max": 0}
            continue
        out[n] = {"mean": float(d[:, i].mean()), "p99": float(np.percentile(d[:, i], 99)), "max": int(d[:, i].max())}
    return out


def save_pictures(out_dir: str, pa: dict, pb: dict, where, a, b) -> list[str]:
    from PIL import Image
    os.makedirs(out_dir, exist_ok=True)
    made = []
    for name, img in pa.items():
        h, w = img.shape[:2]
        rust_in_cpp = np.zeros_like(img)
        mask = np.zeros((h, w), dtype=bool)
        at = 0
        for page, pts in where:
            n = len(pts)
            if page == name:
                rust_in_cpp[pts[:, 1], pts[:, 0]] = b[at:at + n]
                mask[pts[:, 1], pts[:, 0]] = True
            at += n
        diff = np.abs(img.astype(np.int32) - rust_in_cpp.astype(np.int32))
        diff[~mask] = 0
        safe = name.replace("#", "_")
        for tag, arr in (("cpp", img), ("rust_in_cpp_layout", rust_in_cpp)):
            Image.fromarray(np.ascontiguousarray(arr[:, :, :3]).astype(np.uint8), "RGB").save(os.path.join(out_dir, f"{safe}_base_{tag}.png"))
            Image.fromarray(np.ascontiguousarray(arr[:, :, 4:7]).astype(np.uint8), "RGB").save(os.path.join(out_dir, f"{safe}_sun_{tag}.png"))
            Image.fromarray(np.ascontiguousarray(arr[:, :, 7]).astype(np.uint8), "L").save(os.path.join(out_dir, f"{safe}_hemi_{tag}.png"))
        amp = np.clip(diff[:, :, [0, 4, 7]] * 8, 0, 255).astype(np.uint8)
        Image.fromarray(amp, "RGB").save(os.path.join(out_dir, f"{safe}_diff_x8.png"))
        made.append(safe)
    for name, img in pb.items():
        safe = name.replace("#", "_")
        Image.fromarray(np.ascontiguousarray(img[:, :, :3]).astype(np.uint8), "RGB").save(os.path.join(out_dir, f"{safe}_base_rust.png"))
        Image.fromarray(np.ascontiguousarray(img[:, :, 7]).astype(np.uint8), "L").save(os.path.join(out_dir, f"{safe}_hemi_rust.png"))
    return made


# ---------------------------------------------------------------------------------------- details

def cmp_details(a: bytes, b: bytes, out: list[str]) -> dict:
    ca, cb = chunk_map(a), chunk_map(b)
    for cid in ca:
        if cid != 2 and ca[cid] != cb.get(cid):
            out.append(f"level.details: chunk {cid} differs")
    sa = np.frombuffer(ca.get(2, b""), dtype="<u4").reshape(-1, 4)
    sb = np.frombuffer(cb.get(2, b""), dtype="<u4").reshape(-1, 4)
    if sa.shape != sb.shape:
        out.append(f"level.details: {sa.shape} / {sb.shape} slots")
        return {}
    keep_w1 = np.uint32(0x00000fff)
    same = (sa[:, 0] == sb[:, 0]) & (sa[:, 2] == sb[:, 2]) & (sa[:, 3] == sb[:, 3]) & ((sa[:, 1] & keep_w1) == (sb[:, 1] & keep_w1))
    if not same.all():
        out.append(f"level.details: {int((~same).sum())} slots differ outside their colours")
    ea = np.stack([(sa[:, 1] >> s) & 0xf for s in (12, 16, 20, 24, 28)], axis=1).astype(np.int32)
    eb = np.stack([(sb[:, 1] >> s) & 0xf for s in (12, 16, 20, 24, 28)], axis=1).astype(np.int32)
    d = np.abs(ea - eb)
    names = ["dir", "hemi", "r", "g", "b"]
    return {n: {"mean": float(d[:, i].mean()) if len(d) else 0.0,
                "p99": float(np.percentile(d[:, i], 99)) if len(d) else 0.0,
                "max": int(d[:, i].max()) if len(d) else 0} for i, n in enumerate(names)}


# ---------------------------------------------------------------------------------------- the whole

def compare_dirs(da: str, db: str, files_a: set, files_b: set, out_dir: str | None) -> dict:
    """One C++ output against another output (the Rust one, or the second C++ run)."""
    res: dict = {"problems": [], "rules": {}, "light": {}, "vertex": {}, "details": {}}
    pr = res["problems"]
    skip_b = {"build.err"}
    # xrDO_Light's bookkeeping for a later partial relight (recalculation.cpp): only its own next run reads it, and
    # the resumed light is not ported (STATE.md, phase 8a)
    skip_a = {"recalculation_data_slots.details"}
    only_a, only_b = sorted(files_a - files_b - skip_a), sorted(files_b - files_a - skip_b)
    if only_a:
        pr.append(f"only in the C++ output: {only_a}")
    if only_b:
        pr.append(f"only in the other output: {only_b}")
    for n in sorted(files_a & files_b):
        a, b = read(da, n), read(db, n)
        if n == "build.lights":
            if a != b:
                pr.append("build.lights differs")
        elif n == "build.cform":
            pr += cmp_build_cform(a, b)
        elif n == "level.cform":
            p, rule, res["cform"] = cmp_level_cform(a, b)
            pr += p
            res["rules"]["level.cform"] = rule
        elif n == "level":
            p, st = cmp_level(a, b)
            pr += p
            res["rules"]["visuals"] = st
        elif n in ("level.geom", "level.geomx"):
            sets = (read(da, "level"), read(db, "level")) if n == "level.geomx" else None
            cmp_geom(n, a, b, pr, res["vertex"], sets)
        elif n == "level.details":
            res["details"] = cmp_details(a, b, pr)
        elif n.startswith("lmap#"):
            continue
        elif n.endswith(".dds"):
            A, B = dds_rgba(a), dds_rgba(b)
            if A.shape != B.shape:
                pr.append(f"{n}: {A.shape} / {B.shape}")
            else:
                d = np.abs(A.astype(np.int32) - B.astype(np.int32))
                res["light"][n] = {"mean": float(d.mean()), "p99": float(np.percentile(d, 99)), "max": int(d.max())}
        elif a != b:
            pr.append(f"{n} differs ({len(a)} / {len(b)} bytes)")
    if "level" in files_a & files_b and "level.geom" in files_a & files_b:
        got, why, pa, pb, where = light_compare(da, db)
        if got is None:
            pr.append("lightmaps: " + why)
        else:
            a, b = got
            res["light"]["lightmaps"] = channel_stats(a, b)
            res["light"]["samples"] = int(len(a))
            if out_dir:
                res["pictures"] = save_pictures(out_dir, pa, pb, where, a, b)
    return res


CFORM_AREA = 1e-4


def judge(res: dict, floor: dict) -> list[str]:
    bad = []
    # QSlim re-rates the edges around a contraction in the order of their heap addresses (MxQSlim.cpp,
    # apply_contraction), so ties fall another way from run to run: the triangles one side alone has may be as many
    # as between the two C++ runs (at least 1e-4 of all), and every material word keeps its area within 1e-4
    cf = res.get("cform") or {}
    if cf:
        allowed = max((floor.get("cform") or {}).get("one_sided", 0), -(-cf["triangles"] // 10000))
        if cf["one_sided"] > allowed:
            bad.append(f"level.cform: {cf['one_sided']} triangles one side alone has (allowed {allowed})")
        for w, (x, y) in cf["area"].items():
            if abs(x - y) > CFORM_AREA * max(x, 1e-6):
                bad.append(f"level.cform: material word {w} has the area {y:.6f} against {x:.6f}")
    lm = res["light"].get("lightmaps", {})
    for ch, s in lm.items():
        if isinstance(s, dict) and (s["mean"] > LIGHT_MEAN or s["p99"] > LIGHT_P99):
            bad.append(f"lightmap channel {ch}: mean {s['mean']:.2f}, p99 {s['p99']:.1f} (bounds {LIGHT_MEAN}, {LIGHT_P99})")
    for n, s in res["vertex"].items():
        if s["mean"] > VERTEX_MEAN or s["p99"] > VERTEX_P99:
            bad.append(f"{n} vertex light: mean {s['mean']:.2f}, p99 {s['p99']:.1f}")
    for n, s in res["light"].items():
        if n.endswith(".dds") and (s["mean"] > LIGHT_MEAN or s["p99"] > LIGHT_P99):
            bad.append(f"{n}: mean {s['mean']:.2f}, p99 {s['p99']:.1f}")
    for ch, s in res["details"].items():
        if s["mean"] > DETAIL_MEAN or s["p99"] > DETAIL_P99:
            bad.append(f"details {ch}: mean {s['mean']:.2f}, p99 {s['p99']:.1f}")
    return bad


PHASE_SLACK = 0.25


def phase_starts(r: dict) -> list[tuple[float, str]]:
    starts = []
    for t, l in r["lines"]:
        text = l if isinstance(l, str) else str(l)
        if "New phase started:" in text:
            name = text.split("New phase started:", 1)[1].strip()
            starts.append((t, "Reading project" if name.startswith("Reading project") else name))
    return starts


def load_of(samples, t0, t1) -> float:
    if not samples:
        return 0.0
    c0 = next((c for t, c in samples if t >= t0), samples[-1][1])
    c1 = next((c for t, c in samples if t >= t1), samples[-1][1])
    return (c1 - c0) / max(t1 - t0, 1e-6)


def spans(r: dict, names: set) -> list[tuple[str, float, float]]:
    """The phases of a run that the other compiler names too, each lasting until the next of them starts: a phase
    only one side has (the C++'s "CFORM: creating...", "Building lightmap 0...") counts with the one before it."""
    starts = [(t, n) for t, n in phase_starts(r) if n in names] + [(r["wall"], "end")]
    seen: dict = {}
    out = []
    for (t0, n), (t1, _) in zip(starts, starts[1:]):
        k = seen.get(n, 0)
        seen[n] = k + 1
        out.append((n if k == 0 else f"{n} ({k + 1})", t1 - t0, load_of(r["cpu"], t0, t1)))
    return out


def phase_rows(c: dict, r: dict) -> tuple[list[str], list[str]]:
    # a name the two sides use a different number of times ("Saving..." of each lightmap page in the C++) cannot
    # be paired and folds into the phase before it too
    count = lambda run: Counter(n for _, n in phase_starts(run))  # noqa: E731
    cc_, rc_ = count(c), count(r)
    common = {n for n in cc_ if rc_.get(n) == cc_[n]}
    rs = {n: (t, l) for n, t, l in spans(r, common)}
    rows, slower = [], []
    for n, t, l in spans(c, common):
        rt, rl = rs.get(n, (0.0, 0.0))
        rows.append(f"{n[:36]:36} {t:8.2f} {l:5.1f} {rt:8.2f} {rl:5.1f}")
        if rt > t + PHASE_SLACK:
            slower.append(f"phase {n}: Rust {rt:.2f} s, C++ {t:.2f} s")
    if r["wall"] > c["wall"] + PHASE_SLACK:
        slower.append(f"the whole run: Rust {r['wall']:.2f} s, C++ {c['wall']:.2f} s")
    rows.append(f"{'total':36} {c['wall']:8.2f} {load_of(c['cpu'], 0, c['wall']):5.1f} {r['wall']:8.2f} "
                f"{load_of(r['cpu'], 0, r['wall']):5.1f}")
    return rows, slower


def phase_table(rec: dict, rrec: dict) -> tuple[list[str], list[str]]:
    """The phases both compilers name, side by side with their seconds and CPU load (CPU time over wall time), and
    the Rust phases that took longer than the C++ ones by more than the C++ clock's resolution (its log is read
    every 0.1 s and written a line at a time). The level compiler first, then the detail compiler."""
    rows, slower = [f"{'phase':36} {'C++ s':>8} {'load':>5} {'Rust s':>8} {'load':>5}"], []
    for cpp, rust in (("XrLC.exe", "-compile"), ("xrDO_Light.exe", "-compile_details")):
        c, r = rec["cpp1"]["runs"].get(cpp), rrec["runs"].get(rust)
        if c and r:
            rows.append(f"[{cpp} / {rust}]")
            more, late = phase_rows(c, r)
            rows += more
            slower += late
    return rows, slower


def compare(scene, rd, rec, rust_dir, rrec, out, log) -> bool:
    cpp1, cpp2 = os.path.join(rd, "cpp1"), os.path.join(rd, "cpp2")
    files1, files2 = set(rec["cpp1"]["files"]), set(rec["cpp2"]["files"])
    floor = compare_dirs(cpp1, cpp2, files1, files2, None)
    res = compare_dirs(cpp1, rust_dir, files1, set(rrec["files"]), os.path.join(out, "pictures"))
    res["floor"] = floor
    res["judged"] = judge(res, floor)
    res["phases"], slower = phase_table(rec, rrec)
    res["judged"] += slower
    with open(os.path.join(out, "compare.json"), "w", encoding="utf-8") as f:
        json.dump(res, f, indent=1)
    log(f"  floor (C++ against C++): problems {floor['problems'][:6]}")
    log(f"  floor light: {json.dumps(floor['light'].get('lightmaps', {}))[:600]}")
    for p in res["problems"]:
        log("  ! " + p)
    for p in res["judged"]:
        log("  ! " + p)
    log(f"  rules: {json.dumps(res['rules'])}")
    log(f"  light: {json.dumps(res['light'])[:1500]}")
    log(f"  vertex: {json.dumps(res['vertex'])}  details: {json.dumps(res['details'])}")
    for row in res["phases"]:
        log("  " + row)
    return not res["problems"] and not res["judged"]
