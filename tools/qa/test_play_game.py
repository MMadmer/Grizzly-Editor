#!/usr/bin/env python3
"""The game loads a level the Rust editor built and started with Play (plan D1, phase 6's gate): on a THROW-AWAY
COPY of a project, in a game root of the editor's own, on a hidden desktop, within three minutes.

    python tools/qa/test_play_game.py --project D:\\XFinedProjects\\Test [--keep-log <file>]

What it does:
  1. copies the project into the system temp folder and puts the repository's smallest scene into it (mp_test:
     one arena, one light); its multiplayer spawns give way to an actor and a graph point, an AI map is generated
     over the arena, and the scene is saved and built (Make All);
  2. compiles the geometry with the editor's own level compiler, in its process (level_build compile_geometry: a
     job on every core, into the project's level folder; the command answers when it ends, with its report);
  3. starts Play: the editor builds the collision, the AI map, the game graph (joined onto the linked game's,
     whose vertex numbers the game's start scripts name) and editor.spawn, makes a game root in a short temp folder
     of its own (root=; the game takes paths of 255 characters at most) - the linked game's executables copied, a
     link to its archives, its loose gamedata with the level staged into it, an appdata of its own - and runs the
     game there with -start server(editor/single/alife/new);
  4. reads the game's log until the level is loaded (LOADED, below), with no assert and no crash dump;
  5. stops the game (Play's stop; one left running is ended and fails the run), closes the editor over MCP,
     removes the copy and the root (the archives' link first, as a link).

The level is in when the game's log holds every line of LOADED: the new game made of editor.spawn, the player made
of its actor, the client connected, the level's collision model built under the level's name, its geometry and
collision read, and the load's synchronisation ended (the actor's first update has run by then: the game's start
scripts spawn its kit there). Then five seconds more, and no assert, no fatal error, no crash dump.
Opt-in in run_all.py (--with-game): it starts the real game, half a minute of the run.

Why the game root holds copies of the executables, not the installed engine run in place with -fsltx: the game's
content service works in the folder of xrCore.dll whatever the fs config says (Core.ApplicationPath,
ContentService.cpp): every start's verification rewrites <game>\.dead-air-x64\content-state.txt, and a level
starts only when $arch_dir$ is that folder's database. The suite proves it on the root (the file is written there)
and lists the installed game before and after the run (path, size, write time): any difference fails.

The editor runs on a hidden desktop (the game repository's Start-DetachedHiddenDesktopProcess.ps1), so the game it
starts is there too and no window reaches the user's screen. Nothing of the installed game is written: its
executables are copied, not linked, its archives are read through a junction, its modules, saves and settings are
never opened for writing. The game's own QA roots are built the same way (Run-NqQa.ps1, New-QaRoot).
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
SCENE = "mp_test"
GAME_LIMIT = 180.0
MCP = qa_target.MCP
MP_SPAWNS = ("tdm_blue_0001,tdm_green_0001,ah_cta_blue_0001,ah_cta_green_0001,ah_artefact_0001,cta_artefakt_blue,"
             "cta_artefakt_green,spectator_0001,dm_0000,ah_cta_green_base_box,ah_cta_blue_base_box,tdm_weapons_0001,"
             "tdm_weapons_0000,cta_wpn_toz34,cta_medkit,cta_ammo_12x76_zhekan,cta_ammo_12x76_zhekan_0000,"
             "cta_ammo_12x76_zhekan_0001")
# the settings the game's own QA forces (Run-NqQa.ps1, Update-QaUserLtx): a window, no sound, no pause
USER_LTX = {"renderer": "renderer_r4", "vid_mode": "1024x768", "vid_window_mode": "st_opt_windowed",
            "rs_fullscreen": "off", "rs_v_sync": "off", "rs_fps_limit": "60", "g_pause_in_background": "0",
            "snd_volume_music": "0", "snd_volume_eff": "0"}
# what the game's log says once the level is in: the new game made of editor.spawn, the level's own files read
LOADED = {
    "the new game is created from editor.spawn": "New game is successfully created!",
    "the player is made of the spawn's actor": "# New player created.",
    "the client is connected": "client : connection accepted",
    "the level's collision model is built for the level": "cdb_cache\\" + SCENE + "\\objspace.bin",
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


def games_in(root: str) -> list[int]:
    """The processes running an executable of the game root (the game Play started there)."""
    try:
        out = subprocess.run(["powershell", "-NoProfile", "-Command",
                              "Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path } | "
                              "ForEach-Object { \"$($_.Id)|$($_.Path)\" }"],
                             capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    prefix = os.path.normcase(os.path.realpath(root)).rstrip("\\") + os.sep
    return [int(pid) for pid, _, path in (l.partition("|") for l in out.splitlines())
            if pid.strip().isdigit() and os.path.normcase(os.path.realpath(path.strip())).startswith(prefix)]


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
    """Every file and folder under `top` with its size and write time; a link is listed, never followed."""
    out: dict[str, tuple[int, int]] = {}
    for d, dirs, files in os.walk(top):
        keep = []
        for n in dirs:
            p = os.path.join(d, n)
            st = os.lstat(p)
            out[os.path.relpath(p, top)] = (0, st.st_mtime_ns)
            if not (os.path.islink(p) or getattr(st, "st_reparse_tag", 0)):
                keep.append(n)
        dirs[:] = keep
        for n in files:
            p = os.path.join(d, n)
            try:
                st = os.lstat(p)
            except OSError:
                continue
            out[os.path.relpath(p, top)] = (st.st_size, st.st_mtime_ns)
    return out


def run(project: str, root: str, keep_log: str | None) -> None:
    for p in (os.path.join(project, "rawdata", "levels", SCENE + ".level"),):
        shutil.copy2(os.path.join(REPO, "rawdata", "levels", SCENE + ".level"), p)
    shutil.copytree(os.path.join(REPO, "rawdata", "levels", SCENE), os.path.join(project, "rawdata", "levels", SCENE))
    user_ltx(root)

    opened = mcp("open_scene", scene=SCENE + ".level", discard=True)
    if not check(opened.get("ok"), "the scene opens", short(opened)):
        return
    for cmd, args in (
        ("select_objects", {"names": MP_SPAWNS}), ("delete_selected", {}),
        ("place_spawn", {"section": "actor", "x": 2.89, "y": 1.0, "z": 2.363}),
        ("place_spawn", {"section": "graph_point", "x": 1.0, "y": 1.0, "z": 1.0}),
        ("select_objects", {"names": "scenes\\mp_test\\mp_test_arena"}), ("aimap_tool", {"show": True}),
        ("snap_objects", {"action": "set_from_selection"}),
        ("aimap_tool", {"action": "add_nodes", "position": [2.89, 1.0, 2.363]}),
        ("aimap_tool", {"action": "generate", "timeout": 120}),
        # the quickest light XrLC makes: one arena does not need the scene's own sun and sky at quality 3
        ("property_set", {"target": "world", "key": "Scene\\Build options\\Quality", "value": "Custom"}),
        ("property_set", {"target": "world", "key": "Scene\\Build options\\Lighting\\Hemisphere quality [0-3]",
                          "value": "0"}),
        ("property_set", {"target": "world", "key": "Scene\\Build options\\Lighting\\Sun shadow quality [0-3]",
                          "value": "0"}),
        ("property_set", {"target": "world", "key": "Scene\\Build options\\Lighting\\Pixel per meter",
                          "value": "1"}),
        ("save_scene", {}),
    ):
        a = mcp(cmd, **args)
        if not check(a.get("ok"), f"{cmd} answers ok", short(a)):
            return
    built = mcp("level_build", action="make_all")
    if not check(built.get("ok"), "Make All builds the level", short(built)):
        return
    t = time.monotonic()
    done = mcp("level_build", action="compile_geometry", timeout=280)
    report = done.get("report") or {}
    check(done.get("ok") and report.get("parts"), "the editor's level compiler compiled the level in its process "
          "(%.1f s, %s threads)" % (time.monotonic() - t, report.get("threads")),
          short({k: v for k, v in done.items() if k != "report"}))
    wait_idle(30)
    level_dir = os.path.join(project, "gamedata", "levels", SCENE)
    compiled = all(os.path.isfile(os.path.join(level_dir, f)) for f in ("level", "level.geom", "level.cform"))
    check(compiled, "the level's files are in the project's level folder",
          ", ".join(sorted(os.listdir(level_dir))))
    if not compiled:
        return

    t0, play_wall = time.monotonic(), time.time()
    play = mcp("level_build", action="play", root=root, params="-i -always_active -silent_error_mode -force_flushlog")
    if not check(play.get("ok") and play.get("run", {}).get("started"), "Play starts the game in its own root",
                 short(play)):
        return
    print("play: " + short(play), flush=True)
    staged = os.path.join(root, "gamedata", "levels", SCENE)
    check(all(os.path.isfile(os.path.join(staged, f)) for f in ("level", "level.ai", "level.geom")) and
          os.path.isfile(os.path.join(root, "gamedata", "spawns", "editor.spawn")),
          "the level, its level.ai and editor.spawn are staged in the game root")
    text, loaded_at = "", None
    while time.monotonic() - t0 < GAME_LIMIT:
        text = engine_log(root)
        if all(m in text for m in LOADED.values()) or any(f in text for f in FATAL) or dumps(root):
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
    for what, marker in LOADED.items():
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
    state = os.path.join(root, ".dead-air-x64", "content-state.txt")
    check(os.path.isfile(state) and os.path.getmtime(state) >= play_wall - 1,
          "the game's content service rewrote content-state.txt beside the root's engine (why the engine is not run "
          "in the installed game's folder)")
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
    sweep_stale()
    before = listing(GAME)
    work = tempfile.mkdtemp(prefix="gzpg_")
    # a folder of its own, right in the temp folder: every path the game indexes under its root, its own shader
    # cache's among them, has to fit 255 characters (play.rs, GAME_PATH_LIMIT) - a deeper root stops the game at
    # its start, and Play refuses it
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
        run(project, root, args.keep_log)
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
