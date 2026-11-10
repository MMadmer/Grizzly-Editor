#!/usr/bin/env python3
"""End-to-end check of the preview and library tabs (docs/UI_DESIGN.md, section 6: Model Preview, Texture Preview,
Libraries) over MCP, on a THROW-AWAY COPY of a project. Nothing of the editor's libraries is changed: the
library tabs are opened, searched and looked at, never saved.

What it proves:
  T  texture_preview: a project image, an editor library texture and a game one open in the texture tab, each
     naming where it lives (origin) apart from its path; fit, 1:1 and a
     factor; the channel views RGBA, R, G, B and A (rgb still names RGBA); a mip; the Details pane on and off
     and keeping its pt size; a bad channel or zoom is refused; close frees the tab
  M  preview_model: a model (the editor's objects, else the linked game's meshes) opens in the model tab,
     reports its geometry, switches Studio / Unlit, wireframe, bones and Details, frames and resets; a character
     of the linked game names its textures, none of them missing (the loader reads them out of the game)
     T and M: an opener's answer is the frame it opened (panes in pt, the zoom a fit came to), not the one before
  L  library: Light Animations, Texture Library and Sound Library open as tabs, count and search their items,
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
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(1)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("window_control", action="close", discard=True)
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def shot(folder: str | None, name: str) -> dict:
    # a screenshot is also how a frame gets drawn in a minimized editor: the tab lays itself out in it
    pic = mcp("screenshot_editor")
    if folder and pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)
    return pic


def escaped_in_tab() -> list | None:
    """What the kit saw spill out of the work area's windows last frame; None when ui_audit is not there."""
    a = mcp("ui_audit")
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


# ---------------------------------------------------------------------------------------------------- T
def run_texture(project: str, shots: str | None) -> None:
    os.makedirs(os.path.join(project, "qa_previews"), exist_ok=True)
    shutil.copyfile(SHEET, os.path.join(project, "qa_previews", "sheet.dds"))
    t = mcp("texture_preview", action="open", path="qa_previews/sheet.dds", source="project")
    check(t.get("ok") and t.get("tab") == "texture" and t.get("active") and t.get("open"), "T a project image opens the texture tab",
          json.dumps(t)[:300])
    check(t.get("width", 0) > 0 and t.get("height", 0) > 0 and t.get("fit") is True and t.get("channel") == "rgba",
          "T it reports its size, fits and shows RGBA", json.dumps(t)[:300])
    check(t.get("container") == "DDS" and t.get("decoded") is False, "T a DDS is shown as the GPU holds the file, not decoded",
          json.dumps(t)[:300])
    check(t.get("origin") == "project" and t.get("file"), "T it names where it lives, apart from its path", json.dumps(t)[:300])
    # an opener answers the frames it owed (docs/UI_DESIGN.md, section 7): the pane and the fit are laid out already
    panes = t.get("panes", {})
    check(t.get("pending") is False and 200 <= panes.get("details_width", 0) <= 480 and 0 < t.get("zoom", 0) <= 1.0
          and abs(t.get("zoom", 0) - t.get("scale", -1)) < 1e-3, "T the opener's answer is the frame it opened",
          json.dumps({k: t.get(k) for k in ("pending", "panes", "zoom", "fit", "scale")}))
    shot(shots, "texture_fit")
    t = mcp("texture_preview")
    panes = t.get("panes", {})
    check(panes.get("details") is True and 200 <= panes.get("details_width", 0) <= 480, "T Details keeps its pt size", json.dumps(panes))
    check(0 < t.get("scale", 0) <= 1.0, "T fit never blows a texture up past 1:1", str(t.get("scale")))
    audit("T fit")
    t = mcp("texture_preview", zoom="1")
    check(t.get("zoom") == 1 and t.get("fit") is False, "T 1:1", json.dumps(t)[:200])
    shot(shots, "texture_1to1")
    t = mcp("texture_preview", zoom="4")
    check(abs(t.get("zoom", 0) - 4) < 1e-3, "T a factor", json.dumps(t)[:200])
    shot(shots, "texture_4x")
    for ch in ("r", "g", "b", "a"):
        t = mcp("texture_preview", channel=ch, zoom="fit")
        check(t.get("channel") == ch, "T the " + ch.upper() + " view", json.dumps(t)[:200])
    shot(shots, "texture_alpha")
    t = mcp("texture_preview", channel="rgb")
    check(t.get("channel") == "rgba", "T rgb still names the colour view", json.dumps(t)[:200])
    bad = mcp("texture_preview", channel="x")
    check(not bad.get("ok"), "T an unknown channel is refused", json.dumps(bad)[:200])
    bad = mcp("texture_preview", zoom="-2")
    check(not bad.get("ok"), "T a zoom that is no factor is refused", json.dumps(bad)[:200])
    if t.get("mips", 0) > 1:
        t = mcp("texture_preview", mip=1)
        check(t.get("mip") == 1, "T a mip", json.dumps(t)[:200])
        mcp("texture_preview", mip=0)
    t = mcp("texture_preview", details=False)
    check(t.get("panes", {}).get("details") is False, "T Details off", json.dumps(t.get("panes")))
    shot(shots, "texture_no_details")
    t = mcp("texture_preview", details=True)
    check(t.get("panes", {}).get("details") is True, "T Details on")
    # the editor's own library: a texture every SDK has
    t = mcp("texture_preview", action="open", path="ed/ed_nodata", source="editor")
    if t.get("ok"):
        check(t.get("name", "").replace("/", "\\") == "ed\\ed_nodata" and t.get("width", 0) > 0 and t.get("origin") == "library",
              "T an editor library texture", json.dumps(t)[:300])
    else:
        note("the editor library has no ed\\ed_nodata: " + json.dumps(t)[:200])
    # the linked game's: a block-compressed file names its own format and where the game keeps it
    game = mcp("darf_list", ext="dds", filter="textures", limit=1)
    files = game.get("files", []) if game.get("ok") else []
    if files and files[0].get("path"):
        t = mcp("texture_preview", action="open", path=files[0]["path"], source="darf")
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
    bad = mcp("texture_preview", action="open", path="qa_previews/none.dds", source="project")
    check(not bad.get("ok"), "T a file that is not there is refused", json.dumps(bad)[:200])
    t = mcp("texture_preview", action="close")
    check(t.get("ok") and t.get("open") is False, "T close frees the tab", json.dumps(t)[:200])


# ---------------------------------------------------------------------------------------------------- M
def find_model() -> dict | None:
    objs = mcp("list_assets", category="Objects", limit=5)
    for a in objs.get("assets", []) if objs.get("ok") else []:
        if a.get("name"):
            return {"name": a["name"]}
    # a prop of the game's: a static mesh the model pool always takes
    game = mcp("darf_list", ext="ogf", filter="dynamics", limit=5)
    for a in game.get("files", []) if game.get("ok") else []:
        if a.get("path"):
            return {"name": a["path"], "source": "darf"}
    return None


def run_model(shots: str | None) -> None:
    model = find_model()
    if not model:
        note("no editor object and no linked game model: the model tab is not checked")
        return
    m = mcp("preview_model", **model)
    check(m.get("ok") and m.get("tab") == "model" and m.get("active") and m.get("open"), "M a model opens the model tab",
          json.dumps(m)[:300])
    check(not m.get("error") and m.get("verts", 0) > 0 and m.get("tris", 0) > 0, "M it reports its geometry", json.dumps(m)[:300])
    check(m.get("lighting") == "studio", "M the headlight is the default", json.dumps(m)[:200])
    check(m.get("pending") is False and 200 <= m.get("panes", {}).get("details_width", 0) <= 480,
          "M the opener's answer is the frame it opened", json.dumps({k: m.get(k) for k in ("pending", "panes")}))
    shot(shots, "model_studio")
    m = mcp("preview_model")
    panes = m.get("panes", {})
    check(panes.get("details") is True and 200 <= panes.get("details_width", 0) <= 480, "M Details keeps its pt size", json.dumps(panes))
    audit("M studio")
    m = mcp("preview_model", lighting="unlit")
    check(m.get("lighting") == "unlit", "M Unlit", json.dumps(m)[:200])
    shot(shots, "model_unlit")
    m = mcp("preview_model", lighting="studio", wireframe=True)
    check(m.get("wireframe") is True and m.get("lighting") == "studio", "M wireframe", json.dumps(m)[:200])
    shot(shots, "model_wireframe")
    m = mcp("preview_model", wireframe=False, bones=True)
    check(m.get("wireframe") is False and (m.get("bones_shown") is False or m.get("bones", 0) > 0), "M bones only where there are some",
          json.dumps(m)[:200])
    m = mcp("preview_model", bones=False, details=False, frame=True)
    check(m.get("ok") and m.get("panes", {}).get("details") is False, "M Details off, framed", json.dumps(m)[:200])
    shot(shots, "model_no_details")
    m = mcp("preview_model", details=True, reset=True)
    check(m.get("ok") and m.get("panes", {}).get("details") is True, "M Details on, the view reset")
    bad = mcp("preview_model", lighting="neon")
    check(not bad.get("ok"), "M an unknown lighting is refused", json.dumps(bad)[:200])
    run_game_character(shots)


# a character of the linked game draws with textures only its archives hold: each is found, none is the placeholder
def run_game_character(shots: str | None) -> None:
    game = mcp("darf_list", ext="ogf", filter="actors", limit=5)
    for a in game.get("files", []) if game.get("ok") else []:
        if not a.get("path"):
            continue
        m = mcp("preview_model", name=a["path"], source="darf", lighting="studio")
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
    s = mcp("library", kind=kind, action="state")
    check(not s.get("ok") or s.get("tab") == kind, "L " + kind + ": state of a closed tab refuses or answers", json.dumps(s)[:200])
    o = mcp("library", kind=kind, action="open")
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
    s = mcp("library", kind=kind, action="state", search=leaf)
    check(s.get("ok") and 0 < s.get("shown", 0) <= s.get("items", 0), "L " + kind + " searches", json.dumps(s)[:200])
    s = mcp("library", kind=kind, action="select", item=first)
    check(s.get("ok") and s.get("selected") == first, "L " + kind + " selects " + first, json.dumps(s)[:300])
    shot(shots, kind + "_selected")
    bad = mcp("library", kind=kind, action="select", item="qa-no-such-item-anywhere")
    check(not bad.get("ok"), "L " + kind + " refuses an item it does not have", json.dumps(bad)[:200])
    mcp("library", kind=kind, action="state", search="")
    # a state call never switches tabs
    if before and before != kind:
        mcp("ui_tab", action="activate", id=before)
        s = mcp("library", kind=kind, action="state")
        check(s.get("ok") and s.get("active") is False, "L " + kind + ": state leaves the active tab alone", json.dumps(s)[:200])
    s = mcp("library", kind=kind, action="state")
    check(s.get("dirty") is False, "L " + kind + " is still clean", json.dumps(s)[:200])
    c = mcp("ui_tab", action="close", id=kind)
    if c.get("ok") is False and "unknown" in str(c.get("error", "")):
        note("ui_tab is not there: the " + kind + " tab stays open")


def run_libraries(shots: str | None) -> None:
    bad = mcp("library", kind="nope")
    check(not bad.get("ok"), "L an unknown kind is refused", json.dumps(bad)[:200])
    for kind in ("light_animations", "sound_library", "texture_library"):
        run_library(kind, shots)


# ---------------------------------------------------------------------------------------------------- S
def run_sizes(project: str, shots: str | None) -> None:
    sizes = [(1707, 912, 1.5, "large"), (1280, 720, 1.0, "small")]
    for w, h, scale, tag in sizes:
        r = mcp("window_control", action="resize", width=w, height=h)
        u = mcp("ui_scale", value=scale)
        if not r.get("ok") or not u.get("ok"):
            note("window_control resize / ui_scale are not there: sizes not checked")
            return
        t = mcp("texture_preview", action="open", path="qa_previews/sheet.dds", source="project")
        check(t.get("ok"), "S " + tag + ": the texture tab")
        shot(shots, "size_%s_texture" % tag)
        audit("S " + tag + " texture")
        model = find_model()
        if model:
            mcp("preview_model", **model)
            shot(shots, "size_%s_model" % tag)
            audit("S " + tag + " model")
        for kind in ("light_animations", "sound_library"):
            if mcp("library", kind=kind, action="open").get("ok"):
                shot(shots, "size_%s_%s" % (tag, kind))
                audit("S " + tag + " " + kind)
    mcp("ui_scale", value=1.5)


def run(project: str, shots: str | None, sizes: bool) -> None:
    ed = Editor()
    try:
        ed.start(project)
        run_texture(project, shots)
        run_model(shots)
        run_libraries(shots)
        if sizes:
            run_sizes(project, shots)
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for the tabs' screenshots")
    ap.add_argument("--sizes", action="store_true", help="also look at every tab at 1707x912 / 1.5 and 1280x720 / 1.0")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    if not os.path.isfile(SHEET):
        print("the bundled PDA sheet is missing: " + SHEET)
        return 2

    work = tempfile.mkdtemp(prefix="grizzly_previews_")
    copy = os.path.join(work, "QaPreviews")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git"))
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
    keep = [os.path.join(REPO, n) for n in ("_projects.ini", "level.ini", "level_imgui.ini")]
    backups = {}
    for path in keep:
        if os.path.isfile(path):
            with open(path, "rb") as f:
                backups[path] = f.read()

    try:
        run(copy, args.shots, args.sizes)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        for path, data in backups.items():
            with open(path, "wb") as f:
                f.write(data)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"PREVIEWS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
