#!/usr/bin/env python3
"""End-to-end check of the Content Browser's Library and the preview and library tabs (docs/UI_DESIGN.md,
section 6: Content Browser, Model Preview, Texture Preview, Libraries) over MCP, on a THROW-AWAY COPY of a
project. Nothing of the editor's libraries is changed: the library tabs are opened, searched and looked at,
never saved.

What it proves:
  C  content.view: an .object with its .thm is ONE tile, in the project and in the Library; a folder of
     thumbnails alone is empty; the Library lists its kinds; a Library spawn section and object drag as the
     kit's payloads of their kinds and say what scene.place and file.import take; Bring into Project
     (file.import) brings an object with its thumbnail in; perf.stalls stays empty while a list of 300 and
     more objects is scrolled through and its thumbnails come in
  T  preview.texture: a project image, a library texture and a game one open in the texture tab, each
     naming where it lives (origin) apart from its path; fit, 1:1 and a
     factor; the channel views RGBA, R, G, B and A (rgb still names RGBA); a mip; the Details pane on and off
     and keeping its pt size; a bad channel or zoom is refused; close frees the tab
  M  preview.model: a model (the editor's objects, else the linked game's meshes) opens in the model tab,
     reports its geometry, switches Studio / Unlit, wireframe, bones and Details, frames and resets; a character
     of the linked game names its textures, none of them missing (the loader reads them out of the game)
     T and M: an opener's answer is the frame it opened (panes in pt, the zoom a fit came to), not the one before
  L  library.view: Light Animations, Texture Library and Sound Library open as tabs, count and search their items,
     select one, answer state without switching tabs, and stay clean; each names where it reads from
  S  with --sizes: every tab again at 1707x912 / ui scale 1.5 and 1280x720 / 1.0, nothing escaping its window

    python tools/qa/test_previews.py --project <project> [--shots <folder>] [--sizes]
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP
# a picture that ships with the editor: the PDA's texture sheet
SHEET = os.path.join(REPO, "gamedata", "configs", "pda", "ui_xms_pda.dds")

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def note(text: str) -> None:
    print("NOTE " + text, flush=True)


def mcp(cmd: str, timeout: float = 600.0, **extra) -> dict:
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


class Editor:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self, project: str) -> None:
        exe = qa_target.exe()
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        qa_target.guard_project(project)
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                if mcp("mod.info", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("app.quit", discard=True)
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def shot(folder: str | None, name: str) -> dict:
    # a screenshot is also how a frame gets drawn in a minimized editor: the tab lays itself out in it
    pic = mcp("shot.window")
    if folder and pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)
    return pic


def escaped_in_tab() -> list | None:
    """What the kit saw spill out of the work area's windows last frame; None when ui_audit is not there."""
    a = mcp("ui.audit")
    if not isinstance(a.get("escaped"), list):
        return None
    return [e for e in a["escaped"] if "doc.host" in str(e.get("window", ""))]


def audit(label: str) -> None:
    esc = escaped_in_tab()
    if esc is None:
        note("ui_audit is not there: " + label + " not audited")
        return
    check(esc == [], label + ": nothing escapes the tab", json.dumps(esc)[:400])


def active_tab() -> str:
    return str(mcp("state").get("active_tab", ""))


# ---------------------------------------------------------------------------------------------------- C
LIB = os.path.join(REPO, "rawdata", "objects")


def entries_of(answer: dict) -> list[dict]:
    return answer.get("entries", []) if answer.get("ok") else []


def run_content(project: str, shots: str | None) -> None:
    # an object of the library with its thumbnail, copied into the project the way it lives there
    src = os.path.join(LIB, "corps")
    if not os.path.isfile(os.path.join(src, "corp_dog.object")):
        note("the editor's library has no corps\\corp_dog.object: the content browser is not checked")
        return
    dst = os.path.join(project, "rawdata", "objects", "qa_one")
    os.makedirs(dst, exist_ok=True)
    for ext in (".object", ".thm"):
        shutil.copyfile(os.path.join(src, "corp_dog" + ext), os.path.join(dst, "corp_dog" + ext))
    os.makedirs(os.path.join(project, "rawdata", "objects", "qa_thumbs"), exist_ok=True)
    shutil.copyfile(os.path.join(src, "corp_dog.thm"), os.path.join(project, "rawdata", "objects", "qa_thumbs", "only.thm"))
    v = mcp("content.view", action="go", source="project", folder="rawdata/objects/qa_one")
    names = [e.get("path") for e in entries_of(v)]
    check(names == ["rawdata\\objects\\qa_one\\corp_dog.object"], "C an .object with its .thm is one tile in the project",
          json.dumps(v)[:400])
    one = (entries_of(v) or [{}])[0]
    check(one.get("picture") == "model" and (one.get("place") or {}).get("ref") == "qa_one\\corp_dog",
          "C its picture is the object's and Place in Level names its reference", json.dumps(one)[:300])
    v = mcp("content.view", action="go", source="project", folder="rawdata/objects/qa_thumbs")
    check(v.get("ok") and v.get("total") == 0, "C a folder of thumbnails alone is empty", json.dumps(v)[:300])
    shot(shots, "content_project_object")

    home = mcp("content.view", action="go", source="library", kind="home")
    kinds = [e.get("name") for e in entries_of(home)]
    check(kinds == ["Objects", "Groups", "Textures", "Sounds", "Particles", "Spawn Sections", "Base Scenes"],
          "C the Library lists its kinds", json.dumps(kinds))
    v = mcp("content.view", action="go", source="library", kind="objects", folder="corps")
    paths = [e.get("path", "") for e in entries_of(v)]
    check(paths and not [x for x in paths if x.lower().endswith(".thm")] and "corps\\corp_dog.object" in paths,
          "C the Library's objects are one tile each, no thumbnail a tile of its own", json.dumps(paths)[:300])
    dog = next((e for e in entries_of(v) if e.get("path") == "corps\\corp_dog.object"), {})
    check((dog.get("payload") or {}) == {"kind": "object", "path": "corps\\corp_dog.object"}
          and (dog.get("place") or {}).get("ref") == "corps\\corp_dog"
          and (dog.get("import") or {}).get("path") == "rawdata/objects/corps/corp_dog.object",
          "C a Library object drags as an object and names what places and brings it in", json.dumps(dog)[:400])
    shot(shots, "content_library_objects")

    spawns = mcp("content.view", action="go", source="library", kind="spawn_sections")
    section = None
    folder = ""
    for _ in range(6):
        files = [e for e in entries_of(spawns) if not e.get("folder")]
        if files:
            section = files[0]
            break
        sub = next((e for e in entries_of(spawns) if e.get("folder")), None)
        if not sub:
            break
        folder = sub["path"].rstrip("\\")
        spawns = mcp("content.view", action="go", source="library", kind="spawn_sections", folder=folder)
    if section:
        leaf = section["path"].rsplit("\\", 1)[-1]
        check((section.get("payload") or {}) == {"kind": "spawn", "path": leaf}
              and (section.get("place") or {}) == {"kind": "spawn", "section": leaf},
              "C a Library spawn section drags as a spawn payload of its section", json.dumps(section)[:300])
        shot(shots, "content_library_spawns")
    else:
        note("the spawn sections did not list: " + json.dumps(spawns)[:200])

    # Bring into Project: what the entry says file.import takes, and the object comes with its thumbnail
    imp = dog.get("import") or {}
    if imp:
        r = mcp("file.import", **imp)
        there = os.path.join(project, "rawdata", "objects", "corps")
        check(r.get("ok") and os.path.isfile(os.path.join(there, "corp_dog.object"))
              and os.path.isfile(os.path.join(there, "corp_dog.thm")),
              "C Bring into Project copies the object and its thumbnail", json.dumps(r)[:300])

    # a list of 300 objects and more, scrolled through while its thumbnails come in: no stall
    mcp("perf.stalls", clear=True)
    big = mcp("content.view", action="go", source="library", kind="objects", folder="", query="musor", view="tiles", tile=96)
    total = big.get("total", 0)
    files = [e for e in entries_of(mcp("content.view", limit=1000)) if not e.get("folder")]
    check(total >= 300, "C a search lists 300 objects and more (%d)" % total, json.dumps({k: big.get(k) for k in ("ok", "total", "error")}))
    for i in range(0, len(files), max(1, len(files) // 8)):
        mcp("content.view", action="select", path=files[i]["path"])
        mcp("shot.window")
    time.sleep(1.0)
    mcp("shot.window")
    stalls = mcp("perf.stalls")
    check(stalls.get("ok") and not stalls.get("stalls"), "C no UI pass stalls while the thumbnails come in",
          json.dumps(stalls)[:400])
    shot(shots, "content_library_300")
    mcp("content.view", action="clear", query="", tile_fit=True)
    mcp("content.view", action="go", source="project", folder="")


# ---------------------------------------------------------------------------------------------------- T
def run_texture(project: str, shots: str | None) -> None:
    os.makedirs(os.path.join(project, "qa_previews"), exist_ok=True)
    shutil.copyfile(SHEET, os.path.join(project, "qa_previews", "sheet.dds"))
    t = mcp("preview.texture", action="open", path="qa_previews/sheet.dds", source="project")
    check(t.get("ok") and t.get("tab") == "texture" and t.get("active") and t.get("open"), "T a project image opens the texture tab",
          json.dumps(t)[:300])
    check(t.get("width", 0) > 0 and t.get("height", 0) > 0 and t.get("fit") is True and t.get("channel") == "rgba",
          "T it reports its size, fits and shows RGBA", json.dumps(t)[:300])
    check(t.get("container") == "DDS" and t.get("decoded") is False, "T a DDS is shown as the GPU holds the file, not decoded",
          json.dumps(t)[:300])
    check(t.get("origin") == "project" and t.get("file"), "T it names where it lives, apart from its path", json.dumps(t)[:300])
    # an opener answers the frames it owed (docs/UI_DESIGN.md, section 7): the pane and the fit are laid out already
    panes = t.get("panes", {})
    check(200 <= panes.get("details_width", 0) <= 480 and 0 < t.get("zoom", 0) <= 1.0
          and abs(t.get("zoom", 0) - t.get("scale", -1)) < 1e-3, "T the opener's answer is the frame it opened",
          json.dumps({k: t.get(k) for k in ("panes", "zoom", "fit", "scale")}))
    shot(shots, "texture_fit")
    t = mcp("preview.texture")
    panes = t.get("panes", {})
    check(panes.get("details") is True and 200 <= panes.get("details_width", 0) <= 480, "T Details keeps its pt size", json.dumps(panes))
    check(0 < t.get("scale", 0) <= 1.0, "T fit never blows a texture up past 1:1", str(t.get("scale")))
    audit("T fit")
    t = mcp("preview.texture", zoom="1")
    check(t.get("zoom") == 1 and t.get("fit") is False, "T 1:1", json.dumps(t)[:200])
    shot(shots, "texture_1to1")
    t = mcp("preview.texture", zoom="4")
    check(abs(t.get("zoom", 0) - 4) < 1e-3, "T a factor", json.dumps(t)[:200])
    shot(shots, "texture_4x")
    for ch in ("r", "g", "b", "a"):
        t = mcp("preview.texture", channel=ch, zoom="fit")
        check(t.get("channel") == ch, "T the " + ch.upper() + " view", json.dumps(t)[:200])
    shot(shots, "texture_alpha")
    t = mcp("preview.texture", channel="rgb")
    check(t.get("channel") == "rgba", "T rgb still names the colour view", json.dumps(t)[:200])
    bad = mcp("preview.texture", channel="x")
    check(not bad.get("ok"), "T an unknown channel is refused", json.dumps(bad)[:200])
    bad = mcp("preview.texture", zoom="-2")
    check(not bad.get("ok"), "T a zoom that is no factor is refused", json.dumps(bad)[:200])
    if t.get("mips", 0) > 1:
        t = mcp("preview.texture", mip=1)
        check(t.get("mip") == 1, "T a mip", json.dumps(t)[:200])
        mcp("preview.texture", mip=0)
    t = mcp("preview.texture", details=False)
    check(t.get("panes", {}).get("details") is False, "T Details off", json.dumps(t.get("panes")))
    shot(shots, "texture_no_details")
    t = mcp("preview.texture", details=True)
    check(t.get("panes", {}).get("details") is True, "T Details on")
    # the editor's own library: a texture every SDK has
    t = mcp("preview.texture", action="open", path="ed/ed_nodata", source="library")
    if t.get("ok"):
        check(t.get("name", "").replace("/", "\\") == "ed\\ed_nodata" and t.get("width", 0) > 0 and t.get("origin") == "library",
              "T an editor library texture", json.dumps(t)[:300])
    else:
        note("the editor library has no ed\\ed_nodata: " + json.dumps(t)[:200])
    # the linked game's: a block-compressed file names its own format and where the game keeps it
    game = mcp("file.list", source="game", kind="dds", query="textures", limit=1)
    files = game.get("files", []) if game.get("ok") else []
    if files and files[0].get("path"):
        t = mcp("preview.texture", action="open", path=files[0]["path"], source="game")
        check(t.get("ok") and t.get("container") == "DDS" and (t.get("archive") or t.get("file")),
              "T a game texture names the file it came from", json.dumps(t)[:300])
        check((t.get("origin") == "game_archive" and t.get("archive") and not t.get("file"))
              or (t.get("origin") == "game_folder" and t.get("file")), "T and whether the game keeps it packed or loose",
              json.dumps(t)[:300])
        if t.get("ok") and not t.get("decoded"):
            check(t.get("format", "").startswith(("BC", "B8G8R8", "R8G8B8")) and t.get("mips", 0) >= 1,
                  "T its format and mips are the file's", json.dumps(t)[:300])
        shot(shots, "texture_game")
    else:
        note("no linked game texture: the game source is not checked")
    bad = mcp("preview.texture", action="open", path="qa_previews/none.dds", source="project")
    check(not bad.get("ok"), "T a file that is not there is refused", json.dumps(bad)[:200])
    t = mcp("preview.texture", action="close")
    check(t.get("ok") and t.get("open") is False, "T close frees the tab", json.dumps(t)[:200])


# ---------------------------------------------------------------------------------------------------- M
def find_model() -> dict | None:
    objs = mcp("file.list", source="library", kind="objects", limit=5)
    for a in objs.get("files", []) if objs.get("ok") else []:
        if a.get("path"):
            return {"name": a["path"]}
    # a prop of the game's: a static mesh the model pool always takes
    game = mcp("file.list", source="game", kind="ogf", query="dynamics", limit=5)
    for a in game.get("files", []) if game.get("ok") else []:
        if a.get("path"):
            return {"name": a["path"], "source": "game"}
    return None


def run_model(shots: str | None) -> None:
    model = find_model()
    if not model:
        note("no editor object and no linked game model: the model tab is not checked")
        return
    m = mcp("preview.model", **model)
    check(m.get("ok") and m.get("tab") == "model" and m.get("active") and m.get("open"), "M a model opens the model tab",
          json.dumps(m)[:300])
    check(not m.get("error") and m.get("verts", 0) > 0 and m.get("tris", 0) > 0, "M it reports its geometry", json.dumps(m)[:300])
    check(m.get("lighting") == "studio", "M the headlight is the default", json.dumps(m)[:200])
    check(200 <= m.get("panes", {}).get("details_width", 0) <= 480,
          "M the opener's answer is the frame it opened", json.dumps({k: m.get(k) for k in ("panes",)}))
    shot(shots, "model_studio")
    m = mcp("preview.model")
    panes = m.get("panes", {})
    check(panes.get("details") is True and 200 <= panes.get("details_width", 0) <= 480, "M Details keeps its pt size", json.dumps(panes))
    audit("M studio")
    m = mcp("preview.model", lighting="unlit")
    check(m.get("lighting") == "unlit", "M Unlit", json.dumps(m)[:200])
    shot(shots, "model_unlit")
    m = mcp("preview.model", lighting="studio", wireframe=True)
    check(m.get("wireframe") is True and m.get("lighting") == "studio", "M wireframe", json.dumps(m)[:200])
    shot(shots, "model_wireframe")
    m = mcp("preview.model", wireframe=False, bones=True)
    check(m.get("wireframe") is False and (m.get("bones_shown") is False or m.get("bones", 0) > 0), "M bones only where there are some",
          json.dumps(m)[:200])
    m = mcp("preview.model", bones=False, details=False, frame=True)
    check(m.get("ok") and m.get("panes", {}).get("details") is False, "M Details off, framed", json.dumps(m)[:200])
    shot(shots, "model_no_details")
    m = mcp("preview.model", details=True, reset=True)
    check(m.get("ok") and m.get("panes", {}).get("details") is True, "M Details on, the view reset")
    bad = mcp("preview.model", lighting="neon")
    check(not bad.get("ok"), "M an unknown lighting is refused", json.dumps(bad)[:200])
    run_game_character(shots)


# a character of the linked game draws with textures only its archives hold: each is found, none is the placeholder
def run_game_character(shots: str | None) -> None:
    game = mcp("file.list", source="game", kind="ogf", query="actors", limit=5)
    for a in game.get("files", []) if game.get("ok") else []:
        if not a.get("path"):
            continue
        m = mcp("preview.model", name=a["path"], source="game", lighting="studio")
        if not m.get("ok") or m.get("error"):
            continue
        textures = m.get("textures", [])
        froms = {"project", "editor", "game", "missing"}
        check(textures and all(t.get("name") and t.get("from") in froms for t in textures),
              "M a game character names its textures and where each comes from", json.dumps(textures)[:300])
        check(not [t for t in textures if t.get("from") == "missing"], "M none of them is missing (drawn as the placeholder)",
              json.dumps(textures)[:300])
        shot(shots, "model_game_character")
        return
    note("no linked game character: its textures are not checked")


# ---------------------------------------------------------------------------------------------------- L
def run_library(kind: str, shots: str | None) -> None:
    before = active_tab()
    s = mcp("library.view", kind=kind, action="state")
    check(not s.get("ok") or s.get("tab") == kind, "L " + kind + ": state of a closed tab refuses or answers", json.dumps(s)[:200])
    o = mcp("library.view", kind=kind, action="open")
    if not check(o.get("ok") and o.get("tab") == kind and o.get("active"), "L " + kind + " opens as a tab", json.dumps(o)[:300]):
        return
    check(o.get("dirty") is False, "L " + kind + " starts clean", json.dumps(o)[:200])
    # where it reads from - what its empty state tells the author: the light animations' file, the others a folder
    check(bool(o.get("file" if kind == "light_animations" else "folder")), "L " + kind + " names where it reads from",
          json.dumps(o)[:300])
    shot(shots, kind)
    audit("L " + kind)
    if o.get("items", 0) == 0:
        note(kind + " holds no item in this SDK: search and select not checked")
        return
    first = o.get("first", "")
    check(bool(first) and o.get("shown") == o.get("items"), "L " + kind + " lists every item", json.dumps(o)[:200])
    leaf = first.split("\\")[-1]
    s = mcp("library.view", kind=kind, action="state", query=leaf)
    check(s.get("ok") and 0 < s.get("shown", 0) <= s.get("items", 0), "L " + kind + " searches", json.dumps(s)[:200])
    s = mcp("library.view", kind=kind, action="select", item=first)
    check(s.get("ok") and s.get("selected") == first, "L " + kind + " selects " + first, json.dumps(s)[:300])
    shot(shots, kind + "_selected")
    bad = mcp("library.view", kind=kind, action="select", item="qa-no-such-item-anywhere")
    check(not bad.get("ok"), "L " + kind + " refuses an item it does not have", json.dumps(bad)[:200])
    mcp("library.view", kind=kind, action="state", query="")
    # a state call never switches tabs
    if before and before != kind:
        mcp("ui.tab", action="activate", doc=before)
        s = mcp("library.view", kind=kind, action="state")
        check(s.get("ok") and s.get("active") is False, "L " + kind + ": state leaves the active tab alone", json.dumps(s)[:200])
    s = mcp("library.view", kind=kind, action="state")
    check(s.get("dirty") is False, "L " + kind + " is still clean", json.dumps(s)[:200])
    c = mcp("ui.tab", action="close", doc=kind)
    if c.get("ok") is False and "unknown" in str(c.get("error", "")):
        note("ui_tab is not there: the " + kind + " tab stays open")


def run_libraries(shots: str | None) -> None:
    bad = mcp("library.view", kind="nope")
    check(not bad.get("ok"), "L an unknown kind is refused", json.dumps(bad)[:200])
    for kind in ("light_animations", "sound_library", "texture_library"):
        run_library(kind, shots)


# ---------------------------------------------------------------------------------------------------- S
def run_sizes(project: str, shots: str | None) -> None:
    sizes = [(1707, 912, 1.5, "large"), (1280, 720, 1.0, "small")]
    for w, h, scale, tag in sizes:
        r = mcp("app.window", action="resize", width=w, height=h)
        u = mcp("ui.scale", value=scale)
        if not r.get("ok") or not u.get("ok"):
            note("window_control resize / ui_scale are not there: sizes not checked")
            return
        t = mcp("preview.texture", action="open", path="qa_previews/sheet.dds", source="project")
        check(t.get("ok"), "S " + tag + ": the texture tab")
        shot(shots, "size_%s_texture" % tag)
        audit("S " + tag + " texture")
        model = find_model()
        if model:
            mcp("preview.model", **model)
            shot(shots, "size_%s_model" % tag)
            audit("S " + tag + " model")
        for kind in ("light_animations", "sound_library"):
            if mcp("library.view", kind=kind, action="open").get("ok"):
                shot(shots, "size_%s_%s" % (tag, kind))
                audit("S " + tag + " " + kind)
    mcp("ui.scale", value=1.5)


PHASES = "CTMLS"


def run(project: str, shots: str | None, sizes: bool, only: str = "CTML") -> None:
    ed = Editor()
    try:
        ed.start(project)
        steps = {"C": lambda: run_content(project, shots), "T": lambda: run_texture(project, shots), "M": lambda: run_model(shots),
                 "L": lambda: run_libraries(shots), "S": lambda: run_sizes(project, shots)}
        for letter in PHASES:
            # S, every tab at two sizes, runs when --sizes or --only asks for it
            if letter in only and (letter != "S" or sizes or "S" in only):
                t0 = time.monotonic()
                steps[letter]()
                print("TIME phase %s %.1f s" % (letter, time.monotonic() - t0), flush=True)
        stalls = mcp("perf.stalls")
        check(stalls.get("ok") and not stalls.get("stalls"), "no UI pass stalled", json.dumps(stalls)[:400])
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for the tabs' screenshots")
    ap.add_argument("--sizes", action="store_true", help="also look at every tab at 1707x912 / 1.5 and 1280x720 / 1.0")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    ap.add_argument("--only", help="phase letters: C (content browser), T (texture), M (model), L (libraries), S (sizes)")
    args = ap.parse_args()
    asked = "".join(p for p in (args.only or "CTML").replace(",", "").upper() if p.strip())
    if any(p not in PHASES for p in asked):
        print("no such phase in %s: %s" % (PHASES, args.only))
        return 2

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    if not os.path.isfile(SHEET):
        print("the bundled PDA sheet is missing: " + SHEET)
        return 2

    work = tempfile.mkdtemp(prefix="grizzly_previews_")
    copy = os.path.join(work, "QaPreviews")
    qa_target.copy_project(src, copy, ignore=(".git",))
    print("running against a copy: " + copy, flush=True)
    # the copy's own session would bring the author's tabs back: it starts from none
    session = os.path.join(copy, "temp", "session.ltx")
    if os.path.isfile(session):
        os.remove(session)
    pl = os.path.join(copy, "project.ltx")
    if os.path.isfile(pl):
        with open(pl, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        low = text.lower()
        at = low.find(src.lower())
        while at >= 0:
            text = text[:at] + copy + text[at + len(src):]
            low = text.lower()
            at = low.find(src.lower(), at + len(copy))
        with open(pl, "w", encoding="utf-8", newline="") as f:
            f.write(text)

    # what the editor keeps between runs: the project list, and the layout and preferences a resize or a new
    # scale would leave behind
    keep = qa_target.KEPT
    backups = {}
    for path in keep:
        if os.path.isfile(path):
            with open(path, "rb") as f:
                backups[path] = f.read()

    try:
        run(copy, args.shots, args.sizes, asked)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        for path, data in backups.items():
            with open(path, "wb") as f:
                f.write(data)
        if not args.keep:
            qa_target.remove_tree(work)

    ok = sum(1 for good, _ in results if good)
    print(f"PREVIEWS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
