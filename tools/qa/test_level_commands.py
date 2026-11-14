#!/usr/bin/env python3
"""Building a level, over MCP on a THROW-AWAY COPY of a project with the repository's smallest scene
(rawdata\\levels\\mp_test.level):

  A  level.status before any build: every step never built (the details, which the scene has none of, not needed);
     level.problems is an empty list; level.play action=reload_configs reads
     the linked game's configs again
  B  level.build quality=draft builds every step in order as one job (a graph point and the player placed beside the
     multiplayer spawns first: the game graph and the game's spawn need them); level.status then says every step is fresh but the geometry
     and the light, "Built at Draft quality."; the same draft build again builds nothing; the level folder holds
     what the game loads (level, level.geom, level.cform, level.ai, level.graph, level.ltx) and build.state, the
     spawn is in gamedata\\spawns
  U  building from the window: the Level toolbar's Build Level, Play and Stop are labelled actions; Build Level's
     menu lists every step under its title, why it is not fresh in the row's tip; the status bar's Build chip names
     the stale steps and opens the same popover; Validate's problems are the Problems panel's rows, and going to one
     selects its object; Edit > Delete (the action) deletes a light and a glow together; the menus' rows are actions
     off without a selection; pictures under --shots
  R  the status after a restart: the scene saved, the editor closed and started again on the copy - level.status
     says what it said before; a scene object moved - the steps that read it are stale ("1 object moved."), the
     others not
  S  Stop ends the build now: after the move, level.build wait=false and job.stop while the compiler runs - the job's
     stage named the compiler's phase by its plain name (no "..."), the job ends cancelled within seconds of the Stop,
     no compiler stage folder is left, the geometry is fresh, the light is not
  C  level.compile on a folder: refused for a folder that holds no build.prj, naming it; the level folder compiled
     again, waited for with job.wait, its report naming the compiler's phases and files
  E  level.export: the static geometry as OBJ, the spawn layer of one object named, the visuals - models that name
     the editor that built them and neither this computer nor this user
  Y  Play takes no folder it did not make: level.play start / root with root= a folder that is a project, inside one,
     holds a file of somebody else's, is the editor's or relative - every one refused before anything is built or
     written, the folders unchanged, no game started
  K  the commandlet from a cold start: Grizzly-Cmd.exe run level.status -project <copy> -json answers what the editor
     said last (the light stale), with no editor running
  L  the painted layers and the zones on the ground (every static object, no list): scene.ai_map clears and
     generates the map from the spawn points, one undo step; scene.details adds two meshes, paints, erases, fills
     and removes one, the status following; scene.wallmarks lays a mark under a point; scene.zones lists and
     validates; level.build only=detail_light builds the details and their light

  D  the doors: saving as a scene that is there, and discarding the recovery snapshots, pass the destroy door alone;
     scene.info writes no file, scene.summary writes the summary and refuses a file that is there
  F  an error ends the build: the light on a build.prj cut short fails, the answer names it and the detail light
     after it as not run; with the actor gone the spawn is refused before any step runs - in seconds, the problem on
     the step, nothing built
  G  a level the linked game has (the scene saved as l01_escape, its level path l01_escape): level.status says it is
     the game's - every compile step not needed, the Build chip asks for none -, Build Level builds the spawn layer
     alone; a compile step asked for by name still runs, and what it wrote is the warning that it replaces the
     game's files, which level.clean (the destroy door; dry_run names them) removes; Play plays the module
     (dry_run): the root's modules\<id> holds the module with Play's Player Start, the new game's options are a file
     of the root's own, no level and no editor.spawn are staged, and the command line starts a new game

    python tools/qa/test_level_commands.py --project <project> [--only A,B,...]
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import time

import qa
from qa import check, cmd, short


SCENE = "mp_test"
PHASES = "ABURSCEYKLDFG"
SHOTS: str | None = None
def listing(folder: str) -> list[tuple[str, int]]:
    out = []
    for d, _dirs, files in os.walk(folder):
        for n in files:
            p = os.path.join(d, n)
            out.append((os.path.relpath(p, folder), os.path.getsize(p)))
    return sorted(out)


def status_states() -> dict:
    st = cmd("level.status", timeout=120)
    return {s["step"]: s for s in st.get("steps", [])} if st.get("ok") else {}


class Ctx:
    project = ""
    work = ""
    level = ""
    editor: qa.Editor | None = None
    before_restart: dict = {}
    actor = ""


CTX = Ctx()


def open_small_scene() -> bool:
    # the scene as saved once it has been, else the SDK scene it is imported from
    saved = os.path.isfile(os.path.join(CTX.project, "rawdata", "levels", SCENE + ".scene"))
    rel = "rawdata/levels/%s.%s" % (SCENE, "scene" if saved else "level")
    r = cmd("scene.open", path=rel, discard=True, timeout=120)
    if not r.get("ok"):
        print("  scene.open: " + short(r), flush=True)
        return False
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        jobs = cmd("job.list", running=True).get("jobs") or []
        if not any(j.get("kind") == "scene_load" for j in jobs):
            return True
        time.sleep(0.2)
    return False


def spawn_positions() -> list[tuple[str, list[float]]]:
    """The spawn points of the scene, by name, with their positions."""
    r = cmd("scene.objects", classes=["spawn"], limit=1000)
    out = []
    for o in r.get("objects") or []:
        p = o.get("position")
        if isinstance(p, list) and len(p) == 3:
            out.append((o.get("name"), p))
    return out


def level_dir() -> str:
    return os.path.join(CTX.project, "gamedata", "levels", CTX.level)


# ---- A ------------------------------------------------------------------------------------------------------------

def phase_a() -> None:
    st = cmd("level.status", timeout=120)
    states = {s["step"]: s["state"] for s in st.get("steps", [])}
    CTX.level = st.get("level", "")
    check(st.get("ok") and states and all(v in ("never", "not_needed") for v in states.values())
          and states.get("details") == "not_needed" and states.get("light") == "never",
          "A level.status before a build: every step never built, the details not needed", short(st))
    pr = cmd("level.problems")
    check(pr.get("ok") and pr.get("problems") == [] and pr.get("total") == 0, "A level.problems is an empty list",
          short(pr))
    rc = cmd("level.play", action="reload_configs", timeout=120)
    check(rc.get("ok") and rc.get("reloaded") is True, "A level.play action=reload_configs reads the game's configs again",
          short(rc))


# ---- B ------------------------------------------------------------------------------------------------------------

def phase_b() -> None:
    spawns = spawn_positions()
    if not check(spawns, "B the scene has spawn points to stand a graph point on", "none listed"):
        return
    _name, at = spawns[0]
    placed = cmd("scene.place", kind="spawn", section="graph_point", position=at)
    check(placed.get("ok"), "B a graph point is placed where a spawn point stood", short(placed))
    # a multiplayer scene has no player: the game's spawn needs one
    actor = cmd("scene.place", kind="spawn", section="actor", position=at)
    check(actor.get("ok"), "B the player's spawn point is placed beside it", short(actor))
    CTX.actor = (actor.get("object") or {}).get("name", "")
    t = time.monotonic()
    built = cmd("level.build", quality="draft", timeout=600)
    seconds = time.monotonic() - t
    ran = [r.get("step") for r in built.get("ran") or []]
    check(built.get("ok") and {"game", "ai_map", "game_graph", "spawn", "geometry", "light"} <= set(ran),
          "B level.build quality=draft builds every step as one job (%.1f s: %s)" % (seconds, ", ".join(ran)),
          short(built))
    states = status_states()
    draft = {"geometry", "light"}
    fresh = all(s["state"] in ("fresh", "not_needed") for k, s in states.items() if k not in draft)
    check(fresh and states and states.get("geometry", {}).get("why") == "Built at Draft quality."
          and states.get("light", {}).get("state") == "stale",
          "B level.status: every step is fresh but the geometry and the light, built at Draft quality",
          short({k: (v["state"], v.get("why")) for k, v in states.items()}))
    CTX.before_restart = {k: (v["state"], v.get("why")) for k, v in states.items()}
    again = cmd("level.build", quality="draft", timeout=600)
    check(again.get("ok") and again.get("ran") == [], "B the same draft build again builds nothing", short(again))
    d = level_dir()
    files = ("level", "level.geom", "level.cform", "level.ai", "level.graph", "level.ltx", "build.prj", "build.state")
    missing = [f for f in files if not os.path.isfile(os.path.join(d, f))]
    spawn = os.path.join(CTX.project, "gamedata", "spawns", "editor.spawn")
    check(not missing and os.path.isfile(spawn), "B the level folder holds what the game loads, and build.state",
          "missing: " + ", ".join(missing) + ("" if os.path.isfile(spawn) else " editor.spawn"))
    with open(os.path.join(d, "build.state"), encoding="utf-8", errors="replace") as f:
        text = f.read()
    check("build = 1" in text and "light = {" in text and "objects = {" in text,
          "B build.state is Lua data with every step's fingerprint and the objects", text[:200])


# ---- U ------------------------------------------------------------------------------------------------------------

def save_shot(name: str) -> None:
    import base64
    pic = cmd("shot.window", timeout=120)
    data = base64.b64decode(pic.get("png_base64") or b"")
    if SHOTS and data:
        os.makedirs(SHOTS, exist_ok=True)
        with open(os.path.join(SHOTS, name + ".png"), "wb") as f:
            f.write(data)


def audit_controls() -> dict:
    audit = cmd("ui.audit")
    return {c.get("label"): c for c in audit.get("controls", []) if c.get("label")}


def phase_u() -> None:
    cmd("ui.tab", action="activate", doc="level")
    cmd("app.window", action="resize", width=1600, height=900)
    c = audit_controls()
    check(all(x in c for x in ("Save", "Build Level", "Play", "Stop", "More")),
          "U the Level toolbar: Save, Build Level, Play, Stop, More, each labelled", short(sorted(c)))
    old = [x for x in ("Build Collision", "Build AI Map", "Build Game Graph", "Build All") if x in c]
    check(not old, "U no icon-only build button is left", ", ".join(old))
    states = status_states()
    shown = cmd("level.show", popover="build")
    check(shown.get("ok") and shown.get("popover_open"), "U Build Level's menu opens", short(shown))
    save_shot("build_menu")
    c = audit_controls()
    titles = [s.get("title") for s in states.values()]
    missing = [t for t in titles if t not in c]
    check(states and not missing and "Rebuild All" in c and "Advanced" in c,
          "U it lists every step by its title, Rebuild All and Advanced", "missing: " + ", ".join(missing))
    # ("Details" is also a panel's tab: its row is told apart by nothing in the audit)
    whys = [(s.get("title"), s.get("why")) for s in states.values() if s.get("why") and s.get("title") != "Details"]
    wrong = [t for t, w in whys if (c.get(t) or {}).get("help") != w]
    check(not wrong, "U a step's row says why it is not fresh in its tip", ", ".join(wrong))
    cmd("level.show", popover="none")
    if cmd("level.show").get("build"):
        chip = cmd("level.show", popover="rebuild")
        check(chip.get("popover_open"), "U the Build chip opens the same popover", short(chip))
        save_shot("build_chip_popover")
        c = audit_controls()
        check(any(k.startswith("Build: ") for k in c) and "Build Level" in c,
              "U the chip names the stale steps", short([k for k in c if k.startswith("Build")]))
        cmd("level.show", popover="none")
    # the popover while a build runs: drawn from the status, nothing waits for the build
    started = cmd("level.build", force=True, quality="draft", wait=False)
    during = cmd("level.show", popover="build")
    check(started.get("ok") and during.get("popover_open"), "U Build Level's menu opens while a build runs",
          short(started) + short(during))
    save_shot("build_menu_during_build")
    cmd("level.show", popover="none")
    if started.get("job"):
        cmd("job.wait", job=started["job"], timeout=600)
    # the menus: their rows are actions, off without a selection
    cmd("scene.select", none=True)
    cmd("ui.popup", show="menu:Scene")
    save_shot("menu_scene")
    c = audit_controls()
    check(c.get("Select All", {}).get("enabled") is True and c.get("Hide Unselected", {}).get("enabled") is False,
          "U Scene: Select All is on, Hide Unselected off without a selection",
          short({k: c.get(k) for k in ("Select All", "Hide Unselected")}))
    for menu in ("Build", "View", "File", "Edit"):
        cmd("ui.popup", show="menu:" + menu)
        save_shot("menu_" + menu.lower())
    cmd("ui.popup", show="none")
    keys = {r.get("id"): r for r in cmd("app.keys", query="Build Level").get("keys", [])}
    check("level.build" in keys and keys["level.build"].get("command") == "level.build",
          "U Build Level is an action over level.build", short(keys.get("level.build")))
    # Edit > Delete through its action: a light and a glow go together
    light = (cmd("scene.place", kind="light", position=[1, 2, 1]).get("object") or {}).get("name")
    glow = (cmd("scene.place", kind="glow", position=[1, 2, 3]).get("object") or {}).get("name")
    cmd("scene.select", objects=[light, glow])
    cmd("ui.tab", action="activate", doc="level")
    ran = cmd("ui.palette", action="run", id="edit.delete")
    deadline = time.monotonic() + 10
    left = {light, glow}
    while left and time.monotonic() < deadline:
        names = {o.get("name") for o in cmd("scene.objects", classes=["light", "glow"], limit=5000).get("objects") or []}
        left = {light, glow} & names
        time.sleep(0.2)
    check(ran.get("ok") and not left, "U Edit > Delete deletes a light and a glow selected together",
          short(ran) + " left: " + ", ".join(sorted(left)))
    # the problems: Validate's findings are the Problems panel's rows; going to one selects its object
    cmd("level.validate", timeout=300)
    listed = cmd("level.problems", limit=1000)
    total = listed.get("total", 0)
    cmd("ui.panel", action="open", id="problems")
    rows = (cmd("level.show").get("problems") or {}).get("rows")
    deadline = time.monotonic() + 10
    while rows != total and time.monotonic() < deadline:
        time.sleep(0.2)
        rows = (cmd("level.show").get("problems") or {}).get("rows")
    check(rows == total, "U the Problems panel lists the build's %s problems" % total, "rows %s" % rows)
    save_shot("problems")
    named = [(i, p) for i, p in enumerate(listed.get("problems") or []) if p.get("object")]
    if named:
        i, p = named[0]
        went = cmd("level.show", problem=i)
        sel = cmd("scene.info").get("selection") or {}
        names = sel.get("objects") if isinstance(sel, dict) else sel
        check(went.get("ok") and p["object"] in json.dumps(names), "U going to a problem selects its object",
              short(went) + short(sel))
    elif total:
        went = cmd("level.show", problem=0)
        check(went.get("ok"), "U going to a problem without an object answers", short(went))


# ---- R ------------------------------------------------------------------------------------------------------------

def phase_r() -> None:
    saved = cmd("scene.save", timeout=120)
    check(saved.get("ok"), "R the scene is saved (the AI map the build made is in it)", short(saved))
    CTX.editor.close()
    CTX.editor.start()
    if not open_small_scene():
        check(False, "R the scene opens again after the restart")
        return
    states = status_states()
    now = {k: (v["state"], v.get("why")) for k, v in states.items()}
    check(states and now == CTX.before_restart, "R after a restart level.status says what it said before", short(now))
    # a static object moved: the steps that read scene objects are stale, the ones that do not are fresh
    objs = cmd("scene.objects", classes=["object"], limit=1).get("objects") or []
    if not check(objs, "R the scene has a scene object to move"):
        return
    name = objs[0]["name"]
    p = objs[0].get("position") or [0, 0, 0]
    moved = cmd("scene.set", objects=[name], position=[p[0] + 0.5, p[1], p[2]])
    check(moved.get("ok"), "R an object is moved", short(moved))
    states = status_states()
    stale = sorted(k for k, v in states.items() if v["state"] == "stale")
    fresh = sorted(k for k, v in states.items() if v["state"] == "fresh")
    geometry = states.get("geometry", {})
    check({"geometry", "light", "game", "ai_map"} <= set(stale) and "1 object moved." == geometry.get("why"),
          "R the steps that read it are stale, saying what changed (%s)" % ", ".join(stale),
          short({k: (v["state"], v.get("why")) for k, v in states.items()}))
    check("game_graph" not in stale or states["game_graph"].get("why") == "AI Map is not fresh.",
          "R a step stale only through what it needs says so", short(states.get("game_graph")))
    check(not fresh or all(k not in fresh for k in ("geometry", "light")), "R nothing that reads it stays fresh")


# ---- S ------------------------------------------------------------------------------------------------------------

# the compiler's phases by their plain names (gz-lc's PHASES): what the job's stage shows while one runs
COMPILER_PHASES = ("Reading the build project", "Loading the build project into the compiler", "Optimizing the geometry",
                   "Tessellating the geometry for its vertex light", "Building the normals", "Building the collision",
                   "Building the tangent basis", "Building the ray model of the light", "Lighting the static models",
                   "Resolving the materials", "Mapping the lightmaps", "Subdividing the geometry",
                   "Lighting the implicit textures", "Lighting the lightmaps", "Lighting the vertices",
                   "Packing the lightmaps", "Merging the geometry", "Converting the geometry to visuals",
                   "Converting the static models to visuals", "Building the sectors", "Writing the level")


def phase_s() -> None:
    # the level's own settings: the compiler takes long enough to be stopped inside it
    started = cmd("level.build", wait=False)
    job = started.get("job")
    if not check(started.get("ok") and job, "S level.build wait=false answers its job", short(started)):
        return
    deadline = time.monotonic() + 300
    stage, stages = "", []
    while time.monotonic() < deadline:
        j = cmd("job.get", job=job)
        stage = j.get("stage") or ""
        if stage and (not stages or stages[-1] != stage):
            stages.append(stage)
        # inside the compiler, past its reading: a phase of the geometry or the light that is under way
        if j.get("state") != "running" or stage.startswith(COMPILER_PHASES[2:]):
            break
        time.sleep(0.02)
    t = time.monotonic()
    stopped = cmd("job.stop", job=job)
    end = cmd("job.wait", job=job, timeout=300)
    after = time.monotonic() - t
    check(stopped.get("ok") and end.get("state") == "cancelled",
          "S job.stop while the compiler runs ends the build cancelled (stopped in: %s)" % stage, short(end))
    check(stage.startswith(COMPILER_PHASES) and not any(x.rstrip().endswith("...") for x in stages),
          "S the job's stage follows the compiler's phases by their plain names (%s)" % stage, short(stages[-6:]))
    check(after < 5.0, "S the build stopped %.1f s after the Stop, in the compiler's phase" % after, timing=True)
    left = [d for d in (".lc_new", ".lc_old") if os.path.isdir(os.path.join(level_dir(), d))]
    check(not left, "S the compiler's stage folders are gone", ", ".join(left))
    states = status_states()
    check(states.get("geometry", {}).get("state") == "fresh" and states.get("game", {}).get("state") == "fresh"
          and states.get("light", {}).get("state") != "fresh",
          "S the steps that ended before the Stop are fresh, the light is not",
          short({k: (v["state"], v.get("why")) for k, v in states.items()}))


# ---- C ------------------------------------------------------------------------------------------------------------

def phase_c() -> None:
    empty = os.path.join(CTX.work, "no_build")
    os.makedirs(empty, exist_ok=True)
    r = cmd("level.compile", folder=empty)
    check(not r.get("ok") and "build.prj" in str(r.get("error")), "C level.compile on a folder without build.prj is refused",
          short(r))
    started = cmd("level.compile", folder=level_dir(), wait=False)
    job = started.get("job")
    done = cmd("job.wait", job=job, timeout=600)
    report = done.get("result") or {}
    parts = report.get("parts") or [{}]
    phases = [p.get("name") for p in parts[0].get("phases", [])]
    files = {f.get("name") for f in parts[0].get("files", [])}
    check(started.get("ok") and done.get("state") == "done" and "Lighting the lightmaps" in phases
          and not any(p.endswith("...") or p.startswith("LIGHT") for p in phases)
          and {"level", "level.geom"} <= files,
          "C the level folder compiled again, followed with job.wait (%d phases, %d files)" % (len(phases), len(files)),
          short(done)[:300])


# ---- E ------------------------------------------------------------------------------------------------------------

def phase_e() -> None:
    obj = os.path.join(CTX.work, "export", "level.obj")
    r = cmd("level.export", kind="obj", path=obj, all=True, timeout=300)
    check(r.get("ok") and os.path.isfile(obj) and os.path.isfile(obj[:-4] + ".mtl"),
          "E level.export kind=obj writes the static geometry and its MTL (%s objects)" % r.get("objects"), short(r))
    # the spawn layer holds what the module adds: the player's spawn point placed in B
    r = cmd("level.export", kind="spawn", objects=[CTX.actor], timeout=300)
    path = r.get("path") or ""
    check(CTX.actor and r.get("ok") and path.endswith(".xspawn") and os.path.isfile(path),
          "E level.export kind=spawn of one object named writes the spawn layer", short(r))
    r = cmd("level.export", kind="cut", objects=["no_such_object"])
    check(not r.get("ok") and r.get("code") == "not_found", "E an object that is not there is named back", short(r))
    # an exported model ships with the module: it says the editor built it, never this computer or this user
    r = cmd("level.export", kind="visuals", all=True, timeout=300)
    visuals = r.get("dir") or ""
    models = [os.path.join(visuals, n) for n in os.listdir(visuals)] if os.path.isdir(visuals) else []
    models = [m for m in models if m.endswith(".ogf")]
    words = [w.encode("ascii", "ignore").lower() for w in (os.environ.get("COMPUTERNAME", ""), os.environ.get("USERNAME", ""))
             if len(w) > 2]
    named = [os.path.basename(m) for m in models if any(w in open(m, "rb").read().lower() for w in words)]
    stamped = sum(1 for m in models if b"Grizzly Editor " in open(m, "rb").read())
    check(r.get("ok") and models and not named and stamped == len(models),
          "E level.export kind=visuals: %d models name the editor and neither this computer nor this user" % len(models),
          short({"answer": r, "named": named[:3], "stamped": stamped}))


# ---- D: the doors -------------------------------------------------------------------------------------------------

def phase_d() -> None:
    """What loses a scene's files or its unsaved work passes the destroy door alone."""
    # the scene's own files are on the disk first: it may have come from an import and never been saved
    own = (cmd("scene.save").get("file") or "").replace("\\", "/")
    other = os.path.splitext(own)[0] + "_qa_other.scene"
    made = cmd("scene.save", path=other)
    check(made.get("ok") and made.get("file", "").replace("\\", "/") == other, "D scene.save path= saves as a new scene", short(made))
    back = cmd("scene.save", path=own)
    check(not back.get("ok") and back.get("code") == "exists" and "overwrite" in str(back.get("error")),
          "D saving as a scene that is there is refused, naming overwrite", short(back))
    run_door = qa.gate("run", "scene.save", path=own, overwrite=True)
    check(not run_door.get("ok") and run_door.get("code") == "gate",
          "D scene.save overwrite=true does not pass the run door", short(run_door))
    destroy_door = qa.gate("destroy", "scene.save", path=own, overwrite=True)
    check(destroy_door.get("ok") and destroy_door.get("file", "").replace("\\", "/") == own,
          "D ... and replaces the scene through the destroy door", short(destroy_door))
    discard = qa.gate("run", "scene.recover", action="discard")
    check(not discard.get("ok") and discard.get("code") == "gate",
          "D scene.recover action=discard does not pass the run door", short(discard))
    listed = qa.gate("run", "scene.recover", action="get")
    check(listed.get("ok"), "D scene.recover action=get does", short(listed))
    info = cmd("scene.info", summary=True, path="import/qa_summary.txt")
    check(not info.get("ok") and info.get("code") == "invalid" and "path" in str(info.get("error")),
          "D scene.info reads: it takes no file to write", short(info))
    text = os.path.join(CTX.project, "import", "qa_summary.txt")
    wrote = cmd("scene.summary", path="import/qa_summary.txt", timeout=120)
    check(wrote.get("ok") and os.path.isfile(text) and os.path.getsize(text) > 0,
          "D scene.summary writes the summary as a text file", short(wrote))
    again = cmd("scene.summary", path="import/qa_summary.txt", timeout=120)
    check(not again.get("ok") and again.get("code") == "exists", "D ... and refuses a file that is there", short(again))


# ---- Y ------------------------------------------------------------------------------------------------------------

def tree_of(top: str) -> dict:
    out = {}
    for d, dirs, files in os.walk(top):
        for n in dirs + files:
            p = os.path.join(d, n)
            st = os.lstat(p)
            out[os.path.relpath(p, top)] = (0 if n in dirs else st.st_size, st.st_mtime_ns)
    return out


def phase_y() -> None:
    victim = os.path.join(CTX.work, "victim")
    os.makedirs(os.path.join(victim, "gamedata", "levels", "my_level"), exist_ok=True)
    for rel, text in (("mod.ltx", "[module]\nid = victim\nname = Victim\n"), ("fsgame.ltx", "; the folder's own\n"),
                      ("gamedata\\precious.txt", "the author's work\n")):
        with open(os.path.join(victim, rel), "w") as f:
            f.write(text)
    busy = os.path.join(CTX.work, "busy")
    os.makedirs(busy, exist_ok=True)
    with open(os.path.join(busy, "notes.txt"), "w") as f:
        f.write("mine\n")
    before = {k: tree_of(k) for k in (victim, busy)}
    cases = ((victim, "it is a project folder"), (os.path.join(victim, "gamedata"), "it lies inside the project"),
             (busy, "is not a Play root: choose an empty folder"), (qa.REPO, "the editor"),
             ("relative\\root", "by a full path"))
    for root, said in cases:
        for args in ({"action": "root"}, {"action": "start", "clean": True}, {"action": "start", "build": False}):
            a = cmd("level.play", root=root, timeout=60, **args)
            check(not a.get("ok") and said in str(a.get("error")),
                  "Y level.play %s root=<%s> is refused (%s)" % (json.dumps(args), os.path.basename(root), said), short(a))
    after = {k: tree_of(k) for k in (victim, busy)}
    check(before == after, "Y the folders are as they were")
    st = cmd("level.play", action="status")
    check(st.get("ok") and not st.get("running") and not st.get("building"), "Y no game was started", short(st))


# ---- K ------------------------------------------------------------------------------------------------------------

def phase_k() -> None:
    saved = cmd("scene.save", timeout=120)
    check(saved.get("ok"), "K the scene is saved before the editor closes", short(saved))
    CTX.editor.close()
    exe = os.path.join(os.path.dirname(qa.exe()), "Grizzly-Cmd.exe")
    if not check(os.path.isfile(exe), "K the commandlet is built", exe):
        return
    t = time.monotonic()
    p = subprocess.run([exe, "run", "level.status", "-project", CTX.project, "-json", "-quiet", "-home", qa.home()],
                       capture_output=True, text=True, timeout=300)
    seconds = time.monotonic() - t
    line = (p.stdout.strip().splitlines() or ["{}"])[-1]
    try:
        r = json.loads(line)
    except ValueError:
        r = {}
    stale = r.get("stale") or []
    check(p.returncode == 0 and r.get("ok") and "light" in stale and "geometry" not in stale,
          "K Grizzly-Cmd.exe run level.status from a cold start answers the light stale (%.1f s)" % seconds,
          short(r) + " " + p.stderr[-300:])
    CTX.editor.start()


# ---- L ------------------------------------------------------------------------------------------------------------

def phase_l() -> None:
    if not check(open_small_scene(), "L the scene opens"):
        return
    spawns = spawn_positions()
    if not check(spawns, "L the scene has a spawn point to paint round"):
        return
    x, y, z = spawns[0][1]
    info = cmd("scene.ai_map")
    check(info.get("ok") and "ground" in info, "L scene.ai_map answers its ground and parameters", short(info))
    cmd("scene.ai_map", action="clear")
    cleared = cmd("scene.ai_map")
    gen = cmd("scene.ai_map", action="generate", timeout=300)
    made = cmd("scene.ai_map")
    check(cleared.get("nodes") == 0 and gen.get("ok") and (made.get("nodes") or 0) > 0,
          "L scene.ai_map action=generate makes the map over the ground from the spawn points (%s nodes)"
          % made.get("nodes"), short(gen))
    cmd("doc.undo", doc="level")
    undone = cmd("scene.ai_map")
    check(undone.get("nodes") == 0, "L the generation is one undo step", short(undone))
    meshes = ["detail\\trava_green", "detail\\trava_black"]
    added = cmd("scene.details", action="add", objects=meshes)
    names = [m.get("object") for m in cmd("scene.details", action="list").get("meshes") or []]
    check(added.get("ok") and set(meshes) <= set(names), "L scene.details action=add: the meshes are the layer's",
          short(added))
    painted = cmd("scene.details", action="paint", object=meshes[0], position=[x, y, z], radius=6.0, timeout=120)
    filled = cmd("scene.details").get("filled", 0)
    check(painted.get("ok") and filled > 0, "L a stroke paints the slots under the brush (%d)" % filled,
          short(painted))
    cmd("scene.details", action="erase", position=[x, y, z], radius=6.0, timeout=120)
    check(cmd("scene.details").get("filled", 1) == 0, "L the eraser empties them again")
    fill = cmd("scene.details", action="fill", object=meshes[1], density=0.5, timeout=120)
    filled = cmd("scene.details").get("filled", 0)
    states = status_states()
    check(fill.get("ok") and filled > 0 and states.get("details", {}).get("state") == "never",
          "L a fill lays a mesh over the whole ground (%d slots) and the details step is to be built" % filled,
          short(states.get("details")))
    check(cmd("scene.details", action="remove", objects=[meshes[0]]).get("ok"), "L a mesh is taken out")
    cmd("scene.wallmarks", action="set", texture="wm\\wm_anarh1", width=2.0, height=2.0)
    laid = cmd("scene.wallmarks", action="add", start=[x, y + 5.0, z], dir=[0, -1, 0])
    marks = cmd("scene.wallmarks").get("marks") or 0
    check(laid.get("ok") and laid.get("added") and marks >= 1, "L scene.wallmarks lays a mark on the ground",
          short(laid))
    zl, zv = cmd("scene.zones"), cmd("scene.zones", action="validate")
    check(zl.get("ok") and zv.get("ok"), "L scene.zones lists and validates", short(zl) + short(zv))
    built = cmd("level.build", quality="draft", only="detail_light", timeout=300)
    ran = [r.get("step") for r in built.get("ran") or []]
    check(built.get("ok") and {"details", "detail_light"} <= set(ran),
          "L level.build only=detail_light builds the details and their light (%s)" % ", ".join(ran), short(built))


# ---- F ------------------------------------------------------------------------------------------------------------

def phase_f() -> None:
    """An error ends the build; what a later step would refuse is refused before the first step runs."""
    if not check(open_small_scene(), "F the scene opens"):
        return
    prj = os.path.join(level_dir(), "build.prj")
    if not check(os.path.isfile(prj), "F the level folder holds build.prj"):
        return
    keep = prj + ".qa"
    shutil.copy2(prj, keep)
    try:
        # the light reads a build.prj cut short: it fails, and the detail light it holds up never runs
        with open(prj, "r+b") as f:
            f.truncate(max(16, os.path.getsize(prj) // 3))
        t = time.monotonic()
        r = cmd("level.build", only="detail_light", force=True, nosun=True, quality="draft", timeout=600)
        secs = time.monotonic() - t
        err = str(r.get("error"))
        check(not r.get("ok") and "Light failed:" in err and "Not run: Detail Light" in err,
              "F a step that fails ends the build, naming it and the steps not run (%.1f s)" % secs, short(r))
        states = status_states()
        check(states.get("light", {}).get("state") == "failed", "F the light is failed, the build went no further",
              short(states.get("light")))
    finally:
        shutil.copy2(keep, prj)
        os.remove(keep)
    # the actor gone: the spawn is refused before a single step runs, the problem on the step
    actors = [o["name"] for o in cmd("scene.objects", classes=["spawn"], limit=100000).get("objects") or []
              if o.get("ref") == "actor"]
    if not check(actors, "F the scene has the player's spawn point"):
        return
    gone = cmd("scene.delete", objects=actors)
    check(gone.get("ok"), "F the player's spawn point is deleted", short(gone))
    t = time.monotonic()
    r = cmd("level.build", only="spawn", force=True, timeout=600)
    secs = time.monotonic() - t
    err = str(r.get("error"))
    check(not r.get("ok") and "did not start" in err and "no actor spawn point" in err and "Not run:" in err,
          "F with no actor the build refuses before any step (%.1f s)" % secs, short(r))
    check(secs < 30, "F ... in seconds, not after the long steps (%.1f s)" % secs, timing=True)
    probs = cmd("level.problems", step="spawn").get("problems") or []
    check(any(p.get("code") == "preflight" and "actor" in p.get("text", "") and p.get("severity") == "error"
              for p in probs), "F the refusal is a problem of the spawn step", short(probs))
    undo = cmd("doc.undo", doc="level")
    back = []
    deadline = time.monotonic() + 10
    while not back and time.monotonic() < deadline:
        back = [o["name"] for o in cmd("scene.objects", classes=["spawn"], limit=100000).get("objects") or []
                if o.get("ref") == "actor"]
        if not back:
            time.sleep(0.2)
    check(back, "F the actor comes back with Undo", short(undo) + " " + short(back))


# ---- G ------------------------------------------------------------------------------------------------------------

STOCK = "l01_escape"


def phase_g() -> None:
    """A level the linked game has: the module adds to it, nothing is compiled, Play plays the module."""
    levels = {l.get("name") for l in cmd("world.lookup", type="levels", timeout=120).get("levels") or []}
    if not check(STOCK in levels, "G the linked game has %s" % STOCK, "levels: %d" % len(levels)):
        return
    if not check(open_small_scene(), "G the scene opens"):
        return
    rel = "rawdata/levels/%s.scene" % STOCK
    saved = cmd("scene.save", path=rel, timeout=120)
    path = cmd("scene.set", world=True, set={"level.path": STOCK})
    check(saved.get("ok") and path.get("ok"), "G the scene is saved as %s with its level path" % STOCK,
          short(saved) + short(path))
    st = cmd("level.status", timeout=120)
    states = {s["step"]: s for s in st.get("steps", [])}
    compile_steps = [k for k in states if k != "spawn_layer"]
    check(st.get("ok") and st.get("stock") is True and compile_steps
          and all(states[k]["state"] == "not_needed" for k in compile_steps)
          and set(st.get("stale") or []) <= {"spawn_layer"},
          "G level.status: a level of the linked game, every compile step not needed",
          short({k: v["state"] for k, v in states.items()}))
    why = states.get("light", {}).get("why", "")
    check("is a level of the linked game" in why, "G ... saying why", why)
    chip = str(cmd("level.show").get("build") or "")
    check(not any(t in chip for t in ("Game Files", "AI Map", "Geometry", "Light", "Spawn ·")),
          "G the Build chip asks for no compile step (%s)" % (chip or "nothing"), chip)
    built = cmd("level.build", timeout=600)
    ran = [r.get("step") for r in built.get("ran") or []]
    check(built.get("ok") and set(ran) <= {"spawn_layer"} and built.get("stock") is True,
          "G Build Level builds what the module needs and no compile (%s)" % (", ".join(ran) or "nothing"), short(built))
    # a compile step asked for by name runs: its files would replace the game's own
    game = cmd("level.build", only="game", force=True, timeout=600)
    check(game.get("ok") and [r.get("step") for r in game.get("ran") or []] == ["game"],
          "G a compile step asked for by name still runs", short(game))
    st = cmd("level.status", timeout=120)
    replaces = st.get("replaces") or []
    check("level.ltx" in replaces and all(not r.startswith("build.") for r in replaces),
          "G level.status names the game's files the module would replace (%d)" % len(replaces), short(replaces))
    probs = cmd("level.problems").get("problems") or []
    check(any(p.get("code") == "replaces_stock" and p.get("severity") == "warning" for p in probs),
          "G ... and the problems hold the warning", short([p for p in probs if p.get("code") == "replaces_stock"]))
    refused = cmd("level.play", action="start", dry_run=True, timeout=120)
    if "fsltx-in-place" not in str(refused.get("error")):
        check(not refused.get("ok") and refused.get("code") == "refused" and "level.clean" in str(refused.get("error")),
              "G Play refuses while the module would replace the game's files, naming the fix", short(refused))
    run_door = qa.gate("run", "level.clean")
    check(not run_door.get("ok") and run_door.get("code") == "gate", "G level.clean does not pass the run door",
          short(run_door))
    dry = qa.gate("destroy", "level.clean", dry_run=True)
    d = os.path.join(CTX.project, "gamedata", "levels", STOCK)
    check(dry.get("ok") and "level.ltx" in (dry.get("files") or []) and os.path.isfile(os.path.join(d, "level.ltx")),
          "G level.clean dry_run=true names the files and removes none", short(dry))
    gone = qa.gate("destroy", "level.clean")
    left = [f for f in (gone.get("files") or []) if os.path.exists(os.path.join(d, f))]
    check(gone.get("ok") and gone.get("removed") and not left, "G level.clean through the destroy door removes them",
          short(gone))
    st = cmd("level.status", timeout=120)
    check(not st.get("replaces"), "G nothing of the game's level is replaced any more", short(st.get("replaces")))
    # Play plays the module: nothing compiled, nothing of the level staged, a new game
    t = time.monotonic()
    play = cmd("level.play", action="start", dry_run=True, timeout=600)
    secs = time.monotonic() - t
    if "fsltx-in-place" in str(play.get("error")) or "no game" in str(play.get("error")).lower():
        print("SKIP G Play: the linked game does not run in place here: " + short(play), flush=True)
        return
    check(play.get("ok") and play.get("way") == "module" and "no compile needed" in str(play.get("note")),
          "G Play on %s plays the module (%.1f s)" % (STOCK, secs), short(play))
    root = (play.get("root") or "").replace("/", "\\")
    module = (play.get("module_dir") or "").replace("/", "\\")
    check(os.path.isfile(os.path.join(module, "mod.ltx")), "G the module is in the root's modules\\<id>", module)
    shipped = [os.path.relpath(os.path.join(dp, n), module) for dp, _dn, fs in os.walk(module) for n in fs]
    check(not [f for f in shipped if os.path.basename(f).startswith("build.")],
          "G the module ships none of the level build's own files", short(shipped[:20]))
    layer = os.path.join(module, "spawn", STOCK + ".xspawn")
    text = open(layer, encoding="utf-8", errors="replace").read() if os.path.isfile(layer) else ""
    start = play.get("start") or {}
    check("op        = start" in text and "level     = %s" % STOCK in text and start.get("placed") is True,
          "G Play's Player Start where the camera looks is in its copy of the module (%s)" % start.get("source"),
          text[:300])
    check(not os.path.exists(os.path.join(CTX.project, "spawn", STOCK + ".xspawn"))
          or "grizzly_play_start" not in open(os.path.join(CTX.project, "spawn", STOCK + ".xspawn"),
                                              encoding="utf-8", errors="replace").read(),
          "G ... and never in the project's own layer")
    marker = open(os.path.join(root, "play_root.txt"), encoding="utf-8", errors="replace").read() if root else ""
    check("module = " in marker, "G the root's marker names the module it holds", marker[-200:])
    options = os.path.join(root, "gamedata", "configs", "axr_options.ltx")
    check(os.path.isfile(options) and os.stat(options).st_nlink == 1,
          "G the new game's options are a file of the root's own (the game's is never written)")
    check(not os.path.isdir(os.path.join(root, "gamedata", "levels", STOCK))
          and not os.path.isfile(os.path.join(root, "gamedata", "spawns", "editor.spawn")),
          "G no level and no editor.spawn are staged")
    line = str(play.get("command"))
    check("-start server(all/single/alife/new) client(localhost)" in line and "-fsltx" in line
          and play.get("started") is None, "G the command line starts a new game on the root's modules", line)
    st = cmd("level.play", action="status")
    check(st.get("ok") and not st.get("running"), "G nothing was started (dry_run)", short(st))


STEPS = {"A": phase_a, "B": phase_b, "U": phase_u, "R": phase_r, "S": phase_s, "C": phase_c, "E": phase_e, "Y": phase_y,
         "K": phase_k, "L": phase_l, "D": phase_d, "F": phase_f, "G": phase_g}


def run(ctx: qa.Run) -> None:
    global SHOTS
    SHOTS = ctx.shots
    project = ctx.project
    CTX.project, CTX.work = project, ctx.work
    levels = os.path.join(project, "rawdata", "levels")
    os.makedirs(levels, exist_ok=True)
    shutil.copy2(os.path.join(qa.REPO, "rawdata", "levels", SCENE + ".level"), levels)
    shutil.copytree(os.path.join(qa.REPO, "rawdata", "levels", SCENE), os.path.join(levels, SCENE),
                    dirs_exist_ok=True)
    CTX.editor = qa.Editor(project)
    try:
        CTX.editor.start()
        if not check(open_small_scene(), "the small scene opens"):
            return
        st = cmd("level.status", timeout=120)
        CTX.level = st.get("level", "") or SCENE
        qa.mark("checks")
        qa.run_phases(STEPS, ctx.only)
    finally:
        CTX.editor.close()


def main() -> int:
    return qa.suite("LEVEL COMMANDS", run, doc=__doc__, trimmed=True, phases=PHASES)


if __name__ == "__main__":
    sys.exit(main())
