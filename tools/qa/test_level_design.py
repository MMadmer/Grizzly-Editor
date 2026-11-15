#!/usr/bin/env python3
"""Level design by an agent (docs/LEVEL_DESIGN.md), over MCP on a THROW-AWAY COPY of a project with the Escape scene
(rawdata\\levels\\l01_escape.level, opened as the other scene suites open it and saved as its .scene). Every
phase works on the open ground east of the village, round the yard of the worked example:

  A  scene.query: ground (one point, a list, `top` over a tree, the surface named), heights (the grid's shape, a
     sample is the ground there, a cap refused), ray (one hit at the ground, `all` through a tree), box, fit (blocked
     by a tree; free, and `rest` stands the model on the ground: a second ground says so), free (every spot passes
     fit), ai (on and off the map, a walk); scene.objects with bounds, near and sort
  B  the files: a line appended to objects.lua is in the scene; objects.lua broken (a brace taken out) changes
     nothing - every object still there, the problem at the line, asset.check names it, scene.save answers
     `skipped` and leaves the file - and the file fixed is followed again; a group written with `ref` and no
     `members` is placed from its asset and saved with its members
  C  scene.align drop, and drop with `offset`, then scene.save: only the two changed lines of objects.lua differ
  D  scene.audit: written 3 m up is S201, two written into each other S203, one written twice S204, one not marked
     authored S210 (the level's own objects known from its X-Ray SDK scene); each problem at the object's file and
     line; fixed, the audit is empty
  E  models: a glTF written as text (a base64 buffer) and an OBJ imported with asset.import, their assets and the
     texture asset of a picture made; model.info answers their sizes and surfaces; a blockout written as `parts`;
     both placed in objects.lua by their project paths and found by scene.query box and a ray; file.create makes
     the three kinds; preview.model path= answers the asset's box
  F  pictures: shot.map in its three modes (the PNG is the size answered; px_per_m and map carry a legend centre
     to its pixel; highlight changes the picture; the same call gives the same bytes), shot.viewport with a view and
     with a camera of its own (the author's camera stays where it was), shot.models
  G  terrain: terrain.lua written over the slope (`base = "ground"`, a `rect`, `cut = true`): heights inside are
     the flattened height, outside the old ground; a ray from above meets the terrain and not the old ground;
     no S211 - raised 3 m by `base`, S211
  H  the build, on the game's level with a game linked: level.build runs the piece step and writes overlay.cut and
     overlay.aiedit; level.inspect cuts (one region, the terrain's centre and size), ai_edits, pieces (the piece
     carries the cut); mod.build into a folder publishes the texture asset (the .dds and the .texasset under
     gamedata\\textures, no source)

    python tools/qa/test_level_design.py --project D:\\XFinedProjects\\Test [--only A,G] [--shots <folder>]
"""
from __future__ import annotations

import base64
import io
import json
import math
import os
import re
import struct
import sys
import time
import zlib

import qa
from qa import check, cmd, short

PHASES = "ABCDEFGH"
SCENE = "rawdata/levels/l01_escape.scene"
LEVEL = "rawdata/levels/l01_escape.level"
GROUND = "scenes\\escape\\escape_terrain"
TREE = "trees\\new_trees\\trees_topol_1_01_0010"
TREE_AT = (-108.94, -150.27)
BARREL = "statics\\barrel\\bochka_wood1"
CABINET = "statics\\shkaf\\cabinet_village_140x210_ph"
# the yard: a rectangle 16 x 10 m on the slope, flattened inside +-5 x +-3 m
YARD = (-106.0, -164.5)
YARD_SIZE = (16.0, 10.0)
FLAT = -12.7
TERRAIN_NAME = "yard_ground"
SHOTS: str | None = None


class Ctx:
    project = ""
    work = ""
    module = "test"


CTX = Ctx()


# ---- the scene's files ----------------------------------------------------------------------------------------------

def scene_file(name: str) -> str:
    return os.path.join(CTX.project, "rawdata", "levels", "l01_escape", name)


def read(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()


def write(path: str, text: str) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def lua(s: str) -> str:
    return s.replace("\\", "\\\\")


def row(name: str, ref: str, pos, authored: bool = True, rot=None) -> str:
    extra = ", rot = { %g, %g, %g }" % tuple(rot) if rot else ""
    return '    { name = "%s", ref = "%s", pos = { %g, %g, %g }%s%s },' % (
        name, lua(ref), pos[0], pos[1], pos[2], extra, ", authored = true" if authored else "")


def append_objects(rows: list[str]) -> None:
    """Rows written at the end of objects.lua's list, as another program would."""
    text = read(scene_file("objects.lua")).rstrip()
    cut = text.rfind("  },")
    write(scene_file("objects.lua"), text[:cut] + "\n".join(rows) + "\n" + text[cut:] + "\n")


def line_of(text: str, name: str) -> int:
    """The 1-based line that names `name`, 0 when none does."""
    return next((i + 1 for i, l in enumerate(text.split("\n")) if 'name = "%s"' % lua(name) in l), 0)


def names() -> set[str]:
    out: set[str] = set()
    offset = 0
    while True:
        page = cmd("scene.objects", limit=2000, offset=offset)
        out |= {o["name"] for o in page.get("objects") or []}
        offset += 2000
        if offset >= page.get("total", 0):
            return out


def present(name: str) -> bool:
    return any(o.get("name") == name for o in cmd("scene.objects", query=name).get("objects") or [])


def wait_for(name: str, there: bool = True, seconds: float = 15.0) -> bool:
    return bool(qa.until(lambda: present(name) == there, seconds, 0.2))


def get(name: str) -> dict:
    return (cmd("scene.get", objects=[name], keys=["pos"]).get("objects") or [{}])[0]


def floor(x: float, z: float, **kw) -> float | None:
    return cmd("scene.query", action="ground", position=[x, 0.0, z], top=True, **kw).get("floor")


def near(a, b, tol: float) -> bool:
    return a is not None and b is not None and abs(float(a) - float(b)) <= tol


def problems_of(file_part: str) -> list[dict]:
    return [p for p in cmd("scene.info").get("problems") or [] if file_part in str(p.get("file"))]


def stalls_since(what: str) -> None:
    st = cmd("perf.stalls", clear=True)
    check(st.get("ok") and not st.get("stalls"), "%s holds the UI thread under %s ms" % (what, st.get("limit_ms")),
          short(st.get("stalls")), timing=True)


# ---- A: the scene answers in numbers --------------------------------------------------------------------------------

def phase_a() -> None:
    x, z = YARD
    one = cmd("scene.query", action="ground", position=[x, 0.0, z], top=True)
    n = one.get("normal") or [0, 0, 0]
    surface = one.get("surface") or {}
    check(one.get("ok") and isinstance(one.get("floor"), float) and one.get("object") == GROUND
          and abs(math.sqrt(sum(c * c for c in n)) - 1) < 0.01 and 0 <= (one.get("slope") or -1) < 30
          and surface.get("texture") and surface.get("material"),
          "A ground: the floor of the yard (%.2f m), its normal, slope %.1f degrees, the object and its surface %s"
          % (one.get("floor") or 0, one.get("slope") or 0, surface.get("texture")), short(one))
    pts = [[x, 0.0, z], [x + 4, 0.0, z + 2], [x - 4, 0.0, z - 2]]
    many = cmd("scene.query", action="ground", positions=pts, top=True).get("points") or []
    check(len(many) == 3 and [p.get("position") for p in many] == pts and near(many[0].get("floor"), one.get("floor"), 0.001)
          and all(p.get("object") == GROUND for p in many),
          "A ground with a list: one answer per point, the first the same as the single one", short(many))
    tx, tz = TREE_AT
    top = cmd("scene.query", action="ground", position=[tx, 0.0, tz], top=True)
    under = cmd("scene.query", action="ground", position=[tx, 0.0, tz], top=True, only=[GROUND])
    check(top.get("object") == TREE and under.get("object") == GROUND
          and (top.get("floor") or 0) > (under.get("floor") or 0) + 3,
          "A ground top=true under a tree is the tree's crown (%.1f m), with only= the ground under it (%.1f m)"
          % (top.get("floor") or 0, under.get("floor") or 0), short(top) + short(under))

    area = [-114.0, -169.5, -98.0, -159.5]
    h = cmd("scene.query", action="heights", area=area, step=2)
    grid = h.get("heights") or []
    flat = [v for r in grid for v in r if v is not None]
    check(h.get("ok") and h.get("columns") == 9 and h.get("rows") == 6 and len(grid) == 6
          and all(len(r) == 9 for r in grid) and flat and near(min(flat), h.get("min"), 1e-3)
          and near(max(flat), h.get("max"), 1e-3),
          "A heights: 9 x 6 samples over 16 x 10 m at 2 m, rows from z0, min %.2f max %.2f"
          % (h.get("min") or 0, h.get("max") or 0), short({k: v for k, v in h.items() if k != "heights"}))
    sample = grid[2][3] if len(grid) > 2 and len(grid[2]) > 3 else None
    check(near(sample, floor(area[0] + 6, area[1] + 4), 0.01),
          "A a sample of heights is the ground at its place (%s)" % sample)
    capped = cmd("scene.query", action="heights", area=[-300, -300, 300, 300], step=1)
    check(not capped.get("ok") and capped.get("code") == "invalid" and "257" in str(capped.get("error")),
          "A heights over a cap is refused, naming the cap and a step that fits", short(capped))

    g = one.get("floor") or 0.0
    ray = cmd("scene.query", action="ray", **{"from": [x, g + 20, z], "to": [x, g - 20, z]})
    hit = ray.get("hit") or {}
    check(hit.get("object") == GROUND and near((hit.get("point") or [0, 0, 0])[1], g, 0.01)
          and near(hit.get("distance"), 20, 0.01) and (hit.get("surface") or {}).get("texture"),
          "A ray: the first hit is the ground 20 m below, with its surface", short(ray))
    through = cmd("scene.query", action="ray", all=True, **{"from": [tx, 40, tz], "to": [tx, -40, tz]})
    hits = through.get("hits") or []
    objs = [x_.get("object") for x_ in hits]
    dists = [x_.get("distance") or 0 for x_ in hits]
    check(TREE in objs and GROUND in objs and dists == sorted(dists) and through.get("total") == len(hits),
          "A ray all=true down through the tree: %d hits nearest first, the tree and the ground under it" % len(hits),
          short(objs))

    box = cmd("scene.query", action="box", min=[tx - 1, -15, tz - 1], max=[tx + 1, 0, tz + 1])
    found = {o.get("name"): o for o in box.get("objects") or []}
    tree_box = (found.get(TREE) or {}).get("bounds") or {}
    check(TREE in found and (tree_box.get("min") or [99])[0] <= tx + 1 and (tree_box.get("max") or [-999])[0] >= tx - 1
          and box.get("total", 0) >= len(found),
          "A box: the objects that meet it, the tree among them with its world box (%d)" % len(found), short(list(found)))

    blocked = cmd("scene.query", action="fit", ref=BARREL, position=[tx, (under.get("floor") or 0) + 0.2, tz])
    by = [b.get("object") for b in blocked.get("blocked_by") or []]
    check(blocked.get("ok") and blocked.get("fits") is False and TREE in by,
          "A fit at the tree's trunk: blocked by the tree", short(blocked))
    fx, fz = x + 2, z - 2.5
    free = cmd("scene.query", action="fit", ref=BARREL, position=[fx, g + 5, fz])
    rest = free.get("rest") or [0, 0, 0]
    ground = free.get("ground") or {}
    check(free.get("fits") is True and not free.get("blocked_by") and near(rest[1], ground.get("max"), 0.01)
          and near(free.get("gap"), g + 5 - (ground.get("centre") or 0), 0.05),
          "A fit 5 m over the yard: fits, its gap, `rest` on the highest ground under it (%.2f m)" % rest[1], short(free))
    again = cmd("scene.query", action="ground", position=[rest[0], rest[1] + 1, rest[2]])
    at_rest = cmd("scene.query", action="fit", ref=BARREL, position=rest)
    check(near(again.get("floor"), ground.get("centre"), 0.01) and at_rest.get("fits") is True
          and -0.01 <= (at_rest.get("gap") or -1) <= 0.5,
          "A at `rest` a second ground finds the ground under it and the model fits standing there (gap %.2f m)"
          % (at_rest.get("gap") or 0), short(again) + short(at_rest))

    spots = cmd("scene.query", action="free", ref=BARREL, position=[x, 0.0, z], radius=6, count=4)
    got = spots.get("spots") or []
    d = [s.get("distance") or 0 for s in got]
    fit_all = all(cmd("scene.query", action="fit", ref=BARREL, position=s["position"]).get("fits") for s in got)
    check(0 < len(got) <= 4 and d == sorted(d) and max(d) <= 6 and fit_all,
          "A free: %d spots within 6 m, nearest first, and every one passes fit" % len(got), short(got))

    ai = cmd("scene.query", action="ai", positions=[[x, g, z], [-500.0, 0.0, 900.0]])
    p = ai.get("points") or [{}, {}]
    check(p[0].get("on_map") is True and (p[0].get("distance") or 9) < 1 and p[1].get("on_map") is False,
          "A ai: the yard is on the AI map (%.2f m to its node), a point outside the level is not"
          % (p[0].get("distance") or 0), short(ai))
    walk = cmd("scene.query", action="ai", **{"from": [x, g, z], "to": [x - 6, g, z - 10]})
    check(walk.get("reachable") is True and (walk.get("length") or 0) >= math.hypot(6, 10) - 1,
          "A ai from + to: reachable, a walk of %.1f m" % (walk.get("length") or 0), short(walk))

    listed = cmd("scene.objects", near=[tx, 0.0, tz], radius=12, sort="distance", bounds=True, limit=20)
    rows = listed.get("objects") or []
    dist = [o.get("distance") or 0 for o in rows]
    check(rows and all("bounds" in o and "size" in o for o in rows) and dist == sorted(dist) and max(dist) <= 12
          and TREE in [o.get("name") for o in rows],
          "A scene.objects near + radius + sort=distance + bounds: %d objects, each with its box" % len(rows),
          short(rows[:3]))


# ---- B: the files ---------------------------------------------------------------------------------------------------

B_AT = (-112.0, -172.0)


def phase_b() -> None:
    x, z = B_AT
    y = floor(x, z) or 0.0
    before = cmd("scene.info").get("objects", 0)
    append_objects([row("qa_line_a", BARREL, (x, y, z))])
    check(wait_for("qa_line_a") and near(get("qa_line_a").get("position", [0, 0, 0])[1], y, 0.001),
          "B a line appended to objects.lua is in the open scene where it says")
    info = cmd("scene.info")
    check(info.get("objects") == before + 1 and not info.get("modified"),
          "B as the file holds it: one object more, nothing unsaved", short({k: info.get(k) for k in ("objects", "modified")}))

    everyone = names()
    good = read(scene_file("objects.lua"))
    at = line_of(good, "qa_line_a")
    broken = good.replace('{ name = "qa_line_a"', 'name = "qa_line_a"', 1)
    write(scene_file("objects.lua"), broken)
    syntax = qa.until(lambda: [p for p in problems_of("objects.lua") if p.get("code") == "syntax"], 15, 0.2) or []
    check(syntax and abs((syntax[0].get("line") or 0) - at) <= 1 and syntax[0].get("column"),
          "B objects.lua broken: a syntax error at its line %d, column %s" % (at, syntax[0].get("column") if syntax else "-"),
          short(syntax))
    check(names() == everyone, "B and the scene keeps every object it had (%d)" % len(everyone))
    checked = cmd("asset.check", path=SCENE)
    named = [p for p in checked.get("problems") or [] if "objects.lua" in str(p.get("file")) and p.get("code") == "syntax"]
    check(named and abs((named[0].get("line") or 0) - at) <= 1, "B asset.check of the scene names the line",
          short(checked))
    # the file itself, and the scene's folder, are checked as the scene: the file's own problems, the scene's
    part = cmd("asset.check", path=SCENE.rsplit(".", 1)[0] + "/objects.lua")
    folder = cmd("asset.check", path=SCENE.rsplit(".", 1)[0])
    of_part = [p for p in part.get("problems") or []]
    check(part.get("ok") and of_part and all("objects.lua" in str(p.get("file")) for p in of_part)
          and any(p.get("code") == "syntax" for p in of_part)
          and folder.get("ok") and any(p.get("code") == "syntax" for p in folder.get("problems") or []),
          "B asset.check of objects.lua answers that file's problems; of the scene's folder, the scene's",
          short(part) + " | " + short(folder))
    cmd("scene.set", objects=["qa_line_a"], position=[x, y + 1, z])
    saved = cmd("scene.save")
    check(saved.get("ok") and "objects.lua" in (saved.get("skipped") or []) and saved.get("unwritten") == ["objects.lua"]
          and read(scene_file("objects.lua")) == broken,
          "B scene.save answers objects.lua skipped, its change unwritten, and leaves the broken file as it is",
          short(saved))
    write(scene_file("objects.lua"), good)
    back = qa.until(lambda: not [p for p in problems_of("objects.lua") if p.get("code") == "syntax"], 15, 0.2)
    # what the scene changed while the file did not read is unsaved work: it is kept over the fixed file
    kept = qa.until(lambda: back and near(get("qa_line_a").get("position", [0, 0, 0])[1], y + 1, 0.001), 10, 0.2)
    check(back and kept and names() == everyone,
          "B the file fixed is followed again: no problem, every object, the change made meanwhile kept")
    again = cmd("scene.save")
    now = read(scene_file("objects.lua"))
    differ = [i for i, (a, b) in enumerate(zip(good.splitlines(), now.splitlines())) if a != b]
    check(again.get("ok") and not again.get("skipped") and len(good.splitlines()) == len(now.splitlines())
          and len(differ) == 1 and "qa_line_a" in now.splitlines()[differ[0]],
          "B and the next save writes it into the fixed file: that one line", short(again) + " %s" % differ[:5])
    # back where the line says, for the phases that follow
    cmd("scene.set", objects=["qa_line_a"], position=[x, y, z])
    cmd("scene.save")

    # a prefab is one line of groups.lua
    write(os.path.join(CTX.project, "prefabs", "qa_post.group"),
          "return {\n  group = 1,\n  objects = {\n"
          '    { name = "barrel", ref = "%s", pos = { 0, 0, 0 } },\n'
          '    { name = "cabinet", ref = "%s", pos = { 2, 0, 0 }, rot = { 0, 90, 0 } },\n'
          "  },\n}\n" % (lua(BARREL), lua(CABINET)))
    gx, gz = x + 4, z
    gy = floor(gx, gz) or 0.0
    write(scene_file("groups.lua"), "return {\n  groups = {\n"
          '    { name = "qa_post", ref = "prefabs/qa_post.group", pos = { %g, %g, %g }, rot = { 0, 90, 0 }, authored = true },\n'
          "  },\n}\n" % (gx, gy, gz))
    placed = wait_for("qa_post_barrel") and wait_for("qa_post_cabinet")
    barrel, cabinet = get("qa_post_barrel"), get("qa_post_cabinet")
    bp, cp = barrel.get("position") or [0, 0, 0], cabinet.get("position") or [0, 0, 0]
    check(placed and barrel.get("group") == "qa_post" and near(bp[0], gx, 0.001) and near(bp[2], gz, 0.001)
          and near(math.hypot(cp[0] - gx, cp[2] - gz), 2, 0.001) and near((cabinet.get("rotation") or [0, 0])[1], 180, 0.01),
          "B a group written with ref and no members is placed from its asset: <group>_<member>, turned with it",
          short([bp, cp, cabinet.get("rotation")]))
    note = [p for p in problems_of("groups.lua") if p.get("code") == "S023"]
    check(note and note[0].get("line") == 3 and cmd("scene.info").get("modified"),
          "B with a note at the group's line, and the scene unsaved", short(note))
    cmd("scene.save")
    text = read(scene_file("groups.lua"))
    check('members = { "qa_post_barrel", "qa_post_cabinet" }' in text, "B the save writes the group's members", text)


# ---- C: settle and save ---------------------------------------------------------------------------------------------

C_AT = [(-102.0, -174.0), (-99.0, -174.5)]


def phase_c() -> None:
    ys = [floor(x, z) or 0.0 for x, z in C_AT]
    append_objects([row("qa_drop_%d" % i, BARREL, (x, y + 3, z)) for i, ((x, z), y) in enumerate(zip(C_AT, ys))])
    if not check(wait_for("qa_drop_1"), "C two objects written 3 m over the ground"):
        return
    before = read(scene_file("objects.lua"))
    rests = [cmd("scene.query", action="fit", ref=BARREL, position=[x, y + 3, z], **{"except": ["qa_drop_0", "qa_drop_1"]})
             .get("rest") or [0, 0, 0] for (x, z), y in zip(C_AT, ys)]
    a = cmd("scene.align", action="drop", objects=["qa_drop_0"])
    pa = ((a.get("objects") or [{}])[0].get("position") or [0, 0, 0])
    check(a.get("requested") == 1 and a.get("landed") == 1 and near(pa[1], rests[0][1], 0.05),
          "C scene.align drop: it lands where fit's `rest` said (%.2f m)" % pa[1], short(a))
    b = cmd("scene.align", action="drop", objects=["qa_drop_1"], offset=0.5)
    pb = ((b.get("objects") or [{}])[0].get("position") or [0, 0, 0])
    check(b.get("landed") == 1 and near(pb[1], rests[1][1] + 0.5, 0.05),
          "C scene.align drop offset=0.5: half a metre above it (%.2f m)" % pb[1], short(b))
    saved = cmd("scene.save")
    after = read(scene_file("objects.lua"))
    x_, y_ = before.split("\n"), after.split("\n")
    changed = [(p, q) for p, q in zip(x_, y_) if p != q]
    check(saved.get("ok") and len(x_) == len(y_) and len(changed) == 2
          and all("qa_drop_" in q for _, q in changed),
          "C scene.save: only the two lines of the dropped objects differ", short(changed[:4]))


# ---- D: the audit ---------------------------------------------------------------------------------------------------

D_AT = (-97.0, -154.0)
D_NAMES = ["qa_float", "qa_in_a", "qa_in_b", "qa_twice_a", "qa_twice_b", "qa_unmarked"]


def phase_d() -> None:
    x, z = D_AT
    y = {k: floor(x + dx, z + dz) or 0.0 for k, (dx, dz) in
         {"float": (-2, -2), "in": (0, 0), "twice": (2, 2), "un": (-2, 2)}.items()}
    append_objects([
        row("qa_float", BARREL, (x - 2, y["float"] + 3, z - 2)),
        row("qa_in_a", CABINET, (x, y["in"], z)),
        row("qa_in_b", CABINET, (x - 0.3, y["in"], z - 0.1)),
        row("qa_twice_a", BARREL, (x + 2, y["twice"], z + 2)),
        row("qa_twice_b", BARREL, (x + 2, y["twice"], z + 2)),
        row("qa_unmarked", BARREL, (x - 2, y["un"], z + 2), authored=False),
    ])
    if not check(wait_for("qa_unmarked"), "D six objects written for the audit"):
        return
    text = read(scene_file("objects.lua"))
    audit = cmd("scene.audit", objects=D_NAMES)
    found = audit.get("problems") or []
    by = {}
    for p in found:
        by.setdefault(p.get("code"), set()).add(p.get("object"))
    placed_right = all(p.get("file") == "rawdata/levels/l01_escape/objects.lua" and p.get("line") == line_of(text, p.get("object"))
                       for p in found)
    floating = next((p for p in found if p.get("code") == "S201"), {})
    number = re.search(r"([\d.]+) m", str(floating.get("text")))
    check("qa_float" in by.get("S201", ()) and number and 2.5 < float(number.group(1)) < 3.5,
          "D written 3 m up: S201 - %s" % floating.get("text"), short(found))
    check(by.get("S203", set()) & {"qa_in_a", "qa_in_b"}, "D two written into each other: S203", short(sorted(by)))
    check(by.get("S204", set()) & {"qa_twice_a", "qa_twice_b"}, "D one written twice: S204", short(sorted(by)))
    check("qa_unmarked" in by.get("S210", ()),
          "D one not marked authored on the game's level, its objects known from l01_escape.level: S210",
          short(audit.get("skipped") or {k: sorted(v) for k, v in by.items()}))
    check(found and placed_right, "D every problem is at its object's line of objects.lua (%d)" % len(found),
          short([(p.get("object"), p.get("line"), line_of(text, p.get("object"))) for p in found]))

    cmd("scene.align", action="drop", objects=["qa_float"])
    cmd("scene.set", objects=["qa_in_b"], position=[x - 3, floor(x - 3, z - 3) or 0.0, z - 3])
    cmd("scene.delete", objects=["qa_twice_b"])
    cmd("scene.set", objects=["qa_unmarked"], set={"authored": True})
    left = [n for n in D_NAMES if n != "qa_twice_b"]
    again = cmd("scene.audit", objects=left)
    check(again.get("ok") and again.get("total") == 0 and again.get("checked") == len(left),
          "D fixed - dropped, moved apart, the copy deleted, marked authored - the audit is empty", short(again))
    cmd("scene.delete", objects=left)
    cmd("scene.save")


# ---- E: models ------------------------------------------------------------------------------------------------------

def gltf_box(sx: float, sy: float, sz: float, material: str) -> str:
    """A glTF 2.0 box sx x sy x sz standing on y = 0, its buffer a base64 data URI: 24 vertices, 12 triangles."""
    hx, hz = sx / 2, sz / 2
    faces = [((1, 0, 0), [(hx, 0, hz), (hx, 0, -hz), (hx, sy, -hz), (hx, sy, hz)]),
             ((-1, 0, 0), [(-hx, 0, -hz), (-hx, 0, hz), (-hx, sy, hz), (-hx, sy, -hz)]),
             ((0, 1, 0), [(-hx, sy, hz), (hx, sy, hz), (hx, sy, -hz), (-hx, sy, -hz)]),
             ((0, -1, 0), [(-hx, 0, -hz), (hx, 0, -hz), (hx, 0, hz), (-hx, 0, hz)]),
             ((0, 0, 1), [(-hx, 0, hz), (hx, 0, hz), (hx, sy, hz), (-hx, sy, hz)]),
             ((0, 0, -1), [(hx, 0, -hz), (-hx, 0, -hz), (-hx, sy, -hz), (hx, sy, -hz)])]
    pos, nrm, uv, idx = [], [], [], []
    for n, quad in faces:
        b = len(pos)
        for p, t in zip(quad, [(0, 1), (1, 1), (1, 0), (0, 0)]):
            pos.append(p)
            nrm.append(n)
            uv.append(t)
        idx += [b, b + 1, b + 2, b, b + 2, b + 3]
    blob = b"".join(struct.pack("<3f", *p) for p in pos) + b"".join(struct.pack("<3f", *n) for n in nrm)
    blob += b"".join(struct.pack("<2f", *t) for t in uv) + b"".join(struct.pack("<H", i) for i in idx)
    nv = len(pos)
    views = [(0, nv * 12), (nv * 12, nv * 12), (nv * 24, nv * 8), (nv * 32, len(idx) * 2)]
    return json.dumps({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0, "name": "box"}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3,
                                    "material": 0}]}],
        "materials": [{"name": material}],
        "buffers": [{"byteLength": len(blob),
                     "uri": "data:application/octet-stream;base64," + base64.b64encode(blob).decode()}],
        "bufferViews": [{"buffer": 0, "byteOffset": o, "byteLength": n} for o, n in views],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": nv, "type": "VEC3", "min": [-hx, 0, -hz],
                       "max": [hx, sy, hz]},
                      {"bufferView": 1, "componentType": 5126, "count": nv, "type": "VEC3"},
                      {"bufferView": 2, "componentType": 5126, "count": nv, "type": "VEC2"},
                      {"bufferView": 3, "componentType": 5123, "count": len(idx), "type": "SCALAR"}],
    }, indent=1)


RAMP_OBJ = """# a ramp 2 wide, 1 high, 3 deep, rising along +z
mtllib ramp.mtl
o ramp
v 0 0 0
v 2 0 0
v 2 0 3
v 0 0 3
v 2 1 3
v 0 1 3
vt 0 0
vt 1 0
vt 1 1
vt 0 1
usemtl ramp_top
f 1/1 2/2 5/3 6/4
usemtl ramp_side
f 1/1 4/2 3/3 2/4
f 4/1 6/2 5/3 3/4
f 1/1 6/2 4/3
f 2/1 3/2 5/3
"""


def png(w: int, h: int, rgb=(150, 110, 70)) -> bytes:
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


HUT = """return {
  mesh = 1,
  surfaces = {
    { name = "wall", texture = "%(tex)s", material = "materials\\\\wooden_board" },
    { name = "floor", texture = "%(tex)s", material = "materials\\\\wooden_board" },
  },
  parts = {
    { shape = "box", size = { 6, 0.2, 4 }, surface = "floor", tile = 2 },
    { shape = "box", size = { 6, 2.6, 0.2 }, pos = { 0, 0.2, 1.9 }, surface = "wall", tile = 2 },
    { shape = "box", size = { 0.2, 2.6, 4 }, pos = { -2.9, 0.2, 0 }, surface = "wall", tile = 2 },
    { shape = "box", size = { 0.2, 2.6, 4 }, pos = { 2.9, 0.2, 0 }, surface = "wall", tile = 2 },
    { shape = "prism", outline = { { -3.2, -2.2 }, { 3.2, -2.2 }, { 3.2, 2.2 }, { -3.2, 2.2 } }, height = 0.15,
      pos = { 0, 2.8, 0 }, surface = "wall" },
  },
}
"""
HUT_AT = (YARD[0] - 1, FLAT, YARD[1])
CRATE_AT = (YARD[0] + 3.5, FLAT, YARD[1] + 2)


def size_is(model: dict, want, tol: float = 0.002) -> bool:
    return all(near(a, b, tol) for a, b in zip(model.get("size") or [], want)) and len(model.get("size") or []) == 3


def phase_e() -> None:
    src = os.path.join(CTX.work, "art")
    os.makedirs(src, exist_ok=True)
    with open(os.path.join(src, "crate.gltf"), "w", encoding="utf-8") as f:
        f.write(gltf_box(1.2, 0.8, 0.6, "crate_wood"))
    with open(os.path.join(src, "ramp.obj"), "w", encoding="utf-8") as f:
        f.write(RAMP_OBJ)
    with open(os.path.join(src, "ramp.mtl"), "w", encoding="utf-8") as f:
        f.write("newmtl ramp_top\nKd 0.5 0.5 0.5\nnewmtl ramp_side\nKd 0.4 0.4 0.4\n")
    with open(os.path.join(src, "planks.png"), "wb") as f:
        f.write(png(64, 64))
    files = [os.path.join(src, n) for n in ("crate.gltf", "ramp.obj", "planks.png")]
    imported = cmd("asset.import", **{"from": files, "to": "models/yard"})
    assets = set(imported.get("assets") or [])
    want = {"models/yard/crate.meshasset", "models/yard/ramp.meshasset", "models/yard/planks.texasset"}
    on_disk = all(os.path.isfile(os.path.join(CTX.project, *p.split("/"))) for p in want | {
        "models/yard/crate.gltf", "models/yard/ramp.obj", "models/yard/ramp.mtl", "models/yard/planks.png"})
    check(imported.get("ok") and assets == want and on_disk and "models/yard/ramp.mtl" in (imported.get("copied") or []),
          "E asset.import: the glTF, the OBJ with its .mtl and the picture copied, an asset beside each", short(imported))
    head = read(os.path.join(CTX.project, "models", "yard", "crate.meshasset"))
    check("mesh = 1" in head and 'source = "crate.gltf"' in head, "E the model asset names its source", head)

    info = {m.get("ref"): m for m in cmd("model.info", refs=["models/yard/crate.meshasset", "models/yard/ramp.meshasset"]).get("models") or []}
    crate, ramp = info.get("models/yard/crate.meshasset") or {}, info.get("models/yard/ramp.meshasset") or {}
    check(size_is(crate, [1.2, 0.8, 0.6]) and crate.get("triangles") == 12 and crate.get("source") == "asset"
          and [s.get("name") for s in crate.get("surfaces") or []] == ["crate_wood"],
          "E model.info of the glTF: 1.2 x 0.8 x 0.6 m, 12 triangles, its material a surface", short(crate))
    check(size_is(ramp, [2, 1, 3]) and ramp.get("triangles") == 8 and near((ramp.get("bounds") or {}).get("max", [0, 0, 1])[2], 0, 0.001)
          and sorted(s.get("name") for s in ramp.get("surfaces") or []) == ["ramp_side", "ramp_top"],
          "E model.info of the OBJ: 2 x 1 x 3 m, 8 triangles, two surfaces, z turned to the game's side", short(ramp))
    untextured = [p for p in cmd("asset.check", path="models/yard/crate.meshasset").get("problems") or []
                  if p.get("code") == "M112"]
    check(untextured and untextured[0].get("severity") == "error",
          "E asset.check names the glTF surface that has no texture: an error, as the level build would refuse it",
          short(untextured))

    tex = "%s\\planks" % CTX.module
    write(os.path.join(CTX.project, "models", "yard", "crate.meshasset"),
          'return {\n  mesh = 1,\n  source = "crate.gltf",\n  surfaces = {\n'
          '    { name = "crate_wood", texture = "%s", material = "materials\\\\wooden_board" },\n  },\n}\n' % lua(tex))
    # the OBJ's two materials name no picture either: each gets the texture, and no material - the game's `default`
    write(os.path.join(CTX.project, "models", "yard", "ramp.meshasset"),
          'return {\n  mesh = 1,\n  source = "ramp.obj",\n  surfaces = {\n'
          '    { name = "ramp_top", texture = "%s" },\n    { name = "ramp_side", texture = "%s" },\n  },\n}\n'
          % (lua(tex), lua(tex)))
    write(os.path.join(CTX.project, "models", "yard", "hut.meshasset"), HUT % {"tex": lua(tex)})
    hut = qa.until(lambda: (cmd("model.info", refs=["models/yard/hut.meshasset"]).get("models") or [{}])[0].get("size"), 10)
    hut = (cmd("model.info", refs=["models/yard/hut.meshasset"]).get("models") or [{}])[0]
    check(size_is(hut, [6.4, 2.95, 4.4]) and {s.get("texture") for s in hut.get("surfaces") or []} == {tex},
          "E a blockout written as parts: 6.4 x 2.95 x 4.4 m, its surfaces on the texture asset", short(hut))
    checked = cmd("asset.check", path="models/yard")
    errors = [p for p in checked.get("problems") or [] if p.get("severity") == "error"]
    check(checked.get("ok") and not errors, "E asset.check of the models: no error", short(checked))

    append_objects([row("yard_hut", "models/yard/hut.meshasset", HUT_AT),
                    row("yard_crate", "models/yard/crate.meshasset", CRATE_AT)])
    if not check(wait_for("yard_crate"), "E the two model assets placed in objects.lua by their project paths"):
        return
    box = cmd("scene.query", action="box", position=list(CRATE_AT), size=[0.4, 3, 0.4])
    crate_row = next((o for o in box.get("objects") or [] if o.get("name") == "yard_crate"), {})
    b = crate_row.get("bounds") or {}
    check(crate_row.get("ref") == "models/yard/crate.meshasset" and near((b.get("min") or [0])[0], CRATE_AT[0] - 0.6, 0.002)
          and near((b.get("max") or [0, 0])[1], CRATE_AT[1] + 0.8, 0.002),
          "E scene.query box finds the crate with its world box", short(crate_row))
    hx, hy, hz = HUT_AT
    roof = cmd("scene.query", action="ray", **{"from": [hx, hy + 20, hz], "to": [hx, hy - 5, hz]}).get("hit") or {}
    check(roof.get("object") == "yard_hut" and near((roof.get("point") or [0, 0])[1], hy + 2.95, 0.002)
          and (roof.get("surface") or {}).get("texture") == tex,
          "E a ray meets the blockout's roof at 2.95 m, on its surface", short(roof))

    made = [cmd("file.create", kind=k, path="models/new") for k in ("model", "texture", "sound")]
    paths = [m.get("path") for m in made]
    check(all(m.get("ok") for m in made) and [p.rsplit(".", 1)[-1] for p in paths if p] == ["meshasset", "texasset", "sndasset"],
          "E file.create makes a model, a texture and a sound asset from their templates", short(made))
    template = [p for p in cmd("asset.check", path=paths[0] or "").get("problems") or []]
    check(not template, "E the model template checks without a problem", short(template))
    # a template's texture and sound name sources nobody gave them: the module build refuses them until then
    qa.remove_tree(os.path.join(CTX.project, "models", "new"))
    shown = cmd("preview.model", path="models/yard/hut.meshasset")
    check(shown.get("ok") and size_is(shown, [6.4, 2.95, 4.4], 0.01) and len(shown.get("surfaces") or []) == 2,
          "E preview.model path= shows the asset and answers its box and surfaces", short(shown))
    cmd("ui.tab", action="open", doc="level")
    cmd("scene.save")


# ---- F: pictures ----------------------------------------------------------------------------------------------------

def decode(answer: dict):
    from PIL import Image
    data = qa.png_of(answer)
    return data, Image.open(io.BytesIO(data)).convert("RGB")


def keep(name: str, data: bytes) -> None:
    if SHOTS and data:
        os.makedirs(SHOTS, exist_ok=True)
        with open(os.path.join(SHOTS, name + ".png"), "wb") as f:
            f.write(data)


def phase_f() -> None:
    x, z = YARD
    at = dict(position=[x, 0.0, z], size=40, px=512)
    for mode in ("objects", "height", "ai"):
        a = cmd("shot.map", mode=mode, **at)
        data, im = decode(a)
        keep("map_" + mode, data)
        check(a.get("ok") and im.size == (a.get("width"), a.get("height")) and near(a.get("px_per_m"), 512 / 40, 1e-6)
              and a.get("mode") == mode,
              "F shot.map mode=%s: a %d x %d PNG at %.1f px a metre" % (mode, im.size[0], im.size[1], a.get("px_per_m") or 0),
              short(a))
    plain = dict(at, labels=False, signs=False)
    a = cmd("shot.map", **plain)
    data, im = decode(a)
    m, ppm, area = a.get("map") or {}, a.get("px_per_m") or 0, a.get("area") or [0, 0, 0, 0]
    leg = {o.get("object"): o for o in a.get("legend") or []}
    crate = leg.get("yard_crate") or {}
    c = crate.get("centre") or [0, 0]
    px = m.get("left", 0) + (c[0] - area[0]) * ppm
    py = m.get("top", 0) + (area[3] - c[1]) * ppm
    want_px = m.get("left", 0) + (CRATE_AT[0] - area[0]) * ppm
    want_py = m.get("top", 0) + (area[3] - CRATE_AT[2]) * ppm
    colour = tuple(int(crate.get("colour", "#000000")[i:i + 2], 16) for i in (1, 3, 5))
    got = im.getpixel((int(px), int(py))) if 0 <= px < im.size[0] and 0 <= py < im.size[1] else (0, 0, 0)
    # the map shades every object by its slope: the crate's top faces up and keeps its colour within a few levels
    check(abs(px - want_px) <= 1 and abs(py - want_py) <= 1 and all(abs(g - w) <= 40 for g, w in zip(got, colour)),
          "F px_per_m and map carry the crate's legend centre to its pixel (%.1f, %.1f), drawn in its colour %s"
          % (px, py, crate.get("colour")), short([crate, got, want_px, want_py]))
    again, _ = decode(cmd("shot.map", **plain))
    check(again == data, "F the same map twice is the same bytes")
    lit = cmd("shot.map", highlight=["yard_crate"], **plain)
    lit_data, _ = decode(lit)
    keep("map_highlight", lit_data)
    check(lit_data != data and (lit.get("legend") or [{}])[0].get("object") == "yard_crate",
          "F highlight changes the picture and names the crate first", short((lit.get("legend") or [])[:1]))

    cam = cmd("viewport.camera", action="get").get("camera")
    top = cmd("shot.viewport", view="top", objects=["yard_hut"], width=640, height=360, clean=True, labels=True)
    data, im = decode(top)
    keep("viewport_top", data)
    labels = {l.get("object"): l for l in top.get("labels") or []}
    hut = labels.get("yard_hut") or {}
    check(im.size == (640, 360) and (top.get("camera") or {}).get("orthographic") is True
          and (top.get("camera") or {}).get("px_per_m") and near(hut.get("x"), 320, 2) and near(hut.get("y"), 180, 2),
          "F shot.viewport view=top frames the hut: flat, %.1f px a metre, its label in the middle"
          % ((top.get("camera") or {}).get("px_per_m") or 0), short(top))
    own = cmd("shot.viewport", position=[x + 12, FLAT + 8, z - 12], target=[x, FLAT, z], width=640, height=360)
    data, im = decode(own)
    keep("viewport_camera", data)
    check(im.size == (640, 360) and near(((own.get("camera") or {}).get("position") or [0])[0], x + 12, 0.01),
          "F shot.viewport position + target: a picture from a camera of its own", short(own))
    still = cmd("viewport.camera", action="get").get("camera")
    check(cam and still == cam, "F and the author's camera is where it was", short([cam, still]))
    sheet = cmd("shot.models", refs=["models/yard/hut.meshasset", BARREL], px=512)
    data, im = decode(sheet)
    keep("models", data)
    rows = sheet.get("models") or []
    check(im.size == (sheet.get("width"), sheet.get("height")) and len(rows) == 2 and size_is(rows[0], [6.4, 2.95, 4.4], 0.01),
          "F shot.models: a sheet of the blockout and a library prop with their sizes", short(rows))


# ---- G: terrain -----------------------------------------------------------------------------------------------------

def terrain_row(name: str, **over) -> str:
    keys = {"pos": "{ %g, %g, %g }" % (YARD[0], FLAT, YARD[1]), "size": "{ %g, %g }" % YARD_SIZE, "cell": "1",
            "base": '"ground"', "edge": "1.5",
            "edits": '{ { op = "rect", min = { -5, -3 }, max = { 5, 3 }, falloff = 1.5, height = %g } }' % FLAT,
            "surface": '"ground"', "cut": "true", "authored": "true"}
    keys.update(over)
    body = ", ".join("%s = %s" % (k, v) for k, v in keys.items() if v is not None)
    return "return {\n  terrains = {\n    { name = \"%s\", %s },\n  },\n}\n" % (name, body)


def meets(x: float, z: float) -> dict:
    return cmd("scene.query", action="ray", all=True, **{"from": [x, 30.0, z], "to": [x, -40.0, z]})


def phase_g() -> None:
    area = [YARD[0] - 10, YARD[1] - 8, YARD[0] + 10, YARD[1] + 8]
    old = cmd("scene.query", action="heights", area=area, step=1, only=[GROUND]).get("heights") or []
    write(scene_file("terrain.lua"), terrain_row(TERRAIN_NAME))
    x, z = YARD[0] + 4, YARD[1] - 2
    hit = qa.until(lambda: TERRAIN_NAME in [h.get("object") for h in meets(x, z).get("hits") or []], 15, 0.3)
    if not check(wait_for(TERRAIN_NAME) and hit, "G terrain.lua written: the terrain is in the scene and rays meet it",
                 short(meets(x, z))):
        return
    new = cmd("scene.query", action="heights", area=area, step=1, only=[GROUND, TERRAIN_NAME]).get("heights") or []
    inside, outside, bad = 0, 0, []
    for r, line in enumerate(new):
        for c, v in enumerate(line):
            lx, lz = area[0] + c - YARD[0], area[1] + r - YARD[1]
            if abs(lx) <= 5 and abs(lz) <= 3:
                inside += 1
                if not near(v, FLAT, 0.02):
                    bad.append(("in", lx, lz, v))
            elif abs(lx) > YARD_SIZE[0] / 2 + 1 or abs(lz) > YARD_SIZE[1] / 2 + 1:
                outside += 1
                if not near(v, old[r][c] if r < len(old) and c < len(old[r]) else None, 0.01):
                    bad.append(("out", lx, lz, v, old[r][c] if r < len(old) and c < len(old[r]) else None))
    check(new and inside == 77 and outside > 0 and not bad,
          "G heights over it: the flattened %.1f m at all %d samples inside, the old ground at the %d outside"
          % (FLAT, inside, outside), short(bad[:6]))
    hits = meets(x, z).get("hits") or []
    first = hits[0] if hits else {}
    check(first.get("object") == TERRAIN_NAME and first.get("kind") == "terrain"
          and near((first.get("point") or [0, 0])[1], FLAT, 0.02) and GROUND not in [h.get("object") for h in hits],
          "G a ray from above meets the terrain at %.2f m and no old ground under it (cut)" % FLAT, short(hits))
    probe = cmd("scene.place", kind="object", ref=BARREL, name="qa_on_terrain", position=[x, FLAT + 3, z])
    dropped = cmd("scene.align", action="drop", objects=["qa_on_terrain"])
    landed = (dropped.get("objects") or [{}])[0]
    if not check(probe.get("ok") and dropped.get("landed") == 1 and near((landed.get("position") or [0, 0])[1], FLAT, 0.02),
                 "G scene.align drop lands an object on the terrain (%.2f m)" % FLAT, short(dropped)):
        # what was under it, as the editor answers now
        print("  under it: " + short(cmd("scene.query", action="ray", **{"from": [x, FLAT + 2.9, z], "direction": [0, -1, 0], "all": True}), 900))
        print("  the object: " + short(cmd("scene.get", objects=["qa_on_terrain"], keys=["authored"]), 600))
        print("  the terrain: " + short(cmd("scene.audit", objects=[TERRAIN_NAME]), 600))
        print("  again: " + short(cmd("scene.align", action="drop", objects=["qa_on_terrain"]), 600))
    cmd("scene.delete", objects=["qa_on_terrain"])
    audit = cmd("scene.audit", objects=[TERRAIN_NAME])
    check(audit.get("ok") and not [p for p in audit.get("problems") or [] if p.get("code") == "S211"],
          "G its border meets the ground around: no S211", short(audit))
    write(scene_file("terrain.lua"), terrain_row("yard_raised", base="3", edits=None))
    raised = qa.until(lambda: [p for p in cmd("scene.audit", objects=["yard_raised"]).get("problems") or []
                               if p.get("code") == "S211"] if present("yard_raised") else [], 15, 0.3)
    check(raised and "terrain.lua" in str(raised[0].get("file")),
          "G raised 3 m by base: S211 at its line of terrain.lua - %s" % (raised[0].get("text") if raised else "-"),
          short(raised))
    write(scene_file("terrain.lua"), terrain_row(TERRAIN_NAME))
    wait_for(TERRAIN_NAME)
    cmd("scene.save")


# ---- H: the build ---------------------------------------------------------------------------------------------------

def phase_h() -> None:
    linked = (cmd("state").get("project") or {}).get("game_linked")
    status = cmd("level.status", timeout=120)
    if not linked or not status.get("stock"):
        print("SKIP H: no game is linked or l01_escape is not a level of it (%s)" % short(status, 200), flush=True)
        return
    t = time.monotonic()
    built = cmd("level.build", timeout=600)
    ran = {r.get("step"): r for r in built.get("ran") or []}
    piece = ran.get("piece") or {}
    files = piece.get("files") or []
    folder = os.path.join(CTX.project, "levels", "l01_escape")
    check(built.get("ok") and "piece" in ran and "levels/l01_escape/overlay.cut" in files
          and "levels/l01_escape/overlay.aiedit" in files
          and all(os.path.isfile(os.path.join(folder, n)) for n in ("overlay.cut", "overlay.aiedit", "overlay.xcform")),
          "H level.build runs the piece step and writes overlay.cut and overlay.aiedit (%.1f s)" % (time.monotonic() - t),
          short({k: v for k, v in built.items() if k not in ("ran", "replaces")} | {"ran": sorted(ran)}))
    with open(os.path.join(folder, "overlay.cut"), "rb") if os.path.isfile(os.path.join(folder, "overlay.cut")) else io.BytesIO() as f:
        head = f.read(12)
    check(head[:4] == b"XCU1" and struct.unpack("<II", head[4:12]) == (1, 1) if len(head) == 12 else False,
          "H overlay.cut is XCU1 version 1 with one region", repr(head))
    cuts = cmd("level.inspect", part="cuts")
    region = (cuts.get("regions") or [{}])[0]
    c, h = region.get("centre") or [0, 0, 0], region.get("half") or [0, 0, 0]
    check(cuts.get("total") == 1 and region.get("kind") == "ground" and near(c[0], YARD[0], 0.01) and near(c[2], YARD[1], 0.01)
          and near(h[0], YARD_SIZE[0] / 2, 0.01) and near(h[2], YARD_SIZE[1] / 2, 0.01) and near(region.get("yaw"), 0, 1e-4),
          "H level.inspect part=cuts: one region of the ground at the terrain's centre, half %.1f x %.1f m"
          % (h[0], h[2]), short(cuts))
    edits = cmd("level.inspect", part="ai_edits", limit=5)
    check(edits.get("ok") and (edits.get("total") or 0) > 0 and edits.get("moved") == edits.get("total")
          and 0 <= (edits.get("closed") or 0) <= edits.get("total"),
          "H level.inspect part=ai_edits: %s node(s) moved, %s closed" % (edits.get("moved"), edits.get("closed")),
          short(edits))
    pieces = {p.get("name"): p for p in cmd("level.inspect", part="pieces").get("pieces") or []}
    main = pieces.get("main") or {}
    check(main.get("cut") is True and main.get("stream") is False and not main.get("problems"),
          "H level.inspect part=pieces: the piece carries the cut and does not stream", short(main))

    out = os.path.join(CTX.work, "module_out")
    os.makedirs(out, exist_ok=True)
    made = cmd("mod.build", target="folder", path=out.replace("\\", "/"), timeout=600)
    root = os.path.join(out, "modules", CTX.module)
    tex = os.path.join(root, "gamedata", "textures", CTX.module)
    dds = os.path.join(tex, "planks.dds")
    magic = open(dds, "rb").read(4) if os.path.isfile(dds) else b""
    sources = [os.path.relpath(os.path.join(d, n), root) for d, _, fs in os.walk(root) for n in fs
               if n.lower().endswith((".png", ".gltf", ".obj", ".mtl", ".meshasset"))]
    check(made.get("ok") and (made.get("media") or 0) >= 1 and magic == b"DDS "
          and os.path.isfile(os.path.join(tex, "planks.texasset")) and not sources,
          "H mod.build publishes the texture asset: gamedata\\textures\\%s\\planks.dds and .texasset, no source (%s media)"
          % (CTX.module, made.get("media")), short({"answer": made, "sources": sources}))
    check(all(os.path.isfile(os.path.join(root, "levels", "l01_escape", n)) for n in ("overlay.cut", "overlay.aiedit")),
          "H and the module carries the level's overlay.cut and overlay.aiedit")


STEPS = {"A": phase_a, "B": phase_b, "C": phase_c, "D": phase_d, "E": phase_e, "F": phase_f, "G": phase_g, "H": phase_h}
NEEDS = {"F": "E", "G": "E", "H": "EG"}


def run(ctx: qa.Run) -> None:
    global SHOTS
    SHOTS = ctx.shots
    CTX.project, CTX.work = ctx.project, ctx.work
    ed = qa.Editor(ctx.project).start()
    try:
        opened = cmd("scene.open", path=LEVEL, timeout=300)
        if not check(opened.get("ok"), "the Escape scene is open", short(opened)):
            return
        saved = cmd("scene.save", timeout=300)
        reopened = cmd("scene.open", path=SCENE, timeout=300)
        cmd("ui.tab", action="open", doc="level")
        if not check(saved.get("ok") and reopened.get("ok") and reopened.get("file") == SCENE,
                     "the scene saved as its .scene and opened from its files", short(reopened)):
            return
        CTX.module = (cmd("mod.info").get("manifest") or {}).get("id") or CTX.module
        stalls_since("opening Escape")
        qa.mark("scene")
        qa.run_phases(STEPS, ctx.only, NEEDS)
    finally:
        ed.close()


def main() -> int:
    return qa.suite("LEVEL DESIGN", run, doc=__doc__, ignore=(".git", "releases", "temp", "logs", "backup"),
                    requires=[LEVEL], phases=PHASES)


if __name__ == "__main__":
    sys.exit(main())
