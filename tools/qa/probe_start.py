#!/usr/bin/env python3
"""Starts the Rust editor windowed (shown, never activated) a few times and reads the start-up
profile of each start from its log: when the window was shown and when the first frame was on the
screen, in seconds after the process was created.

    python tools/qa/probe_start.py [--starts 4] [--limit 0.3]

Every start has a home of its own in the system temp folder and opens no project. Exit code 1 when a
warm start (every start but the first) showed its window later than --limit.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

EXE = os.path.join(qa_target.REPO, "Intermediate", "cargo", "debug", "Grizzly.exe")
PORT = 28961


def call(cmd: str, timeout: float = 30.0, **args) -> dict:
    with socket.create_connection(("127.0.0.1", PORT), timeout=5) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps({"cmd": cmd, **args}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8", "replace") or "{}")


PROFILE: list[str] = []


def milestones(home: str) -> dict[str, float]:
    for _ in range(100):
        for path in glob.glob(os.path.join(home, "logs", "grizzly_*.log")):
            text = open(path, encoding="utf-8", errors="replace").read()
            m = re.search(r"startup after the process was created: (.*)", text)
            if m:
                PROFILE[:] = [line for line in text.splitlines() if " reads " in line or "startup" in line]
                return {k.strip(): float(v) for k, v in re.findall(r"([a-z ]+?) ([0-9.]+) s", m.group(1))}
        time.sleep(0.05)
    return {}


def start_once(home: str) -> tuple[dict[str, float], str]:
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    # shown without taking the foreground: a minimised start shows no window ('window minimised')
    startup.wShowWindow = 4  # SW_SHOWNOACTIVATE
    proc = subprocess.Popen([EXE, "-mcp_port", str(PORT), "-home", home, "-nodlg"], cwd=os.path.dirname(EXE),
                            startupinfo=startup, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                return {}, f"the editor exited ({proc.returncode})"
            try:
                if call("ping", timeout=5).get("ok"):
                    break
            except OSError:
                time.sleep(0.05)
        shot = call("shot.window", timeout=30)
        marks = milestones(home)
        note = "" if shot.get("ok") else "screenshot: " + json.dumps(shot)[:200]
        return marks, note
    finally:
        try:
            call("app.quit", timeout=10, discard=True)
        except OSError:
            pass
        try:
            proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            print(f"! the editor did not leave: ended by force (pid {proc.pid})", flush=True)
            proc.kill()
            proc.wait(timeout=30)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--starts", type=int, default=4)
    ap.add_argument("--limit", type=float, default=0.3)
    ap.add_argument("--profile", action="store_true", help="print each start's profile lines")
    args = ap.parse_args()
    late = []
    for i in range(args.starts):
        home = tempfile.mkdtemp(prefix="grizzly_probe_")
        try:
            marks, note = start_once(home)
            logs = glob.glob(os.path.join(home, "logs", "grizzly_*.log"))
            if not marks and logs:
                print(open(logs[0], encoding="utf-8", errors="replace").read()[-3000:])
        finally:
            qa_target.remove_tree(home)
        shown, first = marks.get("window shown"), marks.get("first frame")
        print(f"START {i}: window shown {shown} s, first frame {first} s {note}", flush=True)
        if args.profile:
            for line in PROFILE:
                print("    " + line, flush=True)
        if i > 0 and (shown is None or shown > args.limit):
            late.append(i)
    if late:
        print(f"FAIL: warm starts {late} showed the window later than {args.limit} s")
        return 1
    print(f"PASS: every warm start showed its window within {args.limit} s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
