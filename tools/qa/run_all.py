#!/usr/bin/env python3
"""THE way the QA suites are run: side by side against the editor, each on its own port, editor home,
project copy and mock port, with a hard timeout, and each suite's last result kept.

    python tools/qa/run_all.py                 every suite
    python tools/qa/run_all.py --failed        only the suites that failed last time
    python tools/qa/run_all.py test_pda ...    the suites named (extra arguments after --extra)
    python tools/qa/run_all.py test_pda --only B,C   only those phases of the suite named
    python tools/qa/run_all.py -j 4            four at a time (default: a quarter of the cores)
    python tools/qa/run_all.py --with-game     also the suites that start the real game (test_play_game: the game
                                               loads a level Play made; half a minute, so not in the everyday set)

Prints `PASS/FAIL <suite> <seconds> <failed checks>` per suite and a total, and says every minute which suites
are still running; exit code 1 on any failure. Each suite's output is Intermediate/qa/<suite>.log; the results Intermediate/qa/state.json.
A suite marked `alone` (it times frames or loads) runs after the others, by itself; one marked `game` starts the
real game and runs only with --with-game (or when named).
Every editor a suite starts gets the suite's home on its command line and in GRIZZLY_HOME; the run fails when the
repository root's _projects.ini or grizzly_editor.ltx changed or appeared meanwhile (and puts it back).
"""
from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qa import remove_tree  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(REPO, "Intermediate", "qa")
STATE = os.path.join(OUT, "state.json")
OD = r"D:\XFinedProjects\DARF-Overnight-Delivery"
TEST = r"D:\XFinedProjects\Test"
# the editor the suites drive: the debug build, or the binary GRIZZLY_QA_EXE names (qa.exe())
RUST_EXE = os.environ.get("GRIZZLY_QA_EXE") or os.path.join(REPO, "Intermediate", "cargo", "debug", "Grizzly.exe")

# name -> arguments, timeout (s: every suite is held to two minutes), alone, game
SUITES: dict[str, dict] = {
    "test_project_browser": {"args": ["--no-explorer"], "timeout": 120},
    # a project moved and opened in a home that never saw it; a second start hands over (two homes, its own ports)
    "test_portable": {"args": ["--project", TEST], "timeout": 120},
    # its phase A times a start to the shown window (0.3 s): alone, like the other suites that time something
    "test_ui_shell": {"args": ["--project", OD], "timeout": 120, "alone": True},
    # every tab kind, dialog and menu at two sizes: audited and pictured once
    "test_tour": {"args": ["--project", OD], "timeout": 120},
    # the same tour in a light theme: the kit and every surface hold the light half of the themes
    "test_tour_light": {"script": os.path.join(HERE, "test_tour.py"),
                        "args": ["--project", OD, "--theme", "grizzly-light"], "timeout": 120},
    "test_asset_icons": {"args": ["--project", OD], "timeout": 120},
    "test_unicode_names": {"args": ["--project", OD], "timeout": 120},
    "test_build_contents": {"args": ["--project", OD], "timeout": 120},
    # it times the UI's frames during a build: alone, like the other suites that time something
    "test_build_job": {"args": ["--project", OD], "timeout": 120, "alone": True},
    "test_characters": {"args": ["--project", OD], "timeout": 120},
    # the Items tab over MCP: the kind, the form from the catalog, a save as the difference, the icon's bake
    "test_items": {"args": ["--project", TEST], "timeout": 120},
    "test_behaviours": {"args": ["--project", OD], "timeout": 120},
    "test_graph_ui": {"args": ["--project", OD], "timeout": 120},
    "test_assets": {"args": ["--project", OD], "timeout": 120},
    "test_pda": {"args": ["--project", OD], "timeout": 120},
    "test_screens": {"args": ["--project", OD], "timeout": 120},
    "test_text": {"args": [], "timeout": 120},
    "test_world": {"args": ["--project", TEST], "timeout": 120},
    "test_properties": {"args": ["--project", TEST], "timeout": 120},
    "test_scene_tools": {"args": ["--project", TEST], "timeout": 120},
    "test_level_commands": {"args": ["--project", TEST], "timeout": 120},
    # an agent's level design on Escape: the scene's answers, its files, the audit, models, pictures, a terrain and
    # the piece that carries its cut (about a minute; the build of the piece is a quarter of it)
    "test_level_design": {"args": ["--project", TEST], "timeout": 300},
    # the game as the editor's library and a scene over a level of the game: seconds when the library is current,
    # the first reading of the game when it never was (minutes)
    "test_game_library": {"args": ["--project", TEST], "timeout": 900},
    # the compiler's regression net: two small scenes at draft quality from a cold start, against tools/qa/golden
    "test_level_compile": {"args": ["--project", TEST], "timeout": 120},
    "test_previews": {"args": ["--project", OD], "timeout": 120},
    "test_mod_publish": {"args": ["--project", OD], "timeout": 120},
    "test_mod_catalog": {"args": ["--project", OD], "timeout": 120},
    # the editor updates itself: a fake installation of harmless files against the mock (headless), then the window's face of it
    "test_update": {"args": ["--project", TEST], "timeout": 120},
    # the Go mock of GitHub and VirusTotal that the updater's, the publisher's and the catalog's suites stand on
    "test_github_mock": {"args": [], "timeout": 120},
    # the bridge, the six doors and the socket under load
    "test_bridge": {"args": ["--project", TEST], "timeout": 120},
    # the commandlets and the editor's process lifecycle (a console of its own for Ctrl+Break)
    "test_cli": {"args": ["--project", TEST], "timeout": 120},
    # the real game in a root of its own (three minutes at most for the game itself)
    "test_play_game": {"args": ["--project", TEST], "timeout": 300, "alone": True, "game": True},
    # a module's piece of a level the game has - an object of the module's own compiled and lit - built, shipped by
    # Play and attached by the game to the sector it finds
    "test_play_game_piece": {"script": os.path.join(HERE, "test_play_game.py"),
                             "args": ["--project", TEST, "--scene", "l01_escape_piece"], "timeout": 300, "alone": True,
                             "game": True},
    # a yard cut into Escape's slope by a terrain, a blockout on a texture asset: the game cuts its ground, moves the
    # AI map's nodes and reads the texture's description
    "test_play_game_terrain": {"script": os.path.join(HERE, "test_play_game.py"),
                               "args": ["--project", TEST, "--scene", "l01_escape_terrain"], "timeout": 300,
                               "alone": True, "game": True},
    # a level of the project's own in streaming cells, played: the game loads the cells beside the level
    "test_play_game_cells": {"script": os.path.join(HERE, "test_play_game.py"),
                             "args": ["--project", TEST, "--scene", "mp_close_combat_cells"], "timeout": 300, "alone": True,
                             "game": True},
}

# What an editor without a home of its own would write into the repository root.
ROOT_FILES = [os.path.join(REPO, n) for n in ("_projects.ini", "grizzly_editor.ltx")]


def read_opt(path: str) -> bytes | None:
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def root_files_changed(before: dict[str, bytes | None]) -> list[str]:
    """The repository root's files an editor changed or made during the run, each put back as it was."""
    changed = []
    for path, was in before.items():
        now = read_opt(path)
        if now == was:
            continue
        changed.append("%s %s" % (os.path.basename(path), "appeared" if was is None else "changed"))
        if was is None:
            os.remove(path)
        else:
            with open(path, "wb") as f:
                f.write(was)
    return changed


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def load_state() -> dict:
    try:
        with open(STATE, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_state(state: dict) -> None:
    os.makedirs(OUT, exist_ok=True)
    tmp = STATE + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f, indent=1)
    os.replace(tmp, STATE)


def ours() -> list[int]:
    """Rust editors of this repository still running."""
    try:
        out = subprocess.run(["powershell", "-NoProfile", "-Command",
                              "Get-Process Grizzly -ErrorAction SilentlyContinue | "
                              "ForEach-Object { \"$($_.Id)|$($_.Path)\" }"],
                             capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    pids = []
    for line in out.splitlines():
        pid, _, path = line.partition("|")
        if path.strip().lower() == RUST_EXE.lower() and pid.strip().isdigit():
            pids.append(int(pid))
    return pids


# the suites running now, with the second each started at: the pulse says every minute how far a run is
RUNNING: dict[str, float] = {}
PULSE_SECONDS = 60.0


def pulse(done: threading.Event, total: int, finished: list) -> None:
    while not done.wait(PULSE_SECONDS):
        now = time.monotonic()
        names = ", ".join("%s %ds" % (n, now - t) for n, t in sorted(RUNNING.items(), key=lambda kv: kv[1]))
        print("... %d of %d suites ended; running: %s" % (len(finished), total, names or "none"), flush=True)


def run_one(name: str, extra: list[str], port_lock: threading.Lock) -> dict:
    RUNNING[name] = time.monotonic()
    try:
        return run_suite(name, extra, port_lock)
    finally:
        RUNNING.pop(name, None)


def run_suite(name: str, extra: list[str], port_lock: threading.Lock) -> dict:
    spec = SUITES[name]
    script = spec.get("script") or os.path.join(HERE, name + ".py")
    with port_lock:
        port = free_port()
    home = tempfile.mkdtemp(prefix="grizzly_qa_home_")
    # the suite's temp folder is its own: whatever it makes there (its copies, its homes) goes with it, also when
    # the suite is ended by its timeout and cleans up nothing itself
    # short: a suite that starts the game puts its root in here, and the game takes paths of 255 characters at most
    tmp = tempfile.mkdtemp(prefix="gzqa_")
    env = dict(os.environ)
    env.update({"TEMP": tmp, "TMP": tmp,
                "GRIZZLY_QA_PORT": str(port), "GRIZZLY_QA_HOME": home,
                # an editor started without -home takes this one
                "GRIZZLY_HOME": home,
                # the QA credentials of the Credential Manager are this suite's own: two suites side by
                # side never take away each other's token or VirusTotal key
                "GRIZZLY_QA_CREDENTIAL": "%s-%d" % (name.replace("_", "-"), port),
                "PYTHONIOENCODING": "utf-8", "PYTHONUNBUFFERED": "1"})
    log_path = os.path.join(OUT, name + ".log")
    t0 = time.monotonic()
    timed_out = False
    lines: list[str] = []
    with open(log_path, "w", encoding="utf-8", errors="replace") as log:
        proc = subprocess.Popen([sys.executable, script, *spec["args"], *extra], cwd=REPO, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

        def pump() -> None:
            # every line with the second it came at: where a suite's time goes is in its log
            assert proc.stdout is not None
            for raw in proc.stdout:
                line = raw.decode("utf-8", "replace").rstrip("\r\n")
                lines.append(line)
                log.write("[%6.1f] %s\n" % (time.monotonic() - t0, line))
                log.flush()

        reader = threading.Thread(target=pump, daemon=True)
        reader.start()
        try:
            code = proc.wait(timeout=spec["timeout"])
        except subprocess.TimeoutExpired:
            timed_out = True
            # the whole tree: the suite's Python, its editor, its mock - none of them may outlive the run
            tree = subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True, text=True)
            print("! %s timed out after %d s: its process tree was ended (%s)"
                  % (name, spec["timeout"], (tree.stdout or tree.stderr).strip().replace(chr(10), "; ")[:300]), flush=True)
            try:
                code = proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                proc.kill()
                code = proc.wait(timeout=30)
        reader.join(timeout=30)
    seconds = time.monotonic() - t0
    trouble = log_trouble(home)
    # GRIZZLY_QA_KEEP_LOGS=<folder>: the editors' logs of this suite are copied there before its home goes
    keep = os.environ.get("GRIZZLY_QA_KEEP_LOGS")
    if keep:
        os.makedirs(keep, exist_ok=True)
        for root, _dirs, files in os.walk(home):
            for f in files:
                if f.endswith(".log"):
                    shutil.copyfile(os.path.join(root, f), os.path.join(keep, "%s-%d-%s" % (name, int(time.time()), f)))
    remove_tree(home)
    # what a suite left in its temp folder - all of it, when its timeout ended it
    left = [os.path.join(tmp, n) for n in os.listdir(tmp)] if os.path.isdir(tmp) else []
    if left and not timed_out:
        print("! %s left %d entr%s in its temp folder: removed" % (name, len(left), "y" if len(left) == 1 else "ies"),
              flush=True)
    remove_tree(tmp)
    text = "\n".join(lines)
    failed = [line[5:].split("  <- ")[0].strip() for line in lines if line.startswith(("FAIL ", "SLOW "))]
    slow = [line[5:].split("  <- ")[0].strip() for line in lines if line.startswith("SLOW ")]
    # what no suite asks about: an editor of the suite that the graphics device complained to, or that panicked
    failed.extend(trouble)
    if timed_out:
        failed.append("timed out after %d s" % spec["timeout"])
    elif code != 0 and not failed:
        tail = [l for l in text.strip().splitlines() if l.strip()][-1:] or ["no output"]
        failed.append("exit code %d: %s" % (code, tail[0][:200]))
    ok = code == 0 and not timed_out and not failed
    return {"ok": ok, "seconds": round(seconds, 1), "failed": failed, "slow": slow, "code": code}


TROUBLE = ("the graphics device reported an error", "panicked at")

def only_timings(result: dict) -> bool:
    """The suite failed, and every check that failed in it timed something (`qa.check(..., timing=True)` prints
    those as SLOW)."""
    failed, slow = result.get("failed") or [], set(result.get("slow") or [])
    return bool(failed) and not result.get("ok") and all(f in slow for f in failed)


def log_trouble(home: str) -> list[str]:
    """The lines of the editors' logs under a suite's home that mean trouble whatever the suite checked: an error
    of the graphics device (a frame drawn into a surface of the wrong size, a refused configuration) and a panic.
    One line per kind and log, cut short."""
    found: list[str] = []
    for root, _dirs, files in os.walk(home):
        for name in files:
            if not name.endswith(".log"):
                continue
            try:
                with open(os.path.join(root, name), encoding="utf-8", errors="replace") as f:
                    text = f.read()
            except OSError:
                continue
            for mark in TROUBLE:
                lines = [l.strip() for l in text.splitlines() if mark in l]
                if lines:
                    found.append("%s: %d line(s) like '%s'" % (name, len(lines), lines[0][:160]))
    return found


def settle(limit: float = 20.0) -> float:
    """Waits until the machine is quiet - under a tenth of its cores busy for half a second - before a suite that
    times something. The suites before it leave work behind (folders being removed, a virus scanner reading the
    files a Play root held): `test_build_job` right after `test_play_game` once met a 95 ms frame that way."""
    import ctypes
    from ctypes import wintypes

    def times() -> tuple[int, int]:
        idle, kernel, user = wintypes.FILETIME(), wintypes.FILETIME(), wintypes.FILETIME()
        ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user))

        def ticks(ft: wintypes.FILETIME) -> int:
            return (ft.dwHighDateTime << 32) | ft.dwLowDateTime
        # the kernel time holds the idle time
        return ticks(idle), ticks(kernel) + ticks(user)

    t0 = time.monotonic()
    before = times()
    while time.monotonic() - t0 < limit:
        time.sleep(0.5)
        now = times()
        total = now[1] - before[1]
        if total <= 0 or 1.0 - (now[0] - before[0]) / total < 0.10:
            break
        before = now
    return time.monotonic() - t0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("suites", nargs="*", help="suites to run (default: all)")
    ap.add_argument("--failed", action="store_true", help="only the suites that failed last time")
    ap.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 4))
    ap.add_argument("--only", help="phases of the one suite named, passed to it as --only")
    ap.add_argument("--with-game", action="store_true", help="also the suites that start the real game")
    ap.add_argument("--extra", nargs=argparse.REMAINDER, default=[], help="arguments for a single suite")
    args = ap.parse_args()

    # a suite that is gone takes its last result with it
    state = {k: v for k, v in load_state().items() if k in SUITES}
    names = [n for n in SUITES if args.with_game or not SUITES[n].get("game")]
    if args.suites:
        unknown = [n for n in args.suites if n not in SUITES]
        if unknown:
            print("no such suite: " + ", ".join(unknown))
            return 2
        names = args.suites
    elif args.failed:
        names = [n for n in names if not state.get(n, {}).get("ok")]
    if not names:
        print("nothing to run: every suite passed last time")
        return 0
    if not os.path.isfile(RUST_EXE):
        print("build the editor first: bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge")
        return 2
    left = ours()
    end = time.monotonic() + 30
    while left and time.monotonic() < end:
        time.sleep(1.0)
        left = ours()
    if left:
        print("Editors of an earlier run are still running (pids %s): close them first" % left)
        return 2
    os.makedirs(OUT, exist_ok=True)
    if args.only and len(names) != 1:
        print("--only names phases of ONE suite: name it")
        return 2
    extra = (["--only", args.only] if args.only else []) + (args.extra if len(names) == 1 else [])
    together = [n for n in names if not SUITES[n].get("alone")]
    alone = [n for n in names if SUITES[n].get("alone")]
    lock = threading.Lock()
    root_before = {p: read_opt(p) for p in ROOT_FILES}
    t0 = time.monotonic()
    results: dict[str, dict] = {}

    def report(n: str, r: dict) -> None:
        results[n] = r
        state[n] = dict(r, when=time.strftime("%Y-%m-%d %H:%M:%S"), **({"only": args.only} if args.only else {}))
        save_state(state)
        print("%s %-24s %6.1f s %s" % ("PASS" if r["ok"] else "FAIL", n, r["seconds"],
                                     "; ".join(r["failed"][:6]) + (" ..." if len(r["failed"]) > 6 else "")),
              flush=True)

    quiet = threading.Event()
    threading.Thread(target=pulse, args=(quiet, len(names), results), daemon=True).start()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {pool.submit(run_one, n, extra, lock): n for n in together}
        for f in concurrent.futures.as_completed(futures):
            report(futures[f], f.result())
    for n in alone:
        settle()
        report(n, run_one(n, extra, lock))
    # a time measured beside other suites says how busy the machine was: a suite that failed on nothing but its
    # timings is measured again with the machine to itself, and that run is its result
    for n in [n for n in together if only_timings(results[n])]:
        settle()
        print("again, alone: %s (beside the others: %s)" % (n, "; ".join(results[n]["failed"][:3])), flush=True)
        report(n, run_one(n, extra, lock))
    quiet.set()
    wall = time.monotonic() - t0
    bad = [n for n, r in results.items() if not r["ok"]]
    # an editor that was told to close may still be closing: it gets a minute
    left = ours()
    end = time.monotonic() + 60
    while left and time.monotonic() < end:
        time.sleep(1.0)
        left = ours()
    if left:
        print("! Editors left running after the run: pids %s" % left)
        bad.append("editors left running")
    touched = root_files_changed(root_before)
    if touched:
        print("! an editor wrote into the repository root: %s (put back)" % ", ".join(touched))
        bad.append("repository root written")
    print("TOTAL %d suites, %d passed, %d failed, wall %.1f s" % (len(results), len(results) - len(
        [n for n in results if not results[n]["ok"]]), len([n for n in results if not results[n]["ok"]]), wall))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
