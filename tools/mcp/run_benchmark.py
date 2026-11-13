#!/usr/bin/env python3
"""benchmark_scene_load.py against the Rust editor, held to the C++ editor's numbers.

The C++ numbers are measured once (Intermediate/qa/benchmark_cpp.json) and only when nothing answers
on 127.0.0.1:28016 - an editor there is the author's and is never touched; the C++ editor's own files
(_projects.ini, level.ini, level_imgui.ini) come back byte for byte. Both editors load the Escape scene
of a copy of D:\\XFinedProjects\\Test. FAIL when the Rust median load or ready time is above the C++
one.

    python tools/mcp/run_benchmark.py [--repeats 3] [--again]
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

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "tools", "qa"))
import kept  # noqa: E402
import qa_target  # noqa: E402

SOURCE = r"D:\XFinedProjects\Test"
SCENE = "l01_escape.level"
CPP_EXE = os.path.join(REPO, "Bin", "x64", "CppEditor", "LevelEditor.exe")
CPP_PORT = 28016
CPP_KEPT = [os.path.join(REPO, n) for n in ("_projects.ini", "level.ini", "level_imgui.ini")]
RECORD = os.path.join(REPO, "Intermediate", "qa", "benchmark_cpp.json")
BENCH = os.path.join(HERE, "benchmark_scene_load.py")


def answers(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5) as s:
            s.sendall(b'{"cmd":"ping"}\n')
            return b'"ok"' in s.recv(4096)
    except OSError:
        return False


def call(port: int, payload: dict, timeout: float = 30.0) -> dict:
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as s:
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            c = s.recv(1 << 20)
            if not c:
                break
            buf += c
    return json.loads(buf.decode("utf-8", "replace") or "{}")


def start(cmd: list[str], cwd: str, port: int, limit: float = 120.0) -> subprocess.Popen:
    info = subprocess.STARTUPINFO()
    info.dwFlags = subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 7
    proc = subprocess.Popen(cmd, cwd=cwd, startupinfo=info)
    end = time.monotonic() + limit
    while time.monotonic() < end:
        if answers(port):
            return proc
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup (%s)" % proc.returncode)
        time.sleep(0.05)
    proc.kill()
    raise RuntimeError("the editor never answered")


def stop(proc: subprocess.Popen, port: int) -> None:
    try:
        call(port, {"cmd": "app.quit", "discard": True}, timeout=10)
    except OSError:
        pass
    try:
        proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        print("! the editor did not leave: ended by force (pid %d)" % proc.pid, flush=True)
        proc.kill()
        proc.wait(timeout=30)


def bench(port: int, repeats: int) -> dict:
    out = subprocess.run([sys.executable, BENCH, SCENE, "--port", str(port), "--repeats", str(repeats),
                          "--warmup", "1", "--timeout", "600"], capture_output=True, text=True, timeout=1200)
    print(out.stdout.strip(), flush=True)
    if out.returncode:
        raise RuntimeError("the benchmark failed: " + out.stderr.strip()[-400:])
    text = out.stdout
    return json.loads(text[text.index("{"):])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repeats", type=int, default=3)
    ap.add_argument("--again", action="store_true", help="measure the C++ editor again")
    args = ap.parse_args()
    work = tempfile.mkdtemp(prefix="grizzly_bench_")
    ok = True
    try:
        if args.again or not os.path.isfile(RECORD):
            if not os.path.isfile(CPP_EXE):
                print("FAIL no C++ record (%s) and no C++ editor to make one: %s is not there" % (RECORD, CPP_EXE),
                      flush=True)
                return 1
            if answers(CPP_PORT):
                print("FAIL something answers on 127.0.0.1:%d (the author's editor): no C++ numbers to compare with"
                      % CPP_PORT, flush=True)
                return 1
            cpp_copy = qa_target.copy_project(SOURCE, os.path.join(work, "cpp", "Test"))
            # the root's files of the author, also on disk while this run has them aside (kept.py)
            kept.recover()
            saved = {p: (open(p, "rb").read() if os.path.isfile(p) else None) for p in CPP_KEPT}
            kept.to_disk({os.path.basename(p): data for p, data in saved.items()})
            try:
                qa_target.guard_project(cpp_copy)
                proc = start([CPP_EXE, "-nodlg", "-project", cpp_copy], os.path.dirname(CPP_EXE), CPP_PORT, 300)
                try:
                    cpp = bench(CPP_PORT, args.repeats)
                finally:
                    stop(proc, CPP_PORT)
            finally:
                for p, data in saved.items():
                    if data is None:
                        if os.path.isfile(p):
                            os.remove(p)
                    else:
                        with open(p, "wb") as f:
                            f.write(data)
                kept.done()
            os.makedirs(os.path.dirname(RECORD), exist_ok=True)
            with open(RECORD, "w", encoding="utf-8") as f:
                json.dump(cpp, f, indent=1)
        with open(RECORD, encoding="utf-8") as f:
            cpp = json.load(f)
        qa_target.mark("C++ numbers")
        rust_copy = qa_target.copy_project(SOURCE, os.path.join(work, "rust", "Test"))
        exe = qa_target.exe()
        qa_target.guard_project(rust_copy)
        proc = start([exe, *qa_target.ARGS, "-nodlg", "-project", rust_copy], os.path.dirname(exe), qa_target.PORT)
        qa_target.mark("editor start")
        try:
            rust = bench(qa_target.PORT, args.repeats)
        finally:
            stop(proc, qa_target.PORT)
        for what in ("load", "ready"):
            c, r = cpp[what]["median_ms"], rust[what]["median_ms"]
            good = r <= c
            ok = ok and good
            print("%s %s median: Rust %.1f ms, C++ %.1f ms" % ("PASS" if good else "FAIL", what, r, c), flush=True)
        good = rust.get("object_total") == cpp.get("object_total")
        ok = ok and good
        print("%s both load %s objects (C++ %s)" % ("PASS" if good else "FAIL", rust.get("object_total"),
                                                 cpp.get("object_total")), flush=True)
    except Exception as e:  # noqa: BLE001
        print("FAIL the benchmark ran: %r" % e, flush=True)
        ok = False
    finally:
        qa_target.remove_tree(work)
    print("BENCHMARK: %s" % ("DONE" if ok else "FAILED"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
