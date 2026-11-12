#!/usr/bin/env python3
"""THE way the QA suites are run: side by side against the Rust editor, each on its own port,
editor home, project copy and mock port, with a hard timeout, and each suite's last result kept.

    python tools/qa/run_all.py                 every suite
    python tools/qa/run_all.py --failed        only the suites that failed last time
    python tools/qa/run_all.py test_pda ...    the suites named (extra arguments after --extra)
    python tools/qa/run_all.py test_pda --only B,C   only those phases of the suite named
    python tools/qa/run_all.py -j 4            four at a time (default: a quarter of the cores)
    python tools/qa/run_all.py --with-game     also the suites that start the real game (test_play_game: the game
                                               loads a level Play made; half a minute, so not in the everyday set)

Prints `PASS/FAIL <suite> <seconds> <failed checks>` per suite and a total; exit code 1 on any
failure. Each suite's output is Intermediate/qa/<suite>.log; the results Intermediate/qa/state.json.
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
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(REPO, "Intermediate", "qa")
STATE = os.path.join(OUT, "state.json")
OD = r"D:\XFinedProjects\DARF-Overnight-Delivery"
TEST = r"D:\XFinedProjects\Test"
RUST_EXE = os.path.join(REPO, "Intermediate", "cargo", "debug", "Grizzly.exe")

# name -> script, arguments, timeout (s), alone
SUITES: dict[str, dict] = {
    "test_project_browser": {"args": ["--no-explorer"], "timeout": 120},
    # its phase A times a start to the shown window (0.3 s): alone, like the other suites that time something
    "test_ui_shell": {"args": ["--project", OD], "timeout": 120, "alone": True},
    "test_ui_tour": {"args": ["--project", OD], "timeout": 180},
    "test_asset_icons": {"args": ["--project", OD], "timeout": 120},
    "test_unicode_names": {"args": ["--project", OD], "timeout": 120},
    "test_build_contents": {"args": ["--project", OD], "timeout": 180},
    # it times the UI's frames during a build: alone, like the other suites that time something
    "test_build_job": {"args": ["--project", OD], "timeout": 180, "alone": True},
    "test_characters": {"args": ["--project", OD], "timeout": 180},
    "test_behaviours": {"args": ["--project", OD], "timeout": 180},
    "test_graph_ui": {"args": ["--project", OD], "timeout": 180},
    "test_pda": {"args": ["--project", OD], "timeout": 180},
    "test_screens": {"args": ["--project", OD], "timeout": 180},
    "test_world": {"args": ["--project", TEST], "timeout": 180},
    "test_properties": {"args": ["--project", TEST], "timeout": 180},
    "test_scene_tools": {"args": ["--project", TEST], "timeout": 120},
    "test_previews": {"args": ["--project", OD], "timeout": 180},
    "test_mod_publish": {"args": ["--project", OD], "timeout": 240},
    "test_mod_catalog": {"args": ["--project", OD], "timeout": 300},
    "test_mcp_bridge": {"args": [], "timeout": 120},
    "test_stress": {"args": ["--project", TEST], "timeout": 120, "alone": True},
    "test_server_lifecycle": {"script": os.path.join(REPO, "tools", "mcp", "run_lifecycle.py"), "args": [],
                              "timeout": 180, "alone": True},
    "benchmark_scene_load": {"script": os.path.join(REPO, "tools", "mcp", "run_benchmark.py"), "args": [],
                             "timeout": 300, "alone": True},
    # the real game in a root of its own (three minutes at most for the game itself)
    "test_play_game": {"args": ["--project", TEST], "timeout": 300, "alone": True, "game": True},
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


# IO_REPARSE_TAG name-surrogate bit: a junction or a symbolic link, not a placeholder of a cloud folder
_NAME_SURROGATE = 0x20000000


def unlink_links(path: str) -> list[str]:
    """Every junction and symbolic link under `path`, the link itself removed and never followed: what it points at
    (the game's archives, for one) stays. Answers the links removed."""
    gone: list[str] = []
    try:
        entries = list(os.scandir(path))
    except OSError:
        return gone
    for e in entries:
        try:
            st = e.stat(follow_symlinks=False)
        except OSError:
            continue
        if e.is_symlink() or getattr(st, "st_reparse_tag", 0) & _NAME_SURROGATE:
            try:
                (os.rmdir if e.is_dir() else os.unlink)(e.path)
            except OSError:
                os.unlink(e.path)
            gone.append(e.path)
        elif e.is_dir(follow_symlinks=False):
            gone += unlink_links(e.path)
    return gone


def remove_tree(path: str) -> bool:
    """A suite's folders, gone for good: read-only entries too (a copy's git objects), with a few retries for a
    file an editor lets go of a moment late. Says so when something stayed."""
    if not path or not os.path.exists(path):
        return True

    def writable(func, p, _exc):
        try:
            os.chmod(p, 0o666)
            func(p)
        except OSError:
            pass

    # a link goes first, as a link: a delete never walks into what it points at
    unlink_links(path)
    for _ in range(20):
        shutil.rmtree(path, onerror=writable)
        if not os.path.exists(path):
            return True
        time.sleep(0.25)
    print("! %s could not be removed" % path, flush=True)
    return False


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


def run_one(name: str, extra: list[str], port_lock: threading.Lock) -> dict:
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
                "GRIZZLY_QA_TARGET": "rust", "GRIZZLY_QA_PORT": str(port), "GRIZZLY_QA_HOME": home,
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
    remove_tree(home)
    # what a suite left in its temp folder - all of it, when its timeout ended it
    left = [os.path.join(tmp, n) for n in os.listdir(tmp)] if os.path.isdir(tmp) else []
    if left and not timed_out:
        print("! %s left %d entr%s in its temp folder: removed" % (name, len(left), "y" if len(left) == 1 else "ies"),
              flush=True)
    remove_tree(tmp)
    text = "\n".join(lines)
    failed = [line[5:].split("  <- ")[0].strip() for line in lines if line.startswith("FAIL ")]
    if timed_out:
        failed.append("timed out after %d s" % spec["timeout"])
    elif code != 0 and not failed:
        tail = [l for l in text.strip().splitlines() if l.strip()][-1:] or ["no output"]
        failed.append("exit code %d: %s" % (code, tail[0][:200]))
    ok = code == 0 and not timed_out and not failed
    return {"ok": ok, "seconds": round(seconds, 1), "failed": failed, "code": code}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("suites", nargs="*", help="suites to run (default: all)")
    ap.add_argument("--failed", action="store_true", help="only the suites that failed last time")
    ap.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 4))
    ap.add_argument("--only", help="phases of the one suite named, passed to it as --only")
    ap.add_argument("--with-game", action="store_true", help="also the suites that start the real game")
    ap.add_argument("--extra", nargs=argparse.REMAINDER, default=[], help="arguments for a single suite")
    args = ap.parse_args()

    state = load_state()
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
        print("build the Rust editor first: bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge")
        return 2
    left = ours()
    if left:
        print("Rust editors of an earlier run are still running (pids %s): close them first" % left)
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

    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {pool.submit(run_one, n, extra, lock): n for n in together}
        for f in concurrent.futures.as_completed(futures):
            report(futures[f], f.result())
    for n in alone:
        report(n, run_one(n, extra, lock))
    wall = time.monotonic() - t0
    bad = [n for n, r in results.items() if not r["ok"]]
    left = ours()
    if left:
        print("! Rust editors left running after the run: pids %s" % left)
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
