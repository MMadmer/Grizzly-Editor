#!/usr/bin/env python3
"""A build runs beside the editor.

Over MCP on a THROW-AWAY COPY of a project, with a gigabyte of payload so that a build takes long enough to look at:

  * while a build copies, the editor answers at once and the Task Center names the job, its steps and how far it is;
  * a second build is refused while one runs;
  * Stop leaves the last build exactly as it was, and no scratch folder beside it;
  * a build reuses what the last one already holds (the very same file on disk) and takes what changed;
  * a module the game holds open is refused with its name, and stays whole;
  * a packaging runs the same way and answers what mod_package always answered;
  * the UI's frames keep their own time while a build runs (frame_pacing: the frame's work apart from the wait for
    the presentation, View > Real Time on so that the editor draws every frame).

    python tools/qa/test_build_job.py --project <project> [--only A,C,...] [--shots <folder>]

Each phase stands on its own setup: B, C and D build the module first when no phase before them did.

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

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP
PAYLOAD_FILES = 24
PAYLOAD_MB = 48
PHASES = "ABCDE"
# the frame's own work while a build runs, mean over the build's frames: measured 3.5-7.8 ms on the reference mod
# (STATE.md, "What the QA pass left, closed"); a frame that waited for vsync does not count
FRAME_WORK_MEAN_MS = 16.0
# and the worst frame of the build: 18.0-46.5 ms measured once tessellation left rayon's global pool (STATE.md, "The
# reviewer's findings, closed"); 36-218 ms before, the UI thread asleep in `par_iter`
FRAME_WORK_MAX_MS = 64.0

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
    exe = qa_target.exe()
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    # shown, never activated: a minimised editor draws no frame by itself, and A times the frames
    startup.wShowWindow = 4  # SW_SHOWNOACTIVATE
    qa_target.guard_project(project)
    proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
    deadline = time.monotonic() + 240
    while time.monotonic() < deadline:
        try:
            if mcp("mod_manifest", timeout=30).get("ok"):
                return proc
        except OSError:
            pass
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup")
        time.sleep(0.05)
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


class Run:
    def __init__(self, project: str, game: str, release: str, shots: str | None) -> None:
        self.project, self.game, self.release, self.shots = project, game, release, shots
        self.payload = os.path.join(project, "gamedata", "textures", "qa_job")
        self.module = ""
        self.scratch = os.path.join(game, "modules", ".new")
        self.target = game.replace("\\", "/")
        self.built = False

    def ensure_built(self) -> None:
        """What B, C and D start from: a module built from the payload as it was written."""
        if not self.built:
            r = mcp("mod_export", target=self.target, timeout=590.0)
            self.built = bool(r.get("ok"))
            check(self.built, "the module is built for the phase", json.dumps(r)[:300])

    # ---- A: the editor goes on while a build copies ---------------------------------------------------------------
    def a(self) -> None:
        mcp("ui_tab", action="open", id="level")
        mcp("view_mode", real_time=True)
        mcp("frame_pacing")
        job = Job("mod_export", target=self.target)
        row = wait_for_copy(job)
        check(bool(row), "A the Task Center follows the build's copying", json.dumps(job_row())[:300])
        check("Check, Copy, Install" == row.get("steps"), "A it names the steps", json.dumps(row)[:300])
        check(" of " in row.get("numbers", "") and "files" in row.get("numbers", ""), "A it says how far in bytes and files",
              json.dumps(row)[:300])
        refused = mcp("mod_export", target=self.target) if job.is_alive() else {"error": "the build ended first"}
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
        if job.is_alive() and self.shots:
            mcp("ui_popup", show="tasks")
            pic = mcp("screenshot_editor")
            if pic.get("png_base64"):
                os.makedirs(self.shots, exist_ok=True)
                with open(os.path.join(self.shots, "job_card.png"), "wb") as f:
                    f.write(base64.b64decode(pic["png_base64"]))
        job.join()
        pacing = mcp("frame_pacing")
        mcp("view_mode", real_time=False)
        work = pacing.get("frame_work_ms") or {}
        check(pacing.get("sampled_frames", 0) > 0 and work.get("mean", 99.0) < FRAME_WORK_MEAN_MS
              and work.get("max", 999.0) < FRAME_WORK_MAX_MS,
              "A the frames keep their own time during the build (%s frames, work mean %s ms, p95 %s, max %s; wait max %s)"
              % (pacing.get("sampled_frames"), work.get("mean"), work.get("p95"), work.get("max"),
                 (pacing.get("frame_wait_ms") or {}).get("max")), json.dumps(pacing)[:400])
        check(job.answer.get("ok") and job.answer.get("files", 0) >= PAYLOAD_FILES, "A the build answers when it has ended",
              json.dumps(job.answer)[:300])
        check(not os.path.exists(self.scratch), "A it leaves no scratch folder beside the module")
        self.built = bool(job.answer.get("ok"))

    # ---- B: Stop ----------------------------------------------------------------------------------------------------
    def b(self) -> None:
        self.ensure_built()
        before = fingerprint(self.module)
        for i in range(PAYLOAD_FILES):
            with open(os.path.join(self.payload, f"p_{i:02}.dds"), "r+b") as f:
                f.write((i + 1000).to_bytes(4, "little"))
        job = Job("mod_export", target=self.target)
        check(bool(wait_for_copy(job)), "B the changed files are being copied")
        stopped = mcp("progress", action="cancel")
        job.join()
        check(stopped.get("ok") and job.answer.get("error") == "cancelled", "B Stop ends the build", json.dumps(job.answer)[:300])
        check(fingerprint(self.module) == before, "B the last build is exactly as it was")
        check(not os.path.exists(self.scratch), "B and nothing is left beside it")
        # the payload as it was written: the next phase starts from it
        for i in range(PAYLOAD_FILES):
            with open(os.path.join(self.payload, f"p_{i:02}.dds"), "r+b") as f:
                f.write(i.to_bytes(4, "little"))

    # ---- C: what the last build holds is not written again ----------------------------------------------------------
    def c(self) -> None:
        self.ensure_built()
        built = mcp("mod_export", target=self.target, timeout=590.0)
        check(built.get("ok"), "C a build after that goes through", json.dumps(built)[:300])
        again_ids = {n: os.stat(os.path.join(self.module, n)).st_ino for n in fingerprint(self.module) if n.endswith(".dds")}
        with open(os.path.join(self.payload, "p_05.dds"), "r+b") as f:
            f.write(b"NEW!")
        began = time.monotonic()
        built = mcp("mod_export", target=self.target, timeout=590.0)
        spent = time.monotonic() - began
        rel = os.path.join("gamedata", "textures", "qa_job", "p_05.dds")
        now = {n: os.stat(os.path.join(self.module, n)).st_ino for n in again_ids}
        same = sum(1 for n in again_ids if n != rel and now[n] == again_ids[n])
        check(built.get("ok") and same == len(again_ids) - 1, f"C it reuses the {same} unchanged files ({spent:.1f} s)",
              json.dumps(built)[:200])
        with open(os.path.join(self.module, rel), "rb") as f:
            check(f.read(4) == b"NEW!", "C and takes the changed one")

    # ---- D: a module the game holds -----------------------------------------------------------------------------------
    def d(self) -> None:
        self.ensure_built()
        rel = os.path.join("gamedata", "textures", "qa_job", "p_05.dds")
        before = fingerprint(self.module)
        held = open(os.path.join(self.module, rel), "rb")
        try:
            refused = mcp("mod_export", target=self.target, timeout=590.0)
        finally:
            held.close()
        check("in use" in refused.get("error", ""), "D a module in use is refused and named", json.dumps(refused)[:300])
        check(fingerprint(self.module) == before and not os.path.exists(self.scratch), "D and it stays whole")

    # ---- E: a packaging --------------------------------------------------------------------------------------------
    def e(self) -> None:
        # the project is enough to package: random payload only costs the zip its time
        for i in range(PAYLOAD_FILES):
            p = os.path.join(self.payload, f"p_{i:02}.dds")
            if os.path.exists(p):
                os.remove(p)
        job = Job("mod_package", out_dir=self.release.replace("\\", "/"), requires_game="")
        seen_pack = False
        worst = 0.0
        while job.is_alive():
            began = time.monotonic()
            row = job_row()
            worst = max(worst, time.monotonic() - began)
            seen_pack |= row.get("step") == 2 and "Pack" in row.get("steps", "")
            time.sleep(.05)
        job.join()
        check(job.answer.get("ok") and job.answer.get("packages"), "E the packaging answers as it always did",
              json.dumps(job.answer)[:300])
        check(seen_pack, "E the Task Center follows it to its packing step")
        check(worst < .75, f"E a command is answered at once while it packs ({worst * 1000:.0f} ms at worst)")
        names = os.listdir(os.path.join(self.project, "temp")) if os.path.isdir(os.path.join(self.project, "temp")) else []
        check(not [n for n in names if n.startswith("release_stage_")], "E its stage is gone", str(names))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True)
    parser.add_argument("--only", help="phase letters, e.g. A,C")
    parser.add_argument("--shots")
    args = parser.parse_args()
    asked = [p for p in (args.only or PHASES).replace(",", "").upper() if p.strip()]
    only = [p for p in PHASES if p in asked]
    if any(p not in PHASES for p in asked):
        print("no such phase: %s (%s)" % (args.only, PHASES))
        return 2
    source = os.path.abspath(args.project)
    work = tempfile.mkdtemp(prefix="grizzly_job_")
    project = os.path.join(work, "project")
    game = os.path.join(work, "game")
    release = os.path.join(work, "release")
    proc = None
    try:
        qa_target.copy_project(source, project, ignore=(".git", "releases", "temp", "logs", "backup", "preview.png"))
        payload = os.path.join(project, "gamedata", "textures", "qa_job")
        os.makedirs(payload)
        # only the phases that copy need the gigabyte: a packaging alone gets the two files it packs
        files = PAYLOAD_FILES if set(only) - {"E"} else 0
        block = os.urandom(PAYLOAD_MB << 20)
        for i in range(files):
            with open(os.path.join(payload, f"p_{i:02}.dds"), "wb") as f:
                f.write(i.to_bytes(4, "little") + block)
        qa_target.mark("payload")
        proc = start_editor(project)
        qa_target.mark("editor start")
        run = Run(project, game, release, args.shots)
        run.module = os.path.join(game, "modules", mcp("mod_manifest").get("id"))
        for p in only:
            t = time.monotonic()
            getattr(run, p.lower())()
            print("TIME phase %s %.1f s" % (p, time.monotonic() - t), flush=True)
    finally:
        if proc:
            try:
                mcp("window_control", action="close")
                proc.wait(timeout=120)
            except (OSError, subprocess.TimeoutExpired) as error:
                print("! the editor did not close:", error, flush=True)
        qa_target.remove_tree(work)
    failed = [text for ok, text in results if not ok]
    print(f"{len(results) - len(failed)}/{len(results)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
