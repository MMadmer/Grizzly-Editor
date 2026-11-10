#!/usr/bin/env python3
"""File names are UTF-8 from the disk to the screen and back.

The editor runs with UTF-8 as its ANSI code page (LevelEditor.manifest). Before, a
Cyrillic name came out of the ANSI file APIs as cp1251 bytes that ImGui cannot draw,
and a name typed into the editor reached the disk as UTF-8 bytes read as cp1251. This
drives the editor over MCP on a THROW-AWAY COPY of a project holding a Cyrillic folder,
a Cyrillic file, a name outside cp1251 altogether and a quest with a Cyrillic file name,
and checks every way a name travels: the content browser, the rename box, a rename to
disk, the quest list, the build, the release package and its file index.

A name outside cp1251 tells the two code pages apart: under cp1251 the file APIs hand
it back as question marks and nothing can open it. A long Cyrillic name (more bytes than
the rename box once held) and the browser's search (case folded, Cyrillic too) ride along.

    python tools/qa/test_unicode_names.py --project <project> [--shot out.png]
"""
from __future__ import annotations

import argparse
import base64
import glob
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import zipfile

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP

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

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 300.0, escaped: bool = False, **extra) -> dict:
    """escaped=True sends non-ASCII as \\u escapes, the way the MCP bridge does; raw UTF-8 otherwise"""
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload, ensure_ascii=escaped) + "\n").encode("utf-8"))
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    # strict: an answer that is not valid UTF-8 is itself a failure of this suite
    return json.loads(buf.decode("utf-8"))


class Editor:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self, project: str) -> None:
        exe = qa_target.exe()
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project],
                                     cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(1)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("window_control", action="close")
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def write(root: str, rel: str, text: str) -> None:
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def same(entry: str, listed: list) -> bool:
    """the entry, whatever separator the answer uses"""
    key = entry.replace("/", "\\").lower()
    return any(str(x).replace("/", "\\").lower() == key for x in listed)


def run(project: str, shot: str | None) -> None:
    write(project, f"gamedata/{FOLDER}/{CYRILLIC}", "night\n")
    write(project, f"gamedata/{FOLDER}/{BEYOND}", "map\n")
    write(project, f"gamedata/{FOLDER}/{LONG}", "long\n")
    write(project, f"quests/{QUEST_FILE}", QUEST)
    folder = os.path.join(project, "gamedata", FOLDER)

    ed = Editor()
    target = tempfile.mkdtemp(prefix="grizzly_unicode_target_")
    pkg = tempfile.mkdtemp(prefix="grizzly_unicode_pkg_")
    started = time.time()
    try:
        ed.start(project)
        module = mcp("mod_manifest").get("id", "")

        # ------------------------------------------------------------------ B: the browser
        r = mcp("content_browser_open", name=f"gamedata/{FOLDER}", source="project")
        check(r.get("ok"), "B a folder with a Cyrillic name opens in the content browser", json.dumps(r, ensure_ascii=False)[:200])
        entry = f"gamedata\\{FOLDER}\\{CYRILLIC}"
        r = mcp("content_browser_selection", action="select", name=entry)
        check(r.get("ok") and same(entry, r.get("selection", [])),
              "B a Cyrillic file is listed under its own name", json.dumps(r, ensure_ascii=False)[:240])
        beyond = f"gamedata\\{FOLDER}\\{BEYOND}"
        r = mcp("content_browser_selection", action="select", name=beyond)
        check(r.get("ok") and same(beyond, r.get("selection", [])),
              "B so is a name outside cp1251", json.dumps(r, ensure_ascii=False)[:240])
        r = mcp("content_browser_selection", action="rename", name=entry)
        box = (r.get("rename") or {}).get("text", "")
        check(r.get("ok") and box == CYRILLIC, "B the rename box holds the name as it is", repr(box))
        mcp("content_browser_selection", action="clear")
        long_entry = f"gamedata\\{FOLDER}\\{LONG}"
        r = mcp("content_browser_selection", action="rename", name=long_entry)
        box = (r.get("rename") or {}).get("text", "")
        check(r.get("ok") and box == LONG and len(LONG.encode("utf-8")) > 127,
              "B the rename box holds a long Cyrillic name whole (%d bytes)" % len(LONG.encode("utf-8")), repr(box))
        mcp("content_browser_selection", action="clear")

        # the search folds case, Cyrillic included, and lists only what matches
        r = mcp("content_browser_navigation", search="ночная")
        check(r.get("ok") and r.get("search") == "ночная", "B the search takes a Cyrillic query",
              json.dumps(r, ensure_ascii=False)[:240])
        hit = mcp("content_browser_selection", action="select", name=entry)
        miss = mcp("content_browser_selection", action="select", name=beyond)
        check(hit.get("ok") and not miss.get("ok"),
              "B a lower-case Cyrillic query finds the capitalised name, and only what matches",
              json.dumps(hit, ensure_ascii=False)[:160] + " | " + json.dumps(miss, ensure_ascii=False)[:160])
        r = mcp("content_browser_navigation", search="")
        check(r.get("ok") and r.get("search") == "", "B an empty search lists the folder again", json.dumps(r)[:200])
        mcp("content_browser_selection", action="clear")
        if shot:
            mcp("content_browser_selection", action="select", name=entry)
            pic = mcp("screenshot_editor")
            if pic.get("png_base64"):
                with open(shot, "wb") as f:
                    f.write(base64.b64decode(pic["png_base64"]))
                print("screenshot: " + shot, flush=True)

        # ------------------------------------------------------------------ C: a rename reaches the disk
        r = mcp("content_rename", escaped=True, path=f"gamedata/{FOLDER}/{CYRILLIC}", name=RENAMED)
        on_disk = sorted(os.listdir(folder))
        check(r.get("ok") and RENAMED in on_disk and CYRILLIC not in on_disk,
              "C a rename sent as \\u escapes (like the bridge) writes that name to disk, not its bytes read as cp1251",
              json.dumps(r, ensure_ascii=False)[:160] + " disk=" + repr(on_disk))

        # ------------------------------------------------------------------ D: quests
        quests = [q.get("path", "") for q in mcp("quest_list").get("quests", [])]
        check(any(q.replace("\\", "/").endswith("quests/" + QUEST_FILE) for q in quests),
              "D a quest with a Cyrillic file name is found under that name", repr(quests)[:240])

        # ------------------------------------------------------------------ E: the build
        built = mcp("mod_export", target=target.replace("\\", "/"))
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
        pk = mcp("mod_package", out_dir=pkg.replace("\\", "/"))
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
    finally:
        ed.close()
        for d in (target, pkg):
            shutil.rmtree(d, ignore_errors=True)

    # ---------------------------------------------------------------------- G: the process
    logs = [p for p in glob.glob(os.path.join(REPO, "logs", "leveleditor_*.log"))
            if not p.endswith("_lua.log") and os.path.getmtime(p) >= started]
    text = io.open(logs[0], encoding="utf-8", errors="replace").read() if logs else ""
    check("file names: UTF-8" in text, "G the editor ran with UTF-8 as its ANSI code page",
          "log: " + (logs[0] if logs else "none written since the start"))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True)
    ap.add_argument("--shot", help="save a screenshot of the browser showing the Cyrillic file")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="grizzly_unicodeproj_")
    copy = os.path.join(work, "QaUnicodeNames")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git", ".gitignore"))
    print("running against a copy: " + copy, flush=True)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()

    try:
        run(copy, os.path.abspath(args.shot) if args.shot else None)
    except Exception as exc:  # noqa: BLE001
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"UNICODE NAMES: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
