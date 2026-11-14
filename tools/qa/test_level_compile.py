#!/usr/bin/env python3
"""The level compiler's regression net: two small scenes of the editor's library built at draft quality from a cold
start, the outputs' structure compared with the records in tools/qa/golden/compile/<scene>.json.

For each scene, on a THROW-AWAY COPY of a project whose scene it is made:

    Grizzly-Cmd.exe run level.build steps=light quality=draft -project <copy>

(the geometry step and the compiler, nothing else) and of the level folder it wrote:
  - every file the build and the compiler wrote, with its size and SHA-256 (the compiler gives the same bytes on any
    number of threads and on every run, so every one is compared exactly);
  - the compiled level's visuals, sectors, portals and lights (the chunks of `level`);
  - the collision form's vertices and triangles (`level.cform`);
  - the light maps and their sizes.
A difference names what differs. --record writes the records instead (small enough to commit); a record that is
missing fails the scene with the sentence that says so.

    python tools/qa/test_level_compile.py --project <project> [--record] [--scene mp_test]
"""
from __future__ import annotations

import argparse
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

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.join(HERE, "golden", "compile")
LIMITED = os.path.join(qa_target.REPO, "tools", "rust", "limited.py")
SCENES = ["mp_test", "mp_close_combat"]
# the build's own record and the files of the editor's other steps are not the compiler's
NOT_COMPARED = {"build.state", "build.err"}
results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


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


def compile_scene(src: str, scene: str, record: bool) -> None:
    lib = os.path.join(qa_target.REPO, "rawdata", "levels")
    if not os.path.isfile(os.path.join(lib, scene + ".level")):
        check(False, "%s is in the editor's library (rawdata\\levels)" % scene)
        return
    work = tempfile.mkdtemp(prefix="gzqa_compile_")
    try:
        project = qa_target.copy_project(src, os.path.join(work, "project"), trimmed=True)
        levels = os.path.join(project, "rawdata", "levels")
        os.makedirs(levels, exist_ok=True)
        shutil.copy2(os.path.join(lib, scene + ".level"), levels)
        shutil.copytree(os.path.join(lib, scene), os.path.join(levels, scene), dirs_exist_ok=True)
        set_scene(project, "rawdata/levels/%s.level" % scene)
        exe = os.path.join(os.path.dirname(qa_target.exe()), "Grizzly-Cmd.exe")
        t = time.monotonic()
        p = subprocess.run([sys.executable, LIMITED, "100", exe, "run", "level.build", "steps=light", "quality=draft",
                            "-project", project, "-home", qa_target.HOME, "-json", "-quiet"],
                           capture_output=True, text=True, timeout=120)
        seconds = time.monotonic() - t
        line = (p.stdout.strip().splitlines() or ["{}"])[-1]
        try:
            answer = json.loads(line)
        except ValueError:
            answer = {}
        if not check(p.returncode == 0 and answer.get("ok"),
                     "%s builds and compiles from a cold start at draft quality (%.1f s)" % (scene, seconds),
                     (line + " " + p.stderr[-400:])[:600]):
            return
        level = answer.get("level") or scene
        got = structure(os.path.join(project, "gamedata", "levels", level))
        golden = os.path.join(GOLDEN, scene + ".json")
        if record:
            os.makedirs(GOLDEN, exist_ok=True)
            with open(golden, "w", encoding="utf-8", newline="\n") as f:
                json.dump({"scene": scene, "quality": "draft", **got}, f, indent=1, sort_keys=True)
                f.write("\n")
            check(True, "%s recorded: %d files, %s visuals, %s sectors, %s light maps" % (
                scene, len(got["files"]), got.get("visuals"), got.get("sectors"), len(got.get("lightmaps", []))))
            return
        if not check(os.path.isfile(golden), "%s has a record (run with --record once)" % scene, golden):
            return
        with open(golden, encoding="utf-8") as f:
            want = json.load(f)
        diff = differences(want, got)
        check(not diff, "%s compiles as recorded: %d files, %s visuals, %s sectors, %s portals, %s triangles" % (
            scene, len(got["files"]), got.get("visuals"), got.get("sectors"), got.get("portals"),
            (got.get("cform") or {}).get("triangles")), "; ".join(diff[:8]))
    finally:
        qa_target.remove_tree(work)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True, help="a project to COPY and build in")
    ap.add_argument("--record", action="store_true", help="write the records instead of comparing")
    ap.add_argument("--scene", action="append", help="only this scene (again for more)")
    args = ap.parse_args()
    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    for scene in args.scene or SCENES:
        compile_scene(src, scene, args.record)
    ok = sum(1 for good, _ in results if good)
    print(f"LEVEL COMPILE: DONE {ok}/{len(results)}", flush=True)
    return 0 if results and ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
