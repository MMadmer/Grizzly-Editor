#!/usr/bin/env python3
"""A build runs beside the editor.

Over MCP on a THROW-AWAY COPY of a project, with a gigabyte of payload so that a build takes long enough to look at:

  * while a build copies, the editor answers at once and the Task Center names the job, its steps and how far it is;
  * a second build is refused while one runs;
  * Stop leaves the last build exactly as it was, and no scratch folder beside it;
  * a build reuses what the last one already holds (the very same file on disk) and takes what changed;
  * a module the game holds open is refused with its name, and stays whole;
  * a packaging runs the same way and answers what mod_package always answered.

    python tools/qa/test_build_job.py --project <project> [--shots <folder>]

The target and the release folder are inside the copy: nothing reaches the linked game.
"""
from __future__ import annotations

import argparse
import base64
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
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = ("127.0.0.1", 28016)
PAYLOAD_FILES = 24
PAYLOAD_MB = 48

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 300.0, **extra) -> dict:
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def start_editor(project: str) -> subprocess.Popen:
    exe = os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 7
    proc = subprocess.Popen([exe, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
    deadline = time.monotonic() + 240
    while time.monotonic() < deadline:
        try:
            if mcp("mod_manifest", timeout=30).get("ok"):
                return proc
        except OSError:
            pass
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup")
        time.sleep(1)
    raise RuntimeError("the editor never opened the project")


def fingerprint(root: str) -> dict[str, str]:
    out = {}
    for base, _, files in os.walk(root):
        for name in files:
            path = os.path.join(base, name)
            with open(path, "rb") as f:
                out[os.path.relpath(path, root)] = hashlib.md5(f.read(1 << 20)).hexdigest() + str(os.path.getsize(path))
    return out


class Job(threading.Thread):
    """A command that answers when its job has ended."""

    def __init__(self, cmd: str, **extra) -> None:
        super().__init__(daemon=True)
        self.cmd, self.extra, self.answer = cmd, extra, {}
        self.start()

    def run(self) -> None:
        self.answer = mcp(self.cmd, timeout=590.0, **self.extra)


def job_row() -> dict:
    rows = mcp("progress").get("background") or []
    return next((r for r in rows if r.get("key")), {})


def wait_for_copy(job: Job) -> dict:
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline and job.is_alive():
        row = job_row()
        if row.get("determinate") and row.get("step") == 1 and row.get("fraction", 0) > 0:
            return row
        time.sleep(.02)
    return {}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True)
    parser.add_argument("--shots")
    args = parser.parse_args()
    source = os.path.abspath(args.project)
    work = tempfile.mkdtemp(prefix="grizzly_job_", dir=os.path.splitdrive(source)[0] + os.sep)
    project = os.path.join(work, "project")
    game = os.path.join(work, "game")
    release = os.path.join(work, "release")
    proc = None
    try:
        shutil.copytree(source, project, ignore=shutil.ignore_patterns(".git", "releases", "temp", "logs", "backup", "preview.png"))
        payload = os.path.join(project, "gamedata", "textures", "qa_job")
        os.makedirs(payload)
        block = os.urandom(PAYLOAD_MB << 20)
        for i in range(PAYLOAD_FILES):
            with open(os.path.join(payload, f"p_{i:02}.dds"), "wb") as f:
                f.write(i.to_bytes(4, "little") + block)
        proc = start_editor(project)
        module_id = mcp("mod_manifest").get("id")
        module = os.path.join(game, "modules", module_id)
        scratch = os.path.join(game, "modules", ".new")
        target = game.replace("\\", "/")

        # ---- the editor goes on while a build copies ------------------------------------------------------------------
        job = Job("mod_export", target=target)
        row = wait_for_copy(job)
        check(bool(row), "A the Task Center follows the build's copying", json.dumps(job_row())[:300])
        check("Check, Copy, Install" == row.get("steps"), "A it names the steps", json.dumps(row)[:300])
        check(" of " in row.get("numbers", "") and "files" in row.get("numbers", ""), "A it says how far in bytes and files", json.dumps(row)[:300])
        refused = mcp("mod_export", target=target) if job.is_alive() else {"error": "the build ended first"}
        check("running" in refused.get("error", ""), "A a second build is refused while one runs", json.dumps(refused)[:300])
        worst = 0.0
        copying = 0
        while job.is_alive() and copying < 60:
            began = time.monotonic()
            mcp("ping", timeout=30)
            worst = max(worst, time.monotonic() - began)
            copying += 1
            time.sleep(.05)
        check(worst < .75, f"A a command is answered at once while it copies ({worst * 1000:.0f} ms at worst, {copying} asked)")
        if job.is_alive() and args.shots:
            mcp("ui_popup", show="tasks")
            pic = mcp("screenshot_editor")
            if pic.get("png_base64"):
                os.makedirs(args.shots, exist_ok=True)
                with open(os.path.join(args.shots, "job_card.png"), "wb") as f:
                    f.write(base64.b64decode(pic["png_base64"]))
        job.join()
        check(job.answer.get("ok") and job.answer.get("files", 0) >= PAYLOAD_FILES, "A the build answers when it has ended", json.dumps(job.answer)[:300])
        check(not os.path.exists(scratch), "A it leaves no scratch folder beside the module")
        before = fingerprint(module)

        # ---- Stop ---------------------------------------------------------------------------------------------------
        changed = os.path.join(payload, "p_00.dds")
        for i in range(PAYLOAD_FILES):
            with open(os.path.join(payload, f"p_{i:02}.dds"), "r+b") as f:
                f.write((i + 1000).to_bytes(4, "little"))
        job = Job("mod_export", target=target)
        check(bool(wait_for_copy(job)), "B the changed files are being copied")
        stopped = mcp("progress", action="cancel")
        job.join()
        check(stopped.get("ok") and job.answer.get("error") == "cancelled", "B Stop ends the build", json.dumps(job.answer)[:300])
        check(fingerprint(module) == before, "B the last build is exactly as it was")
        check(not os.path.exists(scratch), "B and nothing is left beside it")

        # ---- what the last build holds is not written again ----------------------------------------------------------
        with open(changed, "r+b") as f:
            f.write((0).to_bytes(4, "little"))
        for i in range(1, PAYLOAD_FILES):
            with open(os.path.join(payload, f"p_{i:02}.dds"), "r+b") as f:
                f.write(i.to_bytes(4, "little"))
        built = mcp("mod_export", target=target, timeout=590.0)
        check(built.get("ok"), "C a build after that goes through", json.dumps(built)[:300])
        again_ids = {n: os.stat(os.path.join(module, n)).st_ino for n in fingerprint(module) if n.endswith(".dds")}
        with open(os.path.join(payload, "p_05.dds"), "r+b") as f:
            f.write(b"NEW!")
        began = time.monotonic()
        built = mcp("mod_export", target=target, timeout=590.0)
        spent = time.monotonic() - began
        rel = os.path.join("gamedata", "textures", "qa_job", "p_05.dds")
        now = {n: os.stat(os.path.join(module, n)).st_ino for n in again_ids}
        same = sum(1 for n in again_ids if n != rel and now[n] == again_ids[n])
        check(built.get("ok") and same == len(again_ids) - 1, f"C it reuses the {same} unchanged files ({spent:.1f} s)", json.dumps(built)[:200])
        with open(os.path.join(module, rel), "rb") as f:
            check(f.read(4) == b"NEW!", "C and takes the changed one")

        # ---- a module the game holds ----------------------------------------------------------------------------------
        before = fingerprint(module)
        held = open(os.path.join(module, rel), "rb")
        try:
            refused = mcp("mod_export", target=target, timeout=590.0)
        finally:
            held.close()
        check("in use" in refused.get("error", ""), "D a module in use is refused and named", json.dumps(refused)[:300])
        check(fingerprint(module) == before and not os.path.exists(scratch), "D and it stays whole")

        # ---- a packaging --------------------------------------------------------------------------------------------
        for i in range(2, PAYLOAD_FILES):
            os.remove(os.path.join(payload, f"p_{i:02}.dds"))
        job = Job("mod_package", out_dir=release.replace("\\", "/"), requires_game="")
        seen_pack = False
        worst = 0.0
        while job.is_alive():
            began = time.monotonic()
            row = job_row()
            worst = max(worst, time.monotonic() - began)
            seen_pack |= row.get("step") == 2 and "Pack" in row.get("steps", "")
            time.sleep(.05)
        job.join()
        check(job.answer.get("ok") and job.answer.get("packages"), "E the packaging answers as it always did", json.dumps(job.answer)[:300])
        check(seen_pack, "E the Task Center follows it to its packing step")
        check(worst < .75, f"E a command is answered at once while it packs ({worst * 1000:.0f} ms at worst)")
        names = os.listdir(os.path.join(project, "temp")) if os.path.isdir(os.path.join(project, "temp")) else []
        check(not [n for n in names if n.startswith("release_stage_")], "E its stage is gone", str(names))
    finally:
        if proc:
            try:
                mcp("window_control", action="close")
                proc.wait(timeout=120)
            except (OSError, subprocess.TimeoutExpired) as error:
                print("! the editor did not close:", error, flush=True)
        shutil.rmtree(work, ignore_errors=True)
    failed = [text for ok, text in results if not ok]
    print(f"{len(results) - len(failed)}/{len(results)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
