#!/usr/bin/env python3
"""End-to-end check of PDA interfaces (EditorPda: the .pdaasset, its preview tab, the
manifest pickup, the build gate) over MCP, on a THROW-AWAY COPY of a project.

What it proves:
  A  a new layout from the template (file.create) becomes the module's ([pda] layout in mod.ltx)
     as it appears, and mod.info reports it
  B  the game's reader judges a file (asset.check): the courier fixture reads clean, broken files
     come back with their codes (E3xx), a file that does not parse names its line
  C  mod.set pda.layout picks, clears and refuses (a missing file, a file the build leaves out)
  D  the preview plays the layout like the game: tabs, a row that fills vars and opens an
     overlay, a button that sets a var and switches pages, the keys, a stock section, the
     16:9 device, a click by canvas point - and the tab follows a write to the file
  E  the build ships the layout with [pda] in the module's mod.ltx, and refuses one with
     an error
  F  the used layout's file removed clears [pda]; Create > PDA Interface makes NewPda
  G  the editor's bundled reader and sheet are the game's, byte for byte
  H  the tab: its panes, Details tab and zoom over MCP, each answer holding the frame it asked for (panes,
     scale, device gap, screen edges - no second call); text put into the Source pane stays
     unsaved (the file untouched) and is never dropped without save= or discard=; a text that
     does not parse shows its line; nothing of the tab spills out of its window (ui.audit)

    python tools/qa/test_pda.py --project <project> [--only A,D,...] [--shots <folder>]

Each phase stands on its own setup: one that needs what another wrote (the template of A, the courier fixture of B,
C's pickup) runs that phase first, without its checks, when it was not asked for.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import shutil
import sys
import tempfile
import time

import qa
from qa import check, cmd


GAME_REPO = r"D:\Games\Dead Air\DeadAir-x64"
FIXTURE = os.path.join(qa.HERE, "fixtures", "courier.pdaasset")

PHASES = "ABCDHEFG"
NEEDS = {"C": "AB", "D": "B", "H": "AB", "E": "BC", "F": "BC"}
def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def sha(path: str) -> str:
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


PROJECT = [""]


def put(rel: str, text: str) -> None:
    """Writes a layout the way an agent does: a file of the project."""
    full = os.path.join(PROJECT[0], rel.replace("/", os.sep))
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as f:
        f.write(text)


def judged(text: str, rel: str = "pda/qa_judged.pdaasset") -> dict:
    """What asset.check finds in a text, written as a file of its own and taken away again."""
    put(rel, text)
    try:
        return cmd("asset.check", path=rel)
    finally:
        os.remove(os.path.join(PROJECT[0], rel.replace("/", os.sep)))


def until(what, seconds: float = 10.0):
    deadline = time.monotonic() + seconds
    got = what()
    while not got and time.monotonic() < deadline:
        time.sleep(0.1)
        got = what()
    return got


def codes(answer: dict) -> list[str]:
    return [p.get("code", "") for p in answer.get("problems", [])]


def visible(state: dict, **match) -> list[dict]:
    out = []
    for it in state.get("visible", []):
        if all(it.get(k) == v for k, v in match.items()):
            out.append(it)
    return out


def shot(folder: str | None, name: str) -> None:
    # a screenshot is also how a frame gets drawn in a minimized editor: the tab lays itself out in it
    pic = cmd("shot.window")
    if not folder:
        return
    if pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)


def settle(shots: str | None) -> None:
    """A picture for the operator waits for the preview's fonts and textures; without one nothing waits."""
    if shots:
        time.sleep(1.0)


def follows() -> dict:
    """The preview's state: pda.preview reads a file written behind it before it answers, so nothing waits."""
    return cmd("pda.preview", action="state")


def escaped_in_tab() -> list | None:
    """What the kit saw spill out of the work area's windows last frame; None when ui.audit is not there."""
    a = cmd("ui.audit")
    if not isinstance(a.get("escaped"), list):
        return None
    return [e for e in a["escaped"] if "doc.host" in str(e.get("window", ""))]


def run_tab(project: str, fixture: str, shots: str | None) -> None:
    courier = os.path.join(project, "pda", "courier.pdaasset")
    before = sha(courier)
    p = cmd("pda.preview", action="open", path="pda/courier.pdaasset", pane="both", tab="problems", zoom="fit")
    panes = p.get("panes", {})
    check(p.get("ok") and p.get("active") and panes.get("source") and panes.get("details") and panes.get("tab") == "problems"
          and p.get("zoom") == "fit", "H open: the pda tab is up with Source, Details on Problems, fit", json.dumps(p)[:300])
    # an opener answers the frames it owed, not the state before them (docs/UI_DESIGN.md, section 7)
    check(200 <= panes.get("details_width", 0) <= 480 and 100 <= panes.get("source_height", 0) <= 400
          and p.get("device_gap", -1) >= 15.0 and p.get("screen_edges"), "H the opener's answer is the frame it opened: panes, gap, edges",
          json.dumps({k: p.get(k) for k in ("panes", "device_gap", "screen_edges")}))
    shot(shots, "pda_tab_both")
    p = cmd("pda.preview", action="state")
    panes = p.get("panes", {})
    check(200 <= panes.get("details_width", 0) <= 480 and 100 <= panes.get("source_height", 0) <= 400,
          "H Details and Source keep their pt size", json.dumps(panes))
    esc = escaped_in_tab()
    if esc is not None:
        check(esc == [], "H nothing of the tab escapes its window", json.dumps(esc)[:400])
    for tab in ("pages", "vars", "log"):
        p = cmd("pda.preview", action="state", tab=tab)
        check(p.get("panes", {}).get("tab") == tab, "H the Details tab " + tab, json.dumps(p.get("panes")))
        shot(shots, "pda_tab_" + tab)
    bad = cmd("pda.preview", action="state", tab="nope")
    check(not bad.get("ok"), "H an unknown Details tab is refused", json.dumps(bad)[:200])
    p = cmd("pda.preview", action="state", zoom="100")
    check(p.get("zoom") == 1, "H zoom 100", json.dumps(p.get("zoom")))
    # the same answer already holds the frame drawn at 100%
    check(abs(p.get("scale", 0) - 1.0) < 0.01, "H at 100% the device is drawn pixel for pixel", str(p.get("scale")))
    shot(shots, "pda_tab_100")
    p = cmd("pda.preview", action="state", zoom="fit", pane="details")
    check(p.get("zoom") == "fit" and not p.get("panes", {}).get("source"), "H back to fit, Source hidden")
    # Fit fits the device - texture, screen and close button - with a 16 pt margin, on both aspects
    for aspect in ("4:3", "16:9"):
        # the aspect's own answer is the frame drawn at it
        p = cmd("pda.preview", action="aspect", aspect=aspect)
        check(15.0 <= p.get("device_gap", -1) <= 64.0, "H " + aspect + " fit keeps the device clear of the canvas' edges",
              str(p.get("device_gap")))
        # the device's art runs past the game's screen: the edges that crop it are drawn - its bottom on every aspect
        edges = p.get("screen_edges", [])
        check("bottom" in edges and (aspect == "16:9" or len(edges) == 4), "H " + aspect + " the screen's edge is drawn where it crops the device",
              json.dumps(edges))
        shot(shots, "pda_tab_fit_" + aspect.replace(":", "x"))
    cmd("pda.preview", action="aspect", aspect="4:3")

    # the Source pane: an applied text is shown, unsaved, and the file stays as it is
    edited = fixture.replace('title = "Orders"', 'title = "Queue"', 1)
    check(edited != fixture, "H the fixture takes an edit")
    p = cmd("pda.preview", action="apply", text=edited)
    titles = ["Queue"] if 'title = "Queue"' in read(courier) else []
    check(p.get("ok") and p.get("text_dirty") is True and sha(courier) == before and "Queue" not in titles,
          "H apply shows the text unsaved and leaves the file alone", json.dumps(p)[:200])
    shot(shots, "pda_tab_dirty")
    other = cmd("pda.preview", action="open", path="pda/qa_app.pdaasset")
    check(not other.get("ok") and other.get("error") == "unsaved" and other.get("dirty"),
          "H opening another layout with unsaved text answers unsaved", json.dumps(other)[:200])
    closed = cmd("pda.preview", action="close")
    check(not closed.get("ok") and closed.get("error") == "unsaved", "H so does closing the tab", json.dumps(closed)[:200])
    p = cmd("pda.preview", action="reload")
    check(p.get("ok") and p.get("text_dirty") is True, "H reload keeps the Source pane's edits", json.dumps(p)[:200])
    p = cmd("pda.preview", action="revert")
    check(not p.get("ok") and p.get("code") == "unsaved", "H revert without discard keeps them: it would lose them", json.dumps(p)[:200])
    p = cmd("pda.preview", action="revert", discard=True)
    check(p.get("ok") and p.get("text_dirty") is False and sha(courier) == before, "H revert discard=true drops them, the file untouched")
    cmd("pda.preview", action="apply", text=edited)
    p = cmd("pda.preview", action="open", path="pda/qa_app.pdaasset", discard=True)
    check(p.get("ok") and p.get("state", {}).get("path", "").endswith("qa_app.pdaasset") and sha(courier) == before,
          "H discard=true opens the other layout and drops the edits", json.dumps(p)[:200])
    p = cmd("pda.preview", action="open", path="pda/courier.pdaasset", pane="both", tab="problems")
    check(p.get("ok") and p.get("text_dirty") is False, "H and the first layout comes back clean", json.dumps(p)[:200])

    # the shell's save of the tab: the command answers once the file holds the text (the tab is dirty until then),
    # and a layout saved on the way to another one is written before the other one opens
    cmd("pda.preview", action="apply", text=edited)
    saved = cmd("doc.save", doc="pda")
    state = cmd("pda.preview", action="state")
    check(saved.get("ok") and 'title = "Queue"' in read(courier) and state.get("text_dirty") is False,
          "H doc.save answers once the file holds the text", json.dumps(saved)[:200])
    cmd("pda.preview", action="apply", text=fixture)
    moved = cmd("ui.tab", action="open", kind="pda", path="pda/qa_app.pdaasset", save=True)
    state = cmd("pda.preview", action="state")
    check(moved.get("ok") and 'title = "Orders"' in read(courier)
          and state.get("state", {}).get("path", "").endswith("qa_app.pdaasset"),
          "H ui.tab open save=true writes the layout it leaves, then opens the other one", json.dumps(moved)[:200])
    p = cmd("pda.preview", action="open", path="pda/courier.pdaasset", pane="both", tab="problems")
    check(p.get("ok") and p.get("text_dirty") is False, "H the first layout is back, saved", json.dumps(p)[:200])
    put("pda/courier.pdaasset", fixture)
    follows()

    # a text that does not parse: its line, and nothing written
    broken = fixture.replace('title = "Orders"', 'title = = "Orders"', 1)
    p = cmd("pda.preview", action="apply", text=broken)
    check(p.get("ok") and p.get("state", {}).get("read") is False and p.get("read_line", 0) > 0 and sha(courier) == before,
          "H a text that does not parse shows its line and writes nothing", json.dumps(p)[:300])
    shot(shots, "pda_tab_error")
    p = cmd("pda.preview", action="save", pane="details")
    check(p.get("ok") and p.get("text_dirty") is False, "H save writes the pane's text", json.dumps(p)[:200])
    check(sha(courier) != before, "H and the file has it")
    put("pda/courier.pdaasset", fixture)
    p = follows()
    check(p.get("state", {}).get("read") is True and p.get("text_dirty") is False, "H the fixture is back", json.dumps(p)[:200])

    # the file changes on disk while the pane holds edits: the two are merged value by value
    cmd("pda.preview", action="apply", text=fixture.replace('title = "Orders"', 'title = "Queue"', 1))
    put("pda/courier.pdaasset", fixture.replace('text = "Orders"', 'text = "Deliveries"', 1))
    merged = until(lambda: any(i.get("text") == "Deliveries"
                               for i in follows().get("state", {}).get("visible", [])))
    p = follows()
    check(bool(merged) and p.get("text_dirty") is True and not p.get("disk_note"),
          "H an outside change reaches a pane with edits, the edits kept", json.dumps(p)[:200])
    cmd("pda.preview", action="save")
    on_disk = read(courier)
    check('title = "Queue"' in on_disk and 'text = "Deliveries"' in on_disk,
          "H and Save writes both: the pane's value and the file's", on_disk[:200])
    # both change the same value: the pane keeps its own, the tab says so, Take Theirs takes the file's
    cmd("pda.preview", action="apply", text=on_disk.replace('title = "Queue"', 'title = "Mine"', 1))
    put("pda/courier.pdaasset", on_disk.replace('title = "Queue"', 'title = "Theirs"', 1))
    noted = until(lambda: "1 value" in str(follows().get("disk_note", "")))
    check(bool(noted), "H a value both changed is named: Changed on disk too", json.dumps(follows())[:300])
    t = cmd("asset.take_theirs", path="pda/courier.pdaasset")
    p = follows()
    check(t.get("ok") and p.get("text_dirty") is False and not p.get("disk_note")
          and 'title = "Theirs"' in read(courier), "H asset.take_theirs takes the file's text", json.dumps(t)[:200])
    put("pda/courier.pdaasset", fixture)
    until(lambda: not any(i.get("text") == "Deliveries" for i in follows().get("state", {}).get("visible", [])))


def run(ctx: qa.Run) -> None:
    project, shots, only = ctx.project, ctx.shots, ctx.only
    # a project that already names a layout starts from none: the test makes its own
    for folder, _dirs, files in os.walk(project):
        for name in files:
            if name.lower().endswith(".pdaasset"):
                os.remove(os.path.join(folder, name))
    ml = os.path.join(project, "mod.ltx")
    if os.path.isfile(ml):
        lines, skip = [], False
        for line in read(ml).splitlines():
            if line.strip().startswith("["):
                skip = line.strip().lower() == "[pda]"
            if not skip:
                lines.append(line)
        with open(ml, "w", encoding="utf-8", newline="\r\n") as f:
            f.write("\n".join(lines) + "\n")
    ed = qa.Editor(project)
    PROJECT[0] = project
    out_dir = tempfile.mkdtemp(prefix="grizzly_pda_")
    try:
        ed.start()
        manifest = cmd("mod.info").get("manifest", {})
        module = manifest.get("id", "")
        check(bool(module), "the project has a module id", json.dumps(manifest)[:200])
        mod_ltx = os.path.join(project, "mod.ltx")
        fixture = read(FIXTURE)
        broken = fixture.replace('kind = "rect", x = 0, y = 0, w = 960', 'kind = "circle", x = 0, y = 0, w = 960', 1)
        qa.run_phases(PHASE, only, NEEDS, project, shots, module, mod_ltx, fixture, broken, out_dir)
    finally:
        ed.close()
        qa.remove_tree(out_dir)



def layout_of() -> str:
    """`[pda] layout` as mod.info answers it: project-relative, with /."""
    return cmd("mod.info").get("manifest", {}).get("pda_layout", "")


def use_layout(path: str) -> dict:
    return cmd("mod.set", set={"pda": {"layout": path}})

def phase_a(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        none = [f for _, _, fs in os.walk(project) for f in fs if f.lower().endswith(".pdaasset")]
        check(layout_of() == "" and none == [], "A a fresh project has no PDA layout",
              json.dumps(none)[:300])
        os.makedirs(os.path.join(project, "pda"), exist_ok=True)
        w = cmd("file.create", kind="pda", path="pda")
        made = w.get("path", "")
        used = until(lambda: layout_of() == made)
        check(w.get("ok") and bool(used), "A the template is written and, the first layout, used as it appears",
              json.dumps(w)[:400] + " " + str(layout_of()))
        # the later phases know it as qa_app: renamed on disk, named again
        os.replace(os.path.join(project, made.replace("/", os.sep)), os.path.join(project, "pda", "qa_app.pdaasset"))
        # the old name is gone (the module drops it) and a new file appeared: either may come first
        until(lambda: layout_of() != made)
        u = use_layout("pda/qa_app.pdaasset")
        check(u.get("ok"), "A mod.set pda.layout names the renamed file", json.dumps(u)[:200])
        c = cmd("asset.check", path="pda/qa_app.pdaasset")
        check(c.get("ok") and c.get("counts", {}).get("error", 1) == 0, "A and it reads clean", json.dumps(c)[:300])
        check(os.path.isfile(os.path.join(project, "pda", "qa_app.pdaasset")), "A pda/qa_app.pdaasset exists")
        text = read(mod_ltx)
        check("[pda]" in text and "pda\\qa_app.pdaasset" in text, "A mod.ltx names it in [pda] layout", text[-300:])
        check(layout_of() == "pda/qa_app.pdaasset", "A mod.info reports it")
        tpl = read(os.path.join(project, "pda", "qa_app.pdaasset"))
        check(all(('id = "%s"' % pg) in tpl for pg in ("home", "map", "journal")) and "stock" in tpl,
              "A the template has home, map and a stock journal", tpl[:300])



def phase_b(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        put("pda/courier.pdaasset", fixture)
        w = cmd("asset.check", path="pda/courier.pdaasset")
        counts = w.get("counts", {})
        check(w.get("ok") and counts.get("error") == 0 and counts.get("warning") == 0,
              "B the courier fixture reads clean", json.dumps(w.get("problems"))[:400])
        time.sleep(0.4)
        check(layout_of() == "pda/qa_app.pdaasset",
              "B and is not used by itself: the module has a layout already")
        v = judged(broken)
        e305 = [x for x in v.get("problems", []) if x.get("code") == "E305"]
        check(bool(e305) and e305[0].get("line", 0) > 0, "B an unknown widget kind is E305, at its line", json.dumps(v)[:300])
        v = judged(fixture.replace('page = "map" }', 'page = "nowhere" }', 1))
        check("E308" in codes(v), "B a tab to a page that does not exist is E308", str(codes(v)))
        v = judged(fixture.replace('"ui_xms_icon_plus"', '"ui_qa_no_such_texture"', 1))
        check("W320" in codes(v), "B a texture neither the game nor the project has is W320", str(codes(v)))
        bad = judged("return { pda = 1,\n  id = \n}")
        syntax = [x for x in bad.get("problems", []) if x.get("code") == "syntax"]
        check(bool(syntax) and syntax[0].get("line", 0) >= 2, "B a file that does not parse names its line",
              json.dumps(bad)[:300])
        code = judged("os.execute('calc') return { pda = 1 }")
        check(any(x.get("severity") == "error" for x in code.get("problems", [])), "B code outside strings is refused",
              json.dumps(code)[:300])
        newer = judged("return { pda = 2, id = \"qa_newer\" }")
        check("newer_version" in codes(newer), "B a layout of a newer format is refused with the sentence",
              json.dumps(newer)[:300])



def phase_c(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        u = use_layout("pda/courier.pdaasset")
        check(u.get("ok") and u.get("manifest", {}).get("pda_layout") == "pda/courier.pdaasset", "C mod.set pda.layout picks another layout", json.dumps(u))
        check("pda\\courier.pdaasset" in read(mod_ltx), "C mod.ltx follows")
        u = use_layout("pda/none.pdaasset")
        check(not u.get("ok"), "C a missing file is refused", json.dumps(u))
        os.makedirs(os.path.join(project, "drafts"), exist_ok=True)
        shutil.copyfile(FIXTURE, os.path.join(project, "drafts", "draft.pdaasset"))
        cmd("mod.exclude", path="drafts", on=True)
        u = use_layout("drafts/draft.pdaasset")
        check(not u.get("ok") and "left out" in u.get("error", ""), "C a file the build leaves out is refused", json.dumps(u))
        cmd("mod.exclude", path="drafts", on=False)
        listed = cmd("asset.check", path="pda").get("files")
        check(listed == 2, "C asset.check of the folder covers both layouts", str(listed))



def phase_d(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        p = cmd("pda.preview", action="open", path="pda/courier.pdaasset")
        st = p.get("state", {})
        check(p.get("ok") and p.get("open") and st.get("page") == "orders", "D the preview opens on the start page",
              json.dumps(p)[:300])
        settle(shots)
        shot(shots, "pda_orders")
        tabs = visible(st, id="tab_orders")
        check(tabs and tabs[0].get("selected") is True, "D the tab of the page is selected", json.dumps(tabs))
        rows = visible(st, kind="row")
        check(len(rows) == 2, "D the list shows its two rows", json.dumps(rows)[:300])
        check(visible(st, id="details") == [], "D the overlay starts hidden")
        p = cmd("pda.preview", action="click", n=rows[0]["n"])
        st = p.get("state", {})
        check(st.get("vars", {}).get("picked") == "Тушёнка" and visible(st, id="details"),
              "D a row fills its var and opens the overlay", json.dumps(st.get("vars"), ensure_ascii=False))
        check(visible(st, kind="row", selected=True), "D and becomes the list's selection")
        settle(shots)
        shot(shots, "pda_details")
        p = cmd("pda.preview", action="click", id="orders", row=2)
        check(p.get("state", {}).get("vars", {}).get("picked") == "Batteries", "D a list row by its number",
              json.dumps(p.get("state", {}).get("vars"), ensure_ascii=False))
        bad = cmd("pda.preview", action="click", id="orders", row=9)
        check(not bad.get("ok"), "D a row the list does not have is refused", json.dumps(bad)[:200])
        cmd("pda.preview", action="click", n=rows[0]["n"])
        p = cmd("pda.preview", action="click", id="take")
        st = p.get("state", {})
        check(st.get("page") == "map" and st.get("vars", {}).get("current") == "Тушёнка" and not visible(st, id="details"),
              "D Take: var, overlay hidden, the map page", json.dumps(st, ensure_ascii=False)[:300])
        settle(shots)
        shot(shots, "pda_map")
        p = cmd("pda.preview", action="click", id="done")
        st = p.get("state", {})
        check(st.get("vars", {}).get("current") == "Нет заказа" and "signal:courier_done" in st.get("log", []),
              "D Done: the signal goes to the log, the var resets", json.dumps(st.get("log"), ensure_ascii=False))
        orders_tab = visible(st, id="tab_orders")[0]
        p = cmd("pda.preview", action="click", x=orders_tab["x"] + orders_tab["w"] / 2, y=orders_tab["y"] + orders_tab["h"] / 2)
        check(p.get("state", {}).get("page") == "orders", "D a click by canvas point hits the tab", json.dumps(p)[:200])
        p = cmd("pda.preview", action="key", key="map")
        check(p.get("state", {}).get("page") == "map", "D the map key opens the map page")
        p = cmd("pda.preview", action="key", key="contacts")
        st = p.get("state", {})
        check(st.get("stock") == "eptContacts", "D the contacts key shows the stock contacts", json.dumps(st)[:200])
        check(visible(st, id="tab_journal") == [] and visible(st, id="back"),
              "D a stock section hides the chrome and keeps the overlay", json.dumps(st.get("visible"))[:300])
        p = cmd("pda.preview", action="click", id="back")
        check(p.get("state", {}).get("stock") == "" and p.get("state", {}).get("page") == "map",
              "D the overlay's back leaves the stock section", json.dumps(p.get("state"))[:200])
        p = cmd("pda.preview", action="click", id="tab_journal")
        st = p.get("state", {})
        check(st.get("page") == "journal" and st.get("stock") == "eptTasks", "D a stock page shows the stock section")
        settle(shots)
        shot(shots, "pda_journal")
        p = cmd("pda.preview", action="aspect", aspect="16:9")
        check(p.get("state", {}).get("aspect") == "16:9", "D the 16:9 device")
        settle(shots)
        shot(shots, "pda_wide")
        cmd("pda.preview", action="aspect", aspect="4:3")
        p = cmd("pda.preview", action="reset")
        st = p.get("state", {})
        check(st.get("page") == "orders" and st.get("vars", {}).get("current") == "Нет заказа", "D reset: the start page and the declared vars")
        # the tab follows the file
        put("pda/courier.pdaasset", fixture.replace('text = "Orders"', 'text = "Deliveries"', 1))
        p = follows()
        check(p.get("state", {}).get("read") is True, "D the tab re-reads a file written behind it", json.dumps(p)[:200])
        seen = until(lambda: any(i.get("text") == "Deliveries"
                                 for i in cmd("pda.preview", action="state").get("state", {}).get("visible", [])))
        check(bool(seen), "D and the change is there")
        # the map slot lists what it shows, in canvas units: the player and the markers inside it
        p = cmd("pda.preview", action="page", page="map")
        mp = p.get("state", {}).get("map", {})
        b = mp.get("bound") or [0, 0, 100, 100]
        cx, cz = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
        marked = fixture.replace('preview = { level = "l01_escape" },',
                                 'preview = { level = "l01_escape", actor = { x = %g, z = %g }, markers = {'
                                 ' { x = %g, z = %g, hint = "Parcel" }, { x = %g, z = %g, hint = "Far" } } },'
                                 % (cx, cz, cx, cz + 20, b[0] + 1, b[1] + 1), 1)
        check(marked != fixture, "D the fixture takes preview markers")
        put("pda/courier.pdaasset", marked)
        follows()
        cmd("pda.preview", action="reset")
        p = cmd("pda.preview", action="page", page="map")
        spots = p.get("state", {}).get("map", {}).get("spots", [])
        me = [s for s in spots if s.get("what") == "player"]
        parcel = [s for s in spots if s.get("hint") == "Parcel"]
        check(len(me) == 1 and len(parcel) == 1 and parcel[0]["y"] < me[0]["y"],
              "D the map slot lists the player and a marker north of them", json.dumps(spots))
        check(not [s for s in spots if s.get("hint") == "Far"], "D a marker outside the slot is not listed", json.dumps(spots))
        put("pda/courier.pdaasset", fixture)
        follows()


def phase_h(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        run_tab(project, fixture, shots)


def phase_e(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        ex = cmd("mod.build", target="folder", path=out_dir)
        check(ex.get("ok"), "E the module builds with its PDA layout", json.dumps(ex)[:300])
        root = os.path.join(out_dir, "modules", module)
        check(os.path.isfile(os.path.join(root, "pda", "courier.pdaasset")), "E the layout ships as it is")
        shipped = read(os.path.join(root, "mod.ltx")) if os.path.isfile(os.path.join(root, "mod.ltx")) else ""
        check("[pda]" in shipped and "courier.pdaasset" in shipped, "E the module's mod.ltx carries [pda]", shipped[-200:])
        put("pda/courier.pdaasset", broken)
        ex = cmd("mod.build", target="folder", path=out_dir)
        check(not ex.get("ok") and "E305" in ex.get("error", ""), "E a used layout with an error stops the build",
              json.dumps(ex)[:300])
        put("pda/courier.pdaasset", fixture)



def phase_f(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        os.remove(os.path.join(project, "pda", "courier.pdaasset"))
        cleared = until(lambda: layout_of() == "")
        check(bool(cleared), "F the used layout's file removed clears [pda] layout",
              str(layout_of()))
        check("courier" not in read(mod_ltx), "F mod.ltx names it no more", read(mod_ltx)[-200:])
        c = cmd("file.create", kind="pda", path="pda")
        made = c.get("path", "")
        check(c.get("ok") and made.lower().endswith("newpda.pdaasset"), "F Create > PDA Interface makes NewPda", json.dumps(c))
        now = until(lambda: layout_of() == made)
        check(bool(now), "F and, the only layout now, the module uses it", str(layout_of()))



def phase_g(project, shots, module, mod_ltx, fixture, broken, out_dir) -> None:
        pairs = [
            ("gamedata/configs/pda/xms_pda_layout.script", "packaging/dead-air-x64/compatibility/gamedata/scripts/xms_pda_layout.script"),
            ("gamedata/configs/pda/ui_xms_pda.xml", "packaging/dead-air-x64/compatibility/gamedata/configs/ui/textures_descr/ui_xms_pda.xml"),
        ]
        if os.path.isdir(GAME_REPO):
            for mine, theirs in pairs:
                a, b = os.path.join(qa.REPO, mine), os.path.join(GAME_REPO, theirs)
                check(os.path.isfile(a) and os.path.isfile(b) and sha(a) == sha(b), "G the bundled " + os.path.basename(mine) + " is the game's")
            fx = os.path.join(GAME_REPO, "tools", "nq", "fixtures", "pda", "courier.pdaasset")
            check(os.path.isfile(fx) and sha(fx) == sha(FIXTURE), "G the fixture is the game repo's")


PHASE = {"A": phase_a, "B": phase_b, "C": phase_c, "D": phase_d, "H": phase_h, "E": phase_e, "F": phase_f,
         "G": phase_g}


def main() -> int:
    return qa.suite("PDA", run, doc=__doc__, ignore=(".git",), phases="ABCDHEFG")


if __name__ == "__main__":
    sys.exit(main())
