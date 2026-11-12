#!/usr/bin/env python3
"""The game loads a level the Rust editor built and started with Play (plan D1, phase 6's gate): on a THROW-AWAY
COPY of a project, in a game root of the editor's own, on a hidden desktop, within three minutes.

    python tools/qa/test_play_game.py --project D:\\XFinedProjects\\Test [--scene <scene>] [--keep-log <file>]

What it does:
  1. copies the project into the system temp folder and puts a scene of the repository into it (--scene, SCENES:
     mp_test by default - one arena, one light -, l03u_agr_underground - multiple-usage models with their LODs,
     sectors and portals -, or mp_close_combat - terrain with implicit light and multiple-usage models); its spawns
     give way to an actor and a graph point, an AI map is generated where the scene has none, a scene named like a
     level of the game gets a level path of its own, and the scene is saved and built (Make All);
  2. compiles the geometry with the editor's own level compiler, in its process (level_build compile_geometry: a
     job on every core, into the project's level folder; the command answers when it ends, with its report);
  3. makes Play's game root in an empty temp folder of its own (level_build play_root root=; Play takes only a
     missing or empty folder or one carrying its play_root.txt, as deep as the linked game takes - its engine says
     so; without root= an editor with a home of its own, as every suite's, plays in <home>\\play\\<id> - never in
     the user's profile) - the linked game's engine says it runs in place (EngineAbilities), so the root holds no
     program and no link -, writes the root's own settings, then starts Play: the editor builds the collision, the
     AI map, the game graph (joined onto the linked game's, whose vertex numbers the game's start scripts name) and
     editor.spawn, stages the level into the root's copy of the loose gamedata, and runs the INSTALLED engine where
     it is with -fsltx <root>\\fsgame.ltx -start server(editor/single/alife/new);
  4. reads the game's log until the level is loaded (LOADED, below), with no assert and no crash dump;
  5. stops the game (Play's stop; one left running is ended and fails the run), closes the editor over MCP,
     removes the copy and the root (the archives' link first, as a link).

The level is in when the game's log holds every line of LOADED: the new game made of editor.spawn, the player made
of its actor, the client connected, the level's collision model built under the level's name, its geometry and
collision read, and the load's synchronisation ended (the actor's first update has run by then: the game's start
scripts spawn its kit there). Then five seconds more, and no assert, no fatal error, no crash dump.
Opt-in in run_all.py (--with-game): it starts the real game, half a minute of the run.

Why the installed engine may run in place: started on another fs config it writes nothing into its own folder - its
content records go to the config's app data (the suite wants <root>\\appdata\\content\\content-state.txt written
during Play), a relative root of the config is the config's folder, no update check - and it takes the archives
from the config's $arch_dir$, which the root's fsgame.ltx names by the game's own path (the game repository's
CONTENT_BUNDLES.md, "A game on another fs config"). The suite lists the installed game before and after the run
(path, size, write time; links listed, never followed): any difference fails.

The editor runs on a hidden desktop (the game repository's Start-DetachedHiddenDesktopProcess.ps1), so the game it
starts is there too and no window reaches the user's screen. The game the suite started is told from the author's
by its command line (it names the root); with the installed engine running and not the suite's, the author is
playing and the suite does not start.
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
GAME = r"D:\Games\Dead Air"
LAUNCHER = os.path.join(GAME, "DeadAir-x64", "tools", "qa", "Start-DetachedHiddenDesktopProcess.ps1")
GAME_LIMIT = 180.0
MCP = qa_target.MCP
MP_SPAWNS = ("tdm_blue_0001,tdm_green_0001,ah_cta_blue_0001,ah_cta_green_0001,ah_artefact_0001,cta_artefakt_blue,"
             "cta_artefakt_green,spectator_0001,dm_0000,ah_cta_green_base_box,ah_cta_blue_base_box,tdm_weapons_0001,"
             "tdm_weapons_0000,cta_wpn_toz34,cta_medkit,cta_ammo_12x76_zhekan,cta_ammo_12x76_zhekan_0000,"
             "cta_ammo_12x76_zhekan_0001")
# The quickest light the level compiler makes: one arena does not need the scene's own sun and sky at quality 3
QUICK_LIGHT = {"Scene\\Build options\\Quality": "Custom", "Scene\\Build options\\Lighting\\Hemisphere quality [0-3]": "0",
               "Scene\\Build options\\Lighting\\Sun shadow quality [0-3]": "0",
               "Scene\\Build options\\Lighting\\Pixel per meter": "1"}
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
    # the scene's own actor graph point is. The light at diff_compile.py's options for it (4 px/m, one jitter
    # sample) and -noise: the static visuals without progressive meshes, the MU models keep theirs.
    "l03u_agr_underground": {"remove": "*", "actor": (81.035, -20.411, -181.991),
                             "graph": (81.035, -20.411, -181.991), "aimap": None, "level": "qa_l03u",
                             "options": {**QUICK_LIGHT, "Scene\\Build options\\Lighting\\Pixel per meter": "4",
                                         "Scene\\Build options\\Lighting\\Jitter samples": "1 - Low"},
                             "flags": "-noise"},
    # terrain with implicit light AND multiple-usage models - the pair the compiler's proof has no C++ record of
    # (diff_compile.py empties the MU models for its implicit record), so the game judges it. Its 124 spawns go; the
    # actor and a graph point stand on a team spawn of the arena; an AI map is generated over the terrain from
    # there. The light at diff_compile.py's options for its implicit record: the scene's own 10 px/m and quality,
    # one jitter sample (at nine the compile passes the three minutes an MCP command waits); a level path of its own.
    "mp_close_combat": {"remove": "*", "actor": (-423.07376, 1.0, -391.087648), "graph": (-423.07376, 1.0, -391.087648),
                        "aimap": {"objects": "scenes\\mp_close_combat\\mp_close_combat_conteiners_terrain",
                                  "seed": (-423.07376, 1.0, -391.087648)},
                        "level": "qa_mpcc",
                        "options": {"Scene\\Build options\\Quality": "Custom",
                                    "Scene\\Build options\\Lighting\\Pixel per meter": "10",
                                    "Scene\\Build options\\Lighting\\Jitter samples": "1 - Low"}, "flags": ""},
}
SCENE = "mp_test"
# the settings the game's own QA forces (Run-NqQa.ps1, Update-QaUserLtx): a window, no sound, no pause
USER_LTX = {"renderer": "renderer_r4", "vid_mode": "1024x768", "vid_window_mode": "st_opt_windowed",
            "rs_fullscreen": "off", "rs_v_sync": "off", "rs_fps_limit": "60", "g_pause_in_background": "0",
            "snd_volume_music": "0", "snd_volume_eff": "0"}
# what the game's log says once the level is in: the new game made of editor.spawn, the level's own files read
LOADED = {
    "the new game is created from editor.spawn": "New game is successfully created!",
    "the player is made of the spawn's actor": "# New player created.",
    "the client is connected": "client : connection accepted",
    "the level's collision model is built for the level": "cdb_cache\\<level>\\objspace.bin",
    "the level's geometry is read": "geometry level.geom",
    "the level's collision is read": "[ground] collision mesh:",
    "the level's load has ended": "End of synchronization",
}
FATAL = ("FATAL ERROR", "Expression    :", "stack trace:", "[error]", "! Cannot find level")

results: list[tuple[bool, str]] = []


def check(ok, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 300.0, **extra) -> dict:
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps({"cmd": cmd, **extra}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def short(a: dict) -> str:
    return json.dumps(a, ensure_ascii=False)[:400]


def wait_idle(limit: float) -> dict:
    """Until no compiler or game runs (level_build status), or `limit` seconds."""
    deadline = time.monotonic() + limit
    st = {}
    while time.monotonic() < deadline:
        st = mcp("level_build", action="status")
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
    """The games Play started on the game root: an executable of the root (a copied engine), or any process whose
    command line names the root (the installed engine run in place on the root's fsgame.ltx)."""
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
    """What a run that was killed left in the temp folder: its game root still holds a junction to the game's
    archives. The link goes first, then the folders - never a delete through the link. A root a game still runs
    from is left alone."""
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
        qa_target.remove_tree(path)


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
    opened = mcp("open_scene", scene=scene + ".level", discard=True)
    if not check(opened.get("ok"), "the scene opens", short(opened)):
        return False
    remove = cfg["remove"]
    if remove == "*":
        listed = mcp("list_objects", **{"class": "spawn", "limit": 100000}).get("objects", [])
        remove = ",".join(o["name"] for o in listed)
    steps = [("select_objects", {"names": remove}), ("delete_selected", {}),
             ("place_spawn", {"section": "actor", "x": cfg["actor"][0], "y": cfg["actor"][1], "z": cfg["actor"][2]}),
             ("place_spawn", {"section": "graph_point", "x": cfg["graph"][0], "y": cfg["graph"][1],
                              "z": cfg["graph"][2]})]
    if cfg["aimap"]:
        seed = list(cfg["aimap"]["seed"])
        steps += [("select_objects", {"names": cfg["aimap"]["objects"]}), ("aimap_tool", {"show": True}),
                  ("snap_objects", {"action": "set_from_selection"}),
                  ("aimap_tool", {"action": "add_nodes", "position": seed}),
                  ("aimap_tool", {"action": "generate", "timeout": 120})]
    if cfg["level"] != scene:
        steps.append(("property_set", {"target": "world", "key": "Scene\\Build options\\Level path",
                                       "value": cfg["level"]}))
    steps += [("property_set", {"target": "world", "key": k, "value": v}) for k, v in cfg["options"].items()]
    steps.append(("save_scene", {}))
    for cmd, args in steps:
        a = mcp(cmd, **args)
        if not check(a.get("ok"), f"{cmd} answers ok", short(a)):
            return False
    return True


def run(project: str, root: str, scene: str, keep_log: str | None) -> None:
    cfg = SCENES[scene]
    level = cfg["level"]
    loaded = {k: v.replace("<level>", level) for k, v in LOADED.items()}
    shutil.copy2(os.path.join(REPO, "rawdata", "levels", scene + ".level"),
                 os.path.join(project, "rawdata", "levels", scene + ".level"))
    shutil.copytree(os.path.join(REPO, "rawdata", "levels", scene), os.path.join(project, "rawdata", "levels", scene),
                    dirs_exist_ok=True)
    if not prepare_scene(scene):
        return
    built = mcp("level_build", action="make_all")
    if not check(built.get("ok"), "Make All builds the level", short(built)):
        return
    t = time.monotonic()
    done = mcp("level_build", action="compile_geometry", params=cfg["flags"], timeout=280)
    report = done.get("report") or {}
    compile_secs = time.monotonic() - t
    check(done.get("ok") and report.get("parts"), "the editor's level compiler compiled the level in its process "
          "(%.1f s, %s threads)" % (compile_secs, report.get("threads")),
          short({k: v for k, v in done.items() if k != "report"}))
    print("TIME compile %.1f s" % compile_secs, flush=True)
    wait_idle(30)
    level_dir = os.path.join(project, "gamedata", "levels", level)
    compiled = os.path.isdir(level_dir) and all(os.path.isfile(os.path.join(level_dir, f))
                                                for f in ("level", "level.geom", "level.cform"))
    check(compiled, "the level's files are in the project's level folder",
          ", ".join(sorted(os.listdir(level_dir))) if os.path.isdir(level_dir) else "no folder")
    if not compiled:
        return

    # Play makes its root only in an empty folder (play_root.txt marks it its own): made first, then the root's
    # own settings written into it, then Play
    t = time.monotonic()
    made = mcp("level_build", action="play_root", root=root)
    engine = os.path.join(GAME, "xrEngine.exe").replace("\\", "/")
    check(made.get("ok") and made.get("was") == "empty" and made.get("play") == "in_place"
          and made.get("exe_path", "").lower() == engine.lower() and made.get("programs") == 0,
          "Play makes its root in the empty folder for the installed engine in place (%.1f s, paths up to %s)"
          % (time.monotonic() - t, made.get("path_limit")), short(made))
    top = sorted(os.listdir(root)) if os.path.isdir(root) else []
    check(top == ["appdata", "fsgame.ltx", "gamedata", "modules", "play_root.txt"] and not programs_in(root)
          and not links_in(root), "the root holds the marker, fsgame.ltx, gamedata, appdata and modules - no program, "
          "no link", "top %s, programs %s, links %s" % (top, programs_in(root)[:5], links_in(root)))
    fsgame = open(os.path.join(root, "fsgame.ltx"), encoding="cp1251").read() if "fsgame.ltx" in top else ""
    arch = next((l for l in fsgame.splitlines() if l.startswith("$arch_dir$")), "")
    check(("%s\\|" % GAME).lower() in arch.lower(), "the root's fsgame.ltx names the installed archives: " + arch.strip())
    user_ltx(root)
    t0, play_wall = time.monotonic(), time.time()
    play = mcp("level_build", action="play", root=root, params="-i -always_active -silent_error_mode -force_flushlog")
    if not check(play.get("ok") and play.get("run", {}).get("started") and play.get("play") == "in_place"
                 and play.get("run", {}).get("exe", "").lower() == engine.lower(),
                 "Play starts the installed engine in place on its root", short(play)):
        return
    print("play: " + short(play), flush=True)
    staged = os.path.join(root, "gamedata", "levels", level)
    check(all(os.path.isfile(os.path.join(staged, f)) for f in ("level", "level.ai", "level.geom")) and
          os.path.isfile(os.path.join(root, "gamedata", "spawns", "editor.spawn")),
          "the level, its level.ai and editor.spawn are staged in the game root")
    text, loaded_at = "", None
    while time.monotonic() - t0 < GAME_LIMIT:
        text = engine_log(root)
        if all(m in text for m in loaded.values()) or any(f in text for f in FATAL) or dumps(root):
            loaded_at = time.monotonic() - t0
            # a moment more: the first frames of the level, where a missing actor or graph would stop it
            time.sleep(5)
            text = engine_log(root)
            break
        if not mcp("level_build", action="status").get("running"):
            text = engine_log(root)
            break
        if not text and time.monotonic() - t0 > 60:
            # a minute without a line of the game's log: it never started its file system
            break
        time.sleep(1)
    seconds = time.monotonic() - t0
    still = mcp("level_build", action="status").get("running")
    mcp("level_build", action="play_stop")
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
    if keep_log:
        os.makedirs(os.path.dirname(os.path.abspath(keep_log)), exist_ok=True)
        with open(keep_log, "w", encoding="utf-8") as f:
            f.write(text)
        print("engine log kept: " + keep_log, flush=True)
        # what the editor's log says of the run (the game's own output lines go there)
        import glob
        lines = []
        for p in glob.glob(qa_target.log_glob()):
            lines += [l for l in open(p, encoding="utf-8", errors="replace").read().splitlines()
                      if "Run " in l or "game" in l.lower() or "play" in l.lower() or l.startswith("!")]
        with open(os.path.join(os.path.dirname(os.path.abspath(keep_log)), "editor.log"), "w", encoding="utf-8") as f:
            f.write("\n".join(lines[-400:]) + "\n")
        found = []
        for d, _dirs, files in os.walk(os.path.join(root, "appdata")):
            found += [os.path.relpath(os.path.join(d, n), root) for n in files]
        print("the root's appdata: " + ", ".join(sorted(found)[:40]), flush=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True)
    ap.add_argument("--keep-log")
    ap.add_argument("--scene", default=SCENE, choices=sorted(SCENES), help="the scene Play takes (default mp_test)")
    args = ap.parse_args()
    if not qa_target.RUST:
        print("the Rust editor's Play only: set GRIZZLY_QA_TARGET=rust")
        return 2
    try:
        socket.create_connection(MCP, timeout=1).close()
        print(qa_target.busy_text())
        return 2
    except OSError:
        pass
    if players():
        print("the installed game runs and the suite did not start it (pids %s): the author is playing" % players())
        return 2
    sweep_stale()
    before = listing(GAME)
    work = tempfile.mkdtemp(prefix="gzpg_")
    # a folder of its own in the temp folder; the game run in place on it is told apart by this path
    root = tempfile.mkdtemp(prefix="gzpr_")
    project = ""
    launcher = None
    pid_file, launcher_file = os.path.join(work, "editor.pid"), os.path.join(work, "launcher.pid")
    try:
        project = qa_target.copy_project(os.path.abspath(args.project), os.path.join(work, "project"))
        qa_target.guard_project(project)
        exe = qa_target.exe()
        line = " ".join(f'"{a}"' if " " in a else a for a in [*qa_target.ARGS, "-headless", "-nodlg", "-project", project])
        # the value goes with a space in front: pwsh -File takes a value that starts with '-' for a parameter;
        # no pipe either - the launcher's own launcher inherits it and the call would wait for it to end
        launcher = subprocess.run(
            ["pwsh", "-NoProfile", "-File", LAUNCHER, "-FilePath", exe, "-Arguments", " " + line,
             "-ProcessIdFile", pid_file, "-LauncherIdFile", launcher_file, "-DesktopName", "GrizzlyPlayQa",
             "-TimeoutSeconds", "900"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
        check(launcher.returncode == 0, "the editor is started on a hidden desktop")
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    break
            except OSError:
                time.sleep(0.1)
        qa_target.mark("editor start (hidden desktop)")
        run(project, root, args.scene, args.keep_log)
    finally:
        try:
            mcp("level_build", action="play_stop", timeout=30)
            mcp("window_control", action="close", discard=True, timeout=30)
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
            qa_target.remove_tree(root)
            qa_target.remove_tree(work)
    after = listing(GAME)
    differ = sorted(k for k in set(before) | set(after) if before.get(k) != after.get(k))
    check(not differ, "nothing of the installed game changed (%d entries listed before and after)" % len(before),
          ", ".join(differ[:10]))
    failed = [t for ok, t in results if not ok]
    print("PLAY GAME: %d pass, %d fail" % (len(results) - len(failed), len(failed)), flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
