#!/usr/bin/env python3
"""What a compile in the editor's process costs, against the same compile headless (phase 8c, then "After phase 8c:
the window while a job runs"): the compile's time, the editor's UI frames and scene passes, its frame work and the
process's CPU load while the level compiler runs on the middle scene.

    python tools/qa/measure_compile.py [--scene l12u_control_monolith] [--exe <Grizzly.exe>] [--pairs 3] [--orbit]

On a THROW-AWAY COPY of the Test project with the scene copied in from rawdata\\levels: the editor (the fast build
unless --exe names another) starts on a hidden desktop (the game repository's Start-DetachedHiddenDesktopProcess.ps1,
so no window reaches the author's screen and the window still draws), opens the scene, runs Make All, then
  idle      ten seconds of the Level tab with a frame asked for every quarter second (ui_list holds for one);
  pairs     --pairs times (--fresh: a new editor for each): level.compile of the level folder (wait=false) with the same frames asked for until the job ends
            (--no-frames: none; the window then draws what the job's progress wakes it for), then the same level
            folder compiled headless right after (`Grizzly-Cmd.exe run level.compile folder=<folder>`, a process of its own, the editor
            idle beside it);
  card      with --card, one more compile in the editor with the Task Center's card up (its spinner animates);
  orbit     with --orbit, one more compile in the editor while the camera orbits (viewport.gesture: an Alt+left
            drag, one step a frame) until it ends.
--cool N waits N seconds of nothing before every compile, editor and headless alike: on a laptop the first
seconds of full load run on the turbo's power budget, and a compile that follows another starts without it (the
processors' speed of each run is recorded, `clock_percent`). Each editor window reads perf.frames (UI frames and scene passes counted apart, the passes by reason, frame_work_ms
mean / p95 / max, slow_frames) and the process's CPU time over its wall time (GetProcessTimes: 16.0 is every core of
this machine busy); the headless run its own process's. Everything goes to Intermediate/compile/measure.log as it
happens and the numbers to Intermediate/compile/measure.json.
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
k32.CloseHandle.argtypes = [wt.HANDLE]
k32.SetProcessInformation.argtypes = [wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.DWORD]


def cpu_of(handle) -> float:
    c, e, k, u = wt.FILETIME(), wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
    k32.GetProcessTimes(handle, ctypes.byref(c), ctypes.byref(e), ctypes.byref(k), ctypes.byref(u))
    f = lambda t: (t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7  # noqa: E731
    return f(k) + f(u)


class Clock:
    r"""The processors' speed while something runs: `\Processor Information(_Total)\% Processor Performance` (100 is
    the base clock; a laptop's turbo goes well past it and falls back once its power budget is spent), sampled every
    half second on a thread of its own through PDH by its English name (the counter names are localized)."""

    def __init__(self) -> None:
        import threading
        self.pdh = ctypes.WinDLL("pdh")
        self.query, self.counter = wt.HANDLE(), wt.HANDLE()
        self.samples: list[float] = []
        self.ok = (self.pdh.PdhOpenQueryW(None, None, ctypes.byref(self.query)) == 0 and
                   self.pdh.PdhAddEnglishCounterW(self.query, r"\Processor Information(_Total)\% Processor Performance",
                                                  None, ctypes.byref(self.counter)) == 0)
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        if self.ok:
            self.pdh.PdhCollectQueryData(self.query)
            self.thread.start()

    def run(self) -> None:
        class Value(ctypes.Structure):
            _fields_ = [("status", wt.DWORD), ("pad", wt.DWORD), ("value", ctypes.c_double)]
        while not self.stop.wait(0.5):
            v = Value()
            if self.pdh.PdhCollectQueryData(self.query) == 0 and \
                    self.pdh.PdhGetFormattedCounterValue(self.counter, 0x200, None, ctypes.byref(v)) == 0:
                self.samples.append(v.value)

    def done(self) -> float | None:
        self.stop.set()
        if self.ok:
            self.thread.join(timeout=2)
            self.pdh.PdhCloseQuery(self.query)
        return round(sum(self.samples) / len(self.samples), 1) if self.samples else None


PACING_KEYS = ("frame_work_ms", "frame_wait_ms", "slow_frames", "sampled_frames", "ui_frames", "ui_frame_causes",
               "scene_passes", "scene_pass_reasons", "last_scene_pass")


def window(handle, until, frames: bool = True, step=None) -> dict:
    """Frames asked for every quarter second (with `frames`) or `step()` called in a loop until `until()` says stop;
    the frame pacing and the CPU load of it."""
    mcp("perf.frames")
    clock = Clock()
    t0, c0 = time.monotonic(), cpu_of(handle)
    last = t0
    while not until():
        if step:
            step()
        else:
            if frames:
                mcp("ui.list", timeout=30)
            time.sleep(0.25 if frames else 1.0)
        if time.monotonic() - last > 10:
            last = time.monotonic()
            log(f"    ... {last - t0:.0f} s, load {(cpu_of(handle) - c0) / (last - t0):.1f}")
    wall, cpu = time.monotonic() - t0, cpu_of(handle) - c0
    fp = mcp("perf.frames")
    out = {"seconds": round(wall, 3), "load": round(cpu / max(wall, 1e-6), 3), "clock_percent": clock.done()}
    out.update({k: fp.get(k) for k in PACING_KEYS})
    return out


def headless(exe: str, folder: str, home: str) -> dict:
    """The level folder compiled by the commandlet (`level.compile`), a process of its own: the compiler's seconds,
    the wall time and the process's load."""
    clock = Clock()
    t0 = time.monotonic()
    cmd = os.path.join(os.path.dirname(exe), "Grizzly-Cmd.exe")
    p = subprocess.Popen([cmd, "run", "level.compile", "folder=" + folder, "-home", home, "-json"], stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, cwd=os.path.dirname(exe),
                         creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    handle = k32.OpenProcess(0x1000 | 0x0010, False, p.pid)
    cpu = 0.0
    out = []
    for raw in p.stdout:
        out.append(raw.decode("utf-8", "replace").rstrip())
        cpu = max(cpu, cpu_of(handle))
    p.wait()
    cpu = max(cpu, cpu_of(handle))
    memory = memory_of(handle)
    k32.CloseHandle(handle)
    wall = time.monotonic() - t0
    clock_percent = clock.done()
    answer = {}
    for line in reversed(out):
        try:
            answer = json.loads(line)
            break
        except ValueError:
            continue
    parts = (answer.get("parts") or [{}])
    phases = [[ph.get("name"), round(float(ph.get("seconds", 0)), 2)] for ph in parts[0].get("phases", [])]
    return {"code": p.returncode, "compiler_seconds": answer.get("seconds"), "seconds": round(wall, 3),
            "load": round(cpu / max(wall, 1e-6), 3), "memory": memory, "clock_percent": clock_percent, "phases": phases,
            "last": out[-1] if out else ""}


class PMC(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("PageFaultCount", wt.DWORD)] + [(n, ctypes.c_size_t) for n in (
        "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
        "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]


psapi = ctypes.WinDLL("psapi", use_last_error=True)
psapi.GetProcessMemoryInfo.argtypes = [wt.HANDLE, ctypes.POINTER(PMC), wt.DWORD]


def memory_of(handle) -> dict:
    """The process's private bytes and working set now, and their peaks, in MB."""
    m = PMC()
    m.cb = ctypes.sizeof(PMC)
    psapi.GetProcessMemoryInfo(handle, ctypes.byref(m), m.cb)
    mb = lambda v: round(v / 1048576.0)  # noqa: E731
    return {"private_mb": mb(m.PrivateUsage), "peak_private_mb": mb(m.PeakPagefileUsage),
            "working_set_mb": mb(m.WorkingSetSize), "peak_working_set_mb": mb(m.PeakWorkingSetSize)}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scene", default="l12u_control_monolith")
    ap.add_argument("--exe", default=os.path.join(REPO, "Intermediate", "cargo", "fast", "Grizzly.exe"))
    ap.add_argument("--pairs", type=int, default=1, help="editor compile + headless compile, this many times")
    ap.add_argument("--cool", type=float, default=0.0,
                    help="seconds of nothing before every compile, so each starts with the same power budget")
    ap.add_argument("--headless-editor", action="store_true",
                    help="the editor without a window (-headless): its compile with nothing drawn at all")
    ap.add_argument("--no-throttle", action="store_true",
                    help="take the editor out of Windows' power throttling (EcoQoS) before it compiles")
    ap.add_argument("--fresh", action="store_true", help="a new editor for every pair (else one editor for all)")
    ap.add_argument("--orbit", action="store_true", help="one more compile in the editor with the camera orbiting")
    ap.add_argument("--no-headless", action="store_true", help="the editor's compiles alone")
    ap.add_argument("--card", action="store_true", help="one more compile in the editor with the Task Center's card up")
    ap.add_argument("--no-frames", action="store_true", help="ask for no frame while the compile runs")
    ap.add_argument("--away", action="store_true", help="the Preferences tab up while the compile runs (no viewport)")
    ap.add_argument("--idle-only", action="store_true", help="the idle window and nothing else")
    args = ap.parse_args()
    exe = os.path.abspath(args.exe)
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
    ed = {"pid": 0, "handle": None}
    result: dict = {"scene": args.scene, "exe": exe, "cores": os.cpu_count(), "pairs": [], "started": time.ctime(),
                    "fresh": args.fresh}

    def close_editor() -> None:
        pid = ed["pid"]
        if not pid:
            return
        try:
            mcp("app.quit", discard=True, timeout=30)
        except OSError:
            pass
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if str(pid) not in subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True,
                                              text=True).stdout:
                break
            time.sleep(0.5)
        else:
            log(f"! the editor did not leave: ending pid {pid}")
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True)
        if ed["handle"]:
            k32.CloseHandle(ed["handle"])
        ed["pid"], ed["handle"] = 0, None
        # the port is free again before the next editor listens on it
        time.sleep(1)

    def start_editor(project: str) -> bool:
        for f in (pid_file, launcher_file):
            if os.path.exists(f):
                os.remove(f)
        line = " ".join(f'"{a}"' if " " in a else a
                        for a in ["-mcp_port", str(PORT), "-home", home, "-nodlg", "-project", project])
        os.environ["GRIZZLY_HOME"] = home
        if args.headless_editor:
            p = subprocess.Popen([exe, "-headless", "-mcp_port", str(PORT), "-home", home, "-nodlg", "-project",
                                  project], cwd=os.path.dirname(exe), stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
            with open(pid_file, "w") as f:
                f.write(str(p.pid))
        else:
            subprocess.run(["pwsh", "-NoProfile", "-File", LAUNCHER, "-FilePath", exe, "-Arguments", " " + line,
                            "-ProcessIdFile", pid_file, "-LauncherIdFile", launcher_file, "-DesktopName",
                            "GrizzlyMeasure", "-TimeoutSeconds", "3600"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60, check=True)
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            try:
                if mcp("mod.info", timeout=30).get("ok"):
                    break
            except OSError:
                time.sleep(0.2)
        ed["pid"] = int(open(pid_file).read().strip())
        ed["handle"] = k32.OpenProcess(0x1000 | 0x0010 | 0x0200, False, ed["pid"])
        if args.no_throttle:
            # PROCESS_POWER_THROTTLING_STATE {Version 1, ControlMask EXECUTION_SPEED, StateMask 0}: never EcoQoS
            state = (wt.ULONG * 3)(1, 1, 0)
            ok = k32.SetProcessInformation(ed["handle"], 4, ctypes.byref(state), ctypes.sizeof(state))
            log(f"  the editor out of power throttling: {bool(ok)}")
        log(f"{args.scene}: editor {exe} pid {ed['pid']}")
        a = mcp("scene.open", path="rawdata/levels/" + args.scene + ".level", timeout=300)
        mcp("ui.tab", action="open", doc="level")
        if not a.get("ok"):
            log(f"! scene.open: {a}")
            return False
        t = time.monotonic()
        a = mcp("level.build", only="geometry", force=True, timeout=1800)
        log(f"  The geometry step: {a.get('ok')} {time.monotonic() - t:.1f} s")
        if not a.get("ok"):
            return False
        # the pictures of the scene loaded before anything is measured (a window's)
        if not args.headless_editor:
            mcp("shot.viewport", timeout=60)
        if args.away:
            mcp("ui.tab", action="open", kind="preferences")
        return True

    def settle(label: str) -> dict:
        """Waits until the editor works on nothing of its own (under a fifth of a core over a second, a minute at
        most): what it did before a compile - loading what a start or the last compile's files asked for - is not
        the compile's."""
        if args.cool > 0:
            time.sleep(args.cool)
        t0, waited = time.monotonic(), []
        while time.monotonic() - t0 < 60:
            c0 = cpu_of(ed["handle"])
            time.sleep(1.0)
            load = cpu_of(ed["handle"]) - c0
            waited.append(round(load, 2))
            if load < 0.2:
                break
        out = {"seconds": round(time.monotonic() - t0, 1), "loads": waited}
        log(f"  {label}: settled {json.dumps(out)}")
        return out

    def compile_in_editor(label: str, step=None, card: bool = False) -> tuple[dict, dict]:
        quiet = settle(label)
        level = mcp("level.status").get("level", args.scene)
        started = mcp("level.compile", folder=os.path.join(project, "gamedata", "levels", level), wait=False)
        job = started.get("job")
        log(f"  {label}: compile started: {started}")
        if card:
            log(f"  {label}: the Task Center's card: {mcp('ui.popup', show='tasks')}")

        def ended() -> bool:
            j = next((j for j in mcp("job.list").get("jobs", []) if j.get("id") == job), {})
            return j.get("state") != "running"

        busy = window(ed["handle"], ended, frames=not args.no_frames, step=step)
        j = next((j for j in mcp("job.list").get("jobs", []) if j.get("id") == job), {})
        res = j.get("result") or {}
        busy["compiler_seconds"] = res.get("seconds")
        busy["state"] = j.get("state")
        busy["threads"] = res.get("threads")
        busy["memory"] = memory_of(ed["handle"])
        busy["settled"] = quiet
        busy["phases"] = [[ph.get("name"), round(ph.get("seconds") or 0, 2)]
                          for ph in ((res.get("parts") or [{}])[0].get("phases") or [])]
        log(f"  {label}: state {j.get('state')}, {res.get('seconds', 0):.1f} s in the compiler, threads "
            f"{res.get('threads')}; window {json.dumps(busy)}")
        return busy, res

    try:
        project = qa_target.copy_project(TEST, os.path.join(work, "Test"))
        qa_target.guard_project(project)
        levels = os.path.join(project, "rawdata", "levels")
        shutil.copy2(os.path.join(REPO, "rawdata", "levels", args.scene + ".level"), levels)
        shutil.copytree(os.path.join(REPO, "rawdata", "levels", args.scene), os.path.join(levels, args.scene),
                        dirs_exist_ok=True)
        if not start_editor(project):
            return 1
        mcp("perf.frames")
        time.sleep(1)
        idle_until = time.monotonic() + 10
        result["idle"] = idle = window(ed["handle"], lambda: time.monotonic() > idle_until)
        log(f"  idle: {json.dumps(idle)}")
        if args.idle_only:
            return 0

        for n in range(args.pairs):
            if args.fresh and n > 0:
                close_editor()
                if not start_editor(project):
                    return 1
            editor, res = compile_in_editor(f"pair {n + 1} editor")
            if editor.get("state") != "done":
                result["pairs"].append({"editor": editor})
                return 1
            if args.no_headless:
                result["pairs"].append({"editor": editor})
                continue
            folder = res.get("dir", "").replace("/", "\\")
            quiet = settle(f"pair {n + 1} headless")
            head = headless(exe, folder, os.path.join(work, "home_headless"))
            head["settled"] = quiet
            log(f"  pair {n + 1} headless: {json.dumps(head)}")
            ratio = (editor["compiler_seconds"] or 0) / max(head["compiler_seconds"] or 1e-6, 1e-6)
            result["pairs"].append({"editor": editor, "headless": head, "ratio": round(ratio, 3)})
            log(f"  pair {n + 1}: editor / headless = {ratio:.3f}")

        if args.card:
            result["card"], _ = compile_in_editor("card", card=True)

        if args.orbit:
            view = mcp("viewport.gesture", kind="hover", at=[200, 200], timeout=60).get("viewport") or [800, 500]
            w, h = view[0], view[1]
            cx, cy = w / 2, h / 2
            side = [1]

            def orbit() -> None:
                side[0] = -side[0]
                mcp("viewport.gesture", kind="drag", at=[cx - side[0] * w * 0.3, cy], to=[cx + side[0] * w * 0.3, cy],
                    steps=60, mods="alt", timeout=60)

            result["orbit"], _ = compile_in_editor("orbit", step=orbit)
        return 0
    finally:
        result["ended"] = time.ctime()
        with open(os.path.join(OUT, "measure.json"), "w", encoding="utf-8") as f:
            json.dump(result, f, indent=1)
        close_editor()
        qa_target.remove_tree(work)


if __name__ == "__main__":
    sys.exit(main())
