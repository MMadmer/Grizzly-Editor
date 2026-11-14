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
  K  the library commands, in an editor whose data root is a library of the test's own (a texture, a sound and a copy
     of lanims.xr in a temp folder; nothing of the real library is touched): library.set changes a texture's
     description, a sound's parameters and a light animation's keys by named keys (a misspelt key is refused with the
     nearest one, nothing changes on a failure), writes the files as the tab's Save does or leaves an unsaved edit;
     library.rebuild makes a game texture and a game sound; library.import brings a picture in from anywhere (refusing
     what it would replace without overwrite) and a picture dropped on the Texture Library tab runs it; library.view
     check turns the tab into New Textures; library.play plays and stops; library.reload refuses to lose unsaved
     keys; the tabs' toolbars press actions; the import folder keeps no copy of a picture the command staged;
     library.sync (both libraries) and library.rewrite_descriptions are jobs the command waits for - they make what
     is missing, say what they made, answer a job with wait=false, refuse an unknown kind with the real ones, and
     open no dialog

    python tools/qa/test_previews.py --project <project> [--shots <folder>] [--sizes]
"""
from __future__ import annotations

import json
import os
import shutil
import struct
import sys
import tempfile
import time
import wave
import zlib

import qa
from qa import check, cmd, shot


# a picture that ships with the editor: the PDA's texture sheet
SHEET = os.path.join(qa.REPO, "gamedata", "configs", "pda", "ui_xms_pda.dds")

def note(text: str) -> None:
    print("NOTE " + text, flush=True)


def escaped_in_tab() -> list | None:
    """What the kit saw spill out of the work area's windows last frame; None when ui.audit is not there."""
    a = cmd("ui.audit")
    if not isinstance(a.get("escaped"), list):
        return None
    return [e for e in a["escaped"] if "doc.host" in str(e.get("window", ""))]


def audit(label: str) -> None:
    esc = escaped_in_tab()
    if esc is None:
        note("ui.audit is not there: " + label + " not audited")
        return
    check(esc == [], label + ": nothing escapes the tab", json.dumps(esc)[:400])


def active_tab() -> str:
    return str(cmd("state").get("active_tab", ""))


# ---------------------------------------------------------------------------------------------------- C
LIB = os.path.join(qa.REPO, "rawdata", "objects")


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
    v = cmd("content.view", action="go", source="project", folder="rawdata/objects/qa_one")
    names = [e.get("path") for e in entries_of(v)]
    check(names == ["rawdata\\objects\\qa_one\\corp_dog.object"], "C an .object with its .thm is one tile in the project",
          json.dumps(v)[:400])
    one = (entries_of(v) or [{}])[0]
    check(one.get("picture") == "model" and (one.get("place") or {}).get("ref") == "qa_one\\corp_dog",
          "C its picture is the object's and Place in Level names its reference", json.dumps(one)[:300])
    v = cmd("content.view", action="go", source="project", folder="rawdata/objects/qa_thumbs")
    check(v.get("ok") and v.get("total") == 0, "C a folder of thumbnails alone is empty", json.dumps(v)[:300])
    shot(shots, "content_project_object")

    home = cmd("content.view", action="go", source="library", kind="home")
    kinds = [e.get("name") for e in entries_of(home)]
    check(kinds == ["Objects", "Groups", "Textures", "Sounds", "Particles", "Spawn Sections", "Base Scenes"],
          "C the Library lists its kinds", json.dumps(kinds))
    v = cmd("content.view", action="go", source="library", kind="objects", folder="corps")
    paths = [e.get("path", "") for e in entries_of(v)]
    check(paths and not [x for x in paths if x.lower().endswith(".thm")] and "corps\\corp_dog.object" in paths,
          "C the Library's objects are one tile each, no thumbnail a tile of its own", json.dumps(paths)[:300])
    dog = next((e for e in entries_of(v) if e.get("path") == "corps\\corp_dog.object"), {})
    check((dog.get("payload") or {}) == {"kind": "object", "path": "corps\\corp_dog.object"}
          and (dog.get("place") or {}).get("ref") == "corps\\corp_dog"
          and (dog.get("import") or {}).get("path") == "rawdata/objects/corps/corp_dog.object",
          "C a Library object drags as an object and names what places and brings it in", json.dumps(dog)[:400])
    shot(shots, "content_library_objects")

    spawns = cmd("content.view", action="go", source="library", kind="spawn_sections")
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
        spawns = cmd("content.view", action="go", source="library", kind="spawn_sections", folder=folder)
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
        r = cmd("file.import", **imp)
        there = os.path.join(project, "rawdata", "objects", "corps")
        check(r.get("ok") and os.path.isfile(os.path.join(there, "corp_dog.object"))
              and os.path.isfile(os.path.join(there, "corp_dog.thm")),
              "C Bring into Project copies the object and its thumbnail", json.dumps(r)[:300])

    # a list of 300 objects and more, scrolled through while its thumbnails come in: no stall
    cmd("perf.stalls", clear=True)
    big = cmd("content.view", action="go", source="library", kind="objects", folder="", query="musor", view="tiles", tile=96)
    total = big.get("total", 0)
    files = [e for e in entries_of(cmd("content.view", limit=1000)) if not e.get("folder")]
    check(total >= 300, "C a search lists 300 objects and more (%d)" % total, json.dumps({k: big.get(k) for k in ("ok", "total", "error")}))
    for i in range(0, len(files), max(1, len(files) // 8)):
        cmd("content.view", action="select", path=files[i]["path"])
        cmd("shot.window")
    time.sleep(1.0)
    cmd("shot.window")
    qa.stalls("C no UI pass stalls while the thumbnails come in", clear=True)
    shot(shots, "content_library_300")
    cmd("content.view", action="clear", query="", tile_fit=True)
    cmd("content.view", action="go", source="project", folder="")


# ---------------------------------------------------------------------------------------------------- T
def run_texture(project: str, shots: str | None) -> None:
    os.makedirs(os.path.join(project, "qa_previews"), exist_ok=True)
    shutil.copyfile(SHEET, os.path.join(project, "qa_previews", "sheet.dds"))
    t = cmd("preview.texture", action="open", path="qa_previews/sheet.dds", source="project")
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
    t = cmd("preview.texture")
    panes = t.get("panes", {})
    check(panes.get("details") is True and 200 <= panes.get("details_width", 0) <= 480, "T Details keeps its pt size", json.dumps(panes))
    check(0 < t.get("scale", 0) <= 1.0, "T fit never blows a texture up past 1:1", str(t.get("scale")))
    audit("T fit")
    t = cmd("preview.texture", zoom="1")
    check(t.get("zoom") == 1 and t.get("fit") is False, "T 1:1", json.dumps(t)[:200])
    shot(shots, "texture_1to1")
    t = cmd("preview.texture", zoom="4")
    check(abs(t.get("zoom", 0) - 4) < 1e-3, "T a factor", json.dumps(t)[:200])
    shot(shots, "texture_4x")
    for ch in ("r", "g", "b", "a"):
        t = cmd("preview.texture", channel=ch, zoom="fit")
        check(t.get("channel") == ch, "T the " + ch.upper() + " view", json.dumps(t)[:200])
    shot(shots, "texture_alpha")
    t = cmd("preview.texture", channel="rgb")
    check(t.get("channel") == "rgba", "T rgb still names the colour view", json.dumps(t)[:200])
    bad = cmd("preview.texture", channel="x")
    check(not bad.get("ok"), "T an unknown channel is refused", json.dumps(bad)[:200])
    bad = cmd("preview.texture", zoom="-2")
    check(not bad.get("ok"), "T a zoom that is no factor is refused", json.dumps(bad)[:200])
    if t.get("mips", 0) > 1:
        t = cmd("preview.texture", mip=1)
        check(t.get("mip") == 1, "T a mip", json.dumps(t)[:200])
        cmd("preview.texture", mip=0)
    t = cmd("preview.texture", details=False)
    check(t.get("panes", {}).get("details") is False, "T Details off", json.dumps(t.get("panes")))
    shot(shots, "texture_no_details")
    t = cmd("preview.texture", details=True)
    check(t.get("panes", {}).get("details") is True, "T Details on")
    # the editor's own library: a texture every SDK has
    t = cmd("preview.texture", action="open", path="ed/ed_nodata", source="library")
    if t.get("ok"):
        check(t.get("name", "").replace("/", "\\") == "ed\\ed_nodata" and t.get("width", 0) > 0 and t.get("origin") == "library",
              "T an editor library texture", json.dumps(t)[:300])
    else:
        note("the editor library has no ed\\ed_nodata: " + json.dumps(t)[:200])
    # the linked game's: a block-compressed file names its own format and where the game keeps it
    game = cmd("file.list", source="game", kind="dds", query="textures", limit=1)
    files = game.get("files", []) if game.get("ok") else []
    if files and files[0].get("path"):
        t = cmd("preview.texture", action="open", path=files[0]["path"], source="game")
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
    bad = cmd("preview.texture", action="open", path="qa_previews/none.dds", source="project")
    check(not bad.get("ok"), "T a file that is not there is refused", json.dumps(bad)[:200])
    # it says so in its own sentence (not "cannot be decoded"), the tab shows it as an error state, and ui.audit reports
    # that state: what the tour fails on
    check("was not found" in str(bad.get("error", "")) and "decode" not in str(bad.get("error", "")),
          "T and says it was not found, with where it looked", json.dumps(bad)[:300])
    shown = qa.until(lambda: cmd("ui.audit").get("errors"), 10)
    check(bool(shown) and "was not found" in json.dumps(shown), "T the tab shows it as an error state that ui.audit reports",
          json.dumps(shown)[:300])
    shot(shots, "texture_not_found")
    t = cmd("preview.texture", action="close")
    check(t.get("ok") and t.get("open") is False, "T close frees the tab", json.dumps(t)[:200])


# ---------------------------------------------------------------------------------------------------- M
def find_model() -> dict | None:
    objs = cmd("file.list", source="library", kind="objects", limit=5)
    for a in objs.get("files", []) if objs.get("ok") else []:
        if a.get("path"):
            return {"name": a["path"]}
    # a prop of the game's: a static mesh the model pool always takes
    game = cmd("file.list", source="game", kind="ogf", query="dynamics", limit=5)
    for a in game.get("files", []) if game.get("ok") else []:
        if a.get("path"):
            return {"name": a["path"], "source": "game"}
    return None


def run_model(shots: str | None) -> None:
    model = find_model()
    if not model:
        note("no editor object and no linked game model: the model tab is not checked")
        return
    m = cmd("preview.model", **model)
    check(m.get("ok") and m.get("tab") == "model" and m.get("active") and m.get("open"), "M a model opens the model tab",
          json.dumps(m)[:300])
    check(not m.get("error") and m.get("verts", 0) > 0 and m.get("tris", 0) > 0, "M it reports its geometry", json.dumps(m)[:300])
    check(m.get("lighting") == "studio", "M the headlight is the default", json.dumps(m)[:200])
    check(200 <= m.get("panes", {}).get("details_width", 0) <= 480,
          "M the opener's answer is the frame it opened", json.dumps({k: m.get(k) for k in ("panes",)}))
    shot(shots, "model_studio")
    m = cmd("preview.model")
    panes = m.get("panes", {})
    check(panes.get("details") is True and 200 <= panes.get("details_width", 0) <= 480, "M Details keeps its pt size", json.dumps(panes))
    audit("M studio")
    m = cmd("preview.model", lighting="unlit")
    check(m.get("lighting") == "unlit", "M Unlit", json.dumps(m)[:200])
    shot(shots, "model_unlit")
    m = cmd("preview.model", lighting="studio", wireframe=True)
    check(m.get("wireframe") is True and m.get("lighting") == "studio", "M wireframe", json.dumps(m)[:200])
    shot(shots, "model_wireframe")
    m = cmd("preview.model", wireframe=False, bones=True)
    check(m.get("wireframe") is False and (m.get("bones_shown") is False or m.get("bones", 0) > 0), "M bones only where there are some",
          json.dumps(m)[:200])
    m = cmd("preview.model", bones=False, details=False, frame=True)
    check(m.get("ok") and m.get("panes", {}).get("details") is False, "M Details off, framed", json.dumps(m)[:200])
    shot(shots, "model_no_details")
    m = cmd("preview.model", details=True, reset=True)
    check(m.get("ok") and m.get("panes", {}).get("details") is True, "M Details on, the view reset")
    bad = cmd("preview.model", lighting="neon")
    check(not bad.get("ok"), "M an unknown lighting is refused", json.dumps(bad)[:200])
    run_game_character(shots)


# a character of the linked game draws with textures only its archives hold: each is found, none is the placeholder
def run_game_character(shots: str | None) -> None:
    game = cmd("file.list", source="game", kind="ogf", query="actors", limit=5)
    for a in game.get("files", []) if game.get("ok") else []:
        if not a.get("path"):
            continue
        m = cmd("preview.model", name=a["path"], source="game", lighting="studio")
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
    s = cmd("library.view", kind=kind, action="state")
    check(not s.get("ok") or s.get("tab") == kind, "L " + kind + ": state of a closed tab refuses or answers", json.dumps(s)[:200])
    o = cmd("library.view", kind=kind, action="open")
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
    s = cmd("library.view", kind=kind, action="state", query=leaf)
    check(s.get("ok") and 0 < s.get("shown", 0) <= s.get("items", 0), "L " + kind + " searches", json.dumps(s)[:200])
    s = cmd("library.view", kind=kind, action="select", item=first)
    check(s.get("ok") and s.get("selected") == first, "L " + kind + " selects " + first, json.dumps(s)[:300])
    shot(shots, kind + "_selected")
    bad = cmd("library.view", kind=kind, action="select", item="qa-no-such-item-anywhere")
    check(not bad.get("ok"), "L " + kind + " refuses an item it does not have", json.dumps(bad)[:200])
    cmd("library.view", kind=kind, action="state", query="")
    # a state call never switches tabs
    if before and before != kind:
        cmd("ui.tab", action="activate", doc=before)
        s = cmd("library.view", kind=kind, action="state")
        check(s.get("ok") and s.get("active") is False, "L " + kind + ": state leaves the active tab alone", json.dumps(s)[:200])
    s = cmd("library.view", kind=kind, action="state")
    check(s.get("dirty") is False, "L " + kind + " is still clean", json.dumps(s)[:200])
    c = cmd("ui.tab", action="close", doc=kind)
    if c.get("ok") is False and "unknown" in str(c.get("error", "")):
        note("ui.tab is not there: the " + kind + " tab stays open")


def run_libraries(shots: str | None) -> None:
    bad = cmd("library.view", kind="nope")
    check(not bad.get("ok"), "L an unknown kind is refused", json.dumps(bad)[:200])
    for kind in ("light_animations", "sound_library", "texture_library"):
        run_library(kind, shots)


# ---------------------------------------------------------------------------------------------------- S
def run_sizes(project: str, shots: str | None) -> None:
    sizes = [(1707, 912, 1.5, "large"), (1280, 720, 1.0, "small")]
    for w, h, scale, tag in sizes:
        r = cmd("app.window", action="resize", width=w, height=h)
        u = cmd("ui.scale", value=scale)
        if not r.get("ok") or not u.get("ok"):
            note("app.window resize / ui.scale are not there: sizes not checked")
            return
        t = cmd("preview.texture", action="open", path="qa_previews/sheet.dds", source="project")
        check(t.get("ok"), "S " + tag + ": the texture tab")
        shot(shots, "size_%s_texture" % tag)
        audit("S " + tag + " texture")
        model = find_model()
        if model:
            cmd("preview.model", **model)
            shot(shots, "size_%s_model" % tag)
            audit("S " + tag + " model")
        for kind in ("light_animations", "sound_library"):
            if cmd("library.view", kind=kind, action="open").get("ok"):
                shot(shots, "size_%s_%s" % (tag, kind))
                audit("S " + tag + " " + kind)
    cmd("ui.scale", value=1.5)


# ---------------------------------------------------------------------------------------------------- K
def write_tga(path: str, side: int = 64) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    head = bytes([0, 0, 2]) + bytes(5) + struct.pack("<HHHH", 0, 0, side, side) + bytes([24, 0])
    pixels = b"".join(bytes([(x * 4) % 256, (y * 4) % 256, 96]) for y in range(side) for x in range(side))
    with open(path, "wb") as f:
        f.write(head + pixels)


def write_png(path: str, side: int = 64) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    rows = b"".join(b"\x00" + b"".join(bytes([(x * 4) % 256, 200, (y * 4) % 256]) for x in range(side))
                    for y in range(side))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def write_wav(path: str) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(b"".join(struct.pack("<h", int(12000 * (1 if (i // 40) % 2 else -1))) for i in range(4410)))


def run_library_commands(project: str, shots: str | None) -> None:
    """K: the library commands in an editor whose data root is a throw-away library."""
    lib = os.path.realpath(tempfile.mkdtemp(prefix="grizzly_qa_lib_"))
    art = os.path.realpath(tempfile.mkdtemp(prefix="grizzly_qa_art_"))
    try:
        for sub in ("rawdata/textures", "rawdata/sounds", "gamedata/textures", "gamedata/sounds", "import"):
            os.makedirs(os.path.join(lib, sub), exist_ok=True)
        shutil.copy(os.path.join(qa.REPO, "gamedata", "lanims.xr"), os.path.join(lib, "gamedata", "lanims.xr"))
        write_tga(os.path.join(lib, "rawdata", "textures", "qa", "qa_swatch.tga"))
        write_wav(os.path.join(lib, "rawdata", "sounds", "qa", "qa_tone.wav"))
        png = os.path.join(art, "qa_import.png")
        write_png(png)
        write_png(os.path.join(art, "qa_drop.png"))
        with open(os.path.join(art, "notes.txt"), "w") as f:
            f.write("not a picture")
        ed = qa.Editor(project, args=["-root", lib]).start()
        try:
            library_texture(lib, art, png, shots)
            library_sound(lib)
            library_lanim(lib)
            library_maintenance(lib)
            library_is_no_game(art)
            library_pick_popups(shots)
        finally:
            ed.close()
    finally:
        qa.remove_tree(lib)
        qa.remove_tree(art)


def library_pick_popups(shots: str | None) -> None:
    """A description's texture picks are the kit's list, and a command puts each up to be looked at."""
    item = "qa" + chr(92) + "qa_swatch"
    v = cmd("library.view", kind="texture_library", action="select", item=item, show="detail")
    rows = [c.get("label") for c in cmd("ui.audit").get("controls") or []]
    check(v.get("ok") and v.get("pick") == "detail" and "None" in rows,
          "K library.view show=detail puts the detail texture's list up, None first", json.dumps(v)[:200])
    qa.shot(shots, "library_pick_detail")
    cmd("ui.popup", show="none")
    no = cmd("library.view", kind="texture_library", action="select", item=item, show="normal")
    check(not no.get("ok") and "no normal texture row" in str(no.get("error")),
          "K a pick the description has no row for is refused in a sentence", json.dumps(no)[:200])
    wrong = cmd("ui.popup", show="detail")
    check(not wrong.get("ok") and "library.view" in str(wrong.get("error")),
          "K ui.popup names the command that puts a surface's pick list up", json.dumps(wrong)[:200])
    views = cmd("ui.contract").get("view_popups") or []
    check("library.view kind=texture_library show=detail" in views, "K ui.contract names the pick lists a view command puts up",
          json.dumps(views)[:300])


def library_is_no_game(art: str) -> None:
    """A folder that is a game is refused as the library's: the library is written, a game never is. A game is told
    by what only a game has (its archives, its engine), not by what the editor's own folder has too."""
    def laid_out(name: str, *more: str) -> str:
        folder = os.path.join(art, name)
        for sub in ("gamedata", "rawdata") + more:
            os.makedirs(os.path.join(folder, sub))
        with open(os.path.join(folder, "fsgame.ltx"), "w") as f:
            f.write("; the file system's table")
        return folder

    fake = laid_out("a_game", "database")
    before = sorted(os.listdir(fake))
    r = cmd("project.set_library", path=fake)
    check(not r.get("ok") and "a game's folder" in str(r.get("error")),
          "K a game's folder is refused as the library's folder", json.dumps(r)[:300])
    check(sorted(os.listdir(fake)) == before, "K ... and nothing is written into it", str(sorted(os.listdir(fake))))
    # another editor's folder holds fsgame.ltx, gamedata and rawdata as well, and is a library
    other = laid_out("an_editor")
    taken = cmd("project.set_library", path=other)
    check(taken.get("ok") and taken.get("own") is False, "K a folder laid out like the editor's own is taken as the library",
          json.dumps(taken)[:300])
    own = cmd("project.set_library")
    check(own.get("ok") and own.get("own") is True, "K and the editor's own library comes back", json.dumps(own)[:300])


def exists(lib: str, rel: str) -> bool:
    return os.path.isfile(os.path.join(lib, *rel.split("/")))


def library_texture(lib: str, art: str, png: str, shots: str | None = None) -> None:
    item = "qa\\qa_swatch"
    o = cmd("library.view", kind="texture_library", action="open")
    check(o.get("ok") and o.get("items") == 1 and o.get("first") == item, "K the texture library lists the test's texture",
          json.dumps(o)[:300])
    # the description by named keys, written as the tab's Save writes it
    s = cmd("library.set", kind="texture_library", item=item, set={"format": "dxt5", "mips": False})
    check(s.get("ok") and s.get("saved") is True and {c["key"] for c in s.get("changed", [])} == {"format", "mips"},
          "K library.set changes a texture's description by key and says what it changed", json.dumps(s)[:300])
    check(exists(lib, "rawdata/textures/qa/qa_swatch.thm") and exists(lib, "gamedata/textures/qa/qa_swatch.dds"),
          "K and writes the description and the game texture it asks for")
    v = cmd("library.view", kind="texture_library", action="select", item=item)
    check(v.get("format") == "DXT5" and v.get("dirty") is False, "K the tab reads the new description and is clean",
          json.dumps(v)[:300])
    qa.shot(shots, "k_texture_library_description")
    labels = [str(x.get("label")) for x in cmd("ui.audit").get("controls", [])]
    check(labels.count("None") >= 1, "K the description's texture picks are the kit's combo, showing None", json.dumps(labels)[:300])
    bad = cmd("library.set", kind="texture_library", item=item, set={"formt": "DXT1"})
    check(not bad.get("ok") and "did you mean 'format'" in str(bad.get("error")), "K a misspelt key is refused with the nearest one",
          json.dumps(bad)[:300])
    bad = cmd("library.set", kind="texture_library", item=item, set={"dither": False, "format": "nope"})
    check(not bad.get("ok") and "DXT1" in str(bad.get("error")), "K a value that is none of the words is refused, listing them",
          json.dumps(bad)[:300])
    bad = cmd("library.set", kind="texture_library", item=item, set={"bump_height": 0.05})
    check(not bad.get("ok") and "no row" in str(bad.get("error")), "K a key the type has no row for is refused", json.dumps(bad)[:300])
    check(cmd("library.view", kind="texture_library", action="state").get("format") == "DXT5", "K a refused call changes nothing")
    nobody = cmd("library.set", kind="texture_library", item="qa\\qa_swatc", set={"mips": True})
    check(not nobody.get("ok") and "did you mean" in str(nobody.get("error")), "K an entry is named exactly; near ones are suggested",
          json.dumps(nobody)[:300])
    # unsaved: an edit of the tab, written by the document's save
    e = cmd("library.set", kind="texture_library", item=item, set={"dither": False}, save=False)
    check(e.get("ok") and e.get("saved") is False and e.get("dirty") is True, "K save=false leaves an unsaved edit of the tab",
          json.dumps(e)[:300])
    check(cmd("library.view", kind="texture_library", action="state").get("dirty") is True, "K and the tab is dirty")
    sv = cmd("doc.save", doc="texture_library")
    check(sv.get("ok") and qa.until(lambda: cmd("library.view", kind="texture_library", action="state").get("dirty") is False, 20),
          "K doc.save writes it and the tab is clean", json.dumps(sv)[:300])
    # the game texture again from its source
    os.remove(os.path.join(lib, "gamedata", "textures", "qa", "qa_swatch.dds"))
    r = cmd("library.rebuild", kind="texture_library", item=item)
    check(r.get("ok") and exists(lib, "gamedata/textures/qa/qa_swatch.dds"), "K library.rebuild makes the game texture again",
          json.dumps(r)[:300])
    # import: from anywhere, refusing what it would replace
    notpic = cmd("library.import", **{"from": [os.path.join(art, "notes.txt")]})
    check(not notpic.get("ok") and "no picture" in str(notpic.get("error")), "K library.import refuses what is no picture",
          json.dumps(notpic)[:300])
    i = cmd("library.import", **{"from": [png], "set": {"format": "DXT1"}})
    check(i.get("ok") and i.get("imported") == ["qa_import.png"] and exists(lib, "rawdata/textures/qa/qa_import.tga")
          and exists(lib, "gamedata/textures/qa/qa_import.dds") and exists(lib, "rawdata/textures/qa/qa_import.thm"),
          "K library.import takes a picture from anywhere into the library, built", json.dumps(i)[:300])
    check(os.path.isfile(png), "K and leaves the picture where it was")
    check(not exists(lib, "import/qa_import.png") and not exists(lib, "import/qa_import.~png"),
          "K and the import folder keeps no copy of it")
    again = cmd("library.import", **{"from": [png]})
    check(not again.get("ok") and again.get("code") == "exists", "K a second import would replace a texture: refused, named",
          json.dumps(again)[:300])
    again = cmd("library.import", **{"from": [png], "overwrite": True})
    check(again.get("ok"), "K overwrite=true replaces it", json.dumps(again)[:300])
    check(cmd("library.view", kind="texture_library", action="state").get("items") == 2, "K the library lists both")
    # a picture dropped on the tab is imported by the same command
    size = cmd("shot.window")
    x, y = float(size.get("width", 1400)) / 2, float(size.get("height", 800)) / 2
    d = cmd("library.view", kind="texture_library", action="state", simulate_drop=os.path.join(art, "qa_drop.png"), x=x, y=y)
    check(d.get("ok") and d.get("items") == 3 and exists(lib, "rawdata/textures/qa/qa_drop.tga"),
          "K a picture dropped on the Texture Library tab is imported", json.dumps(d)[:300])
    check(not exists(lib, "import/qa_drop.png") and not exists(lib, "import/qa_drop.~png"),
          "K a dropped picture leaves no copy in the import folder either")
    # a picture the author put in the import folder is theirs: taken in, renamed out of the way, never deleted
    write_png(os.path.join(lib, "import", "qa_mine.png"))
    mine = cmd("library.import")
    check(mine.get("ok") and mine.get("imported") == ["qa_mine.png"] and exists(lib, "rawdata/textures/qa/qa_mine.tga")
          and not exists(lib, "import/qa_mine.png") and exists(lib, "import/qa_mine.~png"),
          "K a picture of the import folder's own is renamed to .~png, not deleted", json.dumps(mine)[:300])
    # Check New Textures looks in the import folder; with nothing new it says so in a line, never a dialog
    nothing = cmd("library.view", kind="texture_library", action="check")
    said = qa.until(lambda: any("is new to the library" in t for t in cmd("ui.list").get("toasts") or []), 10)
    win = cmd("shot.window")
    back = cmd("library.view", kind="texture_library", action="state")
    check(nothing.get("ok") and said and win.get("modal") is None and back.get("import") is False,
          "K a check that finds nothing says so in a note over the tab and leaves no dialog",
          json.dumps({"modal": win.get("modal"), "import": back.get("import"), "toasts": cmd("ui.list").get("toasts")})[:300])
    # one new picture, and one whose name the library has
    write_png(os.path.join(lib, "import", "qa_fresh.png"))
    write_png(os.path.join(lib, "import", "qa_import.png"))
    c = cmd("library.view", kind="texture_library", action="check")
    check(c.get("ok") and c.get("import") is True and c.get("items") == 2, "K library.view check turns the tab into New Textures",
          json.dumps(c)[:300])
    audit = cmd("ui.audit")
    labels = {x.get("label") for x in audit.get("controls", [])}
    check({"Import", "More"} <= labels, "K New Textures offers Import and More", json.dumps(sorted(l for l in labels if l))[:300])
    # the tab's save asked by a command: the window's Import asks before it replaces a texture of the library; a
    # command is refused at once, with the textures and the command that replaces them
    began = time.monotonic()
    refused = cmd("doc.save", doc="texture_library")
    took = time.monotonic() - began
    check(not refused.get("ok") and refused.get("code") == "exists" and "qa_import" in str(refused.get("error"))
          and "library.import overwrite=true" in str(refused.get("error")),
          "K doc.save on New Textures that would replace a texture answers exists, naming it and the command",
          json.dumps(refused)[:300])
    check(took < 1.0, "K and it answers inside a second (%.2f s)" % took, timing=True)
    win = cmd("shot.window")
    check(win.get("modal") is None and exists(lib, "import/qa_import.png") and exists(lib, "import/qa_fresh.png"),
          "K no question is up and nothing was imported", json.dumps(win.get("modal"))[:200])
    cmd("ui.tab", action="close", doc="texture_library", discard=True)
    os.remove(os.path.join(lib, "import", "qa_import.png"))
    t = cmd("library.view", kind="texture_library", action="open", types=["images"])
    check(t.get("types") == ["images"], "K the type filter is a parameter", json.dumps(t)[:300])
    cmd("library.view", kind="texture_library", action="state", types=["images", "cube_maps", "bump_maps", "normal_maps", "terrain"])


def library_sound(lib: str) -> None:
    item = "qa\\qa_tone"
    o = cmd("library.view", kind="sound_library", action="open")
    check(o.get("ok") and o.get("items") == 1, "K the sound library lists the test's sound", json.dumps(o)[:300])
    r = cmd("library.rebuild", kind="sound_library", item=item)
    check(r.get("ok") and exists(lib, "gamedata/sounds/qa/qa_tone.ogg"), "K library.rebuild makes the game sound", json.dumps(r)[:300])
    s = cmd("library.set", kind="sound_library", item=item, set={"min_dist": 2.0, "max_dist": 40.0, "game_type": "weapon shooting"})
    check(s.get("ok") and s.get("saved") is True and len(s.get("changed", [])) == 3, "K library.set changes a sound's parameters",
          json.dumps(s)[:300])
    v = cmd("library.view", kind="sound_library", action="select", item=item)
    check(v.get("min_dist") == 2.0 and v.get("max_dist") == 40.0 and v.get("game_sound") is True, "K the tab reads them",
          json.dumps(v)[:300])
    bad = cmd("library.set", kind="sound_library", item=item, set={"min_dst": 1})
    check(not bad.get("ok") and "did you mean 'min_dist'" in str(bad.get("error")), "K a misspelt sound key is refused", json.dumps(bad)[:300])
    a = cmd("library.view", kind="sound_library", auto_play=True)
    check(a.get("auto_play") is True, "K auto play is a parameter", json.dumps(a)[:200])
    cmd("library.view", kind="sound_library", auto_play=False)
    p = cmd("library.play", kind="sound_library", item=item)
    if p.get("ok"):
        check(p.get("playing") is True, "K library.play plays the sound", json.dumps(p)[:300])
        check(cmd("library.play", kind="sound_library", action="stop").get("playing") is False, "K and stops it")
    else:
        # a machine without an audio device says so instead of pretending
        check("audio" in str(p.get("error", "")).lower() or "device" in str(p.get("error", "")).lower(),
              "K library.play without a device says so", json.dumps(p)[:300])
        note("no audio device: playing not checked")
    labels = {x.get("label") for x in cmd("ui.audit", id="##toolbar").get("controls", [])}
    check({"Play", "Stop", "Auto Play", "Rebuild", "Save", "More"} <= labels, "K the Sound Library's toolbar presses actions",
          json.dumps(sorted(l for l in labels if l))[:300])
    nowin = cmd("ui.tab", action="close", doc="sound_library", discard=True)
    check(nowin.get("ok"), "K the sound tab closes")


def library_lanim(lib: str) -> None:
    o = cmd("library.view", kind="light_animations", action="open")
    check(o.get("ok") and o.get("items", 0) > 0, "K light animations lists the copy of lanims.xr", json.dumps(o)[:300])
    item = o.get("first", "")
    s = cmd("library.view", kind="light_animations", action="select", item=item, frame=0)
    frames, keys_before = s.get("frames", 0), s.get("keys", 0)
    cmd("library.view", kind="light_animations", action="select", item=item, frame=0)
    free = None
    for f in range(1, frames):
        v = cmd("library.view", kind="light_animations", action="select", item=item, frame=f)
        if v.get("on_key") is False:
            free = f
            break
    check(free is not None, "K the animation has a free frame to put a key on", json.dumps(s)[:200])
    if free is None:
        return
    a = cmd("library.set", kind="light_animations", item=item, set={"keys": {str(free): "#FF8040"}}, save=False)
    check(a.get("ok") and a.get("saved") is False and a.get("dirty") is True, "K a key added by library.set is an unsaved edit",
          json.dumps(a)[:300])
    v = cmd("library.view", kind="light_animations", action="state")
    check(v.get("keys") == keys_before + 1 and v.get("on_key") is True, "K the tab shows the key", json.dumps(v)[:300])
    labels = {x.get("label") for x in cmd("ui.audit").get("controls", [])}
    check({"Add Key", "Delete Key", "Play", "Reload", "Alpha"} <= labels, "K the toolbars press actions",
          json.dumps(sorted(l for l in labels if l))[:300])
    m = cmd("library.set", kind="light_animations", item=item, set={"move_key": {"from": free, "to": free + 1 if free + 1 < frames else free - 1, "follow": True}}, save=False)
    check(m.get("ok"), "K library.set moves a key", json.dumps(m)[:300])
    moved = cmd("library.view", kind="light_animations", action="state")
    check(moved.get("on_key") is True, "K and the timeline follows it", json.dumps(moved)[:300])
    d = cmd("library.set", kind="light_animations", item=item, set={"keys": {str(moved.get("frame")): None}}, save=False)
    check(d.get("ok") and cmd("library.view", kind="light_animations", action="state").get("keys") == keys_before,
          "K a key is deleted with null", json.dumps(d)[:300])
    first = cmd("library.set", kind="light_animations", item=item, set={"keys": {"0": None}}, save=False)
    check(not first.get("ok") and "first key" in str(first.get("error")), "K the first key stays", json.dumps(first)[:300])
    bad = cmd("library.set", kind="light_animations", item=item, set={"fpss": 3})
    check(not bad.get("ok") and "did you mean 'fps'" in str(bad.get("error")), "K a misspelt key is refused", json.dumps(bad)[:300])
    r = cmd("library.reload", kind="light_animations")
    check(not r.get("ok") and r.get("code") == "unsaved", "K reload refuses to lose unsaved changes", json.dumps(r)[:300])
    # written: the file changes
    path = os.path.join(lib, "gamedata", "lanims.xr")
    before = open(path, "rb").read()
    w = cmd("library.set", kind="light_animations", item=item, set={"fps": 12.0})
    check(w.get("ok") and w.get("saved") is True and open(path, "rb").read() != before, "K library.set writes lanims.xr",
          json.dumps(w)[:300])
    rr = cmd("library.reload", kind="light_animations", discard=True)
    check(rr.get("ok"), "K reload with discard reads it again", json.dumps(rr)[:300])
    v = cmd("library.view", kind="light_animations", action="select", item=item)
    check(v.get("fps") == 12.0 and v.get("dirty") is False, "K and the saved fps is what the tab reads", json.dumps(v)[:300])
    n = cmd("library.view", kind="light_animations", key="next")
    check(n.get("ok"), "K the timeline goes to the next key", json.dumps(n)[:200])
    pl = cmd("library.play", kind="light_animations")
    check(pl.get("ok") and pl.get("playing") is True, "K library.play runs the animation", json.dumps(pl)[:300])
    check(cmd("library.play", kind="light_animations", action="stop").get("playing") is False, "K and stops it")
    check(cmd("library.view", kind="light_animations", alpha=True).get("alpha") is True, "K alpha is a parameter")
    cmd("ui.tab", action="close", doc="light_animations", discard=True)


def library_maintenance(lib: str) -> None:
    """library.sync and library.rewrite_descriptions: commands like the rest - they start their job, wait for it and
    answer what it did, with no window asked - on the library the earlier steps filled."""
    swatch, imported = r"qa\qa_swatch", r"qa\qa_import"
    dds = "gamedata/textures/qa/qa_swatch.dds"
    thm = "rawdata/textures/qa/qa_import.thm"
    os.remove(os.path.join(lib, *dds.split("/")))
    os.remove(os.path.join(lib, *thm.split("/")))
    s = cmd("library.sync", kind="texture_library")
    check(s.get("ok") and "started" not in s and s.get("updated", 0) >= 2 and {swatch, imported} <= set(s.get("names", []))
          and exists(lib, dds) and exists(lib, thm), "K library.sync kind=texture_library makes what is missing and says what it made",
          json.dumps(s)[:300])
    again = cmd("library.sync", kind="texture_library")
    check(again.get("ok") and again.get("updated") == 0 and again.get("up_to_date", 0) >= 2,
          "K and a second run finds everything current and writes nothing", json.dumps(again)[:300])
    # wait=false answers the job at once; job.wait ends it
    os.remove(os.path.join(lib, *dds.split("/")))
    j = cmd("library.sync", kind="texture_library", wait=False)
    check(j.get("ok") and isinstance(j.get("job"), int) and "updated" not in j, "K wait=false answers the job", json.dumps(j)[:300])
    w = cmd("job.wait", job=j.get("job", -1))
    check(w.get("ok") and w.get("state") == "done" and (w.get("result") or {}).get("updated") == 1 and exists(lib, dds),
          "K job.wait ends it with what it made", json.dumps(w)[:300])
    # sounds: the game sound made again; the sounds with no .wav any more are named, renamed only when asked
    ogg = "gamedata/sounds/qa/qa_tone.ogg"
    orphan = "gamedata/sounds/qa/qa_orphan.ogg"
    os.remove(os.path.join(lib, *ogg.split("/")))
    with open(os.path.join(lib, *orphan.split("/")), "wb") as f:
        f.write(b"orphan")
    s = cmd("library.sync", kind="sound_library")
    check(s.get("ok") and s.get("updated", 0) >= 1 and exists(lib, ogg) and s.get("orphan_count") == 1
          and s.get("orphans") == [r"qa\qa_orphan.ogg"] and s.get("retired") == 0 and exists(lib, orphan),
          "K library.sync kind=sound_library makes the game sound and names the orphan it leaves", json.dumps(s)[:300])
    s = cmd("library.sync", kind="sound_library", retire_orphans=True)
    check(s.get("ok") and s.get("retired") == 1 and not exists(lib, orphan) and exists(lib, "gamedata/sounds/qa/qa_orphan.~ogg"),
          "K retire_orphans renames it out of the way, nothing is deleted", json.dumps(s)[:300])
    j = cmd("library.sync", kind="sound_library", wait=False)
    w = cmd("job.wait", job=j.get("job", -1))
    check(j.get("ok") and "job" in j and w.get("state") == "done" and (w.get("result") or {}).get("kind") == "sound_library",
          "K the sound synchronisation is a job too", json.dumps([j, w])[:300])
    # a library is named as everywhere else, and the parameters are the command's own
    bad = cmd("library.sync", kind="textures")
    check(not bad.get("ok") and "texture_library" in str(bad.get("error")) and "sound_library" in str(bad.get("error")),
          "K an unknown kind is refused with the real ones", json.dumps(bad)[:300])
    bad = cmd("library.sync", kind="light_animations")
    check(not bad.get("ok") and "sound_library" in str(bad.get("error")), "K a library with no sources is refused", json.dumps(bad)[:300])
    bad = cmd("library.sync", kind="texture_library", retire_orphans=True)
    check(not bad.get("ok") and "retire_orphans" in str(bad.get("error")), "K retire_orphans is the sound library's", json.dumps(bad)[:300])
    bad = cmd("library.sync", kind="texture_library", discard=True)
    check(not bad.get("ok"), "K a parameter of another command is refused", json.dumps(bad)[:300])
    # the descriptions rewritten: one with a chunk this editor does not write comes out as it writes it, its age kept
    path = os.path.join(lib, "rawdata", "textures", "qa", "qa_swatch.thm")
    good = open(path, "rb").read()
    age = int(os.stat(path).st_mtime) - 100
    with open(path, "ab") as f:
        f.write(struct.pack("<II", 0x7777, 4) + b"junk")
    os.utime(path, (age, age))
    dry = cmd("library.rewrite_descriptions", dry_run=True)
    check(dry.get("ok") and dry.get("dry_run") is True and dry.get("files", 0) >= 2 and open(path, "rb").read() != good,
          "K dry_run counts the descriptions and writes none", json.dumps(dry)[:300])
    r = cmd("library.rewrite_descriptions")
    check(r.get("ok") and "started" not in r and r.get("rewritten") == 1 and r.get("unchanged", 0) >= 1
          and r.get("names") == [r"qa\qa_swatch.thm"], "K library.rewrite_descriptions rewrites and answers the count",
          json.dumps(r)[:300])
    check(open(path, "rb").read() == good and int(os.stat(path).st_mtime) == age,
          "K the description is what the editor writes and keeps its age")
    j = cmd("library.rewrite_descriptions", wait=False)
    w = cmd("job.wait", job=j.get("job", -1))
    check(j.get("ok") and "job" in j and w.get("state") == "done" and (w.get("result") or {}).get("rewritten") == 0,
          "K the rewrite is a job too", json.dumps([j, w])[:300])
    # the window's actions: a texture synchronisation runs at once (it only makes what is missing), the rewrite and the
    # orphans of a sound synchronisation are asked about, and the editor's logged answer (-nodlg) is No
    os.remove(os.path.join(lib, *dds.split("/")))
    ran = cmd("ui.palette", action="run", id="library.sync_textures")
    check(ran.get("ok") and qa.until(lambda: exists(lib, dds), 20), "K the action Synchronize Textures runs the command without a question",
          json.dumps(ran)[:300])
    with open(path, "ab") as f:
        f.write(struct.pack("<II", 0x7777, 4) + b"junk")
    tampered = open(path, "rb").read()
    ran = cmd("ui.palette", action="run", id="library.rewrite_descriptions")
    for _ in range(3):
        cmd("ui.list")
    check(ran.get("ok") and open(path, "rb").read() == tampered and not cmd("job.list", running=True).get("jobs"),
          "K the action Rewrite Texture Descriptions asks first: answered No, nothing is written", json.dumps(ran)[:300])
    os.remove(os.path.join(lib, *ogg.split("/")))
    os.remove(os.path.join(lib, "gamedata", "sounds", "qa", "qa_orphan.~ogg"))
    with open(os.path.join(lib, *orphan.split("/")), "wb") as f:
        f.write(b"orphan")
    ran = cmd("ui.palette", action="run", id="library.sync_sounds")
    check(ran.get("ok") and qa.until(lambda: exists(lib, ogg), 20), "K the action Synchronize Sounds synchronizes at once",
          json.dumps(ran)[:300])
    for _ in range(3):
        cmd("ui.list")
    check(exists(lib, orphan) and not exists(lib, "gamedata/sounds/qa/qa_orphan.~ogg"),
          "K and asks before it renames the orphans: answered No, they stay")
    # none of this asked a person anything
    listed = cmd("ui.list")
    check(listed.get("dialogs") == [], "K no command opened a dialog", json.dumps(listed.get("dialogs"))[:300])


PHASES = "CTMLSK"


def run(ctx: qa.Run) -> None:
    project, shots, sizes = ctx.project, ctx.shots, ctx.args.sizes
    if not os.path.isfile(SHEET):
        raise RuntimeError("the bundled PDA sheet is missing: " + SHEET)
    # the copy's own session would bring the author's tabs back: it starts from none
    session = os.path.join(project, "temp", "session.ltx")
    if os.path.isfile(session):
        os.remove(session)
    only = ctx.only.upper().replace(",", "") or "CTMLK"
    # K runs in an editor of its own: its data root is a library the test makes
    phases = only.replace("K", "")
    if phases or sizes:
        ed = qa.Editor(project).start()
        try:
            steps = {"C": lambda: run_content(project, shots), "T": lambda: run_texture(project, shots),
                     "M": lambda: run_model(shots), "L": lambda: run_libraries(shots),
                     "S": lambda: run_sizes(project, shots)}
            # S, every tab at two sizes, runs when --sizes or --only asks for it
            qa.run_phases(steps, phases if sizes is False else phases + "S")
        finally:
            ed.close()
    if "K" in only:
        run_library_commands(project, shots)


def main() -> int:
    def more(ap) -> None:
        ap.add_argument("--sizes", action="store_true", help="also look at every tab at 1707x912 / 1.5 and 1280x720 / 1.0")

    return qa.suite("PREVIEWS", run, doc=__doc__, add_args=more, ignore=(".git",), phases="CTMLS")


if __name__ == "__main__":
    sys.exit(main())
