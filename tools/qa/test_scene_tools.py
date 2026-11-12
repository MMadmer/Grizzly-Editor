#!/usr/bin/env python3
"""The viewport's gestures and the scene tools' pictures, over MCP on a THROW-AWAY COPY of a project with the Escape
scene (rawdata\\levels\\l01_escape.level).

Every gesture goes through viewport_gesture, which feeds the viewport's own input - the same code a mouse reaches:

  R  Random Append: the Object tool in Place with Randomize Added Objects on - a click adds one object of the random
     set, its scale and rotation out of the set's ranges
  A  Attach on Click of the Spawn tool: a click on a shape gives it to the selected spawn point; the switch goes off
  M  a marquee (Ctrl + drag) selects what its box holds
  G  a move drag on the gizmo's X arrow moves the selection along X alone
  P  the Group tool's Pivot chooser (ui_dialog id=group_pivot) lists the group's objects; align_pivot is its pick
  V  a motion's playback with Camera View: the camera rides on the object while it plays, and stops with it
  D  detail objects drawn: a snap list, a detail object list, a base texture, First Initialize, Reinitialize - the
     viewport's picture differs over the ground against the same view with the details hidden
  W  a wallmark of a texture the game has, laid on the ground: the picture differs where it lies, and only there
  S  a screenshot is current: the scene opened again, ONE screenshot_editor and ONE screenshot_viewport right after it
     show the scene (the viewport is not flat) and say nothing is pending
  C  the level compiler in the editor's process: mp_test (the repository's smallest scene) built by Make All, a
     compile started without waiting and stopped (the job ends cancelled), then compiled whole over MCP (the job
     ends done, its report names the phases and the files, the files are in the level folder)
  Y  Play takes no folder it did not make: play_clean, play_root and play name as root= a throw-away project folder
     (its own gamedata, fsgame.ltx, a level), a folder inside it, a folder that holds a file, a folder inside the open
     project, the editor's own folder and a relative path - every one refused before anything is built or written,
     and a listing of the work folder before and after is equal; no game is started. Then a root Play makes
     itself (play_root in an empty temp folder) gets a project in its gamedata: play_root, play_clean and play
     refuse it, naming the project, and its listing is unchanged

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
import shutil
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
MCP = qa_target.MCP
PHASES = "RAMGPVDWSCY"
# open ground of Escape east of the village, no tree over it; every phase works round a point of its own
SPOTS = {"R": (-119.0, -162.0), "A": (-130.0, -147.0), "M": (-100.0, -154.0), "G": (-119.0, -136.0),
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
    placed = mcp("place_object", ref=BOX, x=x, y=200.0, z=z, snap_to_ground=True)
    name = (placed.get("object") or {}).get("name")
    y = position(name)[1]
    mcp("select_objects", names=name)
    mcp("delete_selected")
    return float(y)


def position(name: str) -> list[float]:
    return (mcp("object_info", name=name).get("object") or {}).get("position") or [0.0, 0.0, 0.0]


def look_down(phase: str, height: float = 12.0) -> tuple[float, float, float]:
    """The camera straight over the phase's spot: the spot is the viewport's centre."""
    x, z = SPOTS[phase]
    y = CTX.ground.get(phase)
    if y is None:
        y = CTX.ground[phase] = ground_y(x, z)
    mcp("camera_set", x=x, y=y + height, z=z, h=0.0, p=-math.pi / 2 + 1e-4, b=0.0)
    return x, y, z


def target(class_id: int) -> None:
    mcp("exec_command", name="COMMAND_CHANGE_TARGET", p1i=class_id)


def gesture(**kw) -> dict:
    r = mcp("viewport_gesture", **kw)
    if r.get("ok") and r.get("viewport"):
        CTX.size = tuple(r["viewport"])
    return r


def place(ref: str, x: float, y: float, z: float, **kw) -> str:
    r = mcp("place_object", ref=ref, x=x, y=y, z=z, **kw)
    return (r.get("object") or {}).get("name") or r.get("name") or ""


def deselect() -> None:
    mcp("deselect_all")


# ---- R: Random Append -----------------------------------------------------------------------------------------------

def phase_r() -> None:
    look_down("R", height=5.0)
    target(2)
    deselect()
    rnd = mcp("object_tool", action="random", on=True, objects=[BOX, CABINET], scale=True, proportional=True,
              min_scale=0.5, max_scale=2.0, rotate=True, min_rotation=[0, 0, 0], max_rotation=[0, 180, 0])
    r = rnd.get("random") or {}
    check(rnd.get("ok") and r.get("on") and r.get("objects") == [BOX, CABINET] and r.get("max_rotation") == [0, 180, 0],
          "R object_tool action=random sets Randomize Added Objects and its ranges", short(rnd))
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
    mcp("object_tool", action="random", on=False)
    mcp("viewport_controls", normal_align=True)


# ---- A: Attach on Click ---------------------------------------------------------------------------------------------

def phase_a() -> None:
    x, y, z = look_down("A")
    deselect()
    shape = mcp("shape_tool", action="add", position="%f,%f,%f" % (x, y + 0.5, z), shape="box", show=True)
    shape_name = next((s.get("name") for s in (shape.get("selected") or [])), "")
    spawn = mcp("place_spawn", section="space_restrictor", x=x + 3.0, y=y, z=z)
    spawn_name = (spawn.get("object") or {}).get("name") or ""
    check(shape.get("ok") and spawn.get("ok") and spawn_name, "A a shape and a spawn point to attach it to",
          short(shape) + " " + short(spawn))
    mcp("select_objects", names=spawn_name)
    on = mcp("spawn_attach", action="on_click", on=True)
    check(on.get("attach_on_click") is True, "A spawn_attach action=on_click turns Attach on Click on", short(on))
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
    check(g.get("tool") == "select" and not mcp("spawn_attach", action="on_click", on=False).get("attach_on_click"),
          "A the tool goes back to Select and Attach on Click goes off", str(g.get("tool")))


# ---- M: a marquee ---------------------------------------------------------------------------------------------------

def phase_m() -> None:
    x, y, z = look_down("M")
    target(2)
    names = [place(BOX, x + dx, y, z, snap_to_ground=True) for dx in (-1.5, 1.5)]
    far = place(BOX, x + 9.0, y, z, snap_to_ground=True)
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
    target(2)
    name = place(BOX, x, y, z, snap_to_ground=True)
    mcp("select_objects", names=name)
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
    undo = mcp("undo")
    back = position(name)
    check(undo.get("ok") and abs(back[0] - a[0]) < 1e-3, "G the drag is one undo step", short(undo) + " " + short(back))
    mcp("viewport_controls", tool="select")


# ---- P: the Group tool's pivot --------------------------------------------------------------------------------------

def phase_p() -> None:
    x, y, z = look_down("P")
    a = place(BOX, x - 2.0, y, z, snap_to_ground=True)
    b = place(BOX, x + 2.0, y, z, snap_to_ground=True, ry=0.5)
    mcp("select_objects", names="%s,%s" % (a, b))
    grp = mcp("group_tool", action="group")
    gname = (grp.get("object") or {}).get("name") or ""
    check(grp.get("ok") and grp.get("members") == 2, "P group_tool action=group makes a group of the two", short(grp))
    mcp("select_objects", names=gname)
    opened = mcp("ui_dialog", action="open", id="group_pivot")
    pic = mcp("screenshot_editor")
    modal = pic.get("modal") or {}
    check(opened.get("ok") and modal.get("id") == "dlg.choose" and "Object" in str(modal.get("title")),
          "P ui_dialog open id=group_pivot puts the Pivot chooser up", short(modal) + " " + short(opened))
    if SHOTS and pic.get("png_base64"):
        os.makedirs(SHOTS, exist_ok=True)
        with open(os.path.join(SHOTS, "p_chooser.png"), "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
    audit = mcp("ui_audit", id="dlg.choose")
    controls = {str(c.get("label")): c for c in audit.get("controls") or []}
    check(audit.get("ok") and not audit.get("escaped") and (controls.get("Select") or {}).get("enabled") is False
          and "Cancel" in controls, "P its Select waits for a pick, and nothing leaves it", short(sorted(controls)))
    refused = mcp("viewport_gesture", kind="click", at=CTX.centre())
    check(not refused.get("ok") and "a dialog is up" in str(refused.get("error")), "P a gesture is refused while it is up",
          short(refused))
    mcp("ui_dialog", action="close", id="dlg.choose")
    check(not mcp("screenshot_editor").get("modal"), "P it closes as Cancel does")
    aligned = mcp("group_tool", action="align_pivot", object=b)
    gp, bp = position(gname), position(b)
    check(aligned.get("changed") == 1 and all(abs(p - q) < 1e-3 for p, q in zip(gp, bp)),
          "P its pick, align_pivot object=, puts the group's pivot on that object", "%s vs %s" % (gp, bp))
    no_group = mcp("deselect_all") and mcp("ui_dialog", action="open", id="group_pivot")
    check(not no_group.get("ok") and "Select a group" in str(no_group.get("error")),
          "P without a selected group it says so", short(no_group))


# ---- V: playback with Camera View -----------------------------------------------------------------------------------

def phase_v() -> None:
    x, y, z = look_down("V", height=20.0)
    name = place(BOX, x, y + 1.0, z)
    # a key where it stands at 0 s; then at 2 s, 10 m east and 3 m up (a frame change poses the object by the
    # motion, so the key comes first and the move after it)
    steps = [mcp("object_motion", name=name, action="motionable", on=True),
             mcp("object_motion", name=name, action="add_key"),
             mcp("object_motion", name=name, action="current", t=2.0)]
    steps.append(mcp("object_set_position", name=name, x=x + 10.0, y=y + 4.0, z=z))
    steps += [mcp("object_motion", name=name, action="add_key"),
              mcp("object_motion", name=name, action="range", min=0.0, max=2.0),
              mcp("object_motion", name=name, action="first")]
    st = mcp("object_motion", name=name)
    check(all(s.get("ok") for s in steps) and (st.get("motion") or {}).get("keys", 0) >= 2,
          "V a motion of two keys, two seconds long", short([s for s in steps if not s.get("ok")] or st))
    cam0 = (mcp("camera_get").get("camera") or {}).get("position")
    still = picture("v_still")
    on = mcp("object_motion", name=name, action="camera_view", on=True)
    play = mcp("object_motion", name=name, action="play")
    check(on.get("ok") and (play.get("motion") or {}).get("playing") and (play.get("motion") or {}).get("camera_view"),
          "V Camera View on, the motion plays", short(play))
    time.sleep(0.6)
    _, view = until_changed("v_playing", still, share=0.2)
    cam1 = (mcp("camera_get").get("camera") or {}).get("position") or [0, 0, 0]
    obj = position(name)
    check(cam1 != cam0 and x < cam1[0] < x + 10.0, "V the camera rides on the playing object", "%s -> %s" % (cam0, cam1))
    check(abs(obj[0] - x) < 1e-3, "V and the scene keeps the object where it stands until the pause", str(obj))
    check(view > 0.2, "V the viewport shows the view from the object (%.0f %% changed)" % (view * 100))
    paused = mcp("object_motion", name=name, action="pause")
    cam2 = (mcp("camera_get").get("camera") or {}).get("position")
    time.sleep(0.3)
    cam3 = (mcp("camera_get").get("camera") or {}).get("position")
    m = paused.get("motion") or {}
    check(paused.get("ok") and not m.get("playing") and 0.0 < m.get("current", 0) <= 2.0,
          "V pause stops it at the time it reached", short(paused))
    check(cam2 == cam3, "V and the camera stops with it", "%s -> %s" % (cam2, cam3))
    mcp("object_motion", name=name, action="camera_view", on=False)


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
    look_down("D", height=6.0)
    # a pick makes the picked object's tool current: the Detail Objects tool comes after it
    mcp("select_objects", names=TERRAIN)
    mcp("details_tool", show=True)
    snap = mcp("snap_objects", action="set_from_selection")
    deselect()
    check(snap.get("ok") and TERRAIN in (snap.get("objects") or []), "D the Detail Objects' snap list holds the terrain",
          short(snap))
    # the base texture: one opaque colour over the whole level, which the two grasses scatter
    dds(os.path.join(CTX.project, "gamedata", "textures", "qa", "qa_detail_base.dds"), 64, 0xFF00FF00)
    objects = ["detail\\trava_green", "detail\\trava_black"]
    added = mcp("details_tool", action="add_objects", objects=objects)
    base = mcp("details_tool", action="set", base_texture="qa\\qa_detail_base", draw_objects=False)
    first = mcp("details_tool", action="first_initialize", seed=7, timeout=120)
    colour = mcp("details_tool", action="set_color", color="ff00ff00", objects=objects)
    state = mcp("details_tool")
    check(added.get("ok") and base.get("ok") and colour.get("ok") and set(objects) <= set(state.get("objects") or [])
          and state.get("base_texture") == "qa\\qa_detail_base", "D a detail object list, a base texture and its colour",
          short(state))
    check(first.get("ok") and first.get("slots", 0) > 0, "D First Initialize lays the slots", short(first))
    again = mcp("details_tool", action="reinitialize", seed=7, timeout=120)
    check(again.get("ok") and again.get("filled", 0) == again.get("slots", -1),
          "D Reinitialize gives every slot its objects", short(again))
    hidden = picture("d_no_details")
    shown_on = mcp("details_tool", action="update_renderer")
    w, h = hidden.size
    _, share = until_changed("d_details", hidden, (0, h // 4, w, h), level=8, share=0.2)
    check(shown_on.get("draw_objects") is True and share > 0.2,
          "D the picture differs over the ground where the details stand (%.0f %%)" % (share * 100))
    mcp("details_tool", action="set", draw_base_texture=True)
    deadline = time.monotonic() + 6.0
    while True:
        small = picture("d_base_texture").resize((64, 64))
        green = sum(1 for y in range(64) for x in range(64)
                    if (lambda px: px[1] > 200 and px[0] < 80 and px[2] < 80)(small.getpixel((x, y))))
        # the texture loads beside the pass
        if green > 64 * 64 * 0.05 or time.monotonic() > deadline:
            break
        time.sleep(0.1)
    mcp("details_tool", action="set", draw_base_texture=False, draw_objects=False)
    check(green > 64 * 64 * 0.05, "D the base texture is drawn over the ground in its colour (%d of 4096)" % green)


# ---- W: a wallmark drawn --------------------------------------------------------------------------------------------

def phase_w() -> None:
    x, y, z = look_down("W", height=6.0)
    # the marks lie on the scene's snap list: the terrain, with the Object tool current
    mcp("select_objects", names=TERRAIN)
    target(2)
    mcp("snap_objects", action="set_from_selection")
    mcp("snap_objects", use=True)
    deselect()
    st = mcp("wallmark_tool", action="set", show=True, texture="wm\\wm_anarh1", width=2.0, height=2.0, draw=True)
    check(st.get("ok") and st.get("texture") == "wm\\wm_anarh1", "W the Wallmark tool makes marks of a game texture",
          short(st))
    without = picture("w_without")
    added = mcp("wallmark_tool", action="add", start="%f,%f,%f" % (x, y + 5.0, z), dir="0,-1,0")
    check(added.get("ok") and added.get("added"), "W a mark is laid on the ground under the spot", short(added))
    mcp("wallmark_tool", action="deselect_all")
    w, h = without.size
    # 2 m from 6 m up with a 60 degree field: about 0.29 of the height round the middle; the poster fills the
    # middle of its square
    r = int(h * 0.08)
    shown, inner = until_changed("w_mark", without, (w // 2 - r, h // 2 - r, w // 2 + r, h // 2 + r), level=8,
                                 share=0.3)
    outer = changed(without, shown, (0, 0, w, h // 4), level=8)
    check(inner > 0.3, "W the picture shows the mark where it lies (%.0f %% of it changed)" % (inner * 100))
    check(outer < 0.01, "W and nothing else changed (%.2f %% of the top)" % (outer * 100))
    mcp("wallmark_tool", action="set", draw=False)
    hidden = picture("w_hidden")
    check(changed(without, hidden, (w // 2 - r, h // 2 - r, w // 2 + r, h // 2 + r), level=8) < 0.02,
          "W draw=false hides it again")
    mcp("wallmark_tool", action="set", draw=True)


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
    opened = mcp("open_scene", scene="l01_escape.level", discard=True, timeout=300)
    check(opened.get("ok"), "S the scene opens again", short(opened))
    mcp("ui_tab", action="open", id="level")
    # the view a fresh load of Escape has (the phases before moved it): the picture must be of it
    mcp("camera_set", **ESCAPE_VIEW)
    # one call each, nothing waited for in between: the answer holds until the picture is current
    shot = mcp("screenshot_editor", timeout=60)
    view = [p for p in mcp("ui_list").get("panels", []) if p.get("id") == "viewport"]
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
    opened = mcp("open_scene", scene="l01_escape.level", discard=True, timeout=300)
    mcp("camera_set", **ESCAPE_VIEW)
    pic = mcp("screenshot_viewport", timeout=60)
    data = base64.b64decode(pic.get("png_base64") or b"")
    spread = flat(Image.open(io.BytesIO(data)).convert("RGB")) if data else 0.0
    check(pic.get("ok") and not pic.get("pending") and spread > 8.0,
          "S the first screenshot_viewport after the load shows the scene (spread %.1f)" % spread,
          short({k: v for k, v in pic.items() if k != "png_base64"}))


def job_of(job_id: int) -> dict:
    return next((j for j in mcp("jobs").get("jobs", []) if j.get("id") == job_id), {})


def phase_c() -> None:
    scene = "mp_test"
    levels = os.path.join(CTX.project, "rawdata", "levels")
    shutil.copy2(os.path.join(qa_target.REPO, "rawdata", "levels", scene + ".level"), levels)
    shutil.copytree(os.path.join(qa_target.REPO, "rawdata", "levels", scene), os.path.join(levels, scene),
                    dirs_exist_ok=True)
    opened = mcp("open_scene", scene=scene + ".level", discard=True, timeout=120)
    if not check(opened.get("ok"), "C the small scene opens", short(opened)):
        return
    built = mcp("level_build", action="make_all", timeout=120)
    if not check(built.get("ok"), "C Make All writes its build", short(built)):
        return
    started = mcp("level_compile", wait=False)
    stopped = mcp("level_build", action="stop")
    job = started.get("job")
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and job_of(job).get("state") == "running":
        time.sleep(0.1)
    j = job_of(job)
    check(started.get("ok") and started.get("started") and stopped.get("stopped") and j.get("state") == "cancelled",
          "C a compile started without waiting is stopped by the level build's stop (state %s)" % j.get("state"),
          short({"started": started, "stopped": stopped, "job": j}))
    t = time.monotonic()
    done = mcp("level_build", action="compile", timeout=120)
    seconds = time.monotonic() - t
    report = done.get("report") or {}
    parts = report.get("parts") or [{}]
    phases = [p.get("name") for p in parts[0].get("phases", [])]
    files = {f.get("name") for f in parts[0].get("files", [])}
    level_dir = os.path.join(CTX.project, "gamedata", "levels", scene)
    there = all(os.path.isfile(os.path.join(level_dir, n)) for n in ("level", "level.geom", "level.geomx",
                                                                     "level.cform", "build.cform", "build.lights"))
    j = job_of(done.get("job"))
    check(done.get("ok") and j.get("state") == "done" and report.get("threads", 0) >= 1,
          "C the compile over MCP ends done (%.1f s, %s threads)" % (seconds, report.get("threads")),
          short({k: v for k, v in done.items() if k != "report"}))
    check("LIGHT: LMaps..." in phases and "Converting to OGFs..." in phases and {"level", "level.geom"} <= files,
          "C its report names the compiler's phases and the files (%d phases, %d files)" % (len(phases), len(files)),
          short(parts[0]))
    check(there, "C the level's files are in the project's level folder",
          ", ".join(sorted(os.listdir(level_dir))) if os.path.isdir(level_dir) else "no folder")


# ---- Y: Play takes no folder it did not make --------------------------------------------------------------------------

def tree_of(top: str) -> dict:
    """Every entry under `top` with its size and write time; a link is listed, never followed."""
    out = {}
    for d, dirs, files in os.walk(top):
        keep = []
        for n in dirs:
            p = os.path.join(d, n)
            st = os.lstat(p)
            out[os.path.relpath(p, top)] = (0, st.st_mtime_ns)
            if not (os.path.islink(p) or getattr(st, "st_reparse_tag", 0)):
                keep.append(n)
        dirs[:] = keep
        for n in files:
            st = os.lstat(os.path.join(d, n))
            out[os.path.relpath(os.path.join(d, n), top)] = (st.st_size, st.st_mtime_ns)
    return out


def phase_y() -> None:
    work = os.path.dirname(CTX.project)
    # the reviewer's stand-in for a project named as root=: its own gamedata, fsgame.ltx and a level
    victim = os.path.join(work, "victim")
    os.makedirs(os.path.join(victim, "gamedata", "levels", "my_level"))
    for rel, text in (("mod.ltx", "[module]\nid = victim\nname = Victim\n"), ("fsgame.ltx", "; the folder's own\n"),
                      ("gamedata\\precious.txt", "the author's work\n"),
                      ("gamedata\\levels\\my_level\\level.ltx", "the author's level\n")):
        with open(os.path.join(victim, rel), "w") as f:
            f.write(text)
    # a folder that is no project but holds something, and a folder inside the open project
    busy = os.path.join(work, "busy")
    os.makedirs(busy)
    with open(os.path.join(busy, "notes.txt"), "w") as f:
        f.write("mine\n")
    inside = os.path.join(CTX.project, "temp", "play_here")
    # the folders named (the project itself is the editor's to write meanwhile)
    listed = lambda: {k: tree_of(k) for k in (victim, busy)}  # noqa: E731
    before = listed()
    cases = (
        (victim, "it is a project folder"),
        (os.path.join(victim, "gamedata"), "it lies inside the project"),
        (busy, "is not a Play root: choose an empty folder"),
        (inside, "it lies inside the project"),
        (qa_target.REPO, "the editor"),
        ("relative\\root", "by a full path"),
    )
    for root, said in cases:
        for action in ("play_clean", "play_root", "play"):
            a = mcp("level_build", action=action, root=root, timeout=60)
            check(not a.get("ok") and said in str(a.get("error")),
                  "Y level_build %s root=<%s> is refused (%s)" % (action, os.path.basename(root), said), short(a))
    after = listed()
    differ = sorted("%s\\%s" % (os.path.basename(top), k) for top in before
                    for k in set(before[top]) | set(after[top]) if before[top].get(k) != after[top].get(k))
    count = sum(len(v) for v in before.values())
    check(not differ and not os.path.exists(inside),
          "Y the folders are as they were (%d entries listed before and after)" % count, ", ".join(differ[:10]))
    st = mcp("level_build", action="status")
    check(st.get("ok") and not st.get("running") and not st.get("building"), "Y nothing was started", short(st))
    phase_y_marked()


def phase_y_marked() -> None:
    """A root Play made (it carries play_root.txt) that holds a project inside its gamedata: refused, untouched."""
    # right in the temp folder: the game takes paths of 255 characters at most
    root = tempfile.mkdtemp(prefix="gzpr_")
    try:
        made = mcp("level_build", action="play_root", root=root, timeout=120)
        if not check(made.get("ok") and made.get("was") == "empty",
                     "Y play_root makes Play's root in an empty folder (%s programs)" % made.get("programs"),
                     short(made)):
            return
        again = mcp("level_build", action="play_root", root=root, timeout=120)
        check(again.get("ok") and again.get("was") == "play_root" and again.get("walked", 0) > 0,
              "Y the root again: its gamedata walked for a project before it is cleared (%s entries, %s ms)"
              % (again.get("walked"), again.get("walk_ms")), short(again))
        nested = os.path.join(root, "gamedata", "mods", "my_mod")
        os.makedirs(nested)
        with open(os.path.join(nested, "mod.ltx"), "w") as f:
            f.write("[module]\nid = my_mod\n")
        before = tree_of(root)
        for action in ("play_root", "play_clean", "play"):
            a = mcp("level_build", action=action, root=root, timeout=60)
            check(not a.get("ok") and "it holds a project" in str(a.get("error")) and "my_mod" in str(a.get("error")),
                  "Y level_build %s on a Play root whose gamedata holds a project is refused" % action, short(a))
        after = tree_of(root)
        differ = sorted(k for k in set(before) | set(after) if before.get(k) != after.get(k))
        check(not differ, "Y the root is as it was (%d entries listed before and after)" % len(before),
              ", ".join(differ[:10]))
    finally:
        link = os.path.join(root, "database")
        if os.path.lexists(link):
            # a junction to the game's archives: the link goes, the archives stay
            os.rmdir(link)
        qa_target.remove_tree(root)


STEPS = {"R": phase_r, "A": phase_a, "M": phase_m, "G": phase_g, "P": phase_p, "V": phase_v, "D": phase_d,
         "W": phase_w, "S": phase_s, "C": phase_c, "Y": phase_y}


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
                if mcp("mod_manifest", timeout=30).get("ok"):
                    break
            except OSError:
                pass
            if proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        qa_target.mark("editor start")
        opened = mcp("open_scene", scene="l01_escape.level", timeout=300)
        mcp("ui_tab", action="open", id="level")
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
        mcp("ui_tab", action="open", kind="preferences")
        elsewhere = mcp("viewport_gesture", kind="click", at=CTX.centre())
        check(not elsewhere.get("ok") and "Level tab is not the active one" in str(elsewhere.get("error")),
              "a gesture is refused while another tab is up", short(elsewhere))
    finally:
        if proc:
            try:
                mcp("window_control", action="close", discard=True)
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
