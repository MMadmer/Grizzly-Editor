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
into the twin's stream, closed over MCP it ends the twin with the editor's exit code 0, and a
headless editor whose twin is ended by force is gone with it; an editor with a window outlives
its twin (no console can end it), started minimised because the twin was.

Then the streams and the console (`streams`, `console`): a dump whose reader leaves after one
line and a headless editor whose log echo lost its reader end with 0 and leave no crash
report; in a console of its own (hidden), Ctrl+Break closes a headless editor started through
the twin the way a quit does, also while it opens its project (exit code 0, "Grizzly Editor
closed" in its log), and an editor
with a window started there with -log_echo is not in that console and lives on after it.

    python tools/mcp/run_lifecycle.py
"""
from __future__ import annotations

import ctypes
import glob
import json
import os
import shutil
import signal
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
                    s.sendall(b'{"cmd":"app.quit","discard":true}\n')
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
            s.sendall(b'{"cmd":"app.quit","discard":true}\n')
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
        check(up and gone, "a headless editor whose twin is ended by force is gone with it",
              "up %s, gone %s" % (up, gone))
    finally:
        if answers(port):
            close_over_mcp(port)
            wait_until(lambda: not answers(port), 15)

    # an editor with a window is the editor as Explorer starts it: the twin's end is not its end. The twin is
    # started minimised and passes that on, so the window stays off the author's screen
    proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            startupinfo=minimised())
    try:
        up = wait_until(lambda: answers(port), 60)
        proc.kill()
        proc.wait(timeout=30)
        time.sleep(2.0)
        alive = answers(port)
        check(up and alive, "an editor with a window outlives its twin", "up %s, alive %s" % (up, alive))
    finally:
        if proc.poll() is None:
            proc.kill()
        if answers(port):
            close_over_mcp(port)
            gone = wait_until(lambda: not answers(port), 30)
            check(gone, "that editor closes over MCP")
    # the startup profile of that run, whole in the log once the editor has closed, says how the window came up
    time.sleep(1.0)
    marks = ""
    for path in glob.glob(qa_target.log_glob()):
        with open(path, encoding="utf-8", errors="replace") as f:
            marks = " | ".join(line.strip() for line in f if "window minimised" in line or "window shown" in line)
    check("window minimised" in marks and "window shown" not in marks,
          "the twin passes a minimised start on to the editor", marks[:200] or "no mark in the log")
    return good


def minimised() -> subprocess.STARTUPINFO:
    info = subprocess.STARTUPINFO()
    info.dwFlags = subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 7
    return info


def ask(port: int, request: dict) -> dict:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=10) as s:
            s.sendall(json.dumps(request).encode() + b"\n")
            data = b""
            while not data.endswith(b"\n"):
                chunk = s.recv(65536)
                if not chunk:
                    break
                data += chunk
        return json.loads(data.decode("utf-8", "replace") or "{}")
    except (OSError, ValueError):
        return {}


def crash_reports() -> list[str]:
    return glob.glob(os.path.join(qa_target.HOME, "logs", "*crash*"))


def streams() -> bool:
    """A reader that leaves is no error of the editor's: it stops printing and ends as it would have."""
    exe, port = qa_target.exe(), qa_target.PORT
    good = True

    def check(ok: bool, what: str, detail: str = "") -> None:
        nonlocal good
        good = good and ok
        print(("PASS " if ok else "FAIL ") + "streams: " + what + ((" - " + detail) if detail and not ok else ""),
              flush=True)

    proc = subprocess.Popen([exe, *qa_target.ARGS, "-dump_commands"], stdout=subprocess.PIPE)
    first = proc.stdout.readline()
    proc.stdout.close()
    try:
        code = proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        proc.kill()
        code = None
    check(bool(first.strip()) and code == 0 and not crash_reports(),
          "a dump whose reader leaves after one line ends with 0 and no crash report",
          "first %r, exit %s, reports %s" % (first, code, crash_reports()))

    proc = subprocess.Popen([exe, *qa_target.ARGS, "-headless", "-nodlg", "-log_echo"], stdout=subprocess.DEVNULL,
                            stderr=subprocess.PIPE)
    try:
        up = wait_until(lambda: answers(port), 60)
        echoed = proc.stderr.readline()
        proc.stderr.close()
        # the editor goes on logging into a stream nobody reads, and closes as it would have
        pings = [ask(port, {"cmd": "project.list"}).get("ok") for _ in range(3)]
        close_over_mcp(port)
        try:
            code = proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            code = None
        check(up and bool(echoed.strip()) and all(pings) and code == 0 and not crash_reports(),
              "a headless editor whose log echo lost its reader answers on and ends with 0, no crash report",
              "up %s, echoed %r, answers %s, exit %s, reports %s" % (up, echoed[:60], pings, code, crash_reports()))
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=30)
    return good


def console_inner(out: str, project: str) -> int:
    """In a console of its own: Ctrl+Break on a headless editor started through the twin, and an editor with a
    window started here with -log_echo. Writes what it saw to `out`; the windowed editor is left running."""
    port = qa_target.PORT
    twin_exe = os.path.join(os.path.dirname(qa_target.exe()), "Grizzly-Cmd.exe")
    seen: dict = {}
    # with a project to open: the endpoint answers before the project is open, and the break right then finds
    # the editor opening it
    qa_target.guard_project(project)
    proc = subprocess.Popen([twin_exe, *qa_target.ARGS, "-headless", "-nodlg", "-project", project],
                            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
    seen["up"] = wait_until(lambda: answers(port), 60)
    proc.send_signal(signal.CTRL_BREAK_EVENT)
    try:
        seen["exit"] = proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        seen["exit"] = None
        proc.kill()
    seen["gone"] = wait_until(lambda: not answers(port), 15)
    tail = ""
    for path in glob.glob(qa_target.log_glob()):
        with open(path, encoding="utf-8", errors="replace") as f:
            tail = "".join(f.readlines()[-3:])
    seen["closed"] = "Grizzly Editor closed" in tail

    # an editor with a window, typed into this console with -log_echo: it joins no console
    window = subprocess.Popen([qa_target.exe(), *qa_target.ARGS, "-nodlg", "-log_echo"], startupinfo=minimised())
    seen["window_up"] = wait_until(lambda: answers(port), 60)
    pids = (ctypes.c_uint32 * 64)()
    count = ctypes.windll.kernel32.GetConsoleProcessList(pids, 64)
    seen["console"] = [int(x) for x in pids[:count]]
    seen["window_pid"] = window.pid
    with open(out, "w", encoding="utf-8") as f:
        json.dump(seen, f)
    return 0


def console(work: str) -> bool:
    """What a console's end does to an editor in it: a headless one closes properly, one with a window is not
    in the console at all."""
    port = qa_target.PORT
    good = True

    def check(ok: bool, what: str, detail: str = "") -> None:
        nonlocal good
        good = good and ok
        print(("PASS " if ok else "FAIL ") + "console: " + what + ((" - " + detail) if detail and not ok else ""),
              flush=True)

    out = os.path.join(work, "console.json")
    hidden = subprocess.STARTUPINFO()
    hidden.dwFlags = subprocess.STARTF_USESHOWWINDOW
    hidden.wShowWindow = 0
    env = dict(os.environ, GRIZZLY_QA_PORT=str(port), GRIZZLY_QA_HOME=qa_target.HOME)
    try:
        project = project_copy(work, "console")
        subprocess.run([sys.executable, os.path.abspath(__file__), "--console-inner", out, project], env=env,
                       creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=hidden, timeout=240)
        seen = {}
        if os.path.isfile(out):
            with open(out, encoding="utf-8") as f:
                seen = json.load(f)
        check(seen.get("up") and seen.get("exit") == 0 and seen.get("gone") and seen.get("closed"),
              "Ctrl+Break in its console closes a headless editor started through the twin the way a quit does",
              json.dumps(seen)[:300])
        # that console is gone now (its only program, the inner run, ended)
        time.sleep(2.0)
        alive = answers(port)
        check(bool(seen.get("window_up")) and seen.get("window_pid") not in (seen.get("console") or [None])
              and alive,
              "an editor with a window started in a console with -log_echo is not in it and lives on after it",
              "alive %s, %s" % (alive, json.dumps(seen)[:300]))
    finally:
        if answers(port):
            close_over_mcp(port)
            gone = wait_until(lambda: not answers(port), 30)
            check(gone, "that editor closes over MCP")
    return good


def main() -> int:
    if len(sys.argv) == 4 and sys.argv[1] == "--console-inner":
        return console_inner(sys.argv[2], sys.argv[3])
    if answers(qa_target.PORT):
        print(qa_target.busy_text())
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_lifecycle_")
    try:
        ok = run(work, "lifecycle", {"GRIZZLY_MCP_REQUEST_TIMEOUT_MS": "250"},
                 ["--timeout-scene", "l01_escape.level", "--shutdown-via-palette"])
        ok = twin(work) and ok
        ok = streams() and ok
        ok = console(work) and ok
    finally:
        qa_target.remove_tree(work)
    print("LIFECYCLE: %s" % ("DONE" if ok else "FAILED"), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
