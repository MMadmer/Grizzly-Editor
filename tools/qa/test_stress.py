#!/usr/bin/env python3
"""The whole editor under load, on a THROW-AWAY COPY of a project with a scene (D:\\XFinedProjects\\Test).

For a few seconds several connections at once send read commands, scene edits, UI commands and
screenshots while builds of the module and loads of the scene run one after another. What it proves:
  A  every answer is one well-formed JSON object: ok true, or ok false with an error text
  B  nothing deadlocks: no request goes unanswered within its timeout, and a ping is answered quickly
     throughout (the worst ping is printed)
  C  the builds and the scene loads still end as they should while the rest goes on
  D  the editor closes clean within seconds of window_control close, and its log holds no panic

    python tools/qa/test_stress.py [--project D:\\XFinedProjects\\Test] [--seconds 8]
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import random
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

import qa_target

results: list[tuple[bool, str]] = []
lock = threading.Lock()


def check(ok, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def call(cmd: str, timeout: float = 20.0, **extra) -> tuple[dict | None, float, str]:
    """One request on a connection of its own: the answer (None when it is no JSON object), the
    seconds it took, and what went wrong."""
    payload = dict(cmd=cmd, **extra)
    t0 = time.monotonic()
    try:
        with socket.create_connection(qa_target.MCP, timeout=5) as s:
            s.settimeout(timeout)
            s.sendall((json.dumps(payload) + "\n").encode())
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = s.recv(1 << 20)
                if not chunk:
                    break
                buf += chunk
    except socket.timeout:
        return None, time.monotonic() - t0, "no answer within %.0f s" % timeout
    except OSError as e:
        return None, time.monotonic() - t0, "transport: %s" % e
    dt = time.monotonic() - t0
    try:
        v = json.loads(buf.decode("utf-8"))
    except ValueError:
        return None, dt, "not JSON: %r" % buf[:200]
    if not isinstance(v, dict) or not isinstance(v.get("ok"), bool):
        return None, dt, "no ok field: %r" % buf[:200]
    if v["ok"] is False and not isinstance(v.get("error"), str):
        return None, dt, "a refusal with no error text: %r" % buf[:200]
    return v, dt, ""


class Load:
    def __init__(self, seconds: float, target: str) -> None:
        self.stop_at = time.monotonic() + seconds
        self.target = target
        self.count = 0
        self.bad: list[str] = []
        self.slow: list[str] = []
        self.ping_worst = 0.0
        self.builds = [0, 0]  # started, ended ok
        self.build_errors: list[str] = []
        self.loads = [0, 0]

    def note(self, cmd: str, answer, dt: float, why: str) -> None:
        with lock:
            self.count += 1
            if why:
                self.bad.append("%s: %s" % (cmd, why))

    def running(self) -> bool:
        return time.monotonic() < self.stop_at

    # ---- the workers -------------------------------------------------------------------------
    def reader(self, seed: int) -> None:
        rnd = random.Random(seed)
        reads = [("state", {}), ("scene_info", {}), ("list_objects", {"class": "spawn", "limit": 20}),
                 ("quest_list", {}), ("mod_manifest", {}), ("progress", {}), ("frame_pacing", {}),
                 ("camera_get", {}), ("ui_list", {}), ("scene_stats", {}), ("selection", {}),
                 ("list_commands", {}), ("characters_list", {}), ("world_get", {}), ("scene_load_status", {})]
        while self.running():
            cmd, args = rnd.choice(reads)
            a, dt, why = call(cmd, **args)
            self.note(cmd, a, dt, why)

    def editor(self, seed: int) -> None:
        rnd = random.Random(seed)
        while self.running():
            x, z = rnd.uniform(-250, -200), rnd.uniform(-180, -140)
            a, dt, why = call("place_spawn", section="graph_point", x=x, y=-20.0, z=z, snap_to_ground=False)
            self.note("place_spawn", a, dt, why)
            name = ((a or {}).get("object") or {}).get("name", "graph_point")
            for cmd, args in (("camera_set", {"x": x, "y": 10.0, "z": z}),
                              ("select_objects", {"names": name}),
                              ("object_set_position", {"name": name, "x": x, "y": -19.0, "z": z}),
                              ("undo", {}), ("redo", {}), ("deselect_all", {})):
                a, dt, why = call(cmd, **args)
                self.note(cmd, a, dt, why)

    def ui(self, seed: int) -> None:
        rnd = random.Random(seed)
        while self.running():
            for cmd, args in (("ui_tab", {"action": "open", "kind": "characters"}),
                              ("ui_drawer", {"action": "open", "id": "log"}),
                              ("screenshot_editor", {}),
                              ("ui_drawer", {"action": "close"}),
                              ("ui_tab", {"action": "activate", "id": "level"}),
                              ("ui_panel", {"action": "focus", "id": rnd.choice(["details", "world_outliner", "tools"])}),
                              ("command_palette", {"action": "query", "query": rnd.choice(["log", "scene", "build"])}),
                              ("ui_audit", {})):
                a, dt, why = call(cmd, **args)
                self.note(cmd, a, dt, why)

    def builder(self) -> None:
        while self.running():
            self.builds[0] += 1
            # a build of this module takes a second or two: twenty is a hang
            a, dt, why = call("mod_export", timeout=20, target=self.target)
            self.note("mod_export", a, dt, why)
            if a and a.get("ok"):
                self.builds[1] += 1
            elif a:
                self.build_errors.append(str(a.get("error", ""))[:200])

    def loader(self) -> None:
        while self.running():
            self.loads[0] += 1
            a, dt, why = call("open_scene", timeout=20, scene="l01_escape.level", discard=True)
            self.note("open_scene", a, dt, why)
            if a and a.get("ok"):
                self.loads[1] += 1

    def pinger(self) -> None:
        while self.running():
            a, dt, why = call("ping", timeout=10)
            self.note("ping", a, dt, why)
            with lock:
                self.ping_worst = max(self.ping_worst, dt)
            time.sleep(0.02)


def run(project: str, seconds: float, out_dir: str) -> None:
    exe = qa_target.exe()
    info = subprocess.STARTUPINFO()
    info.dwFlags = subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 7  # SW_SHOWMINNOACTIVE
    proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe),
                            startupinfo=info)
    try:
        end = time.monotonic() + 60
        while time.monotonic() < end:
            a, _, _ = call("scene_load_status", timeout=5)
            if a and a.get("loaded"):
                break
            if proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        qa_target.mark("editor start")
        load = Load(seconds, out_dir.replace("\\", "/"))
        workers = [threading.Thread(target=load.reader, args=(i,)) for i in range(4)]
        workers += [threading.Thread(target=load.editor, args=(10 + i,)) for i in range(2)]
        workers += [threading.Thread(target=load.ui, args=(20,)), threading.Thread(target=load.builder),
                    threading.Thread(target=load.loader), threading.Thread(target=load.pinger)]
        for w in workers:
            w.start()
        for w in workers:
            w.join(timeout=seconds + 30)
        hung = [w.name for w in workers if w.is_alive()]
        print("NOTE %d requests in %.0f s; builds %d/%d, scene loads %d/%d, worst ping %.0f ms" % (
            load.count, seconds, load.builds[1], load.builds[0], load.loads[1], load.loads[0], load.ping_worst * 1000),
            flush=True)
        check(not load.bad, "A every answer is a well-formed JSON object (%d requests)" % load.count,
              "; ".join(load.bad[:6]))
        check(not hung and not any("no answer" in b for b in load.bad), "B nothing deadlocks: every request answered",
              ", ".join(hung))
        check(load.ping_worst < 2.0, "B a ping is answered throughout (worst %.0f ms)" % (load.ping_worst * 1000))
        check(load.builds[0] > 0 and load.builds[1] == load.builds[0],
              "C every build ended as a build (%d/%d)" % (load.builds[1], load.builds[0]),
              "; ".join(sorted(set(load.build_errors))[:4]))
        check(load.loads[0] > 0 and load.loads[1] == load.loads[0],
              "C every scene load ended loaded (%d/%d)" % (load.loads[1], load.loads[0]))
        st, _, why = call("state")
        check(st and st.get("ok"), "C the editor still answers after the load", why)
    finally:
        t0 = time.monotonic()
        call("window_control", timeout=10, action="close", discard=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            print("! the editor did not leave: ended by force (pid %d)" % proc.pid, flush=True)
            proc.kill()
            proc.wait(timeout=30)
        gone = time.monotonic() - t0
        check(gone < 5.0 and proc.returncode == 0, "D the editor closes clean (%.2f s, exit code %s)" % (gone, proc.returncode))
        logs = glob.glob(qa_target.log_glob())
        text = "".join(open(p, encoding="utf-8", errors="replace").read() for p in logs)
        panics = [l for l in text.splitlines() if "panic" in l.lower()]
        check(logs and not panics, "D its log holds no panic", "; ".join(panics[:3]) or "no log")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", default=r"D:\XFinedProjects\Test")
    ap.add_argument("--seconds", type=float, default=8.0)
    args = ap.parse_args()
    if call("ping", timeout=1)[0]:
        print(qa_target.busy_text())
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_stress_")
    try:
        copy = qa_target.copy_project(os.path.abspath(args.project), os.path.join(work, "QaStress"))
        run(copy, args.seconds, os.path.join(work, "out"))
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    ok = sum(1 for good, _ in results if good)
    print(f"STRESS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
