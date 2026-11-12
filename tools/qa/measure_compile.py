#!/usr/bin/env python3
"""What a compile in the editor's process costs the window (phase 8c): the editor's frame work and the process's CPU
load while the level compiler runs on the middle scene, against the same window idle.

    python tools/qa/measure_compile.py [--scene l12u_control_monolith] [--exe <Grizzly.exe>]

On a THROW-AWAY COPY of the Test project with the scene copied in from rawdata\\levels: the editor (the fast build
unless --exe names another) starts on a hidden desktop (the game repository's Start-DetachedHiddenDesktopProcess.ps1,
so no window reaches the author's screen and the window still draws), opens the scene, runs Make All, then
  idle     ten seconds of the Level tab with a frame asked for every quarter second (ui_list holds for one);
  compile  level_build compile (wait=false) and the same frames asked for until the job ends (--no-frames: none;
           the window then draws what the job's progress wakes it for).
Each window reads frame_pacing (frame_work_ms mean / p95 / max, slow_frames, sampled_frames) and the process's CPU
time over its wall time (GetProcessTimes: 16.0 is every core of this machine busy). Everything goes to
Intermediate/compile/measure.log as it happens and the numbers to Intermediate/compile/measure.json.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import qa_target  # noqa: E402

REPO = qa_target.REPO
OUT = os.path.join(REPO, "Intermediate", "compile")
TEST = r"D:\XFinedProjects\Test"
LAUNCHER = os.path.join(r"D:\Games\Dead Air", "DeadAir-x64", "tools", "qa", "Start-DetachedHiddenDesktopProcess.ps1")
PORT = 28941
os.makedirs(OUT, exist_ok=True)
_LOG = open(os.path.join(OUT, "measure.log"), "a", encoding="utf-8")


def log(s: str) -> None:
    line = time.strftime("%H:%M:%S ") + s
    print(line, flush=True)
    _LOG.write(line + "\n")
    _LOG.flush()


def mcp(cmd: str, timeout: float = 120.0, **args) -> dict:
    with socket.create_connection(("127.0.0.1", PORT), timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps({"cmd": cmd, **args}) + "\n").encode())
        buf = bytearray()
        while not buf.endswith(b"\n"):
            c = s.recv(1 << 20)
            if not c:
                break
            buf += c
    return json.loads(buf.decode("utf-8", "replace") or "{}")


k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.OpenProcess.restype = wt.HANDLE
k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
k32.GetProcessTimes.argtypes = [wt.HANDLE] + [ctypes.POINTER(wt.FILETIME)] * 4


def cpu_of(handle) -> float:
    c, e, k, u = wt.FILETIME(), wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
    k32.GetProcessTimes(handle, ctypes.byref(c), ctypes.byref(e), ctypes.byref(k), ctypes.byref(u))
    f = lambda t: (t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7  # noqa: E731
    return f(k) + f(u)


def window(handle, until, frames: bool = True) -> dict:
    """Frames asked for every quarter second (with `frames`) until `until()` says stop; the frame pacing and the CPU
    load of it."""
    mcp("frame_pacing", action="stats")
    t0, c0 = time.monotonic(), cpu_of(handle)
    last = t0
    while not until():
        if frames:
            mcp("ui_list", timeout=30)
        time.sleep(0.25 if frames else 1.0)
        if time.monotonic() - last > 10:
            last = time.monotonic()
            log(f"    ... {last - t0:.0f} s, load {(cpu_of(handle) - c0) / (last - t0):.1f}")
    wall, cpu = time.monotonic() - t0, cpu_of(handle) - c0
    fp = mcp("frame_pacing", action="stats")
    return {"seconds": wall, "load": cpu / max(wall, 1e-6), "frame_work_ms": fp.get("frame_work_ms"),
            "frame_wait_ms": fp.get("frame_wait_ms"), "slow_frames": fp.get("slow_frames"),
            "sampled_frames": fp.get("sampled_frames")}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scene", default="l12u_control_monolith")
    ap.add_argument("--exe", default=os.path.join(REPO, "Intermediate", "cargo", "fast", "Grizzly.exe"))
    ap.add_argument("--no-frames", action="store_true", help="ask for no frame while the compile runs")
    ap.add_argument("--away", action="store_true", help="the Preferences tab up while the compile runs (no viewport)")
    args = ap.parse_args()
    try:
        socket.create_connection(("127.0.0.1", PORT), timeout=1).close()
        log(f"an editor already answers on {PORT}")
        return 2
    except OSError:
        pass
    work = tempfile.mkdtemp(prefix="gzmc_")
    home = os.path.join(work, "home")
    os.makedirs(home)
    pid_file, launcher_file = os.path.join(work, "editor.pid"), os.path.join(work, "launcher.pid")
    pid = 0
    try:
        project = qa_target.copy_project(TEST, os.path.join(work, "Test"))
        qa_target.guard_project(project)
        levels = os.path.join(project, "rawdata", "levels")
        shutil.copy2(os.path.join(REPO, "rawdata", "levels", args.scene + ".level"), levels)
        shutil.copytree(os.path.join(REPO, "rawdata", "levels", args.scene), os.path.join(levels, args.scene),
                        dirs_exist_ok=True)
        line = " ".join(f'"{a}"' if " " in a else a
                        for a in ["-mcp_port", str(PORT), "-home", home, "-nodlg", "-project", project])
        os.environ["GRIZZLY_HOME"] = home
        subprocess.run(["pwsh", "-NoProfile", "-File", LAUNCHER, "-FilePath", args.exe, "-Arguments", " " + line,
                        "-ProcessIdFile", pid_file, "-LauncherIdFile", launcher_file, "-DesktopName",
                        "GrizzlyMeasure", "-TimeoutSeconds", "1800"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60, check=True)
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    break
            except OSError:
                time.sleep(0.2)
        pid = int(open(pid_file).read().strip())
        handle = k32.OpenProcess(0x1000, False, pid)
        log(f"{args.scene}: editor {args.exe} pid {pid}")
        a = mcp("open_scene", scene=args.scene + ".level", timeout=300)
        mcp("ui_tab", action="open", id="level")
        if not a.get("ok"):
            log(f"! open_scene: {a}")
            return 1
        t = time.monotonic()
        a = mcp("level_build", action="make_all", timeout=1800)
        log(f"  Make All: {a.get('ok')} {time.monotonic() - t:.1f} s")
        if not a.get("ok"):
            return 1
        mcp("frame_pacing", action="stats")
        time.sleep(1)
        idle_until = time.monotonic() + 10
        idle = window(handle, lambda: time.monotonic() > idle_until)
        log(f"  idle: {json.dumps(idle)}")
        if args.away:
            mcp("ui_tab", action="open", kind="preferences")
        started = mcp("level_build", action="compile", wait=False)
        job = started.get("job")
        log(f"  compile started: {started}")

        def ended() -> bool:
            j = next((j for j in mcp("jobs").get("jobs", []) if j.get("id") == job), {})
            return j.get("state") != "running"

        busy = window(handle, ended, frames=not args.no_frames)
        j = next((j for j in mcp("jobs").get("jobs", []) if j.get("id") == job), {})
        res = j.get("result") or {}
        log(f"  compile: state {j.get('state')}, {res.get('seconds', 0):.1f} s in the compiler, threads "
            f"{res.get('threads')}; window {json.dumps(busy)}")
        with open(os.path.join(OUT, "measure.json"), "w", encoding="utf-8") as f:
            json.dump({"scene": args.scene, "exe": args.exe, "cores": os.cpu_count(), "idle": idle,
                       "compile": busy, "job": {"state": j.get("state"), "error": j.get("error"),
                                               "seconds": res.get("seconds"), "threads": res.get("threads"),
                                               "phases": [(p.get("name"), p.get("seconds")) for p in
                                                          ((res.get("parts") or [{}])[0].get("phases") or [])]}},
                      f, indent=1)
        return 0 if j.get("state") == "done" else 1
    finally:
        try:
            mcp("window_control", action="close", discard=True, timeout=30)
        except OSError:
            pass
        deadline = time.monotonic() + 60
        while pid and time.monotonic() < deadline:
            if str(pid) not in subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True,
                                              text=True).stdout:
                break
            time.sleep(0.5)
        else:
            if pid:
                log(f"! the editor did not leave: ending pid {pid}")
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True)
        qa_target.remove_tree(work)


if __name__ == "__main__":
    sys.exit(main())
