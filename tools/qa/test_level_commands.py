#!/usr/bin/env python3
"""The Level tab's toolbar commands by their own names, over MCP on a THROW-AWAY COPY of a project with the
repository's smallest scene (rawdata\\levels\\mp_test.level). The same builds run through `level_build action=`
in test_scene_tools (C, Y); here every name the toolbar and the Build menu post is driven once:

  A  level_rebuild_status answers what the scene's edits left for the level builders (collision, ai_map,
     game_graph, text); reload_configs reads the linked game's configs again
  B  build_cform, build_aimap (on a scene without AI nodes: refused in the C++ words) and build_all (the build and
     the compile) each end as a job and answer; after them the collision is not left to rebuild
  C  compile_details on a level folder that does not hold what the detail compiler reads is refused, naming the file
  D  play_game with root= a folder that holds a file of somebody else's is refused before anything is built or
     written, the folder unchanged; no game is started
  E  the scene's file rows of the C++ table over exec_command: COMMAND_LOAD without a scene and Save As open no
     window for an MCP caller and name p1s; COMMAND_SAVE p1s=<file> saves there

    python tools/qa/test_level_commands.py --project <project>
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
results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def short(a: dict) -> str:
    return json.dumps(a)[:400]


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
                if mcp("mod_manifest", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        # never killed: a killed editor leaves project.ltx behind half written
        try:
            mcp("app.quit", discard=True)
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def run(project: str, work: str) -> None:
    levels = os.path.join(project, "rawdata", "levels")
    os.makedirs(levels, exist_ok=True)
    shutil.copy2(os.path.join(qa_target.REPO, "rawdata", "levels", SCENE + ".level"), levels)
    shutil.copytree(os.path.join(qa_target.REPO, "rawdata", "levels", SCENE), os.path.join(levels, SCENE),
                    dirs_exist_ok=True)
    ed = Editor()
    try:
        ed.start(project)
        opened = mcp("open_scene", scene=SCENE + ".level", discard=True, timeout=120)
        if not check(opened.get("ok"), "the small scene opens", short(opened)):
            return

        # ------------------------------------------------------------ A: the status, the configs
        st = mcp("level_rebuild_status")
        check(st.get("ok") and all(isinstance(st.get(k), bool) for k in ("collision", "ai_map", "game_graph"))
              and isinstance(st.get("text"), str), "A level_rebuild_status names the three builds", short(st))
        rc = mcp("reload_configs", timeout=120)
        check(rc.get("ok") and rc.get("reloaded") is True, "A reload_configs reads the game's configs again", short(rc))

        # ------------------------------------------------------------ B: the toolbar's builds
        for name in ("build_cform", "build_aimap", "build_all"):
            t = time.monotonic()
            a = mcp(name, timeout=300)
            # a scene without AI nodes is refused in the C++ words: the job ran and said why
            empty_map = name == "build_aimap" and "AIMap is empty" in a.get("error", "")
            check(a.get("ok") or empty_map, "B %s ends and answers (%.1f s)" % (name, time.monotonic() - t), short(a))
        after = mcp("level_rebuild_status")
        check(after.get("ok") and after.get("collision") is False,
              "B after them the collision is not left to rebuild", short(after))

        # ------------------------------------------------------------ C: the detail compiler's input
        level_dir = os.path.join(project, "gamedata", "levels", SCENE)
        missing = next((n for n in ("build.prj", "build.details", "build.cform")
                        if not os.path.isfile(os.path.join(level_dir, n))), None)
        cd = mcp("compile_details", timeout=120)
        if missing:
            check(not cd.get("ok") and missing in cd.get("error", ""),
                  "C compile_details is refused, naming %s" % missing, short(cd))
        else:
            check(cd.get("ok") and bool(cd.get("report")), "C compile_details compiles the details", short(cd))

        # ------------------------------------------------------------ D: Play takes no folder it did not make
        root = os.path.join(work, "somebody_elses")
        os.makedirs(root)
        with open(os.path.join(root, "keep.txt"), "w", encoding="utf-8") as f:
            f.write("not Play's\n")
        before = listing(root)
        pg = mcp("play_game", root=root, timeout=120)
        check(not pg.get("ok") and listing(root) == before,
              "D play_game refuses root= a folder that holds a file, and leaves it as it was", short(pg))
        jobs = mcp("job.list")
        game = [j for j in (jobs.get("jobs") or []) if j.get("state") == "running"
                and (j.get("kind") == "level_run" or j.get("group") == "game")]
        check(jobs.get("ok") and not game, "D no game job runs after the refusal: no game was started", short(jobs))

        # ------------------------------------------------------------ E: the scene's file rows of the C++ table
        # COMMAND_LOAD without a scene and COMMAND_SAVE's Save As open the Open Scene / Save Scene As dialogs for a
        # person; an MCP caller gets no window, and the refusal names the argument to pass instead
        for row, extra in (("COMMAND_LOAD", {}), ('COMMAND_SAVE."Save As"', {})):
            r = mcp("exec_command", name=row, **extra)
            shot = mcp("shot.window", timeout=120)
            check(r.get("ok") and r.get("result") == 0 and "p1s" in (r.get("refused") or "")
                  and shot.get("modal") is None,
                  "E %s over MCP opens no window and names p1s" % row, short(r) + " / modal " + short(shot.get("modal")))
        saved = os.path.join(project, "rawdata", "levels", "qa_saved_as.level")
        r = mcp("exec_command", name="COMMAND_SAVE", p1s=saved)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline and not os.path.isfile(saved):
            time.sleep(0.25)
        check(r.get("ok") and r.get("result") == 1 and os.path.isfile(saved),
              "E COMMAND_SAVE p1s=<file> saves the scene there", short(r))
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()
    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="gzqa_levelcmds_")
    try:
        copy = qa_target.copy_project(src, os.path.join(work, "project"), trimmed=True)
        print("running against a copy: " + copy, flush=True)
        run(copy, work)
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
