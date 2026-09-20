#!/usr/bin/env python3
"""End-to-end check of the module's own characters (EditorCharacters).

Drives the editor over MCP on a THROW-AWAY COPY of a project: writes characters,
reads them back, builds the module into a temporary folder and verifies that the
generated half is exactly what the game needs to load them — the character_desc
entry, the profile, the string table, the spawn and squad sections, and the
.ltxp patch that appends (never replaces) the two profile lists.

    python tools/qa/test_characters.py --project D:\\XFinedProjects\\Test
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = ("127.0.0.1", 28016)

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


class Editor:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self, project: str) -> None:
        exe = os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        self.proc = subprocess.Popen([exe, "-nodlg", "-project", project],
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


COURIER = """return {
    npc       = 1,
    id        = "qa_courier",
    name      = { rus = "Kurier", eng = "Courier" },
    bio       = "QA character",
    community = "stalker",
    visual    = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
    icon      = "ui_npc_u_stalker_neutral_1",
    rank      = 300,
    money     = { min = 500, max = 1500 },
    supplies  = {
        { section = "wpn_ak74u", count = 1 },
        { section = "ammo_5.45x39_fmj", count = 3 },
        { section = "medkit" },
    },
}
"""

GUARD = """return {
    npc       = 1,
    id        = "qa_guard",
    name      = "Guard",
    community = "dolg",
    visual    = "actors\\\\stalker_dolg\\\\stalker_dolg_1",
    squad     = false,
}
"""

BROKEN = """return {
    npc       = 1,
    id        = "QA Bad Id",
    community = "stalker",
}
"""


def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("cp1251", "replace")


def run(project: str) -> None:
    ed = Editor()
    out_dir = tempfile.mkdtemp(prefix="xfined_chars_")
    try:
        ed.start(project)

        manifest = mcp("mod_manifest")
        module = manifest.get("id", "")
        check(bool(module), "A the project has a module id", json.dumps(manifest)[:200])

        # ---------------------------------------------------------------- A: write
        w = mcp("characters_write", text=COURIER)
        check(w.get("ok"), "A a character is written", json.dumps(w)[:200])
        check(w.get("character", {}).get("id") == "qa_courier", "A it comes back with its id")
        check(w.get("problems") == [], "A and with no problems", json.dumps(w.get("problems")))

        w2 = mcp("characters_write", text=GUARD)
        check(w2.get("ok"), "A a second character is written", json.dumps(w2)[:200])

        lst = mcp("characters_list")
        ids = sorted(c["id"] for c in lst.get("characters", []))
        check(ids == ["qa_courier", "qa_guard"], "A both are listed", str(ids))

        got = mcp("characters_get", id="qa_courier")
        check(got.get("ok") and "qa_courier" in got.get("text", ""), "A one reads back whole")
        check(got.get("character", {}).get("supplies") == 3, "A its kit has three rows",
              str(got.get("character")))

        # canonical: writing what was read changes nothing
        again = mcp("characters_write", id="qa_courier", text=got.get("text", ""))
        again_text = mcp("characters_get", id="qa_courier").get("text", "")
        check(again.get("ok") and again_text == got.get("text", ""),
              "A write(read(x)) is the same file")

        # ---------------------------------------------------------------- B: refusals
        bad = mcp("characters_write", text=BROKEN)
        problems = " ".join(bad.get("problems", []))
        check(bad.get("ok") and "E102" in problems, "B an id with spaces is reported", problems)
        check("E103" in problems, "B and so is a missing name", problems)
        check("E104" in problems, "B and a missing visual", problems)
        mcp("characters_delete", id="QA Bad Id")

        # ---------------------------------------------------------------- C: the build
        built = mcp("mod_export", target=out_dir)
        check(built.get("ok"), "C the module builds", json.dumps(built)[:300])

        root = os.path.join(out_dir, "modules", module)
        desc = os.path.join(root, "gamedata", "configs", "gameplay", f"character_desc_{module}.xml")
        prof = os.path.join(root, "gamedata", "configs", "gameplay", f"npc_profile_{module}.xml")
        ltx = os.path.join(root, "gamedata", "configs", "xms", f"{module}_npc.ltx")
        patch = os.path.join(root, "patch", f"{module}_characters.ltxp")
        st_rus = os.path.join(root, "gamedata", "configs", "text", "rus", f"{module}_npc.xml")
        st_eng = os.path.join(root, "gamedata", "configs", "text", "eng", f"{module}_npc.xml")

        for path, what in ((desc, "character_desc"), (prof, "npc_profile"),
                           (ltx, "spawn sections"), (patch, "the ltxp patch"),
                           (st_rus, "the rus string table"), (st_eng, "the eng string table")):
            check(os.path.isfile(path), f"C {what} is in the module", path)

        gid = f"{module}_qa_courier"
        d = read(desc)
        check(f'<specific_character id="{gid}"' in d, "C the character is described", d[:200])
        check("<visual>actors\\stalker_neutral\\stalker_neutral_1</visual>" in d,
              "C with its model")
        check(f"<name>{gid}_name</name>" in d, "C and a string-table name, not a literal")
        check("wpn_ak74u \\n" in d and "ammo_5.45x39_fmj = 3 \\n" in d and "medkit \\n" in d,
              "C its kit is written the way the engine reads one")
        check('<money min="500" max="1500" infinitive="0"/>' in d, "C and its money")

        p = read(prof)
        check(f'<character id="{gid}">' in p and f"<specific_character>{gid}</specific_character>" in p,
              "C the profile names the character outright (no random pick)")

        l = read(ltx)
        check(f"[{gid}]:stalker" in l, "C the spawn section inherits stalker", l[:200])
        check(f"character_profile     = {gid}" in l, "C and carries the profile")
        check("$spawn" in l, "C and is spawnable, so it shows up in the spawn roster")
        check(f"[{gid}_squad]:online_offline_group" in l, "C a one-man squad exists for spawn.squad")
        check(f"[{module}_qa_guard_squad]" not in l, "C and squad = false leaves none")

        t = read(patch)
        check("+files = npc_profile_" + module in t, "C the patch APPENDS the profile file")
        check("+specific_characters_files = character_desc_" + module in t,
              "C and appends the character file")
        check("[profiles]" in t and "-files" not in t, "C nothing of the game is replaced")

        s_rus = read(st_rus)
        s_eng = read(st_eng)
        check(f'<string id="{gid}_name">' in s_rus and "Kurier" in s_rus, "C rus name is in place")
        check("Courier" in s_eng, "C eng name is in place")
        check(f"{module}_qa_guard_name" in s_rus and "Guard" in s_rus,
              "C a single-language name lands in rus")

        # ---------------------------------------------------------------- D: removal
        d1 = mcp("characters_delete", id="qa_guard")
        check(d1.get("ok"), "D a character can be removed", json.dumps(d1)[:200])
        d2 = mcp("characters_delete", id="qa_courier")
        check(d2.get("ok"), "D and so can the last one")

        built2 = mcp("mod_export", target=out_dir)
        check(built2.get("ok"), "D the module builds again", json.dumps(built2)[:200])
        gone = [os.path.isfile(x) for x in (desc, prof, ltx, patch, st_rus, st_eng)]
        check(not any(gone), "D the generated half went with them", str(gone))

        # the project is left as it was found
        left = mcp("characters_list").get("characters", [])
        check(left == [], "D no character is left in the project", str(left))

    finally:
        ed.close()
        shutil.rmtree(out_dir, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="xfined_charproj_")
    copy = os.path.join(work, "QaCharacters")
    shutil.copytree(src, copy)
    print("running against a copy: " + copy, flush=True)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()

    try:
        run(copy)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"CHARACTERS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
