"""The level compiler of the editor (gz-lc) against the C++ compilers of the cutover.

    python tools/qa/diff_compile.py                 the scenes that failed or changed since the last run
    python tools/qa/diff_compile.py --all           every scene
    python tools/qa/diff_compile.py --scene mp_test one scene
    python tools/qa/diff_compile.py --refresh-oracle   record the C++ side again (and the input)

The oracle is recorded once per scene, keyed by the scene's files, the C++ compilers and the editor's data, under
Intermediate/oracle/compile-<scene>-<key>/:
  input/        the build files the Rust editor's Make All wrote (build.prj and the rest) - the ONE input both
                compilers read
  cpp1/, cpp2/  what XrLC.exe and xrDO_Light.exe wrote in two runs (the second is the C++ run-to-run floor)
  record.json   every output file with size and SHA-256, the C++ log with the time each line appeared, the
                compilers' own phase list, the process's CPU time sampled through the run
A later run starts no C++ compiler when the record is there.

The C++ compilers run from a stage under the system temp folder with an fsgame.ltx of their own (the editor's, its
levels the stage's), `-silent`, under a hard limit of ten minutes, ended by their process id past it.

The Rust compile runs `Grizzly.exe -headless -compile <level folder>` (Intermediate/cargo/fast unless
GRIZZLY_LC_EXE names another), then `-compile_details`. The comparison is in compare_compile.py; this file runs the
compilers and records. Everything a run prints goes to Intermediate/compile/run.log as it happens.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import qa_target  # noqa: E402

REPO = qa_target.REPO
INTER = os.path.join(REPO, "Intermediate")
ORACLE = os.path.join(INTER, "oracle")
OUT = os.path.join(INTER, "compile")
CPP_BIN = os.path.join(REPO, "Bin", "x64", "CppEditor")
RUST_EXE = os.environ.get("GRIZZLY_LC_EXE") or os.path.join(INTER, "cargo", "fast", "Grizzly.exe")
EDITOR_EXE = os.environ.get("GRIZZLY_QA_EXE") or RUST_EXE
TEST_PROJECT = r"D:\XFinedProjects\Test"
SCENES = ["mp_test", "l12u_control_monolith", "mp_close_combat"]
# Scenes proven for the detail compiler alone: XrLC does not finish them within the limit (mp_close_combat's
# implicit light passes ten minutes), so both detail compilers read the same build.prj, build.details and the
# Rust compiler's build.cform and build.lights (equal to the C++ ones under the rules of the other scenes).
DETAILS_ONLY = {"mp_close_combat"}
FORMAT = 2
LIMIT = 600.0
PORT = 28931

os.makedirs(OUT, exist_ok=True)
_LOG = open(os.path.join(OUT, "run.log"), "a", encoding="utf-8")


def log(s: str) -> None:
    line = time.strftime("%H:%M:%S ") + s
    print(line, flush=True)
    _LOG.write(line + "\n")
    _LOG.flush()


def sha256_file(p: str) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def stamp(p: str) -> str:
    try:
        st = os.stat(p)
        return f"{st.st_size}:{st.st_mtime_ns}"
    except OSError:
        return "missing"


def scene_files(scene: str) -> list[tuple[str, str]]:
    root = os.path.join(REPO, "rawdata", "levels")
    out = [(scene + ".level", os.path.join(root, scene + ".level"))]
    base = os.path.join(root, scene)
    for d, dirs, files in os.walk(base):
        dirs.sort()
        for n in sorted(files):
            p = os.path.join(d, n)
            out.append((scene + "/" + os.path.relpath(p, base).replace("\\", "/"), p))
    return out


def key_of(scene: str) -> str:
    parts = {"format": FORMAT, "scene": [f"{r}|{stamp(p)}" for r, p in scene_files(scene)]}
    parts["cpp"] = [f"{n}|{stamp(os.path.join(CPP_BIN, n))}" for n in sorted(os.listdir(CPP_BIN))
                    if n.lower().endswith((".exe", ".dll"))]
    parts["data"] = [stamp(os.path.join(REPO, "gamedata", "shaders_xrlc.xr")), stamp(os.path.join(REPO, "fsgame.ltx"))]
    return hashlib.sha256(json.dumps(parts, sort_keys=True).encode()).hexdigest()[:16]


def record_dir(scene: str, key: str) -> str:
    return os.path.join(ORACLE, f"compile-{scene}-{key}")


# ------------------------------------------------------------------------------------------- processes

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.OpenProcess.restype = wt.HANDLE
k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
k32.GetProcessTimes.argtypes = [wt.HANDLE] + [ctypes.POINTER(wt.FILETIME)] * 4
k32.CloseHandle.argtypes = [wt.HANDLE]


def cpu_seconds(handle) -> float | None:
    c, e, k, u = wt.FILETIME(), wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
    if not k32.GetProcessTimes(handle, ctypes.byref(c), ctypes.byref(e), ctypes.byref(k), ctypes.byref(u)):
        return None
    f = lambda t: (t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7  # noqa: E731
    return f(k) + f(u)


class Watch:
    """A process followed while it runs: its CPU time every tenth of a second and, for a C++ compiler, the lines of
    its log file (written line by line under -flushlog) with the second each was first seen. The compilers' own log
    window is no help here: its thread answers one message per 200 ms, so a reader lags and loses the end."""

    def __init__(self, proc: subprocess.Popen, t0: float, log_dir: str | None) -> None:
        self.proc, self.t0, self.log_dir = proc, t0, log_dir
        self.samples: list[tuple[float, float]] = []
        self.lines: list[tuple[float, str]] = []
        self.phases: list[str] = []
        self.before = self.logs()
        self.stop = threading.Event()
        self.handle = k32.OpenProcess(0x1000, False, proc.pid)
        self.th = threading.Thread(target=self.run, daemon=True)
        self.th.start()

    def logs(self) -> dict[str, float]:
        if not self.log_dir or not os.path.isdir(self.log_dir):
            return {}
        out = {}
        for n in os.listdir(self.log_dir):
            if n.lower().endswith(".log"):
                try:
                    out[n] = os.path.getmtime(os.path.join(self.log_dir, n))
                except OSError:
                    pass
        return out

    def poll_log(self, now: float) -> None:
        now_logs = self.logs()
        mine = [n for n, m in now_logs.items() if self.before.get(n) != m]
        if not mine:
            return
        try:
            with open(os.path.join(self.log_dir, mine[0]), "rb") as f:
                text = f.read().decode("cp1251", "replace")
        except OSError:
            return
        rows = text.splitlines()
        # the last row may still be written
        if rows and not text.endswith(("\r", "\n")):
            rows.pop()
        for s in rows[len(self.lines):]:
            self.lines.append((now, s))
            if s.startswith("* New phase started"):
                log(f"    [{now:8.1f}] {s}")

    def run(self) -> None:
        last_note = 0.0
        while not self.stop.is_set():
            now = time.monotonic() - self.t0
            try:
                if self.handle:
                    c = cpu_seconds(self.handle)
                    if c is not None:
                        self.samples.append((now, c))
                if self.log_dir:
                    self.poll_log(now)
            except Exception as e:  # noqa: BLE001 - a watcher never ends the run it watches
                log(f"    ! watcher: {e!r}")
            if now - last_note > 30:
                last_note = now
                cpu = self.samples[-1][1] if self.samples else 0.0
                log(f"    ... {now:.0f} s, cpu {cpu:.0f} s")
            time.sleep(0.1)

    def finish(self) -> None:
        self.stop.set()
        self.th.join(timeout=5)
        if self.log_dir:
            self.poll_log(time.monotonic() - self.t0)
        if self.handle:
            k32.CloseHandle(self.handle)


def run_limited(cmd: list[str], cwd: str, log_dir: str | None, stdout_lines: list | None = None) -> tuple[int | None, Watch, float]:
    t0 = time.monotonic()
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
    proc = subprocess.Popen(cmd, cwd=cwd, startupinfo=startup,
                            stdout=subprocess.PIPE if stdout_lines is not None else subprocess.DEVNULL,
                            stderr=subprocess.STDOUT if stdout_lines is not None else subprocess.DEVNULL,
                            text=stdout_lines is not None, encoding="utf-8" if stdout_lines is not None else None,
                            errors="replace")
    w = Watch(proc, t0, log_dir)
    reader = None
    if stdout_lines is not None:
        def read():
            for line in proc.stdout:
                t = time.monotonic() - t0
                line = line.rstrip("\n")
                stdout_lines.append((t, line))
                if "New phase started" in line or line.startswith("[") and ("Build" in line):
                    log("    " + line)
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
    code = None
    try:
        code = proc.wait(timeout=LIMIT)
    except subprocess.TimeoutExpired:
        log(f"! {os.path.basename(cmd[0])} passed its {LIMIT:.0f} s: ended by its process id {proc.pid}")
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
        proc.wait(timeout=30)
    if reader:
        reader.join(timeout=10)
    wall = time.monotonic() - t0
    w.finish()
    return code, w, wall


# ------------------------------------------------------------------------------------------- the input

def mcp(cmd: str, timeout: float = 120.0, **args) -> dict:
    payload = {"cmd": cmd}
    payload.update(args)
    with socket.create_connection(("127.0.0.1", PORT), timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = bytearray()
        while not buf.endswith(b"\n"):
            c = s.recv(1 << 20)
            if not c:
                break
            buf += c
    return json.loads(buf.decode("utf-8", "replace") or "{}")


def make_input(scene: str, dst: str) -> None:
    """The Rust editor's Make All of the scene in a copy of the Test project: its level folder is the input."""
    work = tempfile.mkdtemp(prefix="gzlc_in_")
    try:
        project = qa_target.copy_project(TEST_PROJECT, os.path.join(work, "Test"))
        os.makedirs(os.path.join(project, "temp"), exist_ok=True)
        for rel, p in scene_files(scene):
            to = os.path.join(project, "rawdata", "levels", *rel.split("/"))
            os.makedirs(os.path.dirname(to), exist_ok=True)
            shutil.copy2(p, to)
        home = os.path.join(work, "home")
        os.makedirs(home, exist_ok=True)
        env = dict(os.environ, GRIZZLY_HOME=home)
        qa_target.guard_project(project)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7
        proc = subprocess.Popen([EDITOR_EXE, "-mcp_port", str(PORT), "-home", home, "-nodlg", "-headless",
                                 "-project", project], cwd=os.path.dirname(EDITOR_EXE), env=env, startupinfo=startup,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 120
            while True:
                if proc.poll() is not None:
                    raise RuntimeError("the editor exited during its start")
                try:
                    if mcp("mod_manifest", timeout=30).get("ok"):
                        break
                except (OSError, ValueError):
                    pass
                if time.monotonic() > deadline:
                    raise RuntimeError("the editor never opened the project")
                time.sleep(0.1)
            a = mcp("open_scene", timeout=300, scene=scene + ".level")
            if not a.get("ok"):
                raise RuntimeError(f"open_scene: {a}")
            a = mcp("level_build", timeout=1800, action="make_all")
            if not a.get("ok"):
                raise RuntimeError(f"make_all: {a}")
            level = os.path.join(project, "gamedata", "levels", scene)
            os.makedirs(dst, exist_ok=True)
            for n in sorted(os.listdir(level)):
                if os.path.isfile(os.path.join(level, n)):
                    shutil.copy2(os.path.join(level, n), os.path.join(dst, n))
            log(f"  input of {scene}: {', '.join(sorted(os.listdir(dst)))}")
        finally:
            try:
                mcp("window_control", timeout=30, action="close", discard=True)
            except (OSError, ValueError):
                pass
            try:
                proc.wait(timeout=60)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=30)
    finally:
        qa_target.remove_tree(work)


# ------------------------------------------------------------------------------------------- the C++ side

def stage_fsgame(stage: str) -> str:
    with open(os.path.join(REPO, "fsgame.ltx"), "rb") as f:
        text = f.read().decode("cp1251")
    root = REPO.rstrip("\\/") + "\\"
    out = []
    for line in text.splitlines():
        alias = line.split("=", 1)[0].strip().lower()
        if alias == "$game_levels$":
            out.append(f"$game_levels$ = true| false| {stage}\\| gamedata\\levels\\")
        elif alias == "$app_data_root$":
            out.append(line)
        else:
            out.append(line.replace("$fs_root$", root))
    return "\r\n".join(out) + "\r\n"


def output_files(level: str, inputs: dict[str, str]) -> dict[str, dict]:
    files = {}
    for d, dirs, names in os.walk(level):
        for n in names:
            p = os.path.join(d, n)
            rel = os.path.relpath(p, level).replace("\\", "/")
            h = sha256_file(p)
            if inputs.get(rel) == h:
                continue
            files[rel] = {"size": os.path.getsize(p), "sha256": h}
    return files


def run_cpp(scene: str, input_dir: str, dst: str, details_only: bool = False) -> dict:
    stage = tempfile.mkdtemp(prefix="gzlc_cpp_")
    try:
        bin_dir = os.path.join(stage, "bin", "x64", "Release")
        os.makedirs(bin_dir)
        for n in os.listdir(CPP_BIN):
            if n.lower().endswith((".exe", ".dll")):
                shutil.copy2(os.path.join(CPP_BIN, n), os.path.join(bin_dir, n))
        with open(os.path.join(stage, "fsgame.ltx"), "wb") as f:
            f.write(stage_fsgame(stage).encode("cp1251"))
        level = os.path.join(stage, "gamedata", "levels", scene)
        shutil.copytree(input_dir, level)
        inputs = {n: sha256_file(os.path.join(input_dir, n)) for n in os.listdir(input_dir)}
        rec: dict = {"runs": {}}
        for exe, enabled in (("XrLC.exe", not details_only),
                             ("xrDO_Light.exe", os.path.isfile(os.path.join(level, "build.details")))):
            if not enabled:
                continue
            log(f"  C++ {exe} -f {scene} -silent")
            code, w, wall = run_limited([os.path.join(bin_dir, exe), "-f", scene, "-silent", "-flushlog"], bin_dir,
                                         os.path.join(stage, "_appdata_", "logs"))
            cpu = w.samples[-1][1] if w.samples else 0.0
            log(f"  C++ {exe}: exit {code}, {wall:.1f} s wall, {cpu:.1f} s cpu, {len(w.lines)} log lines")
            rec["runs"][exe] = {"exit": code, "wall": wall, "cpu": w.samples, "lines": w.lines, "phases": w.phases}
            if code != 0:
                break
        logs = os.path.join(stage, "_appdata_", "logs")
        rec["files"] = output_files(level, inputs)
        os.makedirs(dst, exist_ok=True)
        for rel in rec["files"]:
            to = os.path.join(dst, *rel.split("/"))
            os.makedirs(os.path.dirname(to), exist_ok=True)
            shutil.copy2(os.path.join(level, *rel.split("/")), to)
        if os.path.isdir(logs):
            for n in os.listdir(logs):
                shutil.copy2(os.path.join(logs, n), os.path.join(dst, "_log_" + n))
        return rec
    finally:
        left = [p for p in our_processes() if stage.lower() in p.lower()]
        if left:
            log(f"! a compiler of the stage is still running: {left}")
        qa_target.remove_tree(stage)


def our_processes() -> list[str]:
    try:
        out = subprocess.run(["pwsh", "-NoProfile", "-Command",
                              "Get-CimInstance Win32_Process | ForEach-Object { $_.ExecutablePath }"],
                             capture_output=True, text=True, timeout=60).stdout
    except (OSError, subprocess.TimeoutExpired):
        return []
    return [l.strip() for l in out.splitlines() if "gzlc_" in l]


def record_oracle(scene: str, key: str) -> str:
    rd = record_dir(scene, key)
    tmp = rd + ".part"
    qa_target.remove_tree(tmp)
    os.makedirs(tmp)
    log(f"{scene}: recording the oracle ({key})")
    make_input(scene, os.path.join(tmp, "input"))
    rec = {"scene": scene, "key": key, "made": time.strftime("%Y-%m-%d %H:%M:%S")}
    rec["input"] = {n: {"size": os.path.getsize(os.path.join(tmp, "input", n)),
                        "sha256": sha256_file(os.path.join(tmp, "input", n))} for n in os.listdir(os.path.join(tmp, "input"))}
    if scene in DETAILS_ONLY:
        rust_level_files(scene, os.path.join(tmp, "input"))
        rec["input"] = {n: {"size": os.path.getsize(os.path.join(tmp, "input", n)),
                            "sha256": sha256_file(os.path.join(tmp, "input", n))}
                        for n in os.listdir(os.path.join(tmp, "input"))}
    for tag in ("cpp1", "cpp2"):
        rec[tag] = run_cpp(scene, os.path.join(tmp, "input"), os.path.join(tmp, tag), scene in DETAILS_ONLY)
        bad = [e for e, r in rec[tag]["runs"].items() if r["exit"] != 0]
        if bad:
            # a C++ run that failed or passed its limit records nothing, and the second run is not started
            qa_target.remove_tree(tmp)
            raise RuntimeError(f"{scene}: {', '.join(bad)} did not finish within {LIMIT:.0f} s: no oracle")
    with open(os.path.join(tmp, "record.json"), "w", encoding="utf-8") as f:
        json.dump(rec, f, indent=1)
    for old in os.listdir(ORACLE):
        if old.startswith(f"compile-{scene}-") and not old.endswith(".part"):
            qa_target.remove_tree(os.path.join(ORACLE, old))
    os.replace(tmp, rd)
    return rd


# ------------------------------------------------------------------------------------------- the Rust side

def rust_level_files(scene: str, input_dir: str) -> None:
    """The Rust compile of the input in a copy; its build.cform and build.lights go into the input."""
    work = tempfile.mkdtemp(prefix="gzlc_lvl_")
    try:
        r = run_rust(scene, input_dir, os.path.join(work, "level"), details=False)
        if any(x["exit"] != 0 for x in r["runs"].values()):
            raise RuntimeError(f"{scene}: the Rust compile failed")
        for n in ("build.cform", "build.lights"):
            shutil.copy2(os.path.join(work, "level", n), os.path.join(input_dir, n))
    finally:
        qa_target.remove_tree(work)


def run_rust(scene: str, input_dir: str, dst: str, details: bool = True, level: bool = True) -> dict:
    qa_target.remove_tree(dst)
    shutil.copytree(input_dir, dst)
    rec: dict = {"runs": {}, "exe": RUST_EXE, "exe_stamp": stamp(RUST_EXE)}
    home = tempfile.mkdtemp(prefix="gzlc_home_")
    try:
        inputs = {n: sha256_file(os.path.join(input_dir, n)) for n in os.listdir(input_dir)}
        for flag, enabled in (("-compile", level),
                              ("-compile_details", details and os.path.isfile(os.path.join(dst, "build.details")))):
            if not enabled:
                continue
            lines: list = []
            log(f"  Rust {flag} {scene}")
            code, w, wall = run_limited([RUST_EXE, "-headless", "-nodlg", "-root", REPO, "-home", home, flag, dst], REPO,
                                        None, lines)
            cpu = w.samples[-1][1] if w.samples else 0.0
            log(f"  Rust {flag}: exit {code}, {wall:.1f} s wall, {cpu:.1f} s cpu")
            rec["runs"][flag] = {"exit": code, "wall": wall, "cpu": w.samples, "lines": lines}
            if code != 0:
                for _, l in lines[-15:]:
                    log("    | " + l)
                break
        rec["files"] = output_files(dst, inputs)
    finally:
        qa_target.remove_tree(home)
    return rec


# ------------------------------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--scene", action="append")
    ap.add_argument("--refresh-oracle", action="store_true")
    ap.add_argument("--no-compare", action="store_true")
    args = ap.parse_args()
    import compare_compile
    state_path = os.path.join(OUT, "state.json")
    state = json.load(open(state_path, encoding="utf-8")) if os.path.isfile(state_path) else {}
    scenes = args.scene or SCENES
    failed = []
    for scene in scenes:
        key = key_of(scene)
        rd = record_dir(scene, key)
        if args.refresh_oracle or not os.path.isfile(os.path.join(rd, "record.json")):
            try:
                rd = record_oracle(scene, key)
            except RuntimeError as e:
                log(f"! {e}")
                failed.append(scene)
                continue
        finger = f"{key}|{stamp(RUST_EXE)}|{compare_compile.VERSION}"
        if not args.all and not args.scene and state.get(scene, {}).get("finger") == finger and state[scene].get("ok"):
            log(f"{scene}: unchanged since it passed ({finger})")
            continue
        with open(os.path.join(rd, "record.json"), encoding="utf-8") as f:
            rec = json.load(f)
        rust_dir = os.path.join(OUT, scene, "rust")
        log(f"{scene}: the Rust compile")
        rrec = run_rust(scene, os.path.join(rd, "input"), rust_dir, level=scene not in DETAILS_ONLY)
        with open(os.path.join(OUT, scene, "rust.json"), "w", encoding="utf-8") as f:
            json.dump(rrec, f, indent=1)
        ok = all(r["exit"] == 0 for r in rrec["runs"].values())
        if ok and not args.no_compare:
            ok = compare_compile.compare(scene, rd, rec, rust_dir, rrec, os.path.join(OUT, scene), log)
        state[scene] = {"finger": finger, "ok": ok, "when": time.strftime("%Y-%m-%d %H:%M:%S")}
        with open(state_path, "w", encoding="utf-8") as f:
            json.dump(state, f, indent=1)
        log(f"{scene}: {'PASS' if ok else 'FAIL'}")
        if not ok:
            failed.append(scene)
    left = our_processes()
    if left:
        log(f"! processes of ours still running: {left}")
        failed.append("processes")
    log("ALL PASS" if not failed else f"FAILED: {', '.join(failed)}")
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
