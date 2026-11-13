"""The comparison of diff_compile.py: what the Rust compiler wrote against what the C++ compilers wrote.

Rules (one per chunk; anything else is byte for byte):
  build.lights        byte for byte.
  build.err           byte for byte: the Rust build.err beside the level against the C++ build_<user>.err of its
                      log folder (T-junctions, multiple edges, invalid faces).
  build.cform         chunk 0: header and vertices byte for byte, each triangle's last word left out (the C++ writes
                      the low half of a face pointer, the Rust the triangle's index); chunk 1: per face (b_rc_face,
                      36 bytes) the material, the game material, the texture coordinates and `reserved`, the
                      padding after the material and after `reserved` left out (uninitialised in the C++).
  level.cform         each triangle's suppress_shadows / suppress_wm bits (14, 15 of its word) left out: XrLC never
                      sets them - CollectorPacked::add_face assigns the material and the sector of a TRI on the
                      stack, the two bits keep what the stack held (the Draft quality, where no QSlim pass adds the
                      faces again: 92,001 of mp_close_combat's triangles carry one, the same in both C++ runs); the
                      Rust writes 0. Then byte for byte, else the triangles as a multiset; else (QSlim re-rates the edges around a
                      contraction in the order of their heap addresses, MxQSlim.cpp apply_contraction: the two C++
                      runs differ too, and so do two C++ builds of one source) the header equal and either the same
                      triangle count and the same surface triangulated another way - the same vertices, every corner
                      of a triangle one side alone a vertex of the other, per material word and plane the one-sided
                      triangles' areas equal within 1e-4 - or, against what cpp1 and cpp2 differ in, no more than
                      twice that (CFORM_FLOOR): the triangle counts apart, the triangles one side alone has (each at
                      least 1e-4 of all), the area that changed material per word (or within 1e-4 of its area) and
                      in all (or within 1e-6 of the area). The port's QSlim draw is a third one; twice since
                      l03u_agr_underground_own: material word 1310754 moved 0.0170 m2 against the C++ runs' 0.0089
                      (half again before, and once no more than theirs for the one-sided triangles -
                      l03u_agr_underground_paged: 452 against 419).
  level               chunks 1, 2, 4, 6, 7, 8 byte for byte; chunk 3 visual by visual, each sub-chunk byte for byte
                      but a progressive visual's fast path (22), whose sliding windows' four reserved words are left
                      out (never set in the C++: cpp1 and cpp2 differ there), and the light of an MU reference: a
                      tree's c_scale / c_bias (OGF_TREEDEF2; its xform byte for byte) and a LOD's corner colours
                      (OGF_LODDEF2; its corners and texture coordinates byte for byte), judged as below.
  MU models           their visuals' geometry byte for byte (the vertices but their light, the indices, the sliding
                      windows, a tree's xform, a LOD's quads); a model's own light (level.geom) under the vertex
                      bounds; a reference's light - every tree vertex lit through c_scale / c_bias, every LOD corner
                      colour - under MU_REF_MEAN / MU_REF_P99 for the references the C++ lights alone (its four
                      threads draw the sample directions from the one ::Random, so even those take another
                      sequence from run to run; the port seeds each vertex from its position). The C++ keeps a
                      vertex's light in the MODEL its references share (xrMU_Model::calc_lighting writes _vertex::C)
                      and lights the references on four threads and a fifth for the rest (CMULight, get_intervals):
                      a reference whose model has another reference on another thread takes some of that one's
                      light, so its light is reported, not judged.
  level.geom/geomx    the header's version byte for byte, its quality left out (uninitialised in the C++); the vertex
                      buffers byte for byte but the light bytes (normal's w, the colour) under the light bounds below;
                      the lightmap coordinates (tc1) left out of that: a deflector the light finds flat shrinks to a
                      texel or two (compress_*), so a light difference moves the deflectors after it in the page;
                      the light is compared through each side's own coordinates (below); index buffers
                      and sliding windows byte for byte, but in level.geomx the fast path of a visual that is not
                      progressive (D3DX9's D3DXOptimizeFaces, closed code; here Forsyth's linear-speed vertex cache
                      order) as a multiset of triangles by corner positions, with the same vertex positions.
  lightmaps           the same pages on both sides; each texel centre inside every lightmapped triangle sampled in
                      both pages through the triangle's own lightmap coordinates (deflectors may sit elsewhere in a
                      page); a FLAT triangle - its coordinates a line or a point, a deflector the light found flat and
                      shrank (about half of all) - by the texels along its longest side, or the one under it, again
                      through each side's own coordinates (flat_samples). Mean absolute error and 99th percentile per
                      channel over all samples, under the bounds below; the flat ones are reported on their own too.
                      Every channel is judged as it is. The second picture's colour is the sun once more
                      (Pack_hemi) and the game reads its green (get_sun: lmh.g of s_hemi); its red and blue are
                      DXT5's 5-bit channels, where the two encoders round a flat value two steps apart (mp_test:
                      mean 2.0, p99 2), so those two are held to 3 / 16 (LIGHT_MEAN_5BIT), the others to the
                      bounds below. (A rule that took each side's own difference between its two copies of the
                      sun off the second picture's colour was a hidden difference - a black or a noisy second
                      picture passed it - and is gone.)
  implicit textures   texel by texel (the layout is the texture's own), under the lightmap bounds.
  recalculation_data_slots.details   the C++ detail compiler's state for its partial relight, read by nothing but
                      its own next run: not written (the resumed light is not ported).
  level.details       byte for byte (the slots are compared one by one only to say where a difference is).
  deflectors on other pages   with more than one lightmap page a deflector's size follows its light (compress_Zero,
                      compress_RMS shrink a layer whose light varies little), so a light difference moves the
                      deflectors after it to another page and a material on two pages is two visuals (paged): then
                      the static triangles are compared as one multiset (material without the page, every vertex byte
                      but the light and the lightmap coordinates), level.geomx's fast paths as one multiset of
                      triangles by corner positions, the level's header, portals, lights, glows and the sectors'
                      portals byte for byte; the visuals' split, order and spheres, the hierarchy nodes and the
                      sectors' root visual follow the visual list and are left out. Which sector a triangle belongs
                      to (the sector whose visual tree holds its visual, visual_sectors) is part of its key, so a
                      triangle in another sector is one side alone. A progressive visual counts with its whole mesh
                      (its first sliding window; the coarser ones follow how the visuals are split and are held by
                      the windows' own check). The triangles one side alone has - of the static ones, and of the fast
                      paths - no more than twice what the C++ runs' own two have (at least 1e-4 of all): the C++'s
                      progressive meshes differ from run to run in places (mp_close_combat_pages: two wall_farm
                      visuals, one 132 against 216 faces in its whole window; 19 / 31 of 116,757 triangles; the port
                      37 / 49). The light through the matched triangles: vertex light under the vertex bounds,
                      lightmaps through each side's page.
  sliding windows     on every scene and both sides, each side by itself (swi_problems): every progressive
                      visual's windows (OGF_SWIDATA, or an MU model's shared ones through OGF_SWICONTAINER) and every
                      progressive fast path's held to their own index buffer and vertex count - each window's faces
                      inside the visual's indices, every index of a window below its vertex count, that count no
                      more than the visual's, the first window every vertex, no window more faces than the one
                      before. It fails a broken window (a count, an offset or a vertex count changed); it is what
                      holds the windows where the paged rule cannot compare them across sides.
  a refused input     (diff_compile.py REFUSED) XrLC ends through Debug.fatal (exit 3 under -nodlg), the Rust compile
                      with its error exit 1; both name the same number of invalid faces, build.err byte for byte, the
                      Rust compile writes nothing else (the C++ leaves build.lights and the head of level).
  phases              no Rust phase longer than the C++ one by more than 0.25 s (its log is read every 0.1 s); the
                      phases from "LIGHT: Starting MU" up to "Converting MU-models to OGFs" as one span: the C++
                      lights the MU models on a thread beside them, the port inside "Starting MU" on every core.

Recorded (diff_compile.py SCENES), CBuild::Run of cpp-compilers-last phase by phase (the oracle: that tag with
XrLC linked to a 128 MB stack, STATE.md "Closing: the compiler's proof"):
  Load, build.lights, Optimizing (PreOptimize, CorrectTJunctions), Adaptive HT, Building normals (smoothing groups),
  collision database (BuildCForm, QSlim), tangent basis, rcast-CFORM model, Resolving materials, UV mapping,
  Subdividing, LMaps, Vertex light, Merging geometry, Building sectors, Saving: every scene.
  Portals and several sectors: l12u_control_monolith (26 sectors), l03u_agr_underground and its _paged (31).
  Glows: every scene.
  LIGHT: Starting MU / Converting MU-models to OGFs (MU models, references, LODs): l03u_agr_underground (4 px/m, one
    sample, -noise), l03u_agr_underground_paged (6 px/m, one sample, -noise), l03u_agr_underground_own (its own
    settings), mp_close_combat_whole (its own 10
    px/m and nine samples, the static progressive meshes; 29 models, 102 references), mp_close_combat_pages, the
    Draft pair (one MU sample).
  LIGHT: Implicit: mp_close_combat_implicit (one sample, MU models emptied), mp_close_combat_whole (nine samples,
    with its MU models), mp_close_combat_pages (one sample, with them). The nine-sample jitter of the lightmaps:
    mp_test, l12u_control_monolith, mp_close_combat_whole, l03u_agr_underground_own.
  Merging lightmaps: one page (mp_test, l12u, l03u), two (mp_close_combat_implicit, _whole, l03u_agr_underground_
    paged, _own), five (mp_close_combat_pages, 20 px/m). Deflectors on other pages (the paged rule): mp_close_combat_
    implicit, _whole, _pages, l03u_agr_underground_paged and _own (two pages over 31 sectors: the sector in the key).
  Converting to OGFs with progressive static visuals: mp_test, l12u, mp_close_combat_implicit, _whole, _pages,
    l03u_agr_underground_own; with -noise: l03u_agr_underground and _paged.
  The Draft quality (no implicit light, no progressive meshes, one MU sample, no QSlim pass over level.cform):
    mp_close_combat_draft and _draft_nosmg.
  The flags, each where it changes something: -nosun mp_test_nosun (the sun of the page and of level.geom),
    -nosmg mp_test_nosmg (level.cform alone) and mp_close_combat_draft_nosmg (the normals: level.geom, the visuals,
    build.cform), -no_optimize mp_test_no_optimize (no weld: level.geom), -att mp_test_att (linear point lights: the
    page's colour, up to 57 steps), -no_invalidefaces mp_test_invalid_kept (an invalid face kept: build.err, two
    more vertices). The abort on invalid faces: mp_test_invalid (REFUSED). -noise: the l03u scenes.
  The detail compiler: mp_close_combat (the Rust build.cform), mp_close_combat_implicit, _whole, the Draft scene.
  At a scene's own settings with MU models: mp_close_combat_whole and l03u_agr_underground_own (10 px/m, nine
    samples, no -noise: two pages, the static progressive meshes; XrLC 1187 / 1154 s a run).
  Not recorded: a level with implicit light loaded by the game (the game loaded mp_test and l03u_agr_underground;
    the files it reads are the ones compared here). Dropped: -gi, -net (D24).
"""
from __future__ import annotations

import io
import json
import os
import struct
from collections import Counter

import numpy as np

VERSION = 9

# The bounds of the light, per channel in 0..255 steps (mean absolute error, 99th percentile), about twice the largest
# seen over every recorded scene (flat triangles included, every channel as it is):
#   lightmaps and implicit textures  seen mean 0.85, p99 12 (mp_close_combat_implicit, 2.g - the sun the game
#                                    reads), 0.68 / 8 on the first picture (l12u_control_monolith, 1.r); the
#                                    implicit texture 0.02 / 1 (mp_close_combat_implicit)
#   vertex light, MU models' light   seen mean 0.085 (mp_close_combat_implicit, level.geom), p99 1
#                                    (l03u_agr_underground, the LODs of the references lit alone)
#   the detail objects' light        level.details byte for byte on every scene: no bound
# The C++'s own two runs are the floor: zero but on the shrunk (flat) deflectors, where they differ in one texel of
# a thousand or less by up to 127 (l03u, l12u, mp_close_combat_implicit) - most likely the cached triangle of the C++ light
# (xrDeflectorLight.cpp rayTrace, R_Light::tri), which every thread writes. What the bounds allow is the DXT5 encoder
# (NVTT there, texpresso here: a flat 128 decodes as 126 against 128), another ray-triangle test than Opcode's on
# grazing and coincident faces, the transparencies multiplied nearest first, Rust's sin/cos/pow.
LIGHT_MEAN = 1.5
LIGHT_P99 = 16.0
# The second picture's red and blue: DXT5's 5-bit colour channels, where NVTT and texpresso round a flat value two
# steps apart (seen: mean 2.0, p99 2 on mp_test). The game reads that picture's green.
LIGHT_MEAN_5BIT = 3.0
FIVE_BIT = ("2.r", "2.b")
VERTEX_MEAN = 0.2
VERTEX_P99 = 2.0
# An MU reference's light (a tree's c_scale / c_bias through its vertices, a LOD's corner colours): the C++ lights
# the references on four threads that draw their sample directions from the one ::Random (random_dir,
# xrMU_Model_Calc_lighting.cpp), so even a reference whose model no other thread lights takes another sequence
# from run to run; the port seeds each vertex from its position. mp_close_combat_draft, where one sample is the
# whole light: the C++ runs' own difference trees 0.086 / 3, LODs 0.26 / 2; the port trees 0.245 / 5, LODs
# 0.35 / 3 - the bounds below are under twice that (the vertex bounds held a reference only at six samples).
MU_REF_MEAN = 0.45
MU_REF_P99 = 6.0


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
        # the suppress_shadows / suppress_wm bits left out: XrLC never sets them (CollectorPacked::add_face assigns
        # material and sector of a TRI on the stack and leaves those two bits as the stack held them)
        rows.append((tuple(corners), int(t[3]) & ~STRAY_BITS))
    rows.sort()
    return rows


# CDB::TRI's suppress_shadows and suppress_wm (bits 14 and 15 of the triangle's word)
STRAY_BITS = 0xC000


def stray_bits(data: bytes) -> int:
    """The triangles of a level.cform whose word carries a suppress bit."""
    vc, fc = struct.unpack_from("<II", data, 4)
    tris = np.frombuffer(data, dtype="<u4", count=fc * 4, offset=36 + vc * 12).reshape(-1, 4)
    return int(((tris[:, 3] & STRAY_BITS) != 0).sum())


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
    stray = [stray_bits(a), stray_bits(b)]
    if ma == mb:
        return [], "as a multiset of triangles" + (f" (suppress bits on {stray[0]} / {stray[1]} triangles left out)"
                                                    if any(stray) else ""), {}
    # a count that differs is judged against the C++ runs' own (judge)
    problems: list[str] = []
    ca, cb = Counter(ma), Counter(mb)
    only_a, only_b = ca - cb, cb - ca
    words: dict = {}
    for rows, k in ((ma, 0), (mb, 1)):
        for t, w in rows:
            words.setdefault(w, [0.0, 0.0])[k] += tri_area(t)
    stats = {"triangles": len(ma), "count_diff": abs(len(ma) - len(mb)), "one_sided": max(sum(only_a.values()), sum(only_b.values())),
             "area": {str(w): v for w, v in words.items()},
             "moved": sum(abs(x - y) for x, y in words.values()),
             "moved_max": max((abs(x - y) for x, y in words.values()), default=0.0),
             "total": sum(x for x, _ in words.values()), "stray_bits": stray}
    if len(ma) == len(mb) and retriangulated(ma, mb, only_a, only_b):
        return [], "the same surface triangulated another way", stats
    return problems, "the C++ runs' own difference", stats


def plane_key(t) -> tuple:
    """A triangle's plane, its normal turned to a positive largest component, rounded: 1e-3 on the normal, 1 cm on
    the distance."""
    p = np.array(t, dtype=np.float64)
    n = np.cross(p[1] - p[0], p[2] - p[0])
    ln = float(np.linalg.norm(n))
    if ln == 0.0:
        return ()
    n /= ln
    if n[int(np.argmax(np.abs(n)))] < 0:
        n = -n
    return tuple(np.round(n, 3).tolist()) + (round(float(n @ p[0]), 2),)


def retriangulated(ma, mb, only_a: Counter, only_b: Counter) -> bool:
    """The two collision models hold the same vertices, every corner of a triangle one side alone has is a vertex of
    the other side, and per material word and plane the one-sided triangles cover the same area (within 1e-4):
    the same surface, triangulated in another order of QSlim's ties."""
    va = {c for t, _ in ma for c in t}
    vb = {c for t, _ in mb for c in t}
    if va != vb:
        return False
    groups: dict = {}
    for side, only in ((0, only_a), (1, only_b)):
        for (t, w), k in only.items():
            groups.setdefault((w, plane_key(t)), [0.0, 0.0])[side] += k * tri_area(t)
    return all(abs(x - y) <= CFORM_AREA * max(x, y, 1e-9) for x, y in groups.values())


OGF_HEADER = 1


OGF_FASTPATH = 22
OGF_SWIDATA = 6
OGF_SWICONTAINER = 20
OGF_TREEDEF2 = 12
OGF_LODDEF2 = 11
OGF_CHILDREN_L = 10
MT_LOD, MT_TREE_ST, MT_TREE_PM = 6, 7, 11
MU_THREADS = 4


def same_lod_quads(x: bytes, y: bytes) -> bool:
    """OGF_LODDEF2: eight quads of four corners (position, texture coordinates, rgb+hemi, sun, then the struct's three
    bytes of padding - OGF_LOD::_vertex, never set: cpp1 and cpp2 differ there): the corners and coordinates equal."""
    a = np.frombuffer(x, np.uint8).reshape(32, 28)
    b = np.frombuffer(y, np.uint8).reshape(32, 28)
    return bool((a[:, :20] == b[:, :20]).all())


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
    stats = {"visuals": len(va), "equal": 0, "swi_rule": 0, "mu_light": 0, "differ": 0}
    if len(va) != len(vb):
        out.append(f"level: {len(va)} visuals in the C++, {len(vb)} in the Rust")
    for (ia, da), (ib, db) in zip(va, vb):
        if da == db:
            stats["equal"] += 1
            continue
        sa, sb = chunks(da), chunks(db)
        bad, rule = [], "swi_rule"
        if [c for c, _ in sa] != [c for c, _ in sb]:
            bad.append("sub-chunks " + str([c for c, _ in sa]) + " / " + str([c for c, _ in sb]))
        else:
            for (c, x), (_, y) in zip(sa, sb):
                if x == y or (c == OGF_FASTPATH and same_fast_path(x, y)):
                    continue
                # an MU reference's light: cmp_mu judges it
                if (c == OGF_TREEDEF2 and len(x) == len(y) == 104 and x[:64] == y[:64]) or                         (c == OGF_LODDEF2 and len(x) == len(y) == 896 and same_lod_quads(x, y)):
                    rule = "mu_light"
                    continue
                bad.append(f"chunk {c} ({len(x)} / {len(y)} bytes)")
            if not bad:
                stats[rule] += 1
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


FLAT = 1e-9


def flat_samples(p: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """A triangle whose lightmap coordinates collapse to a line or a point (`p` in texel units, texel centres on
    the integers): the texels along its longest side, or the one under it, each with the barycentrics of the point
    that fell into it first. Answers (texels (n, 2), barycentrics (n, 3))."""
    pairs = [(0, 1), (1, 2), (2, 0)]
    i, j = max(pairs, key=lambda e: float(np.linalg.norm(p[e[1]] - p[e[0]])))
    length = float(np.linalg.norm(p[j] - p[i]))
    if length < 1e-6:
        bary = np.full((1, 3), 1.0 / 3.0)
    else:
        # four points per texel the side crosses: every texel it passes through is met
        s = np.linspace(0.0, 1.0, int(np.ceil(length * 4)) + 1)
        bary = np.zeros((len(s), 3))
        bary[:, i] = 1.0 - s
        bary[:, j] = s
    q = bary @ p
    texels = np.floor(q + 0.5).astype(int)
    _, first = np.unique(texels, axis=0, return_index=True)
    first.sort()
    return texels[first], bary[first]


def sample_pages(tris_a, tris_b, pages_a, pages_b):
    """Every texel centre inside each C++ triangle, looked up in the C++ page and, through the same barycentrics, in
    the Rust one; a flat triangle (its lightmap coordinates a line or a point) by the texels along it
    (`flat_samples`). Answers the two value arrays (n, 8), the samples' C++ page positions and which samples came
    from flat triangles."""
    pairs = [(pa, t1, pb, t2) for (pa, ta), (pb, tb) in zip(tris_a, tris_b) if ta.shape == tb.shape
             for t1, t2 in zip(ta, tb)]
    return sample_pairs(pairs, pages_a, pages_b)


def sample_pairs(pairs, pages_a, pages_b):
    """`sample_pages` over matched triangles: (C++ page, its coordinates, other page, its coordinates)."""
    va, vb, where, flats = [], [], [], []
    for pa, t1, pb, t2 in pairs:
        if pa not in pages_a or pb not in pages_b:
            continue
        A, B = pages_a[pa], pages_b[pb]
        h, w = A.shape[:2]
        hb, wb = B.shape[:2]
        p = t1 * np.array([w, h]) - 0.5
        v0, v1 = p[1] - p[0], p[2] - p[0]
        den = v0[0] * v1[1] - v1[0] * v0[1]
        if abs(den) < FLAT:
            pts, bary = flat_samples(p)
            l0, l1, l2 = bary[:, 0], bary[:, 1], bary[:, 2]
            flat = True
        else:
            lo = np.floor(p.min(axis=0)).astype(int)
            hi = np.ceil(p.max(axis=0)).astype(int)
            xs, ys = np.meshgrid(np.arange(lo[0], hi[0] + 1), np.arange(lo[1], hi[1] + 1))
            pts = np.stack([xs.ravel(), ys.ravel()], axis=1).astype(np.float64)
            d = pts - p[0]
            l1 = (d[:, 0] * v1[1] - v1[0] * d[:, 1]) / den
            l2 = (v0[0] * d[:, 1] - d[:, 0] * v0[1]) / den
            l0 = 1 - l1 - l2
            inside = (l0 >= 0) & (l1 >= 0) & (l2 >= 0)
            if not inside.any():
                continue
            pts, l0, l1, l2 = pts[inside].astype(int), l0[inside], l1[inside], l2[inside]
            flat = False
        ok = (pts[:, 0] >= 0) & (pts[:, 0] < w) & (pts[:, 1] >= 0) & (pts[:, 1] < h)
        pts, l0, l1, l2 = pts[ok], l0[ok], l1[ok], l2[ok]
        if not len(pts):
            continue
        q = (t2[0][None] * l0[:, None] + t2[1][None] * l1[:, None] + t2[2][None] * l2[:, None])
        qx = np.clip(np.floor(q[:, 0] * wb).astype(int), 0, wb - 1)
        qy = np.clip(np.floor(q[:, 1] * hb).astype(int), 0, hb - 1)
        va.append(A[pts[:, 1], pts[:, 0]])
        vb.append(B[qy, qx])
        where.append((pa, pts))
        flats.append(np.full(len(pts), flat))
    if not va:
        return np.zeros((0, 8)), np.zeros((0, 8)), [], np.zeros(0, dtype=bool)
    return np.concatenate(va), np.concatenate(vb), where, np.concatenate(flats)


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
    a, b, where, flat = sample_pages(ta, tb, pa, pb)
    return (a, b, flat), "", pa, pb, where


def channel_stats(a: np.ndarray, b: np.ndarray, d: np.ndarray | None = None) -> dict:
    names = ["1.r", "1.g", "1.b", "1.a(sun)", "2.r", "2.g", "2.b", "2.a(hemi)"]
    out = {}
    d = np.abs(a.astype(np.int32) - b.astype(np.int32)) if d is None else d
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


# ---------------------------------------------------------------------------------------- MU models

def mu_refs(level: bytes, geom: bytes) -> list[dict]:
    """The MU references in the order the compiler exports them: each tree visual with its model's vertices and
    indices (level.geom), its sliding windows, its OGF_TREEDEF2 and its LOD's OGF_LODDEF2."""
    buffers, ibs = vbs(geom), ibs_of(geom)
    vis = chunks(chunk_map(level).get(3, b""))
    lod_of: dict = {}
    for _, v in vis:
        s = chunk_map(v)
        h = s.get(OGF_HEADER, b"")
        if len(h) > 1 and h[1] == MT_LOD and OGF_CHILDREN_L in s:
            n = struct.unpack_from("<I", s[OGF_CHILDREN_L], 0)[0]
            for child in struct.unpack_from(f"<{n}I", s[OGF_CHILDREN_L], 4):
                lod_of[child] = s.get(OGF_LODDEF2)
    out = []
    for i, v in vis:
        s = chunk_map(v)
        h = s.get(OGF_HEADER, b"")
        if len(h) < 2 or h[1] not in (MT_TREE_ST, MT_TREE_PM) or 21 not in s:
            continue
        vbid, vs, vc, ibid, is_, ic = struct.unpack_from("<6I", s[21], 0)
        el, raw = buffers[vbid]
        out.append({"visual": i, "model": (vbid, vs, vc), "decl": el, "verts": raw[vs:vs + vc],
                    "idx": ibs[ibid][is_:is_ + ic], "swi": s.get(OGF_SWIDATA), "tree": s.get(OGF_TREEDEF2),
                    "lod": lod_of.get(i)})
    return out


def raced(refs: list[dict]) -> list[bool]:
    """Whether the C++ lights a reference while another reference of its model may be lit on another thread: CMUThread
    gives every one of four threads `n / 4` references in order and a fifth the rest (get_intervals)."""
    n = len(refs)
    stride = n // MU_THREADS if n >= MU_THREADS else 1
    thread = [min(k // stride, MU_THREADS) if n >= MU_THREADS else k for k in range(n)]
    return [any(refs[j]["model"] == r["model"] and thread[j] != thread[k] for j in range(n) if j != k)
            for k, r in enumerate(refs)]


def tree_lit(r: dict) -> np.ndarray:
    """Every vertex of a reference lit as the engine lights a tree: the model's hemisphere (the normal's w) times
    c_scale plus c_bias, per channel r, g, b, hemi, sun, in 0..255 steps."""
    hemi = r["verts"][:, 15].astype(np.float64) / 255.0
    f = np.frombuffer(r["tree"], "<f4").astype(np.float64)
    return (hemi[:, None] * f[None, 16:21] + f[None, 21:26]) * 255.0


def lod_colours(r: dict) -> np.ndarray:
    """A LOD's 32 corners: r, g, b, hemi, sun."""
    return np.frombuffer(r["lod"], np.uint8).reshape(32, 28)[:, 20:25].astype(np.int32)


def cmp_mu(la: bytes, ga: bytes, lb: bytes, gb: bytes, out: list[str]) -> dict:
    """The MU references of two outputs: their models' geometry and their light (see the rules above)."""
    ra, rb = mu_refs(la, ga), mu_refs(lb, gb)
    if len(ra) != len(rb):
        out.append(f"MU: {len(ra)} / {len(rb)} references")
        return {}
    race = raced(ra)
    stats: dict = {"references": len(ra), "models": len({r["model"] for r in ra}), "raced": int(sum(race))}
    found: dict = {"model": [], "tree": [], "lod": [], "tree_raced": [], "lod_raced": []}
    seen = set()
    for k, (a, b) in enumerate(zip(ra, rb)):
        if a["model"] != b["model"] or a["decl"] != b["decl"] or a["verts"].shape != b["verts"].shape:
            out.append(f"MU: reference {k} has another model or vertex format")
            continue
        cols = light_columns(a["decl"])
        rest = [i for i in range(a["verts"].shape[1]) if i not in cols]
        if a["model"] not in seen:
            # the model's own light, once per model
            seen.add(a["model"])
            found["model"].append(a["verts"][:, cols].astype(np.int32) - b["verts"][:, cols].astype(np.int32))
        if (a["verts"][:, rest] != b["verts"][:, rest]).any() or not np.array_equal(a["idx"], b["idx"]) \
                or a["swi"] != b["swi"]:
            out.append(f"MU: reference {k}: its model's geometry differs")
        if (a["tree"] is None) != (b["tree"] is None) or (a["lod"] is None) != (b["lod"] is None):
            out.append(f"MU: reference {k}: a tree or LOD definition on one side only")
            continue
        tag = "_raced" if race[k] else ""
        if a["tree"] is not None:
            if a["tree"][:64] != b["tree"][:64]:
                out.append(f"MU: reference {k}: its xform differs")
            found["tree" + tag].append(tree_lit(a) - tree_lit(b))
        if a["lod"] is not None:
            if not same_lod_quads(a["lod"], b["lod"]):
                out.append(f"MU: reference {k}: its LOD quads differ")
            found["lod" + tag].append(lod_colours(a) - lod_colours(b))
    for name, parts in found.items():
        if parts:
            m, p, x = err_stats(np.concatenate([d.ravel() for d in parts]))
            stats[name] = {"mean": m, "p99": p, "max": x}
    return stats


# ---------------------------------------------------------------------------------------- the whole

# ---------------------------------------------------------------------------------------- deflectors on other pages

def shader_names(level: bytes) -> list[str]:
    sh = chunk_map(level).get(2, b"")
    return [s.decode("cp1251", "replace") for s in sh[4:].split(b"\0")]


def without_page(name: str) -> str:
    return ",".join(p for p in name.split(",") if not p.startswith("lmap#"))


def paged(la: bytes, lb: bytes) -> bool:
    """The two levels hold the same materials but deflectors of some on other lightmap pages: a deflector's size
    follows its light (compress_Zero / compress_RMS of xrDeflectorLight.cpp shrink a layer whose light varies little),
    so with more than one page a light difference moves the deflectors after it to the next page, and a material on
    two pages is two visuals. Then the static geometry is compared as triangles (`paged_compare`). The shader lists
    may be equal - every material on both pages on both sides - while the visuals of a material are split another
    way between the pages: the visuals' shader sequence tells."""
    na, nb = shader_names(la), shader_names(lb)
    pages = {p for n in na + nb for p in n.split(",") if p.startswith("lmap#")}
    if len(pages) <= 2 or {without_page(n) for n in na} != {without_page(n) for n in nb}:
        return False
    if na != nb:
        return True

    def sequence(level: bytes) -> list:
        return [chunk_map(v).get(OGF_HEADER, b"")[:4] for _, v in chunks(chunk_map(level).get(3, b""))]
    return sequence(la) != sequence(lb)


FSP_ROOT = 2


def visual_sectors(level: bytes) -> list:
    """Per visual the sector whose visual tree holds it (the sector's root visual, fsP_Root, and every visual below
    it through OGF_CHILDREN_L), None for a visual no sector reaches."""
    vis = [chunk_map(v) for _, v in chunks(chunk_map(level).get(3, b""))]
    out: list = [None] * len(vis)
    for sid, body in chunks(chunk_map(level).get(8, b"")):
        root = chunk_map(body).get(FSP_ROOT)
        if not root or len(root) < 4:
            continue
        todo = [struct.unpack_from("<I", root, 0)[0]]
        while todo:
            i = todo.pop()
            if i >= len(vis) or out[i] is not None:
                continue
            out[i] = sid
            kids = vis[i].get(OGF_CHILDREN_L)
            if kids and len(kids) >= 4:
                n = struct.unpack_from("<I", kids, 0)[0]
                todo += list(struct.unpack_from(f"<{n}I", kids, 4))
    return out


def swi_shared(data: bytes) -> list[bytes]:
    """A geometry file's chunk 11: the sliding windows MU references share (OGF_SWICONTAINER names one), each as
    OGF_SWIDATA's body."""
    c = chunk_map(data).get(11, b"")
    out, at = [], 4
    for _ in range(struct.unpack_from("<I", c, 0)[0] if len(c) >= 4 else 0):
        n = struct.unpack_from("<I", c, at + 16)[0]
        out.append(c[at:at + 20 + 8 * n])
        at += 20 + 8 * n
    return out


def full_window(sub: dict, shared: list[bytes], ic: int) -> tuple[int, int]:
    """The indices of a visual's whole mesh, as (start, count) inside its container: a progressive visual's index
    range holds every sliding window's faces - the coarser levels too, which follow how the visuals are split -
    and its first window is the whole mesh; any other visual is all of its indices."""
    sw = sub.get(OGF_SWIDATA)
    if not sw and sub.get(OGF_SWICONTAINER):
        sid = struct.unpack_from("<I", sub[OGF_SWICONTAINER], 0)[0]
        sw = shared[sid] if sid < len(shared) else None
    if not sw or len(sw) < 28 or struct.unpack_from("<I", sw, 16)[0] == 0:
        return 0, ic
    off, tris, _ = struct.unpack_from("<IHH", sw, 20)
    return off, 3 * tris


def swi_problems(level: bytes, geom: bytes, geomx: bytes | None, side: str) -> tuple[list[str], int]:
    """Every progressive visual's sliding windows (OGF_SWIDATA: count, then offset, faces, vertices) held to its own
    index buffer and vertex count - its own geometry in level.geom, its fast path's in level.geomx: each window's
    faces inside the visual's indices, every index of a window below the window's vertex count, that count no more
    than the visual's vertices, the first window every vertex, a window never more faces than the one before.
    A check of each side by itself: it holds where the visuals cannot be compared across sides. Answers the
    problems and the number of windows checked."""
    bad: list[str] = []
    checked = 0

    gb = (vbs(geom), ibs_of(geom), swi_shared(geom))
    gx = (vbs(geomx), ibs_of(geomx), swi_shared(geomx)) if geomx else None
    for k, (_, v) in enumerate(chunks(chunk_map(level).get(3, b""))):
        sub = chunk_map(v)
        parts = [(sub, gb, "")]
        fp = sub.get(OGF_FASTPATH)
        if fp and gx:
            parts.append((chunk_map(fp), gx, " fast path"))
        for s, (buffers, ibs, swis), what in parts:
            sw, g = s.get(OGF_SWIDATA), s.get(21)
            if not sw and s.get(OGF_SWICONTAINER):
                sid = struct.unpack_from("<I", s[OGF_SWICONTAINER], 0)[0]
                if sid >= len(swis):
                    bad.append(f"{side}: visual {k}{what}: sliding windows {sid} of {len(swis)}")
                    continue
                sw = swis[sid]
            if not sw or not g:
                continue
            vb, vs, vc, ib, is_, ic = struct.unpack_from("<6I", g, 0)
            if ib >= len(ibs) or is_ + ic > len(ibs[ib]):
                bad.append(f"{side}: visual {k}{what}: its container lies outside the index buffers")
                continue
            idx = ibs[ib][is_:is_ + ic].astype(np.int64)
            n = struct.unpack_from("<I", sw, 16)[0]
            if len(sw) != 20 + 8 * n or n == 0:
                bad.append(f"{side}: visual {k}{what}: {len(sw)} bytes of sliding windows for {n}")
                continue
            rows = [struct.unpack_from("<IHH", sw, 20 + 8 * i) for i in range(n)]
            prev = None
            for w, (off, tris, verts) in enumerate(rows):
                checked += 1
                if off % 3 or off + 3 * tris > ic or verts > vc or (prev is not None and tris > prev):
                    bad.append(f"{side}: visual {k}{what}: window {w} (offset {off}, {tris} faces, {verts} vertices) "
                               f"against {ic} indices, {vc} vertices")
                    break
                if tris and int(idx[off:off + 3 * tris].max()) >= verts:
                    bad.append(f"{side}: visual {k}{what}: window {w} indexes past its {verts} vertices")
                    break
                prev = tris
            if rows[0][2] != vc:
                bad.append(f"{side}: visual {k}{what}: the first window has {rows[0][2]} of {vc} vertices")
    return bad, checked


def static_triangles(level: bytes, geom: bytes) -> list[dict]:
    """Every triangle of every visual with a geometry container (static and MU visuals; a progressive visual's whole
    mesh, its first sliding window - the coarser ones are held by swi_problems): its key - the sector the
    visual belongs to, the material without the page, the corners' vertex bytes but the light and the lightmap
    coordinates - the corners' light bytes, and for a lightmapped one its page and lightmap coordinates (3, 2)."""
    names = shader_names(level)
    buffers, ibs = vbs(geom), ibs_of(geom)
    shared = swi_shared(geom)
    sectors = visual_sectors(level)
    out = []
    for vi, (_, v) in enumerate(chunks(chunk_map(level).get(3, b""))):
        sub = chunk_map(v)
        h, g = sub.get(1), sub.get(21)
        if not h or not g:
            continue
        sid = struct.unpack_from("<H", h, 2)[0]
        name = names[sid] if sid < len(names) else ""
        parts = name.split("/", 1)[-1].split(",")
        page = next((t[:-2] for t in parts if t.startswith("lmap#") and t.endswith("_1")), None)
        vb, vs, vc, ib, is_, ic = struct.unpack_from("<6I", g, 0)
        el, raw = buffers[vb]
        cols = light_columns(el)
        lm = next((o for o, t, u, i in el if u == 5 and i == 1), None)
        skip = set(cols) | (set(range(lm, lm + 4)) if lm is not None else set())
        rest = [i for i in range(raw.shape[1]) if i not in skip]
        verts = raw[vs:vs + vc]
        off, n = full_window(sub, shared, ic)
        idx = ibs[ib][is_ + off:is_ + off + n].reshape(-1, 3)
        uv = (verts[:, lm:lm + 4].copy().view("<i2").reshape(-1, 2).astype(np.float64) / 32768.0
              if lm is not None and page else None)
        material = without_page(name)
        for t in idx:
            out.append({"key": (sectors[vi], material) + tuple(verts[k, rest].tobytes() for k in t),
                        "light": verts[t][:, cols].astype(np.int32) if cols else None,
                        "page": page, "uv": uv[t] if uv is not None else None})
    return out


def paged_compare(la: bytes, ga: bytes, lb: bytes, gb: bytes, xa: bytes, xb: bytes, pr: list[str]) -> dict:
    """The static geometry of two levels whose deflectors sit on other pages: the triangles as a multiset (material,
    vertices), so the visuals they are split into and their order are left out, and so are the visual tree's
    hierarchy nodes and the sectors' root visual (both follow the visual list) and the bounding spheres of the visuals
    (each of its own triangles); level.geomx's fast paths as one multiset of triangles by corner positions. The
    triangles are matched by their key: their vertex light under the vertex bounds, their lightmap texels through
    each side's own page and coordinates. Answers the stats and the matched lightmapped triangles."""
    ta, tb = static_triangles(la, ga), static_triangles(lb, gb)
    ca, cb = Counter(t["key"] for t in ta), Counter(t["key"] for t in tb)
    # judged against the C++ runs' own (judge): their progressive meshes differ from run to run here and there
    one_sided = [sum((ca - cb).values()), sum((cb - ca).values())]
    by_key: dict = {}
    for t in tb:
        by_key.setdefault(t["key"], []).append(t)
    light, pairs = [], []
    for t in ta:
        m = by_key.get(t["key"])
        if not m:
            continue
        u = m.pop(0)
        if t["light"] is not None and u["light"] is not None and t["light"].shape == u["light"].shape:
            light.append(t["light"] - u["light"])
        if t["page"] and u["page"]:
            pairs.append((t["page"], t["uv"], u["page"], u["uv"]))
    stats: dict = {"triangles": len(ta), "visuals": [len(chunks(chunk_map(x).get(3, b""))) for x in (la, lb)],
                   "moved_pages": sum(1 for p, _, q, _ in pairs if p != q), "one_sided": one_sided}
    if light:
        m, p, x = err_stats(np.concatenate([d.ravel() for d in light]))
        stats["vertex"] = {"mean": m, "p99": p, "max": x}
    # the fast paths: every triangle by its corner positions
    def fast(level: bytes, geomx: bytes) -> Counter:
        buffers, ibs, shared = vbs(geomx), ibs_of(geomx), swi_shared(geomx)
        out: Counter = Counter()
        for _, v in chunks(chunk_map(level).get(3, b"")):
            fp = chunk_map(v).get(OGF_FASTPATH)
            fs = chunk_map(fp) if fp else {}
            if 21 not in fs:
                continue
            vb, vs, vc, ib, is_, ic = struct.unpack_from("<6I", fs[21], 0)
            off, n = full_window(fs, shared, ic)
            pos = buffers[vb][1][vs:vs + vc].copy().view("<f4").reshape(-1, 3)
            for t in ibs[ib][is_ + off:is_ + off + n].reshape(-1, 3):
                out[tuple(sorted(tuple(pos[k].tolist()) for k in t))] += 1
        return out
    fa, fb = fast(la, xa), fast(lb, xb)
    stats["fast_one_sided"] = [sum((fa - fb).values()), sum((fb - fa).values())]
    # the level's chunks that hold no visual index
    ma, mb = chunk_map(la), chunk_map(lb)
    for cid, what in ((1, "header"), (4, "portals"), (6, "lights"), (7, "glows")):
        if ma.get(cid) != mb.get(cid):
            pr.append(f"level: chunk {cid} {what} differs")
    sa, sb = chunk_map(ma.get(8, b"")), chunk_map(mb.get(8, b""))
    if len(sa) != len(sb) or any(chunk_map(sa[k]).get(1) != chunk_map(sb.get(k, b"")).get(1) for k in sa):
        pr.append("level: the sectors' portals differ")
    return {"stats": stats, "pairs": pairs}


def err_file(d: str) -> bytes | None:
    """The invalid-face, T-junction and multiple-edge report of an output: the Rust build.err, or the C++
    build_<user>.err its log folder held."""
    names = sorted(n for n in os.listdir(d) if n.lower().startswith("_log_build_") and n.lower().endswith(".err"))
    return read(d, "build.err") if os.path.isfile(os.path.join(d, "build.err")) else (read(d, names[0]) if names else None)


def compare_dirs(da: str, db: str, files_a: set, files_b: set, out_dir: str | None) -> dict:
    """One C++ output against another output (the Rust one, or the second C++ run)."""
    res: dict = {"problems": [], "rules": {}, "light": {}, "vertex": {}, "details": {}, "mu": {}}
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
    both = files_a & files_b
    # build.err: the C++ writes it into its logs (build_<user>.err, kept as _log_build_<user>.err beside its output),
    # the Rust compiler beside the level's files
    ea, eb = err_file(da), err_file(db)
    if ea is not None or eb is not None:
        if ea is None or eb is None:
            pr.append("build.err: written by one compiler only")
        elif ea != eb:
            pr.append(f"build.err differs ({len(ea)} / {len(eb)} bytes)")
        else:
            res["rules"]["build.err"] = "byte for byte"
    is_paged = {"level", "level.geom", "level.geomx"} <= both and paged(read(da, "level"), read(db, "level"))
    # every progressive visual's sliding windows against its own buffers, on each side by itself
    if {"level", "level.geom"} <= both:
        windows = []
        for side, d in (("C++", da), ("other", db)):
            p, n = swi_problems(read(d, "level"), read(d, "level.geom"), read(d, "level.geomx"), side)
            pr += p[:8]
            windows.append(n)
        res["rules"]["sliding windows"] = f"{windows[0]} / {windows[1]} held to their own buffers"
    for n in sorted(both):
        a, b = read(da, n), read(db, n)
        if is_paged and n in ("level", "level.geom", "level.geomx"):
            continue
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
            # byte for byte: both detail compilers wrote the same file on every recorded scene; the slots are
            # compared one by one only to say where they differ
            res["details"] = cmp_details(a, b, pr)
            if a != b:
                pr.append(f"level.details differs ({len(a)} / {len(b)} bytes)")
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
        res["mu"] = cmp_mu(read(da, "level"), read(da, "level.geom"), read(db, "level"), read(db, "level.geom"), pr)
        if is_paged:
            pg = paged_compare(read(da, "level"), read(da, "level.geom"), read(db, "level"), read(db, "level.geom"),
                               read(da, "level.geomx"), read(db, "level.geomx"), pr)
            res["rules"]["visuals"] = "deflectors on other pages: the static triangles as a multiset"
            res["rules"]["paged"] = pg["stats"]
            if "vertex" in pg["stats"]:
                res["vertex"]["level.geom"] = pg["stats"]["vertex"]
            pa, pb = load_pages(da), load_pages(db)
            a, b, where, flat = sample_pairs(pg["pairs"], pa, pb)
            got, why = (a, b, flat), ""
        else:
            got, why, pa, pb, where = light_compare(da, db)
        if got is None:
            pr.append("lightmaps: " + why)
        else:
            a, b, flat = got
            res["light"]["lightmaps"] = channel_stats(a, b)
            res["light"]["samples"] = int(len(a))
            res["light"]["flat_samples"] = int(flat.sum())
            # reported on their own too; judged with the rest
            res["light"]["lightmaps_flat"] = channel_stats(a[flat], b[flat])
            res["light"]["pages"] = len(pa)
            if sorted(pa) != sorted(pb):
                pr.append(f"lightmap pages: {sorted(pa)} in the C++, {sorted(pb)} in the other")
            if out_dir:
                res["pictures"] = save_pictures(out_dir, pa, pb, where, a, b)
    return res


CFORM_AREA = 1e-4
# Another tie order of QSlim may differ from cpp1 twice as far as cpp2 does (l03u_agr_underground_own).
CFORM_FLOOR = 2.0
PAGED_FLOOR = 2.0


def judge(res: dict, floor: dict) -> list[str]:
    bad = []
    # QSlim re-rates the edges around a contraction in the order of their heap addresses (MxQSlim.cpp,
    # apply_contraction), so ties fall another way from run to run: the counts and areas are judged against twice
    # what the two C++ runs differ in (CFORM_FLOOR), or the small absolute floors
    cf = res.get("cform") or {}
    if cf and res["rules"].get("level.cform") != "the same surface triangulated another way":
        fl = floor.get("cform") or {}
        # three draws of QSlim's ties, and the port's is one of them
        allowed = max(int(CFORM_FLOOR * fl.get("one_sided", 0)), -(-cf["triangles"] // 10000))
        count_allowed = max(int(CFORM_FLOOR * fl.get("count_diff", 0)), -(-cf["triangles"] // 10000))
        if cf.get("count_diff", 0) > count_allowed:
            bad.append(f"level.cform: {cf['count_diff']} triangles more on one side (allowed {count_allowed})")
        if cf["one_sided"] > allowed:
            bad.append(f"level.cform: {cf['one_sided']} triangles one side alone has (allowed {allowed})")
        word_allowed = CFORM_FLOOR * fl.get("moved_max", 0.0)
        for w, (x, y) in cf["area"].items():
            if abs(x - y) > max(CFORM_AREA * max(x, 1e-6), word_allowed):
                bad.append(f"level.cform: material word {w} has the area {y:.6f} against {x:.6f} "
                           f"(allowed {max(CFORM_AREA * max(x, 1e-6), word_allowed):.6f})")
        moved_allowed = max(1e-6 * cf.get("total", 0.0), CFORM_FLOOR * fl.get("moved", 0.0))
        if cf.get("moved", 0.0) > moved_allowed:
            bad.append(f"level.cform: {cf['moved']:.6f} m2 changed material (allowed {moved_allowed:.6f})")
    # the paged rule's triangles one side alone (whole meshes, the fast paths by corner positions): no more than
    # twice the C++ runs' own, at least 1e-4 of all. The C++'s progressive meshes differ from run to run in places
    # (mp_close_combat_pages: two wall_farm visuals, 19 / 31 triangles of 116,757; the port 37 / 49)
    pg = res["rules"].get("paged") if isinstance(res["rules"].get("paged"), dict) else None
    if pg:
        fpg = floor["rules"].get("paged") if isinstance(floor["rules"].get("paged"), dict) else {}
        for what, key in (("static triangles", "one_sided"), ("fast-path triangles", "fast_one_sided")):
            allowed = max(int(PAGED_FLOOR * max(fpg.get(key, [0, 0]))), -(-pg["triangles"] // 10000))
            if max(pg.get(key, [0, 0])) > allowed:
                bad.append(f"level: {pg[key]} {what} one side alone has (allowed {allowed})")
    mu = res.get("mu") or {}
    for name in ("model", "tree", "lod"):
        s = mu.get(name)
        mb, pb = (VERTEX_MEAN, VERTEX_P99) if name == "model" else (MU_REF_MEAN, MU_REF_P99)
        if s and (s["mean"] > mb or s["p99"] > pb):
            bad.append(f"MU {name} light: mean {s['mean']:.2f}, p99 {s['p99']:.1f} (bounds {mb}, {pb})")
    lm = res["light"].get("lightmaps", {})
    for ch, s in lm.items():
        mean_bound = LIGHT_MEAN_5BIT if ch in FIVE_BIT else LIGHT_MEAN
        if isinstance(s, dict) and (s["mean"] > mean_bound or s["p99"] > LIGHT_P99):
            bad.append(f"lightmap channel {ch}: mean {s['mean']:.2f}, p99 {s['p99']:.1f} (bounds {mean_bound}, {LIGHT_P99})")
    for n, s in res["vertex"].items():
        if s["mean"] > VERTEX_MEAN or s["p99"] > VERTEX_P99:
            bad.append(f"{n} vertex light: mean {s['mean']:.2f}, p99 {s['p99']:.1f}")
    for n, s in res["light"].items():
        if n.endswith(".dds") and (s["mean"] > LIGHT_MEAN or s["p99"] > LIGHT_P99):
            bad.append(f"{n}: mean {s['mean']:.2f}, p99 {s['p99']:.1f}")
    return bad


PHASE_SLACK = 0.25
MU_FROM, MU_TO = "LIGHT: Starting MU...", "Converting MU-models to OGFs..."


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
    cs = spans(c, common)
    # The C++ lights the MU models on a thread of its own from "LIGHT: Starting MU" on, beside the phases that
    # follow, and waits for it after "Converting to OGFs"; the port lights them inside "LIGHT: Starting MU" on
    # every core. So the phases from there up to "Converting MU-models to OGFs" are judged as one span.
    names = [n for n, _, _ in cs]
    mu_span: set = set()
    if MU_FROM in names and MU_TO in names and names.index(MU_FROM) < names.index(MU_TO):
        mu_span = set(names[names.index(MU_FROM):names.index(MU_TO)])
    rows, slower = [], []
    for n, t, l in cs:
        rt, rl = rs.get(n, (0.0, 0.0))
        rows.append(f"{n[:36]:36} {t:8.2f} {l:5.1f} {rt:8.2f} {rl:5.1f}")
        if n not in mu_span and rt > t + PHASE_SLACK:
            slower.append(f"phase {n}: Rust {rt:.2f} s, C++ {t:.2f} s")
    if mu_span:
        ct = sum(t for n, t, _ in cs if n in mu_span)
        rt = sum(rs.get(n, (0.0, 0.0))[0] for n in mu_span)
        rows.append(f"{'(the MU light and the phases beside it)':36} {ct:8.2f} {'':5} {rt:8.2f}")
        if rt > ct + PHASE_SLACK:
            slower.append(f"the MU light and the phases beside it: Rust {rt:.2f} s, C++ {ct:.2f} s")
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


def refusal(lines, pattern: str) -> int | None:
    """The invalid-face count a refusing compiler's log names (`pattern` its sentence before the count)."""
    for _, l in lines:
        text = l if isinstance(l, str) else str(l)
        if pattern in text:
            rest = text.split(pattern, 1)[1].strip().split()
            if rest and rest[0].isdigit():
                return int(rest[0])
    return None


def compare_refused(rd, rec, rust_dir, rrec, out, log) -> bool:
    """A scene both compilers must refuse: XrLC ends through Debug.fatal (exit 3 under -nodlg), the Rust compile
    with its error exit 1; both name the same number of invalid faces; build.err byte for byte (cpp1, cpp2, Rust);
    the Rust compile writes nothing else. The C++'s build.lights, written at the start of CBuild::Run before the
    check, is reported."""
    bad = []
    counts = {}
    for tag in ("cpp1", "cpp2"):
        r = rec[tag]["runs"].get("XrLC.exe", {})
        counts[tag] = refusal(r.get("lines", []), "* FATAL:")
        if r.get("exit") != 3:
            bad.append(f"{tag}: XrLC exit {r.get('exit')}")
    rr = rrec["runs"].get("-compile", {})
    counts["rust"] = None
    for _, l in rr.get("lines", []):
        if "invalid faces. Compilation aborted" in l:
            words = l.split("invalid faces")[0].split()
            counts["rust"] = int(words[-1]) if words and words[-1].isdigit() else None
    if rr.get("exit") != 1:
        bad.append(f"Rust: exit {rr.get('exit')} (the refusal is 1)")
    if len(set(counts.values())) != 1 or None in counts.values():
        bad.append(f"the invalid faces named: {counts}")
    errs = {tag: err_file(os.path.join(rd, tag)) for tag in ("cpp1", "cpp2")}
    errs["rust"] = err_file(rust_dir)
    if None in errs.values() or len(set(errs.values())) != 1:
        bad.append("build.err differs: " + ", ".join(f"{k} {len(v) if v else None} bytes" for k, v in errs.items()))
    rust_files = sorted(rrec["files"])
    if rust_files != ["build.err"]:
        bad.append(f"the Rust compile wrote {rust_files} (only build.err)")
    cpp_files = sorted(rec["cpp1"]["files"])
    res = {"counts": counts, "build.err": len(errs["rust"] or b""), "cpp_files": cpp_files, "rust_files": rust_files,
           "judged": bad}
    with open(os.path.join(out, "compare.json"), "w", encoding="utf-8") as f:
        json.dump(res, f, indent=1)
    log(f"  refused: invalid faces {counts}, build.err {res['build.err']} bytes on every side, "
        f"exits C++ {[rec[t]['runs']['XrLC.exe']['exit'] for t in ('cpp1', 'cpp2')]} / Rust {rr.get('exit')}; "
        f"the C++ left {cpp_files}, the Rust {rust_files}")
    for p in bad:
        log("  ! " + p)
    return not bad


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
    if res.get("mu"):
        log(f"  MU: {json.dumps(res['mu'])}  floor MU: {json.dumps(floor.get('mu'))}")
    for row in res["phases"]:
        log("  " + row)
    return not res["problems"] and not res["judged"]
