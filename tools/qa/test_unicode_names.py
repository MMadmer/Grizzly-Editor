#!/usr/bin/env python3
"""File names are UTF-8 from the disk to the screen and back.

The editor speaks to the file system in UTF-16 and to the window and MCP in UTF-8: no name goes through a
code page. This
drives the editor over MCP on a THROW-AWAY COPY of a project holding a Cyrillic folder,
a Cyrillic file, a name outside cp1251 altogether and a quest with a Cyrillic file name,
and checks every way a name travels: the content browser, the rename box, a rename to
disk, the quest list, the build, the release package and its file index.

A name outside cp1251 tells the two code pages apart: under cp1251 the file APIs hand
it back as question marks and nothing can open it. A long Cyrillic name (more bytes than
the rename box once held) and the browser's search (case folded, Cyrillic too) ride along.

    python tools/qa/test_unicode_names.py --project <project> [--shots <folder>]
"""
from __future__ import annotations

import base64
import glob
import io
import json
import os
import sys
import tempfile
import zipfile


import qa
from qa import check


def cmd(name: str, /, escaped: bool = False, **params) -> dict:
    """Raw UTF-8 by default; escaped=True sends non-ASCII as escapes, the way the MCP bridge does."""
    return qa.cmd(name, escaped=escaped, **params)

FOLDER = "qa_юникод"
CYRILLIC = "Ночная доставка.txt"
RENAMED = "Сюжет ✓.txt"          # the check mark is outside cp1251
BEYOND = "日本の地図.txt"          # nothing of it is in cp1251
QUEST_FILE = "Доставка.nqasset"
# over 127 bytes in UTF-8: the rename box used to cut it short
LONG = "Очень длинное название файла, которое не помещалось в прежнее поле переименования.txt"

QUEST = """return {
    nq = 1, id = "qa_unicode_quest", title = "Unicode",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

def write(root: str, rel: str, text: str) -> None:
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def same(entry: str, listed: list) -> bool:
    """the entry, whatever separator the answer uses"""
    key = entry.replace("/", "\\").lower()
    return any(str(x).replace("/", "\\").lower() == key for x in listed)


def run(ctx: qa.Run) -> None:
    project = ctx.project
    shot = os.path.join(ctx.shots, "cyrillic.png") if ctx.shots else None
    if shot:
        os.makedirs(ctx.shots, exist_ok=True)
    write(project, f"gamedata/{FOLDER}/{CYRILLIC}", "night\n")
    write(project, f"gamedata/{FOLDER}/{BEYOND}", "map\n")
    write(project, f"gamedata/{FOLDER}/{LONG}", "long\n")
    write(project, f"quests/{QUEST_FILE}", QUEST)
    folder = os.path.join(project, "gamedata", FOLDER)

    ed = qa.Editor(project)
    target = tempfile.mkdtemp(prefix="grizzly_unicode_target_")
    pkg = tempfile.mkdtemp(prefix="grizzly_unicode_pkg_")
    try:
        ed.start()
        module = cmd("mod.info").get("manifest", {}).get("id", "")

        # ------------------------------------------------------------------ B: the browser
        r = cmd("content.view", action="reveal", source="project", path=f"gamedata/{FOLDER}/")
        check(r.get("ok"), "B a folder with a Cyrillic name opens in the content browser", json.dumps(r, ensure_ascii=False)[:200])
        entry = f"gamedata\\{FOLDER}\\{CYRILLIC}"
        r = cmd("content.view", action="select", path=entry)
        check(r.get("ok") and same(entry, r.get("selection", [])),
              "B a Cyrillic file is listed under its own name", json.dumps(r, ensure_ascii=False)[:240])
        beyond = f"gamedata\\{FOLDER}\\{BEYOND}"
        r = cmd("content.view", action="select", path=beyond)
        check(r.get("ok") and same(beyond, r.get("selection", [])),
              "B so is a name outside cp1251", json.dumps(r, ensure_ascii=False)[:240])
        r = cmd("content.view", action="rename", path=entry)
        box = (r.get("rename") or {}).get("text", "")
        check(r.get("ok") and box == CYRILLIC, "B the rename box holds the name as it is", repr(box))
        cmd("content.view", action="clear")
        long_entry = f"gamedata\\{FOLDER}\\{LONG}"
        r = cmd("content.view", action="rename", path=long_entry)
        box = (r.get("rename") or {}).get("text", "")
        check(r.get("ok") and box == LONG and len(LONG.encode("utf-8")) > 127,
              "B the rename box holds a long Cyrillic name whole (%d bytes)" % len(LONG.encode("utf-8")), repr(box))
        cmd("content.view", action="clear")

        # the search folds case, Cyrillic included, and lists only what matches
        r = cmd("content.view", query="ночная")
        check(r.get("ok") and r.get("query") == "ночная", "B the search takes a Cyrillic query",
              json.dumps(r, ensure_ascii=False)[:240])
        hit = cmd("content.view", action="select", path=entry)
        miss = cmd("content.view", action="select", path=beyond)
        check(hit.get("ok") and not miss.get("ok"),
              "B a lower-case Cyrillic query finds the capitalised name, and only what matches",
              json.dumps(hit, ensure_ascii=False)[:160] + " | " + json.dumps(miss, ensure_ascii=False)[:160])
        r = cmd("content.view", query="")
        check(r.get("ok") and r.get("query") == "", "B an empty search lists the folder again", json.dumps(r)[:200])
        cmd("content.view", action="clear")
        if shot:
            cmd("content.view", action="select", path=entry)
            pic = cmd("shot.window")
            if pic.get("png_base64"):
                with open(shot, "wb") as f:
                    f.write(base64.b64decode(pic["png_base64"]))
                print("screenshot: " + shot, flush=True)

        # ------------------------------------------------------------------ C: a rename reaches the disk
        r = cmd("file.rename", escaped=True, path=f"gamedata/{FOLDER}/{CYRILLIC}", name=RENAMED)
        on_disk = sorted(os.listdir(folder))
        check(r.get("ok") and RENAMED in on_disk and CYRILLIC not in on_disk,
              "C a rename sent as \\u escapes (like the bridge) writes that name to disk, not its bytes read as cp1251",
              json.dumps(r, ensure_ascii=False)[:160] + " disk=" + repr(on_disk))

        # ------------------------------------------------------------------ D: quests
        quests = [q.get("extra", "") for q in cmd("game.lookup", type="quest_id", limit=500).get("entries", [])]
        check(any(q.replace("\\", "/").endswith("quests/" + QUEST_FILE) for q in quests),
              "D a quest with a Cyrillic file name is found under that name", repr(quests)[:240])

        # ------------------------------------------------------------------ E: the build
        built = cmd("mod.build", target="folder", path=target.replace("\\", "/"))
        shipped = os.path.join(target, "modules", module, "gamedata", FOLDER)
        names = sorted(os.listdir(shipped)) if os.path.isdir(shipped) else []
        check(built.get("ok") and RENAMED in names and BEYOND in names,
              "E the build ships both files under their own names",
              json.dumps(built, ensure_ascii=False)[:200] + " shipped=" + repr(names))
        left = json.dumps(built.get("left_out", []), ensure_ascii=False)
        check(FOLDER not in left, "E and leaves neither out", left[:200])
        quest_out = os.path.join(target, "modules", module, "quests", QUEST_FILE)
        check(os.path.isfile(quest_out), "E the quest ships under its Cyrillic name", quest_out)
        # the game reads a module through cp1251: the two names outside it never open there
        warned = built.get("warnings", "")
        block = warned.split("file names the game cannot open", 1)[1] if "file names the game cannot open" in warned else ""
        check(RENAMED in block and BEYOND in block and QUEST_FILE not in block,
              "E the build warns about the two names outside cp1251, and only those", warned[-300:])

        # ------------------------------------------------------------------ F: the release package
        pk = cmd("mod.package", path=pkg.replace("\\", "/"))
        zips = sorted(glob.glob(os.path.join(pkg, "*.zip")))
        entries: dict[str, zipfile.ZipInfo] = {}
        for z in zips:
            with zipfile.ZipFile(z) as zf:
                for info in zf.infolist():
                    entries[info.filename] = info
        want = [f"modules/{module}/gamedata/{FOLDER}/{RENAMED}", f"modules/{module}/gamedata/{FOLDER}/{BEYOND}"]
        check(pk.get("ok") and all(w in entries for w in want),
              "F the package holds both under their own names",
              json.dumps(pk, ensure_ascii=False)[:160] + " " + repr([e for e in entries if FOLDER in e]))
        check(all(w in entries and entries[w].flag_bits & 0x800 for w in want),
              "F and marks those names as UTF-8 (zip flag 11)")
        index = sorted(glob.glob(os.path.join(pkg, "*.files")))
        text = io.open(index[0], encoding="utf-8").read() if index else ""
        check(all(w.split(f"modules/{module}/", 1)[1] in text.replace("\\", "/") for w in want),
              "F the file index names them the same way", text[:200])

        # -------------------------------------------------------------- G: a name from the disk and back
        # The Rust editor speaks to the file system in UTF-16 alone: no code page is involved, so its log
        # line about one says nothing. What is checked is the round trip itself, with a name beyond the
        # Basic Multilingual Plane (a UTF-16 surrogate pair): made on disk behind the editor's back, listed
        # by the browser under that name, renamed by the editor to another such name, found on disk so.
        made = "Медведь \U0001F43B.txt"
        moved = "Медведь \U0001F43B\U0001F332.txt"
        with open(os.path.join(folder, made), "wb") as f:
            f.write(b"bear")
        cmd("content.view", action="reveal", source="project", path=f"gamedata/{FOLDER}/")
        cmd("content.view", action="refresh")
        entry = f"gamedata\\{FOLDER}\\{made}"
        r = cmd("content.view", action="select", path=entry)
        check(r.get("ok") and same(entry, r.get("selection", [])),
              "G a name made on disk with a character beyond the BMP is listed under that name",
              json.dumps(r, ensure_ascii=False)[:240])
        cmd("content.view", action="clear")
        r = cmd("file.rename", escaped=True, path=f"gamedata/{FOLDER}/{made}", name=moved)
        on_disk = os.listdir(folder)
        check(r.get("ok") and moved in on_disk and made not in on_disk,
              "G and the editor's rename writes such a name to disk as it is",
              json.dumps(r, ensure_ascii=False)[:160] + " disk=" + repr(on_disk))
    finally:
        ed.close()
        for d in (target, pkg):
            qa.remove_tree(d)


def main() -> int:
    return qa.suite("UNICODE NAMES", run, doc=__doc__, ignore=(".git", ".gitignore"))


if __name__ == "__main__":
    sys.exit(main())
