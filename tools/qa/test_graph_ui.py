#!/usr/bin/env python3
"""The graph editor's layout as the redesign promises it (docs/UI_DESIGN.md section 6,
NQ_ARCHITECTURE par. 13.8-13.9).

Drives the editor over MCP on a THROW-AWAY COPY of a project, at 1707x912 pt with UI scale
1.5 and at 1280x720 pt with UI scale 1.0:

- Frame All leaves every title readable: title_px >= 12, the zoom never under frame_min_level; a
  graph taller than the view is framed AT the floor from its start, and quest_view `beyond` counts
  the nodes past each edge (none for a graph that fits); the canvas's note saying so is no warning
  the status bar counts;
- the Graph panel, the canvas and the Details panel are there, the canvas keeps 400 pt, and
  a narrow window folds the Graph panel to a rail that opens over the canvas;
- the widths and the two splits answer quest_view and read back, and the node pane scrolls the row
  of the action just selected into view;
- a scroll into a pane that is not on screen is an error, into one that is is not;
- the Problems drawer and the Shortcuts card come and go;
- the minimap stays away from a small graph and comes for a big one, and the toolbar's toggle of a
  small graph is disabled and says why instead of reading as on;
- the zoom menu keeps its percentage at every size (a value, not a label the toolbar may fold), and
  no other label of the toolbar folds or unfolds as the percentage changes;
- quest_layout reports content-sized boxes: a node with actions is taller than one without, a wait
  with a condition a line taller than one without;
- a view change over quest_view answers from the frames that show it, in one raw call;
- a behaviour graph's tab says "graph", never "quest".

The editor's level.ini, level_imgui.ini and _projects.ini come back byte for byte.

    python tools/qa/test_graph_ui.py --project <project> [--shots <dir>]
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
KEPT = qa_target.KEPT

BIG = "quests/qa_ui_big.nqasset"
SMALL = "npc/qa_ui_small.behasset"
SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))

results: list[tuple[bool, str]] = []
shots_dir: str | None = None


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 300.0, **extra) -> dict:
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


def settle() -> None:
    """The next frames are drawn: ui_audit is held until they are (GrizzlyMCP::IsShot)."""
    mcp("ui_audit")


def shot(name: str) -> None:
    if not shots_dir:
        return
    r = mcp("screenshot_editor")
    if r.get("ok") and r.get("png_base64"):
        with open(os.path.join(shots_dir, name + ".png"), "wb") as f:
            f.write(base64.b64decode(r["png_base64"]))


def view(path: str, **extra) -> dict:
    return mcp("quest_view", path=path, **extra)


def settled(path: str, **extra) -> dict:
    """quest_view, a frame, and the view as that frame left it."""
    first = view(path, **extra)
    if not first.get("ok"):
        return first
    settle()
    settle()
    return view(path)


def pane(v: dict, name: str):
    return (v.get("panes") or {}).get(name)


def toolbar_control(audit: dict, want) -> dict | None:
    """The first control of the shell's toolbar row whose label passes `want`."""
    for e in audit.get("controls", []):
        if "toolbar" in str(e.get("window", "")) and want(str(e.get("label", ""))):
            return e
    return None


class Editor:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self, project: str) -> None:
        exe = qa_target.exe()
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7
        qa_target.guard_project(project)
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project],
                                     cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("window_control", action="close")
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def big_quest() -> str:
    """Fourteen nodes in a chain: enough for the minimap, too tall for the canvas at 100%. Beside it two
    waits, one with a condition and one without: a condition is a line of its node."""
    nodes = ['        { id = "start", kind = "trigger.start", out = { next = "s1" } },']
    for i in range(1, 13):
        nxt = "s%d" % (i + 1) if i < 12 else "finish"
        acts = ""
        if i % 2:
            acts = ' on_enter = { { kind = "var.set", params = { name = "n", value = %d } } },' % i
        nodes.append('        { id = "s%d", kind = "flow.step",%s out = { next = "%s" } },' % (i, acts, nxt))
    nodes.append('        { id = "finish", kind = "flow.end", params = { status = "completed" } },')
    nodes.append('        { id = "w_cond", kind = "wait.when", cond = { { kind = "event.signal", params = { name = "qa_go" } } } },')
    nodes.append('        { id = "w_plain", kind = "wait.when" },')
    return ("return {\n    nq = 1,\n    id = \"qa_ui_big\",\n    title = \"QA big graph\",\n"
            "    vars = { n = 0 },\n    nodes = {\n" + "\n".join(nodes) + "\n    },\n}\n")


SMALL_TEXT = """return {
    beh   = 1,
    id    = "qa_ui_small",
    title = "QA small graph",
    roles = { walker = { title = "Walker" } },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "wait" } },
        { id = "wait", kind = "flow.step", out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""


def run(project: str) -> None:
    ed = Editor()
    try:
        ed.start(project)
        w1 = mcp("quest_write", path=BIG, lua=big_quest())
        check(w1.get("ok"), "the big quest is written", json.dumps(w1)[:300])
        w2 = mcp("quest_write", path=SMALL, lua=SMALL_TEXT)
        check(w2.get("ok"), "the small behaviour graph is written", json.dumps(w2)[:300])

        # ------------------------------------------------------------------ A: content-sized boxes
        lay = mcp("quest_layout", path=BIG, all=False)
        boxes = {p.get("id"): p for p in lay.get("positions", [])}
        check(all("w" in p and "h" in p for p in boxes.values()), "A quest_layout reports w and h of every node",
              json.dumps(lay)[:300])
        with_action, without = boxes.get("s1", {}), boxes.get("s2", {})
        check(with_action.get("w") == 240 and without.get("w") == 240, "A a node is 240 pt wide",
              json.dumps([with_action, without]))
        check(with_action.get("h", 0) > without.get("h", 0), "A a node with an action is taller than one without",
              json.dumps([with_action, without]))
        waits, plain = boxes.get("w_cond", {}), boxes.get("w_plain", {})
        check(waits.get("h", 0) == plain.get("h", 0) + 26, "A a wait lists the condition it waits on: one line more",
              json.dumps([waits, plain]))

        for width, height, scale in SIZES:
            tag = "%dx%d@%g" % (width, height, scale)
            mcp("ui_scale", value=scale)
            r = mcp("window_control", action="resize", width=width, height=height)
            check(r.get("ok"), tag + " the window takes the size", json.dumps(r)[:200])
            settle()

            # -------------------------------------------------------------- B: Frame All is readable
            warned = mcp("progress").get("warnings")
            v = settled(BIG, frame="all", graph_panel="show", details="show")
            check(v.get("ok"), tag + " quest_view answers", json.dumps(v)[:300])
            check(abs(v.get("ui_scale", 0) - scale) < 0.01, tag + " the reply names the UI scale", str(v.get("ui_scale")))
            check(v.get("title_px", 0) >= 12, tag + " titles are at least 12 px after Frame All", str(v.get("title_px")))
            check(v.get("zoom_level", -1) >= v.get("frame_min_level", 99), tag + " Frame All stays at or above its floor",
                  "%s < %s" % (v.get("zoom_level"), v.get("frame_min_level")))
            # fourteen steps do not fit at either size: the most of them that stays readable, from the start
            check(v.get("zoom_level") == v.get("frame_min_level"), tag + " a graph taller than the view is framed at the floor",
                  "%s != %s" % (v.get("zoom_level"), v.get("frame_min_level")))
            beyond = v.get("beyond") or {}
            check(beyond.get("above") == 0 and beyond.get("below", 0) > 0,
                  tag + " from its start: nothing above the view, the rest counted below it", json.dumps(beyond))
            now = mcp("progress").get("warnings")
            check(now == warned, tag + " the canvas's note about it is no warning the status bar counts", "%s -> %s" % (warned, now))
            shot("frame_all_" + tag)

            # -------------------------------------------------------------- C: the panes
            check(v.get("graph_panel") == "open", tag + " the Graph panel is open", str(v.get("graph_panel")))
            for name in ("graph", "variables", "canvas", "details"):
                check(pane(v, name) is not None, tag + " the %s pane is drawn" % name, json.dumps(v.get("panes")))
            canvas = pane(v, "canvas") or [0, 0, 0, 0]
            check(canvas[2] >= 400, tag + " the canvas keeps 400 pt", json.dumps(canvas))
            graph = pane(v, "graph") or [0, 0, 0, 0]
            details = pane(v, "details") or [0, 0, 0, 0]
            check(graph[0] < canvas[0] < details[0], tag + " Graph | canvas | Details, left to right",
                  json.dumps([graph, canvas, details]))

            v = settled(BIG, graph_width=300, details_width=320)
            check(abs(v.get("graph_width", 0) - 300) <= 1, tag + " graph_width sets the Graph panel", str(v.get("graph_width")))
            check(abs(v.get("details_width", 0) - 320) <= 1, tag + " details_width sets the Details panel", str(v.get("details_width")))
            settled(BIG, graph_width=280, details_width=360)

            # -------------------------------------------------------------- D: the splits
            # one raw call: the editor holds the answer until the frames it changed are drawn
            v = view(BIG, select="s1", slot="enter:0", action_split=0.3, vars_split=0.5)
            check(v.get("pending") is False, tag + " a view change answers from the frames that show it, in one call",
                  json.dumps(v)[:300])
            check(pane(v, "node") is not None and pane(v, "action") is not None, tag + " the node and its action have a pane each",
                  json.dumps(v.get("panes")))
            # at 0.3 of the column the Actions section is below the node pane's fold: the selected action's
            # row is brought into it (the audit lists a row only while it is drawn; its rect is px)
            settle()
            node = pane(v, "node") or [0, 0, 0, 0]
            row = next((e for e in mcp("ui_audit").get("controls", [])
                        if str(e.get("help", "")).startswith("Runs when the node is entered")), None)
            top, h = [c / scale for c in ((row or {}).get("rect") or [0, -9999, 0, 0])[1::2]]
            check(row is not None and node[1] - 1 <= top and top + h <= node[1] + node[3] + 1,
                  tag + " the selected action's row is scrolled into the node pane", json.dumps([row, node])[:300])
            again = view(BIG, keys="toggle")
            check(again.get("keys") is True, tag + " a toggle is applied once, not again by the read after its frames",
                  str(again.get("keys")))
            view(BIG, keys="hide")
            check(abs(v.get("action_split", 0) - 0.3) < 0.06, tag + " action_split reads back", str(v.get("action_split")))
            check(abs(v.get("vars_split", 0) - 0.5) < 0.06, tag + " vars_split reads back", str(v.get("vars_split")))
            shot("details_" + tag)
            settled(BIG, action_split=0.5, vars_split=0.38)

            # -------------------------------------------------------------- E: scrolls
            e1 = view(BIG, slot="none", scroll="action:top")
            check(not e1.get("ok"), tag + " no action open: scroll=action:top is an error", json.dumps(e1)[:200])
            e2 = view(BIG, graph_panel="hide", scroll="variables")
            check(not e2.get("ok"), tag + " no Graph panel: scroll=variables is an error", json.dumps(e2)[:200])
            ok1 = view(BIG, graph_panel="show", scroll="variables")
            check(ok1.get("ok"), tag + " with the Graph panel shown it is not", json.dumps(ok1)[:200])
            ok2 = view(BIG, select="s1", slot="enter:0", scroll="action:bottom")
            check(ok2.get("ok"), tag + " an open action scrolls", json.dumps(ok2)[:200])
            view(BIG, select="none", slot="none")

            # -------------------------------------------------------------- F: the drawer and the card
            v = settled(BIG, problems="show")
            check(v.get("drawer") == "problems" and pane(v, "drawer") is not None, tag + " problems=show opens the drawer",
                  json.dumps([v.get("drawer"), pane(v, "drawer")]))
            shot("drawer_" + tag)
            v = settled(BIG, problems="hide")
            check(v.get("drawer") == "none" and pane(v, "drawer") is None, tag + " problems=hide closes it",
                  json.dumps([v.get("drawer"), pane(v, "drawer")]))
            v = settled(BIG, keys="show")
            check(v.get("keys") is True, tag + " keys=show puts up the Shortcuts card", str(v.get("keys")))
            shot("keys_" + tag)
            settled(BIG, keys="hide")

            # -------------------------------------------------------------- G: the minimap
            settled(BIG, frame="all")
            big = mcp("quest_minimap", path=BIG)
            check(big.get("visible") is True and big.get("shown") is True and big.get("available") is True,
                  tag + " a graph of 16 nodes gets its minimap", json.dumps(big)[:200])
            settle()
            toggle = toolbar_control(mcp("ui_audit"), lambda label: label == "Minimap")
            check(toggle is not None and toggle.get("enabled") is True, tag + " its toolbar toggle is enabled",
                  json.dumps(toggle)[:200])
            # a zoom of two digits is led by a figure space (U+2007): every level is as wide as "100%"
            def is_zoom(label: str) -> bool:
                return label.strip().endswith("%") and label.strip()[:-1].isdigit()

            zoom = toolbar_control(mcp("ui_audit"), is_zoom)
            rect = (zoom or {}).get("rect") or [0, 0, 0, 1]
            check(zoom is not None and rect[2] > rect[3] * 1.5, tag + " the zoom menu shows its percentage, not a bare icon",
                  json.dumps(zoom)[:200])

            def toolbar_widths() -> list:
                """Every other control of the toolbar with its width: a folded label is a button's width."""
                settle()
                return sorted((str(e.get("label", "")), round((e.get("rect") or [0, 0, 0, 0])[2]))
                              for e in mcp("ui_audit").get("controls", [])
                              if "toolbar" in str(e.get("window", "")) and not is_zoom(str(e.get("label", ""))))

            settled(BIG, zoom_level=5)
            at75 = toolbar_widths()
            settled(BIG, zoom_level=8)
            at150 = toolbar_widths()
            check(at75 == at150, tag + " no toolbar label folds or unfolds between 75% and 150%",
                  json.dumps([x for x in at75 if x not in at150] + [x for x in at150 if x not in at75])[:300])
            settled(BIG, frame="all")
            v = settled(SMALL, frame="all")
            check(v.get("beyond") == {"above": 0, "below": 0, "left": 0, "right": 0},
                  tag + " a graph that fits leaves nothing beyond the view", json.dumps(v.get("beyond")))
            small = mcp("quest_minimap", path=SMALL)
            check(small.get("visible") is True and small.get("shown") is False and small.get("available") is False,
                  tag + " a graph of 3 nodes does not", json.dumps(small)[:200])
            settle()
            toggle = toolbar_control(mcp("ui_audit"), lambda label: label == "Minimap")
            check(toggle is not None and toggle.get("enabled") is False and "12" in str(toggle.get("why")),
                  tag + " and its toggle is disabled, saying why - not lit with no minimap", json.dumps(toggle)[:200])

            # -------------------------------------------------------------- H: a behaviour graph never says "quest"
            settle()
            audit = mcp("ui_audit")
            said = []
            for key in ("controls", "truncated"):
                for e in audit.get(key, []):
                    window = str(e.get("window", ""))
                    if "doc.host" not in window and "toolbar" not in window:
                        continue
                    text = " ".join(str(e.get(k, "")) for k in ("label", "help", "why", "shown")).lower()
                    if "quest" in text:
                        said.append(text[:120])
            check(not said, tag + " the behaviour graph's tab never says quest", json.dumps(said[:5]))
            shot("behaviour_" + tag)

        # ---------------------------------------------------------------------- I: the rail
        mcp("ui_scale", value=1.0)
        mcp("window_control", action="resize", width=900, height=720)
        settle()
        v = settled(BIG, frame="all")
        check(v.get("graph_panel") == "rail", "I a narrow window folds the Graph panel to a rail", str(v.get("graph_panel")))
        check((pane(v, "canvas") or [0, 0, 0, 0])[2] >= 400, "I and the canvas keeps its 400 pt", json.dumps(pane(v, "canvas")))
        v = settled(BIG, graph_panel="show")
        check(v.get("graph_panel") == "overlay", "I graph_panel=show opens it over the canvas", str(v.get("graph_panel")))
        graph, canvas = pane(v, "graph") or [0, 0, 0, 0], pane(v, "canvas") or [0, 0, 0, 0]
        check(abs(graph[0] - canvas[0]) <= 1, "I on the canvas's left edge", json.dumps([graph, canvas]))
        shot("rail_overlay")
        v = settled(BIG, graph_panel="toggle")
        check(v.get("graph_panel") == "rail", "I toggled back to the rail", str(v.get("graph_panel")))

        # ---------------------------------------------------------------------- J: clean up
        for rel in (BIG, SMALL):
            mcp("quest_close", path=rel, discard=True)
            d = mcp("content_delete", path=rel)
            check(d.get("ok"), "J %s is removed" % rel, json.dumps(d)[:160])
    finally:
        ed.close()


def main() -> int:
    global shots_dir
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True)
    ap.add_argument("--shots", help="a folder for the screenshots")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    if args.shots:
        shots_dir = os.path.abspath(args.shots)
        os.makedirs(shots_dir, exist_ok=True)

    work = tempfile.mkdtemp(prefix="grizzly_graphui_")
    copy = os.path.join(work, "QaGraphUi")
    qa_target.copy_project(src, copy, ignore=())
    os.makedirs(os.path.join(copy, "quests"), exist_ok=True)
    os.makedirs(os.path.join(copy, "npc"), exist_ok=True)
    print("running against a copy: " + copy, flush=True)

    kept = {}
    for path in KEPT:
        if os.path.isfile(path):
            with open(path, "rb") as f:
                kept[path] = f.read()

    try:
        run(copy)
    except Exception as exc:  # noqa: BLE001
        check(False, "the run finished", repr(exc))
    finally:
        for path in KEPT:
            if path in kept:
                with open(path, "wb") as f:
                    f.write(kept[path])
            elif os.path.isfile(path):
                os.remove(path)
        if not args.keep:
            qa_target.remove_tree(work)
    same = all(open(p, "rb").read() == kept[p] for p in kept if os.path.isfile(p))
    check(same, "level.ini, level_imgui.ini and _projects.ini are back byte for byte")

    ok = sum(1 for good, _ in results if good)
    print(f"GRAPH UI: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
