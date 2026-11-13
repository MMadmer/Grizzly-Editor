#!/usr/bin/env python3
"""test_server_lifecycle.py against an editor of its own (the Rust editor through qa_target).

One editor on a copy of D:\\XFinedProjects\\Test that opens no scene by itself, with
GRIZZLY_MCP_REQUEST_TIMEOUT_MS=250: the concurrent clients, the disconnects, the JSON arguments,
the overflow, a scene load that times out, and the shutdown with idle clients through the palette
(COMMAND_EXIT). The script's other shutdown runs exec_command COMMAND_QUIT, a plumbing row the
Rust editor does not list (D15; docs/rust/STATE.md, "What the suites could not hold the Rust
editor to"). The editor must be gone within the script's timeout.

Then the editor's twin for a command line, Grizzly-Cmd.exe beside the editor (a console program
that runs the window program and waits for it): a headless editor started through it prints
into the twin's stream, closed over MCP it ends the twin with the editor's exit code 0, and an
editor whose twin is ended (a closed console, Ctrl+C) is gone with it.

    python tools/mcp/run_lifecycle.py
"""
from __future__ import annotations

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
import qa_target  # noqa: E402

SOURCE = r"D:\XFinedProjects\Test"
SCRIPT = os.path.join(HERE, "test_server_lifecycle.py")


def answers(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5) as s:
            s.sendall(b'{"cmd":"ping"}\n')
            return b'"ok"' in s.recv(4096)
    except OSError:
        return False


def project_copy(work: str, name: str) -> str:
    dst = qa_target.copy_project(SOURCE, os.path.join(work, name, "Test"))
    pl = os.path.join(dst, "project.ltx")
    with open(pl, encoding="utf-8", errors="replace") as f:
        lines = [l for l in f.read().splitlines() if not l.strip().lower().startswith("last_scene")]
    with open(pl, "w", encoding="utf-8", newline="\r\n") as f:
        f.write("\n".join(lines) + "\n")
    return dst


def run(work: str, name: str, extra_env: dict, script_args: list[str]) -> bool:
    project = project_copy(work, name)
    env = dict(os.environ, **extra_env)
    info = subprocess.STARTUPINFO()
    info.dwFlags = subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 7
    exe = qa_target.exe()
    qa_target.guard_project(project)
    proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe),
                            env=env, startupinfo=info)
    try:
        end = time.monotonic() + 60
        while time.monotonic() < end and not answers(qa_target.PORT):
            if proc.poll() is not None:
                print("FAIL %s: the editor exited during startup" % name, flush=True)
                return False
            time.sleep(0.05)
        qa_target.mark("editor start")
        out = subprocess.run([sys.executable, SCRIPT, "--port", str(qa_target.PORT), "--pid", str(proc.pid),
                              *script_args], capture_output=True, text=True, timeout=240)
        text = (out.stdout + out.stderr).strip()
        good = out.returncode == 0 and "lifecycle stress passed" in text
        print(("PASS " if good else "FAIL ") + "%s: %s" % (name, text.splitlines()[-1] if text else "no output"),
              flush=True)
        if not good:
            print(text[-2000:], flush=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            good = False
            print("FAIL %s: the editor is still running after the shutdown" % name, flush=True)
        return good
    finally:
        if proc.poll() is None:
            try:
                with socket.create_connection(("127.0.0.1", qa_target.PORT), timeout=3) as s:
                    s.sendall(b'{"cmd":"window_control","action":"close","discard":true}\n')
                    s.recv(4096)
            except OSError:
                pass
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                print("! the editor did not leave: ended by force (pid %d)" % proc.pid, flush=True)
                proc.kill()
                proc.wait(timeout=30)


def close_over_mcp(port: int) -> None:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=3) as s:
            s.sendall(b'{"cmd":"window_control","action":"close","discard":true}\n')
            s.recv(4096)
    except OSError:
        pass


def wait_until(what, seconds: float) -> bool:
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if what():
            return True
        time.sleep(0.1)
    return bool(what())


def twin(work: str) -> bool:
    """Grizzly-Cmd.exe: the editor's output and exit code reach a shell, and the editor ends with it."""
    exe = os.path.join(os.path.dirname(qa_target.exe()), "Grizzly-Cmd.exe")
    if not os.path.isfile(exe):
        print("FAIL twin: %s is not built" % exe, flush=True)
        return False
    port = qa_target.PORT
    args = [exe, *qa_target.ARGS, "-headless", "-nodlg", "-log_echo"]
    good = True

    def check(ok: bool, what: str, detail: str = "") -> None:
        nonlocal good
        good = good and ok
        print(("PASS " if ok else "FAIL ") + "twin: " + what + ((" - " + detail) if detail and not ok else ""),
              flush=True)

    # closed over MCP: what the editor printed came through the twin, and so did its exit code
    out = os.path.join(work, "twin_out.txt")
    with open(out, "wb") as f:
        proc = subprocess.Popen(args, stdout=f, stderr=subprocess.STDOUT)
    try:
        up = wait_until(lambda: answers(port), 60)
        close_over_mcp(port)
        try:
            code = proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            code = None
        with open(out, encoding="utf-8", errors="replace") as f:
            text = f.read()
        check(up and code == 0 and "control endpoint" in text and "Grizzly Editor closed" in text,
              "a headless editor prints through the twin and ends it with exit code 0 when closed over MCP",
              "up %s, exit %s, output %r" % (up, code, text[-300:]))
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=30)

    # the twin ended the way a closed console ends it: the editor does not outlive it
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        up = wait_until(lambda: answers(port), 60)
        proc.kill()
        proc.wait(timeout=30)
        gone = wait_until(lambda: not answers(port), 15)
        check(up and gone, "an editor whose twin is ended is gone with it", "up %s, gone %s" % (up, gone))
    finally:
        if answers(port):
            close_over_mcp(port)
            wait_until(lambda: not answers(port), 15)
    return good


def main() -> int:
    if answers(qa_target.PORT):
        print(qa_target.busy_text())
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_lifecycle_")
    try:
        ok = run(work, "lifecycle", {"GRIZZLY_MCP_REQUEST_TIMEOUT_MS": "250"},
                 ["--timeout-scene", "l01_escape.level", "--shutdown-via-palette"])
        ok = twin(work) and ok
    finally:
        qa_target.remove_tree(work)
    print("LIFECYCLE: %s" % ("DONE" if ok else "FAILED"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
