#!/usr/bin/env python3
"""The game loads a level the editor built and started with Play: on a THROW-AWAY
COPY of a project, in a game root of the editor's own, on a hidden desktop, within three minutes.

    python tools/qa/test_play_game.py --project D:\\XFinedProjects\\Test [--scene <scene>] [--keep-log <file>]

With --scene l01_escape - a level the linked game has - nothing is compiled: Play builds the project's module into
the root's modules\\<id> the way a module build does, gives the actor a Player Start where the scene's camera looks
(in Play's copy of the module), and starts a new game (-start server(all/single/alife/new)) on the game's own level
with the module mounted; the run wants the module there, no level staged, and the level loaded.

What it does:
  1. copies the project into the system temp folder and puts a scene of the repository into it (--scene, SCENES:
     mp_test by default - one arena, one light -, l03u_agr_underground - multiple-usage models with their LODs,
     sectors and portals -, or mp_close_combat - terrain with implicit light and multiple-usage models); its spawns
     give way to an actor and a graph point, a scene named like a level of the game gets a level path of its own,
     and the scene is saved;
  2. builds the level (level.build: every step in order as one job on every core - the AI map generated over the
     ground from the actor and the graph point, the editor's own level compiler in its process - into the project's
     level folder; the command answers when it ends);
  3. makes Play's game root in an empty temp folder of its own (level.play action=root root=; Play takes only a
     missing or empty folder or one carrying its play_root.txt, as deep as the linked game takes - its engine says
     so; without root= an editor with a home of its own, as every suite's, plays in <home>\\play\\<id> - never in
     the user's profile) - the linked game's engine says it runs in place (EngineAbilities), so the root holds no
     program and no link -, writes the root's own settings, then starts Play (level.play): what is stale is built
     (nothing, here), the level and editor.spawn (its game graph joined onto the linked game's, whose vertex numbers
     the game's start scripts name) are staged into the root's mirror of the loose gamedata - hard links where the
     volume allows -, and the INSTALLED engine runs where it is with -fsltx <root>\\fsgame.ltx
     -start server(editor/single/alife/new);
  4. reads the game's log until the level is loaded (LOADED, below), with no assert and no crash dump;
  5. stops the game (Play's stop; one left running is ended and fails the run);
  6. closes the editor over MCP, removes the copy and the root.

Play's answers name the installed engine (exe, exe_path) and carry no word of a way to run it (play, play_reason,
programs, program_bytes): there is one.

The level is in when the game's log holds every line of LOADED: the new game made of editor.spawn, the player made
of its actor, the client connected, the level's collision model built under the level's name, its geometry and
collision read, and the load's synchronisation ended (the actor's first update has run by then: the game's start
scripts spawn its kit there). Then five seconds more, and no assert, no fatal error, no crash dump.
Opt-in in run_all.py (--with-game): it starts the real game, half a minute of the run.

Why the installed engine may run in place: started on another fs config it writes nothing into its own folder - its
content records go to the config's app data (the suite wants <root>\\appdata\\content\\content-state.txt written
during Play), a relative root of the config is the config's folder, no update check - and it takes the archives
from the config's $arch_dir$, which the root's fsgame.ltx names by the game's own path (the game repository's
CONTENT_BUNDLES.md, "A game on another fs config"). Play starts it only when the engine and the libraries that
implement the words - xrEngine.dll, xrCore.dll, xrGame.dll - all say fsltx-in-place in their version resources, and
refuses any other game before it builds anything. The suite lists the installed game before and after the run (path,
size, write time; links listed, never followed): any difference fails. No step of it puts a program anywhere.

The editor runs on a hidden desktop (the game repository's Start-DetachedHiddenDesktopProcess.ps1), so the game it
starts is there too and no window reaches the user's screen. The game the suite started is told from the author's
by its command line (it names the root); with the installed engine running and not the suite's, the author is
playing and the suite does not start.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time

import qa
from qa import check, cmd


GAME = r"D:\Games\Dead Air"
LAUNCHER = os.path.join(GAME, "DeadAir-x64", "tools", "qa", "Start-DetachedHiddenDesktopProcess.ps1")
GAME_LIMIT = 180.0
MP_SPAWNS = ("tdm_blue_0001,tdm_green_0001,ah_cta_blue_0001,ah_cta_green_0001,ah_artefact_0001,cta_artefakt_blue,"
             "cta_artefakt_green,spectator_0001,dm_0000,ah_cta_green_base_box,ah_cta_blue_base_box,tdm_weapons_0001,"
             "tdm_weapons_0000,cta_wpn_toz34,cta_medkit,cta_ammo_12x76_zhekan,cta_ammo_12x76_zhekan_0000,"
             "cta_ammo_12x76_zhekan_0001")
# The quickest light the level compiler makes: one arena does not need the scene's own sun and sky at quality 3
QUICK_LIGHT = {"build.quality": "custom", "build.sky_quality": 0, "build.sun_quality": 0, "build.pixels_per_meter": 1}
# What Play needs of a scene the suite takes (--scene): its spawns given way to an actor and a graph point, an AI
# map, a level path the linked game does not have (a level of the game's own name keeps a one-level graph, whose
# vertices the game's start scripts do not find), build options that keep the compile short, the compiler's flags.
SCENES = {
    # one arena, one light: the multiplayer spawns go, an AI map is generated over the arena
    "mp_test": {"remove": MP_SPAWNS, "actor": (2.89, 1.0, 2.363), "graph": (1.0, 1.0, 1.0),
                "aimap": {"objects": "scenes\\mp_test\\mp_test_arena", "seed": (2.89, 1.0, 2.363)},
                "level": "mp_test", "options": QUICK_LIGHT, "flags": ""},
    # Agroprom's underground: multiple-usage models with their LODs, 31 sectors and 68 portals, its own AI map.
    # Its 301 spawns (smart terrains, restrictors, the old game's logic) go; the actor and a graph point stand where
    # the scene's own actor graph point is. The light at 4 px/m and one sample per pixel, and -noise: the static
    # visuals without progressive meshes, the MU models keep theirs.
    "l03u_agr_underground": {"remove": "*", "actor": (81.035, -20.411, -181.991),
                             "graph": (81.035, -20.411, -181.991), "aimap": None, "level": "qa_l03u",
                             "options": {**QUICK_LIGHT, "build.pixels_per_meter": 4, "build.jitter_samples": "1"},
                             "flags": "-noise"},
    # terrain with implicit light AND multiple-usage models in one level, which the game loads (the level in 19 s
    # after Play). Its 124 spawns go; the actor and a graph point stand on a team spawn of the arena; an AI map is
    # generated over the terrain from there. The light at the scene's own 10 px/m and quality, one sample per pixel
    # (at nine the compile passes the three minutes an MCP command waits); a level path of its own.
    "mp_close_combat": {"remove": "*", "actor": (-423.07376, 1.0, -391.087648), "graph": (-423.07376, 1.0, -391.087648),
                        "aimap": {"objects": "scenes\\mp_close_combat\\mp_close_combat_conteiners_terrain",
                                  "seed": (-423.07376, 1.0, -391.087648)},
                        "level": "qa_mpcc",
                        "options": {"build.quality": "custom", "build.pixels_per_meter": 10,
                                    "build.jitter_samples": "1"}, "flags": ""},
}
# a level the linked game has: Play plays the module over it, nothing is built
SCENES["l01_escape"] = {"stock": True, "level": "l01_escape"}
# the same level with an object of the module's own on it: the build makes the module's piece of the level (the
# object compiled and lit under the level's own shadows), Play ships it with the module, and the game hangs it on
# the sector it finds at its place
SCENES["l01_escape_piece"] = {"stock": True, "level": "l01_escape", "scene": "l01_escape", "piece": True}
# the containers' arena in streaming cells of sixteen metres: what is larger than a cell stays the level's, the
# rest is a piece per cell, which Play stages as a module of its own and the game loads beside the level
SCENES["mp_close_combat_cells"] = {**SCENES["mp_close_combat"], "scene": "mp_close_combat", "cells": True,
                                   "options": {**SCENES["mp_close_combat"]["options"], "build.cell_size": 16}}
SCENE = "mp_test"
# the settings the game's own QA forces (Run-NqQa.ps1, Update-QaUserLtx): a window, no sound, no pause
USER_LTX = {"renderer": "renderer_r4", "vid_mode": "1024x768", "vid_window_mode": "st_opt_windowed",
            "rs_fullscreen": "off", "rs_v_sync": "off", "rs_fps_limit": "60", "g_pause_in_background": "0",
            "snd_volume_music": "0", "snd_volume_eff": "0"}
# what the game's log says once the level is in: the new game made of editor.spawn, the level's own files read
LOADED = {
    "the new game is created": "New game is successfully created!",
    "the player is made of the spawn's actor": "# New player created.",
    "the client is connected": "client : connection accepted",
    "the level's collision model is built for the level": "cdb_cache\\<level>\\objspace.bin",
    "the level's geometry is read": "geometry level.geom",
    "the level's collision is read": "[ground] collision mesh:",
    "the level's load has ended": "End of synchronization",
}
FATAL = ("FATAL ERROR", "Expression    :", "stack trace:", "[error]", "! Cannot find level")

def wait_idle(limit: float) -> dict:
    """Until no build, compiler or game runs (level.play action=status), or `limit` seconds."""
    deadline = time.monotonic() + limit
    st = {}
    while time.monotonic() < deadline:
        st = cmd("level.play", action="status")
        if not st.get("running") and not st.get("building"):
            return st
        time.sleep(0.5)
    return st


def engine_log(root: str) -> str:
    logs = os.path.join(root, "appdata", "logs")
    found = [os.path.join(logs, n) for n in os.listdir(logs) if n.endswith(".log")] if os.path.isdir(logs) else []
    if not found:
        return ""
    newest = max(found, key=os.path.getmtime)
    return open(newest, "rb").read().decode("cp1251", "replace")


def dumps(root: str) -> list[str]:
    logs = os.path.join(root, "appdata", "logs")
    return [n for n in os.listdir(logs) if n.lower().endswith((".mdmp", ".dmp"))] if os.path.isdir(logs) else []


def user_ltx(root: str) -> None:
    """The root's own settings before Play makes the root: the player's, with the QA's knobs."""
    src = os.path.join(GAME, "appdata", "user.ltx")
    lines = open(src, encoding="cp1251").read().splitlines() if os.path.isfile(src) else []
    lines = [l for l in lines if l.split(" ", 1)[0] not in USER_LTX]
    lines += [f"{k} {v}" for k, v in USER_LTX.items()]
    os.makedirs(os.path.join(root, "appdata"), exist_ok=True)
    with open(os.path.join(root, "appdata", "user.ltx"), "w", encoding="cp1251", newline="\r\n") as f:
        f.write("\n".join(lines) + "\n")


def engines() -> list[tuple[int, str, str]]:
    """Every running process with its executable and command line."""
    try:
        out = subprocess.run(["powershell", "-NoProfile", "-Command",
                              "[Console]::OutputEncoding = [Text.Encoding]::UTF8; "
                              "Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { "
                              "$_.ExecutablePath } | ForEach-Object { \"$($_.ProcessId)|$($_.ExecutablePath)|"
                              "$($_.CommandLine)\" }"],
                             capture_output=True, encoding="utf-8", errors="replace", timeout=60).stdout or ""
    except (OSError, subprocess.SubprocessError):
        return []
    found = []
    for line in out.splitlines():
        pid, _, rest = line.partition("|")
        path, _, command = rest.partition("|")
        if pid.strip().isdigit():
            found.append((int(pid), path.strip(), command))
    return found


def games_in(root: str) -> list[int]:
    """The games Play started on the game root: any process whose command line names the root (the installed engine
    run in place on the root's fsgame.ltx), or an executable of the root."""
    real = os.path.normcase(os.path.realpath(root)).rstrip("\\")
    named = os.path.normcase(root).rstrip("\\")
    return [pid for pid, path, command in engines()
            if os.path.normcase(os.path.realpath(path)).startswith(real + os.sep)
            or named in os.path.normcase(command) or real in os.path.normcase(command)]


def players() -> list[int]:
    """The installed game's engine run by somebody else: no suite's root on its command line."""
    exe = os.path.normcase(os.path.join(GAME, "xrEngine.exe"))
    temp = os.path.normcase(tempfile.gettempdir())
    return [pid for pid, path, command in engines()
            if os.path.normcase(path) == exe and not any(p in os.path.normcase(command)
                                                         for p in (os.path.join(temp, "gzpr_"),))]


def programs_in(root: str) -> list[str]:
    """The files of the root a program is made of: a program's extension or an MZ head. Links are not followed."""
    found = []
    for d, dirs, files in os.walk(root):
        dirs[:] = [n for n in dirs if not (os.path.islink(os.path.join(d, n))
                                           or getattr(os.lstat(os.path.join(d, n)), "st_reparse_tag", 0))]
        for n in files:
            p = os.path.join(d, n)
            head = b""
            try:
                with open(p, "rb") as f:
                    head = f.read(2)
            except OSError:
                pass
            if n.lower().endswith((".exe", ".dll", ".com", ".scr", ".bat", ".cmd", ".ps1", ".msi")) or head == b"MZ":
                found.append(os.path.relpath(p, root))
    return found


def links_in(root: str) -> list[str]:
    found = []
    for d, dirs, _files in os.walk(root):
        for n in list(dirs):
            p = os.path.join(d, n)
            if os.path.islink(p) or getattr(os.lstat(p), "st_reparse_tag", 0):
                found.append(os.path.relpath(p, root))
                dirs.remove(n)
    return found


def unlink_archives(root: str) -> None:
    link = os.path.join(root, "database")
    if os.path.lexists(link):
        # a junction: the link goes, the game's archives stay
        os.rmdir(link)


def sweep_stale() -> None:
    """What a run that was killed left in the temp folder. A root of an earlier run may still hold
    a junction to the game's archives: the link goes first, then the folders - never a delete through the link. A
    root a game still runs from is left alone."""
    temp = tempfile.gettempdir()
    for name in os.listdir(temp):
        path = os.path.join(temp, name)
        if not os.path.isdir(path) or not (name.startswith("gzpr_") or name.startswith("gzpg_")):
            continue
        if name.startswith("gzpr_"):
            if games_in(path):
                continue
            unlink_archives(path)
        print("removing what an earlier run left: %s" % path, flush=True)
        qa.remove_tree(path)


def listing(top: str) -> dict[str, tuple[int, int]]:
    """Every file and folder under `top` with its size and write time; a link is listed, never followed. A `.git`
    is left out: the game folder holds the game's source repository, whose bookkeeping any git command rewrites -
    the author's, meanwhile - and which no game reads or writes."""
    out: dict[str, tuple[int, int]] = {}
    for d, dirs, files in os.walk(top):
        keep = []
        for n in dirs:
            if n.lower() == ".git":
                continue
            p = os.path.join(d, n)
            st = os.lstat(p)
            out[os.path.relpath(p, top)] = (0, st.st_mtime_ns)
            if not (os.path.islink(p) or getattr(st, "st_reparse_tag", 0)):
                keep.append(n)
        dirs[:] = keep
        for n in files:
            if n.lower() == ".git":
                continue
            p = os.path.join(d, n)
            try:
                st = os.lstat(p)
            except OSError:
                continue
            out[os.path.relpath(p, top)] = (st.st_size, st.st_mtime_ns)
    return out


def prepare_scene(scene: str) -> bool:
    """The scene opened and made ready for Play (SCENES), saved."""
    cfg = SCENES[scene]
    opened = cmd("scene.open", path="rawdata/levels/%s.level" % cfg.get("scene", scene), discard=True)
    if not check(opened.get("ok"), "the scene opens", qa.short(opened)):
        return False
    remove = cfg["remove"]
    if remove == "*":
        listed = cmd("scene.objects", classes=["spawn"], limit=100000).get("objects", [])
        names = [o["name"] for o in listed]
    else:
        names = remove.split(",")
    steps = [("scene.delete", {"objects": names}),
             ("scene.place", {"kind": "spawn", "section": "actor", "position": list(cfg["actor"])}),
             ("scene.place", {"kind": "spawn", "section": "graph_point", "position": list(cfg["graph"])})]
    if cfg["aimap"]:
        # the AI map lies on the arena's ground alone: the build generates it there from the spawn points
        steps.append(("scene.ai_map", {"action": "set", "objects": cfg["aimap"]["objects"].split(",")}))
    world = dict(cfg["options"])
    if cfg["level"] != cfg.get("scene", scene):
        world["level.path"] = cfg["level"]
    steps.append(("scene.set", {"world": True, "set": world}))
    steps.append(("scene.save", {}))
    for name, args in steps:
        a = cmd(name, **args)
        if not check(a.get("ok"), f"{name} answers ok", qa.short(a)):
            return False
    return True


def run_scene(project: str, root: str, scene: str, keep_log: str | None) -> None:
    cfg = SCENES[scene]
    level = cfg["level"]
    loaded = {k: v.replace("<level>", level) for k, v in LOADED.items()}
    src = cfg.get("scene", scene)
    os.makedirs(os.path.join(project, "rawdata", "levels"), exist_ok=True)
    shutil.copy2(os.path.join(qa.REPO, "rawdata", "levels", src + ".level"),
                 os.path.join(project, "rawdata", "levels", src + ".level"))
    shutil.copytree(os.path.join(qa.REPO, "rawdata", "levels", src), os.path.join(project, "rawdata", "levels", src),
                    dirs_exist_ok=True)
    if cfg.get("stock"):
        opened = cmd("scene.open", path="rawdata/levels/%s.level" % src, discard=True, timeout=280)
        if check(opened.get("ok"), "the scene of a level the game has opens", qa.short(opened)):
            wait_idle(120)
            st = cmd("level.status", timeout=120)
            check(st.get("stock") is True, "level.status: the level is the game's, nothing to compile", qa.short(st))
            # what an earlier build of the project left in the level's folder would replace the game's own
            if st.get("replaces"):
                gone = qa.gate("destroy", "level.clean")
                check(gone.get("ok"), "the files an earlier build left over the game's level are removed (%d)"
                      % len(gone.get("removed") or []), qa.short(gone))
            pieces = None
            if cfg.get("piece"):
                if not build_piece(project, level):
                    return
                pieces = ["main"]
            play_in_place(root, level, loaded, keep_log, stock=True, pieces=pieces)
        return
    if not prepare_scene(scene):
        return
    t = time.monotonic()
    flags = {f.lstrip("-"): True for f in cfg["flags"].split()}
    done = cmd("level.build", timeout=280, **flags)
    ran = {r.get("step"): r for r in done.get("ran") or []}
    report = (ran.get("light") or {}).get("note") or {}
    compile_secs = time.monotonic() - t
    check(done.get("ok") and report.get("parts") and "ai_map" in ran and "spawn" in ran,
          "level.build builds the level, the editor's level compiler in its process (%.1f s, %s threads)"
          % (compile_secs, report.get("threads")), qa.short({k: v for k, v in done.items() if k != "ran"}))
    print("TIME build %.1f s" % compile_secs, flush=True)
    wait_idle(30)
    level_dir = os.path.join(project, "gamedata", "levels", level)
    compiled = os.path.isdir(level_dir) and all(os.path.isfile(os.path.join(level_dir, f))
                                                for f in ("level", "level.geom", "level.cform"))
    check(compiled, "the level's files are in the project's level folder",
          ", ".join(sorted(os.listdir(level_dir))) if os.path.isdir(level_dir) else "no folder")
    if not compiled:
        return
    pieces = None
    if cfg.get("cells"):
        pieces = cells_of(project, level)
        if not pieces:
            return

    play_in_place(root, level, loaded, keep_log, pieces=pieces)


def pieces_seen(level: str, names: list[str]) -> bool:
    """level.inspect part=pieces: the pieces the build wrote are there, read back without a problem."""
    seen = cmd("level.inspect", part="pieces", level=level, limit=500)
    rows = {r.get("name"): r for r in seen.get("pieces") or []}
    bad = {n: r.get("problems") for n, r in rows.items() if r.get("problems")}
    return check(seen.get("ok") and all(n in rows for n in names) and not bad,
                 "level.inspect reads the level's %d piece(s) back without a problem (%s visuals)"
                 % (len(rows), sum(r.get("visuals") or 0 for r in rows.values())),
                 qa.short({"names": sorted(rows)[:6], "problems": bad, "answer": {k: v for k, v in seen.items() if k != "pieces"}}))


def build_piece(project: str, level: str) -> bool:
    """An object of the module's own placed on the game's level, and the module's piece of the level built: the
    piece's files in the module's levels\\<level>\\pieces\\main, its collision in overlay.xcform."""
    listed = cmd("scene.objects", classes=["object"], limit=200).get("objects") or []
    ref = next((o.get(k) for o in listed for k in ("ref", "reference") if o.get(k)), None)
    if not check(bool(ref), "the scene names a library object to place a copy of", qa.short(listed[:2])):
        return False
    placed = cmd("scene.place", kind="object", ref=ref, name="qa_piece_object")
    if not check(placed.get("ok"), "an object of the module's own is placed in front of the camera (%s)" % ref,
                 qa.short(placed)):
        return False
    st = cmd("level.status", timeout=120)
    step = next((s for s in st.get("steps") or [] if s.get("step") == "piece"), {})
    check(step.get("state") in ("never", "stale", "never_built"), "level.status: the module's piece of the level is to be built",
          qa.short(step or st))
    t = time.monotonic()
    done = cmd("level.build", timeout=280)
    ran = {r.get("step"): r for r in done.get("ran") or []}
    if not check(done.get("ok") and "piece" in ran, "level.build builds the module's piece of the game's level (%.1f s)"
                 % (time.monotonic() - t), qa.short({k: v for k, v in done.items() if k != "ran"} | {"ran": sorted(ran)})):
        return False
    print("TIME piece %.1f s" % (time.monotonic() - t), flush=True)
    wait_idle(30)
    folder = os.path.join(project, "levels", level, "pieces", "main")
    have = sorted(os.listdir(folder)) if os.path.isdir(folder) else []
    check(all(n in have for n in ("piece", "piece.geom", "piece.ltx")) and any(n.startswith("lmap#") for n in have),
          "the piece's visuals, buffers, manifest and lightmap pages are in the module's levels\\%s\\pieces\\main" % level,
          ", ".join(have))
    check(os.path.isfile(os.path.join(project, "levels", level, "overlay.xcform")),
          "the object's collision is in the module's overlay.xcform")
    scratch = os.path.join(project, "temp", "piece_" + level)
    check(os.path.isfile(os.path.join(scratch, "build.prj")) and "build.prj" not in have,
          "the step's own files are under the project's temp, never beside the piece")
    return pieces_seen(level, ["main"])


def cells_of(project: str, level: str) -> list[str]:
    """The streaming cells the build wrote for the level: the pieces of the module's levels\\<level>\\pieces."""
    folder = os.path.join(project, "levels", level, "pieces")
    names = sorted(n for n in os.listdir(folder) if n.startswith("cell_")) if os.path.isdir(folder) else []
    if not check(len(names) > 1, "the level is built in streaming cells: %d of them beside its base" % len(names),
                 folder):
        return []
    return names if pieces_seen(level, names) else []


# what an answer of Play said while it had a second way to run the game
GONE = ("play", "play_reason", "programs", "program_bytes")


def no_way_named(answer: dict) -> list[str]:
    """The keys of GONE an answer (or its run) still carries."""
    return [k for a in (answer, answer.get("run") or {}) for k in GONE if k in a]


def play_in_place(root: str, level: str, loaded: dict[str, str], keep_log: str | None, stock: bool = False,
                  pieces: list[str] | None = None) -> bool:
    """Play's root made, Play started on it - the installed engine where it is -, the level loaded, the game
    stopped. False when Play did not start the game."""
    # Play makes its root only in an empty folder (play_root.txt marks it its own): made first, then the root's own
    # settings written into it, then Play
    t = time.monotonic()
    made = cmd("level.play", action="root", root=root)
    engine = os.path.join(GAME, "xrEngine.exe").replace("\\", "/")
    check(made.get("ok") and made.get("was") == "empty" and made.get("exe_path", "").lower() == engine.lower(),
          "Play makes its root in the empty folder for the installed engine in place (%.1f s, paths up to %s)"
          % (time.monotonic() - t, made.get("path_limit")), qa.short(made))
    check(made.get("ok") and not no_way_named(made), "action=root's answer names no way to run the game",
          str(no_way_named(made)))
    top = sorted(os.listdir(root)) if os.path.isdir(root) else []
    check(top == ["appdata", "fsgame.ltx", "gamedata", "modules", "play_root.txt"] and not programs_in(root)
          and not links_in(root), "the root holds the marker, fsgame.ltx, gamedata, appdata and modules - no "
          "program, no link", "top %s, programs %s, links %s" % (top, programs_in(root)[:5], links_in(root)))
    fsgame = open(os.path.join(root, "fsgame.ltx"), encoding="cp1251").read() if "fsgame.ltx" in top else ""
    arch = next((l for l in fsgame.splitlines() if l.startswith("$arch_dir$")), "")
    check(("%s\\|" % GAME).lower() in arch.lower(),
          "the root's fsgame.ltx names the installed archives: " + arch.strip())
    user_ltx(root)
    t0, play_wall = time.monotonic(), time.time()
    play = cmd("level.play", action="start", root=root, params="-i -always_active -silent_error_mode -force_flushlog",
               timeout=280)
    if not check(play.get("ok") and play.get("started") and play.get("exe", "").lower() == engine.lower(),
                 "Play starts the installed engine in place on its root", qa.short(play)):
        return False
    check(not no_way_named(play), "Play's answer names no way to run the game", str(no_way_named(play)))
    print("play: " + qa.short(play), flush=True)
    staged = os.path.join(root, "gamedata", "levels", level)
    if stock:
        module = (play.get("module_dir") or "").replace("/", "\\")
        check(play.get("way") == "module" and os.path.isfile(os.path.join(module, "mod.ltx"))
              and "-start server(all/single/alife/new)" in str(play.get("command")),
              "Play plays the module: it is in the root's modules\\<id> and a new game starts over the game's level",
              qa.short(play))
        check(not os.path.isdir(staged) and not os.path.isfile(os.path.join(root, "gamedata", "spawns", "editor.spawn")),
              "nothing of the level is staged: the game loads its own")
    else:
        check(all(os.path.isfile(os.path.join(staged, f)) for f in ("level", "level.ai", "level.geom")) and
              os.path.isfile(os.path.join(root, "gamedata", "spawns", "editor.spawn")),
              "the level, its level.ai and editor.spawn are staged in the game root")
        if pieces:
            there = os.path.join(root, "modules", "grizzly_play_level", "levels", level, "pieces")
            check(os.path.isfile(os.path.join(root, "modules", "grizzly_play_level", "mod.ltx")) and
                  all(os.path.isfile(os.path.join(there, n, "piece.ltx")) for n in pieces),
                  "the level's %d cells are staged as a module of Play's own, where the game reads pieces" % len(pieces),
                  there)
    text, loaded_at = "", None
    while time.monotonic() - t0 < GAME_LIMIT:
        text = engine_log(root)
        if all(m in text for m in loaded.values()) or any(f in text for f in FATAL) or dumps(root):
            loaded_at = time.monotonic() - t0
            # a moment more: the first frames of the level, where a missing actor or graph would stop it
            time.sleep(5)
            text = engine_log(root)
            break
        if not cmd("level.play", action="status").get("running"):
            text = engine_log(root)
            break
        if not text and time.monotonic() - t0 > 60:
            # a minute without a line of the game's log: it never started its file system
            break
        time.sleep(1)
    seconds = time.monotonic() - t0
    still = cmd("level.play", action="status").get("running")
    cmd("level.play", action="stop")
    wait_idle(30)
    left = games_in(root)
    for pid in left:
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True)
    check(not left, "Play's stop ends the game", "still running: pids %s (ended)" % left)
    for what, marker in loaded.items():
        line = next((l for l in text.splitlines() if marker in l), "")
        check(bool(line), f"the game's log: {what}" + (f" - {line.strip()[:150]}" if line else ""), marker)
    lines = text.splitlines()
    bad = [l.strip() for l in lines if any(f in l for f in FATAL)]
    # the frame a crash stopped in, and the last line the game wrote before it
    for i, l in enumerate(lines):
        if l.startswith("stack trace:"):
            frame = next((x.strip() for x in lines[i + 1:] if x.strip()), "")
            before = next((x.strip() for x in reversed(lines[:i]) if x.strip()), "")
            bad.append(f"after '{before}' the game stopped in {frame[:220]}")
            break
    check(not bad, "no assert and no fatal error in the game's log", "; ".join(bad[:3]))
    if pieces:
        said = [l.strip() for l in lines if "[pieces]" in l]
        found = next((l for l in said if "piece(s) found" in l), "")
        attached = [l for l in said if ": attached," in l]
        refused = [l for l in said if l.startswith("!")]
        check((": %d piece(s) found" % len(pieces)) in found,
              "the game finds the level's %d piece(s): %s" % (len(pieces), found[:120]), "; ".join(said[:4]))
        check(len(attached) == len(pieces) and all(any(("/%s: attached" % n) in l for l in attached) for n in pieces),
              "the game attaches every piece to the level's sectors: %s" % (attached[0][:150] if attached else "-"),
              "; ".join(said[:6]))
        check(not refused, "the game refuses no piece", "; ".join(refused[:3]))
    check(not dumps(root), "no crash dump", ", ".join(dumps(root)))
    state = os.path.join(root, "appdata", "content", "content-state.txt")
    check(os.path.isfile(state) and os.path.getmtime(state) >= play_wall - 1,
          "the game kept its content records in the root's app data (appdata\\content\\content-state.txt)")
    check("The fs config lies outside the application folder" in text,
          "the game says it runs on another fs config and only reads the installation")
    check(not programs_in(root) and not links_in(root), "the root still holds no program and no link after Play",
          ", ".join(programs_in(root)[:5] + links_in(root)[:5]))
    check(bool(still), "the game was still running when it was stopped (%.0f s after Play, the level in at %s s)"
          % (seconds, "%.0f" % loaded_at if loaded_at is not None else "-"))
    print("TIME in_place %.1f s, the level in at %s s" % (seconds, "%.0f" % loaded_at if loaded_at is not None else "-"),
          flush=True)
    if keep_log:
        os.makedirs(os.path.dirname(os.path.abspath(keep_log)), exist_ok=True)
        with open(keep_log, "w", encoding="utf-8") as f:
            f.write(text)
        print("engine log kept: " + keep_log, flush=True)
        # what the editor's log says of the run (the game's own output lines go there)
        import glob
        lines = []
        for p in glob.glob(os.path.join(qa.home(), "logs", "grizzly_*.log")):
            lines += [l for l in open(p, encoding="utf-8", errors="replace").read().splitlines()
                      if "Run " in l or "game" in l.lower() or "play" in l.lower() or l.startswith("!")]
        with open(os.path.join(os.path.dirname(os.path.abspath(keep_log)), "editor.log"), "w", encoding="utf-8") as f:
            f.write("\n".join(lines[-400:]) + "\n")
        found = []
        for d, _dirs, files in os.walk(os.path.join(root, "appdata")):
            found += [os.path.relpath(os.path.join(d, n), root) for n in files]
        print("the root's appdata: " + ", ".join(sorted(found)[:40]), flush=True)
    return True


def run(ctx: qa.Run) -> None:
    args = ctx.args
    if players():
        raise RuntimeError("the installed game runs and the suite did not start it (pids %s): the author is playing"
                           % players())
    if qa.port_open():
        raise RuntimeError("an editor already answers on 127.0.0.1:%d - close it first" % qa.PORT)
    sweep_stale()
    before = listing(GAME)
    archives = len(os.listdir(os.path.join(GAME, "database")))
    work = ctx.work
    # a folder of its own in the temp folder; the game run in place on it is told apart by this path
    root = tempfile.mkdtemp(prefix="gzpr_")
    project = ctx.project
    pid_file, launcher_file = os.path.join(work, "editor.pid"), os.path.join(work, "launcher.pid")
    try:
        exe = qa.exe()
        line = " ".join(f'"{a}"' if " " in a else a for a in
                        ["-mcp_port", str(qa.PORT), "-home", qa.home(), "-headless", "-nodlg", "-project", project])
        # the value goes with a space in front: pwsh -File takes a value that starts with '-' for a parameter;
        # no pipe either - the launcher's own launcher inherits it and the call would wait for it to end
        launcher = subprocess.run(
            ["pwsh", "-NoProfile", "-File", LAUNCHER, "-FilePath", exe, "-Arguments", " " + line,
             "-ProcessIdFile", pid_file, "-LauncherIdFile", launcher_file, "-DesktopName", "GrizzlyPlayQa",
             "-TimeoutSeconds", "900"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
        check(launcher.returncode == 0, "the editor is started on a hidden desktop")
        # the project is open once `state` says so: the endpoint answers before the project has finished opening
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                if cmd("state", timeout=30).get("project"):
                    break
            except OSError:
                time.sleep(0.1)
        qa.mark("editor start (hidden desktop)")
        run_scene(project, root, args.scene, args.keep_log)
    finally:
        try:
            cmd("level.play", action="stop", timeout=30)
            cmd("app.quit", discard=True, timeout=30)
        except OSError:
            pass
        deadline = time.monotonic() + 60
        editor_pid = int(open(pid_file).read().strip()) if os.path.isfile(pid_file) else 0
        while editor_pid and time.monotonic() < deadline:
            alive = subprocess.run(["tasklist", "/FI", f"PID eq {editor_pid}", "/NH"], capture_output=True,
                                   text=True).stdout
            if str(editor_pid) not in alive:
                break
            time.sleep(0.5)
        else:
            if editor_pid:
                print(f"! the editor did not leave: ending pid {editor_pid}", flush=True)
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(editor_pid)], capture_output=True)
        # a run cut short leaves no game behind either
        for pid in games_in(root):
            print(f"! the game did not leave: ending pid {pid}", flush=True)
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True)
        if os.environ.get("GRIZZLY_PLAY_KEEP"):
            print("kept for a look: %s, %s" % (work, root), flush=True)
        else:
            unlink_archives(root)
            qa.remove_tree(root)
            check(not os.path.exists(root) and len(os.listdir(os.path.join(GAME, "database"))) == archives,
                  "the root is gone and the installed database keeps its %d entries" % archives)
    after = listing(GAME)
    differ = sorted(k for k in set(before) | set(after) if before.get(k) != after.get(k))
    check(not differ, "nothing of the installed game changed (%d entries listed before and after)" % len(before),
          ", ".join(differ[:10]))


def main() -> int:
    def more(ap) -> None:
        ap.add_argument("--keep-log")
        ap.add_argument("--scene", default=SCENE, choices=sorted(SCENES), help="the scene Play takes (default mp_test)")

    return qa.suite("PLAY GAME", run, doc=__doc__, add_args=more)


if __name__ == "__main__":
    sys.exit(main())
