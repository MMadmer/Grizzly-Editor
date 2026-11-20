"""The reference module: one asset of every kind an author makes, built by this editor and frozen into
the game's archive of old modules, which every later game loads (the game repo's
tools/qa/module/Run-OldModulesQa.ps1). The game promises that a module built by any released editor keeps
working: this archive is what proves it, release after release.

    python tools/qa/freeze_reference_module.py --archive "D:/Games/Dead Air/DeadAir-x64/tools/qa/module/old"

Everything is made in a folder of its own under the system temp folder, with an editor home of its own,
through the commandlet (Grizzly-Cmd.exe): a project made from the editor's own templates, its module built
into a folder, and that folder copied to <archive>/<editor version>/<module id>, with <module id>.txt beside
it saying which editor made it and from what. An archive entry that is there already is never replaced: a
frozen module is frozen. Nothing of the author's projects or of the installed game is touched.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import wave
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EXE = os.environ.get("GRIZZLY_QA_EXE", os.path.join(ROOT, "Bin", "x64", "Release", "Grizzly-Cmd.exe"))
if os.path.basename(EXE).lower() == "grizzly.exe":
    EXE = os.path.join(os.path.dirname(EXE), "Grizzly-Cmd.exe")
MODULE = "xms_reference"
# every kind file.create makes, each in a folder of its own
KINDS = ["quest", "behaviour", "character", "pda", "item", "trader", "loot", "recipes", "model", "texture", "sound"]


def run(home: str, project: str | None, *words: str, timeout: int = 900) -> dict:
    args = [EXE, "-home", home, "-nodlg", "-json"]
    if project:
        args += ["-project", project]
    args += list(words)
    env = dict(os.environ, GRIZZLY_QA_GITHUB="1")
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace", env=env,
                       timeout=timeout)
    lines = [ln for ln in r.stdout.splitlines() if ln.strip().startswith("{")]
    answer = json.loads(lines[-1]) if lines else {}
    if r.returncode != 0 or not answer.get("ok", False):
        raise SystemExit(f"{' '.join(words)} failed ({r.returncode}): {answer or r.stdout[-800:]} {r.stderr[-800:]}")
    return answer


def picture(path: str) -> None:
    # a 4 x 4 picture of one colour as a PNG, the way the texture template names its source
    rows = b"".join(bytes([0]) + bytes([90, 140, 60, 255]) * 4 for _ in range(4))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    signature = bytes([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    with open(path, "wb") as f:
        f.write(signature + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def sound(path: str) -> None:
    # a tenth of a second of silence
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(22050)
        w.writeframes(bytes(4410))


def editor_version() -> str:
    text = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
    m = re.search(r'^\s*version\s*=\s*"([^"]+)"', text[text.find("[workspace.package]"):], re.M)
    return m.group(1) if m else "unknown"


def made(project: str, kind: str, ext: str) -> list[str]:
    folder = os.path.join(project, kind)
    return [os.path.join(folder, n[: -len(ext)]) for n in os.listdir(folder) if n.endswith(ext)]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--archive", required=True, help="the game's tools/qa/module/old folder")
    ap.add_argument("--keep", action="store_true", help="keep the work folder")
    a = ap.parse_args()
    version = editor_version()
    target = os.path.join(a.archive, version, MODULE)
    if os.path.exists(target):
        print(f"{target} is there already: a frozen module is never replaced")
        return 0
    work = tempfile.mkdtemp(prefix="gz_reference_")
    home = os.path.join(work, "home")
    try:
        print(f"== editor {version}, work folder {work}")
        run(home, None, "run", "project.create", f"name={MODULE}", f"parent={work}")
        project = os.path.join(work, MODULE)
        for kind in KINDS:
            os.makedirs(os.path.join(project, kind), exist_ok=True)
            run(home, project, "run", "file.create", f"kind={kind}", f"path={kind}")
        # what the templates of a texture and a sound name beside them
        for stem in made(project, "texture", ".texasset"):
            picture(stem + ".png")
        for stem in made(project, "sound", ".sndasset"):
            sound(stem + ".wav")
        run(home, project, "run", "mod.set",
            'set={"module":{"target":"default","version":"1.0.0","name":"XMS Reference","author":"Grizzly QA"}}')
        checked = run(home, project, "run", "asset.check", "limit=50")
        errors = [p for p in checked.get("problems", []) if p.get("severity") == "error"]
        if errors:
            raise SystemExit("the reference project has errors: " + json.dumps(errors[:10]))
        out = os.path.join(work, "out")
        built = run(home, project, "run", "mod.build", "target=folder", f"path={out}")
        module = os.path.join(out, "modules", MODULE)
        if not os.path.isdir(module):
            raise SystemExit(f"the build wrote no module: {built}")
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copytree(module, target)
        files = sum(len(fs) for _, _, fs in os.walk(target))
        with open(target + ".txt", "w", encoding="utf-8", newline="\r\n") as f:
            f.write(f"Built by Grizzly Editor {version} on {time.strftime('%Y-%m-%d')} from its own templates:\n"
                    f"one asset of every kind ({', '.join(KINDS)}).\n"
                    "Frozen: never rebuilt, never edited. Every later game loads it (Run-OldModulesQa.ps1).\n")
        print(f"== frozen: {target} ({files} files)")
        return 0
    finally:
        if not a.keep:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
