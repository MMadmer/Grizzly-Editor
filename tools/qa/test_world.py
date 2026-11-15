#!/usr/bin/env python3
"""End-to-end check of the mode's world (EditorWorld, the Player Start, way paths,
placed NPC orders) over MCP, on a THROW-AWAY COPY of a project.

What it proves:
  A  mod.ltx [world] round-trips through world.set / world.info / mod.info, the
     one-click empty world, and the three-choice limit (refused, nothing saved)
  B  the linked game's levels and transitions come out of its all.spawn, and the
     area / locks close them the way the engine will (changer > level > area)
  U  the World Rules tab opens, points its transitions at a level and its problems
     drawer over MCP (world.view), draws every section and lets nothing escape it, and puts up its two lists of
     levels (world.view show=: the rows `ui.audit` lists, a picture, ui.popup show=none)
  C  the scene: a Player Start, a way path and a stalker with an order bake into the
     spawn layer as op = start, [path:<name>] and [xms_order]; a path named like one
     of the game's is left out with a warning
  D  a second Player Start is E201 and the build refuses it; the build ships the rest
  E  the quest side: level_changer is a picker type, { role = } in a quest is E024
  F  the stock preset takes the section out of mod.ltx again
  R  a radio: Place > Radio (scene.place section=radio) places one the editor tells for a radio, its
     Radio\\Plays lists the module's playlists by title and is set by scene.set and read back from the
     scene's file; the bake writes [xms_radio] playlist = <module>.<id>; scene.audit says S214 for a
     playlist mod.ltx lacks and nothing for one it has; a stalker has no Plays; a quest's radio.play is
     checked by asset.check (a playlist it has, one it lacks, a value that is no name); the Details row and
     the graph's parameter audited and pictured at both sizes, in English and Russian

    python tools/qa/test_world.py --project <project>

The project must have a scene of a level open at start (its last_scene).
"""
from __future__ import annotations

import json
import re
import os
import sys
import tempfile
import time

import qa
from qa import check, cmd


RULES = ["original_npcs", "random_stalkers", "mutants", "npc_quests", "story_quests"]

def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def wait_scene(level: str, seconds: float = 600) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            info = cmd("scene.info", timeout=60)
            if level in (info.get("file") or "").lower():
                return True
        except OSError:
            pass
        time.sleep(2)
    return False


def world(answer: dict) -> dict:
    return answer.get("world", {}) if isinstance(answer, dict) else {}


def placed_name(answer: dict) -> str:
    # scene.place answers with the object under "object"
    return (answer.get("object") or {}).get("name", "") if isinstance(answer, dict) else ""


RADIO_QUEST = """return {
  nq = 1,
  id = "qa_radio",
  title = "QA radio",
  activation = "auto",
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "go" } },
    { id = "go", kind = "flow.step",
      on_enter = {
        { kind = "radio.play", params = { radio = { story = "qa_radio" }, playlist = "qa_road" } },
        { kind = "radio.play", params = { radio = { story = "qa_radio" }, playlist = "qa_gone" } },
        { kind = "radio.play", params = { radio = { story = "qa_radio" }, playlist = "no name", next_track = true } },
        { kind = "radio.reset", params = { radio = { story = "qa_radio" } } },
      },
      out = { next = "done" } },
    { id = "done", kind = "flow.end" },
  },
}
"""

SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))


def plays_row(name: str) -> dict:
    got = (cmd("scene.get", objects=[name], keys=["radio"]).get("objects") or [{}])[0]
    return next((p for p in got.get("properties") or [] if p.get("key") == "radio.plays"), {})


def audit_pass(tag: str, panel: str, want: list, shots, shot_name: str, before=None) -> None:
    """One look at a panel at both sizes, in English and Russian: what it shows, nothing cut to nothing, nothing
    escaping, every text of the editor's own in Russian."""
    for lang in ("en", "ru"):
        cmd("app.prefs", action="set", key="language", value=lang)
        for width, height, scale in SIZES:
            cmd("ui.scale", value=scale)
            cmd("app.window", action="resize", width=width, height=height)
            if before:
                before()
            a = cmd("ui.audit", id=panel) if panel else cmd("ui.audit")
            labels = {c.get("label") for c in a.get("controls", []) if c.get("label")}
            where = "%s %s %dx%d" % (tag, lang, width, height)
            if lang == "en":
                check(all(w in labels for w in want), "%s shows %s" % (where, ", ".join(want)),
                      json.dumps(sorted(labels))[:500])
            cut = a.get("truncated") or []
            vanished = [t for t in cut if not t.get("shown") and (t.get("rect") or [0, 0, 0])[2] > 0]
            check(a.get("ok") and not vanished and not a.get("escaped"), "%s: nothing cut to nothing, nothing escapes" % where,
                  qa.short({"vanished": vanished[:3], "escaped": (a.get("escaped") or [])[:3]}))
            if lang == "ru":
                missing = [m.get("label") for m in a.get("untranslated") or []]
                check(not missing, "%s: the editor's own words read in Russian" % where, qa.short(missing[:12]))
            qa.shot(shots, "%s_%s_%d" % (shot_name, lang, width))
    cmd("app.prefs", action="set", key="language", value="en")
    cmd("ui.scale", value=1.5)


def radio_checks(project: str, module: str, shots) -> None:
    """R: a radio of the world, what it plays, its bake, its audit, a quest's radio actions, the UI."""
    mod_ltx = os.path.join(project, "mod.ltx")
    added = cmd("screens.set", action="playlist_add", id="qa_road", title="QA Road")
    check(added.get("ok") and "[playlist.qa_road]" in read(mod_ltx), "R the module gets a playlist", json.dumps(added)[:300])
    radio = cmd("scene.place", kind="spawn", section="radio", name="qa_radio", position=[-218.0, -20.0, -150.0], on_surface=True)
    check(radio.get("ok") and placed_name(radio) == "qa_radio", "R Place > Radio places a radio", json.dumps(radio)[:300])
    got = (cmd("scene.get", objects=["qa_radio"], keys=["section", "props.visual", "props.custom_data"]).get("objects") or [{}])[0]
    props = {p["key"]: p.get("value") for p in got.get("properties") or []}
    check("priemnik_gorizont" in str(props.get("props.visual")) and "ph_sound@play" in str(props.get("props.custom_data"))
          and "common_radio" in str(props.get("props.custom_data")),
          "R as the game's own level radios are: their model, the radio's logic in its custom data", json.dumps(props)[:400])
    row = plays_row("qa_radio")
    check(row.get("section") == "Radio" and row.get("label") == "Plays" and row.get("value") == "",
          "R a radio shows Radio\\Plays, at the player's station", json.dumps(row)[:400])
    check("qa_road" in (row.get("choices") or []) and "QA Road" in (row.get("choice_labels") or [])
          and {"game", "off"} <= set(row.get("choices") or []),
          "R its list: the module's playlists by title, the game's music, silence", json.dumps(row)[:400])
    ch = cmd("scene.choices", object="qa_radio", key="radio.plays")
    check(ch.get("ok") and "QA Road" in (ch.get("labels") or []) and "As the Player Chose" in (ch.get("labels") or []),
          "R scene.choices answers the pick list with its names", json.dumps(ch)[:400])

    s1 = cmd("scene.set", objects=["qa_radio"], key="radio.plays", value="QA Road")
    check(s1.get("ok") and plays_row("qa_radio").get("value") == "qa_road", "R a title sets it to the playlist's id", json.dumps(s1)[:300])
    bad = cmd("scene.set", objects=["qa_radio"], key="radio.plays", value="not a playlist")
    check(not bad.get("ok") and "no playlist" in bad.get("error", ""), "R a text that is no playlist is refused", json.dumps(bad)[:300])
    roster = cmd("game.lookup", type="spawn_section", query="stalker", limit=400).get("entries", [])
    stalker = ""
    for r in roster:
        if r.get("extra") == "AI_STL_S":
            placed = cmd("scene.place", kind="spawn", section=r["id"], position=[-214.0, -20.0, -146.0], on_surface=True)
            if placed.get("ok"):
                stalker = placed_name(placed)
                break
    if stalker:
        check(not plays_row(stalker), "R a stalker is no radio: no Plays", json.dumps(plays_row(stalker))[:200])
        no = cmd("scene.set", objects=[stalker], key="radio.plays", value="game")
        check(not no.get("ok"), "R and scene.set gives it none", json.dumps(no)[:200])
        cmd("scene.delete", objects=[stalker])

    # the scene's file says it by name
    saved = cmd("scene.save", timeout=900)
    check(saved.get("ok"), "R the scene saves", json.dumps(saved)[:300])
    scene_file = cmd("scene.info").get("file") or ""
    folder = os.path.splitext(os.path.join(project, scene_file) if not os.path.isabs(scene_file) else scene_file)[0]
    spawns = os.path.join(folder, "spawns.lua")
    text = read(spawns) if os.path.isfile(spawns) else ""
    at = text.find('"qa_radio"')
    check(at >= 0 and 'radio = { plays = "qa_road" }' in text[at:at + 4000], "R spawns.lua carries radio = { plays = \"qa_road\" }",
          spawns + " " + text[at:at + 600])

    # the audit: nothing for a playlist the module has, S214 for one it lacks
    a = cmd("scene.audit", objects=["qa_radio"], checks=["radio_playlist"])
    check(a.get("ok") and not a.get("problems"), "R scene.audit finds nothing wrong with a playlist mod.ltx has", json.dumps(a)[:400])
    cmd("scene.set", objects=["qa_radio"], key="radio.plays", value="qa_gone")
    a = cmd("scene.audit", objects=["qa_radio"], checks=["radio_playlist"])
    probs = a.get("problems") or []
    check(len(probs) == 1 and probs[0].get("code") == "S214" and "qa_gone" in json.dumps(probs[0]) and probs[0].get("line"),
          "R S214 for a playlist mod.ltx lacks, at the radio's line of spawns.lua", json.dumps(a)[:500])
    e = cmd("level.export", kind="spawn", objects=["qa_radio"])
    check(e.get("ok") and any("qa_gone" in w and "does not declare" in w for w in e.get("warnings", [])),
          "R the level export says the same", json.dumps(e.get("warnings"))[:400])

    # the bake
    cmd("scene.set", objects=["qa_radio"], key="radio.plays", value="qa_road")
    e = cmd("level.export", kind="spawn", all=True)
    check(e.get("ok") and e.get("radios") == 1, "R the bake binds one radio", json.dumps({k: e.get(k) for k in ("radios", "warnings")})[:400])
    cd = os.path.join(project, "spawn", "custom_data", "l01_escape_qa_radio.ltx")
    baked = read(cd) if os.path.isfile(cd) else ""
    check("[xms_radio]" in baked and ("playlist = %s.qa_road" % module) in baked and "ph_sound@play" in baked,
          "R its custom data: the logic and [xms_radio] playlist = <module>.qa_road", baked[-400:])
    layer = read(os.path.join(project, "spawn", "l01_escape.xspawn"))
    at = layer.find("[obj:qa_radio]")
    check(at >= 0 and "section   = physic_object" in layer[at:at + 400] and "custom_data = @spawn/custom_data/l01_escape_qa_radio.ltx" in layer[at:at + 600],
          "R the spawn layer names it and its custom data", layer[at:at + 500])

    # the quest
    qpath = os.path.join(project, "quests", "qa_radio.nqasset")
    os.makedirs(os.path.dirname(qpath), exist_ok=True)
    with open(qpath, "w", encoding="utf-8") as f:
        f.write(RADIO_QUEST)
    problems = lambda: [p if isinstance(p, str) else json.dumps(p) for p in cmd("asset.check", path=qpath).get("problems", [])]
    unknown = lambda found: [p for p in found if "radio.play' is not" in p or "radio.reset' is not" in p]
    found = problems()
    if unknown(found):
        # the linked game is older than catalog v22: the copy brings the two kinds as a module's own, word for word
        # the game's catalog (the bundled copy), as the deployed game will have them
        print("note: the linked game's catalog has no radio kinds yet - the copy brings them from the bundled catalog",
              flush=True)
        bundled = read(os.path.join(qa.REPO, "gamedata", "configs", "nq", "catalog.ltx"))
        kinds = "".join(m.group(0) for m in re.finditer(r"\[nq\.radio\.(?:play|reset)\][^\[]*", bundled))
        kdir = os.path.join(project, "gamedata", "configs", "nq", "kinds")
        os.makedirs(kdir, exist_ok=True)
        with open(os.path.join(kdir, "qa_radio_kinds.ltx"), "w", encoding="utf-8") as f:
            f.write(kinds)
        found = qa.until(lambda: (lambda f: f if not unknown(f) else [])(problems()), 20) or problems()
    check(not unknown(found), "R the catalog knows radio.play and radio.reset", json.dumps(found)[:300])
    radio_probs = [p for p in found if "playlist" in p]
    check(any("E030" in p and "qa_gone" in p for p in radio_probs), "R a playlist the module lacks is E030", json.dumps(found)[:600])
    check(any("E006" in p and "no name" in p for p in radio_probs), "R a value that is no name is E006", json.dumps(found)[:600])
    check(not any("qa_road" in p for p in found), "R a playlist the module has passes", json.dumps(found)[:600])
    look = cmd("game.lookup", type="radio_playlist")
    ids = [x.get("id") for x in look.get("entries", [])]
    check("qa_road" in ids and "game" in ids and "off" in ids, "R game.lookup lists the playlists, game and off", json.dumps(look)[:300])

    # the UI: the Details row, then the graph's parameter
    cmd("ui.tab", action="activate", doc="level")
    cmd("scene.select", objects=["qa_radio"])
    cmd("ui.panel", id="details", action="open")
    audit_pass("R Details of a radio", "details", ["Radio", "Plays", "QA Road"], shots, "radio_details",
               before=lambda: cmd("ui.scroll", id="details", to="bottom"))
    opened = cmd("ui.tab", action="open", kind="quest", path="quests/qa_radio.nqasset")
    check(opened.get("ok"), "R the quest opens in the graph tab", json.dumps(opened)[:300])
    def show_action():
        cmd("graph.view", doc="quests/qa_radio.nqasset", select=["go"])
        cmd("graph.view", doc="quests/qa_radio.nqasset", slot="enter:0", details="show")
        cmd("graph.view", doc="quests/qa_radio.nqasset", scroll="action:top")
    audit_pass("R the graph's radio.play", "", ["Playlist", "QA Road"], shots, "radio_graph", before=show_action)
    cmd("ui.tab", action="close", doc="quest:quests/qa_radio.nqasset", discard=True)
    cmd("file.delete", path="quests/qa_radio.nqasset")
    cmd("scene.delete", objects=["qa_radio"])
    cmd("screens.set", action="playlist_remove", id="qa_road")
    qa.stalls("R no UI pass stalled")


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


def drawn() -> set:
    """The labels a frame draws now, the search field and the rows of a pick list that is up among them."""
    return {c.get("label") for c in cmd("ui.audit").get("controls", []) if c.get("label")}


def pick_list_checks(shots) -> None:
    """The two lists of levels no click is needed for: `world.view show=` unfolds More and the section, switches
    the list on and puts it up; `ui.audit` lists its search field and rows (a kit pick list is no menu: `ui.list`
    does not name it), a picture shows it, `ui.popup show=none` takes it down."""
    quiet = drawn()
    check(not [l for l in quiet if "  -  " in l], "U no level list is up before one is asked for", str(sorted(quiet))[:200])
    for show in ("add_level", "add_area_level"):
        v = cmd("world.view", show=show)
        check(v.get("ok") and v.get("active"), "U world.view show=%s answers" % show, json.dumps(v)[:300])
        # a level is a row as "<id>  -  <caption>"; the pick list is virtualized, the first rows are drawn
        rows = qa.until(lambda: [l for l in drawn() - quiet if "  -  " in l], 10)
        check(bool(rows), "U show=%s puts the list of the linked game's levels up, with rows" % show, str(rows)[:300])
        # the button the list hangs from is under the fold: bring it into view for the picture
        cmd("ui.scroll", id="world_rules", to="bottom")
        cmd("ui.frame")
        qa.shot(shots, "world_rules_" + show)
        gone = cmd("ui.popup", show="none")
        check(gone.get("ok") and not [l for l in drawn() - quiet if "  -  " in l], "U ui.popup show=none takes the %s list down" % show,
              json.dumps(gone)[:200])
    bad = cmd("world.view", show="nowhere")
    check(not bad.get("ok") and "add_level" in bad.get("error", "") and "add_area_level" in bad.get("error", ""),
          "U an unknown list is refused, naming the lists", json.dumps(bad)[:300])
    # the contract says which popups a view command brings up
    popups = cmd("ui.contract").get("view_popups", [])
    check("world.view show=add_level" in popups and "mod.view show=function" in popups,
          "U ui.contract names the popups world.view and mod.view put up", json.dumps(popups)[:400])


def run_ui_checks(shots) -> None:
    """The World Rules tab: opened, pointed and looked at over MCP, and nothing of it leaves its window."""
    v = cmd("world.view")
    check(v.get("ok") and v.get("tab") == "world_rules", "U world.view answers", json.dumps(v)[:300])
    check(v.get("status") == "Saved to mod.ltx as you edit", "U the tab says it saves as you edit", json.dumps(v)[:300])
    opened = cmd("ui.tab", action="open", kind="world_rules")
    check(opened.get("ok") and opened.get("tab", {}).get("id") == "world_rules", "U ui.tab opens World Rules",
          json.dumps(opened)[:300])
    v = cmd("world.view", drawer="open", level="l03_agroprom", filter="underground")
    check(v.get("ok") and v.get("active") and v.get("drawer") == "open", "U world.view opens the problems drawer",
          json.dumps(v)[:300])
    check(v.get("level") == "l03_agroprom" and v.get("filter") == "underground", "U and points the transitions at a level",
          json.dumps(v)[:300])
    bad = cmd("world.view", level="qa_no_level")
    check(not bad.get("ok") and "qa_no_level" in bad.get("error", ""), "U a level the game lacks is refused by name",
          json.dumps(bad))
    bad = cmd("world.view", drawer="sideways")
    check(not bad.get("ok"), "U and so is a drawer state that is not open or close", json.dumps(bad))
    audit = cmd("ui.audit", id="world_rules")
    check(audit.get("ok") and not audit.get("escaped"), "U nothing escapes the World Rules tab",
          json.dumps(audit.get("escaped"))[:400])
    labels = {c.get("label") for c in audit.get("controls", [])}
    for section in ("Original Game", "Player Start", "Place in Scene"):
        check(section in labels, "U the tab shows " + section, json.dumps(sorted(x for x in labels if x))[:400])
    pick_list_checks(shots)
    # the rest is drawn once the page scrolls to it
    scrolled = cmd("ui.scroll", id="world_rules", to="bottom")
    check(scrolled.get("ok"), "U the page scrolls", json.dumps(scrolled)[:300])
    labels = {c.get("label") for c in cmd("ui.audit", id="world_rules").get("controls", [])}
    for section in ("Applies On", "Transitions"):
        check(section in labels, "U the tab shows " + section, json.dumps(sorted(x for x in labels if x))[:400])
    toolbar = {c.get("label") for c in cmd("ui.audit", id="##toolbar").get("controls", [])}
    check("Reload Game Data" in toolbar and "Empty World..." in toolbar, "U its toolbar offers Reload Game Data and Empty World...",
          json.dumps(sorted(x for x in toolbar if x))[:400])
    cmd("world.view", drawer="close", filter="")
    closed = cmd("ui.tab", action="close", doc="world_rules")
    check(closed.get("ok"), "U the tab closes: it saves as it goes, so nothing is unsaved", json.dumps(closed)[:300])
    check(cmd("world.view").get("open") is False, "U and world.view says it is closed")


def run(ctx: qa.Run) -> None:
    project = ctx.project
    ed = qa.Editor(project).start()
    out_dir = tempfile.mkdtemp(prefix="grizzly_world_")
    try:
        manifest = cmd("mod.info").get("manifest", {})
        module = manifest.get("id", "")
        check(bool(module), "A the project has a module id", json.dumps(manifest)[:200])
        mod_ltx = os.path.join(project, "mod.ltx")

        # ------------------------------------------------------------ A: the manifest
        g = cmd("world.info")
        check(g.get("ok"), "A world.info answers", json.dumps(g)[:300])
        check(world(g).get("declared") is False, "A a fresh project declares no world", json.dumps(world(g)))
        check("[world]" not in read(mod_ltx), "A and mod.ltx has no [world]")

        s = cmd("world.set", preset="empty")
        rules = world(s).get("rules", {})
        check(s.get("ok") and all(rules.get(r) is False for r in RULES), "A preset=empty removes all five", json.dumps(rules))
        text = read(mod_ltx)
        check("[world]" in text and all(f"{r}" in text for r in RULES) and text.count("= false") >= 5,
              "A mod.ltx spells every rule out", text[-400:])

        s = cmd("world.set", original_npcs=True)
        rules = world(s).get("rules", {})
        check(rules.get("original_npcs") is True and rules.get("mutants") is False,
              "A one rule changes alone", json.dumps(rules))

        bad = cmd("world.set", player_choice="random_stalkers,mutants,npc_quests,story_quests")
        check(not bad.get("ok") and "room for 3" in bad.get("error", ""),
              "A a fourth player choice is refused", json.dumps(bad))
        check(world(cmd("world.info")).get("player_choice") == [], "A and nothing was saved")
        bad = cmd("world.set", player_choice="bogus_rule")
        check(not bad.get("ok") and "bogus_rule" in bad.get("error", ""), "A an unknown rule is refused by name", json.dumps(bad))

        s = cmd("world.set", player_choice="random_stalkers,mutants")
        check(world(s).get("player_choice") == ["random_stalkers", "mutants"], "A two choices are kept",
              json.dumps(world(s)))
        check("player_choice" in read(mod_ltx) and "random_stalkers, mutants" in read(mod_ltx),
              "A as one line of mod.ltx")
        check(world(cmd("mod.info").get("manifest", {})).get("player_choice") == ["random_stalkers", "mutants"],
              "A mod.info reports the world too")

        # ------------------------------------------------------------ B: the linked game
        t = cmd("world.lookup", type="transitions")
        check(t.get("ok"), "B the linked game's transitions are read", json.dumps(t)[:300])
        levels = [l["name"] for l in cmd("world.lookup", type="levels").get("levels", [])]
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
        s = cmd("world.set", area=",".join(area))
        check(world(s).get("area") == area, "B the area is saved", json.dumps(world(s)))
        t = cmd("world.lookup", type="transitions", level="l03_agroprom")
        at = t.get("transitions", [])
        check(len(at) > 0 and all(tr["level"] == "l03_agroprom" for tr in at), "B level= narrows the list")
        outside = [tr for tr in at if tr["to"] not in area]
        inside = [tr for tr in at if tr["to"] in area]
        check(outside and all(tr["closed_by"] == "area" for tr in outside),
              "B a way out of the area is closed by the area", json.dumps(outside)[:300])
        check(inside and all(not tr["closed"] for tr in inside), "B a way inside stays open", json.dumps(inside)[:300])

        way_down = inside[0]["name"] if inside else ""
        cmd("world.set", lock=way_down)
        t = {tr["name"]: tr for tr in cmd("world.lookup", type="transitions", level="l03_agroprom").get("transitions", [])}
        check(t.get(way_down, {}).get("closed_by") == "locked", "B lock= closes one changer", json.dumps(t.get(way_down)))
        cmd("world.set", lock_level="l03_agroprom")
        t = cmd("world.lookup", type="transitions", level="l03_agroprom").get("transitions", [])
        check(all(tr["closed"] for tr in t), "B lock_level= closes the whole level")
        check(any(tr["closed_by"] == "locked" for tr in t) and any(tr["closed_by"] == "level" for tr in t),
              "B in the engine's order: the changer, then its level", json.dumps([tr["closed_by"] for tr in t]))
        s = cmd("world.set", unlock=way_down, unlock_level="l03_agroprom")
        check(world(s).get("locked") == [] and world(s).get("locked_levels") == [], "B and both open again", json.dumps(world(s)))

        s = cmd("world.set", levels="qa_no_level", lock="qa_no_changer")
        probs = " ".join(s.get("problems", []))
        check("W203" in probs and "qa_no_level" in probs, "B a level the game lacks is W203", probs)
        check("W204" in probs and "qa_no_changer" in probs, "B a changer the game lacks is W204", probs)
        check("W206" in probs, "B an area with no Player Start is W206", probs)
        # the check puts each of them at the key it is about, not at [world]'s header
        written = read(mod_ltx).splitlines()
        line_of = lambda key: next((i + 1 for i, l in enumerate(written) if re.match(r"\s*%s\s*=" % key, l)), 0)
        found = cmd("asset.check", path="mod.ltx").get("problems", [])
        placed = {(p.get("code"), p.get("line")) for p in found}
        check(line_of("levels") and ("W203", line_of("levels")) in placed,
              "B W203 is at the levels line of mod.ltx", json.dumps([p for p in found if p.get("code") == "W203"])[:300])
        check(line_of("locked") and ("W204", line_of("locked")) in placed,
              "B W204 is at the locked line of mod.ltx", json.dumps([p for p in found if p.get("code") == "W204"])[:300])
        check(line_of("area") and ("W206", line_of("area")) in placed,
              "B W206 is at the area line of mod.ltx", json.dumps([p for p in found if p.get("code") == "W206"])[:300])
        cmd("world.set", levels="", unlock="qa_no_changer")

        patrols = cmd("world.lookup", type="patrols", query="agr_", limit=5)
        agr_patrols = patrols.get("patrols", [])
        check(len(agr_patrols) > 0 and patrols.get("total", 0) > len(agr_patrols),
              "B the game's own patrol paths are listed", json.dumps(patrols)[:300])

        # ------------------------------------------------------------ U: the World Rules tab
        run_ui_checks(ctx.shots)

        # ------------------------------------------------------------ C: the scene
        # the editor does not reopen the last scene by itself on every start: ask for it
        scene_now = (cmd("scene.info").get("file") or "").lower()
        if "l01_escape" not in scene_now:
            opened = cmd("scene.open", timeout=900, path="rawdata/levels/l01_escape.level")
            if not check(opened.get("ok"), "C the project's Escape scene opens", json.dumps(opened)[:200]):
                # everything below stands on that scene: waiting for one that did not open only burns minutes
                raise RuntimeError("the Escape scene did not open")
        check(wait_scene("l01_escape"), "C the project's Escape scene is open")
        start = cmd("scene.place", kind="spawn", section="player_start", position=[-223.0, -20.0, -143.0], on_surface=True)
        start_name = placed_name(start)
        check(start.get("ok") and start_name, "C a Player Start is placed", json.dumps(start)[:300])
        g = cmd("world.info")
        starts = g.get("starts", [])
        check(len(starts) == 1 and starts[0]["source"] == "scene" and starts[0]["level"] == "l01_escape",
              "C world.info sees it in the scene at once", json.dumps(starts))
        check("W205" in " ".join(g.get("problems", [])), "C a start outside the area is W205", json.dumps(g.get("problems")))

        way = cmd("scene.place", kind="way", name="qa_patrol", points=[[-220, -20, -140], [-210, -20, -140], [-210, -20, -150]],
                  on_surface=True)
        check(way.get("ok") and way.get("points") == 3, "C a way path is drawn over MCP", json.dumps(way)[:300])
        again = cmd("scene.place", kind="way", name="qa_patrol", points=[[-200, -20, -140]])
        check(not again.get("ok"), "C a second object of the same name is refused")
        game_named = agr_patrols[0] if agr_patrols else ""
        clash = cmd("scene.place", kind="way", name=game_named, points=[[-230, -20, -140], [-235, -20, -145]]) if game_named else {}
        check(clash.get("ok"), "C a path named like one of the game's can be drawn", json.dumps(clash)[:200])

        stalker = ""
        roster = cmd("game.lookup", type="spawn_section", query="stalker", limit=400).get("entries", [])
        for row in roster:
            if row.get("extra") != "AI_STL_S":
                continue
            placed = cmd("scene.place", kind="spawn", section=row["id"], position=[-215.0, -20.0, -145.0], on_surface=True)
            if placed.get("ok"):
                stalker = placed_name(placed)
                break
        check(bool(stalker), "C a stalker is placed", str(len(roster)))

        def order_of(name: str) -> dict:
            got = (cmd("scene.get", objects=[name], keys=["order"]).get("objects") or [{}])[0]
            return {p["key"]: p.get("value") for p in got.get("properties") or []}

        o = order_of(stalker)
        check(o.get("order.kind") == "None", "C a fresh stalker has no order", json.dumps(o)[:300])
        o = cmd("scene.set", objects=[stalker], key="order.kind", value="patrol")
        check(not o.get("ok") and "route" in o.get("error", ""), "C a patrol without a route is refused", json.dumps(o))
        o = cmd("scene.set", objects=[stalker], set={"order": {"kind": "patrol", "route": "qa_patrol", "run": True, "hostile": True}})
        got = order_of(stalker)
        check(o.get("ok") and got.get("order.kind") == "Patrol" and got.get("order.route") == "qa_patrol"
              and got.get("order.run") is True and got.get("order.hostile") is True,
              "C the order is the stalker's: a patrol of the route, running, hostile", json.dumps(got))
        o = cmd("scene.set", objects=[start_name], key="order.kind", value="guard")
        check(not o.get("ok"), "C a Player Start takes no order")

        e = cmd("level.export", kind="spawn", all=True)
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
        second = cmd("scene.place", kind="spawn", section="player_start", position=[-205.0, -20.0, -150.0])
        g = cmd("world.info")
        probs = " ".join(g.get("problems", []))
        check("E201" in probs and start_name in probs and placed_name(second) in probs,
              "D two Player Starts are E201, both named", probs)
        ex = cmd("mod.build", target="folder", path=out_dir)
        check(not ex.get("ok") and "E201" in ex.get("error", ""), "D the build refuses them", json.dumps(ex)[:300])
        cmd("scene.delete", objects=[placed_name(second)])
        ex = cmd("mod.build", target="folder", path=out_dir)
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
        look = cmd("game.lookup", type="level_changer", query="", limit=500)
        entries = look.get("entries", [])
        check(len(entries) > 20, "E level_changer is a picker type", json.dumps(look)[:300])
        changer = next((x["id"] for x in entries if x.get("id") == way_down), entries[0]["id"] if entries else "")
        qpath = os.path.join(project, "quests", "qa_world_roles.nqasset")
        os.makedirs(os.path.dirname(qpath), exist_ok=True)
        with open(qpath, "w", encoding="utf-8") as f:
            f.write(QUEST_E024.replace("%CHANGER%", changer))
        v = cmd("asset.check", path=qpath)
        vp = " ".join(p if isinstance(p, str) else json.dumps(p) for p in v.get("problems", []))
        check("E024" in vp and "a quest has no roles" in vp, "E { role = } in a quest is E024", vp[:400])
        check("W060" not in vp, "E a changer of the linked game is known", vp[:400])
        cmd("file.delete", path="quests/qa_world_roles.nqasset")

        # ------------------------------------------------------------ F: stock again
        s = cmd("world.set", preset="stock", area="")
        check(world(s).get("declared") is False, "F preset=stock is the stock game again", json.dumps(world(s)))
        check("[world]" not in read(mod_ltx), "F and mod.ltx carries no [world]")

        # ------------------------------------------------------------ G: the rules as the tab draws them
        cmd("world.set", original_npcs=False, random_stalkers=True, mutants=True, player_choice="mutants")
        cmd("world.view", open=True)
        labels = {c.get("label") for c in cmd("ui.audit", id="world_rules").get("controls", [])}
        check({"Remove", "Keep", "Player Decides"} <= labels, "G each rule is one segmented control: Remove, Keep, Player Decides",
              json.dumps(sorted(x for x in labels if x))[:400])
        k = cmd("world.set", start_kit="wpn_knife, bandage=2")
        check(k.get("ok") and len(k.get("start_kit", {}).get("items", [])) == 2, "G the start kit is a list of rows",
              json.dumps(k.get("start_kit"))[:300])
        removed = cmd("world.set", start_kit_remove=True)
        check(removed.get("ok") and not removed.get("start_kit", {}).get("declared"), "G Remove Kit takes it out",
              json.dumps(removed.get("start_kit"))[:200])
        cmd("world.set", preset="stock")

        # ------------------------------------------------------------ R: a radio
        radio_checks(project, module, ctx.shots)
    finally:
        ed.close()
        qa.remove_tree(out_dir)


def main() -> int:
    # C to F place things in the Escape scene
    return qa.suite("WORLD", run, doc=__doc__, ignore=(), requires=["rawdata/levels/l01_escape.level"])


if __name__ == "__main__":
    sys.exit(main())
