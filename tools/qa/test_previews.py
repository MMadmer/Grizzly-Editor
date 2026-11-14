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

import json
import os
import shutil
import sys
import time

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


PHASES = "CTMLS"


def run(ctx: qa.Run) -> None:
    project, shots, sizes = ctx.project, ctx.shots, ctx.args.sizes
    if not os.path.isfile(SHEET):
        raise RuntimeError("the bundled PDA sheet is missing: " + SHEET)
    # the copy's own session would bring the author's tabs back: it starts from none
    session = os.path.join(project, "temp", "session.ltx")
    if os.path.isfile(session):
        os.remove(session)
    only = ctx.only.upper().replace(",", "") or "CTML"
    ed = qa.Editor(project).start()
    try:
        steps = {"C": lambda: run_content(project, shots), "T": lambda: run_texture(project, shots), "M": lambda: run_model(shots),
                 "L": lambda: run_libraries(shots), "S": lambda: run_sizes(project, shots)}
        # S, every tab at two sizes, runs when --sizes or --only asks for it
        qa.run_phases(steps, only if sizes is False else only + "S")
    finally:
        ed.close()


def main() -> int:
    def more(ap) -> None:
        ap.add_argument("--sizes", action="store_true", help="also look at every tab at 1707x912 / 1.5 and 1280x720 / 1.0")

    return qa.suite("PREVIEWS", run, doc=__doc__, add_args=more, ignore=(".git",), phases="CTMLS")


if __name__ == "__main__":
    sys.exit(main())
