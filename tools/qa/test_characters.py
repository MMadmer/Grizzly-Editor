#!/usr/bin/env python3
"""End-to-end check of the module's own characters (EditorCharacters).

Drives the editor over MCP on a THROW-AWAY COPY of a project: writes characters as files (the
way an agent does), checks them with asset.check, builds the module into a temporary folder and
verifies that the assets ship as they are and nothing is made of them: what the game makes of a
character is the game repo's Run-ScreensQa.ps1 -Part Descriptions. Phase U opens the
Characters tab on a character over MCP (characters.view) and wants its form drawn
whole, with nothing escaping its window.

    python tools/qa/test_characters.py --project <project>
"""
from __future__ import annotations

import json
import os
import re
import sys
import tempfile
import time

import qa
from qa import check, cmd


COURIER = """return {
    npc       = 1,
    id        = "qa_courier",
    name      = { rus = "Kurier", eng = "Courier" },
    bio       = "QA character",
    community = "stalker",
    visual    = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
    icon      = "ui_inGame2_neutral_2",
    rank      = 300,
    money     = { min = 500, max = 1500 },
    supplies  = {
        { section = "wpn_ak74u", count = 1 },
        { section = "ammo_5.45x39_fmj", count = 3 },
        { section = "medkit" },
    },
}
"""

GUARD = """return {
    npc       = 1,
    id        = "qa_guard",
    name      = "Guard",
    community = "dolg",
    visual    = "actors\\\\stalker_dolg\\\\stalker_dolg_1",
    squad     = false,
}
"""

BROKEN = """return {
    npc       = 1,
    id        = "QA Bad Id",
    community = "stalker",
}
"""


KIT = """return {
    npc      = 1,
    id       = "qa_kit",
    abstract = true,
    community = "stalker",
    rank     = 250,
    supplies = {
        { section = "device_torch" },
        { section = "wpn_knife" },
        { section = "bandage", count = 2 },
        { section = "medkit", count = 2 },
        { section = "bread" },
    },
}
"""

GUARD_KIT = """return {
    npc      = 1,
    id       = "qa_kitguard",
    base     = "qa_kit",
    name     = "Guard",
    visual   = "actors\\\\stalker_dolg\\\\stalker_dolg_1",
    community = "dolg",
    supplies = {
        { section = "wpn_ak74u" },
        { section = "medkit", count = 4 },
        { section = "bread", count = 0 },
    },
}
"""

NO_BASE = """return {
    npc    = 1,
    id     = "qa_orphan",
    base   = "qa_nothing",
    name   = "Orphan",
    visual = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
}
"""

LOOP_A = """return {
    npc = 1, id = "qa_loop_a", base = "qa_loop_b", name = "A",
    visual = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
}
"""

LOOP_B = """return {
    npc = 1, id = "qa_loop_b", base = "qa_loop_a", name = "B",
    visual = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
}
"""


QA_TPL = """return {
  npc      = 1,
  id       = "qa_tpl",
  abstract = true,
}
"""

QA_RUNNER = """return {
  npc       = 1,
  id        = "qa_runner",
  base      = "qa_tpl",
  name      = "Runner",
  community = "stalker",
  visual    = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
}
"""

SPAWNER = """return {
    nq = 1, id = "qa_spawner", title = "QA spawner",
    nodes = {
        { id = "start", kind = "trigger.start",
          on_enter = {
              { kind = "spawn.npc", params = {
                    place = { smart = "esc_smart_terrain_2_12" },
                    npcs  = {
                        { character = "qa_courier", ref = "courier" },
                        { character = "qa_runner", ref = "guard", unique = false },
                        { section = "sim_default_stalker_1", ref = "extra", unique = false, dead = true },
                    },
              } },
              { kind = "npc.set_name", params = { npc = { ref = "courier" }, name = "Tester" } },
          },
          out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

SPAWNER_BAD = """return {
    nq = 1, id = "qa_spawner_bad", title = "QA spawner, wrong",
    nodes = {
        { id = "start", kind = "trigger.start",
          on_enter = {
              { kind = "spawn.npc", params = {
                    place = { smart = "esc_smart_terrain_2_12" },
                    npcs  = {
                        { character = "qa_nobody" },
                        { character = "qa_tpl" },
                        { character = "qa_courier", section = "stalker" },
                        { ref = "lonely" },
                        { character = "qa_courier", uniqe = true },
                    },
              } },
          },
          out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""


PROJECT = [""]


def char_path(cid: str) -> str:
    return "characters/%s.npcasset" % cid


def text_id(text: str) -> str:
    m = re.search(r'\bid\s*=\s*"([^"]*)"', text)
    return m.group(1) if m else ""


def problem_lines(answer: dict) -> list[str]:
    """asset.check's errors and warnings as `<code> <file>: <text>` lines."""
    return ["%s %s: %s" % (p.get("code"), p.get("file", ""), p.get("text", ""))
            for p in answer.get("problems", []) if p.get("severity") in ("error", "warning", "required")]


def char_write(text: str) -> dict:
    """A character written as a file, and what asset.check says of it."""
    rel = char_path(text_id(text))
    full = os.path.join(PROJECT[0], rel.replace("/", os.sep))
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as f:
        f.write(text)
    r = cmd("asset.check", path=rel)
    return {"ok": r.get("ok"), "file": rel, "problems": problem_lines(r), "answer": r}


def char_delete(cid: str) -> bool:
    full = os.path.join(PROJECT[0], char_path(cid).replace("/", os.sep))
    if os.path.isfile(full):
        os.remove(full)
        return True
    return False


def char_ids() -> list[str]:
    """The project's characters as the editor knows them (the pickers' list)."""
    return sorted(e.get("id") for e in cmd("game.lookup", type="character_id", limit=500).get("entries", []))


def until(what, seconds: float = 10.0):
    """Asks `what` until it answers something true: the project's watcher takes a moment."""
    deadline = time.monotonic() + seconds
    got = what()
    while not got and time.monotonic() < deadline:
        time.sleep(0.1)
        got = what()
    return got


def write_quest(rel: str, text: str) -> dict:
    full = os.path.join(PROJECT[0], rel.replace("/", os.sep))
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8") as f:
        f.write(text)
    return cmd("asset.check", path=rel)


def remove(rel: str) -> None:
    full = os.path.join(PROJECT[0], rel.replace("/", os.sep))
    if os.path.isfile(full):
        os.remove(full)


def run_spawn_checks(module, out_dir):
    """spawn.npc: the character is named by its asset id, and a ref per NPC."""
    # its own cast: a template and a character that starts from it
    char_write(QA_TPL)
    char_write(QA_RUNNER)
    until(lambda: "qa_runner" in char_ids())
    import graph_ref
    by_id = graph_ref.kinds(str(cmd("asset.reference", kind="quest").get("text", "")))
    kind = by_id.get("spawn.npc", {})
    check(bool(kind), "F the catalog carries spawn.npc")
    rows = [r.get("name") for r in kind.get("rows", [])] if isinstance(kind.get("rows"), list) else []
    check("character" in rows and "unique" in rows and "ref" in rows,
          "F its rows name a character, a ref and unique", json.dumps(kind)[:300])
    # on for a character, off for a section: a default that follows the row is not a catalog default
    unique = next((r for r in kind.get("rows", []) if r.get("name") == "unique"), {})
    check("default" not in unique, "F unique declares no flat default - it follows what the row names", json.dumps(unique))

    look = cmd("game.lookup", type="character_id")
    entries = {e.get("id"): e for e in look.get("entries", [])}
    check("qa_courier" in entries and "qa_runner" in entries, "F lookup character_id lists the project's characters",
          json.dumps(sorted(entries)))
    check("template" in str(entries.get("qa_tpl", {}).get("extra", "")), "F and says which one is a template",
          json.dumps(entries.get("qa_tpl")))

    w = write_quest("quests/qa_spawner.nqasset", SPAWNER)
    errs = " ".join(p.get("code", "") for p in w.get("problems", []) if str(p.get("code", "")).startswith("E"))
    check(w.get("ok") and errs == "", "F a quest that spawns characters by their asset id validates", errs or json.dumps(w)[:200])
    refs = cmd("game.lookup", type="ref_name", path="quests/qa_spawner.nqasset")
    names = sorted(e.get("id") for e in refs.get("entries", []))
    check(names == ["courier", "extra", "guard"], "F every row's ref is a ref of the graph", json.dumps(names))
    outline = str(cmd("graph.get", doc="quests/qa_spawner.nqasset").get("outline", ""))
    check("npcs={qa_courier" in outline, "F the outline reads a row as who it creates, not as the ref it keeps", outline[:300])

    b = write_quest("quests/qa_spawner_bad.nqasset", SPAWNER_BAD)
    text = " | ".join("%s %s" % (p.get("code"), p.get("text", "")) for p in b.get("problems", []))
    check("not a character of this project" in text, "F a character that does not exist is refused", text[:300])
    check("template" in text, "F a template cannot be spawned, and the message says why", text[:300])
    check("exactly one" in text, "F a row names a character OR a section", text[:300])
    check("unknown key 'uniqe'" in text, "F a misspelt key is an error, not a silent no-op", text[:300])
    remove("quests/qa_spawner_bad.nqasset")

    built = cmd("mod.build", target="folder", path=out_dir)
    check(built.get("ok"), "F the module builds with the spawner in it", json.dumps(built)[:200])
    shipped = os.path.join(out_dir, "modules", module, "quests", "qa_spawner.nqasset")
    check(os.path.isfile(shipped), "F and the quest ships", shipped)
    # the game makes <module>_<id> of the asset, and the asset ships with the quest that names it
    asset = os.path.join(out_dir, "modules", module, "characters", "qa_courier.npcasset")
    check(os.path.isfile(asset), "F and so does the character it names", asset)

    # deleting the character turns the same quest into an error - no restart, no refresh
    char_delete("qa_runner")
    until(lambda: "qa_runner" not in char_ids())
    v = cmd("asset.check", path="quests/qa_spawner.nqasset")
    text = " | ".join("%s %s" % (p.get("code"), p.get("text", "")) for p in v.get("problems", []))
    check("qa_runner" in text and "not a character" in text, "F a deleted character is an error in the quest that names it", text[:300])

    char_delete("qa_tpl")
    remove("quests/qa_spawner.nqasset")


def run_base_checks(module, out_dir):
    """Inheritance: a shared kit written once, and what refuses to inherit."""
    k = char_write(KIT)
    check(k.get("ok"), "E a template is written", json.dumps(k)[:200])
    look = until(lambda: next((e for e in cmd("game.lookup", type="character_id", limit=500).get("entries", [])
                               if e.get("id") == "qa_kit"), None))
    check("template" in str((look or {}).get("extra", "")), "E and the editor knows it is one", json.dumps(look))
    check(k.get("problems") == [], "E a template needs no name and no model",
          json.dumps(k.get("problems")))

    g = char_write(GUARD_KIT)
    check(g.get("ok"), "E a character inherits from it", json.dumps(g)[:200])
    until(lambda: "qa_kitguard" in char_ids())
    g = char_write(GUARD_KIT)
    check(g.get("problems") == [], "E with no problems: its name and model come from the base",
          json.dumps(g.get("problems")))

    built = cmd("mod.build", target="folder", path=out_dir)
    check(built.get("ok"), "E the module builds", json.dumps(built)[:200])
    root = os.path.join(out_dir, "modules", module)
    for name in ("qa_kit", "qa_kitguard"):
        check(os.path.isfile(os.path.join(root, "characters", name + ".npcasset")),
              "E %s ships as its asset: the game does the inheriting" % name)

    char_write(NO_BASE)
    until(lambda: "qa_orphan" in char_ids())
    o = char_write(NO_BASE)
    check("E111" in " ".join(o.get("problems", [])), "E a base that does not exist is reported",
          json.dumps(o.get("problems")))
    char_write(LOOP_A)
    char_write(LOOP_B)
    loops = until(lambda: "E112" in " ".join(problem_lines(cmd("asset.check", path="characters"))))
    check(bool(loops), "E a base that comes back to itself is reported",
          json.dumps(problem_lines(cmd("asset.check", path="characters")))[:300])

    for gone in ("qa_orphan", "qa_loop_a", "qa_loop_b", "qa_kitguard", "qa_kit"):
        char_delete(gone)


def run_ui_checks():
    """The Characters tab: opened on a character over MCP, looked at, and nothing of it leaves its window."""
    v = cmd("characters.view")
    check(v.get("ok") and v.get("tab") == "characters", "U characters.view answers", json.dumps(v)[:300])
    v = cmd("characters.view", select="qa_courier")
    check(v.get("ok") and v.get("active") and v.get("selected") == "qa_courier", "U it opens the tab on a character",
          json.dumps(v)[:300])
    check(v.get("dirty") is False and v.get("problems") == [], "U with nothing unsaved and no problems", json.dumps(v)[:300])
    check(v.get("count") == 2 and v.get("file", "").endswith("qa_courier.npcasset"), "U the list has both, the form its file",
          json.dumps(v)[:300])
    check(cmd("ui.list").get("active_tab") == "characters", "U Characters is the active tab")
    # the model beside the form is the game's own visual: the viewer is told it is the game's (a typed source, not a
    # word it does not know) and shows the model, not an error state
    def viewer() -> dict:
        return ((cmd("characters.view").get("model") or {}).get("viewer")) or {}

    shown = qa.until(lambda: viewer().get("kind") or viewer().get("error"), 40)
    m = cmd("characters.view").get("model") or {}
    check(m.get("source") == "game", "U the embedded model is looked for in the linked game", json.dumps(m)[:300])
    check(viewer().get("error", "x") == "" and viewer().get("kind") and viewer().get("tris", 0) > 0,
          "U the embedded model is shown for a game model, not an error state", json.dumps(m)[:300])
    check(not cmd("ui.audit", id="characters").get("errors"), "U the tab shows no error state", qa.short(cmd("ui.audit", id="characters").get("errors")))
    audit = cmd("ui.audit", id="characters")
    check(audit.get("ok") and not audit.get("escaped"), "U nothing escapes the Characters tab",
          json.dumps(audit.get("escaped"))[:400])
    labels = {c.get("label") for c in audit.get("controls", [])}
    for want in ("Identity", "Name and Bio"):
        check(want in labels, "U the form draws " + want, json.dumps(sorted(x for x in labels if x))[:400])
    toolbar = {c.get("label") for c in cmd("ui.audit", id="##toolbar").get("controls", [])}
    check({"Save", "Revert", "Delete..."} <= toolbar, "U its toolbar holds Save, Revert and Delete...",
          json.dumps(sorted(x for x in toolbar if x))[:400])
    frame = cmd("shot.window")
    check(frame.get("ok"), "U the tab is drawn with its preview column", json.dumps({k: frame.get(k) for k in ("width", "height")}))
    # what is below the fold is drawn once the form scrolls to it
    scrolled = cmd("ui.scroll", id="characters", to="bottom")
    check(scrolled.get("ok"), "U the form scrolls", json.dumps(scrolled)[:300])
    labels = {c.get("label") for c in cmd("ui.audit", id="characters").get("controls", [])}
    for want in ("Starting Kit", "Add Item"):
        check(want in labels, "U the form draws " + want, json.dumps(sorted(x for x in labels if x))[:400])
    v = cmd("characters.view", path="characters/qa_guard.npcasset")
    check(v.get("ok") and v.get("selected") == "qa_guard", "U a character is found by its path too", json.dumps(v)[:300])
    gone = cmd("characters.view", select="qa_nobody")
    check(not gone.get("ok") and "qa_nobody" in gone.get("error", ""), "U one that does not exist is refused by name",
          json.dumps(gone))
    closed = cmd("ui.tab", action="close", doc="characters")
    check(closed.get("ok"), "U the tab closes when nothing is unsaved", json.dumps(closed)[:300])
    check(cmd("characters.view").get("open") is False, "U and characters.view says it is closed")


def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("cp1251", "replace")


def run(ctx: qa.Run) -> None:
    project = ctx.project
    ed = qa.Editor(project)
    out_dir = tempfile.mkdtemp(prefix="grizzly_chars_")
    try:
        ed.start()

        manifest = cmd("mod.info").get("manifest", {})
        module = manifest.get("id", "")
        check(bool(module), "A the project has a module id", json.dumps(manifest)[:200])

        # the checks count characters: an author's
        # project may bring characters of its own, so the copy starts without them
        PROJECT[0] = project
        own = [os.path.join(b, f) for b, _, fs in os.walk(project) for f in fs if f.lower().endswith(".npcasset")]
        for path in own:
            os.remove(path)
        check(until(lambda: char_ids() == []) is not None and char_ids() == [], "A the copy starts without characters",
              str(char_ids()))

        # ---------------------------------------------------------------- A: write
        w = char_write(COURIER)
        check(w.get("ok"), "A a character is written", json.dumps(w)[:200])
        check(w.get("file") == "characters/qa_courier.npcasset", "A it is the file of its id")
        check(w.get("problems") == [], "A and with no problems", json.dumps(w.get("problems")))

        w2 = char_write(GUARD)
        check(w2.get("ok"), "A a second character is written", json.dumps(w2)[:200])

        ids = until(lambda: char_ids() == ["qa_courier", "qa_guard"]) and char_ids()
        check(ids == ["qa_courier", "qa_guard"], "A both are known to the editor", str(char_ids()))

        ref = cmd("asset.reference", kind="character")
        check(ref.get("ok") and "| `supplies` |" in ref.get("text", ""), "A asset.reference kind=character lists the keys",
              json.dumps(ref)[:200])

        # ---------------------------------------------------------------- B: refusals
        bad = char_write(BROKEN)
        problems = " ".join(bad.get("problems", []))
        check(bad.get("ok") and "E102" in problems, "B an id with spaces is reported", problems)
        expr = char_write('return { npc = 1, id = "qa_expr", name = "a" .. "b", visual = "x" }')
        check("not data" in json.dumps(expr), "B what only Lua could work out is refused: the game runs nothing in the file", json.dumps(expr)[:300])
        check("E103" in problems, "B and so is a missing name", problems)
        check("E104" in problems, "B and a missing visual", problems)
        char_delete("QA Bad Id")
        char_delete("qa_expr")

        # a community is picked from the linked game's list in the window; the text path
        # can still name one the game has never heard of, and the engine refuses those
        odd = char_write(COURIER.replace('"qa_courier"', '"qa_martian"')
                         .replace('community = "stalker"', 'community = "martians"'))
        odd_problems = " ".join(odd.get("problems", []))
        if cmd("game.lookup", type="community", limit=5).get("entries"):
            check("W114" in odd_problems and "martians" in odd_problems,
                  "B a community the linked game does not list is a warning that names it", odd_problems)
            check("W114" not in " ".join(w.get("problems", [])), "B and one it does list is not")
        char_delete("qa_martian")

        # the window picks portraits, dialogs and items from lists; the text path can name
        # anything, and what the linked game does not have is said - the old SoC portrait id
        # is the classic: no picture in Dead Air Refined
        odd = char_write(COURIER.replace('"qa_courier"', '"qa_oddlooks"')
                         .replace('"ui_inGame2_neutral_2"', '"ui_npc_u_stalker_neutral_1"')
                         .replace('rank      = 300,', 'rank      = 300,\n    start_dialog = "qa_no_such_dialog",')
                         .replace('supplies  = {', 'supplies  = {\n        { section = "qa_no_such_item" },'))
        odd_problems = " ".join(odd.get("problems", []))
        if cmd("game.lookup", type="character_icon", limit=5).get("entries"):
            check("W115" in odd_problems, "B a portrait the linked game does not have is a warning", odd_problems)
            check("W116" in odd_problems, "B and so is a dialog", odd_problems)
            check("W117" in odd_problems and "qa_no_such_item" in odd_problems, "B and an item of the kit", odd_problems)
            check(not any(c in " ".join(w.get("problems", [])) for c in ("W115", "W116", "W117")),
                  "B while a character made of what the game has gets none of them")
        char_delete("qa_oddlooks")

        # a model on a skeleton of its own passes every other check and stops the game the moment
        # the NPC comes online: Clear Sky's Forester has no eyes and no jaw for an NPC's damage
        forester = char_write(COURIER.replace('"qa_courier"', '"qa_forester"')
                              .replace("stalker_neutral\\\\stalker_neutral_1", "stalker_lesnik\\\\stalker_lesnik_1"))
        forester_problems = " ".join(forester.get("problems", []))
        if cmd("game.lookup", type="character_icon", limit=5).get("entries"):
            check("E123" in forester_problems and "eye_left" in forester_problems and "jaw_1" in forester_problems,
                  "B a model on a skeleton of its own is an error naming the bones it lacks", forester_problems)
            check("E123" not in " ".join(w.get("problems", [])), "B while a stalker model has them all")
        char_delete("qa_forester")

        # ---------------------------------------------------------------- U: the Characters tab
        run_ui_checks()

        # ---------------------------------------------------------------- C: the build
        built = cmd("mod.build", target="folder", path=out_dir)
        check(built.get("ok"), "C the module builds", json.dumps(built)[:300])

        root = os.path.join(out_dir, "modules", module)
        sources = [os.path.join(b, f) for b, _, fs in os.walk(root) for f in fs if f.lower().endswith(".npcasset")]
        check(len(sources) == 2, "C the assets ship, one per character", json.dumps(sources)[:200])
        same = read(os.path.join(root, "characters", "qa_courier.npcasset")) == read(os.path.join(project, "characters", "qa_courier.npcasset"))
        check(same, "C exactly as the project holds them")
        made = [os.path.join(b, f) for b, _, fs in os.walk(root) for f in fs
                if re.search(r"(character_desc_|npc_profile_|_npc\.|_characters\.ltxp|mode_register|_modes)", f)]
        check(not made, "C and the build makes nothing of them", json.dumps(made)[:300])

        # ---------------------------------------------------------------- E: inheritance
        run_base_checks(module, out_dir)

        # ---------------------------------------------------------------- F: spawning by asset
        run_spawn_checks(module, out_dir)

        # ---------------------------------------------------------------- D: removal
        check(char_delete("qa_guard"), "D a character's file is removed")
        check(char_delete("qa_courier"), "D and so is the last one")
        until(lambda: char_ids() == [])

        built2 = cmd("mod.build", target="folder", path=out_dir)
        check(built2.get("ok"), "D the module builds again", json.dumps(built2)[:200])
        gone = [os.path.join(b, f) for b, _, fs in os.walk(root) for f in fs if f.lower().endswith(".npcasset")]
        check(not gone, "D and with them what the module ships", json.dumps(gone)[:200])

        # the copy is left without characters, as it started
        left = char_ids()
        check(left == [], "D no character is left in the project", str(left))

        # ------------------------------------------------------------------ S: the module's own sections
        own = os.path.join(project, "gamedata", "configs", "qa_own_items.ltx")
        os.makedirs(os.path.dirname(own), exist_ok=True)
        with open(own, "w", encoding="utf-8") as f:
            f.write("[qa_own_parcel]:identity_immunities\ninv_name = qa\n")
        time.sleep(0.4)
        r = char_write('return { npc = 1, id = "qa_own", name = "Own", visual = "actors\\\\stalker_neutral\\\\stalker_neutral_1", '
                       'supplies = { { section = "qa_own_parcel", count = 1 }, { section = "qa_nowhere_item", count = 1 } } }\n')
        lines = r.get("problems", [])
        check(not any("qa_own_parcel" in l for l in lines), "S an item the module brings itself is no unknown item",
              json.dumps(lines)[:400])
        check(any(l.startswith("W117") and "qa_nowhere_item" in l for l in lines),
              "S one neither the game nor the module has is still W117", json.dumps(lines)[:400])
        char_delete("qa_own")
        os.remove(own)

    finally:
        ed.close()
        qa.remove_tree(out_dir)


def main() -> int:
    return qa.suite("CHARACTERS", run, doc=__doc__, ignore=(".git", "releases", "temp", "logs", "backup", "quests"))


if __name__ == "__main__":
    sys.exit(main())
