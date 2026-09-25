#!/usr/bin/env python3
"""End-to-end check of the module's own characters (EditorCharacters).

Drives the editor over MCP on a THROW-AWAY COPY of a project: writes characters,
reads them back, builds the module into a temporary folder and verifies that the
generated half is exactly what the game needs to load them — the character_desc
entry, the profile, the string table, the spawn and squad sections, and the
.ltxp patch that appends (never replaces) the two profile lists.

    python tools/qa/test_characters.py --project D:\\XFinedProjects\\Test
"""
from __future__ import annotations

import argparse
import json
import os
import re
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
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
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


def run_spawn_checks(mcp, check, module, out_dir, os, json):
    """spawn.npc: the character is named by its asset id, and a ref per NPC."""
    # its own cast: a template and a character that starts from it
    mcp("characters_write", text=QA_TPL)
    mcp("characters_write", text=QA_RUNNER)
    cat = mcp("quest_catalog")
    by_id = {k.get("id"): k for k in cat.get("kinds", []) if isinstance(k, dict)}
    kind = by_id.get("spawn.npc", {})
    check(bool(kind), "F the catalog carries spawn.npc")
    rows = [r.get("name") for r in kind.get("rows", [])] if isinstance(kind.get("rows"), list) else []
    check("character" in rows and "unique" in rows and "ref" in rows,
          "F its rows name a character, a ref and unique", json.dumps(kind)[:300])
    # on for a character, off for a section: a default that follows the row is not a catalog default
    unique = next((r for r in kind.get("rows", []) if r.get("name") == "unique"), {})
    check("default" not in unique, "F unique declares no flat default - it follows what the row names", json.dumps(unique))

    look = mcp("quest_lookup", type="character_id")
    entries = {e.get("id"): e for e in look.get("entries", [])}
    check("qa_courier" in entries and "qa_runner" in entries, "F lookup character_id lists the project's characters",
          json.dumps(sorted(entries)))
    check("template" in str(entries.get("qa_tpl", {}).get("extra", "")), "F and says which one is a template",
          json.dumps(entries.get("qa_tpl")))

    w = mcp("quest_write", path="quests/qa_spawner.nqasset", lua=SPAWNER)
    errs = " ".join(p.get("code", "") for p in w.get("problems", []) if str(p.get("code", "")).startswith("E"))
    check(w.get("ok") and errs == "", "F a quest that spawns characters by their asset id validates", errs or json.dumps(w)[:200])
    refs = mcp("quest_lookup", type="ref_name", path="quests/qa_spawner.nqasset")
    names = sorted(e.get("id") for e in refs.get("entries", []))
    check(names == ["courier", "extra", "guard"], "F every row's ref is a ref of the graph", json.dumps(names))
    outline = str(mcp("quest_get", path="quests/qa_spawner.nqasset").get("outline", ""))
    check("npcs={qa_courier" in outline, "F the outline reads a row as who it creates, not as the ref it keeps", outline[:300])

    b = mcp("quest_write", path="quests/qa_spawner_bad.nqasset", lua=SPAWNER_BAD)
    text = " | ".join("%s %s" % (p.get("code"), p.get("message", "")) for p in b.get("problems", []))
    check("not a character of this project" in text, "F a character that does not exist is refused", text[:300])
    check("template" in text, "F a template cannot be spawned, and the message says why", text[:300])
    check("exactly one" in text, "F a row names a character OR a section", text[:300])
    check("unknown key 'uniqe'" in text, "F a misspelt key is an error, not a silent no-op", text[:300])
    mcp("quest_close", path="quests/qa_spawner_bad.nqasset", discard=True)
    mcp("content_delete", path="quests/qa_spawner_bad.nqasset")

    built = mcp("mod_export", target=out_dir)
    check(built.get("ok"), "F the module builds with the spawner in it", json.dumps(built)[:200])
    shipped = os.path.join(out_dir, "modules", module, "quests", "qa_spawner.nqasset")
    check(os.path.isfile(shipped), "F and the quest ships", shipped)
    # the section the game will be asked for is the one the build generated
    ltx = os.path.join(out_dir, "modules", module, "gamedata", "configs", "xms", module + "_npc.ltx")
    body = open(ltx, encoding="cp1251", errors="replace").read() if os.path.isfile(ltx) else ""
    check("[%s_qa_courier]" % module in body, "F <module>_<id> is exactly the spawn section the build wrote", ltx)

    # deleting the character turns the same quest into an error - no restart, no refresh
    mcp("characters_delete", id="qa_runner")
    v = mcp("quest_validate", path="quests/qa_spawner.nqasset")
    text = " | ".join("%s %s" % (p.get("code"), p.get("message", "")) for p in v.get("problems", []))
    check("qa_runner" in text and "not a character" in text, "F a deleted character is an error in the quest that names it", text[:300])

    mcp("characters_delete", id="qa_tpl")
    mcp("quest_close", path="quests/qa_spawner.nqasset", discard=True)
    mcp("content_delete", path="quests/qa_spawner.nqasset")


def run_base_checks(mcp, check, read, module, out_dir, os, json):
    """Inheritance: a shared kit written once, and what refuses to inherit."""
    k = mcp("characters_write", text=KIT)
    check(k.get("ok"), "E a template is written", json.dumps(k)[:200])
    check(k.get("character", {}).get("abstract") is True, "E and says it is one")
    check(k.get("problems") == [], "E a template needs no name and no model",
          json.dumps(k.get("problems")))

    g = mcp("characters_write", text=GUARD_KIT)
    check(g.get("ok"), "E a character inherits from it", json.dumps(g)[:200])
    check(g.get("problems") == [], "E with no problems", json.dumps(g.get("problems")))

    got = mcp("characters_get", id="qa_kitguard")
    ch = got.get("character", {})
    check(ch.get("base") == "qa_kit", "E it reports its base")
    # torch, knife, bandage, medkit(4), ak74u  - bread was taken out by count = 0
    check(ch.get("supplies") == 5, "E the kit is the base plus its own, merged",
          str(ch.get("supplies")))

    built = mcp("mod_export", target=out_dir)
    check(built.get("ok"), "E the module builds", json.dumps(built)[:200])
    root = os.path.join(out_dir, "modules", module)
    desc = read(os.path.join(root, "gamedata", "configs", "gameplay",
                             "character_desc_%s.xml" % module))
    ltx = read(os.path.join(root, "gamedata", "configs", "xms", "%s_npc.ltx" % module))

    gid = "%s_qa_kitguard" % module
    check('<specific_character id="%s"' % gid in desc, "E the character is in the module")
    check('<specific_character id="%s_qa_kit"' % module not in desc,
          "E the template is NOT - it was never a character")
    check("[%s_qa_kit]" % module not in ltx, "E and has no spawn section either")

    block = desc.split('<specific_character id="%s"' % gid)[1].split("</specific_character>")[0]
    check("device_torch \\n" in block and "wpn_knife \\n" in block,
          "E the inherited items are in its kit")
    check("medkit = 4 \\n" in block and "medkit = 2 \\n" not in block,
          "E a repeated section takes the new count")
    check("bread" not in block, "E and a count of 0 takes an inherited item away")
    check("wpn_ak74u \\n" in block, "E its own items are there too")
    check("<rank>250</rank>" in block, "E a field it does not set comes from the base")
    check("<community>dolg</community>" in block, "E one it does set wins")

    o = mcp("characters_write", text=NO_BASE)
    check("E111" in " ".join(o.get("problems", [])), "E a base that does not exist is reported",
          json.dumps(o.get("problems")))
    mcp("characters_write", text=LOOP_A)
    l = mcp("characters_write", text=LOOP_B)
    check("E112" in " ".join(mcp("characters_list").get("problems", [])),
          "E a base that comes back to itself is reported",
          json.dumps(mcp("characters_list").get("problems"))[:200])

    for gone in ("qa_orphan", "qa_loop_a", "qa_loop_b", "qa_kitguard", "qa_kit"):
        mcp("characters_delete", id=gone)


def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("cp1251", "replace")


def run(project: str) -> None:
    ed = Editor()
    out_dir = tempfile.mkdtemp(prefix="xfined_chars_")
    try:
        ed.start(project)

        manifest = mcp("mod_manifest")
        module = manifest.get("id", "")
        check(bool(module), "A the project has a module id", json.dumps(manifest)[:200])

        # the checks count characters, and the last one deleted takes the generated half with it: an author's
        # project may bring characters of its own, so the copy starts without them
        own = [c["id"] for c in mcp("characters_list").get("characters", [])]
        for cid in own:
            mcp("characters_delete", id=cid)
        check(mcp("characters_list").get("characters") == [], "A the copy starts without characters", str(own))

        # ---------------------------------------------------------------- A: write
        w = mcp("characters_write", text=COURIER)
        check(w.get("ok"), "A a character is written", json.dumps(w)[:200])
        check(w.get("character", {}).get("id") == "qa_courier", "A it comes back with its id")
        check(w.get("problems") == [], "A and with no problems", json.dumps(w.get("problems")))

        w2 = mcp("characters_write", text=GUARD)
        check(w2.get("ok"), "A a second character is written", json.dumps(w2)[:200])

        lst = mcp("characters_list")
        ids = sorted(c["id"] for c in lst.get("characters", []))
        check(ids == ["qa_courier", "qa_guard"], "A both are listed", str(ids))

        got = mcp("characters_get", id="qa_courier")
        check(got.get("ok") and "qa_courier" in got.get("text", ""), "A one reads back whole")
        check(got.get("character", {}).get("supplies") == 3, "A its kit has three rows",
              str(got.get("character")))

        # canonical: writing what was read changes nothing
        again = mcp("characters_write", id="qa_courier", text=got.get("text", ""))
        again_text = mcp("characters_get", id="qa_courier").get("text", "")
        check(again.get("ok") and again_text == got.get("text", ""),
              "A write(read(x)) is the same file")

        # ---------------------------------------------------------------- B: refusals
        bad = mcp("characters_write", text=BROKEN)
        problems = " ".join(bad.get("problems", []))
        check(bad.get("ok") and "E102" in problems, "B an id with spaces is reported", problems)
        check("E103" in problems, "B and so is a missing name", problems)
        check("E104" in problems, "B and a missing visual", problems)
        mcp("characters_delete", id="QA Bad Id")

        # a community is picked from the linked game's list in the window; the text path
        # can still name one the game has never heard of, and the engine refuses those
        odd = mcp("characters_write", text=COURIER.replace('"qa_courier"', '"qa_martian"')
                  .replace('community = "stalker"', 'community = "martians"'))
        odd_problems = " ".join(odd.get("problems", []))
        if mcp("quest_lookup", type="community", limit=5).get("entries"):
            check("W114" in odd_problems and "martians" in odd_problems,
                  "B a community the linked game does not list is a warning that names it", odd_problems)
            check("W114" not in " ".join(w.get("problems", [])), "B and one it does list is not")
        mcp("characters_delete", id="qa_martian")

        # the window picks portraits, dialogs and items from lists; the text path can name
        # anything, and what the linked game does not have is said - the old SoC portrait id
        # is the classic: no picture in Dead Air Refined
        odd = mcp("characters_write", text=COURIER.replace('"qa_courier"', '"qa_oddlooks"')
                  .replace('"ui_inGame2_neutral_2"', '"ui_npc_u_stalker_neutral_1"')
                  .replace('rank      = 300,', 'rank      = 300,\n    start_dialog = "qa_no_such_dialog",')
                  .replace('supplies  = {', 'supplies  = {\n        { section = "qa_no_such_item" },'))
        odd_problems = " ".join(odd.get("problems", []))
        if mcp("quest_lookup", type="character_icon", limit=5).get("entries"):
            check("W115" in odd_problems, "B a portrait the linked game does not have is a warning", odd_problems)
            check("W116" in odd_problems, "B and so is a dialog", odd_problems)
            check("W117" in odd_problems and "qa_no_such_item" in odd_problems, "B and an item of the kit", odd_problems)
            check(not any(c in " ".join(w.get("problems", [])) for c in ("W115", "W116", "W117")),
                  "B while a character made of what the game has gets none of them")
        mcp("characters_delete", id="qa_oddlooks")

        # ---------------------------------------------------------------- C: the build
        built = mcp("mod_export", target=out_dir)
        check(built.get("ok"), "C the module builds", json.dumps(built)[:300])

        root = os.path.join(out_dir, "modules", module)
        desc = os.path.join(root, "gamedata", "configs", "gameplay", f"character_desc_{module}.xml")
        prof = os.path.join(root, "gamedata", "configs", "gameplay", f"npc_profile_{module}.xml")
        ltx = os.path.join(root, "gamedata", "configs", "xms", f"{module}_npc.ltx")
        patch = os.path.join(root, "patch", f"{module}_characters.ltxp")
        st_rus = os.path.join(root, "gamedata", "configs", "text", "rus", f"{module}_npc.xml")
        st_eng = os.path.join(root, "gamedata", "configs", "text", "eng", f"{module}_npc.xml")

        for path, what in ((desc, "character_desc"), (prof, "npc_profile"),
                           (ltx, "spawn sections"), (patch, "the ltxp patch"),
                           (st_rus, "the rus string table"), (st_eng, "the eng string table")):
            check(os.path.isfile(path), f"C {what} is in the module", path)
        # the asset is the source; the game reads what was generated from it
        sources = [os.path.join(b, f) for b, _, fs in os.walk(root) for f in fs if f.lower().endswith(".npcasset")]
        check(not sources, "C the .npcasset sources stay in the project", json.dumps(sources)[:200])

        gid = f"{module}_qa_courier"
        d = read(desc)
        check(f'<specific_character id="{gid}"' in d, "C the character is described", d[:200])
        check("<visual>actors\\stalker_neutral\\stalker_neutral_1</visual>" in d,
              "C with its model")
        check(f"<name>{gid}_name</name>" in d, "C and a string-table name, not a literal")
        check("wpn_ak74u \\n" in d and "ammo_5.45x39_fmj = 3 \\n" in d and "medkit \\n" in d,
              "C its kit is written the way the engine reads one")
        check('<money min="500" max="1500" infinitive="0"/>' in d, "C and its money")
        # the engine asserts on these three (CSpecificCharacter::load_shared): a character that
        # sets none of them must still carry all three, or the game stops loading profiles
        blocks = d.split("<specific_character ")[1:]
        bare = [b.split('"')[1] for b in blocks
                if not all(tag in b.split("</specific_character>")[0] for tag in ("<community>", "<rank>", "<reputation>"))]
        check(blocks and not bare, "C every character carries community, rank and reputation", json.dumps(bare))

        p = read(prof)
        check(f'<character id="{gid}">' in p and f"<specific_character>{gid}</specific_character>" in p,
              "C the profile names the character outright (no random pick)")

        l = read(ltx)
        check(f"[{gid}]:stalker" in l, "C the spawn section inherits stalker", l[:200])
        check(f"character_profile     = {gid}" in l, "C and carries the profile")
        check("$spawn" in l, "C and is spawnable, so it shows up in the spawn roster")
        check(f"[{gid}_squad]:online_offline_group" in l, "C a one-man squad exists for spawn.squad")
        check(f"[{module}_qa_guard_squad]" not in l, "C and squad = false leaves none")

        t = read(patch)
        check("+files = npc_profile_" + module in t, "C the patch APPENDS the profile file")
        check("+specific_characters_files = character_desc_" + module in t,
              "C and appends the character file")
        check("[profiles]" in t and "-files" not in t, "C nothing of the game is replaced")

        s_rus = read(st_rus)
        s_eng = read(st_eng)
        check(f'<string id="{gid}_name">' in s_rus and "Kurier" in s_rus, "C rus name is in place")
        check("Courier" in s_eng, "C eng name is in place")
        check(f"{module}_qa_guard_name" in s_rus and "Guard" in s_rus,
              "C a single-language name lands in rus")

        # ---------------------------------------------------------------- E: inheritance
        run_base_checks(mcp, check, read, module, out_dir, os, json)

        # ---------------------------------------------------------------- F: spawning by asset
        run_spawn_checks(mcp, check, module, out_dir, os, json)

        # ---------------------------------------------------------------- D: removal
        d1 = mcp("characters_delete", id="qa_guard")
        check(d1.get("ok"), "D a character can be removed", json.dumps(d1)[:200])
        d2 = mcp("characters_delete", id="qa_courier")
        check(d2.get("ok"), "D and so can the last one")

        built2 = mcp("mod_export", target=out_dir)
        check(built2.get("ok"), "D the module builds again", json.dumps(built2)[:200])
        gone = [os.path.isfile(x) for x in (desc, prof, ltx, patch, st_rus, st_eng)]
        check(not any(gone), "D the generated half went with them", str(gone))

        # the copy is left without characters, as it started
        left = mcp("characters_list").get("characters", [])
        check(left == [], "D no character is left in the project", str(left))

    finally:
        ed.close()
        shutil.rmtree(out_dir, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="xfined_charproj_")
    copy = os.path.join(work, "QaCharacters")
    shutil.copytree(src, copy)
    print("running against a copy: " + copy, flush=True)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()

    try:
        run(copy)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"CHARACTERS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
