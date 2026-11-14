#!/usr/bin/env python3
"""Building a level, over MCP on a THROW-AWAY COPY of a project with the repository's smallest scene
(rawdata\\levels\\mp_test.level):

  A  level.status before any build: every step never built (the details, which the scene has none of, not needed);
     level.problems is an empty list; the old names of the builds are gone; level.play action=reload_configs reads
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
  S  Stop between steps keeps the finished ones fresh: after the move, level.build wait=false and job.stop while the
     compiler runs - the job ends cancelled, the geometry is fresh, the light is not
  C  level.compile on a folder: refused for a folder that holds no build.prj, naming it; the level folder compiled
     again, waited for with job.wait, its report naming the compiler's phases and files
  E  level.export: the static geometry as OBJ, the spawn layer of one object named
  Y  Play takes no folder it did not make: level.play start / root with root= a folder that is a project, inside one,
     holds a file of somebody else's, is the editor's or relative - every one refused before anything is built or
     written, the folders unchanged, no game started
  K  the commandlet from a cold start: Grizzly-Cmd.exe run level.status -project <copy> -json answers what the editor
     said last (the light stale), with no editor running
  L  the painted layers and the zones on the ground (every static object, no list): scene.ai_map clears and
     generates the map from the spawn points, one undo step; scene.details adds two meshes, paints, erases, fills
     and removes one, the status following; scene.wallmarks lays a mark under a point; scene.zones lists and
     validates; level.build only=detail_light builds the details and their light
  Z  perf.stalls is empty at the end

    python tools/qa/test_level_commands.py --project <project> [--only A,B,...]
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

MCP = qa_target.MCP
SCENE = "mp_test"
PHASES = "ABURSCEYKLZ"
SHOTS: str | None = None
results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def short(a) -> str:
    return json.dumps(a)[:500]


def mcp(cmd: str, timeout: float = 120.0, **extra) -> dict:
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


def listing(folder: str) -> list[tuple[str, int]]:
    out = []
    for d, _dirs, files in os.walk(folder):
        for n in files:
            p = os.path.join(d, n)
            out.append((os.path.relpath(p, folder), os.path.getsize(p)))
    return sorted(out)


def status_states() -> dict:
    st = mcp("level.status", timeout=120)
    return {s["step"]: s for s in st.get("steps", [])} if st.get("ok") else {}


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
        qa_target.guard_project(project)
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project],
                                     cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                if mcp("ping", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        # never killed: a killed editor leaves the project's files half written
        try:
            mcp("app.quit", discard=True)
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)
            self.proc = None


class Ctx:
    project = ""
    work = ""
    level = ""
    editor: Editor | None = None
    before_restart: dict = {}
    actor = ""


CTX = Ctx()


def open_scene() -> bool:
    # the scene as saved once it has been, else the SDK scene it is imported from
    saved = os.path.isfile(os.path.join(CTX.project, "rawdata", "levels", SCENE + ".scene"))
    rel = "rawdata/levels/%s.%s" % (SCENE, "scene" if saved else "level")
    r = mcp("scene.open", path=rel, discard=True, timeout=120)
    if not r.get("ok"):
        print("  scene.open: " + short(r), flush=True)
        return False
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        jobs = mcp("job.list", running=True).get("jobs") or []
        if not any(j.get("kind") == "scene_load" for j in jobs):
            return True
        time.sleep(0.2)
    return False


def spawn_positions() -> list[tuple[str, list[float]]]:
    """The spawn points of the scene, by name, with their positions."""
    r = mcp("scene.objects", classes=["spawn"], limit=1000)
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
    st = mcp("level.status", timeout=120)
    states = {s["step"]: s["state"] for s in st.get("steps", [])}
    CTX.level = st.get("level", "")
    check(st.get("ok") and states and all(v in ("never", "not_needed") for v in states.values())
          and states.get("details") == "not_needed" and states.get("light") == "never",
          "A level.status before a build: every step never built, the details not needed", short(st))
    pr = mcp("level.problems")
    check(pr.get("ok") and pr.get("problems") == [] and pr.get("total") == 0, "A level.problems is an empty list",
          short(pr))
    gone = [n for n in ("build_cform", "build_aimap", "build_all", "make_all", "compile_geometry", "level_build",
                        "level_rebuild_status", "play_clean", "aimap_tool", "sector_tool", "mod_export_spawn",
                        "scene_validate", "compiler_errors") if mcp(n).get("code") != "not_found"]
    check(not gone, "A the old names of the builds and tools are gone", ", ".join(gone))
    rc = mcp("level.play", action="reload_configs", timeout=120)
    check(rc.get("ok") and rc.get("reloaded") is True, "A level.play action=reload_configs reads the game's configs again",
          short(rc))


# ---- B ------------------------------------------------------------------------------------------------------------

def phase_b() -> None:
    spawns = spawn_positions()
    if not check(spawns, "B the scene has spawn points to stand a graph point on", "none listed"):
        return
    _name, at = spawns[0]
    placed = mcp("scene.place", kind="spawn", section="graph_point", position=at)
    check(placed.get("ok"), "B a graph point is placed where a spawn point stood", short(placed))
    # a multiplayer scene has no player: the game's spawn needs one
    actor = mcp("scene.place", kind="spawn", section="actor", position=at)
    check(actor.get("ok"), "B the player's spawn point is placed beside it", short(actor))
    CTX.actor = (actor.get("object") or {}).get("name", "")
    t = time.monotonic()
    built = mcp("level.build", quality="draft", timeout=600)
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
    again = mcp("level.build", quality="draft", timeout=600)
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
    pic = mcp("shot.window", timeout=120)
    data = base64.b64decode(pic.get("png_base64") or b"")
    if SHOTS and data:
        os.makedirs(SHOTS, exist_ok=True)
        with open(os.path.join(SHOTS, name + ".png"), "wb") as f:
            f.write(data)


def audit_controls() -> dict:
    audit = mcp("ui.audit")
    return {c.get("label"): c for c in audit.get("controls", []) if c.get("label")}


def phase_u() -> None:
    mcp("ui.tab", action="activate", doc="level")
    mcp("app.window", action="resize", width=1600, height=900)
    c = audit_controls()
    check(all(x in c for x in ("Save", "Build Level", "Play", "Stop", "More")),
          "U the Level toolbar: Save, Build Level, Play, Stop, More, each labelled", short(sorted(c)))
    old = [x for x in ("Build Collision", "Build AI Map", "Build Game Graph", "Build All") if x in c]
    check(not old, "U no icon-only build button is left", ", ".join(old))
    states = status_states()
    shown = mcp("level.show", popover="build")
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
    mcp("level.show", popover="none")
    if mcp("level.show").get("build"):
        chip = mcp("level.show", popover="rebuild")
        check(chip.get("popover_open"), "U the Build chip opens the same popover", short(chip))
        save_shot("build_chip_popover")
        c = audit_controls()
        check(any(k.startswith("Build: ") for k in c) and "Build Level" in c,
              "U the chip names the stale steps", short([k for k in c if k.startswith("Build")]))
        mcp("level.show", popover="none")
    # the popover while a build runs: drawn from the status, nothing waits for the build
    started = mcp("level.build", force=True, quality="draft", wait=False)
    during = mcp("level.show", popover="build")
    check(started.get("ok") and during.get("popover_open"), "U Build Level's menu opens while a build runs",
          short(started) + short(during))
    save_shot("build_menu_during_build")
    mcp("level.show", popover="none")
    if started.get("job"):
        mcp("job.wait", job=started["job"], timeout=600)
    # the menus: their rows are actions, off without a selection
    mcp("scene.select", none=True)
    mcp("ui.popup", show="menu:Scene")
    save_shot("menu_scene")
    c = audit_controls()
    check(c.get("Select All", {}).get("enabled") is True and c.get("Hide Unselected", {}).get("enabled") is False,
          "U Scene: Select All is on, Hide Unselected off without a selection",
          short({k: c.get(k) for k in ("Select All", "Hide Unselected")}))
    for menu in ("Build", "View", "File", "Edit"):
        mcp("ui.popup", show="menu:" + menu)
        save_shot("menu_" + menu.lower())
    mcp("ui.popup", show="none")
    keys = {r.get("id"): r for r in mcp("app.keys", query="Build Level").get("keys", [])}
    check("level.build" in keys and keys["level.build"].get("command") == "level.build",
          "U Build Level is an action over level.build", short(keys.get("level.build")))
    # Edit > Delete through its action: a light and a glow go together
    light = (mcp("scene.place", kind="light", position=[1, 2, 1]).get("object") or {}).get("name")
    glow = (mcp("scene.place", kind="glow", position=[1, 2, 3]).get("object") or {}).get("name")
    mcp("scene.select", objects=[light, glow])
    mcp("ui.tab", action="activate", doc="level")
    ran = mcp("ui.palette", action="run", id="edit.delete")
    deadline = time.monotonic() + 10
    left = {light, glow}
    while left and time.monotonic() < deadline:
        names = {o.get("name") for o in mcp("scene.objects", classes=["light", "glow"], limit=5000).get("objects") or []}
        left = {light, glow} & names
        time.sleep(0.2)
    check(ran.get("ok") and not left, "U Edit > Delete deletes a light and a glow selected together",
          short(ran) + " left: " + ", ".join(sorted(left)))
    # the problems: Validate's findings are the Problems panel's rows; going to one selects its object
    mcp("level.validate", timeout=300)
    listed = mcp("level.problems", limit=1000)
    total = listed.get("total", 0)
    mcp("ui.panel", action="open", id="problems")
    rows = (mcp("level.show").get("problems") or {}).get("rows")
    deadline = time.monotonic() + 10
    while rows != total and time.monotonic() < deadline:
        time.sleep(0.2)
        rows = (mcp("level.show").get("problems") or {}).get("rows")
    check(rows == total, "U the Problems panel lists the build's %s problems" % total, "rows %s" % rows)
    save_shot("problems")
    named = [(i, p) for i, p in enumerate(listed.get("problems") or []) if p.get("object")]
    if named:
        i, p = named[0]
        went = mcp("level.show", problem=i)
        sel = mcp("scene.info").get("selection") or {}
        names = sel.get("objects") if isinstance(sel, dict) else sel
        check(went.get("ok") and p["object"] in json.dumps(names), "U going to a problem selects its object",
              short(went) + short(sel))
    elif total:
        went = mcp("level.show", problem=0)
        check(went.get("ok"), "U going to a problem without an object answers", short(went))


# ---- R ------------------------------------------------------------------------------------------------------------

def phase_r() -> None:
    saved = mcp("scene.save", timeout=120)
    check(saved.get("ok"), "R the scene is saved (the AI map the build made is in it)", short(saved))
    CTX.editor.close()
    CTX.editor.start(CTX.project)
    if not open_scene():
        check(False, "R the scene opens again after the restart")
        return
    states = status_states()
    now = {k: (v["state"], v.get("why")) for k, v in states.items()}
    check(states and now == CTX.before_restart, "R after a restart level.status says what it said before", short(now))
    # a static object moved: the steps that read scene objects are stale, the ones that do not are fresh
    objs = mcp("scene.objects", classes=["object"], limit=1).get("objects") or []
    if not check(objs, "R the scene has a scene object to move"):
        return
    name = objs[0]["name"]
    p = objs[0].get("position") or [0, 0, 0]
    moved = mcp("scene.set", objects=[name], position=[p[0] + 0.5, p[1], p[2]])
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

def phase_s() -> None:
    # the level's own settings: the compiler takes long enough to be stopped inside it
    started = mcp("level.build", wait=False)
    job = started.get("job")
    if not check(started.get("ok") and job, "S level.build wait=false answers its job", short(started)):
        return
    deadline = time.monotonic() + 300
    stage = ""
    while time.monotonic() < deadline:
        j = mcp("job.get", job=job)
        stage = j.get("stage") or ""
        if j.get("state") != "running" or "geometry and light" in stage or "Compiling" in stage:
            break
        time.sleep(0.05)
    stopped = mcp("job.stop", job=job)
    end = mcp("job.wait", job=job, timeout=300)
    check(stopped.get("ok") and end.get("state") == "cancelled",
          "S job.stop while the compiler runs ends the build cancelled (stopped in: %s)" % stage, short(end))
    states = status_states()
    check(states.get("geometry", {}).get("state") == "fresh" and states.get("game", {}).get("state") == "fresh"
          and states.get("light", {}).get("state") != "fresh",
          "S the steps that ended before the Stop are fresh, the light is not",
          short({k: (v["state"], v.get("why")) for k, v in states.items()}))


# ---- C ------------------------------------------------------------------------------------------------------------

def phase_c() -> None:
    empty = os.path.join(CTX.work, "no_build")
    os.makedirs(empty, exist_ok=True)
    r = mcp("level.compile", folder=empty)
    check(not r.get("ok") and "build.prj" in str(r.get("error")), "C level.compile on a folder without build.prj is refused",
          short(r))
    started = mcp("level.compile", folder=level_dir(), wait=False)
    job = started.get("job")
    done = mcp("job.wait", job=job, timeout=600)
    report = done.get("result") or {}
    parts = report.get("parts") or [{}]
    phases = [p.get("name") for p in parts[0].get("phases", [])]
    files = {f.get("name") for f in parts[0].get("files", [])}
    check(started.get("ok") and done.get("state") == "done" and "LIGHT: LMaps..." in phases
          and {"level", "level.geom"} <= files,
          "C the level folder compiled again, followed with job.wait (%d phases, %d files)" % (len(phases), len(files)),
          short(done)[:300])


# ---- E ------------------------------------------------------------------------------------------------------------

def phase_e() -> None:
    obj = os.path.join(CTX.work, "export", "level.obj")
    r = mcp("level.export", kind="obj", path=obj, all=True, timeout=300)
    check(r.get("ok") and os.path.isfile(obj) and os.path.isfile(obj[:-4] + ".mtl"),
          "E level.export kind=obj writes the static geometry and its MTL (%s objects)" % r.get("objects"), short(r))
    # the spawn layer holds what the module adds: the player's spawn point placed in B
    r = mcp("level.export", kind="spawn", objects=[CTX.actor], timeout=300)
    path = r.get("path") or ""
    check(CTX.actor and r.get("ok") and path.endswith(".xspawn") and os.path.isfile(path),
          "E level.export kind=spawn of one object named writes the spawn layer", short(r))
    r = mcp("level.export", kind="cut", objects=["no_such_object"])
    check(not r.get("ok") and r.get("code") == "not_found", "E an object that is not there is named back", short(r))


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
             (busy, "is not a Play root: choose an empty folder"), (qa_target.REPO, "the editor"),
             ("relative\\root", "by a full path"))
    for root, said in cases:
        for args in ({"action": "root"}, {"action": "start", "clean": True}, {"action": "start", "build": False}):
            a = mcp("level.play", root=root, timeout=60, **args)
            check(not a.get("ok") and said in str(a.get("error")),
                  "Y level.play %s root=<%s> is refused (%s)" % (json.dumps(args), os.path.basename(root), said), short(a))
    after = {k: tree_of(k) for k in (victim, busy)}
    check(before == after, "Y the folders are as they were")
    st = mcp("level.play", action="status")
    check(st.get("ok") and not st.get("running") and not st.get("building"), "Y no game was started", short(st))


# ---- K ------------------------------------------------------------------------------------------------------------

def phase_k() -> None:
    saved = mcp("scene.save", timeout=120)
    check(saved.get("ok"), "K the scene is saved before the editor closes", short(saved))
    CTX.editor.close()
    exe = os.path.join(os.path.dirname(qa_target.exe()), "Grizzly-Cmd.exe")
    if not check(os.path.isfile(exe), "K the commandlet is built", exe):
        return
    t = time.monotonic()
    p = subprocess.run([exe, "run", "level.status", "-project", CTX.project, "-json", "-quiet", "-home", qa_target.HOME],
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
    CTX.editor.start(CTX.project)


# ---- L ------------------------------------------------------------------------------------------------------------

def phase_l() -> None:
    if not check(open_scene(), "L the scene opens"):
        return
    spawns = spawn_positions()
    if not check(spawns, "L the scene has a spawn point to paint round"):
        return
    x, y, z = spawns[0][1]
    info = mcp("scene.ai_map")
    check(info.get("ok") and "ground" in info, "L scene.ai_map answers its ground and parameters", short(info))
    mcp("scene.ai_map", action="clear")
    cleared = mcp("scene.ai_map")
    gen = mcp("scene.ai_map", action="generate", timeout=300)
    made = mcp("scene.ai_map")
    check(cleared.get("nodes") == 0 and gen.get("ok") and (made.get("nodes") or 0) > 0,
          "L scene.ai_map action=generate makes the map over the ground from the spawn points (%s nodes)"
          % made.get("nodes"), short(gen))
    mcp("doc.undo", doc="level")
    undone = mcp("scene.ai_map")
    check(undone.get("nodes") == 0, "L the generation is one undo step", short(undone))
    meshes = ["detail\\trava_green", "detail\\trava_black"]
    added = mcp("scene.details", action="add", objects=meshes)
    names = [m.get("object") for m in mcp("scene.details", action="list").get("meshes") or []]
    check(added.get("ok") and set(meshes) <= set(names), "L scene.details action=add: the meshes are the layer's",
          short(added))
    painted = mcp("scene.details", action="paint", object=meshes[0], position=[x, y, z], radius=6.0, timeout=120)
    filled = mcp("scene.details").get("filled", 0)
    check(painted.get("ok") and filled > 0, "L a stroke paints the slots under the brush (%d)" % filled,
          short(painted))
    mcp("scene.details", action="erase", position=[x, y, z], radius=6.0, timeout=120)
    check(mcp("scene.details").get("filled", 1) == 0, "L the eraser empties them again")
    fill = mcp("scene.details", action="fill", object=meshes[1], density=0.5, timeout=120)
    filled = mcp("scene.details").get("filled", 0)
    states = status_states()
    check(fill.get("ok") and filled > 0 and states.get("details", {}).get("state") == "never",
          "L a fill lays a mesh over the whole ground (%d slots) and the details step is to be built" % filled,
          short(states.get("details")))
    check(mcp("scene.details", action="remove", objects=[meshes[0]]).get("ok"), "L a mesh is taken out")
    mcp("scene.wallmarks", action="set", texture="wm\\wm_anarh1", width=2.0, height=2.0)
    laid = mcp("scene.wallmarks", action="add", start=[x, y + 5.0, z], dir=[0, -1, 0])
    marks = mcp("scene.wallmarks").get("marks") or 0
    check(laid.get("ok") and laid.get("added") and marks >= 1, "L scene.wallmarks lays a mark on the ground",
          short(laid))
    zl, zv = mcp("scene.zones"), mcp("scene.zones", action="validate")
    check(zl.get("ok") and zv.get("ok"), "L scene.zones lists and validates", short(zl) + short(zv))
    built = mcp("level.build", quality="draft", only="detail_light", timeout=300)
    ran = [r.get("step") for r in built.get("ran") or []]
    check(built.get("ok") and {"details", "detail_light"} <= set(ran),
          "L level.build only=detail_light builds the details and their light (%s)" % ", ".join(ran), short(built))


# ---- Z ------------------------------------------------------------------------------------------------------------

def phase_z() -> None:
    st = mcp("perf.stalls")
    stalls = st.get("stalls") or []
    check(st.get("ok") and not stalls, "Z no UI pass stalled (worst %s ms)" % st.get("worst_ms"), short(stalls[:5]))


STEPS = {"A": phase_a, "B": phase_b, "U": phase_u, "R": phase_r, "S": phase_s, "C": phase_c, "E": phase_e, "Y": phase_y,
         "K": phase_k, "L": phase_l, "Z": phase_z}


def run(project: str, work: str, only: list[str]) -> None:
    CTX.project, CTX.work = project, work
    levels = os.path.join(project, "rawdata", "levels")
    os.makedirs(levels, exist_ok=True)
    shutil.copy2(os.path.join(qa_target.REPO, "rawdata", "levels", SCENE + ".level"), levels)
    shutil.copytree(os.path.join(qa_target.REPO, "rawdata", "levels", SCENE), os.path.join(levels, SCENE),
                    dirs_exist_ok=True)
    CTX.editor = Editor()
    try:
        CTX.editor.start(project)
        if not check(open_scene(), "the small scene opens"):
            return
        st = mcp("level.status", timeout=120)
        CTX.level = st.get("level", "") or SCENE
        for p in only:
            qa_target.mark("checks")
            STEPS[p]()
    finally:
        CTX.editor.close()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--only", help="phase letters, e.g. A,B")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    ap.add_argument("--shots", help="a folder for the pictures of phase U")
    args = ap.parse_args()
    global SHOTS
    SHOTS = args.shots

    only = [p.strip().upper() for p in (args.only or PHASES).split(",") if p.strip()]
    if len(only) == 1 and len(only[0]) > 1:
        only = list(only[0])
    bad = [p for p in only if p not in STEPS]
    if bad:
        print("no such phase: %s (%s)" % (",".join(bad), PHASES))
        return 2
    only = [p for p in PHASES if p in only]
    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="gzqa_levelcmds_")
    try:
        copy = qa_target.copy_project(src, os.path.join(work, "project"), trimmed=True)
        print("running against a copy: " + copy, flush=True)
        run(copy, work, only)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if not args.keep:
            qa_target.remove_tree(work)
    ok = sum(1 for good, _ in results if good)
    print(f"LEVEL COMMANDS: DONE {ok}/{len(results)}", flush=True)
    return 0 if results and ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
