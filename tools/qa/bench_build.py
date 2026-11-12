#!/usr/bin/env python3
"""A module build of the reference mod with test_build_job's gigabyte of payload, timed on the Rust editor and on the
C++ one, on copies in the system temp folder (export_target and the build target inside it):

  * the Rust editor, its window shown (not activated) at 2160 x 1350 with View > Real Time on and a scene open:
    frame_pacing at rest and during the clean build (the frame's own work apart from the wait for the
    presentation), the editor's CPU use over the build (process CPU time / wall time / logical cores),
    the clean build's and a rebuild's seconds (nothing changed);
  * the C++ editor (Bin\\x64\\Release\\LevelEditor.exe, minimised, on 28016 - not run when something already
    answers there): the clean build's and the rebuild's seconds. Its _projects.ini, level.ini and level_imgui.ini
    come back byte for byte.

    python tools/qa/bench_build.py [--project D:\\XFinedProjects\\DARF-Overnight-Delivery] [--scene l03_agroprom.level]
        [--no-cpp]
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
CPP_EXE = os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")
CPP_KEPT = ["_projects.ini", "level.ini", "level_imgui.ini"]
PAYLOAD_FILES = 24
PAYLOAD_MB = 48


def mcp(port: int, cmd: str, timeout: float = 600.0, **extra) -> dict:
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(("127.0.0.1", port), timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def answers(port: int) -> bool:
    try:
        socket.create_connection(("127.0.0.1", port), timeout=1).close()
        return True
    except OSError:
        return False


def cpu_seconds(pid: int) -> float:
    """The process's kernel + user time."""
    k32 = ctypes.windll.kernel32
    h = k32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not h:
        return 0.0
    try:
        ft = [ctypes.wintypes.FILETIME() for _ in range(4)]
        if not k32.GetProcessTimes(h, *[ctypes.byref(f) for f in ft]):
            return 0.0
        tick = lambda f: (f.dwHighDateTime << 32 | f.dwLowDateTime) / 1e7  # noqa: E731
        return tick(ft[2]) + tick(ft[3])
    finally:
        k32.CloseHandle(h)


def start(cmd: list[str], port: int, show: int, env=None) -> subprocess.Popen:
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = show
    proc = subprocess.Popen(cmd, cwd=os.path.dirname(cmd[0]), startupinfo=startup, env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        try:
            if mcp(port, "mod_manifest", timeout=30).get("ok"):
                return proc
        except (OSError, ValueError):
            pass
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup: " + cmd[0])
        time.sleep(0.1)
    raise RuntimeError("the editor never opened the project: " + cmd[0])


def close(port: int, proc: subprocess.Popen) -> None:
    try:
        mcp(port, "window_control", action="close", discard=True, timeout=60)
    except (OSError, ValueError):
        pass
    try:
        proc.wait(timeout=120)
    except subprocess.TimeoutExpired:
        print("! pid %d did not leave: ended by force" % proc.pid, flush=True)
        proc.kill()
        proc.wait(timeout=30)


def build(port: int, target: str) -> tuple[float, dict]:
    t = time.monotonic()
    r = mcp(port, "mod_export", target=target.replace("\\", "/"), timeout=900)
    return time.monotonic() - t, r


def prepare(source: str, dst: str, payload: str) -> str:
    qa_target.copy_project(source, dst)
    shutil.copytree(payload, os.path.join(dst, "gamedata", "textures", "qa_job"))
    return dst


def rust(project: str, work: str, scene: str) -> dict:
    port = 28941
    home = os.path.join(work, "rust_home")
    os.makedirs(home, exist_ok=True)
    qa_target.guard_project(project)
    proc = start([qa_target.exe() if qa_target.RUST else os.path.join(REPO, "Intermediate", "cargo", "debug",
                                                                       "Grizzly.exe"),
                  "-mcp_port", str(port), "-home", home, "-nodlg", "-project", project], port, 4)
    out: dict = {}
    try:
        mcp(port, "window_control", action="resize", width=2160, height=1350)
        if scene:
            out["scene"] = mcp(port, "open_scene", scene=scene, timeout=600).get("ok")
            mcp(port, "ui_tab", action="open", id="level")
        mcp(port, "view_mode", real_time=True)
        mcp(port, "frame_pacing")
        time.sleep(float(os.environ.get("BENCH_IDLE", "3")))
        idle = mcp(port, "frame_pacing")
        out["idle"] = {k: idle.get(k) for k in ("sampled_frames", "frame_work_ms", "frame_wait_ms", "slow_frames",
                                                 "measured_frame_ms")}
        target = os.path.join(work, "rust_game")
        cpu0, t0 = cpu_seconds(proc.pid), time.monotonic()
        result: dict = {}
        job = threading.Thread(target=lambda: result.update(zip(("seconds", "answer"), build(port, target))))
        job.start()
        samples = []
        stages: list[tuple[float, str]] = []
        last_pacing = time.monotonic()
        while job.is_alive():
            time.sleep(0.02)
            rows = mcp(port, "progress").get("background") or []
            row = next((r for r in rows if r.get("key")), {})
            label = "%s: %s" % (row.get("step"), row.get("detail") or row.get("status") or row.get("stage") or "")
            if row and not stages:
                print("ROW", json.dumps(row), flush=True)
            if row and (not stages or stages[-1][1] != label):
                stages.append((time.monotonic() - t0, label))
            if time.monotonic() - last_pacing > 0.5:
                samples.append(mcp(port, "frame_pacing"))
                last_pacing = time.monotonic()
        job.join()
        samples.append(mcp(port, "frame_pacing"))
        out["stages"] = [(round(t, 2), l) for t, l in stages]
        wall, cpu = time.monotonic() - t0, cpu_seconds(proc.pid) - cpu0
        cores = os.cpu_count() or 1
        works = [s.get("frame_work_ms") or {} for s in samples if s.get("sampled_frames")]
        waits = [s.get("frame_wait_ms") or {} for s in samples if s.get("sampled_frames")]
        frames = sum(s.get("sampled_frames", 0) for s in samples)
        out["build"] = {
            "clean_s": round(result.get("seconds", 0), 2),
            "ok": (result.get("answer") or {}).get("ok"),
            "files": (result.get("answer") or {}).get("files"),
            "cpu_s": round(cpu, 1), "wall_s": round(wall, 2), "cores": cores,
            "cpu_use": round(cpu / wall / cores, 3) if wall else 0,
            "frames": frames,
            "work_mean_ms": round(sum(w.get("mean", 0) * s.get("sampled_frames", 0) for w, s in
                                      zip(works, [s for s in samples if s.get("sampled_frames")])) / max(frames, 1), 2),
            "work_p95_ms": max((w.get("p95", 0) for w in works), default=0),
            "work_max_ms": max((w.get("max", 0) for w in works), default=0),
            "wait_max_ms": max((w.get("max", 0) for w in waits), default=0),
            "slow_frames": sum(s.get("slow_frames", 0) for s in samples),
        }
        again, r = build(port, target)
        out["rebuild_s"] = round(again, 2)
        out["rebuild_ok"] = r.get("ok")
    finally:
        close(port, proc)
        logs = os.path.join(home, "logs")
        if os.environ.get("BENCH_KEEP_LOG") and os.path.isdir(logs):
            for n in os.listdir(logs):
                shutil.copy(os.path.join(logs, n), os.environ["BENCH_KEEP_LOG"])
    return out


def cpp(project: str, work: str) -> dict:
    if answers(28016):
        return {"skipped": "something answers on 28016: the author's editor, not touched"}
    kept = {n: (open(os.path.join(REPO, n), "rb").read() if os.path.isfile(os.path.join(REPO, n)) else None)
            for n in CPP_KEPT}
    out: dict = {}
    proc = None
    try:
        for n in CPP_KEPT:
            if os.path.exists(os.path.join(REPO, n)):
                os.remove(os.path.join(REPO, n))
        qa_target.guard_project(project)
        proc = start([CPP_EXE, "-nodlg", "-project", project], 28016, 7)
        target = os.path.join(work, "cpp_game")
        clean, r = build(28016, target)
        out["clean_s"], out["ok"], out["files"] = round(clean, 2), r.get("ok"), r.get("files")
        again, r = build(28016, target)
        out["rebuild_s"], out["rebuild_ok"] = round(again, 2), r.get("ok")
        if not r.get("ok"):
            out["rebuild_error"] = str(r.get("error"))[:300]
    finally:
        if proc:
            close(28016, proc)
        for n, data in kept.items():
            p = os.path.join(REPO, n)
            if data is None:
                if os.path.exists(p):
                    os.remove(p)
            else:
                with open(p, "wb") as f:
                    f.write(data)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", default=r"D:\XFinedProjects\DARF-Overnight-Delivery")
    ap.add_argument("--scene", default="l03_agroprom.level")
    ap.add_argument("--no-cpp", action="store_true")
    ap.add_argument("--no-rust", action="store_true")
    args = ap.parse_args()
    work = tempfile.mkdtemp(prefix="grizzly_bench_")
    try:
        payload = os.path.join(work, "payload")
        os.makedirs(payload)
        block = os.urandom(PAYLOAD_MB << 20)
        for i in range(PAYLOAD_FILES):
            with open(os.path.join(payload, f"p_{i:02}.dds"), "wb") as f:
                f.write(i.to_bytes(4, "little") + block)
        result = {}
        if not args.no_rust:
            result["rust"] = rust(prepare(args.project, os.path.join(work, "rust"), payload), work, args.scene)
            print("RUST", json.dumps(result["rust"]), flush=True)
        if not args.no_cpp:
            result["cpp"] = cpp(prepare(args.project, os.path.join(work, "cpp"), payload), work)
            print("CPP", json.dumps(result["cpp"]), flush=True)
    finally:
        qa_target.remove_tree(work)
    return 0


if __name__ == "__main__":
    sys.exit(main())
