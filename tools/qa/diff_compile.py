"""The level compiler of the editor (gz-lc) against the C++ compilers of the cutover.

    python tools/qa/diff_compile.py                 the scenes that failed or changed since the last run
    python tools/qa/diff_compile.py --all           every scene, each compiled twice by the Rust compiler
    python tools/qa/diff_compile.py --scene mp_test one scene
    python tools/qa/diff_compile.py --refresh-oracle   record the C++ side again (and the input)

The oracle is recorded once per scene, keyed by the scene's files, the C++ compilers and the editor's data, under
Intermediate/oracle/compile-<scene>-<key>/ (a scene of SOURCE is another scene's level under build options of its
own, recorded under its own name). A recording is made whole in `<record>.part` and then takes the record's place;
--refresh-oracle moves an old record of the same key aside and removes it once the new one is in, and a recording
that fails leaves the old one as it was.
  input/        the build files the Rust editor's Make All wrote (build.prj and the rest) - the ONE input both
                compilers read
  cpp1/, cpp2/  what XrLC.exe and xrDO_Light.exe wrote in two runs (the second is the C++ run-to-run floor)
  record.json   every output file with size and SHA-256, the C++ log with the time each line appeared, the
                compilers' own phase list, the process's CPU time sampled through the run
A later run starts no C++ compiler when the record is there.

The C++ compilers (Bin/x64/CppCompilers: built from the tagged sources) run from a stage under the system temp
folder with an fsgame.ltx of their own (the editor's, its levels the stage's), `-silent`, hidden (no window, no
taskbar entry), under a hard limit of ten minutes (LIMITS names a scene's own), ended by their process id past it.
When a record is missing and that folder holds no compiler, the scene fails with a sentence saying so.

The Rust compile runs `Grizzly.exe -headless -compile <level folder>` (Intermediate/cargo/fast unless
GRIZZLY_LC_EXE names another), then `-compile_details`. The comparison is in compare_compile.py; this file runs the
compilers and records. Everything a run prints goes to Intermediate/compile/run.log as it happens.

The same on every run and on any number of cores: a scene of TWICE (all of them with --all) is compiled a second
time and every file of the two compiles is wanted byte for byte; ONE_THREAD is compiled once more on one thread
(`-threads=1`) and wanted byte for byte against the compile on every core. A difference names the first file and
byte that differ.
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
import struct
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
# The C++ compilers built from the tagged sources (Build.ps1 -Only compilers, cpp-compilers-last): the oracle. Read
# only; nothing here writes there.
CPP_BIN = os.path.join(REPO, "Bin", "x64", "CppCompilers")
RUST_EXE = os.environ.get("GRIZZLY_LC_EXE") or os.path.join(INTER, "cargo", "fast", "Grizzly.exe")
EDITOR_EXE = os.environ.get("GRIZZLY_QA_EXE") or RUST_EXE
TEST_PROJECT = r"D:\XFinedProjects\Test"
SCENES = ["mp_test", "l12u_control_monolith", "mp_close_combat", "mp_close_combat_implicit", "l03u_agr_underground"]
# Two lightmap pages come from mp_close_combat_implicit: mp_test stays one page at 20 and at 40 px/m, because every
# large deflector is clamped to 508 texels a side (xrDeflector.cpp: clamp(dwWidth, 1, 512 - 2 * BORDER)).
# A scene recorded under a name of its own: the level of another scene under other build options (OPTIONS).
SOURCE = {"mp_close_combat_implicit": "mp_close_combat"}
# Scenes proven for the detail compiler alone: XrLC does not finish them within the limit (mp_close_combat's
# implicit light passes ten minutes), so both detail compilers read the same build.prj, build.details and the
# Rust compiler's build.cform and build.lights (equal to the C++ ones under the rules of the other scenes).
DETAILS_ONLY = {"mp_close_combat"}
# The build options a scene's copy gets before Make All, so that XrLC finishes it within the limit. Both compilers
# read them from the same build.prj. l03u_agr_underground holds MU models (7 models, 15 references); at its own
# 10 px/m and 9 jitter samples XrLC's LMaps alone took 513 s. Neither option reaches the MU models: the pixels per
# metre size the deflectors (xrDeflector.cpp), the jitter samples their texels (xrDeflectorLight.cpp); the MU light
# takes its samples from the quality (xrMU_Model_Calc_lighting.cpp), which stays High.
OPTIONS = {"l03u_agr_underground": {"light_pixel_per_meter": "4.000000", "light_jitter_samples": "1"},
           # the implicit (terrain) light and two lightmap pages, at the scene's own 10 px/m, without -noise (the
           # static progressive meshes too; its MU models are emptied, EMPTIED). XrLC at the scene's own nine jitter
           # samples passed ten minutes in "LIGHT: Implicit". The samples are what the implicit pass costs - every
           # texel of the terrain texture is lit once per sample (xrLight_Implicit.cpp, Jitter_Select) - and the
           # one setting of the build that reaches it besides the quality (Draft skips it) and the lights; one sample
           # runs the same code once per texel. The nine-sample pattern is proven by the lightmaps of mp_test and
           # l12u_control_monolith (the same Jitter_Select).
           "mp_close_combat_implicit": {"light_jitter_samples": "1"}}
# Flags both level compilers get for a scene. -noise leaves the static visuals without progressive meshes
# (xrFlex2OGF.cpp: `if (!b_noise) MakeProgressive`); the MU models keep theirs (xrMU_Model_Calc_ogf.cpp calls it
# unconditionally). With 4 px/m and one sample XrLC still passed 600 s in "Converting to OGFs" (the static
# visuals' VIPM, one thread); the static progressive meshes are proven on l12u_control_monolith.
FLAGS = {"l03u_agr_underground": ["-noise"]}
# The limit of one C++ run where ten minutes do not hold it: the scene's one recording (two runs) may take forty.
LIMITS = {"l03u_agr_underground": 1200.0, "mp_close_combat_implicit": 1200.0}
# Chunks of build.prj emptied in the input both compilers read. XrLC of cpp-compilers-last crashes on
# mp_close_combat's multi-usage models: an access violation in "Converting MU-models to OGFs", right after the
# tangent basis of a model part of 7706 vertices (calc_ogf, xrMU_Model_Calc_ogf.cpp), at the same log line in two
# runs, after every light was written. Without the models (14), their references (15) and the LODs they name (13)
# the implicit light and the two pages are recorded; the MU models stay proven by l03u_agr_underground alone.
EMPTIED = {"mp_close_combat_implicit": [13, 14, 15]}
# The scenes whose second Rust compile costs seconds: compiled twice by default and every file of the two compiles
# wanted byte for byte (--all: every scene). ONE_THREAD, the smallest, is compiled a third time on one thread
# (-threads=1) and wanted byte for byte against the compile on every core.
TWICE = {"mp_test", "mp_close_combat", "l03u_agr_underground"}
ONE_THREAD = "mp_test"
FORMAT = 3
CPP_SOURCE = "cpp-compilers-last"
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


def source_of(scene: str) -> str:
    return SOURCE.get(scene, scene)


def scene_files(scene: str) -> list[tuple[str, str]]:
    """The files of the scene's level (the source level of a scene recorded under a name of its own)."""
    scene = source_of(scene)
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
    if scene in SOURCE:
        parts["record"] = scene
    if scene in OPTIONS:
        parts["options"] = OPTIONS[scene]
    if scene in FLAGS:
        parts["flags"] = FLAGS[scene]
    # the oracle is a build of one tagged source, so the key names the tag, not the binaries' stamps: a record stays
    # good when the folder is rebuilt from it, or gone
    parts["cpp"] = CPP_SOURCE
    parts["data"] = [stamp(os.path.join(REPO, "gamedata", "shaders_xrlc.xr")), stamp(os.path.join(REPO, "fsgame.ltx"))]
    return hashlib.sha256(json.dumps(parts, sort_keys=True).encode()).hexdigest()[:16]


def lower_options(level: str, options: dict[str, str]) -> None:
    """The scene file's [build_params] with the given keys set; every key must be there once."""
    with open(level, "rb") as f:
        text = f.read().decode("cp1251")
    lines = text.split("\n")
    section, found = "", set()
    for i, line in enumerate(lines):
        s = line.strip()
        if s.startswith("[") and "]" in s:
            section = s[1:s.index("]")].strip().lower()
            continue
        if section == "build_params" and "=" in s:
            k = s.split("=", 1)[0].strip()
            if k in options:
                head = line[:line.index("=") + 1]
                lines[i] = f"{head} {options[k]}" + ("\r" if line.endswith("\r") else "")
                found.add(k)
    if found != set(options):
        raise RuntimeError(f"{level}: [build_params] lacks {sorted(set(options) - found)}")
    with open(level, "wb") as f:
        f.write("\n".join(lines).encode("cp1251"))


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
        # the most top-level windows the process owned at once (a C++ compiler: none)
        self.windows = 0
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
                    self.windows = max(self.windows, windows_of(self.proc.pid))
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


u32 = ctypes.WinDLL("user32", use_last_error=True)
_ENUM = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
u32.EnumWindows.argtypes = [_ENUM, wt.LPARAM]
u32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]


def windows_of(pid: int) -> int:
    """The top-level windows `pid` owns, visible or not. A C++ compiler under -silent creates none (cl_log.cpp of
    cpp-compilers-last, commit 0ba98df4: no log dialog, no message box)."""
    found = [0]

    def each(hwnd, _):
        owner = wt.DWORD()
        u32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid:
            found[0] += 1
        return True

    u32.EnumWindows(_ENUM(each), 0)
    return found[0]


def hidden() -> dict:
    """What starts a process with no window and no taskbar entry: the author's desktop stays as it is."""
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0  # SW_HIDE
    return {"startupinfo": startup, "creationflags": subprocess.CREATE_NO_WINDOW}


def run_limited(cmd: list[str], cwd: str, log_dir: str | None, stdout_lines: list | None = None,
                limit: float = LIMIT) -> tuple[int | None, Watch, float]:
    t0 = time.monotonic()
    proc = subprocess.Popen(cmd, cwd=cwd, **hidden(),
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
        code = proc.wait(timeout=limit)
    except subprocess.TimeoutExpired:
        log(f"! {os.path.basename(cmd[0])} passed its {limit:.0f} s: ended by its process id {proc.pid}")
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


def empty_chunks(path: str, ids: list[int]) -> None:
    """build.prj with the bodies of the given chunks emptied, every other byte as it was."""
    with open(path, "rb") as f:
        data = f.read()
    out, at = bytearray(), 0
    while at + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, at)
        body = b"" if (cid & 0x7FFFFFFF) in ids else data[at + 8:at + 8 + size]
        out += struct.pack("<II", cid, len(body)) + body
        at += 8 + size
    with open(path, "wb") as f:
        f.write(bytes(out))


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
        source = source_of(scene)
        if scene in OPTIONS:
            lower_options(os.path.join(project, "rawdata", "levels", source + ".level"), OPTIONS[scene])
            log(f"  {scene}: build options {OPTIONS[scene]}")
        home = os.path.join(work, "home")
        os.makedirs(home, exist_ok=True)
        env = dict(os.environ, GRIZZLY_HOME=home)
        qa_target.guard_project(project)
        proc = subprocess.Popen([EDITOR_EXE, "-mcp_port", str(PORT), "-home", home, "-nodlg", "-headless",
                                 "-project", project], cwd=os.path.dirname(EDITOR_EXE), env=env, **hidden(),
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
            a = mcp("open_scene", timeout=300, scene=source + ".level")
            if not a.get("ok"):
                raise RuntimeError(f"open_scene: {a}")
            a = mcp("level_build", timeout=1800, action="make_all")
            if not a.get("ok"):
                raise RuntimeError(f"make_all: {a}")
            level = os.path.join(project, "gamedata", "levels", source)
            os.makedirs(dst, exist_ok=True)
            for n in sorted(os.listdir(level)):
                if os.path.isfile(os.path.join(level, n)):
                    shutil.copy2(os.path.join(level, n), os.path.join(dst, n))
            if scene in EMPTIED:
                empty_chunks(os.path.join(dst, "build.prj"), EMPTIED[scene])
                log(f"  {scene}: build.prj chunks {EMPTIED[scene]} emptied")
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
        level = os.path.join(stage, "gamedata", "levels", source_of(scene))
        shutil.copytree(input_dir, level)
        inputs = {n: sha256_file(os.path.join(input_dir, n)) for n in os.listdir(input_dir)}
        rec: dict = {"runs": {}}
        for exe, enabled in (("XrLC.exe", not details_only),
                             ("xrDO_Light.exe", os.path.isfile(os.path.join(level, "build.details")))):
            if not enabled:
                continue
            flags = FLAGS.get(scene, []) if exe == "XrLC.exe" else []
            log(f"  C++ {exe} -f {source_of(scene)} -silent {' '.join(flags)}")
            code, w, wall = run_limited([os.path.join(bin_dir, exe), "-f", source_of(scene), "-silent", "-flushlog",
                                         *flags],
                                        bin_dir, os.path.join(stage, "_appdata_", "logs"),
                                        limit=LIMITS.get(scene, LIMIT))
            cpu = w.samples[-1][1] if w.samples else 0.0
            log(f"  C++ {exe}: exit {code}, {wall:.1f} s wall, {cpu:.1f} s cpu, {len(w.lines)} log lines, "
                f"{w.windows} windows of its own")
            if w.windows:
                log(f"! {exe} made a window: the oracle build is not the one of {CPP_SOURCE}")
                code = code if code else 1
            rec["runs"][exe] = {"exit": code, "wall": wall, "cpu": w.samples, "lines": w.lines, "phases": w.phases,
                                "windows": w.windows}
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
    """The record of the scene made whole in `<record>.part` and then put in the record's place: a record already
    there (--refresh-oracle) is moved aside first and removed once the new one is in; a recording that fails leaves
    the old record as it was and no `.part`."""
    rd = record_dir(scene, key)
    tmp = rd + ".part"
    qa_target.remove_tree(tmp)
    log(f"{scene}: recording the oracle ({key})")
    if not os.path.isfile(os.path.join(CPP_BIN, "XrLC.exe")):
        raise RuntimeError(f"{scene}: no record under {ORACLE} for this input, and {CPP_BIN} holds no C++ compiler to "
                           f"record one (a build of {CPP_SOURCE}: Build.ps1 of that tag, -Only compilers)")
    os.makedirs(tmp)
    try:
        record_into(scene, key, tmp)
    except BaseException:
        qa_target.remove_tree(tmp)
        raise
    old = rd + ".old"
    qa_target.remove_tree(old)
    if os.path.isdir(rd):
        os.replace(rd, old)
    try:
        os.replace(tmp, rd)
    except OSError:
        if os.path.isdir(old):
            os.replace(old, rd)
        qa_target.remove_tree(tmp)
        raise
    qa_target.remove_tree(old)
    return rd


def ended(code: int | None, limit: float) -> str:
    """How a C++ run that did not succeed ended, in words."""
    if code is None:
        return f"did not finish within {limit:.0f} s"
    if code < 0 or code > 0x7FFFFFFF:
        return f"crashed (exception 0x{code & 0xFFFFFFFF:08X})"
    return f"exited with code {code}"


def record_into(scene: str, key: str, tmp: str) -> None:
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
        bad = [(e, r) for e, r in rec[tag]["runs"].items() if r["exit"] != 0]
        if bad:
            # a C++ run that failed or passed its limit records nothing, and the second run is not started; what it
            # wrote and its log stay under Intermediate/compile/<scene>/cpp_failed for a look
            kept = os.path.join(OUT, scene, "cpp_failed")
            qa_target.remove_tree(kept)
            shutil.copytree(os.path.join(tmp, tag), kept)
            with open(os.path.join(kept, "run.json"), "w", encoding="utf-8") as f:
                json.dump(rec[tag], f, indent=1)
            how = "; ".join(f"{e} {ended(r['exit'], LIMITS.get(scene, LIMIT))}" for e, r in bad)
            raise RuntimeError(f"{scene}: {how}: no oracle (what it wrote and its log: {kept})")
    with open(os.path.join(tmp, "record.json"), "w", encoding="utf-8") as f:
        json.dump(rec, f, indent=1)
    # a record of an earlier input or build stays where it is: only a record of the same key is replaced


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


def run_rust(scene: str, input_dir: str, dst: str, details: bool = True, level: bool = True,
             extra: tuple[str, ...] = ()) -> dict:
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
            flags = [*FLAGS.get(scene, []), *extra] if flag == "-compile" else []
            code, w, wall = run_limited([RUST_EXE, "-headless", "-nodlg", "-root", REPO, "-home", home, flag, dst,
                                         *flags], REPO, None, lines, limit=LIMITS.get(scene, LIMIT))
            cpu = w.samples[-1][1] if w.samples else 0.0
            log(f"  Rust {flag} {' '.join(flags)}: exit {code}, {wall:.1f} s wall, {cpu:.1f} s cpu")
            rec["runs"][flag] = {"exit": code, "wall": wall, "cpu": w.samples, "lines": lines}
            if code != 0:
                for _, l in lines[-15:]:
                    log("    | " + l)
                break
        rec["files"] = output_files(dst, inputs)
    finally:
        qa_target.remove_tree(home)
    return rec


def first_difference(a: str, b: str) -> str | None:
    """Two outputs of the Rust compiler byte for byte: None, or the first file (by name) and offset that differ."""
    def files(d: str) -> dict[str, str]:
        out = {}
        for root, _, names in os.walk(d):
            for n in names:
                p = os.path.join(root, n)
                out[os.path.relpath(p, d).replace("\\", "/")] = p
        return out
    fa, fb = files(a), files(b)
    for name in sorted(set(fa) | set(fb)):
        if name not in fa or name not in fb:
            return f"{name} is in one output only"
        with open(fa[name], "rb") as f:
            x = f.read()
        with open(fb[name], "rb") as f:
            y = f.read()
        if x != y:
            at = next((i for i, (p, q) in enumerate(zip(x, y)) if p != q), min(len(x), len(y)))
            return f"{name} differs from byte {at} on ({len(x)} / {len(y)} bytes)"
    return None


def determinism(scene: str, rd: str, rust_dir: str, every: bool) -> list[str]:
    """The scene compiled again (TWICE, or every scene with --all) and, for ONE_THREAD, on one thread: each output
    byte for byte equal to the first compile's."""
    runs = []
    if every or scene in TWICE:
        runs.append(("again", ()))
    if scene == ONE_THREAD:
        runs.append(("one thread", ("-threads=1",)))
    bad = []
    for what, extra in runs:
        other = os.path.join(OUT, scene, "rust_" + what.replace(" ", "_"))
        log(f"{scene}: the Rust compile {what}")
        r = run_rust(scene, os.path.join(rd, "input"), other, level=scene not in DETAILS_ONLY, extra=extra)
        if any(x["exit"] != 0 for x in r["runs"].values()):
            bad.append(f"the Rust compile {what} failed")
            continue
        diff = first_difference(rust_dir, other)
        log(f"  {what}: " + (diff or f"every file byte for byte ({len(r['files'])} written)"))
        if diff:
            bad.append(f"the Rust compile {what}: {diff}")
        else:
            qa_target.remove_tree(other)
    return bad


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
        if all(r["exit"] == 0 for r in rrec["runs"].values()):
            for p in determinism(scene, rd, rust_dir, args.all):
                log("  ! " + p)
                ok = False
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
