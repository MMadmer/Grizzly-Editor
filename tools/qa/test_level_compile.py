#!/usr/bin/env python3
"""The level compiler's regression net: two small scenes of the editor's library built from a cold start, the
outputs compared with the records in tools/qa/golden/compile/.

For each scene, on a THROW-AWAY COPY of a project whose scene it is made:

    Grizzly-Cmd.exe run level.build steps=light force=true quality=<draft|standard> light=<cpu|auto> -project <copy>

(the geometry step and the compiler, nothing else):
  - **on the CPU lighter, the reference**, at draft quality (no lightmaps: <scene>.json) and, for the smallest
    scene, at standard quality (lightmaps and their atlas: <scene>_standard.json): every file the build and the compiler wrote, with its size
    and SHA-256 - the CPU lighter gives the same bytes on every machine, any number of threads and every run, so
    every one is compared exactly -, the compiled level's visuals, sectors, portals and lights (the chunks of
    `level`), the collision form's vertices and triangles, the light maps and their sizes, the structure's hash;
  - **on the graphics card** (light=auto; skipped with a line on a machine that has none), that scene at standard:
    another light, the same structure - the structure's hash, the visuals, sectors, portals and collision are the
    CPU build's, the collision files and the fast geometry are the same bytes, and the pages are there.
A difference names what differs. --record writes the records instead (small enough to commit); a record that is
missing fails the scene with the sentence that says so.

    python tools/qa/test_level_compile.py --project <project> [--record] [--scene mp_test]
"""
from __future__ import annotations

import glob
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

import qa
from qa import check


HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.join(HERE, "golden", "compile")
SCENES = ["mp_test", "mp_close_combat"]
# the scenes small enough to be lit at standard quality on the CPU inside a short test: the lightmaps, their
# atlas and the graphics card's light are proven on these
WITH_LIGHTMAPS = {"mp_test"}
# the build's own record and the files of the editor's other steps are not the compiler's
NOT_COMPARED = {"build.state", "build.report", "build.err"}
def chunks(data: bytes) -> dict[int, bytes]:
    """The top-level chunks of an X-Ray chunk file by id (the compressed flag taken off)."""
    out: dict[int, bytes] = {}
    at = 0
    while at + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, at)
        out[cid & 0x7FFFFFFF] = data[at + 8:at + 8 + size]
        at += 8 + size
    return out


def structure(folder: str) -> dict:
    """What a level folder holds, as the record keeps it."""
    files = {}
    for path in sorted(glob.glob(os.path.join(folder, "**", "*"), recursive=True)):
        if not os.path.isfile(path):
            continue
        rel = os.path.relpath(path, folder).replace("\\", "/")
        if rel in NOT_COMPARED or rel.startswith("."):
            continue
        with open(path, "rb") as f:
            data = f.read()
        files[rel] = {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    out: dict = {"files": files}
    level = os.path.join(folder, "level")
    if os.path.isfile(level):
        with open(level, "rb") as f:
            c = chunks(f.read())
        out["visuals"] = len(chunks(c.get(3, b"")))
        out["sectors"] = len(chunks(c.get(8, b"")))
        out["portals"] = len(c.get(4, b"")) // 80
        out["light_bytes"] = len(c.get(6, b""))
    cform = os.path.join(folder, "level.cform")
    if os.path.isfile(cform):
        with open(cform, "rb") as f:
            _version, verts, tris = struct.unpack("<III", f.read(12))
        out["cform"] = {"vertices": verts, "triangles": tris}
    maps = []
    for path in sorted(glob.glob(os.path.join(folder, "lmap#*_1.dds"))):
        with open(path, "rb") as f:
            head = f.read(20)
        height, width = struct.unpack_from("<II", head, 12)
        maps.append([os.path.basename(path), width, height])
    out["lightmaps"] = maps
    return out


def differences(want: dict, got: dict) -> list[str]:
    out = []
    for key in ("visuals", "sectors", "portals", "light_bytes", "cform", "lightmaps"):
        if want.get(key) != got.get(key):
            out.append("%s: %s, recorded %s" % (key, json.dumps(got.get(key)), json.dumps(want.get(key))))
    wf, gf = want.get("files", {}), got.get("files", {})
    for name in sorted(set(wf) | set(gf)):
        if name not in gf:
            out.append("%s is not written" % name)
        elif name not in wf:
            out.append("%s is written and not recorded" % name)
        elif wf[name] != gf[name]:
            out.append("%s: %d bytes %s, recorded %d bytes %s" % (name, gf[name]["size"], gf[name]["sha256"][:12],
                                                                     wf[name]["size"], wf[name]["sha256"][:12]))
    return out


def set_scene(project: str, rel: str) -> None:
    """The project file names the scene the project opens with."""
    files = [n for n in os.listdir(project) if n.lower().endswith(".grizzly")]
    if not files:
        # a project not opened since project files came: its old project.ltx names the scene, read once
        legacy = os.path.join(project, "project.ltx")
        if not os.path.isfile(legacy):
            raise RuntimeError("the copy has no project file")
        with open(legacy, encoding="utf-8", errors="replace") as f:
            lines = [l for l in f.read().splitlines() if not l.strip().lower().startswith("last_scene")]
        at = next((i for i, l in enumerate(lines) if l.strip().lower() == "[project]"), None)
        if at is None:
            lines += ["[project]"]
            at = len(lines) - 1
        lines.insert(at + 1, "last_scene=" + os.path.join(project, rel.replace("/", "\\")))
        with open(legacy, "w", encoding="utf-8", newline="") as f:
            f.write("\r\n".join(lines) + "\r\n")
        return
    path = os.path.join(project, files[0])
    with open(path, encoding="utf-8") as f:
        text = f.read()
    line = 'scene = "%s"' % rel
    if re.search(r'^\s*scene\s*=\s*"[^"]*"', text, re.M):
        text = re.sub(r'^(\s*)scene\s*=\s*"[^"]*"', lambda m: m.group(1) + line, text, count=1, flags=re.M)
    else:
        text = re.sub(r'^(\s*grizzly\s*=\s*\d+\s*,?\s*)$', lambda m: m.group(1) + "\n  " + line + ",", text, count=1,
                      flags=re.M)
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def build(project: str, quality: str, light: str) -> tuple[dict, float, str]:
    """`level.build steps=light` of the copy at a quality on a lighter, from scratch: the answer, the seconds, what
    it said besides."""
    exe = os.path.join(os.path.dirname(qa.exe()), "Grizzly-Cmd.exe")
    t = time.monotonic()
    p = subprocess.run([sys.executable, qa.LIMITED, "100", exe, "run", "level.build", "steps=light", "force=true",
                        "quality=" + quality, "light=" + light, "-project", project, "-home", qa.home(), "-json",
                        "-quiet"], capture_output=True, text=True, timeout=120)
    line = (p.stdout.strip().splitlines() or ["{}"])[-1]
    try:
        answer = json.loads(line)
    except ValueError:
        answer = {}
    if p.returncode != 0:
        answer["ok"] = False
    return answer, time.monotonic() - t, (line + " " + p.stderr[-400:])[:600]


def light_of(answer: dict) -> dict:
    """What the compiler says of its light step: the lighter's name, the samples it lit, the structure's hash."""
    for step in answer.get("ran") or []:
        if step.get("step") == "light":
            parts = (step.get("note") or {}).get("parts") or [{}]
            return parts[0]
    return {}


def against_record(scene: str, quality: str, got: dict, record: bool) -> None:
    """The level folder against its record of this quality (or the record written)."""
    name = scene if quality == "draft" else "%s_%s" % (scene, quality)
    golden = os.path.join(GOLDEN, name + ".json")
    if record:
        os.makedirs(GOLDEN, exist_ok=True)
        with open(golden, "w", encoding="utf-8", newline="\n") as f:
            json.dump({"scene": scene, "quality": quality, **got}, f, indent=1, sort_keys=True)
            f.write("\n")
        check(True, "%s at %s recorded: %d files, %s visuals, %s sectors, %s light maps" % (
            scene, quality, len(got["files"]), got.get("visuals"), got.get("sectors"), len(got.get("lightmaps", []))))
        return
    if not check(os.path.isfile(golden), "%s at %s has a record (run with --record once)" % (scene, quality), golden):
        return
    with open(golden, encoding="utf-8") as f:
        want = json.load(f)
    diff = differences(want, got)
    check(not diff, "%s at %s on the CPU compiles as recorded: %d files, %s visuals, %s sectors, %s light maps" % (
        scene, quality, len(got["files"]), got.get("visuals"), got.get("sectors"), len(got.get("lightmaps", []))),
        "; ".join(diff[:8]))


def compile_scene(src: str, scene: str, record: bool) -> None:
    lib = os.path.join(qa.REPO, "rawdata", "levels")
    if not os.path.isfile(os.path.join(lib, scene + ".level")):
        check(False, "%s is in the editor's library (rawdata\\levels)" % scene)
        return
    work = tempfile.mkdtemp(prefix="gzqa_compile_")
    try:
        project = qa.copy_project(src, os.path.join(work, "project"), trimmed=True)
        levels = os.path.join(project, "rawdata", "levels")
        os.makedirs(levels, exist_ok=True)
        shutil.copy2(os.path.join(lib, scene + ".level"), levels)
        shutil.copytree(os.path.join(lib, scene), os.path.join(levels, scene), dirs_exist_ok=True)
        set_scene(project, "rawdata/levels/%s.level" % scene)
        # the reference: the CPU lighter, the same bytes on every machine - without lightmaps, then with them
        on_cpu = {}
        qualities = ("draft", "standard") if scene in WITH_LIGHTMAPS else ("draft",)
        for quality in qualities:
            answer, seconds, said = build(project, quality, "cpu")
            if not check(answer.get("ok"), "%s builds and compiles at %s quality on the CPU (%.1f s)" % (
                    scene, quality, seconds), said):
                return
            level = answer.get("level") or scene
            got = structure(os.path.join(project, "gamedata", "levels", level))
            light = light_of(answer)
            got["structure"] = light.get("structure")
            check(light.get("lighter") == "the CPU" and got["structure"],
                  "%s at %s: the answer names the lighter and the structure's hash" % (scene, quality), light)
            on_cpu[quality] = got
            against_record(scene, quality, got, record)
        if "standard" not in on_cpu:
            return
        check(len(on_cpu["standard"].get("lightmaps", [])) > 0,
              "%s at standard quality has lightmaps: the atlas is part of the record" % scene,
              on_cpu["standard"].get("lightmaps"))
        # the graphics card lights the same level: another light, the same structure
        answer, seconds, said = build(project, "standard", "auto")
        if not check(answer.get("ok"), "%s builds and compiles at standard quality on auto (%.1f s)" % (
                scene, seconds), said):
            return
        light = light_of(answer)
        if light.get("lighter") == "the CPU":
            check(True, "%s: this machine has no graphics card to light on - the lighters are not compared" % scene)
            return
        got = structure(os.path.join(project, "gamedata", "levels", answer.get("level") or scene))
        want = on_cpu["standard"]
        same = [k for k in ("visuals", "sectors", "portals", "cform") if want.get(k) == got.get(k)]
        check(light.get("structure") == want["structure"] and len(same) == 4,
              "%s lit on %s has the structure the CPU's build has (%s)" % (
                  scene, light.get("lighter"), light.get("structure")),
              "structure %s against %s; equal: %s" % (light.get("structure"), want["structure"], same))
        names = ("level.cform", "build.cform", "level.geomx")
        check(all(want["files"].get(n) == got["files"].get(n) for n in names),
              "%s: the collision and the fast geometry are the same bytes on both lighters" % scene,
              [n for n in names if want["files"].get(n) != got["files"].get(n)])
        check(len(got.get("lightmaps", [])) > 0, "%s lit on the card has its lightmap pages" % scene,
              got.get("lightmaps"))
    finally:
        qa.remove_tree(work)


def run(ctx: qa.Run) -> None:
    src = os.path.abspath(ctx.args.project)
    if not os.path.isdir(src):
        raise RuntimeError("no such project: " + src)
    for scene in ctx.args.scene or SCENES:
        compile_scene(src, scene, ctx.args.record)


def main() -> int:
    def more(ap) -> None:
        ap.add_argument("--project", required=True, help="a project to COPY and build in")
        ap.add_argument("--record", action="store_true", help="write the records instead of comparing")
        ap.add_argument("--scene", action="append", help="only this scene (again for more)")

    return qa.suite("LEVEL COMPILE", run, doc=__doc__, add_args=more, project=False)


if __name__ == "__main__":
    sys.exit(main())
