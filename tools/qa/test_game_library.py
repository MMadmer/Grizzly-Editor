#!/usr/bin/env python3
"""The game as the editor's library and a module's scene over a level of the game (docs/GAME_LIBRARY.md), over MCP on
a THROW-AWAY COPY of a project whose game is linked. The game is only ever read; the library the answers come from is
the editor's own data. A machine without a linked game notes it and checks nothing.

  A  the library: game.library answers its state at once; asset.find lists the game's
     levels, models (a size filter keeps to it), sections (people by the size of the model they wear; a class no
     section has is refused with the ones there are) and sounds; asset.card answers a model's box and a level's
     passport; game.read reads a config by lines and refuses a path that climbs out of the game
  B  a scene over a level: scene.new base=<level> holds the level's objects marked the level's own and none of its
     own; saved, the .scene says `scene = 2` and `base`, and no file of a kind exists; an object written into
     objects.lua by hand is the scene's own (no `authored` asked); a level's object cannot be moved or deleted, a
     level's spawn point deleted is an entry `{ name, removed = true }` of spawns.lua and nothing else of the level
     is written; opened again the scene is the same, the spawn point standing marked as taken out; a duplicate of a
     level's spawn point is the scene's own object and the level's own stay out of a group; a model the game lacks
     is S220 at its line
  C  pictures without a window: asset.preview (the PNG is the size answered, the same call gives the same bytes),
     asset.thumbnail, shot.scene from above (the legend names the scene's own object)
  D  what ships: level.build on the scene writes the spawn layer and levels\\<level>\\overlay.things with a block for
     each thing the module adds - a sound source, a particle effect, a spot light that shines in the game, a glow, a
     fog volume and a sound environment -; a directional light asked to shine in the game is S221, a sector over a
     level of the game S222, a fog profile the game lacks S220, a model the game draws nothing for S224 on the
     object that wears it (a stash's own unseen case is none) and named in the build's answer; the linked game is
     not written
  E  the module's own things over the level: an item asset is a spawn section (game.lookup lists it, a spawn point
     of it written by hand checks clean); a model asset over a model of the game (`source = "game:..."`) checks
     clean, model.info says what it is made over, a static and a dynamic one stand in the scene; the module built to
     a folder ships the dynamic one as its own .ogf wearing the texture the asset gave it, the spawn layer names the
     item's section and the model, and the game's file is not among what ships
  F  Base Scenes: the Library lists the game's levels under `game`; opening the level the project has a scene over
     opens that scene, opening another level makes `rawdata\\levels\\<level>.scene` over it

    python tools/qa/test_game_library.py --project D:\\XFinedProjects\\Test [--only A,C] [--shots <folder>]
"""
from __future__ import annotations

import hashlib
import json
import os
import struct
import sys
import tempfile
import time

import qa
from qa import check, cmd, short

PHASES = "ABCDEF"
LEVEL = "l01_escape"
SCENE = "rawdata/levels/qa_over.scene"
OWN = "qa_own_box"


class Ctx:
    project = ""
    model = ""
    at = (0.0, 0.0, 0.0)
    spawn = ""
    shots: str | None = None


CTX = Ctx()


def scene_dir() -> str:
    return os.path.join(CTX.project, "rawdata", "levels", "qa_over")


def scene_file(name: str) -> str:
    return os.path.join(scene_dir(), name)


def png_size(data: bytes) -> tuple[int, int]:
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return (0, 0)
    return struct.unpack(">II", data[16:24])


def picture(answer: dict) -> bytes:
    try:
        return qa.png_of(answer)
    except Exception:  # noqa: BLE001 - an answer without a picture is the failure the check reports
        return b""


def game_listing(root: str) -> dict[str, tuple[int, int]]:
    """The game's own top folders that a module build never touches, by size and time: equal before and after."""
    out = {}
    for top in ("gamedata", "db", "database"):
        folder = os.path.join(root, top)
        if not os.path.isdir(folder):
            continue
        for entry in os.scandir(folder):
            st = entry.stat()
            out[top + "/" + entry.name] = (st.st_size, int(st.st_mtime))
    return out


# ---- A: the library -------------------------------------------------------------------------------------------------

def run_library() -> None:
    state = cmd("game.library")
    check(state.get("ok") and state.get("state") in ("ready", "missing", "syncing"), "A game.library answers its state",
          short(state))
    levels = cmd("asset.find", kind="level", limit=500, timeout=900)
    names = [r.get("name") for r in levels.get("rows", [])]
    check(levels.get("ok") and LEVEL in names, "A asset.find kind=level lists the game's levels", short(names))
    row = next((r for r in levels.get("rows", []) if r.get("name") == LEVEL), {})
    check(row.get("size") and (row.get("counts") or {}).get("spawns", 0) > 0 and (row.get("ai") or {}).get("nodes", 0) > 0,
          "A a level's row says its size, what it holds and its AI map", short(row))

    models = cmd("asset.find", kind="model", query="box", size=[0.3, 1.5], limit=20)
    rows = models.get("rows", [])
    sizes = [max(r.get("size") or [0]) for r in rows]
    check(models.get("ok") and rows and all(0.3 <= s <= 1.5 for s in sizes), "A asset.find kind=model keeps to the size asked",
          short(models))
    if rows:
        # a row's `ref` is what a scene writes: the model's name with `.ogf` (either slash)
        CTX.model = rows[0]["ref"].replace(chr(92), "/")
    card = cmd("asset.card", ref=CTX.model) if CTX.model else {}
    check(card.get("ok") and json.dumps(card).count("box") > 0 and "surfaces" in json.dumps(card),
          "A asset.card answers a model's box and surfaces", short(card))
    passport = cmd("asset.card", ref=LEVEL, kind="level")
    text = json.dumps(passport)
    check(passport.get("ok") and "ground" in text and "structures" in text, "A asset.card answers a level's passport",
          text[:300])
    sections = cmd("asset.find", kind="section", **{"class": "npc"}, size=[1.0, 2.5], limit=5)
    check(sections.get("ok") and sections.get("total", 0) > 0 and all(r.get("kind") == "npc" for r in sections.get("rows", [])),
          "A asset.find kind=section lists people by the size of the model they wear", short(sections))
    wrong = cmd("asset.find", kind="section", **{"class": "stalker"})
    check(not wrong.get("ok") and "npc" in wrong.get("error", ""), "A a class no section has is refused with the ones there are",
          short(wrong))
    sounds = cmd("asset.find", kind="sound", query="fire", limit=3)
    check(sounds.get("ok") and sounds.get("total", 0) > 0, "A asset.find kind=sound lists the game's sounds", short(sounds))

    read = cmd("game.read", path="configs/system.ltx", lines=3)
    check(read.get("ok") and "#include" in json.dumps(read), "A game.read reads a config by lines", short(read))
    for bad in ("../fsgame.ltx", "..\\..\\Windows\\win.ini", "C:/Windows/win.ini"):
        r = cmd("game.read", path=bad)
        check(not r.get("ok"), "A game.read refuses %s" % bad, short(r))


# ---- B: a scene over a level ----------------------------------------------------------------------------------------

def own_objects() -> dict:
    return cmd("scene.objects", own=True, limit=50)


def run_scene() -> None:
    new = cmd("scene.new", base=LEVEL, discard=True, timeout=900)
    check(new.get("ok"), "B scene.new base=%s answers" % LEVEL, short(new))
    everything = cmd("scene.objects", limit=1)
    own = own_objects()
    check(everything.get("total", 0) > 100 and own.get("ok") and own.get("total") == 0,
          "B the level's objects stand in the scene and none is the scene's own", short([everything.get("total"), own]))
    spawns = cmd("scene.objects", classes=["spawn"], limit=30)
    statics = cmd("scene.objects", classes=["object"], limit=1)
    check(all(o.get("level") for o in spawns.get("objects", []) + statics.get("objects", [])),
          "B each answers `level: true`", short(spawns))
    saved = cmd("scene.save", path=SCENE)
    main = qa.read(os.path.join(CTX.project, *SCENE.split("/")))
    check(saved.get("ok") and "scene = 2" in main and 'base = "%s"' % LEVEL in main, "B the saved .scene says its version and its base",
          main[:200])
    check(not os.path.isdir(scene_dir()) or not os.listdir(scene_dir()), "B and no file of a kind exists: the level is not written",
          short(os.listdir(scene_dir()) if os.path.isdir(scene_dir()) else []))

    # an object of the module's, written by hand: where the level's first walker spawn point stands
    walker = next((o for o in spawns.get("objects", []) if o.get("position")), {})
    x, y, z = walker.get("position", [0.0, 0.0, 0.0])
    ground = cmd("scene.query", ground=[[x + 2.0, z + 2.0]])
    hit = (ground.get("ground") or [{}])[0]
    CTX.at = (x + 2.0, hit.get("y", y), z + 2.0)
    os.makedirs(scene_dir(), exist_ok=True)
    line = '    { name = "%s", pos = { %.2f, %.2f, %.2f }, ref = "%s" },' % (OWN, *CTX.at, CTX.model) + chr(10)
    qa.write(scene_file("objects.lua"), "return {\n  objects = {\n" + line + "  },\n}\n")
    own = own_objects()
    mine = next((o for o in own.get("objects", []) if o.get("name") == OWN), None)
    check(own.get("total") == 1 and mine and not mine.get("level"), "B an object written into objects.lua is the scene's own, at once",
          short(own))
    before = qa.read(scene_file("objects.lua"))

    # the level's own cannot be changed
    moved = cmd("scene.set", objects=[statics["objects"][0]["name"]], position=[1, 2, 3])
    check(not moved.get("ok"), "B a level's object is not moved", short(moved))
    gone = cmd("scene.delete", objects=[statics["objects"][0]["name"]])
    check(not gone.get("ok"), "B a level's static object is not deleted", short(gone))
    # a level's spawn point taken out: one entry of spawns.lua
    CTX.spawn = spawns["objects"][0]["name"]
    out = cmd("scene.delete", objects=[CTX.spawn])
    saved = cmd("scene.save")
    stubs = qa.read(scene_file("spawns.lua")) if os.path.isfile(scene_file("spawns.lua")) else ""
    check(out.get("ok") and saved.get("ok") and 'name = "%s"' % CTX.spawn in stubs and "removed = true" in stubs
          and stubs.count("name =") == 1, "B a level's spawn point deleted is one entry `removed = true` of spawns.lua",
          short([out, saved, stubs]))
    check(qa.read(scene_file("objects.lua")) == before and sorted(os.listdir(scene_dir())) == ["objects.lua", "spawns.lua"],
          "B and the save wrote nothing else: the object's file is untouched", short(sorted(os.listdir(scene_dir()))))

    opened = cmd("scene.open", path=SCENE, timeout=900)
    own = own_objects()
    left = [o for o in cmd("scene.objects", query=CTX.spawn, limit=5).get("objects", []) if o.get("name") == CTX.spawn]
    check(opened.get("ok") and own.get("total") == 1 and len(left) == 1 and left[0].get("removed") is True
          and CTX.spawn in (opened.get("base") or {}).get("removed", []),
          "B opened again: the scene's own object is there, the spawn point stands marked as taken out",
          short([own, left, (opened.get("base") or {})]))

    # a copy of one of the level's objects is the scene's own; the level's own stay out of a group
    lamp = next((o["name"] for o in spawns.get("objects", []) if o["name"] != CTX.spawn), "")
    dup = cmd("scene.duplicate", objects=[lamp], offset=[1.0, 0.0, 0.0])
    own = own_objects()
    copy = [o for o in own.get("objects", []) if o.get("name") != OWN]
    check(dup.get("ok") and own.get("total") == 2 and len(copy) == 1 and not copy[0].get("level"),
          "B a duplicate of a level's spawn point is the scene's own object", short([dup, own]))
    cmd("scene.select", objects=[lamp])
    grouped = cmd("scene.group", action="group", objects=[lamp], name="qa_group")
    check(not grouped.get("ok") and "level's own" in grouped.get("error", ""), "B the level's own stay out of a group",
          short(grouped))
    removed = cmd("scene.delete", objects=[copy[0]["name"]]) if copy else {}
    check(removed.get("ok") and own_objects().get("total") == 1, "B and the copy is deleted like any object of the scene",
          short(removed))
    cmd("scene.save")

    # a name the game lacks
    wrong = line.replace(OWN, "qa_wrong").replace(CTX.model, "no/such/model.ogf")
    qa.write(scene_file("objects.lua"), "return {\n  objects = {\n" + line + wrong + "  },\n}\n")
    problems = cmd("asset.check", path=SCENE).get("problems", [])
    s220 = [p for p in problems if p.get("code") == "S220"]
    check(len(s220) == 1 and s220[0].get("line") == 4 and "objects.lua" in (s220[0].get("file") or ""),
          "B a model the game lacks is S220 at its line", short(problems))
    qa.write(scene_file("objects.lua"), before)
    check(own_objects().get("total") == 1, "B the file put back is followed again", "")


# ---- C: pictures ----------------------------------------------------------------------------------------------------

def run_pictures() -> None:
    ref = CTX.model
    a = cmd("asset.preview", ref=ref, views=["iso", "top"], size=96)
    b = cmd("asset.preview", ref=ref, views=["iso", "top"], size=96)
    pa, pb = picture(a), picture(b)
    check(a.get("ok") and png_size(pa) == (a.get("width"), a.get("height")) and len(a.get("views", [])) == 2
          and all(v.get("px_per_m", 0) > 0 for v in a.get("views", [])),
          "C asset.preview answers a sheet of the size it says, each tile with its scale", short(a))
    check(pa and hashlib.sha256(pa).digest() == hashlib.sha256(pb).digest(), "C the same call gives the same bytes", "")
    if CTX.shots and pa:
        with open(os.path.join(CTX.shots, "gamelib_preview.png"), "wb") as f:
            f.write(pa)
    thumbs = cmd("asset.thumbnail", refs=[ref], size=64)
    item = (thumbs.get("items") or [{}])[0]
    check(thumbs.get("ok") and png_size(picture(thumbs))[0] == thumbs.get("width") and item.get("size"),
          "C asset.thumbnail answers a sheet with the model's size", short(thumbs))
    x, _, z = CTX.at
    shot = cmd("shot.scene", view="top", area=[x - 8, z - 8, x + 8, z + 8], width=320, height=320)
    data = picture(shot)
    legend = {row.get("name"): row for row in shot.get("legend", [])}
    check(shot.get("ok") and png_size(data) == (320, 320) and OWN in legend and legend[OWN].get("px"),
          "C shot.scene draws the place from above and its legend names the scene's own object", short(shot))
    if CTX.shots and data:
        with open(os.path.join(CTX.shots, "gamelib_scene.png"), "wb") as f:
            f.write(data)


# ---- D: what ships --------------------------------------------------------------------------------------------------

def run_ships() -> None:
    game = cmd("game.library").get("game", "")
    listed = game_listing(game) if game else {}
    x, y, z = CTX.at
    sound = (cmd("asset.find", kind="sound", query="fire", limit=1).get("rows") or [{}])[0].get("name", "")
    effect = (cmd("asset.find", kind="particle", query="fire", limit=1).get("rows") or [{}])[0].get("name", "")
    check(bool(sound) and bool(effect), "D the game has a sound and an effect to place", short([sound, effect]))
    esc = lambda s: s.replace("\\", "\\\\")  # noqa: E731
    qa.write(scene_file("sounds.lua"), 'return {\n  sources = {\n    { name = "qa_fire", pos = { %.2f, %.2f, %.2f }, sound = "%s" },\n  },\n}\n'
             % (x, y + 0.5, z, esc(sound)))
    qa.write(scene_file("particles.lua"), 'return {\n  particles = {\n    { name = "qa_smoke", pos = { %.2f, %.2f, %.2f }, ref = "%s" },\n  },\n}\n'
             % (x, y + 0.5, z, esc(effect)))
    check(own_objects().get("total") == 3, "D a sound source and an effect written by hand are the scene's own", short(own_objects()))
    # the kinds a level of the game takes from a module's text: a spot light, a glow, fog, a sound environment
    glow = next((r.get("name", "") for r in cmd("asset.find", kind="texture", query="glow", limit=40).get("rows", [])
                 if r.get("name", "").lower().startswith("glow")), "")
    envs = [r.get("name", "") for r in cmd("asset.find", kind="sound_env", limit=4).get("rows", [])]
    check(bool(glow) and len(envs) >= 2, "D the game has a glow texture and sound environments to name", short([glow, envs]))
    qa.write(scene_file("lights.lua"),
             'return {\n  lights = {\n'
             '    { name = "qa_beam", pos = { %.2f, %.2f, %.2f }, rot = { -90, 0, 0 }, kind = "spot", cone = 60, dynamic = true },\n'
             '    { name = "qa_second_sun", pos = { %.2f, %.2f, %.2f }, kind = "directional", lightmap = false, dynamic = true },\n'
             '  },\n}\n' % (x, y + 4.0, z, x, y + 8.0, z))
    qa.write(scene_file("glows.lua"),
             'return {\n  glows = {\n'
             '    { name = "qa_glow", pos = { %.2f, %.2f, %.2f }, texture = "%s", radius = 0.5 },\n'
             '    { name = "qa_glow_odd", pos = { %.2f, %.2f, %.2f }, texture = "%s", radius = 0.02, shader = "default", fixed_size = true },\n'
             '  },\n}\n' % (x, y + 3.8, z, esc(glow), x + 1.0, y + 3.8, z, esc(glow)))
    qa.write(scene_file("fog.lua"),
             'return {\n  volumes = {\n'
             '    { name = "qa_mist", pos = { %.2f, %.2f, %.2f }, scale = { 4, 2, 4 }, kind = "emitter", group = 1 },\n'
             '    { name = "qa_mist_wall", pos = { %.2f, %.2f, %.2f }, scale = { 1, 2, 1 }, kind = "occlusion", group = 1 },\n'
             '    { name = "qa_no_profile", pos = { %.2f, %.2f, %.2f }, kind = "emitter", group = 2, profile = "environment/fog/qa_nothing.ltx" },\n'
             '  },\n}\n' % (x + 6.0, y + 1.0, z, x + 6.5, y + 1.0, z, x + 12.0, y + 1.0, z))
    sounds = qa.read(scene_file("sounds.lua")).replace("\r\n", "\n")
    env = '  environments = {\n    { name = "qa_hall", pos = { %.2f, %.2f, %.2f }, scale = { 6, 3, 6 }, inner = "%s", outer = "%s" },\n  },\n' % (
        x, y + 1.5, z, envs[0] if envs else "", envs[1] if len(envs) > 1 else "")
    qa.write(scene_file("sounds.lua"), sounds[:sounds.rindex("}")] + env + "}\n")
    qa.write(scene_file("zones.lua"), 'return {\n  sectors = {\n    { name = "qa_sector", default = true },\n  },\n}\n')
    # a model the game draws nothing for, worn by a physics object and by a stash, whose own model is such
    unseen = next((r.get("ref", "") for r in cmd("asset.find", kind="model", query="equipment_box", limit=40).get("rows", [])
                   if r.get("invisible")), "")
    check(unseen.lower().endswith(".ogf"), "D the library marks the models the game draws nothing for", unseen)
    spawns = qa.read(scene_file("spawns.lua")).replace("\r\n", "\n")
    worn = ('\n    { name = "qa_unseen", pos = { %.2f, %.2f, %.2f }, section = "physic_object", props = { visual = "%s" } },'
            '\n    { name = "qa_cache", pos = { %.2f, %.2f, %.2f }, section = "inventory_box", props = { visual = "%s" } },'
            % (x - 3.0, y + 0.3, z, esc(unseen[:-4]), x - 4.5, y + 0.3, z, esc(unseen[:-4])))
    at = spawns.index("spawns = {") + len("spawns = {")
    qa.write(scene_file("spawns.lua"), spawns[:at] + worn + spawns[at:])
    # a config of the game that is a file and no profile of a fog volume (one that takes no lines from
    # another file: what an #include brings nobody read, and such a file is not judged)
    qa.write(scene_file("fog.lua"), qa.read(scene_file("fog.lua")).replace("\r\n", "\n").replace(
        '  },\n}\n', '    { name = "qa_not_a_profile", pos = { %.2f, %.2f, %.2f }, kind = "emitter", group = 3, profile = "creatures/game_relations.ltx" },\n  },\n}\n'
        % (x + 18.0, y + 1.0, z)))
    want = {"qa_beam", "qa_second_sun", "qa_glow", "qa_glow_odd", "qa_mist", "qa_mist_wall", "qa_no_profile",
            "qa_not_a_profile", "qa_hall", "qa_sector", "qa_unseen", "qa_cache"}
    own = qa.until(lambda: (lambda o: o if want <= {q.get("name") for q in o.get("objects", [])} else None)(own_objects()), 20) or {}
    check(bool(own), "D a spot light, a glow, fog volumes, a sound environment, a sector and two spawn points written by hand "
          "stand in the scene", short(own_objects(), 600))
    problems = cmd("asset.check", path=SCENE).get("problems", [])
    errors = [p for p in problems if p.get("severity") == "error"]
    check(not errors, "D the scene checks without an error", short(errors))
    said = lambda code, name: [q for q in problems if q.get("code") == code and name in q.get("text", "")]  # noqa: E731
    check(len(said("S221", "qa_second_sun")) == 1 and "lights.lua" in (said("S221", "qa_second_sun")[0].get("file") or ""),
          "D a directional light asked to shine in the game is S221 at its line", short(problems, 600))
    check(len(said("S222", "qa_sector")) == 1, "D a sector over a level of the game is S222", short(problems, 600))
    check(len(said("S220", "qa_nothing")) == 1 and not said("S220", "qa_mist ") and not said("S220", glow),
          "D a fog profile the game lacks is S220, and the names it has are not", short(problems, 600))
    check(len(said("S224", "qa_unseen")) == 1 and "spawns.lua" in (said("S224", "qa_unseen")[0].get("file") or "")
          and not said("S224", "qa_cache"),
          "D a model the game draws nothing for is S224 at its object, and a stash's own unseen case is not", short(problems, 600))
    check(len(said("S225", "qa_glow_odd")) == 1 and not said("S225", "qa_glow "),
          "D a glow's shader no glow is drawn with is S225, and a glow that names none is not", short(problems, 600))
    not_profile = said("S220", "qa_not_a_profile")
    check(len(not_profile) == 1 and "[volume]" in not_profile[0].get("text", ""),
          "D a config that is no fog profile is S220 with what it lacks", short(problems, 900))
    built = cmd("level.build", timeout=1200)
    check(built.get("ok"), "D level.build builds what the module adds to the level", short(built, 900))
    layer = next((s.get("note", {}) for s in built.get("ran", []) if s.get("step") == "spawn_layer"), {})
    told = [w for w in layer.get("warnings", []) if "draws nothing" in w]
    check(len(told) == 1 and "'qa_unseen'" in told[0] and layer.get("invisible") == 1,
          "D and the build names the object that wears it in its answer", short(layer, 600))
    counts = layer.get("things", {})
    # a model of the game placed as a static object is compiled as level geometry: no part of the piece
    # keeps a shader for models, and the step says what it took in its place
    piece = next((s.get("note", {}) for s in built.get("ran", []) if s.get("step") == "piece"), {})
    worn = cmd("level.inspect", level=LEVEL, part="visuals", piece="main", limit=200)
    names = [v.get("shader_name", "") for v in worn.get("visuals", [])]
    check(worn.get("ok") and worn.get("total", 0) > 0 and not any(n.lower().startswith("models\\") for n in names),
          "D the piece's visuals are read, and none keeps a shader for models", short(sorted(set(names)), 400))
    was_model = any("models\\" in (s.get("shader") or "").lower()
                    for s in (cmd("asset.card", ref=CTX.model).get("card", {}).get("surfaces") or []))
    check(not was_model or any("shader for level geometry" in n for n in piece.get("notes", [])),
          "D and the build says which shader for models it compiled as which", short(piece.get("notes"), 400))
    # a forced build of a scene over the level builds the module's layers again and nothing of the level
    again = cmd("level.build", force=True, timeout=1200)
    check(again.get("ok") and {s.get("step") for s in again.get("ran", [])} <= {"spawn_layer", "piece"}
          and any(s.get("step") == "spawn_layer" for s in again.get("ran", [])),
          "D a forced build builds the module's own layers again and compiles nothing of the level", short(again, 600))
    named = cmd("level.build", only="geometry", timeout=120)
    check(not named.get("ok") and "holds nothing of the level to compile" in (named.get("error") or ""),
          "D and a step that compiles the level, asked by name, is refused in a sentence", short(named, 400))
    check(counts.get("glows") == 2 and counts.get("fogs") == 3 and counts.get("sound_envs") == 1 and counts.get("lights") == 1,
          "D the build answers how many things of each kind the file holds", short(counts))
    things = os.path.join(CTX.project, "levels", LEVEL, "overlay.things")
    text = qa.read(things) if os.path.isfile(things) else ""
    check("[sound:qa_fire]" in text and "[particles:qa_smoke]" in text and CTX.spawn not in text,
          "D overlay.things holds a block for the sound and one for the effect", text[:400])
    check("[light:qa_beam]" in text and "type = spot" in text and "qa_second_sun" not in text,
          "D the spot light that shines in the game is a block of its own, the directional one is none", text[:900])
    check("[glow:qa_glow]" in text and "[sound_env:qa_hall]" in text and "size = 6, 3, 6" in text
          and "[fog:qa_mist]" in text and "occluder = " in text,
          "D a glow, a sound environment as a box and a fog volume with what it flows around are blocks too", text[-900:])
    odd = text.split("[glow:qa_glow_odd]")[1].split("[")[0] if "[glow:qa_glow_odd]" in text else ""
    check("fixed = true" in odd and "shader" not in odd,
          "D a glow of a fixed size says so, and a shader no glow is drawn with is not written", odd)
    # a file somebody wrote is theirs: the build leaves it as it is and says so
    theirs = "; my own things\r\n[glow:hand]\r\nposition = 0, 2, 0\r\ntexture = glow\\glow_white\r\n"
    qa.write(things, theirs)
    kept = cmd("level.build", only="spawn_layer", force=True, timeout=600)
    told = next((s.get("note", {}).get("warnings", []) for s in kept.get("ran", []) if s.get("step") == "spawn_layer"), [])
    check(kept.get("ok") and qa.read(things) == theirs and any("was not made by the level build" in w for w in told),
          "D a things file somebody wrote is neither replaced nor removed, and the build says so", short(told, 400))
    os.remove(things)
    # the checks' fixtures leave: the next phases read the scene without their warnings
    for name in ("lights.lua", "glows.lua", "fog.lua", "zones.lua"):
        os.remove(scene_file(name))
    qa.write(scene_file("sounds.lua"), sounds)
    qa.write(scene_file("spawns.lua"), spawns)
    qa.until(lambda: own_objects().get("total") == 3 or None, 20)
    if game:
        check(game_listing(game) == listed, "D the linked game is as it was", "")


# ---- E: the module's own things -------------------------------------------------------------------------------------

def run_own_things() -> None:
    module = cmd("mod.info").get("manifest", {}).get("id", "")
    check(bool(module), "E the project has a module id", module)
    x, y, z = CTX.at
    esc = lambda s: s.replace("\\", "\\\\")  # noqa: E731
    # an item of the module: a section the game makes of its asset
    qa.write(os.path.join(CTX.project, "items", "qa_tin.itemasset"),
             'return {\n  item = 1,\n  id = "qa_tin",\n  base = "bread",\n  name = "QA tin",\n  set = { cost = 7 },\n}\n')
    section = "%s_qa_tin" % module
    listed = qa.until(lambda: (lambda a: a if a.get("total") else None)(
        cmd("game.lookup", type="spawn_section", query=section)), 20) or {}
    check(any(e.get("id") == section for e in listed.get("entries", [])), "E the item's section is a spawn section",
          short(listed))
    # the game's model the scene already wears, dressed by two model assets
    name = CTX.model[:-4] if CTX.model.lower().endswith(".ogf") else CTX.model
    theirs = (cmd("model.info", refs=[CTX.model]).get("models") or [{}])[0]
    surface = (theirs.get("surfaces") or [{}])[0]
    worn, leaf = surface.get("texture", ""), surface.get("name", "")
    # a texture name plain enough to look for in the model's bytes
    other = next((r.get("name", "") for r in cmd("asset.find", kind="texture", query="wood", limit=40).get("rows", [])
                  if r.get("name", "").isascii() and " " not in r.get("name", "") and r.get("name") != worn), "")
    check(bool(worn) and bool(leaf) and bool(other), "E the game's model says what it wears and the game has another texture",
          short([worn, leaf, other, theirs], 400))
    row = '    { name = "%s", texture = "%s" },' % (leaf, esc(other))
    head = 'return {\n  mesh = 1,\n%s  source = "game:%s.ogf",\n  surfaces = {\n%s\n  },\n}\n'
    qa.write(os.path.join(CTX.project, "models", "qa_static.meshasset"), head % ("", name, row))
    qa.write(os.path.join(CTX.project, "models", "qa_dynamic.meshasset"),
             head % ('  kind = "dynamic",\n  name = "%s\\\\qa_box",\n' % module, name, row))
    checked = cmd("asset.check", path="models")
    check(checked.get("ok") and checked.get("files") == 2 and not checked.get("problems"),
          "E the model assets over the game's model check clean", short(checked, 600))
    info = cmd("model.info", refs=["models/qa_static.meshasset", "models/qa_dynamic.meshasset"])
    models = info.get("models", [])
    wanted = name.replace("/", "\\").lower()
    check(len(models) == 2 and all((m.get("over") or {}).get("game") == wanted for m in models)
          and [m.get("kind") for m in models] == ["static", "dynamic"]
          and all(any(s.get("texture") == other for s in m.get("surfaces", [])) for m in models),
          "E model.info says what each is made over and what it wears now", short(info, 700))
    lines = [
        '    { name = "%s", pos = { %.2f, %.2f, %.2f }, ref = "%s" },' % (OWN, x, y, z, CTX.model),
        '    { name = "qa_static_box", pos = { %.2f, %.2f, %.2f }, ref = "models/qa_static.meshasset" },' % (x + 1.5, y, z),
        '    { name = "qa_dynamic_box", pos = { %.2f, %.2f, %.2f }, ref = "models/qa_dynamic.meshasset" },' % (x + 3.0, y, z),
    ]
    qa.write(scene_file("objects.lua"), "return {\n  objects = {\n" + "\n".join(lines) + "\n  },\n}\n")
    spawns = qa.read(scene_file("spawns.lua")).replace("\r\n", "\n")
    tin = '{ name = "qa_tin_here", pos = { %.2f, %.2f, %.2f }, section = "%s" },' % (x - 1.5, y + 0.3, z, section)
    at = spawns.index("spawns = {") + len("spawns = {")
    qa.write(scene_file("spawns.lua"), spawns[:at] + "\n    " + tin + spawns[at:])
    own = qa.until(lambda: (lambda o: o if {"qa_static_box", "qa_dynamic_box", "qa_tin_here"} <= {
        x.get("name") for x in o.get("objects", [])} else None)(own_objects()), 20) or {}
    check(bool(own), "E the item's spawn point and the two models stand in the scene", short(own_objects()))
    problems = cmd("asset.check", path=SCENE).get("problems", [])
    bad = [q for q in problems if q.get("severity") == "error" or q.get("code") in ("S021", "S220")]
    check(not bad, "E and the scene checks without an unknown section or model", short(bad, 600))
    out = tempfile.mkdtemp(prefix="grizzly_gamelib_")
    try:
        game = cmd("game.library").get("game", "")
        listed_game = game_listing(game) if game else {}
        before = time.time()
        built = cmd("mod.build", target="folder", path=out, timeout=1200)
        check(built.get("ok"), "E the module builds to a folder", short(built, 900))
        root = os.path.join(out, "modules", module)
        # the static model is level geometry: the module build builds the piece itself, no Build Level asked
        made = os.path.join(CTX.project, "levels", LEVEL, "pieces", "main", "piece.ltx")
        check(os.path.isfile(made) and os.path.getmtime(made) >= before - 1
              and os.path.isfile(os.path.join(root, "levels", LEVEL, "pieces", "main", "piece.ltx")),
              "E the module build builds the piece of the static model first and ships it", made)
        piece = {s.get("step"): s.get("state") for s in cmd("level.status", timeout=120).get("steps", [])}.get("piece")
        check(piece == "fresh", "E and the piece is fresh after it", str(piece))
        ogf = os.path.join(root, "gamedata", "meshes", module, "qa_box.ogf")
        data = open(ogf, "rb").read() if os.path.isfile(ogf) else b""
        check(other.encode("ascii", "replace") in data and worn.encode("ascii", "replace") not in data,
              "E the dynamic model ships as its own .ogf wearing the asset's texture", "%d bytes" % len(data))
        check(os.path.isfile(os.path.join(root, "gamedata", "meshes", module, "qa_box.meshasset")),
              "E with its description beside it")
        check(not os.path.exists(os.path.join(root, "models")), "E and the assets where they lie do not ship")
        layer = os.path.join(root, "spawn", "%s.xspawn" % LEVEL)
        text = qa.read(layer) if os.path.isfile(layer) else ""
        check("section   = %s" % section in text and "visual    = %s\\qa_box" % module in text,
              "E the spawn layer names the item's section and the model", text[:500])
        if game:
            check(game_listing(game) == listed_game, "E the linked game is as it was", "")
    finally:
        qa.remove_tree(out)


# ---- F: Base Scenes -------------------------------------------------------------------------------------------------

def run_base_scenes() -> None:
    saved = cmd("scene.save")
    check(saved.get("ok"), "F the scene is saved before another one opens", short(saved))
    shown = cmd("content.view", action="go", source="library", kind="base_scenes", folder="game", limit=200)
    entries = {e.get("level"): e for e in shown.get("entries", []) if e.get("level")}
    check(shown.get("ok") and LEVEL in entries, "F Base Scenes lists the game's levels under game", short(shown, 500))
    path = (entries.get(LEVEL) or {}).get("path", "game\\%s.scene" % LEVEL)
    opened = cmd("content.view", action="reveal", source="library", kind="base_scenes", path=path, open=True, timeout=900)
    now = cmd("scene.info")
    check(opened.get("ok") and (now.get("file") or "").replace("\\", "/").endswith(SCENE),
          "F opening the level the project has a scene over opens that scene", short([opened.get("error"), now.get("file")]))
    # another level: the smallest one there is
    rows = cmd("asset.find", kind="level", limit=200).get("rows", [])
    others = sorted((r for r in rows if r.get("name") != LEVEL and r.get("name") in entries),
                    key=lambda r: (r.get("counts") or {}).get("spawns", 1 << 30))
    other = others[0]["name"] if others else ""
    check(bool(other), "F the game has another level", short([r.get("name") for r in rows]))
    if not other:
        return
    made = os.path.join(CTX.project, "rawdata", "levels", "%s.scene" % other)
    opened = cmd("content.view", action="reveal", source="library", kind="base_scenes", path="game\\%s.scene" % other,
                 open=True, timeout=900)
    text = qa.read(made) if os.path.isfile(made) else ""
    now = cmd("scene.info")
    check(opened.get("ok") and 'base = "%s"' % other in text and (now.get("base") or {}).get("level") == other,
          "F opening a level without a scene makes the project's scene over it", short([opened.get("error"), text[:200]]))
    again = cmd("content.view", action="reveal", source="library", kind="base_scenes", path="game\\%s.scene" % other,
                open=True, timeout=900)
    scenes = [f for f in os.listdir(os.path.dirname(made)) if f.lower().startswith(other) and f.endswith(".scene")]
    check(again.get("ok") and scenes == ["%s.scene" % other], "F and opening it again makes no second one", short(scenes))


def run(ctx: qa.Run) -> None:
    CTX.project = ctx.project
    CTX.shots = ctx.shots
    if CTX.shots:
        os.makedirs(CTX.shots, exist_ok=True)
    ed = qa.Editor(ctx.project).start()
    try:
        state = cmd("game.library")
        if state.get("state") == "no_game":
            print("NOTE no game is linked on this machine: the game library is not checked", flush=True)
            check(True, "the suite ran (no game to check against)")
            return
        qa.run_phases({"A": run_library, "B": run_scene, "C": run_pictures, "D": run_ships, "E": run_own_things,
                       "F": run_base_scenes}, ctx.only, {"B": "A", "C": "AB", "D": "AB", "E": "AB", "F": "AB"})
    finally:
        ed.close()


def main() -> int:
    return qa.suite("GAME LIBRARY", run, doc=__doc__, phases=PHASES)


if __name__ == "__main__":
    sys.exit(main())
