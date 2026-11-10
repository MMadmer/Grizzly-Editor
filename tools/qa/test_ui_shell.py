#!/usr/bin/env python3
"""The editor's frame over MCP (docs/UI_DESIGN.md, sections 5 and 7), on a THROW-AWAY COPY of a project.

What it proves:
  A  the frame reports itself: state and ui_list name the Level tab, the panels, the drawers; ui_contract has the
     constants QA pins (panels, drawers, dialogs, menus, the palette's id)
  B  the caption: 32 pt tall, the window buttons 3 x 46 pt at its right end, the tabs inside the interactive range -
     and Windows agrees: WM_NCHITTEST answers HTCLIENT on a tab and on a window button, HTCAPTION on the drag band
  C  the major tabs: a tool tab opens, Level stays first and pinned (it refuses to close), next/prev cycle, a clean tab
     closes; a quest made dirty refuses a plain close ('unsaved') and closes with discard=true
  D  the drawers: over a document the Content and Log drawers open and close; ui_list names the one that is up
  E  the Level panels: close, open, focus and toggle round trips, and Level comes up with an opened panel; ui_scroll
     moves a panel whose body is a child of it (Details' grid) to its bottom and back
  F  the dialogs: Import Base Scene opens and closes over ui_dialog, and stays centred in the work area when the window
     is resized both ways
  G  the size: under 1100 pt the menus fold into one Menu button (titlebar_info folded, ui_popup menu:Menu), the size
     class follows the window (wide / compact), and ui_scale rescales the caption (32 pt at 1.0 and 1.5)
  H  menus and the palette: ui_popup puts a caption menu up and takes it down, one popup at a time, and refuses the
     tasks popover while no job runs; the palette ranks the Output Log panel first for 'log' and shows no COMMAND_* id
     as a row's label; opened, it is the modal on top

    python tools/qa/test_ui_shell.py --project <project> [--shots <folder>]

The ini files the editor writes when it closes (_projects.ini, level.ini, level_imgui.ini) come back byte for byte.
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import ctypes.wintypes as wt
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
KEEP = [os.path.join(REPO, n) for n in ("_projects.ini", "level.ini", "level_imgui.ini")]
WIDE = (1500, 800)       # pt: the Wide size class (W >= 1400, H >= 640)
COMPACT = (1280, 720)    # pt: Compact
NARROW = (1000, 700)     # pt: under 1100 the caption's menus fold

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


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


def short(answer: dict) -> str:
    return json.dumps({k: v for k, v in answer.items() if k != "png_base64"})[:400]


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
        # never killed: a killed editor leaves project.ltx behind half written
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
    pic = mcp("screenshot_editor")
    if folder and pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)
    return pic


def resize(w: int, h: int) -> dict:
    r = mcp("window_control", action="resize", width=w, height=h)
    # the swap chain follows the window on the next frames: let them be drawn before anyone looks
    for _ in range(2):
        mcp("ui_list")
    return r


# ---- Win32: what the window's hit test answers where the caption drew something ------------------------------------
user32 = ctypes.windll.user32
HTCLIENT, HTCAPTION = 1, 2
# The editor is Per-Monitor V2 and speaks physical px; an unaware process gets its coordinates scaled by the monitor's
# DPI, and a point put through ClientToScreen lands on another part of the caption.
if not user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4)):
    ctypes.windll.shcore.SetProcessDpiAwareness(2)


def editor_hwnd() -> int | None:
    found: list[int] = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def each(hwnd, _):
        cls = ctypes.create_unicode_buffer(64)
        user32.GetClassNameW(hwnd, cls, 64)
        if cls.value == "Grizzly Editor":
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


def run(project: str, shots: str | None) -> None:
    ed = Editor()
    try:
        ed.start(project)
        before_scale = float(mcp("ui_list").get("ui_scale") or 1.0)

        # ------------------------------------------------------------ A: the frame reports itself
        st = mcp("state")
        check(st.get("active_tab") == "level" and any(t.get("id") == "level" for t in st.get("tabs", [])),
              "A state names the Level tab as the active one", short(st))
        contract = mcp("ui_contract")
        panel_ids = set((contract.get("panels") or {}).keys())
        check({"tools", "world_outliner", "details", "level_settings", "content_browser", "output_log", "viewport"} <= panel_ids,
              "A ui_contract lists the seven Level panels", short(contract))
        check(set((contract.get("drawers") or {}).keys()) == {"content", "log"}, "A ui_contract lists the two drawers", short(contract))
        dialogs = contract.get("dialogs") or {}
        check("import_base_scene" in dialogs and all("###dlg." in w for i, w in dialogs.items() if i not in ("mod_settings", "releases")),
              "A the contract's dialogs are Title###dlg.<name>", json.dumps(dialogs)[:400])
        check(bool(contract.get("palette")) and "Menu" in (contract.get("menus") or []),
              "A ui_contract names the palette's id and the folded Menu", short(contract))
        resize(*WIDE)
        lst = mcp("ui_list")
        check(lst.get("size_class") == "wide" and lst.get("start_page") is False, "A a 1500 x 800 pt window is the Wide class",
              short(lst))
        shot(shots, "shell_wide")

        # ------------------------------------------------------------ B: the caption and its hit test
        tb = mcp("titlebar_info")
        dpi = float(tb.get("dpi") or 1.0)
        band = tb.get("band") or [0, 0, 0, 0]
        check(near(tb.get("caption_h", 0), 32 * dpi, 1.5), "B the caption is 32 pt tall", short(tb))
        check(near(band[2] - tb.get("buttons_x", 0), 3 * 46 * dpi, 2), "B the window buttons are 3 x 46 pt at its right end",
              short(tb))
        tabs = tb.get("tabs") or []
        inter = tb.get("interactive") or [[0, 0], [0, 0]]
        check(bool(tabs) and all(inter[0][0] <= t["rect"][0] and t["rect"][2] <= inter[0][1] for t in tabs),
              "B every tab lies in the interactive range of the caption", short(tb))
        hwnd = editor_hwnd()
        if check(bool(hwnd), "B the editor's window is found"):
            cy = (band[1] + band[3]) / 2.0
            level = next((t for t in tabs if t.get("id") == "level"), tabs[0] if tabs else None)
            if level:
                r = level["rect"]
                got = hit_test(hwnd, (r[0] + r[2]) / 2.0, cy)
                check(got == HTCLIENT, "B the Level tab takes the mouse (HTCLIENT)", "hit %d" % got)
            drag_x = (inter[0][1] + tb.get("buttons_x", 0)) / 2.0
            got = hit_test(hwnd, drag_x, cy)
            check(got == HTCAPTION, "B the band between the tabs and the buttons drags the window (HTCAPTION)", "hit %d" % got)
            got = hit_test(hwnd, band[2] - 23 * dpi, cy)
            check(got == HTCLIENT, "B the close button takes the mouse (HTCLIENT)", "hit %d" % got)

        # ------------------------------------------------------------ C: the major tabs
        o = mcp("ui_tab", action="open", kind="characters")
        check(o.get("ok") and (o.get("tab") or {}).get("id") == "characters", "C a tool tab opens and comes up at once", short(o))
        lt = mcp("ui_tab", action="list")
        ids = [t.get("id") for t in lt.get("tabs", [])]
        check(ids[:1] == ["level"] and "characters" in ids, "C Level is the first tab, the tool tab follows", json.dumps(ids))
        nx = mcp("ui_tab", action="next")
        check(nx.get("ok") and (nx.get("tab") or {}).get("id") != "characters", "C next cycles away from the active tab", short(nx))
        pv = mcp("ui_tab", action="prev")
        check((pv.get("tab") or {}).get("id") == "characters", "C prev comes back", short(pv))
        lv = mcp("ui_tab", action="close", id="level")
        check(not lv.get("ok"), "C the Level tab refuses to close", short(lv))
        cl = mcp("ui_tab", action="close", id="characters")
        check(cl.get("ok") and "characters" not in [t.get("id") for t in cl.get("tabs", [])], "C a clean tab closes", short(cl))

        made = mcp("content_create", kind="quest")
        rel = (made.get("made") or "").replace("/", "\\")
        if check(made.get("ok") and rel, "C a quest is made for the dirty-close check", short(made)):
            op = mcp("ui_tab", action="open", kind="quest", path=rel)
            quest = (op.get("tab") or {}).get("id", "")
            check(op.get("ok") and quest.startswith("quest:"), "C the quest opens as a tab of its own", short(op))
            with open(os.path.join(project, rel), "rb") as f:
                on_disk = f.read()
            mcp("quest_edit", path=rel, op="select_all")
            mcp("quest_edit", path=rel, op="duplicate")
            info = mcp("ui_tab", action="list")
            dirty = any(t.get("id") == quest and t.get("dirty") for t in info.get("tabs", []))
            if check(dirty, "C an edit leaves the quest tab dirty", short(info)):
                refused = mcp("ui_tab", action="close", id=quest)
                check(not refused.get("ok") and refused.get("error") == "unsaved" and refused.get("dirty"),
                      "C a dirty tab refuses a plain close and names what is unsaved", short(refused))
            gone = mcp("ui_tab", action="close", id=quest, discard=True)
            check(gone.get("ok"), "C discard=true closes it", short(gone))
            with open(os.path.join(project, rel), "rb") as f:
                check(f.read() == on_disk, "C the discarded edit never reached the file")

        # ------------------------------------------------------------ D: the drawers over a document
        mcp("ui_tab", action="open", kind="characters")
        for which in ("log", "content"):
            d = mcp("ui_drawer", action="open", id=which)
            lst = mcp("ui_list")
            check(d.get("ok") and lst.get("drawer") == which, "D the %s drawer opens over a document" % which, short(lst))
            if which == "log":
                shot(shots, "shell_log_drawer")
            c = mcp("ui_drawer", action="close", id=which)
            lst = mcp("ui_list")
            check(c.get("ok") and lst.get("drawer") == "none", "D the %s drawer closes" % which, short(lst))
        # a drawer opened over one tab does not cover the next one's canvas
        mcp("ui_drawer", action="open", id="content")
        mcp("ui_tab", action="activate", id="level")
        lst = mcp("ui_list")
        check(lst.get("drawer") == "none", "D another tab coming up closes the drawer", short(lst))
        mcp("ui_tab", action="activate", id="characters")
        mcp("ui_tab", action="close", id="characters")

        # ------------------------------------------------------------ E: the Level panels
        mcp("ui_tab", action="activate", id="level")
        for panel in ("world_outliner", "details", "level_settings", "tools"):
            c = mcp("ui_panel", action="close", id=panel)
            check(c.get("ok") and c.get("open") is False, "E %s closes" % panel, short(c))
            o = mcp("ui_panel", action="open", id=panel)
            check(o.get("ok") and o.get("open") is True and o.get("active_tab") == "level", "E %s opens on Level" % panel, short(o))
        f = mcp("ui_panel", action="focus", id="level_settings")
        lst = mcp("ui_list")
        ls = next((p for p in lst.get("panels", []) if p.get("id") == "level_settings"), {})
        check(f.get("ok") and ls.get("visible"), "E focus raises Level Settings in its dock node", short(ls))
        t1 = mcp("ui_panel", action="toggle", id="output_log")
        t2 = mcp("ui_panel", action="toggle", id="output_log")
        check(t1.get("open") != t2.get("open"), "E toggle flips the Output Log both ways", short(t1) + " / " + short(t2))
        v = mcp("ui_panel", action="close", id="viewport")
        check(not v.get("ok"), "E the viewport does not close", short(v))
        # Details scrolls in its grid, a child of the panel: the bottom is reached, and the top again
        mcp("ui_panel", action="focus", id="details")
        s1 = mcp("ui_scroll", id="details", to="bottom")
        mcp("screenshot_editor")
        s2 = mcp("ui_scroll", id="details", to="top")
        if s1.get("ok") and (s1.get("scroll_max_y") or 0) > 0:
            check(s2.get("scroll_y", 0) >= s1["scroll_max_y"] - 2, "E ui_scroll takes Details' grid to its bottom",
                  short(s1) + " / " + short(s2))
            mcp("screenshot_editor")
            s3 = mcp("ui_scroll", id="details", to="top")
            check(s3.get("scroll_y", 1) == 0, "E ui_scroll takes Details' grid back to the top", short(s3))
        else:
            print("NOTE Details does not scroll here (%s): the scroll round trip is not checked" % short(s1), flush=True)
        mcp("ui_tab", action="open", kind="characters")
        o = mcp("ui_panel", action="open", id="details")
        check(o.get("active_tab") == "level", "E opening a panel from another tab brings Level up", short(o))
        mcp("ui_tab", action="close", id="characters")

        # ------------------------------------------------------------ F: a dialog, centred at both sizes
        def centred(tag: str) -> None:
            lst = mcp("ui_list")
            dlg = next((d for d in lst.get("dialogs", []) if d.get("id") == "import_base_scene"), None)
            work = lst.get("work_area") or [0, 0, 0, 0]
            if not check(dlg is not None, "F Import Base Scene is up (%s)" % tag, short(lst)):
                return
            r = dlg["rect"]
            # centred in the work area (between the toolbar and the status bar) within a few px, and wholly inside it
            centre = near(r[0] + r[2] / 2.0, work[0] + work[2] / 2.0, 3) and near(r[1] + r[3] / 2.0, work[1] + work[3] / 2.0, 3)
            inside = (r[0] >= work[0] - 1 and r[1] >= work[1] - 1 and r[0] + r[2] <= work[0] + work[2] + 1 and
                      r[1] + r[3] <= work[1] + work[3] + 1)
            check(centre and inside, "F it is centred and inside the work area (%s)" % tag, json.dumps({"dialog": r, "work": work}))

        od = mcp("ui_dialog", action="open", id="import_base_scene")
        if check(od.get("ok"), "F Import Base Scene opens over ui_dialog", short(od)):
            centred("wide")
            resize(*COMPACT)
            centred("compact")
            shot(shots, "shell_dialog_compact")
            resize(*WIDE)
            centred("wide again")
            cd = mcp("ui_dialog", action="close", id="import_base_scene")
            lst = mcp("ui_list")
            check(cd.get("ok") and not any(d.get("id") == "import_base_scene" for d in lst.get("dialogs", [])),
                  "F it closes over ui_dialog", short(lst))

        # ------------------------------------------------------------ G: folding, size classes, the scale
        resize(*NARROW)
        tb = mcp("titlebar_info")
        lst = mcp("ui_list")
        check(tb.get("folded") is True and lst.get("size_class") == "compact", "G under 1100 pt the menus fold, the class is Compact",
              short(tb))
        p = mcp("ui_popup", show="menu:Menu")
        lst = mcp("ui_list")
        check(p.get("ok") and lst.get("popups"), "G the folded Menu button puts its menus up", short(lst))
        shot(shots, "shell_folded_menu")
        mcp("ui_popup", show="none")
        resize(*COMPACT)
        check(mcp("ui_list").get("size_class") == "compact", "G 1280 x 720 pt is the Compact class")
        resize(*WIDE)
        tb = mcp("titlebar_info")
        check(tb.get("folded") is False, "G a wide window shows the menus again", short(tb))
        refused = mcp("ui_popup", show="menu:Menu")
        check(not refused.get("ok"), "G menu:Menu is refused while the menus are not folded", short(refused))
        for scale in (1.0, 1.5):
            s = mcp("ui_scale", value=scale)
            tb = mcp("titlebar_info")
            check(s.get("ok") and near(tb.get("caption_h", 0), 32 * scale, 1.5) and near(float(tb.get("dpi") or 0), scale, .01),
                  "G ui_scale %.1f: the caption is 32 pt at that scale" % scale, short(tb))
        mcp("ui_scale", value=before_scale)

        # ------------------------------------------------------------ H: menus and the palette
        for menu in ("File", "Edit", "Mod", "Window"):
            p = mcp("ui_popup", show="menu:" + menu)
            lst = mcp("ui_list")
            check(p.get("ok") and lst.get("popups"), "H the %s menu comes up over ui_popup" % menu, short(lst))
            if menu == "File":
                shot(shots, "shell_menu_file")
            mcp("ui_popup", show="none")
        # one popup looked at at a time: the next one takes the one before down
        mcp("ui_popup", show="menu:Window")
        mcp("screenshot_editor")
        mcp("ui_popup", show="menu:File")
        mcp("screenshot_editor")
        lst = mcp("ui_list")
        check(len(lst.get("popups") or []) == 1, "H a popup put up takes the one before it down", short(lst))
        mcp("ui_popup", show="none")
        pr = mcp("progress")
        if not pr.get("tasks") and not pr.get("background"):
            t = mcp("ui_popup", show="tasks")
            check(not t.get("ok"), "H with no job running, ui_popup tasks says there is nothing to show", short(t))
        q = mcp("command_palette", action="query", query="log", limit=10)
        rows = q.get("results") or []
        # the Log drawer and the Output Log panel both show the log: one of the two leads, before names that only hide it
        check(any(r.get("kind") == "panel" and r.get("label") == "Output Log" for r in rows[:2]),
              "H 'log' ranks the Output Log panel in the first two", json.dumps(rows[:3])[:400])
        check(not any(str(r.get("label", "")).startswith("COMMAND_") for r in rows), "H no row's label is a COMMAND_* id",
              json.dumps(rows[:5])[:400])
        mcp("command_palette", action="open")
        pic = shot(shots, "shell_palette")
        check((pic.get("modal") or {}).get("id") == contract.get("palette"), "H the open palette is the modal on top",
              short(pic))
        mcp("command_palette", action="close")
        ex = mcp("command_palette", action="execute", id="drawer:log")
        mcp("ui_tab", action="activate", id="level")
        check(ex.get("ok"), "H a palette row that is a drawer runs over MCP", short(ex))
        mcp("ui_drawer", action="close")
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for the screenshots")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_shellproj_")
    copy = os.path.join(work, "QaShell")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git", "releases", "temp"))
    print("running against a copy: " + copy, flush=True)
    # project.ltx names this machine's paths: the copy must name its own folder
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
    try:
        run(copy, args.shots)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        for path, data in saved.items():
            if data is None:
                if os.path.isfile(path):
                    os.remove(path)
            else:
                with open(path, "wb") as f:
                    f.write(data)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
    restored = all((open(p, "rb").read() if os.path.isfile(p) else None) == d for p, d in saved.items())
    check(restored, "the ini files are back byte for byte")
    ok = sum(1 for good, _ in results if good)
    print(f"UI SHELL: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
