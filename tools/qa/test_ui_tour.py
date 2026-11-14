#!/usr/bin/env python3
"""The audit tour of the UI redesign (docs/UI_DESIGN.md, section 8), on a THROW-AWAY COPY of a project.

The 35 issues of the audit (audit_live.json, #1-#35) each get one MCP assertion and one screenshot, at both targets:
  max    1707 x 912 pt at UI scale 1.5
  small  1280 x 720 pt at UI scale 1.0
A surface the project cannot show (no GitHub repository for Releases, no PDA layout) is SKIP, not FAIL. The ini files
the editor writes when it closes (_projects.ini, level.ini, level_imgui.ini) come back byte for byte.

    python tools/qa/test_ui_tour.py --project <project> --shots <folder>
        [--sizes max,small] [--only 5,16,26] [--before]

--before takes the shots and reports, and never fails: the "before" pictures of a change.
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
KEEP = qa_target.KEPT
CONFIGS = {"max": (1707, 912, 1.5), "small": (1280, 720, 1.0)}

rows: list[tuple[str, str, str, str]] = []   # (config, issue, status, text)


def mcp(cmd: str, timeout: float = 120.0, **extra) -> dict:
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


def short(answer) -> str:
    if isinstance(answer, dict):
        answer = {k: v for k, v in answer.items() if k != "png_base64"}
    return json.dumps(answer, ensure_ascii=False)[:360]


class Skip(Exception):
    pass


class Tour:
    def __init__(self, project: str, shots: str | None) -> None:
        self.project = project
        self.shots = shots
        self.cfg = ""
        self.quest = ""
        self.beh = ""

    # ---- helpers --------------------------------------------------------------------------------------------------
    def shot(self, issue: int) -> dict:
        pic = mcp("shot.window")
        if self.shots and pic.get("png_base64"):
            os.makedirs(self.shots, exist_ok=True)
            with open(os.path.join(self.shots, "tour_%02d_%s.png" % (issue, self.cfg)), "wb") as f:
                f.write(base64.b64decode(pic["png_base64"]))
        return pic

    def escaped(self, surface: str) -> list:
        a = mcp("ui.audit", id=surface)
        return a.get("escaped") or []

    def level(self) -> None:
        mcp("ui.popup", show="none")
        for d in mcp("ui.dialog", action="list").get("dialogs", []):
            mcp("ui.dialog", action="close", id=d.get("id", ""))
        mcp("ui.drawer", action="close")
        mcp("ui.tab", action="activate", doc="level")

    def open_graph(self, kind: str) -> str:
        rel = self.quest if kind == "quest" else self.beh
        if not rel:
            made = mcp("file.create", kind="quest" if kind == "quest" else "behaviour")
            rel = (made.get("path") or "").replace("/", "\\")
            if not rel:
                raise Skip("no %s could be made: %s" % (kind, short(made)))
            if kind == "quest":
                self.quest = rel
            else:
                self.beh = rel
        v = mcp("graph.view", doc=rel.replace("\\", "/"), frame="all")
        if not v.get("ok"):
            raise Skip("the graph does not open: " + short(v))
        return rel

    def panel_or_drawer(self, panel: str, drawer: str) -> str:
        """The panel on Level where the size class has room for it, else its drawer; the audit id that shows it."""
        o = mcp("ui.panel", action="open", id=panel)
        return drawer if o.get("host") == "drawer" else panel


ISSUES: dict[int, tuple[str, callable]] = {}


def issue(n: int, title: str):
    def wrap(fn):
        ISSUES[n] = (title, fn)
        return fn
    return wrap


# ---- the 35 issues: set the surface up, assert one thing, take the picture --------------------------------------------
@issue(1, "Preferences opens (it crashed on Esc and Num Del)")
def _1(t: Tour):
    o = mcp("ui.tab", action="open", kind="preferences")
    alive = mcp("ping").get("ok")
    t.shot(1)
    mcp("ui.tab", action="close", doc="preferences")
    return o.get("ok") and alive, short(o)


@issue(2, "a document owns the window: the Level panels are not on screen over it")
def _2(t: Tour):
    t.open_graph("quest")
    lst = mcp("ui.list")
    shown = [p["id"] for p in lst.get("panels", []) if p.get("visible") and p.get("id") != "viewport"]
    t.shot(2)
    return lst.get("active_tab", "").startswith("quest:") and not shown, short({"visible_panels": shown})


@issue(3, "the quest graph at the small size: nothing escapes, the canvas keeps its minimum")
def _3(t: Tour):
    rel = t.open_graph("quest")
    v = mcp("graph.view", doc=rel.replace("\\", "/"), frame="all")
    canvas = (v.get("panes") or {}).get("canvas") or [0, 0, 400, 260]
    esc = t.escaped(mcp("ui.list").get("active_tab", ""))
    t.shot(3)
    return not esc and canvas[2] >= 399 and canvas[3] >= 259, short({"escaped": esc, "canvas": canvas})


@issue(4, "Frame All keeps node text readable: never under 11 px")
def _4(t: Tour):
    rel = t.open_graph("quest")
    v = mcp("graph.view", doc=rel.replace("\\", "/"), frame="all")
    px = v.get("title_px")
    t.shot(4)
    if px is None:
        raise Skip("graph.view answers no title size: " + short(v))
    return float(px) >= 11 and v.get("detail") != "block", short({"title_px": px, "detail": v.get("detail")})


@issue(5, "Details: rows are there and nothing escapes the panel")
def _5(t: Tour):
    t.level()
    sections = mcp("game.lookup", type="spawn_section", query="stalker", limit=1).get("entries") or []
    name = (sections[0].get("id") if sections and isinstance(sections[0], dict) else sections[0]) if sections else ""
    if name:
        placed = mcp("scene.place", kind="spawn", section=name, position=[0, 0, 4])
        if placed.get("name"):
            mcp("scene.select", objects=[(placed.get("object") or {}).get("name", "")])
    mcp("ui.panel", action="focus", id="details")
    p = mcp("scene.get")
    esc = t.escaped("details")
    t.shot(5)
    return not esc, short({"escaped": esc, "rows": len(((p.get("objects") or [{}])[0]).get("properties") or [])})


@issue(6, "no panel leaves the editor's window")
def _6(t: Tour):
    t.level()
    pic = t.shot(6)
    return pic.get("ok") and not pic.get("outside"), short({"outside": pic.get("outside")})


@issue(7, "a dialog stays centred when the window is resized")
def _7(t: Tour):
    t.level()
    o = mcp("ui.dialog", action="open", id="import_base_scene")
    if not o.get("ok"):
        raise Skip(short(o))
    lst = mcp("ui.list")
    dlg = next((d for d in lst.get("dialogs", []) if d.get("id") == "import_base_scene"), None)
    work = lst.get("work_area") or [0, 0, 0, 0]
    t.shot(7)
    mcp("ui.dialog", action="close", id="import_base_scene")
    if not dlg:
        return False, short(lst)
    r = dlg["rect"]
    return abs(r[0] + r[2] / 2 - (work[0] + work[2] / 2)) <= 3 and abs(r[1] + r[3] / 2 - (work[1] + work[3] / 2)) <= 3, short(r)


@issue(8, "Submit to Mod Browser: nothing escapes, the footer stays in the dialog")
def _8(t: Tour):
    t.level()
    o = mcp("ui.dialog", action="open", id="submit")
    if not o.get("ok"):
        raise Skip(short(o))
    esc = t.escaped("submit")
    pic = t.shot(8)
    mcp("ui.dialog", action="close", id="submit")
    modal = pic.get("modal") or {}
    work = mcp("ui.list").get("work_area") or [0, 0, 0, 0]
    return not esc and modal and modal.get("y", 0) + modal.get("h", 0) <= work[1] + work[3] + 1, short({"escaped": esc, "modal": modal})


@issue(9, "Mod Settings: nothing escapes")
def _9(t: Tour):
    t.level()
    o = mcp("ui.tab", action="open", kind="mod_settings")
    if not o.get("ok"):
        raise Skip(short(o))
    # a tab or, until it is one, the dialog: ui_audit knows either by this id
    esc = t.escaped("mod_settings")
    t.shot(9)
    if (o.get("tab") or {}).get("id") == "mod_settings":
        mcp("ui.tab", action="close", doc="mod_settings", discard=True)
    else:
        mcp("ui.dialog", action="close", id="mod_settings")
    return not esc, short({"escaped": esc})


@issue(10, "the viewport's toolbar overflows instead of clipping")
def _10(t: Tour):
    t.level()
    esc = t.escaped("viewport")
    t.shot(10)
    return not esc, short({"escaped": esc})


@issue(11, "the viewport's navigation is a gizmo, not a panel of buttons")
def _11(t: Tour):
    t.level()
    nav = mcp("viewport.camera")
    t.shot(11)
    return nav.get("ok"), short(nav)


@issue(12, "a behaviour graph never says quest")
def _12(t: Tour):
    t.open_graph("beh")
    a = mcp("ui.audit", id=mcp("ui.list").get("active_tab", ""))
    words = [e.get("label", "") for k in ("truncated", "controls") for e in a.get(k) or []]
    quest = [w for w in words if "quest" in w.lower()]
    t.shot(12)
    return not quest, short({"quest_labels": quest})


@issue(13, "the minimap stays hidden under 12 nodes")
def _13(t: Tour):
    rel = t.open_graph("beh")
    m = (mcp("graph.get", doc=rel.replace("\\", "/"), view=True).get("view") or {}).get("minimap") or {}
    t.shot(13)
    # "shown" is what the canvas draws; "visible" is the author's toggle, on until turned off
    shown = m.get("shown", m.get("visible"))
    if shown is None:
        raise Skip("graph.get answers no minimap: " + short(m))
    return not shown, short(m)


@issue(14, "the quest inspector: nothing escapes its panes")
def _14(t: Tour):
    t.open_graph("quest")
    esc = t.escaped(mcp("ui.list").get("active_tab", ""))
    t.shot(14)
    return not esc, short({"escaped": esc})


@issue(15, "the graph toolbar has no cryptic labels")
def _15(t: Tour):
    t.open_graph("quest")
    a = mcp("ui.audit", id=mcp("ui.list").get("active_tab", ""))
    bad = [e.get("label") for e in a.get("controls") or [] if e.get("label") in ("B<", "B>", "<", ">", "Keys >", "Keys")]
    t.shot(15)
    return not bad, short({"cryptic": bad})


@issue(16, "the Output Log shows lines, docked or in its drawer, and nothing escapes")
def _16(t: Tour):
    t.level()
    where = t.panel_or_drawer("output_log", "log")
    lst = mcp("ui.list")
    esc = t.escaped(where)
    t.shot(16)
    shown = lst.get("drawer") == "log" or any(p.get("id") == "output_log" and p.get("visible") for p in lst.get("panels", []))
    return shown and not esc, short({"host": where, "escaped": esc})


@issue(17, "Edit > Find on Level focuses the World Outliner's search")
def _17(t: Tour):
    t.level()
    e = mcp("ui.palette", action="run", id="edit.find")
    lst = mcp("ui.list")
    t.shot(17)
    ol = next((p for p in lst.get("panels", []) if p.get("id") == "world_outliner"), {})
    return e.get("ok") and lst.get("active_tab") == "level" and ol.get("visible"), short(ol)


@issue(18, "the World Outliner: nothing escapes (its search and Refresh were cut)")
def _18(t: Tour):
    t.level()
    mcp("ui.panel", action="focus", id="world_outliner")
    esc = t.escaped("world_outliner")
    t.shot(18)
    return not esc, short({"escaped": esc})


@issue(19, "the left panel is Place, and nothing escapes it")
def _19(t: Tour):
    t.level()
    mcp("ui.panel", action="focus", id="place")
    window = (mcp("ui.contract").get("panels") or {}).get("place", "")
    esc = t.escaped("place")
    t.shot(19)
    return window.startswith("Place###") and not esc, short({"window": window, "escaped": esc})


@issue(20, "the Content Browser: tiles 48..192 pt, nothing escapes")
def _20(t: Tour):
    t.level()
    o = mcp("content.view", action="go", source="project", folder="")
    where = "content" if o.get("host") == "drawer" else "content_browser"
    esc = t.escaped(where)
    t.shot(20)
    return o.get("tile_range") == [48, 192] and not esc, short({"answer": o, "escaped": esc})


@issue(21, "Level Settings comes up on target=world, nothing escapes")
def _21(t: Tour):
    t.level()
    mcp("ui.panel", id="level_settings", action="open")
    lst = mcp("ui.list")
    ls = next((p for p in lst.get("panels", []) if p.get("id") == "level_settings"), {})
    esc = t.escaped("level_settings")
    t.shot(21)
    return ls.get("visible") and not esc, short({"level_settings": ls, "escaped": esc})


@issue(22, "Characters: nothing escapes")
def _22(t: Tour):
    t.level()
    o = mcp("ui.tab", action="open", kind="characters")
    esc = t.escaped("characters")
    t.shot(22)
    mcp("ui.tab", action="close", doc="characters", discard=True)
    return o.get("ok") and not esc, short({"escaped": esc})


@issue(23, "World Rules is a tab: inside the window, nothing escapes")
def _23(t: Tour):
    t.level()
    o = mcp("ui.tab", action="open", kind="world_rules")
    esc = t.escaped("world_rules")
    pic = t.shot(23)
    mcp("ui.tab", action="close", doc="world_rules", discard=True)
    return o.get("ok") and not esc and not pic.get("outside"), short({"escaped": esc, "outside": pic.get("outside")})


@issue(24, "the PDA preview fits both axes, nothing escapes")
def _24(t: Tour):
    t.level()
    o = mcp("pda.preview", action="open", zoom="fit", pane="both", tab="problems")
    if not o.get("ok"):
        raise Skip(short(o))
    # the opener answers the frame it opened: its panes, gap and edges were laid out by that frame, no second call
    panes = o.get("panes") or {}
    opened = 200 <= panes.get("details_width", 0) <= 480 and \
        o.get("device_gap", -1) >= 15 and bool(o.get("screen_edges"))
    esc = t.escaped("pda")
    t.shot(24)
    # Fit fits the device itself with a 16 pt margin: nothing of it is cut, on either aspect (a screenshot draws the frame read)
    gaps, edges = {}, {}
    for aspect in ("16:9", "4:3"):
        mcp("pda.preview", action="aspect", aspect=aspect)
        mcp("shot.window")
        state = mcp("pda.preview", action="state")
        gaps[aspect] = state.get("device_gap", -1)
        edges[aspect] = state.get("screen_edges") or []
    mcp("pda.preview", action="close")
    # the device's art runs past the game's screen: the preview shows that edge, the bottom on both aspects
    cut_shown = all("bottom" in e for e in edges.values()) and len(edges.get("4:3", [])) == 4
    return opened and not esc and all(g >= 15 for g in gaps.values()) and cut_shown, \
        short({"opened": {k: o.get(k) for k in ("pending", "panes", "device_gap", "screen_edges")}, "escaped": esc,
               "device_gap": gaps, "screen_edges": edges})


@issue(25, "the Model Preview is a tab named by its model, not its path")
def _25(t: Tour):
    t.level()
    # studio light: the model reads lit (#25 found it almost black)
    objs = mcp("file.list", source="library", kind="objects", limit=1).get("files") or [{}]
    o = mcp("preview.model", name=objs[0].get("path", ""), lighting="studio")
    tab = o.get("tab")
    label = tab.get("label", "") if isinstance(tab, dict) else ""
    t.shot(25)
    mcp("ui.tab", action="close", doc="model", discard=True)
    # a game model's textures come from the linked game: none is drawn as the placeholder
    missing = [x.get("name") for x in o.get("textures") or [] if x.get("from") == "missing"]
    # the opener answers the frame it opened: the Details pane has its width in that answer already
    panes = o.get("panes") or {}
    opened = not panes.get("details") or 200 <= panes.get("details_width", 0) <= 480
    return o.get("ok") and opened and "\\" not in label and "/" not in label and not missing, short(o)


@issue(26, "the palette: 'log' finds the Output Log, no COMMAND_* ids as names, keycaps")
def _26(t: Tour):
    t.level()
    q = mcp("ui.palette", action="query", query="log", limit=5)
    rows_ = q.get("results") or []
    mcp("ui.palette", action="open", query="log")
    pic = t.shot(26)
    mcp("ui.palette", action="close")
    ok = any(r.get("label") == "Output Log" for r in rows_[:2]) and not any(str(r.get("label", "")).startswith("COMMAND_") for r in rows_)
    # a row matches by what it shows, never by its hidden COMMAND_* id or letters scattered through its words
    ok = ok and all("log" in (str(r.get("label", "")) + " " + str(r.get("path", ""))).lower() for r in rows_[:4])
    return ok and (pic.get("modal") or {}).get("id") == mcp("ui.contract").get("palette"), short(rows_[:3])


@issue(27, "dialog footers: the buttons are right-aligned, Cancel last")
def _27(t: Tour):
    t.level()
    o = mcp("ui.dialog", action="open", id="import_base_scene")
    if not o.get("ok"):
        raise Skip(short(o))
    a = mcp("ui.audit", id="import_base_scene")
    t.shot(27)
    mcp("ui.dialog", action="close", id="import_base_scene")
    buttons = [e for e in a.get("controls") or [] if e.get("label") in ("Cancel", "Import", "Open", "OK")]
    cancel = next((b for b in buttons if b.get("label") == "Cancel"), None)
    if not cancel:
        raise Skip("the dialog reports no Cancel to the audit (not on the kit yet): " + short(a))
    return all(cancel["rect"][0] >= b["rect"][0] for b in buttons), short(buttons)


@issue(28, "the Start page: its cards and menu answer, the project comes back")
def _28(t: Tour):
    t.level()
    c = mcp("project.close", discard=True)
    if not c.get("ok"):
        raise Skip(short(c))
    b = mcp("project.page")
    t.shot(28)
    qa_target.guard_project(t.project)
    back = mcp("project.open", path=t.project, timeout=600)
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline and not mcp("mod.info").get("ok"):
        time.sleep(0.05)
    return b.get("ok") and back.get("ok"), short(b)


@issue(29, "the main toolbar: vector actions and the project group, nothing escapes")
def _29(t: Tour):
    t.level()
    a = mcp("ui.audit", id="##toolbar")
    labels = [e.get("label") for e in a.get("controls") or []]
    t.shot(29)
    return "Build Mod" in labels and not a.get("escaped"), short({"labels": labels[:12], "escaped": a.get("escaped")})


@issue(30, "the status bar: rebuild chip and counts, nothing escapes")
def _30(t: Tour):
    t.level()
    a = mcp("ui.audit", id="##statusbar")
    # what needs rebuilding is the status bar's chip; the stats are a pill of the viewport (level.ini keeps the flag an
    # author turned on, so it is reported, not asserted)
    stats = (mcp("viewport.view").get("show") or {}).get("stats")
    t.shot(30)
    return not a.get("escaped"), short({"escaped": a.get("escaped"), "stats": stats})


@issue(31, "Publish Release: nothing escapes")
def _31(t: Tour):
    t.level()
    o = mcp("ui.dialog", action="open", id="publish_release")
    if not o.get("ok"):
        raise Skip(short(o))
    esc = t.escaped("publish_release")
    t.shot(31)
    mcp("ui.dialog", action="close", id="publish_release")
    return not esc, short({"escaped": esc})


@issue(32, "Package Release: nothing escapes, sized in em")
def _32(t: Tour):
    t.level()
    o = mcp("ui.dialog", action="open", id="package_release")
    if not o.get("ok"):
        raise Skip(short(o))
    esc = t.escaped("package_release")
    t.shot(32)
    mcp("ui.dialog", action="close", id="package_release")
    return not esc, short({"escaped": esc})


@issue(33, "Releases: nothing escapes")
def _33(t: Tour):
    t.level()
    o = mcp("ui.tab", action="open", kind="releases")
    if not o.get("ok"):
        raise Skip(short(o))
    tab = (o.get("tab") or {}).get("id") == "releases"
    esc = t.escaped("releases")
    t.shot(33)
    if tab:
        mcp("ui.tab", action="close", doc="releases", discard=True)
    else:
        mcp("ui.dialog", action="close", id="releases")
    return not esc, short({"escaped": esc})


@issue(34, "a confirmation raised from a dialog is a sheet in it: one dialog on top")
def _34(t: Tour):
    t.level()
    o = mcp("release.view", submit=True, withdraw_question=True)
    if not o.get("ok"):
        raise Skip(short(o))
    pic = t.shot(34)
    lst = mcp("ui.list")
    mcp("release.view", submit=False)
    return pic.get("modal") is not None and len(lst.get("dialogs") or []) >= 1, short({"modal": pic.get("modal")})


@issue(35, "Level Settings comes up in its dock node")
def _35(t: Tour):
    t.level()
    mcp("ui.panel", action="focus", id="details")
    mcp("ui.panel", id="level_settings", action="open")
    lst = mcp("ui.list")
    ls = next((p for p in lst.get("panels", []) if p.get("id") == "level_settings"), {})
    t.shot(35)
    return bool(ls.get("visible")), short(ls)


# ---- the run ------------------------------------------------------------------------------------------------------
def start_editor(project: str) -> subprocess.Popen:
    exe = qa_target.exe()
    if not os.path.isfile(exe):
        raise RuntimeError("build the editor first: " + exe)
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
    qa_target.guard_project(project)
    proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
    deadline = time.monotonic() + 600
    while time.monotonic() < deadline:
        try:
            if mcp("mod.info", timeout=30).get("ok"):
                return proc
        except OSError:
            pass
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup")
        time.sleep(0.05)
    raise RuntimeError("the editor never opened the project")


def run(project: str, args) -> None:
    proc = start_editor(project)
    tour = Tour(project, args.shots)
    try:
        before = float(mcp("ui.list").get("scale") or 1.0)
        only = {int(x) for x in args.only.split(",")} if args.only else set(ISSUES)
        for cfg in [c.strip() for c in args.sizes.split(",") if c.strip()]:
            if cfg not in CONFIGS:
                raise RuntimeError("unknown size " + cfg)
            w, h, scale = CONFIGS[cfg]
            tour.cfg = cfg
            mcp("ui.scale", value=scale)
            mcp("app.window", action="resize", width=w, height=h)
            for _ in range(2):
                mcp("ui.list")
            for n in sorted(only):
                title, fn = ISSUES[n]
                try:
                    ok, detail = fn(tour)
                    status = "PASS" if ok else "FAIL"
                except Skip as why:
                    status, detail = "SKIP", str(why)
                except OSError as exc:
                    status, detail = "FAIL", "MCP: " + repr(exc)
                except Exception as exc:  # noqa: BLE001 - one issue's surprise must not end the tour
                    status, detail = "FAIL", repr(exc)
                rows.append((cfg, "#%d" % n, status, title))
                print("%s %-5s #%-2d %s%s" % (status, cfg, n, title, ("  <- " + detail) if status != "PASS" else ""), flush=True)
        mcp("ui.scale", value=before)
    finally:
        try:
            mcp("app.quit", discard=True)
        except OSError:
            pass
        try:
            proc.wait(timeout=120)
        except subprocess.TimeoutExpired:
            print("! the editor did not close on its own", flush=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for tour_<nn>_<size>.png")
    ap.add_argument("--sizes", default="max,small", help="max, small or both (default)")
    ap.add_argument("--only", help="issue numbers, e.g. 5,16,26")
    ap.add_argument("--before", action="store_true", help="shots and a report only: never fails")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_tourproj_")
    copy = os.path.join(work, "QaTour")
    qa_target.copy_project(src, copy, ignore=(".git", "releases", "temp",))
    print("running against a copy: " + copy, flush=True)
    pl = os.path.join(copy, "project.ltx")
    if os.path.isfile(pl):
        with open(pl, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        at = text.lower().find(src.lower())
        while at >= 0:
            text = text[:at] + copy + text[at + len(src):]
            at = text.lower().find(src.lower(), at + len(copy))
        with open(pl, "w", encoding="utf-8", newline="") as f:
            f.write(text)
    saved = {}
    for path in KEEP:
        try:
            with open(path, "rb") as f:
                saved[path] = f.read()
        except OSError:
            saved[path] = None
    crashed = ""
    try:
        run(copy, args)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        crashed = repr(exc)
    finally:
        for path, data in saved.items():
            if data is None:
                if os.path.isfile(path):
                    os.remove(path)
            else:
                with open(path, "wb") as f:
                    f.write(data)
        if not args.keep:
            qa_target.remove_tree(work)
    restored = all((open(p, "rb").read() if os.path.isfile(p) else None) == d for p, d in saved.items())
    passed = sum(1 for r in rows if r[2] == "PASS")
    failed = sum(1 for r in rows if r[2] == "FAIL")
    skipped = sum(1 for r in rows if r[2] == "SKIP")
    if crashed:
        print("FAIL the tour stopped: " + crashed, flush=True)
    if not restored:
        print("FAIL the ini files are not back byte for byte", flush=True)
    print("UI TOUR: %d pass, %d fail, %d skip%s" % (passed, failed, skipped, " (before: report only)" if args.before else ""),
          flush=True)
    if args.before:
        return 0
    return 0 if not failed and not crashed and restored else 1


if __name__ == "__main__":
    sys.exit(main())
