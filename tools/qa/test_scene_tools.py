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

Every gesture goes through viewport.gesture, which feeds the viewport's own input - the same code a mouse reaches:

  R  random placement: Place mode with objects armed and the scene's random placement on - a click adds one object of
     the random set, its scale and rotation out of the set's ranges, and Place mode stays up for the next click
  A  Attach on Click with spawn points armed: a click on a shape gives it to the selected spawn point; the switch
     goes off
  L  the left button as in Unreal: a left drag that starts on an object moves the camera and selects nothing, a
     click on it selects it when the button comes up; in Place a left drag places nothing
  M  a marquee (Ctrl + drag) selects what its box holds, a light and an object alike
  G  a move drag on the gizmo's X arrow moves the selection along X alone
  Q  a drop: Place panel rows' payloads (an object, a spawn section, the AI Map Bounds volume) and a Content Browser
     entry dropped on the viewport land on the ground under the pointer, each one undo step, selected
  H  the gizmo's other handles: the XZ plane square moves in its plane, the Y ring turns by 90 degrees (the angle
     snap's steps), Local space follows the object's own axes, an Alt drag moves a copy
  B  nothing blocks: a marquee over thousands of objects, a gizmo drag and a flight of the camera hold the UI thread
     under the stall limit (perf.stalls)
  P  the Group tool's Pivot chooser (ui_dialog id=group_pivot) lists the group's objects; align_pivot is its pick
  V  a motion's playback with Camera View: the camera rides on the object while it plays, and stops with it
  D  detail objects painted on the ground (scene.details): two meshes, a stroke - the viewport's picture differs
     over the ground where they stand -, the eraser, a colour-index picture imported
  W  a wallmark of a texture the game has, laid on the ground (scene.wallmarks): the picture differs where it lies,
     and only there
The Level tab's left panel and World Outliner (level.view answers what they draw and sets what a person sets there):

  T  the Place panel: its categories, a search that opens what matches (a spawn section found by its in-game name),
     a row's payload is the viewport's, a click arms Place mode and a second one stops it, Recent keeps it, a category
     folds; typing in the search costs the UI thread nothing
  U  the World Outliner: kinds in the order of their names, an eye and a lock per kind and per object, a way path's
     points By Hierarchy, a selection made elsewhere scrolled into view, 3000 objects selected and the search typed in
     with no stall
  N  the modes' panels: Paint > AI Map (its nodes counted, an AI Map Bounds volume placed from it), Foliage, Decals;
     Zones with its first line; the Colour Indices dialog; pictures of each at 1600 x 900 and 1280 x 720

  S  a screenshot is current: the scene opened again, ONE shot.window and ONE shot.viewport right after it
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
PHASES = "FOKXRALMGQHBPVDWTUNS"
SCENE = "rawdata/levels/l01_escape.scene"
LEVEL = "rawdata/levels/l01_escape.level"
# open ground of Escape east of the village, no tree over it; every phase works round a point of its own
SPOTS = {"Q": (-112.0, -150.0), "H": (-127.0, -140.0), "R": (-119.0, -162.0), "A": (-130.0, -147.0), "L": (-125.0, -156.0), "M": (-100.0, -154.0), "G": (-119.0, -136.0),
         "P": (-101.0, -136.0), "U": (-104.0, -170.0), "V": (-130.0, -166.0), "D": (-110.0, -190.0), "W": (-110.0, -190.0)}
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
    pic = mcp("shot.viewport")
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
    mcp("viewport.camera", action="set", position=[x, y + height, z], rotation=[0.0, -89.99, 0.0])
    return x, y, z


def arm(kind: str) -> None:
    """Place mode with `kind` armed: what a click of Place adds."""
    mcp("scene.mode", mode="place", kind=kind)


def gesture(**kw) -> dict:
    r = mcp("viewport.gesture", **kw)
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
    # the object stands upright: the terrain's normal would tilt it (Align to Surface)
    mcp("viewport.view", surface_snap=True, grid_snap=False, surface_align=False)
    before = picture("r_before")
    g = gesture(kind="click", at=CTX.centre())
    placed = g.get("placed") or []
    o = placed[0] if placed else {}
    check(g.get("ok") and len(placed) == 1, "R a click in Place adds one object", short(g))
    check(o.get("reference") in (BOX, CABINET), "R its reference comes out of the random set", short(o))
    s, rot = o.get("scale") or [0, 0, 0], o.get("rotation") or [9, 9, 9]
    check(0.5 <= s[0] <= 2.0 and s[0] == s[1] == s[2], "R its scale is one factor out of 0.5..2", str(s))
    check(abs(rot[0]) < 1e-2 and abs(rot[2]) < 1e-2 and -1e-2 <= rot[1] <= 180.01,
          "R its rotation turns about Y alone, within 0..180 degrees", str(rot))
    x, _, z = o.get("position") or [0, 0, 0]
    check(abs(x - SPOTS["R"][0]) < 0.5 and abs(z - SPOTS["R"][1]) < 0.5, "R it stands where the click was",
          "%s vs %s" % (o.get("position"), SPOTS["R"]))
    check(g.get("mode") == "place", "R Place mode stays up for the next click", str(g.get("mode")))
    w, h = before.size
    middle = (w // 2 - h // 16, h // 2 - h // 16, w // 2 + h // 16, h // 2 + h // 16)
    _, near = until_changed("r_after", before, middle, share=0.1)
    check(near > 0.1, "R the picture shows it where it was clicked (%.1f %% of the middle changed)" % (near * 100))
    random_placement()
    mcp("scene.mode", mode="select")


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
    check(not mcp("scene.attach", on_click=False).get("on_click"), "A and Attach on Click goes off", str(g.get("mode")))
    mcp("scene.mode", mode="select")


# ---- M: a marquee ---------------------------------------------------------------------------------------------------

def phase_l() -> None:
    x, y, z = look_down("L")
    arm("object")
    box = place(BOX, x, y, z, ground=True)
    deselect()
    mcp("scene.mode", mode="select")
    mcp("viewport.view", tool="select")
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
    arm("object")
    mcp("viewport.view", surface_snap=True, grid_snap=False)
    look_down("L")
    g = gesture(kind="drag", at=[cx - CTX.size[1] * 0.1, cy], to=[cx + CTX.size[1] * 0.1, cy])
    check(g.get("ok") and g.get("camera_moved") and not g.get("placed"),
          "L in Place a left drag moves the camera and places nothing", short(g))
    random_placement()
    mcp("scene.mode", mode="select")


def phase_m() -> None:
    x, y, z = look_down("M")
    names = [place(BOX, x + dx, y, z, ground=True) for dx in (-1.5, 1.5)]
    far = place(BOX, x + 9.0, y, z, ground=True)
    lamp = (mcp("scene.place", kind="light", position=[x, y + 1.0, z + 1.0]).get("object") or {}).get("name")
    names.append(lamp)
    deselect()
    mcp("viewport.view", tool="select")
    cx, cy = CTX.centre()
    # 12 m up with a 60 degree field: a metre is about 0.07 of the height; the box takes 4 m round the spot
    half = CTX.size[1] * 0.18
    g = gesture(kind="drag", at=[cx - half, cy - half], to=[cx + half, cy + half], mods="ctrl")
    picked = set(g.get("picked") or [])
    check(g.get("ok") and set(names) <= picked, "M Ctrl + drag selects what its box holds, the light with the boxes",
          short(g))
    check(far not in picked, "M and not the one outside it", short(g.get("picked")))
    check(not g.get("camera_moved") and not g.get("moved"), "M the camera and the objects stay where they are", short(g))


# ---- G: a gizmo drag ------------------------------------------------------------------------------------------------

def phase_g() -> None:
    x, y, z = look_down("G")
    name = place(BOX, x, y, z, ground=True)
    mcp("scene.select", objects=[name])
    mcp("viewport.view", tool="move", grid_snap=False, space="world")
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
    mcp("viewport.view", tool="select")


# ---- Q: a drop --------------------------------------------------------------------------------------------------------

def phase_q() -> None:
    x, y, z = look_down("Q")
    deselect()
    mcp("viewport.view", surface_snap=True, grid_snap=False, surface_align=False)
    g = gesture(kind="drop", at=CTX.centre(), payload=["object:" + BOX])
    placed = g.get("placed") or []
    o = placed[0] if placed else {}
    px, py, pz = o.get("position") or [0, 0, 0]
    check(g.get("ok") and len(placed) == 1 and o.get("reference") == BOX,
          "Q a Place panel row dropped on the viewport places its object", short(g))
    # the box stands with its pivot where the ray met the ground; a drop of it from 200 m up lands it by its bottom
    check(abs(px - x) < 0.5 and abs(pz - z) < 0.5 and abs(py - y) < 4.0,
          "Q on the ground under the pointer", "%s vs %s" % (o.get("position"), [x, y, z]))
    check(g.get("selection") == [o.get("name")], "Q and the placed object is the selection", short(g.get("selection")))
    undo = mcp("doc.undo", doc="level")
    gone = not any(x.get("name") == o.get("name") for x in mcp("scene.objects", query=o.get("name") or "-").get("objects") or [])
    check(undo.get("ok") and gone, "Q the drop is one undo step", short(undo))
    # what the Content Browser drags: its entries by path and kind; a spawn section lands too
    g = gesture(kind="drop", at=CTX.centre(), assets=[{"path": "rawdata/objects/" + BOX.replace("\\", "/") + ".object",
                                                    "kind": "object"}])
    placed = g.get("placed") or []
    check(g.get("ok") and len(placed) == 1 and (placed[0] or {}).get("reference") == BOX,
          "Q a Content Browser entry dropped on the viewport places it", short(g))
    g = gesture(kind="drop", at=CTX.centre(), payload=["spawn:space_restrictor"])
    placed = g.get("placed") or []
    check(g.get("ok") and placed and placed[0].get("class") == "spawn", "Q a spawn section drops as a spawn point",
          short(g))
    # the Place panel's AI Map Bounds row: a box of that use, dropped like any other row
    g = gesture(kind="drop", at=CTX.centre(), payload=["shape:ai_map_bounds"])
    placed = g.get("placed") or []
    name = placed[0].get("name") if placed else None
    props = json.dumps(mcp("scene.get", objects=[name], keys=["usage"]) if name else {}).lower()
    check(g.get("ok") and placed and placed[0].get("class") == "shape" and ("ai_map_bounds" in props or "ai map bounds" in props),
          "Q the AI Map Bounds row drops as a box of that use", short(g) + " " + props[:200])
    if name:
        mcp("scene.delete", objects=[name])
    bad = mcp("viewport.gesture", kind="drop", at=CTX.centre(), payload=["tree:oak"])
    check(not bad.get("ok") and "kind" in str(bad.get("error")), "Q a payload of no placeable kind is refused", short(bad))


# ---- H: the gizmo's planes, rings, space and Alt ----------------------------------------------------------------------

def handle_drag(name: str, handle: str, dx: float, dy: float, mods=None) -> dict:
    h = gesture(kind="hover", at=CTX.centre())
    at = (h.get("gizmo") or {}).get(handle)
    if not at:
        return {"ok": False, "error": "no handle " + handle, "gizmo": h.get("gizmo")}
    extra = {"mods": mods} if mods else {}
    return gesture(kind="drag", at=at, to=[round(at[0] + dx, 1), round(at[1] + dy, 1)], steps=24, **extra)


def phase_h() -> None:
    x, y, z = look_down("H")
    # a look from the side and above: the XZ square and the Y ring face the camera enough to be held
    mcp("viewport.camera", action="set", position=[x - 6.0, y + 6.0, z - 6.0], rotation=[-45.0, -35.0, 0.0])
    name = place(BOX, x, y, z, ground=True)
    mcp("scene.select", objects=[name])
    mcp("viewport.view", tool="move", grid_snap=False, space="world")
    g = handle_drag(name, "xz", 60.0, 30.0)
    moved = [m for m in g.get("moved") or [] if m.get("name") == name]
    a, b = ((moved[0].get("from") or {}).get("position"), (moved[0].get("to") or {}).get("position")) if moved else ([0] * 3, [0] * 3)
    check(g.get("ok") and moved and math.hypot(b[0] - a[0], b[2] - a[2]) > 0.3,
          "H a drag of the XZ square moves the object", short(g))
    check(moved and abs(b[1] - a[1]) < 1e-3, "H in its plane alone: its height stays", "%s -> %s" % (a, b))
    mcp("doc.undo", doc="level")
    # the Y ring, a quarter turn with the angle snap's steps of 15 degrees
    mcp("viewport.view", tool="rotate", angle_snap=True, angle_step=15)
    h = gesture(kind="hover", at=CTX.centre())
    gz = h.get("gizmo") or {}
    if not check(gz.get("y") and gz.get("pivot"), "H the rotate gizmo has its Y ring and its centre", short(gz)):
        return
    (px, py), (rx, ry) = gz["pivot"], gz["y"]
    # round the pivot from the ring's lowest point a quarter of the way: the pointer sweeps 90 degrees on the screen
    r = math.hypot(rx - px, ry - py)
    steps = 24
    best = None
    for sign in (1, -1):
        to = [round(px + sign * r, 1), round(py, 1)]
        g = gesture(kind="drag", at=[rx, ry], to=to, steps=steps)
        moved = [m for m in g.get("moved") or [] if m.get("name") == name]
        if moved:
            rot = (moved[0].get("to") or {}).get("rotation") or [0, 0, 0]
            best = rot
            mcp("doc.undo", doc="level")
            if abs(abs(rot[1]) - 90.0) < 0.5:
                break
    check(best and abs(abs(best[1]) - 90.0) < 0.5 and abs(best[0]) < 0.5 and abs(best[2]) < 0.5,
          "H the Y ring turns the object a quarter turn about Y, on a step of 15 degrees", short(best))
    # Local space follows the object's own axes: turned 45 degrees, its X arrow is not the world's
    mcp("scene.set", objects=[name], rotation=[0.0, 45.0, 0.0])
    mcp("viewport.view", tool="move", space="local")
    g = handle_drag(name, "x", 60.0, 0.0)
    moved = [m for m in g.get("moved") or [] if m.get("name") == name]
    a, b = ((moved[0].get("from") or {}).get("position"), (moved[0].get("to") or {}).get("position")) if moved else ([0] * 3, [0] * 3)
    d = [b[0] - a[0], b[2] - a[2]]
    check(moved and abs(abs(d[0]) - abs(d[1])) < 0.2 * max(abs(d[0]), 1e-3) and abs(d[0]) > 0.1,
          "H in Local space the X arrow moves along the object's own X, 45 degrees off the world's", str(d))
    mcp("doc.undo", doc="level")
    mcp("viewport.view", space="world")
    # Alt: the drag moves a copy and leaves the object where it stood
    g = handle_drag(name, "x", 60.0, 0.0, mods=["alt"])
    placed = g.get("placed") or []
    check(g.get("ok") and len(placed) == 1 and placed[0].get("name") != name,
          "H an Alt drag of a handle makes a copy", short(g))
    still = position(name)
    check(placed and abs(still[0] - x) < 0.5 and abs(placed[0]["position"][0] - still[0]) > 0.1,
          "H the copy moves, the object stays", "%s / %s" % (still, placed[0].get("position") if placed else None))
    mcp("viewport.view", tool="select")


# ---- B: nothing blocks ------------------------------------------------------------------------------------------------

def phase_b() -> None:
    mcp("perf.stalls", clear=True)
    # high over the level: the box takes most of it
    mcp("viewport.camera", action="set", position=[-30.0, 700.0, -80.0], rotation=[0.0, -89.9, 0.0])
    mcp("viewport.view", tool="select")
    deselect()
    w, h = CTX.size
    g = gesture(kind="drag", at=[10, 50], to=[w - 10, h - 10], mods="ctrl", steps=12)
    check(g.get("ok") and g.get("selected", 0) >= 3000, "B a marquee over the level selects %d objects" % g.get("selected", 0),
          short({k: g.get(k) for k in ("ok", "selected", "error")}))
    stalls_since("B", "a marquee over thousands of objects")
    deselect()
    # a gizmo drag of one object and a flight of the camera
    x, y, z = look_down("G")
    name = place(BOX, x, y, z, ground=True)
    mcp("scene.select", objects=[name])
    mcp("viewport.view", tool="move")
    handle_drag(name, "x", 80.0, 0.0)
    gesture(kind="drag", at=CTX.centre(), to=[CTX.centre()[0] + 200, CTX.centre()[1] + 40], button="right", steps=60)
    gesture(kind="drag", at=CTX.centre(), to=[CTX.centre()[0] - 200, CTX.centre()[1]], button="right", steps=60)
    mcp("viewport.view", tool="select")
    stalls_since("B", "a gizmo drag and a flight of the camera")


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
    refused = mcp("viewport.gesture", kind="click", at=CTX.centre())
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


# ---- T, U, N: the left panel and the World Outliner ------------------------------------------------------------------

def view(**kw) -> dict:
    return mcp("level.view", timeout=120, **kw)


def panel_shot(name: str) -> None:
    if SHOTS:
        pic = mcp("shot.window", timeout=60)
        if pic.get("png_base64"):
            os.makedirs(SHOTS, exist_ok=True)
            with open(os.path.join(SHOTS, name + ".png"), "wb") as f:
                f.write(base64.b64decode(pic["png_base64"]))


def items(v: dict) -> list[dict]:
    return [r for r in (v.get("place") or {}).get("rows") or [] if "item" in r]


def stalls_since(phase: str, what: str) -> None:
    st = mcp("perf.stalls", clear=True)
    check(st.get("ok") and not st.get("stalls"), "%s %s holds the UI thread under %s ms" % (phase, what, st.get("limit_ms")),
          short(st.get("stalls")))


def phase_t() -> None:
    mcp("scene.mode", mode="select")
    mcp("perf.stalls", clear=True)
    v = view(place_search="", limit=200)
    cats = [c.get("id") for c in (v.get("place") or {}).get("categories") or []]
    check(v.get("ok") and v.get("panel") == "Place" and v.get("mode") == "select",
          "T in Select mode the left panel is Place", short(v.get("panel")))
    want = ["objects", "lights", "spawn_points", "shapes", "sounds", "particles", "way_paths", "groups"]
    check([c for c in cats if c != "recent"] == want, "T the categories in their order", json.dumps(cats))
    # one search over everything, folders opened where it matches
    v = view(place_search="cabinet", limit=400)
    rows = items(v)
    check(rows and all("cabinet" in (r["item"] + " " + r["what"]).lower() for r in rows),
          "T a search lists only what matches (%d)" % len(rows), short(rows[:3]))
    check(any(r.get("folder") for r in (v.get("place") or {}).get("rows") or []),
          "T and opens the folders that hold it", short((v.get("place") or {}).get("rows")[:4]))
    # a spawn section by the name the player reads
    v = view(place_search="medkit", limit=400)
    spawns = [r for r in items(v) if r.get("category") == "spawn_points"]
    check(any(r["item"] != r["what"] for r in spawns), "T a spawn point reads its in-game name, the section beside it",
          short(spawns[:3]))
    panel_shot("t_place_search")
    # what a row drags is the viewport's own payload: <kind>:<reference>
    v = view(place_search="Glow")
    glow = [r for r in items(v) if r["item"] == "Glow"]
    check(glow and glow[0].get("payload") == "glow", "T the Glow row drags 'glow'", short(glow))
    cab = [r for r in items(view(place_search=CABINET.split("\\")[-1])) if r.get("what") == CABINET]
    check(cab and cab[0].get("payload") == "object:" + CABINET, "T an object's row drags object:<reference>", short(cab))
    # a click arms it: Place mode with its kind; a second click stops placing
    v = view(place="object", ref=CABINET)
    mode = mcp("scene.mode")
    check(v.get("mode") == "place" and (v.get("place") or {}).get("armed") == "object:" + CABINET
          and mode.get("kind") == "object", "T arming a row puts Place mode up with it", short(v.get("place", {}).get("armed")) + " " + short(mode))
    panel_shot("t_place_armed")
    v = view(place="none", place_search="")
    check(v.get("mode") == "select" and not (v.get("place") or {}).get("armed"), "T place=none stops placing",
          short(v.get("mode")))
    rec = [r for r in (v.get("place") or {}).get("rows") or [] if r.get("category") == "recent" and "item" in r]
    check(rec and rec[0].get("payload") == "object:" + CABINET, "T Recent keeps what was placed last", short(rec[:2]))
    # a category folds and opens again
    v = view(category="lights", open=False)
    lights = [c for c in (v.get("place") or {}).get("categories") or [] if c.get("id") == "lights"]
    check(lights and lights[0].get("open") is False, "T Lights folds", short(lights))
    view(category="lights", open=True)
    bad = view(place="boat")
    check(not bad.get("ok") and "boat" in str(bad.get("error")), "T an unknown kind is refused, naming the kinds", short(bad))
    # typing: every keystroke a new search, worked out off the UI thread
    mcp("perf.stalls", clear=True)
    for i in range(1, 9):
        view(place_search="dynamics\\equipment"[: i * 2])
    view(place_search="")
    stalls_since("T", "typing in the Place panel's search")


def outliner_rows(v: dict) -> list[dict]:
    return (v.get("outliner") or {}).get("rows") or []


def phase_u() -> None:
    mcp("scene.mode", mode="select")
    mcp("scene.select", none=True)
    v = view(outliner_search="", layout="type", kinds=[], limit=10)
    names = [k.get("name") for k in (v.get("outliner") or {}).get("kinds") or []]
    check(names and names == sorted(names), "U the kinds come in the order of their names", json.dumps(names))
    # an eye and a lock per kind
    mcp("scene.set", classes=["light"], hidden=True, locked=True)
    v = view(kinds=["light"], limit=5)
    head = [r for r in outliner_rows(v) if r.get("kind") == "light"]
    check(head and head[0].get("visible") is False and head[0].get("locked") is True,
          "U the Lights row shows its kind hidden and locked", short(head))
    mcp("scene.set", classes=["light"], hidden=False, locked=False)
    # and per object
    x, y, z = look_down("U")
    name = place(BOX, x, y, z, ground=True)
    mcp("scene.set", objects=[name], locked=True)
    v = view(kinds=[], outliner_search=name, limit=5)
    row = [r for r in outliner_rows(v) if r.get("object") == name]
    check(row and row[0].get("locked") is True and row[0].get("visible") is True, "U an object's row shows its own lock",
          short(row))
    mcp("scene.set", objects=[name], locked=False)
    # By Hierarchy: a way path lists its points once it is open
    way = mcp("scene.place", kind="way", points=[[x, y, z], [x + 2, y, z], [x + 4, y, z]])
    wname = (way.get("object") or {}).get("name") or ""
    v = view(layout="hierarchy", outliner_search=wname, unfold=[wname], limit=10)
    pts = [r for r in outliner_rows(v) if "point" in r]
    check(len(pts) == 3 and all(p["point"].startswith(wname + "/") for p in pts),
          "U By Hierarchy a way path lists its points", short(outliner_rows(v)))
    panel_shot("u_outliner_hierarchy")
    view(layout="type", outliner_search="")
    panel_shot("u_outliner_type")
    # a selection made elsewhere scrolls its row into view, once the listing is of the scene as it is
    objs = mcp("scene.objects", classes=["object"], limit=1, offset=3000)
    far = ((objs.get("objects") or [{}])[0]).get("name")
    mcp("scene.select", objects=[far])
    v = view(limit=1)
    o = v.get("outliner") or {}
    for _ in range(10):
        if not o.get("scroll_pending"):
            break
        mcp("shot.window")
        o = view(limit=1).get("outliner") or {}
    check(far and not o.get("scroll_pending") and o.get("scroll_y", 0) > 1000,
          "U a selection made elsewhere scrolls its row into view (%.0f pt)" % o.get("scroll_y", 0), short(o))
    # 3000 objects selected and the search typed in, the Outliner open: nothing holds the UI thread
    mcp("perf.stalls", clear=True)
    mcp("scene.select", all=True, timeout=120)
    mcp("shot.window", timeout=60)
    for t in ("c", "ca", "cab", "cabi", "cabinet"):
        view(outliner_search=t, limit=1)
    view(outliner_search="", limit=1)
    mcp("scene.select", none=True)
    stalls_since("U", "selecting every object and typing in the Outliner's search")


def phase_n() -> None:
    mcp("scene.select", none=True)
    for mode, label in (("ai_map", "AI Map"), ("details", "Foliage"), ("wallmarks", "Decals")):
        mcp("scene.mode", mode=mode)
        v = view(limit=1)
        check(v.get("panel") == "Paint" and (v.get("paint") or {}).get("layer") == label,
              "N Paint > %s: the left panel is its" % label, short(v.get("paint")))
        panel_shot("n_paint_" + mode)
    mcp("scene.mode", mode="ai_map")
    ai = (view(limit=1).get("paint") or {}).get("ai_map") or {}
    check(ai.get("nodes", 0) > 1000, "N the AI map's nodes are counted (%s)" % ai.get("nodes"), short(ai))
    # the Place panel arms the bounds volume like every other row; the AI Map panel's Place Bounds Volume puts
    # one down at once, in front of the camera
    v = view(place="shape", usage="ai_map_bounds")
    check((v.get("place") or {}).get("armed") == "shape:ai_map_bounds", "N the Place panel arms the bounds volume",
          short(v.get("place", {}).get("armed")))
    view(place="none")
    mcp("scene.mode", mode="ai_map")
    before = {o["name"] for o in mcp("scene.objects", classes=["shape"], limit=1000).get("objects") or []}
    mcp("scene.place", kind="shape", box=True, usage="ai_map_bounds", on_surface=True)
    vol = None
    for _ in range(20):
        now = {o["name"] for o in mcp("scene.objects", classes=["shape"], limit=1000).get("objects") or []}
        vol = next(iter(now - before), None)
        if vol:
            break
        time.sleep(0.1)
    ai = (view(limit=1).get("paint") or {}).get("ai_map") or {}
    check(vol and ai.get("bounds") == vol, "N Place Bounds Volume puts an AI Map Bounds volume down that the map lies in",
          short(ai))
    if vol:
        mcp("scene.delete", objects=[vol])
    mcp("scene.mode", mode="zones")
    v = view(limit=1)
    check(v.get("panel") == "Zones" and str((v.get("zones") or {}).get("first_line", "")).startswith("The build makes"),
          "N Zones says first what the build does by itself", short(v.get("zones")))
    panel_shot("n_zones")
    mcp("scene.mode", mode="details")
    v = view(colour_indices=True)
    modal = mcp("shot.window").get("modal") or {}
    check(v.get("colour_indices") is True and modal.get("id") == "dlg.colour_indices",
          "N the Colour Indices dialog comes up over level.view", short(modal))
    panel_shot("n_colour_indices")
    mcp("ui.dialog", action="close", id="dlg.colour_indices")
    view(colour_indices=False)
    # each mode's panel at 1600 x 900 and 1280 x 720
    for w, h in ((1600, 900), (1280, 720)):
        mcp("app.window", action="resize", width=w, height=h)
        for mode in ("select", "ai_map", "details", "wallmarks", "zones"):
            mcp("scene.mode", mode=mode)
            v = view(limit=1)
            panel_shot("n_%dx%d_%s" % (w, h, mode))
            audit = mcp("ui.audit", id="place")
            check(audit.get("ok") and not audit.get("escaped"), "N %d x %d, %s: nothing escapes the left panel" % (w, h, mode),
                  short(audit.get("escaped")))
    mcp("app.window", action="resize", width=1600, height=900)
    mcp("scene.mode", mode="select")
    stalls_since("N", "the modes' panels")


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
    cam0 = (mcp("viewport.camera").get("camera") or {}).get("position")
    still = picture("v_still")
    on = mcp("scene.motion", object=name, action="camera_view", on=True)
    play = mcp("scene.motion", object=name, action="play")
    check(on.get("ok") and (play.get("motion") or {}).get("playing") and (play.get("motion") or {}).get("camera_view"),
          "V Camera View on, the motion plays", short(play))
    time.sleep(0.6)
    _, view = until_changed("v_playing", still, share=0.2)
    cam1 = (mcp("viewport.camera").get("camera") or {}).get("position") or [0, 0, 0]
    obj = position(name)
    check(cam1 != cam0 and x < cam1[0] < x + 10.0, "V the camera rides on the playing object", "%s -> %s" % (cam0, cam1))
    check(abs(obj[0] - x) < 1e-3, "V and the scene keeps the object where it stands until the pause", str(obj))
    check(view > 0.2, "V the viewport shows the view from the object (%.0f %% changed)" % (view * 100))
    paused = mcp("scene.motion", object=name, action="pause")
    cam2 = (mcp("viewport.camera").get("camera") or {}).get("position")
    time.sleep(0.3)
    cam3 = (mcp("viewport.camera").get("camera") or {}).get("position")
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
ESCAPE_VIEW = {"position": [0.0, 3.0, -10.0], "rotation": [0.0, 0.0, 0.0]}


def phase_s() -> None:
    from PIL import Image
    own_files()
    opened = mcp("scene.open", path=SCENE, discard=True, timeout=300)
    check(opened.get("ok"), "S the scene opens again", short(opened))
    mcp("ui.tab", action="open", doc="level")
    # the view a fresh load of Escape has (the phases before moved it): the picture must be of it
    mcp("viewport.camera", action="set", **ESCAPE_VIEW)
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
          "S the first shot.window after the load shows the scene in the viewport (spread %.1f)" % spread,
          short({k: v for k, v in shot.items() if k != "png_base64"}))
    opened = mcp("scene.open", path=SCENE, discard=True, timeout=300)
    mcp("viewport.camera", action="set", **ESCAPE_VIEW)
    pic = mcp("shot.viewport", timeout=60)
    data = base64.b64decode(pic.get("png_base64") or b"")
    spread = flat(Image.open(io.BytesIO(data)).convert("RGB")) if data else 0.0
    check(pic.get("ok") and not pic.get("pending") and spread > 8.0,
          "S the first shot.viewport after the load shows the scene (spread %.1f)" % spread,
          short({k: v for k, v in pic.items() if k != "png_base64"}))


def job_of(job_id: int) -> dict:
    return next((j for j in mcp("job.list").get("jobs", []) if j.get("id") == job_id), {})


STEPS = {"F": phase_f, "O": phase_o, "K": phase_k, "X": phase_x, "R": phase_r, "A": phase_a, "L": phase_l, "M": phase_m, "G": phase_g, "Q": phase_q, "H": phase_h, "B": phase_b, "P": phase_p, "V": phase_v, "D": phase_d,
         "W": phase_w, "T": phase_t, "U": phase_u, "N": phase_n, "S": phase_s}


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
        stalls_since("the open", "opening Escape (6000 objects)")
        refused = mcp("viewport.gesture", kind="click", at=[-5, 10])
        check(not refused.get("ok") and "outside the viewport" in str(refused.get("error")),
              "a point outside the viewport is refused", short(refused))
        gesture(kind="hover", at=[10, 10])
        # the scene's AI map covers the ground: the pictures are of what the phases make
        mcp("viewport.view", hide=["ai_map"])
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
        elsewhere = mcp("viewport.gesture", kind="click", at=CTX.centre())
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
