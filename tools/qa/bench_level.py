#!/usr/bin/env python3
"""How long a level build takes, step by step and phase by phase: the measuring tool of the build's speed.

On a THROW-AWAY COPY of a project (its own scene, or a scene of the editor's library put into the copy):

    Grizzly-Cmd.exe run level.build steps=<steps> quality=<quality> force=true -project <copy>

under a time limit. What it prints - and writes as JSON with --out - is every step's seconds and, for the compiler,
every phase's seconds, read from the commandlet's own log, so a run the limit cut still says how far it came and
where the time went. --base <json> prints the same table beside an earlier run with the ratio.

    python tools/qa/bench_level.py --project <project> [--scene l01_escape] [--quality draft]
        [--steps light,detail_light] [--limit 600] [--out run.json] [--base before.json] [--args "gpu=false"]

Nothing is compared with a record here and nothing fails on a number: it measures. The limit is the run's whole
budget: a run that needs longer than the target is cut and its table says "cut".
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

import qa


PHASE = re.compile(r"\* Phase done: (.+) \(([0-9.]+) s\)")
STEP = re.compile(r"\* (.+) done in ([0-9.]+) s\.")
PART = re.compile(r"\* (The level compiler|The detail light compiler) ended in ([0-9.]+) s\.")
STAGE = re.compile(r"\* Phase: (.+)")


def put_scene(project: str, scene: str) -> None:
    """A scene of the editor's library becomes the copy's scene."""
    lib = os.path.join(qa.REPO, "rawdata", "levels")
    if not os.path.isfile(os.path.join(lib, scene + ".level")):
        raise RuntimeError("%s is not in the editor's library (rawdata\\levels)" % scene)
    levels = os.path.join(project, "rawdata", "levels")
    os.makedirs(levels, exist_ok=True)
    shutil.copy2(os.path.join(lib, scene + ".level"), levels)
    shutil.copytree(os.path.join(lib, scene), os.path.join(levels, scene), dirs_exist_ok=True)
    import test_level_compile
    test_level_compile.set_scene(project, "rawdata/levels/%s.level" % scene)


def read_log(home: str) -> dict:
    """The steps and the compiler's phases with their seconds, as the commandlet's log has them."""
    steps: list[list] = []
    phases: list[list] = []
    parts: list[list] = []
    last_stage = ""
    for path in sorted(glob.glob(os.path.join(home, "logs", "grizzly_cmd_*.log"))):
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.rstrip()
                at = line.find("* ")
                if at < 0:
                    continue
                line = line[at:]
                m = PHASE.search(line)
                if m:
                    phases.append([m.group(1), float(m.group(2))])
                    continue
                m = PART.search(line)
                if m:
                    parts.append([m.group(1), float(m.group(2))])
                    continue
                m = STEP.search(line)
                if m:
                    steps.append([m.group(1), float(m.group(2))])
                    continue
                m = STAGE.search(line)
                if m:
                    last_stage = m.group(1)
    return {"steps": steps, "phases": phases, "parts": parts, "last_phase_started": last_stage}


def table(run: dict, base: dict | None) -> str:
    out = []
    before = {}
    if base:
        for kind in ("steps", "phases"):
            for name, s in base.get(kind, []):
                before[(kind, name)] = s

    def row(kind: str, name: str, s: float) -> str:
        b = before.get((kind, name))
        tail = ""
        if b is not None:
            tail = "   was %8.1f s   x%.1f" % (b, b / s if s > 0.005 else float("inf"))
        return "  %-52s %8.1f s%s" % (name, s, tail)

    out.append("%s, %s quality, %s%s: %.1f s%s" % (
        run["level"], run["quality"], run["steps_asked"] or "every step",
        (" " + run["args"]) if run.get("args") else "", run["seconds"],
        "  (CUT at the limit of %d s in '%s')" % (run["limit"], run["last_phase_started"]) if run["cut"] else ""))
    out.append("steps:")
    for name, s in run["steps"]:
        out.append(row("steps", name, s))
    if run["phases"]:
        out.append("the compiler's phases:")
        for name, s in run["phases"]:
            out.append(row("phases", name, s))
    if base and not run["cut"] and not base.get("cut"):
        out.append("whole run: was %.1f s, x%.1f" % (base["seconds"], base["seconds"] / max(run["seconds"], 0.01)))
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True, help="a project to COPY and build in")
    ap.add_argument("--scene", help="a scene of the editor's library to build instead of the project's own")
    ap.add_argument("--quality", default="draft", choices=["draft", "standard", "final"])
    ap.add_argument("--steps", default="", help="the steps to build (left out: every step)")
    ap.add_argument("--limit", type=int, default=600, help="the run's budget in seconds; it is cut there")
    ap.add_argument("--args", default="", help="more parameters of level.build, as name=value words")
    ap.add_argument("--exe", default=os.path.join(qa.REPO, "Bin", "x64", "Release", "Grizzly-Cmd.exe"))
    ap.add_argument("--out", help="write the run as JSON here")
    ap.add_argument("--base", help="an earlier run's JSON to print beside this one")
    ap.add_argument("--keep", help="copy the build's own inputs (build.prj, build.details, build.cform, "
                                   "build.lights) into this folder: what the compiler's bench reads")
    a = ap.parse_args()

    src = os.path.abspath(a.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="gzqa_bench_")
    home = os.path.join(work, "home")
    os.makedirs(home)
    try:
        project = qa.copy_project(src, os.path.join(work, "project"), home_folder=home)
        if a.scene:
            put_scene(project, a.scene)
        # what an earlier build of the source left is not this run's
        built = os.path.join(project, "gamedata", "levels")
        if os.path.isdir(built):
            qa.remove_tree(built)
        cmd = [sys.executable, qa.LIMITED, str(a.limit), a.exe, "run", "level.build", "quality=" + a.quality,
               "force=true"]
        if a.steps:
            cmd.append("steps=" + a.steps)
        cmd += a.args.split()
        cmd += ["-project", project, "-home", home, "-json", "-quiet"]
        t = time.monotonic()
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=a.limit + 60)
        seconds = time.monotonic() - t
        line = (p.stdout.strip().splitlines() or ["{}"])[-1]
        try:
            answer = json.loads(line)
        except ValueError:
            answer = {}
        run = read_log(home)
        run.update({
            "level": answer.get("level") or a.scene or os.path.basename(src),
            "quality": a.quality,
            "steps_asked": a.steps,
            "args": a.args,
            "seconds": round(seconds, 1),
            "limit": a.limit,
            "cut": p.returncode == 124,
            "exit": p.returncode,
            "exe": a.exe,
        })
        if p.returncode not in (0, 124):
            run["error"] = (answer.get("error") or line or p.stderr[-400:])[:600]
        if a.keep:
            os.makedirs(a.keep, exist_ok=True)
            for folder in glob.glob(os.path.join(project, "gamedata", "levels", "*")):
                for name in ("build.prj", "build.details", "build.cform", "build.lights"):
                    if os.path.isfile(os.path.join(folder, name)):
                        shutil.copy2(os.path.join(folder, name), a.keep)
        base = None
        if a.base:
            with open(a.base, encoding="utf-8") as f:
                base = json.load(f)
        print(table(run, base))
        if run.get("error"):
            print("the build ended with an error: " + str(run["error"]))
        if a.out:
            with open(a.out, "w", encoding="utf-8", newline="\n") as f:
                json.dump(run, f, indent=1)
                f.write("\n")
        return 0 if p.returncode in (0, 124) else 1
    finally:
        qa.remove_tree(work)


if __name__ == "__main__":
    sys.exit(main())
