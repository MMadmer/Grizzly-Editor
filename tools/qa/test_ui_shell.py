#!/usr/bin/env python3
"""The editor's frame over MCP (docs/UI_DESIGN.md, sections 5 and 7), on a THROW-AWAY COPY of a project.

What it proves (every length in pt, as the commands answer them; a picture is px with its `scale`):
  A  the frame reports itself: state and ui.list name the Level tab, the panels, the drawers; ui.contract has the
     constants QA pins (panels, drawers, dialogs, menus, the palette's id); the minimised start's profile says
     'window minimised', and one start of its own that SHOWS the window (not activated) had it within 0.3 s
  B  the caption: 32 pt tall, the window buttons 3 x 46 pt at its right end; the major tabs on a row of their own,
     30 pt, right under it - and Windows agrees: WM_NCHITTEST answers HTCLIENT on a tab and on a window button,
     HTCAPTION on the drag band
  C  the major tabs: a tool tab opens, Level stays first and pinned (it refuses to close), next/prev cycle and move no
     tab, a clean tab closes; a quest made dirty refuses a plain close ('unsaved') and closes with discard=true
  D  the drawers: over a document the Content and Log drawers open and close; ui.list names the one that is up
  E  the Level panels: close, open, focus and toggle round trips; ui.scroll, ui.split, ui.toast (a note with a button
     that runs an action)
  F  the dialogs: Import Base Scene opens and closes over ui.dialog and stays centred in the work area at both sizes;
     on the Start page the status bar has no centre and no drawer buttons
  G  the size: under 1100 pt the menus fold into one Menu button (ui.frame folded, ui.popup menu:Menu), the size
     class follows the window, ui.scale rescales the frame (the caption 32 pt at 1.0 and 1.5)
  H  menus and the palette: a caption menu has ONE label column (every row's label starts at one x); one popup at a
     time; the palette ranks a log row first for 'log', shows no COMMAND_* id, names a tab by its project path;
     opened, it is the modal on top
  I  actions: a palette row runs by its id (window.log_drawer opens the Log drawer) and comes back first as recent;
     app.keys lists every action with its group (the graph's under Graph); a binding made in app.keys is what the key
     pass finds (app.keys resolve), the old key no longer runs it, the palette shows the new key; reset puts it back;
     the default keys are Unreal's (Ctrl+B shows the tab's file in the Content Browser, Ctrl+Shift+S saves all,
     Alt+P plays, Shift+1 is Select mode, Ctrl+` World / Local, ` the Output Log) and no long work has a default key;
     Ctrl+B's reveal selects the tab's file in the browser
  J  stable chrome and stalls: every tab kind is opened; switching between them moves no part of the frame (the
     caption, the tab row and every tab, the toolbar row, the project's group, the status bar), a tool tab's empty
     toolbar row holds its status; after the whole tour perf.stalls names no UI pass over 100 ms; the Task Center
     lists what ended
  K  `loading` is no empty promise: while the project's scene is read (a project closed and opened again) `ui.audit`
     answers a non-empty `loading` - the busy lines and the busy headers of the surfaces that wait, which is what
     test_tour's wait looks for - and an empty one once it is read; the Place panel's list that is read shows no
     count (`level.view`'s categories answer `count: null, loading: true`), never 0

    python tools/qa/test_ui_shell.py --project <project> [--only A,F,...] [--shots <folder>]

The editor runs under -qa_github with every service address on the loopback: opening Releases asks nothing of
GitHub.
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import json
import os
import sys
import tempfile
import time

import qa
from qa import check, cmd, shot


WIDE = (1500, 800)       # pt: the Wide size class (W >= 1400, H >= 640)
COMPACT = (1280, 720)    # pt: Compact
NARROW = (1000, 700)     # pt: under 1100 the caption's menus fold
PHASES = "ABCDEFGHIJK"


def short(answer: dict) -> str:
    return json.dumps({k: v for k, v in answer.items() if k != "png_base64"})[:400]


def resize(w: int, h: int) -> dict:
    r = cmd("app.window", action="resize", width=w, height=h)
    # the swap chain follows the window on the next frames: let them be drawn before anyone looks
    for _ in range(2):
        cmd("ui.list")
    return r


# ---- Win32: what the window's hit test answers where the caption drew something ------------------------------------
user32 = ctypes.windll.user32
HTCLIENT, HTCAPTION = 1, 2
# The editor is Per-Monitor V2 and speaks physical px; an unaware process gets its coordinates scaled by the monitor's
# DPI, and a point put through ClientToScreen lands on another part of the caption.
if not user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4)):
    ctypes.windll.shcore.SetProcessDpiAwareness(2)


def editor_hwnd(pid: int) -> int | None:
    """The window of this run's editor: other editors may run beside it (tools/qa/run_all.py)."""
    found: list[int] = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def each(hwnd, _):
        cls = ctypes.create_unicode_buffer(64)
        user32.GetClassNameW(hwnd, cls, 64)
        owner = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if cls.value == "Grizzly Editor" and owner.value == pid:
            found.append(hwnd)
        return True

    user32.EnumWindows(each, 0)
    return found[0] if found else None


def hit_test(hwnd: int, x: float, y: float) -> int:
    pt = wt.POINT(int(x), int(y))
    user32.ClientToScreen(hwnd, ctypes.byref(pt))
    lparam = ((pt.y & 0xFFFF) << 16) | (pt.x & 0xFFFF)
    result = ctypes.c_size_t(0)
    user32.SendMessageTimeoutW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM, wt.UINT, wt.UINT, ctypes.POINTER(ctypes.c_size_t)]
    user32.SendMessageTimeoutW(hwnd, 0x0084, 0, lparam, 0x0002, 5000, ctypes.byref(result))
    return int(result.value)


def near(a: float, b: float, tol: float) -> bool:
    return abs(a - b) <= tol


def same_rect(a, b, tol: float = 0.6) -> bool:
    if a is None or b is None:
        return a is None and b is None
    return len(a) == len(b) and all(near(x, y, tol) for x, y in zip(a, b))


def run(ctx: qa.Run) -> None:
    project, shots = ctx.project, ctx.shots
    ed = qa.Editor(project).start()
    try:
        before_scale = float(cmd("ui.list").get("scale") or 1.0)
        contract = cmd("ui.contract")
        qa.run_phases(PHASE, ctx.only, None, project, shots, ed, before_scale, contract)
    finally:
        ed.close()


def phase_a(project, shots, ed, before_scale, contract) -> None:
    st = cmd("state")
    check(st.get("active_tab") == "level" and any(t.get("id") == "level" for t in st.get("tabs", [])),
          "A state names the Level tab as the active one", short(st))
    check(st.get("window") is True and isinstance(st.get("jobs"), list) and "problems" in st,
          "A state says there is a window, the jobs that run and the problem counts", short(st))
    ping = cmd("ping")
    check(ping.get("name") == "Grizzly Editor" and str(ping.get("version", "")).count(".") == 2,
          "A ping names the editor and the version it was built as", short(ping))
    panel_ids = set((contract.get("panels") or {}).keys())
    check({"place", "world_outliner", "details", "level_settings", "content_browser", "output_log", "viewport"} <= panel_ids,
          "A ui.contract lists the seven Level panels", short(contract))
    check(set((contract.get("drawers") or {}).keys()) == {"content", "log"}, "A ui.contract lists the two drawers", short(contract))
    dialogs = contract.get("dialogs") or {}
    check("import_base_scene" in dialogs and all("###dlg." in w for i, w in dialogs.items() if i not in ("mod_settings", "releases")),
          "A the contract's dialogs are Title###dlg.<name>", json.dumps(dialogs)[:400])
    check(bool(contract.get("palette")) and "Menu" in (contract.get("menus") or []) and contract.get("stall_ms") == 100,
          "A ui.contract names the palette's id, the folded Menu and the stall limit", short(contract))
    resize(*WIDE)
    lst = cmd("ui.list")
    check(lst.get("size_class") == "wide" and lst.get("start_page") is False, "A a 1500 x 800 pt window is the Wide class",
          short(lst))
    centre = lst.get("status_centre")
    check(isinstance(centre, list) and len(centre) == 4 and centre[2] > 0,
          "A the status bar's centre holds the Level tab's status", json.dumps(centre))
    shot(shots, "frame_level")
    marks = shown_start()
    # the suite's own editor starts minimised: its profile says so, and says nothing of a shown window
    own = start_marks(os.path.join(qa.home(), "logs", "grizzly_*.log"))
    check("window minimised" in own and "window shown" not in own,
          "A a minimised start's profile says 'window minimised'", json.dumps(own))
    shown = marks.get("window shown")
    check(shown is not None and shown <= 0.3 and "first frame" in marks and "window minimised" not in marks,
          "A a shown window was on the screen within 0.3 s of the process start (%s s; first frame %s s)"
          % (shown, marks.get("first frame")), json.dumps(marks), timing=True)


def start_marks(pattern: str) -> dict:
    """The start-up profile's milestones in the logs `pattern` names."""
    import glob
    import re
    marks = {}
    for path in glob.glob(pattern):
        text = open(path, encoding="utf-8", errors="replace").read()
        m = re.search(r"startup after the process was created: (.*)", text)
        if m:
            marks = {k.strip(): float(v) for k, v in re.findall(r"([a-z ]+?) ([0-9.]+) s", m.group(1))}
    return marks


def shown_start() -> dict:
    """One start of an editor of its own (no project, a home and port of its own) whose window is SHOWN without
    taking the foreground: what the 0.3 s budget is about. Closed over MCP; its milestones."""
    home = tempfile.mkdtemp(prefix="grizzly_shown_")
    shown = qa.Editor(port=qa.free_port(), home_folder=home, visible=True, stalls=False, ready="ping",
                      log_output=False)
    try:
        shown.start()
        deadline = time.monotonic() + 60
        marks: dict = {}
        while time.monotonic() < deadline and shown.alive():
            marks = start_marks(os.path.join(home, "logs", "grizzly_*.log"))
            if "first frame" in marks:
                break
            time.sleep(0.05)
        return marks
    finally:
        shown.close(wait=30)
        qa.remove_tree(home)


def phase_b(project, shots, ed, before_scale, contract) -> None:
    resize(*WIDE)
    fr = cmd("ui.frame")
    scale = float(fr.get("scale") or 1.0)
    band = fr.get("band") or [0, 0, 0, 0]          # [x, y, w, h] pt
    check(near(fr.get("caption_h", 0), 32, 0.6) and near(band[3], 32, 0.6), "B the caption is 32 pt tall", short(fr))
    check(near(band[0] + band[2] - fr.get("buttons_x", 0), 3 * 46, 1), "B the window buttons are 3 x 46 pt at its right end",
          short(fr))
    tabs = fr.get("tabs") or []
    inter = fr.get("interactive") or [[0, 0], [0, 0]]
    row = fr.get("tab_row") or [0, 0, 0, 0]
    check(near(row[1], band[1] + band[3], 0.6) and near(row[3], 30, 0.6),
          "B the tabs have a row of their own, 30 pt, right under the caption", short(fr))
    check(bool(tabs) and all(row[1] - 0.6 <= t["rect"][1] and t["rect"][1] + t["rect"][3] <= row[1] + row[3] + 0.6 for t in tabs),
          "B every tab lies in the tab row, none in the caption", short(fr))
    hwnd = editor_hwnd(ed.proc.pid if ed.proc else 0)
    if check(bool(hwnd), "B the editor's window is found"):
        cy = (band[1] + band[3] / 2.0) * scale
        level = next((t for t in tabs if t.get("id") == "level"), tabs[0] if tabs else None)
        if level:
            r = level["rect"]
            got = hit_test(hwnd, (r[0] + r[2] / 2.0) * scale, (r[1] + r[3] / 2.0) * scale)
            check(got == HTCLIENT, "B the Level tab takes the mouse (HTCLIENT)", "hit %d" % got)
        drag_x = (inter[0][1] + fr.get("buttons_x", 0)) / 2.0 * scale
        got = hit_test(hwnd, drag_x, cy)
        check(got == HTCAPTION, "B the band between the menus and the buttons drags the window (HTCAPTION)", "hit %d" % got)
        got = hit_test(hwnd, (band[0] + band[2] - 23) * scale, cy)
        check(got == HTCLIENT, "B the close button takes the mouse (HTCLIENT)", "hit %d" % got)


def tab_rects() -> dict:
    return {t.get("id"): t.get("rect") for t in cmd("ui.frame").get("tabs") or []}


def make_quest() -> str:
    made = cmd("file.create", kind="quest")
    return (made.get("path") or "").replace("/", "\\")


def phase_c(project, shots, ed, before_scale, contract) -> None:
    o = cmd("ui.tab", action="open", kind="characters")
    check(o.get("ok") and (o.get("tab") or {}).get("id") == "characters", "C a tool tab opens and comes up at once", short(o))
    lt = cmd("ui.tab", action="list")
    ids = [t.get("id") for t in lt.get("tabs", [])]
    check(ids[:1] == ["level"] and "characters" in ids, "C Level is the first tab, the tool tab follows", json.dumps(ids))
    before_switch = tab_rects()
    nx = cmd("ui.tab", action="next")
    check(nx.get("ok") and (nx.get("tab") or {}).get("id") != "characters", "C next cycles away from the active tab", short(nx))
    after_switch = tab_rects()
    check(len(before_switch) >= 2 and all(same_rect(before_switch[k], after_switch.get(k)) for k in before_switch),
          "C switching tabs moves no tab", json.dumps({"before": before_switch, "after": after_switch}))
    pv = cmd("ui.tab", action="prev")
    check((pv.get("tab") or {}).get("id") == "characters", "C prev comes back", short(pv))
    lv = cmd("ui.tab", action="close", doc="level")
    check(not lv.get("ok"), "C the Level tab refuses to close", short(lv))
    cl = cmd("ui.tab", action="close", doc="characters")
    check(cl.get("ok") and "characters" not in [t.get("id") for t in cl.get("tabs", [])], "C a clean tab closes", short(cl))

    rel = make_quest()
    if check(bool(rel), "C a quest is made for the dirty-close check"):
        op = cmd("ui.tab", action="open", kind="quest", path=rel)
        quest = (op.get("tab") or {}).get("id", "")
        check(op.get("ok") and quest.startswith("quest:"), "C the quest opens as a tab of its own", short(op))
        on_quest = tab_rects()
        cmd("ui.tab", action="prev")
        on_level = tab_rects()
        cmd("ui.tab", action="next")
        check(quest in on_quest and all(same_rect(on_quest[k], on_level.get(k)) for k in on_quest),
              "C from the quest tab to Level and back, no tab moves", json.dumps({"quest": on_quest, "level": on_level}))
        with open(os.path.join(project, rel), "rb") as f:
            on_disk = f.read()
        cmd("graph.view", doc=quest, select="all")
        cmd("graph.edit", doc=quest, ops=[{"op": "duplicate"}])
        info = cmd("ui.tab", action="list")
        dirty = any(t.get("id") == quest and t.get("dirty") for t in info.get("tabs", []))
        if check(dirty, "C an edit leaves the quest tab dirty", short(info)):
            refused = cmd("ui.tab", action="close", doc=quest)
            check(not refused.get("ok") and refused.get("code") == "unsaved" and refused.get("dirty"),
                  "C a dirty tab refuses a plain close and names what is unsaved", short(refused))
        gone = cmd("ui.tab", action="close", doc=quest, discard=True)
        check(gone.get("ok"), "C discard=true closes it", short(gone))
        with open(os.path.join(project, rel), "rb") as f:
            check(f.read() == on_disk, "C the discarded edit never reached the file")


def phase_d(project, shots, ed, before_scale, contract) -> None:
    cmd("ui.tab", action="open", kind="characters")
    for which in ("log", "content"):
        d = cmd("ui.drawer", action="open", id=which)
        lst = cmd("ui.list")
        check(d.get("ok") and lst.get("drawer") == which, "D the %s drawer opens over a document" % which, short(lst))
        c = cmd("ui.drawer", action="close", id=which)
        lst = cmd("ui.list")
        check(c.get("ok") and lst.get("drawer") == "none", "D the %s drawer closes" % which, short(lst))
    # a drawer opened over one tab does not cover the next one's canvas
    cmd("ui.drawer", action="open", id="content")
    cmd("ui.tab", action="activate", doc="level")
    lst = cmd("ui.list")
    check(lst.get("drawer") == "none", "D another tab coming up closes the drawer", short(lst))
    cmd("ui.tab", action="activate", doc="characters")
    cmd("ui.tab", action="close", doc="characters")


def phase_e(project, shots, ed, before_scale, contract) -> None:
    cmd("ui.tab", action="activate", doc="level")
    for panel in ("world_outliner", "details", "level_settings", "place"):
        c = cmd("ui.panel", action="close", id=panel)
        check(c.get("ok") and c.get("open") is False, "E %s closes" % panel, short(c))
        o = cmd("ui.panel", action="open", id=panel)
        check(o.get("ok") and o.get("open") is True and o.get("active_tab") == "level", "E %s opens on Level" % panel, short(o))
    f = cmd("ui.panel", action="focus", id="level_settings")
    lst = cmd("ui.list")
    ls = next((p for p in lst.get("panels", []) if p.get("id") == "level_settings"), {})
    check(f.get("ok") and ls.get("visible"), "E focus raises Level Settings in its dock node", short(ls))
    t1 = cmd("ui.panel", action="toggle", id="output_log")
    t2 = cmd("ui.panel", action="toggle", id="output_log")
    check(t1.get("open") != t2.get("open"), "E toggle flips the Output Log both ways", short(t1) + " / " + short(t2))
    v = cmd("ui.panel", action="close", id="viewport")
    check(not v.get("ok"), "E the viewport does not close", short(v))
    cmd("ui.panel", action="focus", id="details")
    s1 = cmd("ui.scroll", id="details", to="bottom")
    cmd("ui.list")
    if s1.get("ok") and (s1.get("scroll_max_y") or 0) > 0:
        s2 = cmd("ui.scroll", id="details", to="top")
        cmd("ui.list")
        s3 = cmd("ui.scroll", id="details", to="top")
        check(s3.get("scroll_y", 1) == 0, "E ui.scroll takes Details' grid back to the top (pt)", short(s2) + " / " + short(s3))
    else:
        print("NOTE Details does not scroll here (%s): the scroll round trip is not checked" % short(s1), flush=True)
    cmd("ui.tab", action="open", kind="characters")
    o = cmd("ui.panel", action="open", id="details")
    check(o.get("active_tab") == "level", "E opening a panel from another tab brings Level up", short(o))
    cmd("ui.tab", action="close", doc="characters")
    splits = (cmd("ui.split").get("splits") or {})
    check(any(k.startswith("level.") for k in splits), "E ui.split lists the layout store's values", json.dumps(splits)[:300])
    moved = cmd("ui.split", key="qa.shell_split", value=123.5)
    back = cmd("ui.split", key="qa.shell_split")
    listed = (cmd("ui.split").get("splits") or {}).get("qa.shell_split")
    check(moved.get("ok") and back.get("value") == 123.5 and back.get("unit") == "pt" and listed == 123.5,
          "E ui.split sets a value, reads it back and lists it", short(moved) + " / " + short(back))
    none = cmd("ui.split", key="level.no_such_split")
    check(not none.get("ok"), "E ui.split refuses a split that does not exist", short(none))
    t = cmd("ui.toast", text="QA note from test_ui_shell")
    lst = cmd("ui.list")
    check("QA note from test_ui_shell" in (t.get("toasts") or []) and "QA note from test_ui_shell" in (lst.get("toasts") or []),
          "E ui.toast raises a toast that ui.list names", short(t) + " / " + short({"toasts": lst.get("toasts")}))
    # a note may carry one button that runs an action (the updater's Details and What's New)
    b = cmd("ui.toast", text="QA note with a button", action="help.update", label="Details")
    notes = cmd("ui.list").get("toast_notes") or []
    check(b.get("ok") and {"text": "QA note with a button", "button": "Details", "action": "help.update"} in notes,
          "E a toast carries a button that names the action it runs", json.dumps(notes)[:300])
    bad = cmd("ui.toast", text="QA note", action="no.such_action")
    check(not bad.get("ok"), "E a toast's button must run an action that exists", short(bad))


def phase_f(project, shots, ed, before_scale, contract) -> None:
    def centred(tag: str) -> None:
        lst = cmd("ui.list")
        dlg = next((d for d in lst.get("dialogs", []) if d.get("id") == "import_base_scene"), None)
        work = lst.get("work_area") or [0, 0, 0, 0]
        if not check(dlg is not None, "F Import Base Scene is up (%s)" % tag, short(lst)):
            return
        r = dlg["rect"]
        centre = near(r[0] + r[2] / 2.0, work[0] + work[2] / 2.0, 2) and near(r[1] + r[3] / 2.0, work[1] + work[3] / 2.0, 2)
        inside = (r[0] >= work[0] - 1 and r[1] >= work[1] - 1 and r[0] + r[2] <= work[0] + work[2] + 1 and
                  r[1] + r[3] <= work[1] + work[3] + 1)
        check(centre and inside, "F it is centred and inside the work area (%s)" % tag, json.dumps({"dialog": r, "work": work}))

    od = cmd("ui.dialog", action="open", id="import_base_scene")
    if not check(od.get("ok"), "F Import Base Scene opens over ui.dialog", short(od)):
        return
    centred("wide")
    resize(*COMPACT)
    centred("compact")
    resize(*WIDE)
    cd = cmd("ui.dialog", action="close", id="import_base_scene")
    lst = cmd("ui.list")
    check(cd.get("ok") and not any(d.get("id") == "import_base_scene" for d in lst.get("dialogs", [])),
          "F it closes over ui.dialog", short(lst))
    cmd("ui.dialog", action="open", id="import_base_scene")
    cmd("ui.list")
    top = cmd("ui.dialog", action="close")
    lst = cmd("ui.list")
    check(top.get("ok") and top.get("closing") == "import_base_scene"
          and not any(d.get("id") == "import_base_scene" for d in lst.get("dialogs", [])),
          "F ui.dialog close without an id closes the dialog on top", short(top))
    # a dialog of the project goes down with it: the Start page shows no Import Base Scene
    cmd("ui.dialog", action="open", id="import_base_scene")
    cmd("ui.list")
    closed = cmd("project.close", discard=True, timeout=120)
    pic = cmd("shot.window", timeout=60)
    check(closed.get("ok") and pic.get("ok") and pic.get("modal") is None,
          "F Import Base Scene goes down with the project it imports into", short(pic))
    page = cmd("ui.list")
    check(page.get("start_page") is True and page.get("status_centre") is None,
          "F the Start page's status bar has no centre", short(page))
    labels = {c.get("label") for c in cmd("ui.audit").get("controls", [])}
    check("Content Drawer" not in labels and "Output Log" not in labels,
          "F the Start page's status bar has no drawer buttons", json.dumps(sorted(x for x in labels if x))[:300])
    back = cmd("project.open", path=project, timeout=300)
    check(back.get("ok"), "F the project opens again", short(back))


def phase_g(project, shots, ed, before_scale, contract) -> None:
    resize(*NARROW)
    fr = cmd("ui.frame")
    lst = cmd("ui.list")
    check(fr.get("folded") is True and lst.get("size_class") == "compact", "G under 1100 pt the menus fold, the class is Compact",
          short(fr))
    p = cmd("ui.popup", show="menu:Menu")
    lst = cmd("ui.list")
    check(p.get("ok") and lst.get("popups"), "G the folded Menu button puts its menus up", short(lst))
    cmd("ui.popup", show="none")
    resize(*COMPACT)
    lst = cmd("ui.list")
    check(lst.get("size_class") == "compact", "G 1280 x 720 pt is the Compact class")
    level_rects(lst, "compact", shots)
    resize(*WIDE)
    level_rects(cmd("ui.list"), "wide", shots)
    fr = cmd("ui.frame")
    check(fr.get("folded") is False, "G a wide window shows the menus again", short(fr))
    refused = cmd("ui.popup", show="menu:Menu")
    check(not refused.get("ok"), "G menu:Menu is refused while the menus are not folded", short(refused))
    for scale in (1.0, 1.5):
        s = cmd("ui.scale", value=scale)
        fr = cmd("ui.frame")
        check(s.get("ok") and near(fr.get("caption_h", 0), 32, 0.6) and near(float(fr.get("scale") or 0), scale, .01),
              "G ui.scale %.1f: the caption stays 32 pt, drawn at that scale" % scale, short(fr))
    cmd("ui.scale", value=before_scale)


def level_rects(lst: dict, cls: str, shots) -> None:
    """The Level tab's dock at a size class: Place left (260 Wide, 240 Compact), the Outliner over Details on the
    right (340, 300), the browser and the log under the viewport in Wide and in drawers in Compact; the viewport takes
    the rest."""
    cmd("ui.tab", action="activate", doc="level")
    lst = cmd("ui.list")
    panels = {p.get("id"): p for p in lst.get("panels") or []}
    rect = lambda pid: list((panels.get(pid) or {}).get("rect") or [0, 0, 0, 0])  # pt
    left, right = (260, 340) if cls == "wide" else (240, 300)
    place, outliner, details, view = rect("place"), rect("world_outliner"), rect("details"), rect("viewport")
    gap = 2.0 + 0.6
    check(near(place[2], left, gap) and (panels.get("place") or {}).get("visible"),
          "G %s: Place is the left panel, %d pt" % (cls, left), json.dumps(place))
    check(near(outliner[2], right, gap) and near(details[2], right, gap) and outliner[1] < details[1],
          "G %s: the Outliner over Details on the right, %d pt" % (cls, right), json.dumps([outliner, details]))
    check(view[0] >= place[0] + place[2] - 0.6 and view[0] + view[2] <= outliner[0] + 0.6 and view[2] >= 360,
          "G %s: the viewport between them takes the rest" % cls, json.dumps(view))
    cb = panels.get("content_browser") or {}
    if cls == "wide":
        strip = rect("content_browser")
        check(cb.get("docked") and cb.get("visible") and near(strip[3], 240, gap) and strip[0] >= view[0] - 0.6,
              "G wide: the Content Browser under the viewport only, 240 pt", json.dumps(strip))
    else:
        check(not cb.get("visible"), "G compact: the Content Browser lives in its drawer", short(cb))
    shot(shots, "level_" + cls)


def phase_h(project, shots, ed, before_scale, contract) -> None:
    cmd("ui.tab", action="activate", doc="level")
    for menu in ("File", "Edit", "Mod", "Window"):
        p = cmd("ui.popup", show="menu:" + menu)
        lst = cmd("ui.list")
        pops = lst.get("popups") or []
        check(p.get("ok") and pops, "H the %s menu comes up over ui.popup" % menu, short(lst))
        xs = (pops[0].get("label_x") if pops else None) or []
        check(len(xs) >= 2 and max(xs) - min(xs) <= 0.6, "H the %s menu has one label column" % menu, json.dumps(xs))
        if menu == "File":
            shot(shots, "frame_menu_file")
        cmd("ui.popup", show="none")
    cmd("ui.popup", show="menu:Window")
    cmd("ui.list")
    cmd("ui.popup", show="menu:File")
    lst = cmd("ui.list")
    check(len(lst.get("popups") or []) == 1, "H a popup put up takes the one before it down", short(lst))
    cmd("ui.popup", show="none")
    q = cmd("ui.palette", action="query", query="log", limit=10)
    rows = q.get("results") or []
    check(any(r.get("label") in ("Output Log", "Log Drawer") for r in rows[:2]),
          "H 'log' ranks a row that shows the log in the first two", json.dumps(rows[:3])[:400])
    check(not any(str(r.get("label", "")).startswith("COMMAND_") for r in rows), "H no row's label is a COMMAND_* id",
          json.dumps(rows[:5])[:400])
    check(all(r.get("category") for r in rows if r.get("kind") == "action"), "H every action row names its category",
          json.dumps(rows[:5])[:400])
    rel = make_quest()
    if rel:
        cmd("ui.tab", action="open", kind="quest", path=rel)
        name = os.path.splitext(os.path.basename(rel))[0]
        tq = cmd("ui.palette", action="query", query=name, limit=20)
        tab_rows = [r for r in tq.get("results") or [] if r.get("kind") == "tab"]
        check(bool(tab_rows) and all(":" not in str(r.get("detail", "")) and not str(r.get("detail", "")).startswith("\\")
                                     for r in tab_rows),
              "H a tab's row names its file by the project path", json.dumps(tab_rows[:2])[:300])
        cmd("ui.tab", action="close", doc="quest:" + rel.replace("\\", "/"), discard=True)
    cmd("ui.tab", action="activate", doc="level")
    cmd("ui.palette", action="open")
    pic = shot(shots, "frame_palette")
    check((pic.get("modal") or {}).get("id") == contract.get("palette"), "H the open palette is the modal on top", short(pic))
    cmd("ui.palette", action="close")


def phase_i(project, shots, ed, before_scale, contract) -> None:
    cmd("ui.tab", action="activate", doc="level")
    cmd("ui.drawer", action="close")
    ran = cmd("ui.palette", action="run", id="window.log_drawer")
    lst = cmd("ui.list")
    check(ran.get("ok") and lst.get("drawer") == "log", "I a palette row runs by its id: window.log_drawer opens the Log drawer",
          short(ran) + " / drawer " + str(lst.get("drawer")))
    cmd("ui.drawer", action="close")
    first = (cmd("ui.palette", action="query", query="", limit=5).get("results") or [{}])[0]
    check(first.get("id") == "window.log_drawer" and first.get("recent") is True,
          "I the action run last leads the empty palette, as recent", json.dumps(first))
    keys = cmd("app.keys")
    rows = {r.get("id"): r for r in keys.get("keys", [])}
    groups = {r.get("group") for r in rows.values()}
    check({"File", "Edit", "Window", "Graph", "Mod"} <= groups, "I app.keys lists every action under its group",
          json.dumps(sorted(g for g in groups if g)))
    check(any(i.startswith("graph.") and r.get("group") == "Graph" and r.get("default") for i, r in rows.items()),
          "I the graph's keys are actions of the group Graph with their default keys",
          json.dumps([i for i in rows if i.startswith("graph.")])[:300])
    row = rows.get("window.log_drawer") or {}
    old = (row.get("keys") or [""])[0]
    combo = "Ctrl+Shift+Alt+F11"
    if not check(bool(old), "I the Log drawer has a key to rebind", json.dumps(row)):
        return
    before = cmd("app.keys", action="resolve", keys=combo)
    bound = cmd("app.keys", action="bind", id="window.log_drawer", keys=combo)
    now = cmd("app.keys", action="resolve", keys=combo)
    gone = cmd("app.keys", action="resolve", keys=old)
    check(before.get("runs") is None and bound.get("ok") and now.get("runs") == "window.log_drawer",
          "I a binding made in app.keys is what the key pass finds", short(before) + " / " + short(now))
    check(gone.get("runs") != "window.log_drawer", "I the old key no longer runs it", short(gone))
    pal = cmd("ui.palette", action="query", query="Log Drawer", limit=5).get("results") or []
    shown = next((r for r in pal if r.get("id") == "window.log_drawer"), {})
    check(shown.get("shortcut") == combo, "I the palette shows the new key", json.dumps(shown))
    cmd("app.keys", action="reset", id="window.log_drawer")
    again = cmd("app.keys", action="resolve", keys=old)
    check(again.get("runs") == "window.log_drawer", "I reset gives the action its default key back", short(again))
    # Unreal's keys, on the Level tab with nothing selected and no panel holding the keys
    cmd("scene.select", none=True)
    want = {"Ctrl+B": "edit.show_in_browser", "Ctrl+Shift+S": "file.save_all", "Alt+P": "level.play",
            "Shift+1": "viewport.mode_select", "Ctrl+`": "viewport.toggle_space", "`": "window.log_drawer"}
    got = {k: cmd("app.keys", action="resolve", keys=k).get("runs") for k in want}
    check(got == want, "I the default keys are Unreal's", json.dumps(got))
    long_work = [i for i in ("level.build", "mod.build_into_game", "mod.build") if (rows.get(i) or {}).get("default")]
    check(not long_work, "I no long work has a default key: one stray chord never starts a build", json.dumps(long_work))
    tab_path = next((t.get("path") for t in cmd("state").get("tabs", []) if t.get("id") == "level"), "") or ""
    ran = cmd("ui.palette", action="run", id="edit.show_in_browser")
    if not tab_path:
        check(not ran.get("ok") and ran.get("code") == "refused",
              "I with no file on the Level tab, Ctrl+B's action says why it is off", short(ran))
        return
    qa.until(lambda: cmd("content.view").get("selection"), 10)
    view = cmd("content.view")
    leaf = os.path.basename(tab_path).lower()
    picked = [e for e in view.get("selection") or [] if e.lower().endswith(leaf)]
    check(ran.get("ok") and leaf and picked and view.get("visible"),
          "I Ctrl+B's action shows the Level tab's file selected in the Content Browser",
          short(ran) + " / " + json.dumps({"tab": tab_path, "selection": view.get("selection"), "visible": view.get("visible")}))


# the tab kinds a tour opens without a file of their own, and how
SINGLETONS = ("characters", "economy", "mod_settings", "world_rules", "screens", "releases", "preferences",
              "light_animations", "texture_library", "sound_library")


def frame_parts() -> dict:
    fr = cmd("ui.frame")
    return {
        "band": fr.get("band"),
        "tab_row": fr.get("tab_row"),
        "toolbar": fr.get("toolbar"),
        "project_toolbar": fr.get("project_toolbar"),
        "status_bar": fr.get("status_bar"),
        "tabs": {t.get("id"): t.get("rect") for t in fr.get("tabs") or []},
        "toolbar_status": fr.get("toolbar_status"),
        "active": fr.get("active_tab") or cmd("ui.list").get("active_tab"),
    }


def phase_j(project, shots, ed, before_scale, contract) -> None:
    resize(*WIDE)
    cmd("ui.tab", action="activate", doc="level")
    cmd("perf.stalls", clear=True)
    opened = ["level"]
    for kind in SINGLETONS:
        o = cmd("ui.tab", action="open", kind=kind)
        if check(o.get("ok"), "J the %s tab opens" % kind, short(o)):
            opened.append((o.get("tab") or {}).get("id", kind))
        if kind == "mod_settings":
            shot(shots, "frame_mod_settings")
            st = frame_parts().get("toolbar_status") or ""
            check("mod.ltx" in st, "J Mod Settings' toolbar row holds its status", json.dumps(st))
        if kind == "preferences":
            shot(shots, "frame_preferences")
            cmd("app.prefs", action="show", page="keyboard")
            shot(shots, "frame_keyboard")
    rel = make_quest()
    if rel:
        o = cmd("ui.tab", action="open", kind="quest", path=rel)
        if check(o.get("ok"), "J a quest tab opens", short(o)):
            opened.append((o.get("tab") or {}).get("id"))
            cmd("graph.view", doc="quest:" + rel.replace("\\", "/"), frame="all")
            shot(shots, "frame_graph")
    beh = cmd("file.create", kind="behaviour")
    brel = (beh.get("path") or "").replace("/", "\\")
    if brel:
        o = cmd("ui.tab", action="open", kind="beh", path=brel)
        if check(o.get("ok"), "J a behaviour graph tab opens", short(o)):
            opened.append((o.get("tab") or {}).get("id"))
    pda = cmd("pda.open")
    lst = cmd("ui.list")
    if check(pda.get("ok") and lst.get("active_tab", "").startswith("pda"), "J the PDA tab opens", short(pda)):
        opened.append(lst.get("active_tab"))
    for kind, path in (("texture", "ui\\ui_icon_equipment"), ("model", "dynamics\\weapons\\wpn_ak74\\wpn_ak74")):
        o = cmd("ui.tab", action="open", kind=kind, path=path)
        if check(o.get("ok"), "J the %s tab opens" % kind, short(o)):
            opened.append((o.get("tab") or {}).get("id"))
    # every tab is open: from here on no tab is added, so nothing of the frame may move between them
    cmd("ui.tab", action="activate", doc="level")
    base = frame_parts()
    moved = []
    for doc in opened[1:] + ["level"]:
        cmd("ui.tab", action="activate", doc=doc)
        now = frame_parts()
        for part in ("band", "tab_row", "toolbar", "status_bar"):
            if not same_rect(base[part], now[part]):
                moved.append("%s: %s %s -> %s" % (doc, part, base[part], now[part]))
        # Preferences has nothing to build: the project's group is not on its row
        if not doc.startswith("preferences") and not same_rect(base["project_toolbar"], now["project_toolbar"]):
            moved.append("%s: project group %s -> %s" % (doc, base["project_toolbar"], now["project_toolbar"]))
        for t, r in base["tabs"].items():
            if not same_rect(r, now["tabs"].get(t)):
                moved.append("%s: tab %s %s -> %s" % (doc, t, r, now["tabs"].get(t)))
    check(not moved, "J switching between every tab kind moves no part of the frame", "; ".join(moved)[:600])
    qa.stalls("J after the tour no UI pass held the UI thread over 100 ms", clear=True)
    tasks = cmd("ui.tasks", show=True)
    finished = tasks.get("finished") or []
    check(tasks.get("ok") and 0 < len(finished) <= 5 and all(f.get("result") for f in finished),
          "J the Task Center keeps the last jobs that ended (five at most), each with how it ended", short(tasks))
    pops = [p.get("id") for p in cmd("ui.list").get("popups") or []]
    check("tasks" in pops, "J opened by the author, the Task Center stays up with only finished jobs", json.dumps(pops))
    shot(shots, "frame_task_center")
    cmd("ui.tasks", show=False)
    for doc in opened[1:]:
        if doc:
            cmd("ui.tab", action="close", doc=doc, discard=True)


def phase_k(project, shots, ed, before_scale, contract) -> None:
    """The control of `ui.audit`'s `loading`: while an editor reads the project it opens, `ui.audit` names what its
    surfaces wait for; once it is all read it names nothing. The last phase: it ends this suite's editor and starts a
    fresh one on the same copy, and looks at every frame from the first answer on."""
    ed.close()
    fresh = qa.Editor(project, ready="ping", stalls=False)
    fresh.start()
    try:
        seen: list = []
        unknown: list = []
        zero = []
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            loading = fresh.cmd("ui.audit").get("loading") or []
            if loading:
                seen = seen or loading
                # the Place panel's categories are read on the same frames: a list not read has no count
                cats = ((fresh.cmd("level.view").get("place") or {}).get("categories") or []) if len(seen) else []
                unknown = unknown or [c for c in cats if c.get("loading")]
                zero = zero or [c for c in cats if c.get("loading") and c.get("count") == 0]
            elif seen:
                break
            elif fresh.cmd("state").get("project"):
                # the project is open and nothing waited on the frames looked at: no more to see
                break
        print("NOTE K saw %d busy mark(s), first: %s; %d Place categor(ies) without a count" % (
            len(seen), qa.short(seen[:1], 160), len(unknown)), flush=True)
        check(bool(seen), "K ui.audit answers a non-empty loading while the editor reads the project it opens", qa.short(seen))
        check(not zero, "K a category whose list is being read answers no count, never 0", qa.short(zero))
        fresh.cmd("job.wait", timeout=120)
        settled = qa.until(lambda: not fresh.cmd("ui.audit").get("loading"), 30)
        check(settled, "K ui.audit answers an empty loading once everything is read", qa.short(fresh.cmd("ui.audit").get("loading")))
    finally:
        fresh.close()


PHASE = {"A": phase_a, "B": phase_b, "C": phase_c, "D": phase_d, "E": phase_e, "F": phase_f, "G": phase_g, "H": phase_h,
         "I": phase_i, "J": phase_j, "K": phase_k}


def main() -> int:
    return qa.suite("UI SHELL", run, doc=__doc__, ignore=(".git", "releases", "temp"), phases=PHASES)


if __name__ == "__main__":
    sys.exit(main())
