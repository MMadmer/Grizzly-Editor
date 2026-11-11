#!/usr/bin/env python3
"""End-to-end check of the mode's world (EditorWorld, the Player Start, way paths,
placed NPC orders) over MCP, on a THROW-AWAY COPY of a project.

What it proves:
  A  mod.ltx [world] round-trips through world_set / world_get / mod_manifest, the
     one-click empty world, and the three-choice limit (refused, nothing saved)
  B  the linked game's levels and transitions come out of its all.spawn, and the
     area / locks close them the way the engine will (changer > level > area)
  U  the World Rules tab opens, points its transitions at a level and its problems
     drawer over MCP (world_view), draws every section and lets nothing escape it
  C  the scene: a Player Start, a way path and a stalker with an order bake into the
     spawn layer as op = start, [path:<name>] and [xms_order]; a path named like one
     of the game's is left out with a warning
  D  a second Player Start is E201 and the build refuses it; the build ships the rest
  E  the quest side: level_changer is a picker type, { role = } in a quest is E024
  F  the stock preset takes the section out of mod.ltx again

    python tools/qa/test_world.py --project <project>

The project must have a scene of a level open at start (its last_scene).
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

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP
RULES = ["original_npcs", "random_stalkers", "mutants", "npc_quests", "story_quests"]

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
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project],
                                     cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    self.settle()
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        raise RuntimeError("the editor never opened the project")

    @staticmethod
    def settle() -> None:
        # A minimized editor draws only when asked, and its first frame reopens the project's last scene - a blocking
        # load. That frame is drawn here and the load runs out: the checks read the editor, not a load's busy frames.
        mcp("ui_list")
        deadline = time.monotonic() + 600
        while mcp("progress").get("active") and time.monotonic() < deadline:
            time.sleep(0.05)

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


def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def wait_scene(level: str, seconds: float = 600) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            info = mcp("scene_info", timeout=60)
            if level in (info.get("scene") or "").lower():
                return True
        except OSError:
            pass
        time.sleep(2)
    return False


def world(answer: dict) -> dict:
    return answer.get("world", {}) if isinstance(answer, dict) else {}


def placed_name(answer: dict) -> str:
    # place_spawn / place_way answer with the object under "object"
    return (answer.get("object") or {}).get("name", "") if isinstance(answer, dict) else ""


QUEST_E024 = """return {
  nq = 1,
  id = "qa_world_roles",
  title = "QA world",
  activation = "auto",
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "go" } },
    { id = "go", kind = "flow.step",
      on_enter = {
        { kind = "order.follow", params = { who = { story = "esc_m_trader" }, target = { role = "boss" } } },
        { kind = "world.transition", params = { changer = "%CHANGER%", open = true } },
      },
      out = { next = "done" } },
    { id = "done", kind = "flow.end" },
  },
}
"""


def run_ui_checks() -> None:
    """The World Rules tab: opened, pointed and looked at over MCP, and nothing of it leaves its window."""
    v = mcp("world_view")
    check(v.get("ok") and v.get("tab") == "world_rules", "U world_view answers", json.dumps(v)[:300])
    check(v.get("status") == "Saved to mod.ltx as you edit", "U the tab says it saves as you edit", json.dumps(v)[:300])
    opened = mcp("ui_tab", action="open", kind="world_rules")
    check(opened.get("ok") and opened.get("tab", {}).get("id") == "world_rules", "U ui_tab opens World Rules",
          json.dumps(opened)[:300])
    v = mcp("world_view", drawer="open", level="l03_agroprom", filter="underground")
    check(v.get("ok") and v.get("active") and v.get("drawer") == "open", "U world_view opens the problems drawer",
          json.dumps(v)[:300])
    check(v.get("level") == "l03_agroprom" and v.get("filter") == "underground", "U and points the transitions at a level",
          json.dumps(v)[:300])
    bad = mcp("world_view", level="qa_no_level")
    check(not bad.get("ok") and "qa_no_level" in bad.get("error", ""), "U a level the game lacks is refused by name",
          json.dumps(bad))
    bad = mcp("world_view", drawer="sideways")
    check(not bad.get("ok"), "U and so is a drawer state that is not open or close", json.dumps(bad))
    audit = mcp("ui_audit", id="world_rules")
    check(audit.get("ok") and not audit.get("escaped"), "U nothing escapes the World Rules tab",
          json.dumps(audit.get("escaped"))[:400])
    labels = {c.get("label") for c in audit.get("controls", [])}
    for section in ("Original Game", "Player Start", "Place in Scene"):
        check(section in labels, "U the tab shows " + section, json.dumps(sorted(x for x in labels if x))[:400])
    # the rest is drawn once the page scrolls to it
    scrolled = mcp("ui_scroll", id="world_rules", to="bottom")
    check(scrolled.get("ok"), "U the page scrolls", json.dumps(scrolled)[:300])
    labels = {c.get("label") for c in mcp("ui_audit", id="world_rules").get("controls", [])}
    for section in ("Applies On", "Transitions"):
        check(section in labels, "U the tab shows " + section, json.dumps(sorted(x for x in labels if x))[:400])
    toolbar = {c.get("label") for c in mcp("ui_audit", id="##toolbar").get("controls", [])}
    check("Reload Game Data" in toolbar and "Empty World..." in toolbar, "U its toolbar offers Reload Game Data and Empty World...",
          json.dumps(sorted(x for x in toolbar if x))[:400])
    mcp("world_view", drawer="close", filter="")
    closed = mcp("ui_tab", action="close", id="world_rules")
    check(closed.get("ok"), "U the tab closes: it saves as it goes, so nothing is unsaved", json.dumps(closed)[:300])
    check(mcp("world_view").get("open") is False, "U and world_view says it is closed")


def run(project: str) -> None:
    ed = Editor()
    out_dir = tempfile.mkdtemp(prefix="grizzly_world_")
    try:
        ed.start(project)
        manifest = mcp("mod_manifest")
        module = manifest.get("id", "")
        check(bool(module), "A the project has a module id", json.dumps(manifest)[:200])
        mod_ltx = os.path.join(project, "mod.ltx")

        # ------------------------------------------------------------ A: the manifest
        g = mcp("world_get")
        check(g.get("ok"), "A world_get answers", json.dumps(g)[:300])
        check(world(g).get("declared") is False, "A a fresh project declares no world", json.dumps(world(g)))
        check("[world]" not in read(mod_ltx), "A and mod.ltx has no [world]")

        s = mcp("world_set", preset="empty")
        rules = world(s).get("rules", {})
        check(s.get("ok") and all(rules.get(r) is False for r in RULES), "A preset=empty removes all five", json.dumps(rules))
        text = read(mod_ltx)
        check("[world]" in text and all(f"{r}" in text for r in RULES) and text.count("= false") >= 5,
              "A mod.ltx spells every rule out", text[-400:])

        s = mcp("world_set", original_npcs=True)
        rules = world(s).get("rules", {})
        check(rules.get("original_npcs") is True and rules.get("mutants") is False,
              "A one rule changes alone", json.dumps(rules))

        bad = mcp("world_set", player_choice="random_stalkers,mutants,npc_quests,story_quests")
        check(not bad.get("ok") and "room for 3" in bad.get("error", ""),
              "A a fourth player choice is refused", json.dumps(bad))
        check(world(mcp("world_get")).get("player_choice") == [], "A and nothing was saved")
        bad = mcp("world_set", player_choice="bogus_rule")
        check(not bad.get("ok") and "bogus_rule" in bad.get("error", ""), "A an unknown rule is refused by name", json.dumps(bad))

        s = mcp("world_set", player_choice="random_stalkers,mutants")
        check(world(s).get("player_choice") == ["random_stalkers", "mutants"], "A two choices are kept",
              json.dumps(world(s)))
        check("player_choice" in read(mod_ltx) and "random_stalkers, mutants" in read(mod_ltx),
              "A as one line of mod.ltx")
        check(world(mcp("mod_manifest")).get("player_choice") == ["random_stalkers", "mutants"],
              "A mod_manifest reports the world too")

        # ------------------------------------------------------------ B: the linked game
        t = mcp("world_transitions")
        check(t.get("ok"), "B the linked game's transitions are read", json.dumps(t)[:300])
        levels = [l["name"] for l in t.get("levels", [])]
        trans = t.get("transitions", [])
        check("l03_agroprom" in levels and "l01_escape" in levels, "B its levels come from all.spawn", str(levels[:12]))
        check(len(trans) > 20, "B and dozens of level changers", str(len(trans)))
        check(all(tr["level"] in levels and tr["to"] in levels for tr in trans),
              "B every changer stands on a level and leads to one",
              json.dumps([tr for tr in trans if tr["level"] not in levels or tr["to"] not in levels][:3]))
        agro = [tr for tr in trans if tr["level"] == "l03_agroprom"]
        check(any(tr["to"] == "l03u_agr_underground" for tr in agro), "B Agroprom leads into its underground",
              json.dumps(agro)[:400])
        check(all(tr["closed"] is False for tr in trans), "B nothing is closed before an area or a lock")

        area = ["l03_agroprom", "l03u_agr_underground"]
        s = mcp("world_set", area=",".join(area))
        check(world(s).get("area") == area, "B the area is saved", json.dumps(world(s)))
        t = mcp("world_transitions", level="l03_agroprom")
        at = t.get("transitions", [])
        check(len(at) > 0 and all(tr["level"] == "l03_agroprom" for tr in at), "B level= narrows the list")
        outside = [tr for tr in at if tr["to"] not in area]
        inside = [tr for tr in at if tr["to"] in area]
        check(outside and all(tr["closed_by"] == "area" for tr in outside),
              "B a way out of the area is closed by the area", json.dumps(outside)[:300])
        check(inside and all(not tr["closed"] for tr in inside), "B a way inside stays open", json.dumps(inside)[:300])

        way_down = inside[0]["name"] if inside else ""
        mcp("world_set", lock=way_down)
        t = {tr["name"]: tr for tr in mcp("world_transitions", level="l03_agroprom").get("transitions", [])}
        check(t.get(way_down, {}).get("closed_by") == "locked", "B lock= closes one changer", json.dumps(t.get(way_down)))
        mcp("world_set", lock_level="l03_agroprom")
        t = mcp("world_transitions", level="l03_agroprom").get("transitions", [])
        check(all(tr["closed"] for tr in t), "B lock_level= closes the whole level")
        check(any(tr["closed_by"] == "locked" for tr in t) and any(tr["closed_by"] == "level" for tr in t),
              "B in the engine's order: the changer, then its level", json.dumps([tr["closed_by"] for tr in t]))
        s = mcp("world_set", unlock=way_down, unlock_level="l03_agroprom")
        check(world(s).get("locked") == [] and world(s).get("locked_levels") == [], "B and both open again", json.dumps(world(s)))

        s = mcp("world_set", levels="qa_no_level", lock="qa_no_changer")
        probs = " ".join(s.get("problems", []))
        check("W203" in probs and "qa_no_level" in probs, "B a level the game lacks is W203", probs)
        check("W204" in probs and "qa_no_changer" in probs, "B a changer the game lacks is W204", probs)
        check("W206" in probs, "B an area with no Player Start is W206", probs)
        mcp("world_set", levels="", unlock="qa_no_changer")

        patrols = mcp("world_transitions", patrols="agr_", limit=5)
        agr_patrols = patrols.get("patrols", [])
        check(len(agr_patrols) > 0 and patrols.get("patrols_total", 0) > len(agr_patrols),
              "B the game's own patrol paths are listed", json.dumps(patrols)[:300])

        # ------------------------------------------------------------ U: the World Rules tab
        run_ui_checks()

        # ------------------------------------------------------------ C: the scene
        # the editor does not reopen the last scene by itself on every start: ask for it
        scene_now = (mcp("scene_info").get("scene") or "").lower()
        if "l01_escape" not in scene_now:
            opened = mcp("open_scene", timeout=900, scene="l01_escape.level")
            if not check(opened.get("ok"), "C the project's Escape scene opens", json.dumps(opened)[:200]):
                # everything below stands on that scene: waiting for one that did not open only burns minutes
                raise RuntimeError("the Escape scene did not open")
        check(wait_scene("l01_escape"), "C the project's Escape scene is open")
        start = mcp("place_spawn", section="player_start", x=-223.0, y=-20.0, z=-143.0, snap_to_ground=True)
        start_name = placed_name(start)
        check(start.get("ok") and start_name, "C a Player Start is placed", json.dumps(start)[:300])
        g = mcp("world_get")
        starts = g.get("starts", [])
        check(len(starts) == 1 and starts[0]["source"] == "scene" and starts[0]["level"] == "l01_escape",
              "C world_get sees it in the scene at once", json.dumps(starts))
        check("W205" in " ".join(g.get("problems", [])), "C a start outside the area is W205", json.dumps(g.get("problems")))

        way = mcp("place_way", name="qa_patrol", points="-220,-20,-140; -210,-20,-140; -210,-20,-150")
        check(way.get("ok") and way.get("points") == 3, "C a way path is drawn over MCP", json.dumps(way)[:300])
        again = mcp("place_way", name="qa_patrol", points="-200,-20,-140")
        check(not again.get("ok"), "C a second object of the same name is refused")
        game_named = agr_patrols[0] if agr_patrols else ""
        clash = mcp("place_way", name=game_named, points="-230,-20,-140; -235,-20,-145") if game_named else {}
        check(clash.get("ok"), "C a path named like one of the game's can be drawn", json.dumps(clash)[:200])

        stalker = ""
        roster = mcp("list_spawn_sections", filter="stalker", limit=400).get("sections", [])
        for row in roster:
            if row.get("class") != "AI_STL_S":
                continue
            placed = mcp("place_spawn", section=row["section"], x=-215.0, y=-20.0, z=-145.0, snap_to_ground=True)
            if placed.get("ok"):
                stalker = placed_name(placed)
                break
        check(bool(stalker), "C a stalker is placed", str(len(roster)))

        o = mcp("spawn_order", name=stalker)
        check(o.get("ok") and o.get("order", {}).get("kind") == "none", "C a fresh stalker has no order", json.dumps(o)[:300])
        o = mcp("spawn_order", name=stalker, order="patrol")
        check(not o.get("ok") and "route" in o.get("error", ""), "C a patrol without a route is refused", json.dumps(o))
        o = mcp("spawn_order", name=stalker, order="patrol", route="qa_patrol", run=True, hostile=True)
        cd = o.get("custom_data", "")
        check(o.get("ok") and "order = patrol" in cd and "route = qa_patrol" in cd and "move = run" in cd
              and "hostile = true" in cd, "C the order becomes an [xms_order] section", cd)
        o = mcp("spawn_order", name=start_name)
        check(not o.get("ok"), "C a Player Start takes no order")

        e = mcp("mod_export_spawn", selected_only=False)
        check(e.get("ok"), "C the spawn layer bakes", json.dumps(e)[:400])
        check(e.get("starts") == 1 and e.get("orders") == 1, "C with the start and the order",
              json.dumps({k: e.get(k) for k in ("starts", "paths", "orders")}))
        warns = " ".join(e.get("warnings", []))
        check(game_named and game_named in warns and "rename" in warns, "C the path named like the game's is left out, by name", warns)

        layer = read(os.path.join(project, "spawn", "l01_escape.xspawn"))
        check("op        = start" in layer and f"[obj:{start_name}]" in layer, "C the start is an op = start", layer[:300])
        check("[path:qa_patrol]" in layer and "points = 3" in layer and "links  = 0-1:1, 1-2:1, 2-0:1" in layer,
              "C the path ships with its points and links", layer[layer.find("[path:"):][:400])
        check(f"[path:{game_named}]" not in layer, "C the game-named path does not")
        cd_dir = os.path.join(project, "spawn", "custom_data")
        orders = [f for f in os.listdir(cd_dir) if f.startswith("l01_escape_")] if os.path.isdir(cd_dir) else []
        order_text = "".join(read(os.path.join(cd_dir, f)) for f in orders)
        check("[xms_order]" in order_text and "route = qa_patrol" in order_text,
              "C the order rides in the NPC's custom data", order_text[-300:])

        # ------------------------------------------------------------ D: one start
        second = mcp("place_spawn", section="player_start", x=-205.0, y=-20.0, z=-150.0)
        g = mcp("world_get")
        probs = " ".join(g.get("problems", []))
        check("E201" in probs and start_name in probs and placed_name(second) in probs,
              "D two Player Starts are E201, both named", probs)
        ex = mcp("mod_export", target=out_dir)
        check(not ex.get("ok") and "E201" in ex.get("error", ""), "D the build refuses them", json.dumps(ex)[:300])
        mcp("select_objects", names=placed_name(second))
        mcp("delete_selected")
        ex = mcp("mod_export", target=out_dir)
        check(ex.get("ok"), "D with one start the module builds", json.dumps(ex)[:400])
        check("W205" in ex.get("warnings", ""), "D and the build says the start is outside the area", ex.get("warnings", "")[:300])
        root = os.path.join(out_dir, "modules", module)
        shipped = read(os.path.join(root, "mod.ltx"))
        check("[world]" in shipped and "area" in shipped, "D the module ships its world", shipped[-400:])
        shipped_layer = read(os.path.join(root, "spawn", "l01_escape.xspawn"))
        check("op        = start" in shipped_layer and "[path:qa_patrol]" in shipped_layer,
              "D and its spawn layer")
        check(shipped_layer.count("op        = start") == 1, "D with one start")

        # ------------------------------------------------------------ E: quests
        look = mcp("quest_lookup", type="level_changer", query="", limit=500)
        entries = look.get("entries", [])
        check(len(entries) > 20, "E level_changer is a picker type", json.dumps(look)[:300])
        changer = next((x["id"] for x in entries if x.get("id") == way_down), entries[0]["id"] if entries else "")
        qpath = os.path.join(project, "quests", "qa_world_roles.nqasset")
        w = mcp("quest_write", path=qpath, lua=QUEST_E024.replace("%CHANGER%", changer))
        v = mcp("quest_validate", path=qpath)
        vp = " ".join(p if isinstance(p, str) else json.dumps(p) for p in v.get("problems", []))
        check("E024" in vp and "a quest has no roles" in vp, "E { role = } in a quest is E024", vp[:400])
        check("W060" not in vp, "E a changer of the linked game is known", vp[:400])
        mcp("content_delete", path="quests/qa_world_roles.nqasset")

        # ------------------------------------------------------------ F: stock again
        s = mcp("world_set", preset="stock", area="")
        check(world(s).get("declared") is False, "F preset=stock is the stock game again", json.dumps(world(s)))
        check("[world]" not in read(mod_ltx), "F and mod.ltx carries no [world]")
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
    # C to F place things in the Escape scene; <project> has one
    if not os.path.isfile(os.path.join(src, "rawdata", "levels", "l01_escape.level")):
        print("the project has no rawdata\\levels\\l01_escape.level - the world checks need that scene: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="grizzly_worldproj_")
    copy = os.path.join(work, "QaWorld")
    shutil.copytree(src, copy)
    print("running against a copy: " + copy, flush=True)
    # the copy opens its OWN scene: project.ltx names the last one by absolute path
    pl = os.path.join(copy, "project.ltx")
    if os.path.isfile(pl):
        text = read(pl)
        low = text.lower()
        at = low.find(src.lower())
        while at >= 0:
            text = text[:at] + copy + text[at + len(src):]
            low = text.lower()
            at = low.find(src.lower(), at + len(copy))
        with open(pl, "w", encoding="utf-8", newline="") as f:
            f.write(text)

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
    print(f"WORLD: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
