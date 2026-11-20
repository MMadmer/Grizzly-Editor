#!/usr/bin/env python3
"""A module's game data turned into the editor's media assets, where it lies.

A classic mod ships its textures, sounds and models in the game's own containers, and what it says of them in
binaries: a texture's description in a `.thm`, a sound's parameters in its `.ogg`'s comment. A module of the
editor says that as text - a `.texasset`, a `.sndasset`, a `.meshasset` beside each file - and the game reads the
text over the data. This step gives every such file under a project's `gamedata\\textures`, `gamedata\\sounds` and
`gamedata\\meshes` its asset, for any classic mod:

  * every `.dds` (but a `#` companion, which travels with its texture) gets a `.texasset` saying what the game
    reads of its `.thm`, and the `.thm` goes; a `.thm` alone describes a texture of the game
    (`source = "game:<name>"`);
  * every `.ogg` gets a `.sndasset` with the parameters its comment holds;
  * every `.ogf` gets a dynamic `.meshasset`: the model the game loads by its name.

It is the editor's own import (`asset.import ... remove_descriptions=true`, run by `Grizzly-Cmd.exe`): an asset says
exactly what the game reads of its file, read back from its own text, and the build publishes every file as it is -
a game texture, a sound, a model are never made again. A file an asset could not say the same way stays as it is,
and the run says so. A second run changes nothing: a file that has its asset is kept. What lies elsewhere in
`gamedata` (a level's lightmaps) is read by the game where it lies and gets no asset.

    python tools/classic_mod/media_assets.py --project <module project> [--editor <Grizzly-Cmd.exe>] [--dry]
        [--keep-descriptions]

The converter runs it as one of its steps; run by hand it does the same to a project that exists, and keeps the
converter's ledger of the project in step (`ledger.py`). It only ever writes the assets beside the files and
removes the `.thm` files they now say all the game reads of (`--keep-descriptions` keeps them).
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import ledger  # noqa: E402

REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
EDITOR = os.path.join(REPO, "Bin", "x64", "Release", "Grizzly-Cmd.exe")
# the folders of `gamedata` the game looks its media up in, by kind
FOLDERS = ("textures", "sounds", "meshes")
PICTURES = (".dds", ".png", ".jpg", ".jpeg", ".tga", ".bmp")


class Failed(Exception):
    """The import could not run, or the editor refused it: the sentence says why."""


class Done:
    """What a run did, project-relative paths with forward slashes."""

    def __init__(self, answer: dict):
        self.assets: list[str] = answer.get("assets", [])
        self.removed: list[str] = answer.get("removed", [])
        self.kept: list[str] = answer.get("kept", [])
        self.problems: list[str] = answer.get("problems", [])


def media_of(project: str) -> list[str]:
    """The files of the project's game data a media asset is made over, project-relative: what lies under
    `gamedata\\textures`, `gamedata\\sounds` and `gamedata\\meshes`."""
    out = []
    for folder in FOLDERS:
        top = os.path.join(project, "gamedata", folder)
        for d, dirs, files in os.walk(top):
            dirs.sort()
            for f in sorted(files):
                low = f.lower()
                rel = os.path.relpath(os.path.join(d, f), project).replace("\\", "/")
                stem = f[: f.rfind(".")] if "." in f else f
                if folder == "textures" and low.endswith(".dds") and not low.endswith("#.dds"):
                    out.append(rel)
                elif folder == "textures" and low.endswith(".thm") and not any(
                        os.path.isfile(os.path.join(d, stem + e)) for e in PICTURES):
                    # a description alone: of a texture of the game
                    out.append(rel)
                elif (folder == "sounds" and low.endswith(".ogg")) or (folder == "meshes" and low.endswith(".ogf")):
                    out.append(rel)
    return out


def process(project: str, editor: str = EDITOR, dry: bool = False, say=print, remove: bool = True) -> Done:
    """Imports the media of `project` in place through the editor; answers what it did. Raises `Failed`."""
    files = media_of(project)
    if not files:
        return Done({})
    if not os.path.isfile(editor):
        raise Failed(f"the editor's console program is not there: {editor}")
    say(f"media assets: {len(files)} files of the game's formats")
    with tempfile.TemporaryDirectory(prefix="gz_media_assets_") as tmp:
        script = os.path.join(tmp, "import.jsonl")
        with open(script, "w", encoding="utf-8", newline="\n") as f:
            f.write(json.dumps({"cmd": "asset.import", "from": files, "dry_run": dry, "remove_descriptions": remove},
                               ensure_ascii=False) + "\n")
        out_path, err_path = os.path.join(tmp, "out.txt"), os.path.join(tmp, "err.txt")
        limited = os.path.join(REPO, "tools", "rust", "limited.py")
        cmd = [sys.executable, limited, "3600", editor, "script", script, "-project", project,
               "-home", os.path.join(tmp, "home"), "-json"]
        with open(out_path, "wb") as out, open(err_path, "wb") as err:
            p = subprocess.Popen(cmd, stdout=out, stderr=err)
            started, next_note = time.monotonic(), 120.0
            while p.poll() is None:
                time.sleep(1)
                if time.monotonic() - started >= next_note:
                    say(f"media assets: still importing ({int(next_note)} s)")
                    next_note += 120.0
        code = p.returncode
        with open(out_path, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        with open(err_path, "rb") as f:
            errors = f.read().decode("utf-8", "replace")
    answer = None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                answer = json.loads(line)
            except json.JSONDecodeError:
                continue
    if answer is None:
        raise Failed(f"the import did not run (exit {code}):\n{(errors or text)[-3000:]}")
    if not answer.get("ok", False):
        raise Failed(f"the editor refused the import: {answer.get('error') or json.dumps(answer)[:2000]}")
    done = Done(answer)
    say(f"media assets: {len(done.assets)} written, {len(done.removed)} descriptions said by them removed, "
        f"{len(done.kept)} had theirs, {len(done.problems)} problems")
    for line in done.problems:
        say("  ! " + line)
    return done


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--project", required=True, help="the module project")
    ap.add_argument("--editor", default=EDITOR, help="Grizzly-Cmd.exe (default: the repository's release build)")
    ap.add_argument("--dry", action="store_true", help="say what would be written, write nothing")
    ap.add_argument("--keep-descriptions", action="store_true", help="keep the .thm files the assets say")
    args = ap.parse_args()
    project = os.path.abspath(args.project)
    try:
        done = process(project, os.path.abspath(args.editor), args.dry, remove=not args.keep_descriptions)
    except Failed as e:
        print(f"media assets: {e}", file=sys.stderr)
        return 2
    if not args.dry:
        ledger.follow(project, gone=done.removed, added=done.assets)
    return 1 if done.problems else 0


if __name__ == "__main__":
    sys.exit(main())
