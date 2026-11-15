#!/usr/bin/env python3
"""The graph editor as docs/UI_DESIGN.md section 6 and docs/NQ_ARCHITECTURE.md 13.8-13.11 promise it.

Drives the editor over MCP on a THROW-AWAY COPY of a project, at 1707x912 pt with UI scale 1.5 and
at 1280x720 pt with UI scale 1.0:

- A: content-sized boxes: 180-360 pt wide, a node with actions taller than one without, a
  collapsed node only its header; an action row says what it does in words (the catalog's summary);
- B: every wire style in both flows: every wire has its polyline, a back edge goes back and is
  routed, and the canvas's hit test finds every wire at the middle of its own polyline;
- C: the picker: from an output only kinds that take an input, into an input only kinds that go
  on; on the empty canvas the canvas's menu; every menu can be put up to look at;
- D: comment frames and reroutes: a frame around the selection, moving it moves what it holds;
  a reroute survives a rename of its node;
- E: disable and collapse: a disabled node is a note, not a warning, and the outline says so;
- F: align, distribute, straighten, select connected, bookmarks by name, the flow switched;
- G: Frame All of the 110-node quest is readable: every header's text at least 11 px;
- H: the panes: Graph | canvas | Details, the canvas keeps 400 pt, Details folds to a rail under
  1400 pt and opens over the canvas; the Graph panel folds to a rail on a narrow tab;
- I: panning the big quest leaves perf.stalls empty;
- J: a behaviour graph's tab says "graph", never "quest".

    python tools/qa/test_graph_ui.py --project <project> [--shots <dir>] [--only A,B]
"""
from __future__ import annotations

import base64
import json
import os
import sys

import qa
from qa import check, cmd


BIG = "quests/qa_big.nqasset"
WIRES = "quests/qa_wires.nqasset"
FAMILY = "quests/qa_family.nqasset"
SMALL = "npc/qa_small.behasset"
SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))
STYLES = ("curved", "straight", "circuit", "angled", "subway")
TRIGGERS = ("trigger.start", "trigger.when")

shots_dir: str | None = None
only: set[str] = set()


def wanted(phase: str) -> bool:
    return not only or phase in only


def shot(name: str) -> None:
    if not shots_dir:
        return
    r = cmd("shot.window")
    if r.get("ok") and r.get("png_base64"):
        with open(os.path.join(shots_dir, name + ".png"), "wb") as f:
            f.write(base64.b64decode(r["png_base64"]))


def doc(rel: str) -> str:
    return ("beh:" if rel.endswith(".behasset") else "quest:") + rel


def view(rel: str, **extra) -> dict:
    return cmd("graph.view", doc=doc(rel), **extra)


def edit(rel: str, ops: list) -> dict:
    return cmd("graph.edit", doc=doc(rel), ops=ops)


def get(rel: str, **extra) -> dict:
    return cmd("graph.get", doc=doc(rel), **extra)


def pane(v: dict, name: str):
    return (v.get("panes") or {}).get(name)


def boxes_of(rel: str) -> dict:
    return {b.get("id"): b for b in get(rel, boxes=True).get("boxes", [])}


WIRES_TEXT = """return {
  nq = 1,
  id = "qa_wires",
  title = "QA wires",
  vars = { n = 0 },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "pick" } },
    { id = "pick", kind = "flow.branch", params = { cases = {
        { name = "north", cond = { { kind = "var", params = { name = "n", op = "eq", value = 1 } } } },
        { name = "south", cond = { { kind = "var", params = { name = "n", op = "eq", value = 2 } } } },
      } }, out = { north = "go_north", south = "go_south", ["else"] = "wait" } },
    { id = "go_north", kind = "flow.step", on_enter = { { kind = "var.set", params = { name = "n", value = 3 } } }, out = { next = "finish" } },
    { id = "go_south", kind = "flow.step", out = { next = "finish" } },
    { id = "wait", kind = "wait.when", cond = { { kind = "var", params = { name = "n", op = "gt", value = 0 } } },
      params = { timeout = { game_minutes = 5 } }, out = { done = "pick", timeout = "finish" } },
    { id = "finish", kind = "flow.end", params = { status = "completed" } },
  },
}
"""

FAMILY_TEXT = """return {
  nq = 1,
  id = "qa_family",
  title = "QA families",
  vars = { n = 0 },
  tasks = { find = { title = "Find the stash", descr = "Somewhere in the swamp" } },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "talk" }, comment = "Every family on one canvas" },
    { id = "talk", kind = "dialog.topic", params = { npc = { story = "esc_m_trader" }, text = "Got a job for me?" }, out = { done = "go" } },
    { id = "go", kind = "flow.step", out = { next = "wait" }, on_enter = {
        { kind = "task.give", params = { task = "find" } },
        { kind = "item.give", params = { section = "bread", count = 2 } },
        { kind = "var.set", params = { name = "n", value = 1 } },
        { kind = "news.tip", params = { text = "The stash is near the old farm" } },
        { kind = "relation.set", params = { who = { community = "bandit" }, value = "enemy" } },
        { kind = "spawn.squad", params = { section = "simulation_bandit_3", smart = "esc_smart_terrain_2_12" } },
      } },
    { id = "wait", kind = "wait.timer", params = { duration = { game_minutes = 30 } }, out = { done = "reach" } },
    { id = "reach", kind = "objective.reach", params = { place = { smart = "esc_smart_terrain_3_16" } }, out = { done = "finish" } },
    { id = "finish", kind = "flow.end", params = { status = "completed" } },
  },
}
"""

SMALL_TEXT = """return {
  beh = 1,
  id = "qa_small",
  title = "QA small graph",
  roles = { walker = { title = "Walker" } },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "walk" } },
    { id = "walk", kind = "npc.goto", params = { who = "walker", place = { smart = "esc_smart_terrain_2_12" } }, out = { done = "finish" } },
    { id = "finish", kind = "flow.end" },
  },
}
"""


def write(project: str, rel: str, text: str) -> None:
    full = os.path.join(project, rel.replace("/", os.sep))
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as f:
        f.write(text)


def phase_a() -> None:
    boxes = boxes_of(FAMILY)
    widths = [b.get("w", 0) for b in boxes.values()]
    check(widths and all(180 <= w <= 360 for w in widths), "A every box is 180-360 pt wide, by its content", json.dumps(widths))
    check(boxes.get("go", {}).get("h", 0) > boxes.get("finish", {}).get("h", 0), "A a node with actions is taller than one without",
          json.dumps([boxes.get("go"), boxes.get("finish")]))
    outline = get(FAMILY).get("outline", "")
    check("item.give" in outline and "bread" in outline, "A the outline says what the actions do", outline[:300])
    r = edit(FAMILY, [{"op": "collapse", "nodes": ["go"], "on": True}])
    check(r.get("ok"), "A collapse", json.dumps(r)[:200])
    check(boxes_of(FAMILY).get("go", {}).get("h") == 28, "A a collapsed node is its header alone",
          json.dumps(boxes_of(FAMILY).get("go")))
    edit(FAMILY, [{"op": "collapse", "nodes": ["go"], "on": False}])
    view(FAMILY, frame="all", graph_panel="show", details="show")
    view(FAMILY, zoom_level=16, center=[400, 120])
    shot("node_families")


def phase_b(tag: str) -> None:
    for flow in ("right", "down"):
        r = cmd("graph.arrange", doc=doc(WIRES), action="layout", flow=flow)
        check(r.get("ok") and r.get("flow") == flow, "%s B the graph flows %s" % (tag, flow), json.dumps(r)[:200])
        for style in STYLES:
            e = edit(WIRES, [{"op": "wires", "style": style}])
            check(e.get("ok"), "%s B %s %s: the style is set" % (tag, flow, style), json.dumps(e)[:200])
            g = get(WIRES, wires=True)
            wires = g.get("wires", [])
            check(len(wires) == 8 and all(len(w.get("points", [])) >= 2 for w in wires),
                  "%s B %s %s: every wire has its polyline" % (tag, flow, style), json.dumps(wires)[:300])
            back = [w for w in wires if w.get("back")]
            check(any(w.get("from") == "wait" and w.get("to") == "pick" for w in back),
                  "%s B %s %s: the wire back up is a back edge" % (tag, flow, style), json.dumps(back)[:200])
            missed = []
            for w in wires:
                pts = w.get("points", [])
                mid = pts[len(pts) // 2]
                got = get(WIRES, at=mid).get("hit") or {}
                hit = got.get("wire") or {}
                own = (hit.get("from"), hit.get("pin"), hit.get("to")) == (w.get("from"), w.get("pin"), w.get("to"))
                # wires into one input (or out of one node) share their last (first) stretch: a point
                # there is on both, and the hit may name either
                shared = hit.get("distance", 99) <= 0.5 and (hit.get("to") == w.get("to") or hit.get("from") == w.get("from"))
                if not (own or shared or got.get("node")):
                    missed.append([w.get("from"), w.get("pin"), w.get("to"), mid, hit])
            check(not missed, "%s B %s %s: every wire is hit on its own polyline" % (tag, flow, style), json.dumps(missed)[:300])
            v = view(WIRES, frame="all")
            check(v.get("wire_style", {}).get("style") == style and v.get("flow") == flow,
                  "%s B %s %s: the tab draws it" % (tag, flow, style), json.dumps(v.get("wire_style")))
            if tag.startswith("1707"):
                shot("wires_%s_%s" % (flow, style))
    edit(WIRES, [{"op": "wires", "style": "default"}])


def phase_c(tag: str) -> None:
    v = view(WIRES, menu="picker", from_pin="start.next")
    picker = v.get("picker") or {}
    kinds = picker.get("kinds", [])
    check(v.get("menu") == "picker" and picker.get("for") == "from_pin" and kinds,
          "%s C a wire dropped from an output opens the picker" % tag, json.dumps(v.get("picker"))[:200])
    check(not any(k in TRIGGERS for k in kinds), "%s C from an output it offers no trigger" % tag,
          json.dumps([k for k in kinds if k.startswith("trigger")]))
    if tag.startswith("1707"):
        shot("picker_from_pin")
    v = view(WIRES, menu="picker", into="finish")
    picker = v.get("picker") or {}
    check(picker.get("for") == "into_input" and "flow.end" not in picker.get("kinds", []),
          "%s C into an input only kinds that go on: no flow.end" % tag, json.dumps(picker)[:200])
    for menu in ("node", "wire", "canvas"):
        v = view(WIRES, menu=menu)
        check(v.get("menu") == menu, "%s C graph.view menu=%s puts it up" % (tag, menu), json.dumps(v.get("menu")))
        if tag.startswith("1707"):
            shot("menu_" + menu)
    v = view(WIRES, menu="none")
    check(v.get("menu") is None, "%s C menu=none closes it" % tag, json.dumps(v.get("menu")))


def phase_d() -> None:
    view(WIRES, select=["go_north", "go_south"])
    r = edit(WIRES, [{"op": "frame", "action": "add", "title": "The two ways", "tone": 3}])
    frames = get(WIRES).get("frames", [])
    check(r.get("ok") and len(frames) == 1 and frames[0].get("title") == "The two ways", "D a frame around the selection",
          json.dumps(frames))
    before = boxes_of(WIRES)
    edit(WIRES, [{"op": "frame", "action": "move", "index": 1, "by": [64, 32]}])
    after = boxes_of(WIRES)
    check(after["go_north"]["x"] == before["go_north"]["x"] + 64 and after["go_south"]["y"] == before["go_south"]["y"] + 32,
          "D moving the frame moves the nodes inside it", json.dumps([before.get("go_north"), after.get("go_north")]))
    check(after["finish"]["x"] == before["finish"]["x"], "D and nothing outside it", json.dumps(after.get("finish")))
    w = next((x for x in get(WIRES, wires=True).get("wires", []) if x.get("from") == "go_south"), {})
    pts = w.get("points", [[0, 0]])
    # the reroute in the open, under its node and to the left of the end: the wire bends round it
    knot = [pts[0][0] + 40, pts[-1][1] + 60]
    edit(WIRES, [{"op": "route", "from": "go_south", "pin": "next", "to": "finish", "points": [knot]}])
    edit(WIRES, [{"op": "rename", "node": "go_south", "to": "south_way"}])
    routes = get(WIRES).get("routes", [])
    check(any(r.get("wire") == "south_way.next>finish" for r in routes), "D the reroute follows its node's rename", json.dumps(routes))
    view(WIRES, frame="all")
    shot("frames_and_reroutes")
    edit(WIRES, [{"op": "rename", "node": "south_way", "to": "go_south"}])


def phase_e() -> None:
    r = edit(FAMILY, [{"op": "disable", "nodes": ["wait"], "on": True}, {"op": "collapse", "nodes": ["go"], "on": True}])
    check(r.get("ok"), "E disable and collapse", json.dumps(r)[:200])
    g = get(FAMILY)
    probs = [p for p in g.get("problems", []) if p.get("node") == "wait"]
    check(any(p.get("severity") == "note" for p in probs) and not any(p.get("severity") == "warning" for p in probs),
          "E a disabled node is a note, not a warning", json.dumps(probs)[:300])
    check("disabled" in g.get("outline", ""), "E the outline says it is disabled", g.get("outline", "")[:300])
    view(FAMILY, frame="all")
    shot("disabled_and_collapsed")
    edit(FAMILY, [{"op": "disable", "nodes": ["wait"], "on": False}, {"op": "collapse", "nodes": ["go"], "on": False}])


def phase_f() -> None:
    view(FAMILY, select=["talk", "go", "wait"])
    r = cmd("graph.arrange", doc=doc(FAMILY), action="align", how="top")
    b = boxes_of(FAMILY)
    check(r.get("ok") and b["talk"]["y"] == b["go"]["y"] == b["wait"]["y"], "F align top", json.dumps([b["talk"], b["go"], b["wait"]]))
    r = cmd("graph.arrange", doc=doc(FAMILY), action="distribute", how="horizontal")
    b = boxes_of(FAMILY)
    xs = sorted((b[i]["x"], b[i]["w"]) for i in ("talk", "go", "wait"))
    gaps = [xs[i + 1][0] - xs[i][0] - xs[i][1] for i in range(2)]
    check(r.get("ok") and abs(gaps[0] - gaps[1]) <= 16, "F distribute horizontally: equal gaps", json.dumps(gaps))
    view(FAMILY, select=["start"])
    v = view(FAMILY, grow="downstream")
    check(set(v.get("selected", [])) >= {"start", "talk", "finish"}, "F select downstream grows along the wires", json.dumps(v.get("selected")))
    r = cmd("graph.arrange", doc=doc(FAMILY), action="layout")
    check(r.get("ok"), "F auto layout", json.dumps(r)[:200])
    edit(FAMILY, [{"op": "bookmark", "name": "Finale", "node": "finish"}])
    view(FAMILY, select="none")
    v = view(FAMILY, bookmark="Finale")
    check(v.get("selected") == ["finish"], "F a bookmark by its name frames and selects its node", json.dumps(v.get("selected")))
    check(any(b.get("name") == "Finale" for b in v.get("bookmarks", [])), "F the bookmark is the file's", json.dumps(v.get("bookmarks")))


def phase_g(tag: str) -> None:
    if tag.startswith("1707"):
        # the fixture's positions are an old layout's: the auto layout of today
        r = cmd("graph.arrange", doc=doc(BIG), action="layout")
        check(r.get("ok"), tag + " G the big quest is laid out", json.dumps(r)[:200])
        # a tree along the flow, as long as the quest's longest way: no node lies on another
        boxes = r.get("boxes", [])
        on = [(a["id"], b["id"]) for i, a in enumerate(boxes) for b in boxes[i + 1:]
              if a["x"] < b["x"] + b["w"] and b["x"] < a["x"] + a["w"]
              and a["y"] < b["y"] + b["h"] and b["y"] < a["y"] + a["h"]]
        check(boxes and not on, tag + " G no node of it lies on another (%d nodes)" % len(boxes), json.dumps(on[:3]))
    # under 1400 pt Details is a rail: shown it would lie over the framed graph
    v = view(BIG, frame="all", graph_panel="show", details="show" if tag.startswith("1707") else "hide")
    check(v.get("ok"), tag + " G the big quest opens", json.dumps(v)[:200])
    check(v.get("nodes", 0) >= 100, tag + " G it is the 110-node quest", str(v.get("nodes")))
    # no text is drawn under 11 px at any zoom: the whole quest - a tree as long as its longest way - takes a
    # zoom where its nodes are blocks in their families' colours (titles on hover)
    check(v.get("title_px", 0) >= 11, tag + " G Frame All: no header's text under 11 px", json.dumps([v.get("title_px"), v.get("zoom")]))
    check(v.get("detail") in ("full", "header", "block"), tag + " G and the level of detail is one the zoom has",
          json.dumps([v.get("detail"), v.get("zoom")]))
    b = v.get("beyond") or {}
    check(sum(b.values()) == 0, tag + " G the whole graph is in view", json.dumps(b))
    shot("big_quest_framed_" + tag)


def phase_h(tag: str, width: int) -> None:
    v = view(BIG, graph_panel="show", details="show")
    canvas = pane(v, "canvas") or [0, 0, 0, 0]
    check(canvas[2] >= 400, tag + " H the canvas keeps 400 pt", json.dumps(canvas))
    if width >= 1400:
        check(v.get("details") == "open", tag + " H Details is a panel at 1400 pt and more", str(v.get("details")))
        graph, details = pane(v, "graph") or [0, 0, 0, 0], pane(v, "details") or [0, 0, 0, 0]
        check(graph[0] < canvas[0] < details[0], tag + " H Graph | canvas | Details", json.dumps([graph, canvas, details]))
    else:
        check(v.get("details") in ("rail", "overlay"), tag + " H under 1400 pt Details folds to a rail", str(v.get("details")))
        v = view(BIG, details="show")
        check(v.get("details") == "overlay", tag + " H details=show opens it over the canvas", str(v.get("details")))
        shot("details_rail_overlay")
        view(BIG, details="hide")


def phase_i() -> None:
    view(BIG, frame="all")
    view(BIG, zoom_level=14)
    cmd("perf.stalls", clear=True)
    c = view(BIG).get("center", [0, 0])
    for i in range(24):
        view(BIG, center=[c[0] + 60 * i, c[1] + 25 * i])
    qa.stalls("I panning the big quest leaves perf.stalls empty")


def phase_j(tag: str) -> None:
    v = view(SMALL, frame="all", graph_panel="show")
    check(v.get("ok"), tag + " J the behaviour graph opens", json.dumps(v)[:200])
    audit = cmd("ui.audit")
    said = []
    for key in ("controls", "truncated"):
        for e in audit.get(key, []):
            window = str(e.get("window", ""))
            if "doc.host" not in window and "toolbar" not in window:
                continue
            text = " ".join(str(e.get(k, "")) for k in ("label", "help", "why", "shown")).lower()
            if "quest" in text:
                said.append(text[:120])
    check(not said, tag + " J the behaviour graph's tab never says quest", json.dumps(said[:5]))


def run(ctx: qa.Run) -> None:
    global shots_dir, only
    project = ctx.project
    only = {x.strip().upper() for x in ctx.only.split(",") if x.strip()}
    if ctx.shots:
        shots_dir = os.path.abspath(ctx.shots)
        os.makedirs(shots_dir, exist_ok=True)
    ed = qa.Editor(project)
    try:
        ed.start()
        big = open(os.path.join(qa.REPO, "crates", "gz-nq", "src", "fixtures", "big_quest.nqasset"), encoding="utf-8").read()
        # the fixture keeps the id of the quest it was made from: a copy beside it takes its own
        big = big.replace('id = "n7_control"', 'id = "qa_big"', 1)
        for rel, text in ((BIG, big), (WIRES, WIRES_TEXT), (FAMILY, FAMILY_TEXT), (SMALL, SMALL_TEXT)):
            write(project, rel, text)
            c = cmd("asset.check", path=rel)
            errs = [p for p in c.get("problems", []) if p.get("severity") == "error"]
            check(c.get("ok") and not errs, "%s is written and has no error" % rel, json.dumps(errs)[:300])
            o = cmd("ui.tab", action="open", kind="beh" if rel.endswith(".behasset") else "quest", path=rel)
            check(o.get("ok"), "%s opens in its tab" % rel, json.dumps(o)[:200])

        for width, height, scale in SIZES:
            tag = "%dx%d@%g" % (width, height, scale)
            cmd("ui.scale", value=scale)
            r = cmd("app.window", action="resize", width=width, height=height)
            check(r.get("ok"), tag + " the window takes the size", json.dumps(r)[:200])
            if wanted("A") and width == SIZES[0][0]:
                phase_a()
            if wanted("B"):
                phase_b(tag)
            if wanted("C"):
                phase_c(tag)
            if wanted("D") and width == SIZES[0][0]:
                phase_d()
            if wanted("E") and width == SIZES[0][0]:
                phase_e()
            if wanted("F") and width == SIZES[0][0]:
                phase_f()
            if wanted("G"):
                phase_g(tag)
            if wanted("H"):
                phase_h(tag, width)
            if wanted("J"):
                phase_j(tag)
        if wanted("I"):
            phase_i()

        # the Graph panel folds to a rail on a narrow tab
        if wanted("H"):
            cmd("ui.scale", value=1.0)
            cmd("app.window", action="resize", width=700, height=600)
            v = view(BIG, frame="all")
            check(v.get("graph_panel") == "rail", "H a narrow window folds the Graph panel to a rail", str(v.get("graph_panel")))
            v = view(BIG, graph_panel="show")
            check(v.get("graph_panel") == "overlay", "H graph_panel=show opens it over the canvas", str(v.get("graph_panel")))

        for rel in (BIG, WIRES, FAMILY, SMALL):
            cmd("ui.tab", action="close", doc=doc(rel), discard=True)
            d = cmd("file.delete", path=rel)
            check(d.get("ok"), "%s is removed" % rel, json.dumps(d)[:160])
    finally:
        ed.close()


def main() -> int:
    return qa.suite("GRAPH UI", run, doc=__doc__, phases="ABCDEFGHIJ")


if __name__ == "__main__":
    sys.exit(main())
