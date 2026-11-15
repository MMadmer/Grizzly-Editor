#!/usr/bin/env python3
"""Assets are text files the editor follows (docs/ASSETS.md sections 3 and 4).

Drives the editor over MCP on a THROW-AWAY COPY of a project; every file is written with Python,
the way an agent's own file tools write it:

- A: asset.kinds lists the eleven kinds; asset.reference answers each of them (quests with the linked
  game's node kinds, the module manifest's format by any of its names);
- B: asset.check puts problems at their lines and columns, fail_on=error makes it fail, a file
  of a newer version is refused with the sentence;
- C: an open quest follows its file: a comment, an unknown key and a new title written outside
  show in the tab, and the file is still exactly what was written;
- D: an unsaved edit in the tab (graph.edit) and another value changed outside: Save writes
  both, with the comment and the unknown key;
- E: both sides change the same value: the tab keeps its own and lists the conflict;
  asset.take_theirs takes the file's;
- F: a file that stops reading leaves the last good state and says why with the line; it comes
  back by itself;
- G: Grizzly-Cmd.exe run asset.check exits 1 on a broken file, 0 on a good one.

    python tools/qa/test_assets.py --project <project> [--only A,B]
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

import qa
from qa import check, cmd


Q = "quests/qa_assets.nqasset"

def write(project: str, rel: str, text: str) -> None:
    path = os.path.join(project, rel.replace("/", os.sep))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def read(project: str, rel: str) -> str:
    with open(os.path.join(project, rel.replace("/", os.sep)), encoding="utf-8", newline="") as f:
        return f.read()


def line_of(text: str, needle: str) -> int:
    for i, line in enumerate(text.splitlines(), 1):
        if needle in line:
            return i
    return 0


def until(what, seconds: float = 10.0):
    """Asks `what` until it answers something true (the watcher waits 100 ms for quiet)."""
    deadline = time.monotonic() + seconds
    got = what()
    while not got and time.monotonic() < deadline:
        time.sleep(0.1)
        got = what()
    return got


QUEST = """-- The QA quest of the asset suite: a comment the editor keeps.
return {
  nq = 1,
  id = "qa_assets",
  title = "Assets",
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "talk" } },

    -- the talk, written by hand
    { id = "talk", kind = "flow.step", comment = "first",
      out = { next = "finish" } },

    { id = "finish", kind = "flow.end", params = { status = "completed" } },
  },
}
"""


def phase_a() -> None:
    k = cmd("asset.kinds")
    exts = {x.get("extension") for x in k.get("kinds", [])}
    check({".nqasset", ".behasset", ".npcasset", ".pdaasset", "mod.ltx"} <= exts, "A asset.kinds lists every kind",
          json.dumps(k)[:300])
    q = cmd("asset.reference", kind="quest")
    text = q.get("text", "")
    check(q.get("ok") and "node kinds of the linked game" in text and "`trigger.start`" in text,
          "A asset.reference kind=quest is the format with the game's node kinds", text[-300:])
    for kind, needle in (("character", "## The keys"), ("pda", "# PDA interfaces"), ("beh", "behaviour")):
        r = cmd("asset.reference", kind=kind)
        check(r.get("ok") and needle in r.get("text", ""), "A asset.reference kind=%s answers" % kind,
              json.dumps(r)[:200])
    bad = cmd("asset.reference", kind="nothing")
    check(bad.get("code") == "not_found" and "module manifest" in bad.get("error", ""),
          "A an unknown kind is not_found and names the manifest among the kinds", json.dumps(bad)[:200])
    # eleven kinds - the model, the texture and the sound among them - and every one has a reference that answers
    names = [x.get("name") for x in k.get("kinds", [])]
    check(len(names) == 11 and {"module manifest", "model", "texture", "sound"} <= set(names),
          "A asset.kinds lists eleven kinds, the manifest and the media kinds among them", names)
    for name in names:
        r = cmd("asset.reference", kind=name)
        check(r.get("ok") and len(r.get("text", "")) > 200, "A asset.reference kind=%s answers a document" % name,
              json.dumps(r)[:200])
    manifest = next((x for x in k.get("kinds", []) if x.get("extension") == "mod.ltx"), {})
    check("asset.reference kind=manifest" in manifest.get("reference", ""),
          "A asset.kinds names the reference of the manifest", json.dumps(manifest)[:300])
    for word in ("manifest", "mod.ltx", "module manifest"):
        m = cmd("asset.reference", kind=word)
        mtext = m.get("text", "")
        check(m.get("ok") and m.get("kind") == "module manifest"
              and all(w in mtext for w in ("[module]", "target = default | existing | new | any", "[provides_mode]",
                                           "[world]", "[screen.<id>]", "mod.set set=", "Example", "id          = night_watch")),
              "A asset.reference kind=%s answers the manifest's format with its example" % word, mtext[:300])
    modes = mtext.find("Modes of the linked game")
    check(mtext.find("Example") < modes < mtext.find("Left as written"),
          "A the linked game's modes come after the example, before the closing note", mtext[-600:])
    # a game is linked: its modes are listed, one a line, by the names a player reads; none: the page says so
    section = [l for l in mtext[modes:mtext.find("Left as written")].splitlines()[1:] if l.strip()]
    linked = bool(section) and not section[0].lstrip().startswith(("No game is linked", "The linked game has no mode"))
    check(section and (not linked or all(l.startswith("  ") and len(l.strip()) > 0 for l in section)),
          "A the modes section lists the game's modes or says why it has none", section[:6])
    if linked:
        modes_of = cmd("game.lookup", type="mode")
        want = [e.get("id") for e in modes_of.get("entries", [])]
        check(want and all(any(l.strip().split(" ")[0] == i for l in section) for i in want),
              "A every mode game.lookup type=mode knows is in the page", (want[:5], section[:5]))


def phase_b(project: str) -> None:
    text = QUEST.replace('    { id = "finish", kind = "flow.end"',
                         '    { id = "nowhere", kind = "qa.no_such_kind" },\n    { id = "finish", kind = "flow.end"')
    text = text.replace('  title = "Assets",', '  title = "Assets",\n  future_key = { 1 },')
    write(project, "quests/qa_check.nqasset", text)
    r = cmd("asset.check", path="quests/qa_check.nqasset")
    check(r.get("ok") and r.get("files") == 1, "B asset.check of a file answers", json.dumps(r)[:300])
    probs = r.get("problems", [])
    kind_line = line_of(text, "qa.no_such_kind")
    on_kind = [p for p in probs if p.get("line") == kind_line and p.get("severity") == "error"]
    check(bool(on_kind) and on_kind[0].get("column") == 5 and on_kind[0].get("file") == "quests/qa_check.nqasset",
          "B the unknown kind's error stands at its node: line %d, column 5" % kind_line, json.dumps(probs)[:600])
    unknown = [p for p in probs if p.get("code") == "unknown_key"]
    check(bool(unknown) and unknown[0].get("line") == line_of(text, "future_key") and unknown[0].get("severity") == "note",
          "B the unknown key is a note at its line, kept", json.dumps(unknown)[:300])
    check(all("text" in p and "code" in p for p in probs), "B every problem has its code and text", json.dumps(probs)[:300])
    f = cmd("asset.check", path="quests/qa_check.nqasset", fail_on="error")
    check(f.get("ok") is False and f.get("code") == "failed" and f.get("counts", {}).get("error", 0) >= 1,
          "B fail_on=error makes the command fail with the problems", json.dumps(f)[:300])
    write(project, "quests/qa_newer.nqasset", "return {\n  nq = 99,\n  id = \"qa_newer\",\n  nodes = {},\n}\n")
    n = cmd("asset.check", path="quests/qa_newer.nqasset")
    newer = [p for p in n.get("problems", []) if p.get("code") == "newer_version"]
    check(bool(newer) and "newer Grizzly Editor" in newer[0].get("text", "") and newer[0].get("line") == 2,
          "B a newer version is refused with the sentence, at its header", json.dumps(n)[:300])
    o = cmd("graph.get", doc="quests/qa_newer.nqasset")
    check(o.get("ok") is False and "newer Grizzly Editor" in o.get("error", ""), "B and opens nothing",
          json.dumps(o)[:300])
    for rel in ("quests/qa_check.nqasset", "quests/qa_newer.nqasset"):
        os.remove(os.path.join(project, rel.replace("/", os.sep)))


def summary() -> dict:
    return cmd("graph.get", doc=Q)


def phase_c(project: str) -> None:
    write(project, Q, QUEST)
    o = summary()
    check(o.get("ok") and o.get("title") == "Assets" and o.get("dirty") is False, "C the quest opens", json.dumps(o)[:300])
    outside = QUEST.replace('  title = "Assets",', '  title = "Assets, changed outside", -- a new comment\n  future_key = true,')
    write(project, Q, outside)
    got = until(lambda: summary().get("title") == "Assets, changed outside")
    check(bool(got), "C the tab shows what was written outside", json.dumps(summary())[:300])
    check(summary().get("dirty") is False, "C and stays without unsaved edits")
    check(read(project, Q) == outside, "C the file is what was written, byte for byte")


def phase_d(project: str) -> None:
    before = read(project, Q)
    a = cmd("graph.edit", doc=Q, ops='{ op = "set", node = "talk", patch = { comment = "mine" } }')
    check(a.get("ok") and a.get("dirty") is True, "D an unsaved edit in the tab", json.dumps(a)[:300])
    outside = before.replace('params = { status = "completed" }', 'params = { status = "failed" }')
    # whether the watcher delivered it yet or not: Save takes in what the file holds first
    write(project, Q, outside)
    s = cmd("doc.save", doc="quest:" + Q)
    check(s.get("ok"), "D Save", json.dumps(s)[:300])
    text = read(project, Q)
    check('comment = "mine"' in text, "D the tab's edit is in the file", text)
    check('status = "failed"' in text, "D the outside change is in the file too", text)
    check("-- a new comment" in text and "future_key = true" in text and "-- the talk, written by hand" in text,
          "D the comments and the unknown key stay", text)
    lines_before, lines_after = before.splitlines(), text.splitlines()
    changed = [l for l in lines_after if l not in lines_before]
    check(len(changed) == 2, "D only the two changed values' lines differ from before", json.dumps(changed))
    c = cmd("asset.check", path=Q)
    talk_line = line_of(text, 'id = "talk"')
    check(c.get("ok") and all(p.get("line", 0) > 0 for p in c.get("problems", [])),
          "D asset.check of the saved file answers with positions (the talk is at line %d)" % talk_line,
          json.dumps(c)[:300])


def phase_e(project: str) -> None:
    before = read(project, Q)
    cmd("graph.edit", doc=Q, ops='{ op = "set", node = "talk", patch = { comment = "here" } }')
    write(project, Q, before.replace('comment = "mine"', 'comment = "there"'))
    got = until(lambda: cmd("graph.get", doc=Q).get("conflicts"))
    check(bool(got) and got[0].get("path") == "nodes[id=talk].comment" and got[0].get("mine") == "here"
          and got[0].get("theirs") == "there", "E both sides changed the comment: the tab keeps its own and lists it",
          json.dumps(got)[:300])
    t = cmd("asset.take_theirs", path=Q, index=0)
    check(t.get("ok") and t.get("conflicts_left") == 0, "E asset.take_theirs takes the file's value", json.dumps(t)[:300])
    cmd("doc.save", doc="quest:" + Q)
    check('comment = "there"' in read(project, Q), "E and Save writes it", read(project, Q))


def phase_f(project: str) -> None:
    good = read(project, Q)
    broken = good.replace('  title = ', '  title = = ', 1)
    write(project, Q, broken)
    got = until(lambda: cmd("graph.get", doc=Q).get("disk_error"))
    line = line_of(broken, "title = =")
    check(bool(got) and ("line %d:" % line) in got, "F a file that stops reading says why, with its line and column",
          str(got))
    check(cmd("graph.get", doc=Q).get("nodes") == 3, "F the tab keeps the last good state")
    write(project, Q, good)
    back = until(lambda: "disk_error" not in cmd("graph.get", doc=Q))
    check(bool(back), "F it comes back by itself once the file reads", json.dumps(cmd("graph.get", doc=Q))[:300])


def phase_g(project: str) -> None:
    cmd = os.path.join(os.path.dirname(qa.exe()), "Grizzly-Cmd.exe")
    if not os.path.isfile(cmd):
        check(False, "G Grizzly-Cmd.exe is beside the editor", cmd)
        return
    home = tempfile.mkdtemp(prefix="grizzly_cmd_home_")
    write(project, "quests/qa_broken.nqasset", "return {\n  nq = 1,\n  id = ,\n}\n")
    try:
        run = subprocess.run([cmd, "run", "asset.check", "path=quests/qa_broken.nqasset", "fail_on=error",
                              "-project", project, "-home", home, "-json"],
                             capture_output=True, text=True, timeout=180)
        check(run.returncode == 1, "G Grizzly-Cmd.exe run asset.check exits 1 on a broken file",
              "%s %s %s" % (run.returncode, run.stdout[-300:], run.stderr[-300:]))
        check("\"line\":3" in run.stdout.replace(" ", ""), "G and its answer names the line", run.stdout[-300:])
        os.remove(os.path.join(project, "quests", "qa_broken.nqasset"))
        ok = subprocess.run([cmd, "run", "asset.check", "path=" + Q, "fail_on=error", "-project", project,
                             "-home", home], capture_output=True, text=True, timeout=180)
        check(ok.returncode == 0, "G and 0 on a file without errors", "%s %s" % (ok.returncode, ok.stderr[-300:]))
    finally:
        gone(home)
        shutil.rmtree(home, ignore_errors=True)


def gone(home: str, seconds: float = 10.0) -> None:
    """Waits for the editors a commandlet ran on `home` to have ended (its windowless twin ends
    with it, a moment after)."""
    probe = ("Get-CimInstance Win32_Process -Filter \"Name='Grizzly.exe'\" | "
             "Where-Object { $_.CommandLine -like '*%s*' } | ForEach-Object { $_.ProcessId }" % home)
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        out = subprocess.run(["powershell", "-NoProfile", "-Command", probe], capture_output=True, text=True).stdout
        if not out.strip():
            return
        time.sleep(0.2)
    check(False, "G the commandlet's editor ended with it", out.strip())


def run(ctx: qa.Run) -> None:
    project = ctx.project
    only = {p.strip().upper() for p in ctx.only.split(",") if p.strip()}
    os.makedirs(os.path.join(project, "quests"), exist_ok=True)
    ed = qa.Editor(project).start()
    try:
        wanted = lambda p: not only or p in only  # noqa: E731
        if wanted("A"):
            phase_a()
        if wanted("B"):
            phase_b(project)
        if wanted("C") or wanted("D") or wanted("E") or wanted("F"):
            phase_c(project)
        if wanted("D") or wanted("E") or wanted("F"):
            phase_d(project)
        if wanted("E"):
            phase_e(project)
        if wanted("F"):
            phase_f(project)
        cmd("ui.tab", action="close", doc="quest:" + Q, discard=True)
    finally:
        ed.close()
    if not only or "G" in only:
        phase_g(project)


def main() -> int:
    return qa.suite("ASSETS", run, doc=__doc__, trimmed=True, phases="ABCDEFG")


if __name__ == "__main__":
    sys.exit(main())
