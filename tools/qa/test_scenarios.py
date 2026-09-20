#!/usr/bin/env python3
"""End-to-end check of NPC scenarios (NQ_ARCHITECTURE par. 19).

Drives the editor over MCP on a THROW-AWAY COPY of a project: writes a scenario
and a quest that runs it, checks that the halves of the catalog stay apart (a
quest cannot walk an NPC, a scenario cannot hand out a task), that a `who`
naming no role is refused, and that both assets survive a build and a canonical
rewrite unchanged.

    python tools/qa/test_scenarios.py --project D:\\XFinedProjects\\Test
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

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = ("127.0.0.1", 28016)

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
        exe = os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7
        self.proc = subprocess.Popen([exe, "-nodlg", "-project", project],
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
            time.sleep(1)
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


AMBUSH = """return {
    nq    = 1,
    type  = "scenario",
    id    = "qa_ambush",
    title = "QA ambush",
    roles = {
        leader    = { title = "Leader" },
        attackers = { title = "Attackers", many = true },
    },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "move_in" } },
        { id = "move_in", kind = "npc.goto",
          params = { who = { "leader", "attackers" }, path = "qa_walk", move = "run", arrive = true },
          out = { done = "dig_in" } },
        { id = "dig_in", kind = "flow.step",
          on_enter = {
              { kind = "npc.hold", params = { who = "attackers", path = "qa_walk", look = "qa_look" } },
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
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "spring" } },
        { id = "spring", kind = "scenario.run",
          params = { asset = "qa_ambush", wait_end = true, ref = "ambush",
                     roles = { leader = { story = "esc_wolf" } } },
          out = { done = "finish" } },
        { id = "finish", kind = "flow.end", params = { status = "completed" } },
    },
}
"""

# a quest may not walk an NPC, and a scenario may not hand out a task
QUEST_WITH_STEP = """return {
    nq = 1, id = "qa_bad_quest", title = "bad",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "walk" } },
        { id = "walk", kind = "npc.goto", params = { who = "nobody", path = "qa_walk" },
          out = { done = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

SCENARIO_WITH_TASK = """return {
    nq = 1, type = "scenario", id = "qa_bad_scenario", title = "bad",
    roles = { squad = { title = "Squad" } },
    tasks = { find_it = { title = "Find it", type = "additional" } },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "give" } },
        { id = "give", kind = "flow.step",
          on_enter = { { kind = "task.give", params = { name = "find_it" } } },
          out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

SCENARIO_UNKNOWN_ROLE = """return {
    nq = 1, type = "scenario", id = "qa_bad_role", title = "bad",
    roles = { squad = { title = "Squad" } },
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "walk" } },
        { id = "walk", kind = "npc.goto", params = { who = "ghosts", path = "qa_walk" },
          out = { done = "finish" } },
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


def write(path: str, text: str) -> dict:
    return mcp("quest_write", path=path, lua=text)


def run(project: str) -> None:
    ed = Editor()
    out_dir = tempfile.mkdtemp(prefix="xfined_scen_")
    try:
        ed.start(project)

        cat = mcp("quest_catalog")
        kinds = cat.get("kinds", [])
        by_id = {k.get("id"): k for k in kinds if isinstance(k, dict)}
        check("npc.goto" in by_id, "A the catalog carries the scenario steps", str(len(kinds)))
        check("scenario.run" in by_id, "A and the quest side that runs them")

        # ------------------------------------------------------------------ A: the assets
        w = write("quests/qa_ambush.nqasset", AMBUSH)
        check(w.get("ok"), "A a scenario is written", json.dumps(w)[:300])
        check(errors(w.get("problems")) == "", "A with no errors", codes(w.get("problems")))

        w2 = write("quests/qa_caller.nqasset", CALLER)
        check(w2.get("ok"), "A a quest that runs it is written", json.dumps(w2)[:300])
        check(errors(w2.get("problems")) == "", "A with no errors either", codes(w2.get("problems")))

        lst = mcp("quest_list")
        rows = {q.get("id"): q for q in lst.get("quests", []) if q.get("readable")}
        check(rows.get("qa_ambush", {}).get("type") == "scenario", "A the list calls it a scenario")
        check(rows.get("qa_caller", {}).get("type") == "quest", "A and the caller a quest")

        got = mcp("quest_get", path="quests/qa_ambush.nqasset")
        text = got.get("lua", "")
        check('type = "scenario"' in text, "A the type survives a canonical rewrite", text[:200])
        check("roles" in text and "attackers" in text, "A and so do the roles")
        again = write("quests/qa_ambush.nqasset", text)
        text2 = mcp("quest_get", path="quests/qa_ambush.nqasset").get("lua", "")
        check(again.get("ok") and text2 == text, "A write(read(x)) is the same file")

        # ------------------------------------------------------------------ B: the halves stay apart
        b1 = write("quests/qa_bad_quest.nqasset", QUEST_WITH_STEP)
        check("E023" in codes(b1.get("problems")), "B a quest cannot walk an NPC",
              codes(b1.get("problems")))
        b2 = write("quests/qa_bad_scenario.nqasset", SCENARIO_WITH_TASK)
        c2 = codes(b2.get("problems"))
        check("E023" in c2 or "E006" in c2, "B a scenario cannot hand out a task", c2)
        b3 = write("quests/qa_bad_role.nqasset", SCENARIO_UNKNOWN_ROLE)
        check("E024" in codes(b3.get("problems")), "B a who naming no role is refused",
              codes(b3.get("problems")))

        v = mcp("quest_validate", path="quests/qa_ambush.nqasset")
        check(v.get("ok") and v.get("errors", 1) == 0, "B the good scenario still validates",
              json.dumps(v)[:200])

        # ------------------------------------------------------------------ C: the build
        for bad in ("qa_bad_quest", "qa_bad_scenario", "qa_bad_role"):
            mcp("content_delete", path="quests/%s.nqasset" % bad)

        built = mcp("mod_export", target=out_dir)
        check(built.get("ok"), "C the module builds with a scenario in it", json.dumps(built)[:300])
        manifest = mcp("mod_manifest")
        root = os.path.join(out_dir, "modules", manifest.get("id", ""))
        for name in ("qa_ambush", "qa_caller"):
            p = os.path.join(root, "quests", name + ".nqasset")
            check(os.path.isfile(p), "C %s ships" % name, p)

        # ------------------------------------------------------------------ D: clean up
        for name in ("qa_ambush", "qa_caller"):
            d = mcp("content_delete", path="quests/%s.nqasset" % name)
            check(d.get("ok"), "D %s is removed" % name, json.dumps(d)[:160])

    finally:
        ed.close()
        shutil.rmtree(out_dir, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="xfined_scenproj_")
    copy = os.path.join(work, "QaScenarios")
    shutil.copytree(src, copy)
    os.makedirs(os.path.join(copy, "quests"), exist_ok=True)
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
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"SCENARIOS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
