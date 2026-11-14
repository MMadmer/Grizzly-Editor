#!/usr/bin/env python3
"""The scene's files, its commands, the viewport's gestures and the scene tools' pictures, over MCP on a THROW-AWAY
COPY of a project with the Escape scene (rawdata\\levels\\l01_escape.level, an X-Ray SDK scene).

The scene's own files (docs/assets/SCENE.md):

  F  the X-Ray SDK scene opens (imported), saves as rawdata\\levels\\l01_escape.scene with its folder, and opens
     again from those files with the same objects where they stood
  O  objects.lua edited from OUTSIDE (an object added, with a comment): the open scene takes it at once, as one undo
     step (Undo takes it out again, Redo brings it back)
  K  one object moved in the editor and saved: the comment is still there and only that object's line changed
  X  a light and a glow selected together are deleted together, in one undo step, and a locked object is not

Every gesture goes through viewport_gesture, which feeds the viewport's own input - the same code a mouse reaches:

  R  Random Append: the Object tool in Place with Randomize Added Objects on - a click adds one object of the random
     set, its scale and rotation out of the set's ranges
  A  Attach on Click of the Spawn tool: a click on a shape gives it to the selected spawn point; the switch goes off
  L  the left button as in Unreal: a left drag that starts on an object moves the camera and selects nothing, a
     click on it selects it when the button comes up; in Place a left drag places nothing
  M  a marquee (Ctrl + drag) selects what its box holds
  G  a move drag on the gizmo's X arrow moves the selection along X alone
  P  the Group tool's Pivot chooser (ui_dialog id=group_pivot) lists the group's objects; align_pivot is its pick
  V  a motion's playback with Camera View: the camera rides on the object while it plays, and stops with it
  D  detail objects painted on the ground (scene.details): two meshes, a stroke - the viewport's picture differs
     over the ground where they stand -, the eraser, a colour-index picture imported
  W  a wallmark of a texture the game has, laid on the ground (scene.wallmarks): the picture differs where it lies,
     and only there
  S  a screenshot is current: the scene opened again, ONE screenshot_editor and ONE screenshot_viewport right after it
     show the scene (the viewport is not flat) and say nothing is pending
  (The level compiler and Play's roots are test_level_commands.py's.)

    python tools/qa/test_scene_tools.py --project <project> [--only R,A,...] [--shots <folder>]

Each phase stands on its own setup (its own objects, its own camera), so --only runs any of them alone.
"""
from __future__ import annotations

import argparse
import base64
import io
import json
import math
import os
import re
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
MCP = qa_target.MCP
PHASES = "FOKXRALMGPVDWS"
SCENE = "rawdata/levels/l01_escape.scene"
LEVEL = "rawdata/levels/l01_escape.level"
# open ground of Escape east of the village, no tree over it; every phase works round a point of its own
SPOTS = {"R": (-119.0, -162.0), "A": (-130.0, -147.0), "L": (-125.0, -156.0), "M": (-100.0, -154.0), "G": (-119.0, -136.0),
         "P": (-101.0, -136.0), "V": (-130.0, -166.0), "D": (-110.0, -190.0), "W": (-110.0, -190.0)}
BOX = "dynamics\\equipment_cache\\equipment_box_01"
CABINET = "statics\\shkaf\\cabinet_village_140x210_ph"
TERRAIN = "scenes\\escape\\escape_terrain"

results: list[tuple[bool, str]] = []
SHOTS: str | None = None


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 120.0, **extra) -> dict:
    t0 = time.monotonic()
    try:
        return _mcp(cmd, timeout, **extra)
    finally:
        if os.environ.get("QA_SLOW") and time.monotonic() - t0 > 0.2:
            print("SLOW %s %.2f s" % (cmd, time.monotonic() - t0), flush=True)


def _mcp(cmd: str, timeout: float = 120.0, **extra) -> dict:
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def short(d) -> str:
    return json.dumps(d, ensure_ascii=False)[:500]


# ---- pictures --------------------------------------------------------------------------------------------------------

def picture(name: str):
    """The viewport's picture now (RGB), kept under --shots."""
    from PIL import Image
    pic = mcp("screenshot_viewport")
    data = base64.b64decode(pic.get("png_base64") or b"")
    if SHOTS and data:
        os.makedirs(SHOTS, exist_ok=True)
        with open(os.path.join(SHOTS, name + ".png"), "wb") as f:
            f.write(data)
    return Image.open(io.BytesIO(data)).convert("RGB")


def changed(a, b, box=None, level: int = 24) -> float:
    """The share of pixels (of `box` = (x0, y0, x1, y1) in pixels, or of the whole) whose brightness differs by more
    than `level` between two pictures."""
    from PIL import ImageChops
    d = ImageChops.difference(a, b)
    if box:
        d = d.crop(box)
    mask = d.convert("L").point(lambda v: 255 if v > level else 0)
    hist = mask.histogram()
    total = sum(hist)
    return (hist[255] / total) if total else 0.0


def until_changed(name: str, base, box=None, level: int = 24, share: float = 0.0, seconds: float = 6.0):
    """The picture once it differs from `base` by more than `share` (models and textures a pass asked for load
    beside it, on a busy machine later), or the last one taken when it never did. Answers the picture and the share."""
    deadline = time.monotonic() + seconds
    while True:
        pic = picture(name)
        got = changed(base, pic, box, level)
        if got > share or time.monotonic() > deadline:
            return pic, got
        time.sleep(0.1)


# ---- the scene ------------------------------------------------------------------------------------------------------

class Ctx:
    def __init__(self) -> None:
        self.project = ""
        self.size = (0.0, 0.0)  # the viewport in points
        self.dpi = 1.0
        self.ground: dict[str, float] = {}

    def centre(self) -> list[float]:
        return [round(self.size[0] / 2, 1), round(self.size[1] / 2, 1)]


CTX = Ctx()


def ground_y(x: float, z: float) -> float:
    """The terrain's height at (x, z): a box set on it and taken away."""
    placed = mcp("scene.place", kind="object", ref=BOX, position=[x, 200.0, z], on_surface=True)
    o = placed.get("object") or {}
    mcp("scene.delete", objects=[o.get("name")])
    return float((o.get("position") or [0.0, 0.0, 0.0])[1])


def position(name: str) -> list[float]:
    found = mcp("scene.get", objects=[name], keys=["pos"]).get("objects") or [{}]
    return found[0].get("position") or [0.0, 0.0, 0.0]


def look_down(phase: str, height: float = 12.0) -> tuple[float, float, float]:
    """The camera straight over the phase's spot: the spot is the viewport's centre."""
    x, z = SPOTS[phase]
    y = CTX.ground.get(phase)
    if y is None:
        y = CTX.ground[phase] = ground_y(x, z)
    mcp("camera_set", x=x, y=y + height, z=z, h=0.0, p=-math.pi / 2 + 1e-4, b=0.0)
    return x, y, z


def arm(kind: str) -> None:
    """Place mode with `kind` armed: what a click of Place adds."""
    mcp("scene.mode", mode="place", kind=kind)


def gesture(**kw) -> dict:
    r = mcp("viewport_gesture", **kw)
    if r.get("ok") and r.get("viewport"):
        CTX.size = tuple(r["viewport"])
    return r


def place(ref: str, x: float, y: float, z: float, ground: bool = False, rotation=None) -> str:
    extra = {"rotation": rotation} if rotation else {}
    r = mcp("scene.place", kind="object", ref=ref, position=[x, y, z], on_surface=ground, **extra)
    return (r.get("object") or {}).get("name") or ""


def deselect() -> None:
    mcp("scene.select", none=True)


def random_placement(**fields) -> dict:
    """The scene objects' random placement (`random_placement` of the .scene)."""
    return mcp("scene.set", world=True, set={"random_placement": fields})


# ---- F: the scene's own files ---------------------------------------------------------------------------------------

def folder(name: str) -> str:
    return os.path.join(CTX.project, "rawdata", "levels", "l01_escape", name)


def read(name: str) -> str:
    with open(folder(name), encoding="utf-8") as f:
        return f.read()


def write(name: str, text: str) -> None:
    with open(folder(name), "w", encoding="utf-8", newline="") as f:
        f.write(text)


def listing() -> dict:
    """Every object of the scene by name: its kind and position."""
    out = {}
    offset = 0
    while True:
        page = mcp("scene.objects", limit=2000, offset=offset)
        for o in page.get("objects") or []:
            out[o["name"]] = (o.get("kind"), tuple(o.get("position") or ()))
        offset += 2000
        if offset >= page.get("total", 0):
            return out


def phase_f() -> None:
    info = mcp("scene.info")
    check(info.get("imported") == LEVEL and info.get("file") == SCENE,
          "F the X-Ray SDK scene opens as an import that saves as its .scene", short(info))
    before = listing()
    saved = mcp("scene.save", timeout=300)
    main = os.path.join(CTX.project, *SCENE.split("/"))
    check(saved.get("ok") and saved.get("file") == SCENE and os.path.isfile(main),
          "F Save writes the .scene", short(saved))
    files = sorted(os.listdir(os.path.dirname(folder("x"))))
    check({"objects.lua", "lights.lua", "spawns.lua", "ways.lua"} <= set(files),
          "F and its folder holds a Lua file per kind", short(files))
    with open(main, encoding="utf-8") as f:
        head = f.read()
    check(head.lstrip().startswith("return") and "scene = 1" in head, "F the .scene is Lua data with its version",
          head[:200])
    opened = mcp("scene.open", path=SCENE, timeout=300)
    check(opened.get("ok") and opened.get("imported") is None and opened.get("file") == SCENE,
          "F the .scene opens from its own files", short(opened))
    after = listing()
    moved = [n for n, (k, p) in before.items() if after.get(n) != (k, p)]
    check(len(after) == len(before) and not moved, "F with every object of the import where it stood (%d)" % len(after),
          short(moved[:5]))


def own_files() -> None:
    """The scene in its own files (a phase run alone after the import): saved and opened from them."""
    if mcp("scene.info").get("imported"):
        mcp("scene.save", timeout=300)
        mcp("scene.open", path=SCENE, timeout=300)


# ---- O: an edit from outside ----------------------------------------------------------------------------------------

OUTSIDE = "qa_outside_01"
NOTE = "-- added by the QA from outside the editor"


def object_line(text: str, name: str) -> int:
    lines = text.split("\n")
    return next((i for i, l in enumerate(lines) if 'name = "%s"' % name in l), -1)


def wait_for(name: str, present: bool, seconds: float = 15.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        hit = any(o.get("name") == name for o in mcp("scene.objects", query=name).get("objects") or [])
        if hit == present:
            return True
        time.sleep(0.2)
    return False


def add_outside() -> bool:
    """objects.lua edited as another program would: a copy of its first object under a new name, a comment above it.
    `True` once the open scene holds it."""
    text = read("objects.lua")
    lines = text.split("\n")
    first = next((i for i, l in enumerate(lines) if 'name = "' in l and "ref = " in l), -1)
    if first < 0:
        return False
    copy = re.sub(r'name = "[^"]*"', 'name = "%s"' % OUTSIDE, lines[first], count=1)
    indent = lines[first][:len(lines[first]) - len(lines[first].lstrip())]
    lines[first + 1:first + 1] = [indent + NOTE, copy]
    write("objects.lua", "\n".join(lines))
    return wait_for(OUTSIDE, True)


def phase_o() -> None:
    own_files()
    count = mcp("scene.info").get("objects", 0)
    check(add_outside(), "O the object added in objects.lua from outside comes into the open scene")
    info = mcp("scene.info")
    check(info.get("objects") == count + 1 and not info.get("modified"),
          "O as the files hold it: nothing unsaved", short({k: info.get(k) for k in ("objects", "modified")}))
    undo = mcp("doc.undo", doc="level")
    check(undo.get("ok") and wait_for(OUTSIDE, False, 5.0), "O one Undo takes the outside change back", short(undo))
    redo = mcp("doc.redo", doc="level")
    check(redo.get("ok") and wait_for(OUTSIDE, True, 5.0), "O and Redo brings it back", short(redo))


# ---- K: a save keeps what nobody changed ----------------------------------------------------------------------------

def phase_k() -> None:
    own_files()
    if NOTE not in read("objects.lua"):
        add_outside()
    before = read("objects.lua")
    lines = before.split("\n")
    # the name as the file spells it, its backslashes escaped
    name = next((re.search(r'name = "([^"]*)"', l).group(1).replace("\\\\", "\\") for l in lines
                 if 'name = "' in l and "ref = " in l and OUTSIDE not in l), "")
    if not check(name and NOTE in before, "K the outside object and its comment are in objects.lua", name):
        return
    p = position(name)
    moved = mcp("scene.set", objects=[name], position=[p[0] + 1.5, p[1], p[2]])
    saved = mcp("scene.save", timeout=300)
    check(moved.get("ok") and saved.get("ok"), "K an object moved and the scene saved", short(moved) + short(saved))
    after = read("objects.lua")
    check(NOTE in after and OUTSIDE in after, "K the comment and the outside object are still there")
    a, b = before.split("\n"), after.split("\n")
    changed = [(x, y) for x, y in zip(a, b) if x != y]
    check(len(a) == len(b) and len(changed) == 1 and name.replace("\\", "\\\\") in changed[0][1],
          "K only the moved object's line changed", short(changed[:3]))
    check(saved.get("written") == 1, "K and the save wrote that one file", short(saved))


# ---- X: a selection of several kinds --------------------------------------------------------------------------------

def phase_x() -> None:
    light = (mcp("scene.place", kind="light", position=[5.0, 2.0, 5.0]).get("object") or {}).get("name")
    glow = (mcp("scene.place", kind="glow", position=[5.0, 2.0, 7.0]).get("object") or {}).get("name")
    kept = (mcp("scene.place", kind="glow", position=[5.0, 2.0, 9.0]).get("object") or {}).get("name")
    mcp("scene.set", objects=[kept], locked=True)
    picked = mcp("scene.select", objects=[light, glow, kept])
    check(picked.get("selected") == 3, "X a light and two glows are selected together", short(picked))
    gone = mcp("scene.delete")
    left = {o.get("name") for o in mcp("scene.objects", classes=["light", "glow"], limit=5000).get("objects") or []}
    check(gone.get("removed") == 2 and light not in left and glow not in left,
          "X Delete removes the light and the glow at once", short(gone))
    check(kept in left, "X and leaves the locked glow alone")
    undo = mcp("doc.undo", doc="level")
    back = {o.get("name") for o in mcp("scene.objects", classes=["light", "glow"], limit=5000).get("objects") or []}
    check(undo.get("ok") and light in back and glow in back, "X one Undo brings both back", short(undo))
    mcp("scene.set", objects=[kept], locked=False)
    mcp("scene.delete", objects=[light, glow, kept])


# ---- R: Random Append -----------------------------------------------------------------------------------------------

def phase_r() -> None:
    look_down("R", height=5.0)
    arm("object")
    deselect()
    rnd = random_placement(on=True, objects=[BOX, CABINET], random_scale=True, proportional=True,
                           scale=[[0.5, 0.5, 0.5], [2, 2, 2]], random_rotation=True, rotation=[[0, 0, 0], [0, 180, 0]])
    r = (rnd.get("set") or {}).get("random_placement") or {}
    check(rnd.get("ok") and r.get("on") and r.get("objects") == [BOX, CABINET] and r.get("rotation") == [[0, 0, 0], [0, 180, 0]],
          "R random_placement of the scene sets Randomize Added Objects and its ranges", short(rnd))
    # the object stands upright: the terrain's normal would tilt it (Normal Align)
    mcp("viewport_controls", tool="place", object_snap=True, grid_snap=False, normal_align=False)
    before = picture("r_before")
    g = gesture(kind="click", at=CTX.centre())
    placed = g.get("placed") or []
    o = placed[0] if placed else {}
    check(g.get("ok") and len(placed) == 1, "R a click in Place adds one object", short(g))
    check(o.get("reference") in (BOX, CABINET), "R its reference comes out of the random set", short(o))
    s, rot = o.get("scale") or [0, 0, 0], o.get("rotation") or [9, 9, 9]
    check(0.5 <= s[0] <= 2.0 and s[0] == s[1] == s[2], "R its scale is one factor out of 0.5..2", str(s))
    check(abs(rot[0]) < 1e-3 and abs(rot[2]) < 1e-3 and 0.0 <= rot[1] <= math.pi + 1e-3,
          "R its rotation turns about Y alone, within 0..180 degrees", str(rot))
    x, _, z = o.get("position") or [0, 0, 0]
    check(abs(x - SPOTS["R"][0]) < 0.5 and abs(z - SPOTS["R"][1]) < 0.5, "R it stands where the click was",
          "%s vs %s" % (o.get("position"), SPOTS["R"]))
    check(g.get("tool") == "select", "R the tool goes back to Select after the click", str(g.get("tool")))
    w, h = before.size
    middle = (w // 2 - h // 16, h // 2 - h // 16, w // 2 + h // 16, h // 2 + h // 16)
    _, near = until_changed("r_after", before, middle, share=0.1)
    check(near > 0.1, "R the picture shows it where it was clicked (%.1f %% of the middle changed)" % (near * 100))
    random_placement()
    mcp("viewport_controls", normal_align=True)


# ---- A: Attach on Click ---------------------------------------------------------------------------------------------

def phase_a() -> None:
    x, y, z = look_down("A")
    deselect()
    shape = mcp("scene.place", kind="shape", position=[x, y + 0.5, z], box=True)
    shape_name = (shape.get("object") or {}).get("name") or ""
    spawn = mcp("scene.place", kind="spawn", section="space_restrictor", position=[x + 3.0, y, z])
    spawn_name = (spawn.get("object") or {}).get("name") or ""
    check(shape.get("ok") and spawn.get("ok") and spawn_name, "A a shape and a spawn point to attach it to",
          short(shape) + " " + short(spawn))
    mcp("scene.select", objects=[spawn_name])
    arm("spawn")
    on = mcp("scene.attach", on_click=True)
    check(on.get("on_click") is True, "A scene.attach on_click=true turns Attach on Click on", short(on))
    mcp("viewport_controls", tool="place")
    g = gesture(kind="click", at=CTX.centre())
    att = [c for c in (g.get("moved") or []) + (g.get("changed") or []) if c.get("name") == spawn_name]
    check(g.get("ok") and att and att[0].get("attached") == shape_name, "A a click on the shape gives it to the spawn point",
          short(g))
    check(shape_name in (g.get("removed") or []), "A the shape leaves the scene's list: the point holds it now",
          short(g.get("removed")))
    pos = ((att[0].get("to") or {}).get("position") if att else None) or [0, 0, 0]
    check(abs(pos[0] - x) < 1e-3 and abs(pos[2] - z) < 1e-3, "A the point takes the shape's place", str(pos))
    back = g.get("placed") or []
    check(len(back) == 1 and back[0].get("class") == "shape" and back[0].get("name"),
          "A the point's own sphere goes back into the scene under a name of its own (DetachObject)", short(back))
    check(g.get("tool") == "select" and not mcp("scene.attach", on_click=False).get("on_click"),
          "A the tool goes back to Select and Attach on Click goes off", str(g.get("tool")))


# ---- M: a marquee ---------------------------------------------------------------------------------------------------

def phase_l() -> None:
    x, y, z = look_down("L")
    arm("object")
    box = place(BOX, x, y, z, ground=True)
    deselect()
    mcp("viewport_controls", tool="select")
    cx, cy = CTX.centre()
    g = gesture(kind="drag", at=[cx, cy], to=[cx + CTX.size[1] * 0.2, cy])
    check(g.get("ok") and g.get("camera_moved") and not g.get("picked"),
          "L a left drag that starts on an object moves the camera and selects nothing", short(g))
    look_down("L")
    g = gesture(kind="click", at=[cx, cy])
    check(g.get("ok") and box in (g.get("picked") or []) and not g.get("camera_moved"),
          "L a click on it selects it when the button comes up", short(g))
    # Place: the object goes down on the click, and a drag places nothing
    deselect()
    random_placement(on=True, objects=[BOX])
    mcp("viewport_controls", tool="place", object_snap=True, grid_snap=False)
    look_down("L")
    g = gesture(kind="drag", at=[cx - CTX.size[1] * 0.1, cy], to=[cx + CTX.size[1] * 0.1, cy])
    check(g.get("ok") and g.get("camera_moved") and not g.get("placed"),
          "L in Place a left drag moves the camera and places nothing", short(g))
    random_placement()
    mcp("viewport_controls", tool="select")


def phase_m() -> None:
    x, y, z = look_down("M")
    names = [place(BOX, x + dx, y, z, ground=True) for dx in (-1.5, 1.5)]
    far = place(BOX, x + 9.0, y, z, ground=True)
    deselect()
    mcp("viewport_controls", tool="select")
    cx, cy = CTX.centre()
    # 12 m up with a 60 degree field: a metre is about 0.07 of the height; the box takes 4 m round the spot
    half = CTX.size[1] * 0.18
    g = gesture(kind="drag", at=[cx - half, cy - half], to=[cx + half, cy + half], mods="ctrl")
    picked = set(g.get("picked") or [])
    check(g.get("ok") and set(names) <= picked, "M Ctrl + drag selects the objects its box holds", short(g))
    check(far not in picked, "M and not the one outside it", short(g.get("picked")))
    check(not g.get("camera_moved") and not g.get("moved"), "M the camera and the objects stay where they are", short(g))


# ---- G: a gizmo drag ------------------------------------------------------------------------------------------------

def phase_g() -> None:
    x, y, z = look_down("G")
    name = place(BOX, x, y, z, ground=True)
    mcp("scene.select", objects=[name])
    mcp("viewport_controls", tool="move", grid_snap=False)
    h = gesture(kind="hover", at=CTX.centre())
    gz = h.get("gizmo") or {}
    check(gz.get("tool") == "move" and gz.get("x") and gz.get("pivot"), "G a hover answers where the gizmo's arrows are",
          short(h))
    if not gz.get("x"):
        return
    (px, py), (ax, ay) = gz["pivot"], gz["x"]
    dx, dy = ax - px, ay - py
    n = math.hypot(dx, dy) or 1.0
    to = [round(ax + dx / n * 80, 1), round(ay + dy / n * 80, 1)]
    g = gesture(kind="drag", at=[ax, ay], to=to)
    moved = [m for m in g.get("moved") or [] if m.get("name") == name]
    m = moved[0] if moved else {}
    a, b = (m.get("from") or {}).get("position") or [0, 0, 0], (m.get("to") or {}).get("position") or [0, 0, 0]
    check(g.get("ok") and moved and b[0] - a[0] > 0.5, "G the drag on the X arrow moves the object along +X",
          short(g))
    check(moved and abs(b[1] - a[1]) < 1e-3 and abs(b[2] - a[2]) < 1e-3, "G and along X alone", "%s -> %s" % (a, b))
    check(not g.get("camera_moved") and not g.get("placed"), "G the camera stays and nothing is added", short(g))
    undo = mcp("doc.undo", doc="level")
    back = position(name)
    check(undo.get("ok") and abs(back[0] - a[0]) < 1e-3, "G the drag is one undo step", short(undo) + " " + short(back))
    mcp("viewport_controls", tool="select")


# ---- P: the Group tool's pivot --------------------------------------------------------------------------------------

def phase_p() -> None:
    x, y, z = look_down("P")
    a = place(BOX, x - 2.0, y, z, ground=True)
    b = place(BOX, x + 2.0, y, z, ground=True, rotation=[0.0, 28.6, 0.0])
    grp = mcp("scene.group", objects=[a, b])
    gname = (grp.get("object") or {}).get("name") or ""
    check(grp.get("ok") and grp.get("members") == 2, "P scene.group makes a group of the two", short(grp))
    mcp("scene.select", objects=[gname])
    opened = mcp("ui.dialog", action="open", id="group_pivot")
    pic = mcp("shot.window")
    modal = pic.get("modal") or {}
    check(opened.get("ok") and modal.get("id") == "dlg.choose" and "Object" in str(modal.get("title")),
          "P ui_dialog open id=group_pivot puts the Pivot chooser up", short(modal) + " " + short(opened))
    if SHOTS and pic.get("png_base64"):
        os.makedirs(SHOTS, exist_ok=True)
        with open(os.path.join(SHOTS, "p_chooser.png"), "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
    audit = mcp("ui.audit", id="dlg.choose")
    controls = {str(c.get("label")): c for c in audit.get("controls") or []}
    check(audit.get("ok") and not audit.get("escaped") and (controls.get("Select") or {}).get("enabled") is False
          and "Cancel" in controls, "P its Select waits for a pick, and nothing leaves it", short(sorted(controls)))
    refused = mcp("viewport_gesture", kind="click", at=CTX.centre())
    check(not refused.get("ok") and "a dialog is up" in str(refused.get("error")), "P a gesture is refused while it is up",
          short(refused))
    mcp("ui.dialog", action="close", id="dlg.choose")
    check(not mcp("shot.window").get("modal"), "P it closes as Cancel does")
    aligned = mcp("scene.group", action="pivot_to", objects=[gname], object=b)
    gp, bp = position(gname), position(b)
    check(aligned.get("changed") == 1 and all(abs(p - q) < 1e-3 for p, q in zip(gp, bp)),
          "P its pick, scene.group action=pivot_to object=, puts the group's pivot on that object", "%s vs %s" % (gp, bp))
    no_group = mcp("scene.select", none=True) and mcp("ui.dialog", action="open", id="group_pivot")
    check(not no_group.get("ok") and "Select a group" in str(no_group.get("error")),
          "P without a selected group it says so", short(no_group))


# ---- V: playback with Camera View -----------------------------------------------------------------------------------

def phase_v() -> None:
    x, y, z = look_down("V", height=20.0)
    name = place(BOX, x, y + 1.0, z)
    # a key where it stands at 0 s; then at 2 s, 10 m east and 3 m up (a frame change poses the object by the
    # motion, so the key comes first and the move after it)
    steps = [mcp("scene.motion", object=name, action="motionable", on=True),
             mcp("scene.motion", object=name, action="add_key"),
             mcp("scene.motion", object=name, action="current", time=2.0)]
    steps.append(mcp("scene.set", objects=[name], position=[x + 10.0, y + 4.0, z]))
    steps += [mcp("scene.motion", object=name, action="add_key"),
              mcp("scene.motion", object=name, action="range", min=0.0, max=2.0),
              mcp("scene.motion", object=name, action="first")]
    st = mcp("scene.motion", object=name)
    check(all(s.get("ok") for s in steps) and (st.get("motion") or {}).get("keys", 0) >= 2,
          "V a motion of two keys, two seconds long", short([s for s in steps if not s.get("ok")] or st))
    cam0 = (mcp("camera_get").get("camera") or {}).get("position")
    still = picture("v_still")
    on = mcp("scene.motion", object=name, action="camera_view", on=True)
    play = mcp("scene.motion", object=name, action="play")
    check(on.get("ok") and (play.get("motion") or {}).get("playing") and (play.get("motion") or {}).get("camera_view"),
          "V Camera View on, the motion plays", short(play))
    time.sleep(0.6)
    _, view = until_changed("v_playing", still, share=0.2)
    cam1 = (mcp("camera_get").get("camera") or {}).get("position") or [0, 0, 0]
    obj = position(name)
    check(cam1 != cam0 and x < cam1[0] < x + 10.0, "V the camera rides on the playing object", "%s -> %s" % (cam0, cam1))
    check(abs(obj[0] - x) < 1e-3, "V and the scene keeps the object where it stands until the pause", str(obj))
    check(view > 0.2, "V the viewport shows the view from the object (%.0f %% changed)" % (view * 100))
    paused = mcp("scene.motion", object=name, action="pause")
    cam2 = (mcp("camera_get").get("camera") or {}).get("position")
    time.sleep(0.3)
    cam3 = (mcp("camera_get").get("camera") or {}).get("position")
    m = paused.get("motion") or {}
    check(paused.get("ok") and not m.get("playing") and 0.0 < m.get("current", 0) <= 2.0,
          "V pause stops it at the time it reached", short(paused))
    check(cam2 == cam3, "V and the camera stops with it", "%s -> %s" % (cam2, cam3))
    mcp("scene.motion", object=name, action="camera_view", on=False)


# ---- D: detail objects drawn ----------------------------------------------------------------------------------------

def dds(path: str, size: int, argb: int) -> None:
    """An uncompressed 32-bit DDS of one colour."""
    import struct
    head = struct.pack("<4sIIIIIII44sIIIIIIIIIIIII", b"DDS ", 124, 0x100F, size, size, size * 4, 0, 1, bytes(44),
                       32, 0x41, 0, 32, 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000, 0x1000, 0, 0, 0, 0)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(head + struct.pack("<I", argb) * (size * size))


def phase_d() -> None:
    x, y, z = look_down("D", height=6.0)
    # the details lie on the ground - every static object - by themselves: two grasses painted round the spot
    objects = ["detail\\trava_green", "detail\\trava_black"]
    added = mcp("scene.details", action="add", objects=objects)
    listed = mcp("scene.details", action="list")
    names = [m.get("object") for m in listed.get("meshes") or []]
    check(added.get("ok") and set(objects) <= set(names), "D two detail meshes are the layer's", short(listed))
    hidden = picture("d_no_details")
    painted = mcp("scene.details", action="paint", object=objects[0], position=[x, y, z], radius=6.0, timeout=120)
    check(painted.get("ok") and painted.get("slots", painted.get("slots_changed", 0)) > 0,
          "D a stroke paints the first mesh on the ground (the slot grid laid over it first)", short(painted))
    info = mcp("scene.details")
    check(info.get("ok") and info.get("filled", 0) > 0 and (info.get("slots_per_mesh") or [0])[0] > 0,
          "D the slots under the brush hold the mesh", short(info))
    w, h = hidden.size
    _, share = until_changed("d_details", hidden, (0, h // 4, w, h), level=8, share=0.05)
    check(share > 0.05, "D the picture differs over the ground where the details stand (%.0f %%)" % (share * 100))
    erased = mcp("scene.details", action="erase", position=[x, y, z], radius=6.0)
    after = mcp("scene.details")
    check(erased.get("ok") and after.get("filled", 1) == 0, "D the eraser takes them away again", short(after))
    # a colour-index picture is an import: one opaque colour scatters both meshes over the whole ground
    dds(os.path.join(CTX.project, "gamedata", "textures", "qa", "qa_detail_base.dds"), 64, 0xFF00FF00)
    imported = mcp("scene.details", action="import_texture", texture="qa\\qa_detail_base",
                   colors={"00ff00": objects}, seed=7, timeout=120)
    check(imported.get("ok") and imported.get("filled", 0) > 0, "D a colour-index picture imports as painted slots",
          short(imported))
    mcp("scene.details", action="clear")


# ---- W: a wallmark drawn --------------------------------------------------------------------------------------------

def phase_w() -> None:
    x, y, z = look_down("W", height=6.0)
    # the marks lie on the ground - every static object - by themselves
    st = mcp("scene.wallmarks", action="set", texture="wm\\wm_anarh1", width=2.0, height=2.0, draw=True)
    check(st.get("ok") and st.get("texture") == "wm\\wm_anarh1", "W the Wallmark tool makes marks of a game texture",
          short(st))
    without = picture("w_without")
    added = mcp("scene.wallmarks", action="add", start=[x, y + 5.0, z], dir=[0, -1, 0])
    check(added.get("ok") and added.get("added"), "W a mark is laid on the ground under the spot", short(added))
    mcp("scene.wallmarks", action="select", what="none")
    w, h = without.size
    # 2 m from 6 m up with a 60 degree field: about 0.29 of the height round the middle; the poster fills the
    # middle of its square
    r = int(h * 0.08)
    shown, inner = until_changed("w_mark", without, (w // 2 - r, h // 2 - r, w // 2 + r, h // 2 + r), level=8,
                                 share=0.3)
    outer = changed(without, shown, (0, 0, w, h // 4), level=8)
    check(inner > 0.3, "W the picture shows the mark where it lies (%.0f %% of it changed)" % (inner * 100))
    check(outer < 0.01, "W and nothing else changed (%.2f %% of the top)" % (outer * 100))
    mcp("scene.wallmarks", action="set", draw=False)
    hidden = picture("w_hidden")
    check(changed(without, hidden, (w // 2 - r, h // 2 - r, w // 2 + r, h // 2 + r), level=8) < 0.02,
          "W draw=false hides it again")
    mcp("scene.wallmarks", action="set", draw=True)


def flat(img, box=None) -> float:
    """The brightness spread (standard deviation) of a picture or of `box` of it: a viewport that drew nothing is one
    colour."""
    from PIL import ImageStat
    part = img.crop(box) if box else img
    return ImageStat.Stat(part.convert("L")).stddev[0]


# where a fresh load of Escape puts the camera; the scene fills most of the view from there
ESCAPE_VIEW = {"x": 0.0, "y": 3.0, "z": -10.0, "h": 0.0, "p": 0.0, "b": 0.0}


def phase_s() -> None:
    from PIL import Image
    own_files()
    opened = mcp("scene.open", path=SCENE, discard=True, timeout=300)
    check(opened.get("ok"), "S the scene opens again", short(opened))
    mcp("ui.tab", action="open", doc="level")
    # the view a fresh load of Escape has (the phases before moved it): the picture must be of it
    mcp("camera_set", **ESCAPE_VIEW)
    # one call each, nothing waited for in between: the answer holds until the picture is current
    shot = mcp("shot.window", timeout=60)
    view = [p for p in mcp("ui.list").get("panels", []) if p.get("id") == "viewport"]
    rect = view[0].get("rect") if view else None
    data = base64.b64decode(shot.get("png_base64") or b"")
    spread = 0.0
    if data and rect:
        img = Image.open(io.BytesIO(data)).convert("RGB")
        x, y, w, h = rect
        spread = flat(img, (x + w // 8, y + h // 8, x + w * 7 // 8, y + h * 7 // 8))
        if SHOTS:
            os.makedirs(SHOTS, exist_ok=True)
            img.save(os.path.join(SHOTS, "current_editor.png"))
    check(shot.get("ok") and not shot.get("pending") and spread > 8.0,
          "S the first screenshot_editor after the load shows the scene in the viewport (spread %.1f)" % spread,
          short({k: v for k, v in shot.items() if k != "png_base64"}))
    opened = mcp("scene.open", path=SCENE, discard=True, timeout=300)
    mcp("camera_set", **ESCAPE_VIEW)
    pic = mcp("screenshot_viewport", timeout=60)
    data = base64.b64decode(pic.get("png_base64") or b"")
    spread = flat(Image.open(io.BytesIO(data)).convert("RGB")) if data else 0.0
    check(pic.get("ok") and not pic.get("pending") and spread > 8.0,
          "S the first screenshot_viewport after the load shows the scene (spread %.1f)" % spread,
          short({k: v for k, v in pic.items() if k != "png_base64"}))


def job_of(job_id: int) -> dict:
    return next((j for j in mcp("job.list").get("jobs", []) if j.get("id") == job_id), {})


STEPS = {"F": phase_f, "O": phase_o, "K": phase_k, "X": phase_x, "R": phase_r, "A": phase_a, "L": phase_l, "M": phase_m, "G": phase_g, "P": phase_p, "V": phase_v, "D": phase_d,
         "W": phase_w, "S": phase_s}


def main() -> int:
    global SHOTS
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True)
    ap.add_argument("--only", help="phase letters, e.g. R,G")
    ap.add_argument("--shots")
    args = ap.parse_args()
    SHOTS = args.shots
    only = [p.strip().upper() for p in (args.only or PHASES).split(",") if p.strip()]
    if len(only) == 1 and len(only[0]) > 1:
        only = list(only[0])
    bad = [p for p in only if p not in STEPS]
    only = [p for p in PHASES if p in only]
    if bad:
        print("no such phase: %s (%s)" % (",".join(bad), PHASES))
        return 2
    src = os.path.abspath(args.project)
    if not os.path.isfile(os.path.join(src, "rawdata", "levels", "l01_escape.level")):
        print("the project has no rawdata\\levels\\l01_escape.level: " + src)
        return 2
    try:
        socket.create_connection(MCP, timeout=1).close()
        print(qa_target.busy_text())
        return 2
    except OSError:
        pass
    work = tempfile.mkdtemp(prefix="grizzly_scenetools_")
    kept = {p: open(p, "rb").read() for p in qa_target.KEPT if os.path.isfile(p)}
    proc = None
    try:
        project = CTX.project = qa_target.copy_project(src, os.path.join(work, "project"))
        exe = qa_target.exe()
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        qa_target.guard_project(project)
        proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe),
                                startupinfo=startup)
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            try:
                if mcp("mod.info", timeout=30).get("ok"):
                    break
            except OSError:
                pass
            if proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        qa_target.mark("editor start")
        # the copy's own X-Ray SDK scene: what Save writes lands in the copy
        opened = mcp("scene.open", path=LEVEL, timeout=300)
        mcp("ui.tab", action="open", doc="level")
        if not check(opened.get("ok"), "the Escape scene is open", short(opened)):
            return 1
        refused = mcp("viewport_gesture", kind="click", at=[-5, 10])
        check(not refused.get("ok") and "outside the viewport" in str(refused.get("error")),
              "a point outside the viewport is refused", short(refused))
        gesture(kind="hover", at=[10, 10])
        # the scene's AI map covers the ground: the pictures are of what the phases make
        mcp("exec_command", name="COMMAND_TOGGLE_AIMAP_VISIBILITY")
        qa_target.mark("scene")
        for p in only:
            t = time.monotonic()
            try:
                STEPS[p]()
            except Exception as exc:  # noqa: BLE001 - the report is the point
                import traceback
                traceback.print_exc()
                check(False, "%s ran to its end" % p, repr(exc))
            print("TIME phase %s %.1f s" % (p, time.monotonic() - t), flush=True)
        mcp("ui.tab", action="open", kind="preferences")
        elsewhere = mcp("viewport_gesture", kind="click", at=CTX.centre())
        check(not elsewhere.get("ok") and "Level tab is not the active one" in str(elsewhere.get("error")),
              "a gesture is refused while another tab is up", short(elsewhere))
    finally:
        if proc:
            try:
                mcp("app.quit", discard=True)
            except OSError:
                pass
            try:
                proc.wait(timeout=60)
            except subprocess.TimeoutExpired:
                print("! the editor did not close: ending pid %d" % proc.pid, flush=True)
                proc.kill()
        qa_target.remove_tree(work)
        for p, data in kept.items():
            if open(p, "rb").read() != data:
                open(p, "wb").write(data)
                check(False, "%s came back as it was (put back)" % os.path.basename(p))
    failed = [t for ok, t in results if not ok]
    print("%d/%d passed" % (len(results) - len(failed), len(results)))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
