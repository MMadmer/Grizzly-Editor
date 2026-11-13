"""The level compiler of the editor (gz-lc) against the C++ compilers of the cutover.

    python tools/qa/diff_compile.py                 the quick scenes (QUICK: two minutes) that failed or changed
                                                    since the last run - the everyday run
    python tools/qa/diff_compile.py --all           every scene, each compiled twice by the Rust compiler: forty
                                                    minutes, most of them the four big levels - for a change of
                                                    the compiler, before it is called done
    python tools/qa/diff_compile.py --scene mp_test one scene
    python tools/qa/diff_compile.py --refresh-oracle   record the C++ side again (and the input)
    python tools/qa/diff_compile.py --reuse-rust --scene s   the rules again over the last Rust output (no compile)
    python tools/qa/diff_compile.py --rerecord mp_test       a rebuilt oracle against a record, the record untouched
    python tools/qa/diff_compile.py --diagnose s --cpp-bin <diag build> [--set k=v] [--flag=-x]   where XrLC crashes

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

The C++ compilers (Bin/x64/CppCompilers: a build of D:/Games/XFined-Editor-cpp, cpp-compilers-last with the stack
fix of STATE.md "Closing: the compiler's proof") run from a stage under Intermediate/compile/stages with an
fsgame.ltx of their own (the editor's, its levels the stage's): the stage's bin folder holds HARD LINKS to the
oracle's files, so no byte of a program is written and nothing goes under the temp folder (their xrCore reads its
fs config three folders above the executable and takes none on its command line). `-silent -nodlg`, hidden (no
window, no taskbar entry), under a hard limit of ten minutes (LIMITS names a scene's own), ended by their process
id past it. When a record is missing and that folder holds no compiler, the scene fails with a sentence saying so.
--diagnose runs XrLC of a diagnostic build (PDBs beside it) under a debugger and names where it crashes;
--rerecord runs the oracle again on a record's input and compares, the record untouched (a rebuilt oracle).

The Rust compile runs `Grizzly.exe -headless -compile <level folder>` (Intermediate/cargo/fast unless
GRIZZLY_LC_EXE names another), then `-compile_details`. The comparison is in compare_compile.py; this file runs the
compilers and records. Everything a run prints goes to Intermediate/compile/run.log as it happens.

The same on every run and on any number of cores: a scene of TWICE (all of them with --all) is compiled a second
time and every file of the two compiles is wanted byte for byte; ONE_THREAD (with --all also ONE_THREAD_ALL, the
scene whose light once differed from run to run) is compiled once more on one thread
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
SCENES = ["mp_test", "l12u_control_monolith", "mp_close_combat", "mp_close_combat_implicit", "l03u_agr_underground",
          # the flags, each on mp_test, where each changes something (the Rust compile with and without it:
          # -nosun the sun's light in level.geom and the page; -nosmg level.cform; -no_optimize level.geom;
          # -att the lightmap's colour); the invalid face, refused and kept
          "mp_test_nosun", "mp_test_nosmg", "mp_test_no_optimize", "mp_test_att", "mp_test_invalid",
          "mp_test_invalid_kept",
          # the Draft quality on the cheapest scene with implicit light, progressive meshes and MU models
          "mp_close_combat_draft", "mp_close_combat_draft_nosmg",
          # mp_close_combat whole at its own settings (10 px/m, nine jitter samples, its MU models, the static
          # progressive meshes): the implicit light at nine samples, MU models and implicit light in one level,
          # MU models at a scene's own settings; the same level at 20 px/m for five lightmap pages
          "mp_close_combat_whole", "mp_close_combat_pages",
          # l03u_agr_underground at 6 px/m: two pages over 31 sectors, MU models with their sliding windows - the
          # paged rule where the sector of a triangle and the windows count
          "l03u_agr_underground_paged",
          # l03u_agr_underground at its own settings (10 px/m, nine samples, no -noise)
          "l03u_agr_underground_own"]
# Two lightmap pages come from mp_close_combat_implicit: mp_test stays one page at 20 and at 40 px/m, because every
# large deflector is clamped to 508 texels a side (xrDeflector.cpp: clamp(dwWidth, 1, 512 - 2 * BORDER)).
# A scene recorded under a name of its own: the level of another scene under other build options (OPTIONS).
SOURCE = {"mp_close_combat_implicit": "mp_close_combat", "mp_close_combat_draft": "mp_close_combat",
          "mp_test_nosun": "mp_test", "mp_test_nosmg": "mp_test", "mp_test_no_optimize": "mp_test",
          "mp_test_att": "mp_test", "mp_test_invalid": "mp_test", "mp_test_invalid_kept": "mp_test",
          "mp_close_combat_draft_nosmg": "mp_close_combat", "mp_close_combat_whole": "mp_close_combat", "mp_close_combat_pages": "mp_close_combat",
          "l03u_agr_underground_own": "l03u_agr_underground", "l03u_agr_underground_paged": "l03u_agr_underground"}
# Edits of build.prj in the input both compilers read (EDITS names the function). `invalid_face` collapses one
# face's texture coordinates to a point: Face::Verify refuses it (xrFace.cpp: the coordinates' perimeter under half
# a texel of a 4096 texture), and it is no degenerate face PreOptimize would remove first.
EDITS = {"mp_test_invalid": "invalid_face", "mp_test_invalid_kept": "invalid_face"}
# Scenes both compilers must refuse: XrLC's Debug.fatal ("* FATAL: <n> invalid faces. Compilation aborted",
# PreOptimize; under -nodlg it terminates with 3, no dialog), the Rust compile its error exit 1 (3 is its crash).
# build.err of both byte for byte; the C++ leaves the build.lights it wrote at the start of CBuild::Run and the
# 12-byte head of the level file it opened then, the Rust compile writes a level's files together or not at all
# (STATE.md, the final review), so nothing but build.err.
REFUSED = {"mp_test_invalid"}
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
           "mp_close_combat_implicit": {"light_jitter_samples": "1"},
           # the Draft quality as the editor's Draft button sets it (communicate.h setDraftQuality: 0.1 px/m, one
           # jitter sample): no implicit light (xrLight_Implicit.cpp), no progressive meshes (OGF_Face.cpp
           # MakeProgressive), one sample for the MU light (xrMU_Model_Calc_lighting.cpp)
           # five lightmap pages (the Rust compile: 2 at 10 px/m, 3 at 15, 5 at 20), one jitter sample
           "mp_close_combat_pages": {"light_pixel_per_meter": "20.000000", "light_jitter_samples": "1"},
           # two pages (the Rust compile: 2 at 6, 8 and 10 px/m), one sample, -noise like l03u_agr_underground
           "l03u_agr_underground_paged": {"light_pixel_per_meter": "6.000000", "light_jitter_samples": "1"},
           "mp_close_combat_draft": {"light_quality": "0", "light_pixel_per_meter": "0.100000",
                                     "light_jitter_samples": "1"},
           "mp_close_combat_draft_nosmg": {"light_quality": "0", "light_pixel_per_meter": "0.100000",
                                           "light_jitter_samples": "1"}}
# Flags both level compilers get for a scene. -noise leaves the static visuals without progressive meshes
# (xrFlex2OGF.cpp: `if (!b_noise) MakeProgressive`); the MU models keep theirs (xrMU_Model_Calc_ogf.cpp calls it
# unconditionally). With 4 px/m and one sample XrLC still passed 600 s in "Converting to OGFs" (the static
# visuals' VIPM, one thread); the static progressive meshes are proven on l12u_control_monolith.
FLAGS = {"l03u_agr_underground": ["-noise"], "l03u_agr_underground_paged": ["-noise"], "mp_test_nosun": ["-nosun"], "mp_test_nosmg": ["-nosmg"],
         "mp_close_combat_draft_nosmg": ["-nosmg"], "mp_test_no_optimize": ["-no_optimize"], "mp_test_att": ["-att"], "mp_test_invalid_kept": ["-no_invalidefaces"]}
# Scenes whose input leaves the detail objects out (build.details, build_details.dds): neither detail compiler runs.
# -nosmg reaches the level compiler alone.
NO_DETAILS = {"mp_close_combat_draft_nosmg"}
# Given to every C++ compiler run: a Debug.fatal or an unhandled exception terminates the process (exit 3) instead
# of a system-modal box nobody clicks (xrDebugNew.cpp no_dialogs). It changes nothing else.
CPP_FLAGS = ["-nodlg"]
# The limit of one C++ run where ten minutes do not hold it (forty minutes at most).
LIMITS = {"l03u_agr_underground": 1200.0, "mp_close_combat_implicit": 1200.0,
          # forty minutes a run (the brief's bound on LIMITS), for the scenes at their own settings
          "mp_close_combat_whole": 2400.0, "l03u_agr_underground_own": 2400.0, "mp_close_combat_pages": 1800.0,
          "l03u_agr_underground_paged": 1200.0}
# Chunks of build.prj emptied in the input both compilers read. Recorded so when XrLC of cpp-compilers-last crashed on
# mp_close_combat's multi-usage models in "Converting MU-models to OGFs": a stack overflow (reported as an access
# violation) in XrETools' Forsyth optimiser, whose outer search recurses once per face of a sliding window, on the
# 1 MB main thread (diff_compile --diagnose, STATE.md "Closing: the compiler's proof"). The oracle now links XrLC
# with a 128 MB stack; mp_close_combat_whole holds the models. This scene keeps its record: without the models (14),
# their references (15) and the LODs they name (13), at one sample.
EMPTIED = {"mp_close_combat_implicit": [13, 14, 15]}
# The scenes whose second Rust compile costs seconds: compiled twice by default and every file of the two compiles
# wanted byte for byte (--all: every scene). ONE_THREAD, the smallest, is compiled a third time on one thread
# (-threads=1) and wanted byte for byte against the compile on every core.
TWICE = {"mp_test", "mp_close_combat", "l03u_agr_underground"}
# The everyday run: the scenes whose Rust compile and comparison cost seconds - every flag, the invalid face, the
# Draft quality, the detail compiler - about two minutes together. The big levels (l12u, the implicit light, the
# pages, the MU scenes at their own settings) run with --all or by name.
QUICK = ["mp_test", "mp_test_nosun", "mp_test_nosmg", "mp_test_no_optimize", "mp_test_att", "mp_test_invalid",
         "mp_test_invalid_kept", "mp_close_combat", "mp_close_combat_draft", "mp_close_combat_draft_nosmg"]
ONE_THREAD = "mp_test"
# With --all the scene where the compile once depended on what a thread had traced before is compiled on one
# thread too (two minutes on the release build): mp_test never showed that fault.
ONE_THREAD_ALL = "l03u_agr_underground"
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
    if scene in EDITS:
        parts["edit"] = EDITS[scene]
    if scene in NO_DETAILS:
        parts["no_details"] = True
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


# ------------------------------------------------------------------------------------------- a crash, named

class _Dbg:
    """The few kernel32 / dbghelp calls a debugger needs: a C++ compiler started under it reports where it crashed,
    by function, file and line, from the PDBs beside a diagnostic build (build_cpp.ps1 -Diag). Used by --diagnose
    alone, never for a record."""

    def __init__(self) -> None:
        k = ctypes.WinDLL("kernel32", use_last_error=True)
        d = ctypes.WinDLL(os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "dbghelp.dll"))
        self.k, self.d = k, d
        k.WaitForDebugEvent.argtypes = [ctypes.c_void_p, wt.DWORD]
        k.ContinueDebugEvent.argtypes = [wt.DWORD, wt.DWORD, wt.DWORD]
        k.OpenThread.restype = wt.HANDLE
        k.OpenThread.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
        k.GetThreadContext.argtypes = [wt.HANDLE, ctypes.c_void_p]
        k.TerminateProcess.argtypes = [wt.HANDLE, wt.UINT]
        d.SymInitialize.argtypes = [wt.HANDLE, ctypes.c_char_p, wt.BOOL]
        d.SymSetOptions.argtypes = [wt.DWORD]
        d.SymLoadModuleEx.restype = ctypes.c_uint64
        d.SymLoadModuleEx.argtypes = [wt.HANDLE, wt.HANDLE, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint64,
                                      wt.DWORD, ctypes.c_void_p, wt.DWORD]
        d.SymFromAddr.argtypes = [wt.HANDLE, ctypes.c_uint64, ctypes.POINTER(ctypes.c_uint64), ctypes.c_void_p]
        d.SymGetLineFromAddr64.argtypes = [wt.HANDLE, ctypes.c_uint64, ctypes.POINTER(wt.DWORD), ctypes.c_void_p]
        d.SymGetModuleBase64.restype = ctypes.c_uint64
        d.SymGetModuleBase64.argtypes = [wt.HANDLE, ctypes.c_uint64]
        d.StackWalk64.argtypes = [wt.DWORD, wt.HANDLE, wt.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                  ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]

    def load(self, hp, hfile: int, base: int) -> None:
        """A module the process loaded: its symbols, and its range for the stack scan."""
        self.k.GetFinalPathNameByHandleW.argtypes = [wt.HANDLE, wt.LPWSTR, wt.DWORD, wt.DWORD]
        path = ctypes.create_unicode_buffer(1024)
        n = self.k.GetFinalPathNameByHandleW(hfile, path, 1024, 0) if hfile else 0
        name = path.value[4:] if path.value.startswith("\\\\?\\") else path.value
        head = self.read(hp, base, 4096)
        size = 0
        if len(head) >= 0x40:
            nt = struct.unpack_from("<I", head, 0x3C)[0]
            if nt + 0x54 <= len(head):
                size = struct.unpack_from("<I", head, nt + 0x50)[0]
        got = self.d.SymLoadModuleEx(hp, None, name.encode("mbcs") if n else None, None, base, size, None, 0)
        if not hasattr(self, "modules"):
            self.modules = []
        self.modules.append((base, base + size, os.path.basename(name), bool(got)))

    def read(self, hp, addr: int, size: int) -> bytes:
        self.k.ReadProcessMemory.argtypes = [wt.HANDLE, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_size_t,
                                             ctypes.POINTER(ctypes.c_size_t)]
        buf = ctypes.create_string_buffer(size)
        got = ctypes.c_size_t()
        n = size
        while n >= 4096 and not self.k.ReadProcessMemory(hp, addr, buf, n, ctypes.byref(got)):
            n //= 2
        return buf.raw[:got.value]

    def module_of(self, addr: int) -> str | None:
        return next((m[2] for m in getattr(self, "modules", []) if m[0] <= addr < m[1]), None)

    def name(self, hp, addr: int) -> str:
        sym = ctypes.create_string_buffer(88 + 1024)
        struct.pack_into("<I", sym, 0, 88)
        struct.pack_into("<I", sym, 80, 1024)
        disp = ctypes.c_uint64()
        out = f"0x{addr:x}"
        if self.d.SymFromAddr(hp, addr, ctypes.byref(disp), sym):
            out = sym.raw[84:84 + 1024].split(b"\0", 1)[0].decode("ascii", "replace") + f"+0x{disp.value:x}"
        line = ctypes.create_string_buffer(40)
        struct.pack_into("<I", line, 0, 40)
        d32 = wt.DWORD()
        if self.d.SymGetLineFromAddr64(hp, addr, ctypes.byref(d32), line):
            no = struct.unpack_from("<I", line, 16)[0]
            fn = ctypes.string_at(struct.unpack_from("<Q", line, 24)[0]).decode("cp1251", "replace")
            out += f"  {fn}:{no}"
        return out

    def scan(self, hp, rsp: int, size: int = 1 << 20) -> list[str]:
        """The stack read from `rsp` up and every word that points into a loaded module named: what a walk cannot
        unwind (a stack overflow) still shows its frames this way. The innermost 24 names in order, then every name
        with the times it was seen."""
        raw = self.read(hp, rsp, size)
        words = struct.unpack_from(f"<{len(raw) // 8}Q", raw, 0)
        seen: dict[str, int] = {}
        order: list[str] = []
        for w in words:
            mod = self.module_of(w)
            if not mod:
                continue
            nm = self.name(hp, w).split("+", 1)[0]
            if nm.startswith("0x"):
                nm = f"{mod}!?"
            if nm not in seen and len(order) < 24:
                order.append(nm)
            seen[nm] = seen.get(nm, 0) + 1
        top = sorted(seen.items(), key=lambda kv: -kv[1])[:20]
        mods = getattr(self, "modules", [])
        return ([f"modules {len(mods)}, with symbols {sum(1 for m in mods if m[3])}",
                 f"scan of {len(raw)} stack bytes from 0x{rsp:x}:"] + ["  " + o for o in order]
                + [f"  {c:6d} x {nm}" for nm, c in top] + ["walk:"])

    def stack(self, hp, ht, frames: int = 40) -> list[str]:
        ctx = ctypes.create_string_buffer(1232 + 16)
        at = (16 - ctypes.addressof(ctx) % 16) % 16
        cp = ctypes.addressof(ctx) + at
        ctypes.memmove(cp + 0x30, struct.pack("<I", 0x10000B), 4)
        if not self.k.GetThreadContext(ht, cp):
            return [f"GetThreadContext failed ({ctypes.get_last_error()})"]
        raw = ctypes.string_at(cp, 1232)
        rip, rsp, rbp = struct.unpack_from("<Q", raw, 0xF8)[0], struct.unpack_from("<Q", raw, 0x98)[0], \
            struct.unpack_from("<Q", raw, 0xA0)[0]
        frame = ctypes.create_string_buffer(512)
        for off, val in ((0, rip), (32, rbp), (48, rsp)):
            struct.pack_into("<QHxxI", frame, off, val, 0, 3)
        out = self.scan(hp, rsp)
        fta = ctypes.cast(self.d.SymFunctionTableAccess64, ctypes.c_void_p)
        gmb = ctypes.cast(self.d.SymGetModuleBase64, ctypes.c_void_p)
        for _ in range(frames):
            if not self.d.StackWalk64(0x8664, hp, ht, frame, cp, None, fta, gmb, None):
                break
            pc = struct.unpack_from("<Q", frame, 0)[0]
            if not pc:
                break
            out.append(self.name(hp, pc))
        return out


def run_debugged(cmd: list[str], cwd: str, limit: float) -> tuple[int | None, float, list[str]]:
    """`cmd` started under a debugger: its exit code, its wall time and, for every access violation or other fault it
    met (first chance), the stack of the faulting thread - the last three kept."""
    dbg = _Dbg()
    k = dbg.k

    class SI(ctypes.Structure):
        _fields_ = [("cb", wt.DWORD), ("r", ctypes.c_void_p), ("desk", ctypes.c_void_p), ("title", ctypes.c_void_p),
                    ("x", wt.DWORD), ("y", wt.DWORD), ("w", wt.DWORD), ("h", wt.DWORD), ("cx", wt.DWORD),
                    ("cy", wt.DWORD), ("fill", wt.DWORD), ("flags", wt.DWORD), ("show", wt.WORD), ("r2", wt.WORD),
                    ("r3", ctypes.c_void_p), ("i", wt.HANDLE), ("o", wt.HANDLE), ("e", wt.HANDLE)]

    class PI(ctypes.Structure):
        _fields_ = [("hp", wt.HANDLE), ("ht", wt.HANDLE), ("pid", wt.DWORD), ("tid", wt.DWORD)]

    si, pi = SI(), PI()
    si.cb, si.flags, si.show = ctypes.sizeof(SI), 1, 0
    line = subprocess.list2cmdline(cmd)
    # a process under a debugger gets the checking heap unless told otherwise: the crash must meet the release heap
    env = dict(os.environ, _NO_DEBUG_HEAP="1")
    block = "".join(f"{a}={b}\0" for a, b in env.items()) + "\0"
    k.CreateProcessW.argtypes = [wt.LPCWSTR, wt.LPWSTR, ctypes.c_void_p, ctypes.c_void_p, wt.BOOL, wt.DWORD,
                                 ctypes.c_void_p, wt.LPCWSTR, ctypes.c_void_p, ctypes.c_void_p]
    buf = ctypes.create_unicode_buffer(line)
    envbuf = ctypes.create_unicode_buffer(block)
    # DEBUG_ONLY_THIS_PROCESS | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT
    if not k.CreateProcessW(None, buf, None, None, False, 0x2 | 0x08000000 | 0x400, envbuf, cwd, ctypes.byref(si),
                            ctypes.byref(pi)):
        raise OSError(f"CreateProcess: {ctypes.get_last_error()}")
    t0 = time.monotonic()
    ev = ctypes.create_string_buffer(256)
    hp = pi.hp
    crashes: list[str] = []
    code = None
    dbg.d.SymSetOptions(0x2 | 0x10 | 0x4)
    dbg.d.SymInitialize(hp, os.path.dirname(cmd[0]).encode("mbcs"), False)
    while True:
        if time.monotonic() - t0 > limit:
            k.TerminateProcess(hp, 1)
        if not k.WaitForDebugEvent(ev, 500):
            continue
        what, pid, tid = struct.unpack_from("<III", ev, 0)
        cont = 0x00010002
        if what == 3:
            hfile, base = struct.unpack_from("<Q", ev, 16)[0], struct.unpack_from("<Q", ev, 16 + 24)[0]
            dbg.load(hp, hfile, base)
        elif what == 6:
            hfile, base = struct.unpack_from("<QQ", ev, 16)
            dbg.load(hp, hfile, base)
        elif what == 1:
            exc, _, _, addr = struct.unpack_from("<IIQQ", ev, 16)
            first = struct.unpack_from("<I", ev, 16 + 152)[0]
            if exc in (0x80000003, 0x4000001F):
                cont = 0x00010002
            else:
                cont = 0x80010001
                if exc & 0xF0000000 == 0xC0000000:
                    ht = k.OpenThread(0x001F03FF, False, tid)
                    text = [f"exception 0x{exc:08X} at {dbg.name(hp, addr)} ({'first' if first else 'second'} chance, "
                            f"thread {tid}, {time.monotonic() - t0:.1f} s)"]
                    text += ["    " + f for f in dbg.stack(hp, ht)]
                    k32.CloseHandle(ht)
                    crashes = (crashes + ["\n".join(text)])[-3:]
        elif what == 5:
            code = struct.unpack_from("<I", ev, 16)[0]
            k.ContinueDebugEvent(pid, tid, cont)
            break
        k.ContinueDebugEvent(pid, tid, cont)
    k32.CloseHandle(pi.ht)
    k32.CloseHandle(hp)
    return code, time.monotonic() - t0, crashes


def diagnose(scene: str, settings: dict[str, str], flags: list[str], cpp_bin: str, limit: float) -> int:
    """XrLC of `cpp_bin` (a diagnostic build) on the scene under other build options, nothing emptied, under the
    debugger: where it crashes. A stage under Intermediate/compile/<scene>/diag (never the temp folder); no record."""
    base = os.path.join(OUT, scene, "diag")
    inp = os.path.join(base, "input")
    if not os.path.isfile(os.path.join(inp, "build.prj")):
        saved_o, saved_e = OPTIONS.get(scene), EMPTIED.pop(scene, None)
        OPTIONS[scene] = settings
        try:
            make_input(scene, inp)
        finally:
            if saved_o is None:
                OPTIONS.pop(scene, None)
            else:
                OPTIONS[scene] = saved_o
            if saved_e is not None:
                EMPTIED[scene] = saved_e
    stage = os.path.join(base, "stage")
    qa_target.remove_tree(stage)
    bin_dir = os.path.join(stage, "bin", "x64", "Release")
    os.makedirs(bin_dir)
    link_binaries(cpp_bin, bin_dir, (".pdb",))
    with open(os.path.join(stage, "fsgame.ltx"), "wb") as f:
        f.write(stage_fsgame(stage).encode("cp1251"))
    shutil.copytree(inp, os.path.join(stage, "gamedata", "levels", source_of(scene)))
    log(f"{scene}: diagnosis, XrLC of {cpp_bin} under the debugger, {settings} {flags}")
    code, wall, crashes = run_debugged([os.path.join(bin_dir, "XrLC.exe"), "-f", source_of(scene), "-silent",
                                        "-flushlog", *flags], bin_dir, limit)
    log(f"  exit {code if code is None else hex(code)}, {wall:.1f} s")
    for c in crashes:
        log("  " + c)
    logs = os.path.join(stage, "_appdata_", "logs")
    if os.path.isdir(logs):
        for n in os.listdir(logs):
            if n.lower().endswith(".log"):
                with open(os.path.join(logs, n), "rb") as f:
                    for row in f.read().decode("cp1251", "replace").splitlines()[-12:]:
                        log("  | " + row)
    return 0


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


PROGRAMS = (".exe", ".dll", ".com", ".scr", ".sys", ".msi", ".bat", ".cmd", ".ps1")


def no_programs(d: str) -> None:
    """A folder of a run under the temp folder holds no file with a program's name when it is removed: a recording
    writes no program there (the C++ stage holds links, beside the outputs)."""
    found = [os.path.join(r, n) for r, _, ns in os.walk(d) for n in ns if n.lower().endswith(PROGRAMS)]
    if found:
        raise RuntimeError(f"a program was written under the temp folder: {found[:5]}")


def invalid_face(path: str) -> str:
    """build.prj with the texture coordinates of one face - the middle one of EB_Faces - all set to its first corner's:
    an invalid face that is no degenerate one (its corners stay where they are)."""
    with open(path, "rb") as f:
        data = bytearray(f.read())
    at = 0
    while at + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, at)
        if cid & 0x7FFFFFFF == 3:
            n = size // 44
            face = at + 8 + (n // 2) * 44
            u, v = struct.unpack_from("<ff", data, face + 12)
            struct.pack_into("<ffff", data, face + 20, u, v, u, v)
            with open(path, "wb") as f:
                f.write(bytes(data))
            return f"face {n // 2} of {n}: its texture coordinates all ({u}, {v})"
        at += 8 + size
    raise RuntimeError(f"{path}: no faces chunk")


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
                if scene in NO_DETAILS and n in ("build.details", "build_details.dds"):
                    continue
                if os.path.isfile(os.path.join(level, n)):
                    shutil.copy2(os.path.join(level, n), os.path.join(dst, n))
            if scene in EMPTIED:
                empty_chunks(os.path.join(dst, "build.prj"), EMPTIED[scene])
                log(f"  {scene}: build.prj chunks {EMPTIED[scene]} emptied")
            if scene in EDITS:
                what = {"invalid_face": invalid_face}[EDITS[scene]](os.path.join(dst, "build.prj"))
                log(f"  {scene}: build.prj edited, {what}")
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
        no_programs(work)
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


# Where the C++ compilers' stages go: beside the outputs, on the volume of Bin, never under the temp folder.
STAGES = os.path.join(OUT, "stages")


def link_binaries(src: str, bin_dir: str, extra: tuple[str, ...] = ()) -> None:
    """The compilers and their libraries of `src` in the stage's bin folder as hard links: the same files, no byte
    of a program written anywhere. XrCore of cpp-compilers-last takes no fs config on its command line - it reads
    fsgame.ltx three folders above its own executable (LocatorAPI.cpp setup_fs_ltx; Core._initialize gets no
    fs_fname from xrLC.cpp or xrDO_Light.cpp) - so the stage's config is reached through the link's path."""
    for n in os.listdir(src):
        if n.lower().endswith((".exe", ".dll") + extra):
            os.link(os.path.join(src, n), os.path.join(bin_dir, n))


def run_cpp(scene: str, input_dir: str, dst: str, details_only: bool = False) -> dict:
    os.makedirs(STAGES, exist_ok=True)
    stage = tempfile.mkdtemp(prefix="gzlc_cpp_", dir=STAGES)
    try:
        bin_dir = os.path.join(stage, "bin", "x64", "Release")
        os.makedirs(bin_dir)
        link_binaries(CPP_BIN, bin_dir)
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
            log(f"  C++ {exe} -f {source_of(scene)} -silent {' '.join(CPP_FLAGS + flags)}")
            code, w, wall = run_limited([os.path.join(bin_dir, exe), "-f", source_of(scene), "-silent", "-flushlog",
                                         *CPP_FLAGS, *flags],
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
        if scene in REFUSED:
            # the refusal is the record: XrLC must end through its Debug.fatal and nothing else
            r = rec[tag]["runs"].get("XrLC.exe", {})
            if r.get("exit") == 3 and any("* FATAL:" in l for _, l in r.get("lines", [])):
                bad = []
            else:
                raise RuntimeError(f"{scene}: XrLC was to refuse the input and {ended(r.get('exit'), LIMIT)}"
                                   if r.get("exit") else f"{scene}: XrLC compiled an input it was to refuse")
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
        no_programs(work)
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
        no_programs(home)
        qa_target.remove_tree(home)
    return rec


def settle(limit: float = 60.0) -> float:
    """Waits until the machine is quiet - under a tenth of its cores busy for half a second, `limit` at most - before
    the Rust compile whose phases are judged against the C++ ones: the machine is shared with builds."""
    def times() -> tuple[int, int]:
        idle, kernel, user = wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
        k32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user))
        ticks = lambda ft: (ft.dwHighDateTime << 32) | ft.dwLowDateTime  # noqa: E731
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
    if scene == ONE_THREAD or (every and scene == ONE_THREAD_ALL):
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


def rerecord(scene: str) -> int:
    """The C++ compilers of CPP_BIN once more on the record's own input; every output file against the record's
    two runs: equal to one of them byte for byte, or within what those two differ in (compare_compile's rules,
    cpp1 against the new run, judged against cpp1 against cpp2)."""
    import compare_compile
    rd = record_dir(scene, key_of(scene))
    with open(os.path.join(rd, "record.json"), encoding="utf-8") as f:
        rec = json.load(f)
    out = os.path.join(OUT, scene, "rerecord")
    qa_target.remove_tree(out)
    log(f"{scene}: the C++ side again from the record's input, {CPP_BIN}")
    new = run_cpp(scene, os.path.join(rd, "input"), out, scene in DETAILS_ONLY)
    if any(r["exit"] != 0 for r in new["runs"].values()) and scene not in REFUSED:
        log(f"! {scene}: the C++ run failed: {[(e, r['exit']) for e, r in new['runs'].items()]}")
        return 1
    same, other = [], []
    for n, meta in sorted(new["files"].items()):
        if meta["sha256"] in (rec["cpp1"]["files"].get(n, {}).get("sha256"), rec["cpp2"]["files"].get(n, {}).get("sha256")):
            same.append(n)
        else:
            other.append(n)
    missing = sorted(set(rec["cpp1"]["files"]) - set(new["files"]))
    log(f"  byte for byte equal to a run of the record: {len(same)} files; other: {other}; missing: {missing}")
    floor = compare_compile.compare_dirs(os.path.join(rd, "cpp1"), os.path.join(rd, "cpp2"), set(rec["cpp1"]["files"]),
                                         set(rec["cpp2"]["files"]), None)
    res = compare_compile.compare_dirs(os.path.join(rd, "cpp1"), out, set(rec["cpp1"]["files"]), set(new["files"]),
                                       None)
    judged = compare_compile.judge(res, floor)
    for p in res["problems"] + judged:
        log("  ! " + p)
    log(f"  rules: {json.dumps(res['rules'])}")
    log(f"  light: {json.dumps(res['light'].get('lightmaps', {}))[:800]}  vertex: {json.dumps(res['vertex'])}")
    for exe, r in new["runs"].items():
        log(f"  {exe}: {r['wall']:.1f} s wall, {r['cpu'][-1][1] if r['cpu'] else 0:.1f} s cpu")
    ok = not missing and not res["problems"] and not judged
    log(f"{scene}: the rebuilt oracle {'changes no result' if ok else 'CHANGES A RESULT'}")
    return 0 if ok else 1


# ------------------------------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--scene", action="append")
    ap.add_argument("--refresh-oracle", action="store_true")
    ap.add_argument("--no-compare", action="store_true")
    ap.add_argument("--diagnose", metavar="SCENE", help="XrLC of --cpp-bin under a debugger: where it crashes")
    ap.add_argument("--cpp-bin", help="--diagnose: the diagnostic build (PDBs beside it)")
    ap.add_argument("--set", action="append", default=[], metavar="KEY=VALUE", help="--diagnose: a build option")
    ap.add_argument("--flag", action="append", default=[], help="--diagnose: an XrLC flag")
    ap.add_argument("--limit", type=float, default=LIMIT)
    ap.add_argument("--reuse-rust", action="store_true", help="compare the Rust output the last run left (rust.json "
                    "and rust/ of the scene), compiling nothing: a change of the rules alone; no determinism runs")
    ap.add_argument("--rerecord", metavar="SCENE", help="the C++ side again from the record's own input, into "
                    "Intermediate/compile/<scene>/rerecord, and compared with the record: a rebuilt oracle that "
                    "changes no result (the record is not touched)")
    args = ap.parse_args()
    if args.rerecord:
        return rerecord(args.rerecord)
    if args.diagnose:
        return diagnose(args.diagnose, dict(s.split("=", 1) for s in args.set), args.flag, args.cpp_bin, args.limit)
    import compare_compile
    state_path = os.path.join(OUT, "state.json")
    state = json.load(open(state_path, encoding="utf-8")) if os.path.isfile(state_path) else {}
    scenes = args.scene or (SCENES if args.all else QUICK)
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
        reused = args.reuse_rust and os.path.isfile(os.path.join(OUT, scene, "rust.json"))
        if reused:
            log(f"{scene}: the Rust output of the last run, compared again")
            with open(os.path.join(OUT, scene, "rust.json"), encoding="utf-8") as f:
                rrec = json.load(f)
        else:
            waited = settle()
            log(f"{scene}: the Rust compile" + (f" (waited {waited:.0f} s for a quiet machine)" if waited > 1 else ""))
            rrec = run_rust(scene, os.path.join(rd, "input"), rust_dir, level=scene not in DETAILS_ONLY)
            with open(os.path.join(OUT, scene, "rust.json"), "w", encoding="utf-8") as f:
                json.dump(rrec, f, indent=1)
        ok = all(r["exit"] == 0 for r in rrec["runs"].values())
        if scene in REFUSED:
            ok = compare_compile.compare_refused(rd, rec, rust_dir, rrec, os.path.join(OUT, scene), log)
        elif ok and not args.no_compare:
            ok = compare_compile.compare(scene, rd, rec, rust_dir, rrec, os.path.join(OUT, scene), log)
        if all(r["exit"] == 0 for r in rrec["runs"].values()) and not reused:
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
