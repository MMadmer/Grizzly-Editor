#!/usr/bin/env python3
"""End-to-end check of behaviour graphs (NQ_ARCHITECTURE par. 19).

Drives the editor over MCP on a THROW-AWAY COPY of a project: writes a behaviour
graph (.behasset) and a quest that runs it, and checks what makes it its own
asset rather than a quest with a flag - the extension is the kind, the header
has to agree with it, the halves of the catalog stay apart, a destination takes
a path, a ref, an object, a role or a place, behaviour.run has three pins, a
graph may not run itself, and both assets survive a build and a canonical
rewrite unchanged. The graph's tab frames readable titles, draws its panes and
never says "quest".

    python tools/qa/test_behaviours.py --project <project>
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

import graph_ref
import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP

results: list[tuple[bool, str]] = []


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
            mcp("app.quit")
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


AMBUSH = """return {
    beh   = 1,
    id    = "qa_ambush",
    title = "QA ambush",
    roles = {
        leader    = { title = "Leader" },
        attackers = { title = "Attackers", many = true },
    },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "move_in" } },
        { id = "move_in", kind = "npc.goto",
          params = { who = { "leader", "attackers" }, place = { path = "qa_walk" }, move = "run", arrive = true },
          out = { done = "to_the_case" } },
        { id = "to_the_case", kind = "npc.goto",
          params = { who = "leader", place = { ref = "dropped_case" }, arrive = true },
          out = { done = "dig_in" } },
        { id = "dig_in", kind = "flow.step",
          on_enter = {
              { kind = "npc.hold", params = { who = "attackers", place = { role = "leader" }, look = "qa_look" } },
              { kind = "npc.follow", params = { who = "leader", target = { ref = "dropped_case" } } },
              { kind = "npc.attack", params = { who = "attackers", target = { actor = true } } },
          },
          out = { next = "finish" } },
        { id = "finish", kind = "flow.end", params = { status = "completed" } },
    },
}
"""

CALLER = """return {
    nq    = 1,
    id    = "qa_caller",
    title = "QA caller",
    vars  = { began = false, ended = false, failed = false },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "spring" } },
        { id = "spring", kind = "behaviour.run",
          params = { asset = "qa_ambush", ref = "ambush",
                     roles = { leader = { story = "esc_wolf" } } },
          out = { started = "began", finished = "ended", aborted = "failed" } },
        { id = "began", kind = "flow.step",
          on_enter = { { kind = "var.set", params = { name = "began", value = true } } } },
        { id = "ended", kind = "flow.step",
          on_enter = { { kind = "var.set", params = { name = "ended", value = true } } },
          out = { next = "finish" } },
        { id = "failed", kind = "flow.step",
          on_enter = { { kind = "var.set", params = { name = "failed", value = true } } },
          out = { next = "finish" } },
        { id = "finish", kind = "flow.end", params = { status = "completed" } },
    },
}
"""

# a quest may not walk an NPC, and a behaviour graph may not speak to the player
QUEST_WITH_STEP = """return {
    nq = 1, id = "qa_bad_quest", title = "bad",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "walk" } },
        { id = "walk", kind = "npc.goto", params = { who = "nobody", place = { path = "qa_walk" } },
          out = { done = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

BEHAVIOUR_WITH_MONEY = """return {
    beh = 1, id = "qa_bad_behaviour", title = "bad",
    roles = { squad = { title = "Squad" } },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "pay" } },
        { id = "pay", kind = "flow.step",
          on_enter = { { kind = "money.give", params = { amount = 100 } },
                       { kind = "npc.release", params = { who = "squad" } } },
          out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

BEHAVIOUR_UNKNOWN_ROLE = """return {
    beh = 1, id = "qa_bad_role", title = "bad",
    roles = { squad = { title = "Squad" } },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "walk" } },
        { id = "walk", kind = "npc.goto", params = { who = "squad", place = { role = "ghosts" } },
          out = { done = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

BEHAVIOUR_SELF_CALL = """return {
    beh = 1, id = "qa_loop", title = "bad",
    roles = { squad = { title = "Squad" } },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "again" } },
        { id = "again", kind = "behaviour.run",
          params = { asset = "qa_loop", roles = { squad = { story = "esc_wolf" } } },
          out = { finished = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

# the header says quest, the extension says behaviour graph
QUEST_IN_BEHASSET = """return {
    nq = 1, id = "qa_liar", title = "liar",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

WITH_TYPE_KEY = """return {
    nq = 1, type = "scenario", id = "qa_typed", title = "typed",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""


def codes(problems) -> str:
    out = []
    for p in problems or []:
        out.append(p.get("code", "") if isinstance(p, dict) else str(p))
    return " ".join(out)


def errors(problems) -> str:
    """Only the codes that would stop a build; a W is advice, not a failure."""
    return " ".join(c for c in codes(problems).split() if c.startswith("E"))


PROJECT = [""]


def write(path: str, text: str) -> dict:
    """Writes a graph the way an agent does - a file - and answers what asset.check finds in it."""
    full = os.path.join(PROJECT[0], path.replace("/", os.sep))
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8", newline="") as f:
        f.write(text)
    return mcp("asset.check", path=path)


def read(path: str) -> str:
    with open(os.path.join(PROJECT[0], path.replace("/", os.sep)), encoding="utf-8", newline="") as f:
        return f.read()


def remove(path: str) -> None:
    full = os.path.join(PROJECT[0], path.replace("/", os.sep))
    if os.path.isfile(full):
        os.remove(full)


def run(project: str) -> None:
    PROJECT[0] = project
    ed = Editor()
    out_dir = tempfile.mkdtemp(prefix="grizzly_beh_")
    try:
        ed.start(project)

        # ------------------------------------------------------------------ A: the vocabulary
        ref = mcp("asset.reference", kind="behaviour")
        by_id = graph_ref.kinds(str(ref.get("text", "")))
        kinds = list(by_id.values())
        check("npc.goto" in by_id, "A the catalog carries the role steps", str(len(kinds)))
        run_kind = by_id.get("behaviour.run", {})
        check(bool(run_kind), "A and the call that runs a behaviour graph")
        check(run_kind.get("pins") == ["started", "finished", "aborted"],
              "A behaviour.run has three pins and no wait flag", json.dumps(run_kind.get("pins")))
        check("scenario.run" not in by_id, "A the old spelling is gone")
        goto = by_id.get("npc.goto", {})
        place = next((p for p in goto.get("params", []) if p.get("name") == "place"), {})
        check(place.get("type") == "destination", "A npc.goto takes a destination, not only a path",
              json.dumps(place))

        # ------------------------------------------------------------------ B: the asset
        made = mcp("file.create", kind="behaviour", path="npc")
        check(made.get("ok"), "B file.create makes a behaviour graph", json.dumps(made)[:300])
        made_text = read(made.get("path", "npc/NewBehaviour.behasset"))
        check(made_text.startswith("return {\n  beh = 1,\n") and "type" not in made_text.split("nodes")[0],
              "B the header first, no type key, no generated comment", made_text[:160])
        remove(made.get("path", "npc/NewBehaviour.behasset"))

        w = write("npc/qa_ambush.behasset", AMBUSH)
        check(w.get("ok"), "B a behaviour graph is written", json.dumps(w)[:300])
        check(errors(w.get("problems")) == "", "B with no errors - a ref it never creates is the caller's",
              codes(w.get("problems")))

        w2 = write("quests/qa_caller.nqasset", CALLER)
        check(w2.get("ok"), "B a quest that runs it is written", json.dumps(w2)[:300])
        check(errors(w2.get("problems")) == "", "B with no errors either", codes(w2.get("problems")))

        opened = mcp("ui.tab", action="open", kind="beh", path="npc/qa_ambush.behasset")
        check(opened.get("ok"), "B its tab opens", json.dumps(opened)[:200])
        got = mcp("graph.get", doc="beh:npc/qa_ambush.behasset")
        head = got.get("outline", "").split("\n")[0]
        check(head.startswith("behaviour qa_ambush ") and "roles:" in head and "quest" not in head and "active:" not in head,
              "B its outline says behaviour and names its roles", head[:160])
        saved = mcp("doc.save", doc="beh:npc/qa_ambush.behasset")
        check(saved.get("ok") and read("npc/qa_ambush.behasset") == AMBUSH,
              "B opened and saved unchanged, the file is byte for byte what was written", json.dumps(saved)[:200])
        mcp("ui.tab", action="close", doc="beh:npc/qa_ambush.behasset")

        look = mcp("game.lookup", type="behaviour_id")
        ids = [e.get("id") for e in look.get("entries", [])]
        check("qa_ambush" in ids and "qa_caller" not in ids, "B lookup behaviour_id offers graphs, not quests",
              json.dumps(ids))
        look_q = mcp("game.lookup", type="quest_id")
        qids = [e.get("id") for e in look_q.get("entries", [])]
        check("qa_caller" in qids and "qa_ambush" not in qids, "B and quest_id offers quests, not graphs",
              json.dumps(qids))
        roles = mcp("game.lookup", type="role", path="npc/qa_ambush.behasset")
        rnames = sorted(e.get("id") for e in roles.get("entries", []))
        check(rnames == ["attackers", "leader"], "B lookup role lists what the graph declares", json.dumps(rnames))

        # ------------------------------------------------------------------ C: what is refused
        b1 = write("quests/qa_bad_quest.nqasset", QUEST_WITH_STEP)
        check("E023" in codes(b1.get("problems")), "C a quest cannot walk an NPC", codes(b1.get("problems")))
        b2 = write("npc/qa_bad_behaviour.behasset", BEHAVIOUR_WITH_MONEY)
        check("E023" in codes(b2.get("problems")), "C a behaviour graph cannot pay the player",
              codes(b2.get("problems")))
        b3 = write("npc/qa_bad_role.behasset", BEHAVIOUR_UNKNOWN_ROLE)
        check("E024" in codes(b3.get("problems")), "C a destination naming no role is refused",
              codes(b3.get("problems")))
        b4 = write("npc/qa_loop.behasset", BEHAVIOUR_SELF_CALL)
        check("E113" in codes(b4.get("problems")), "C a graph may not run itself", codes(b4.get("problems")))
        b5 = write("npc/qa_liar.behasset", QUEST_IN_BEHASSET)
        check("E001" in codes(b5.get("problems")), "C a quest header inside a .behasset is refused",
              codes(b5.get("problems")))
        b6 = write("quests/qa_typed.nqasset", WITH_TYPE_KEY)
        c6 = codes(b6.get("problems")) + " " + json.dumps(b6)[:400]
        check((not b6.get("ok")) or "E003" in c6 or "type" in c6, "C a type key is refused", c6[:300])

        v = mcp("asset.check", path="npc/qa_ambush.behasset")
        check(v.get("ok") and v.get("counts", {}).get("error", 1) == 0, "C the good graph still checks clean",
              json.dumps(v)[:200])

        # ------------------------------------------------------------------ D: the build
        for bad in ("quests/qa_bad_quest.nqasset", "npc/qa_bad_behaviour.behasset", "npc/qa_bad_role.behasset",
                    "npc/qa_loop.behasset", "npc/qa_liar.behasset", "quests/qa_typed.nqasset"):
            remove(bad)

        built = mcp("mod.build", target="folder", path=out_dir)
        check(built.get("ok"), "D the module builds with a behaviour graph in it", json.dumps(built)[:300])
        manifest = mcp("mod.info").get("manifest", {})
        root = os.path.join(out_dir, "modules", manifest.get("id", ""))
        for rel in ("npc/qa_ambush.behasset", "quests/qa_caller.nqasset"):
            p = os.path.join(root, rel.replace("/", os.sep))
            check(os.path.isfile(p), "D %s ships" % rel, p)

        # ------------------------------------------------------------------ E: the browser
        mcp("content_browser_navigation", action="navigate", source="project", folder="npc")
        c = mcp("file.create", kind="behaviour")
        check(c.get("ok") and str(c.get("path", "")).lower().endswith(".behasset"),
              "E Create > NPC Behaviour makes a .behasset in the open folder", json.dumps(c)[:200])
        if c.get("path"):
            # F2 over MCP: the box holds the NAME - the extension is the kind and is not on offer
            box = mcp("content_browser_selection", action="rename", name=c["path"]).get("rename") or {}
            check(box.get("text") == "NewBehaviour" and box.get("select") == len("NewBehaviour"),
                  "E the inline rename box holds the name alone, all of it selected", json.dumps(box))
            mcp("content_browser_selection", action="clear")
            check(not mcp("content_browser_selection").get("rename"), "E clear closes the box without renaming")
            dotted = mcp("file.rename", path=c["path"], name="patrol.v2")
            check(dotted.get("ok") and os.path.isfile(os.path.join(project, "npc", "patrol.v2.behasset")),
                  "E a name with a dot in it is still only a name", json.dumps(dotted)[:200])
            turned = mcp("file.rename", path="npc/patrol.v2.behasset", name="patrol.nqasset")
            check(not turned.get("ok") and "kind" in str(turned.get("error", "")),
                  "E a rename cannot turn a behaviour graph into a quest", json.dumps(turned)[:200])
            c["path"] = "npc/patrol.v2.behasset"
            renamed = mcp("file.rename", path=c["path"], name="patrol_route")
            check(renamed.get("ok") and str(renamed.get("path", renamed.get("to", ""))).lower().endswith(".behasset")
                  or os.path.isfile(os.path.join(project, "npc", "patrol_route.behasset")),
                  "E renamed without an extension it stays a behaviour graph", json.dumps(renamed)[:200])
            mcp("file.delete", path="npc/patrol_route.behasset")

        # ------------------------------------------------------------------ G: the tab
        # a behaviour graph is its own document: its tab shows the roles where a quest keeps
        # its tasks, frames readable and never says "quest" (docs/UI_DESIGN.md section 6)
        v = mcp("graph.view", doc="beh:npc/qa_ambush.behasset", frame="all", graph_panel="show", details="show",
                select=["dig_in"], slot="enter:0")
        check(v.get("ok"), "G the behaviour graph opens in its tab", json.dumps(v)[:200])
        # a role's Title, Binding and Default sit under its folded More: open it to look at them
        mcp("graph.view", doc="beh:npc/qa_ambush.behasset", roles_more="show")
        mcp("ui.audit")
        mcp("ui.audit")
        v = mcp("graph.view", doc="beh:npc/qa_ambush.behasset")
        panes = v.get("panes") or {}
        check(v.get("title_px", 0) >= 11 and v.get("detail") != "block", "G its titles are readable after Frame All",
              json.dumps([v.get("title_px"), v.get("detail")]))
        check(all(panes.get(k) for k in ("graph", "canvas", "details", "node", "action")),
              "G the Graph panel, the canvas and the node with its action are drawn", json.dumps(panes))
        audit = mcp("ui.audit")
        said = [str(e.get("label", "")) + " " + str(e.get("help", ""))
                for key in ("controls", "truncated") for e in audit.get(key, [])
                if ("doc.host" in str(e.get("window", "")) or "toolbar" in str(e.get("window", "")))
                and "quest" in (str(e.get("label", "")) + str(e.get("help", "")) + str(e.get("why", ""))).lower()]
        check(not said, "G and its tab never says quest", json.dumps(said[:5]))
        labels = [str(e.get("label", "")) for e in audit.get("controls", []) if "doc.host" in str(e.get("window", ""))]
        check("Binding" in labels and "Remove the role" not in labels,
              "G a role's Group and Optional share one Binding row, and no bin sits beside its name", json.dumps(labels[:40]))
        mcp("graph.view", doc="beh:npc/qa_ambush.behasset", select="none", slot="none")

        # ------------------------------------------------------------------ F: clean up
        for rel in ("npc/qa_ambush.behasset", "quests/qa_caller.nqasset"):
            kind = "beh" if rel.endswith(".behasset") else "quest"
            mcp("ui.tab", action="close", doc="%s:%s" % (kind, rel), discard=True)
            d = mcp("file.delete", path=rel)
            check(d.get("ok"), "F %s is removed" % rel, json.dumps(d)[:160])

    finally:
        ed.close()
        qa_target.remove_tree(out_dir)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="grizzly_behproj_")
    copy = os.path.join(work, "QaBehaviours")
    qa_target.copy_project(src, copy, ignore=())
    os.makedirs(os.path.join(copy, "quests"), exist_ok=True)
    os.makedirs(os.path.join(copy, "npc"), exist_ok=True)
    print("running against a copy: " + copy, flush=True)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()

    try:
        run(copy)
    except Exception as exc:  # noqa: BLE001
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            qa_target.remove_tree(work)

    ok = sum(1 for good, _ in results if good)
    print(f"BEHAVIOURS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
