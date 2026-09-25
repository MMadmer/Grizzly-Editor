#!/usr/bin/env python3
"""Every asset kind of the editor's own has its drawn icon in the content browser
(EditorAssetIcons), over MCP, on a THROW-AWAY COPY of a project.

What it proves:
  A  one asset of every kind - quest, behaviour graph, character, character template, PDA
     interface - is made the way an author makes it (content_create, characters_write)
  B  selected in the content browser, each one's tile draws its own icon
     (content_browser_selection 'icons'), and a file of no kind of the editor's own draws none
  C  the tile slider answers over MCP (content_browser_navigation tile=) at both ends of its
     range, and refuses a size outside it; with --shots the tiles are captured at 48 and 192

    python tools/qa/test_asset_icons.py --project D:\\XFinedProjects\\Test [--shots <folder>]
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = ("127.0.0.1", 28016)
KINDS = {
    "quest": "quest",
    "behaviour": "behaviour graph",
    "character": "character",
    "pda": "PDA interface",
}
TEMPLATE = """return {
    npc      = 1,
    id       = "qa_icon_template",
    abstract = true,
    community = "stalker",
}
"""

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 120.0, **extra) -> dict:
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
        deadline = time.monotonic() + 600
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
        # never killed: a killed editor leaves project.ltx behind half written
        try:
            mcp("window_control", action="close")
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def shot(folder: str | None, name: str) -> None:
    if not folder:
        return
    pic = mcp("screenshot_editor")
    if pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)


def run(project: str, shots: str | None) -> None:
    ed = Editor()
    try:
        ed.start(project)
        # a folder of their own: the tiles are all there is to see in it, at any size
        m = mcp("content_mkdir", path="qa_icons")
        nav = mcp("content_browser_navigation", action="navigate", source="project", folder="qa_icons")
        check(m.get("ok") and nav.get("ok"), "A the content browser is in a folder of the test's own",
              json.dumps(nav)[:300])

        # ------------------------------------------------------------ A: one of every kind
        made: dict[str, str] = {}
        for kind in KINDS:
            c = mcp("content_create", kind=kind)
            made[kind] = c.get("made", "")
            check(c.get("ok") and made[kind].replace("/", "\\").lower().startswith("qa_icons\\"),
                  "A Create makes a " + KINDS[kind] + " in the open folder", json.dumps(c)[:300])
        # a template is a character that says abstract = true: the file is all it takes
        template = "qa_icons\\QaTemplate.npcasset"
        with open(os.path.join(project, template), "w", encoding="utf-8", newline="\n") as f:
            f.write(TEMPLATE)
        plain = "qa_icons\\qa_icon_notes.txt"
        with open(os.path.join(project, plain), "w", encoding="utf-8") as f:
            f.write("a file of no kind of the editor's own\n")
        t = mcp("characters_list")
        check(any(c.get("id") == "qa_icon_template" and c.get("abstract") for c in t.get("characters", [])),
              "A the template is a character template to the editor too", json.dumps(t)[:300])

        # ------------------------------------------------------------ B: each tile's icon
        def icon_of(entry: str) -> tuple[str, dict]:
            folder = os.path.dirname(entry).replace("/", "\\")
            n = mcp("content_browser_navigation", action="navigate", source="project", folder=folder)
            s = mcp("content_browser_selection", action="select", name=entry.replace("/", "\\"))
            if not s.get("ok"):
                s = dict(s, entry=entry, navigate=n.get("ok"), folder=(n.get("current") or {}).get("folder"))
            return (s.get("icons") or {}).get(entry.replace("/", "\\"), ""), s

        for kind, entry in made.items():
            icon, answer = icon_of(entry)
            check(icon == KINDS[kind], "B a %s's tile draws its own icon" % KINDS[kind], json.dumps(answer)[:400])
        if template:
            icon, answer = icon_of(template)
            check(icon == "character template", "B a character template's tile draws its own icon",
                  json.dumps(answer)[:400])
        icon, answer = icon_of(plain)
        check(icon == "" and answer.get("ok"), "B a file of no kind of the editor's own draws none",
              json.dumps(answer)[:400])

        # ------------------------------------------------------------ C: the tile slider
        mcp("content_browser_navigation", action="navigate", source="project", folder="qa_icons")
        mcp("content_browser_selection", action="clear")
        for size in (48, 192):
            n = mcp("content_browser_navigation", tile=size)
            check(n.get("ok") and n.get("tile") == size, "C the tile slider goes to %d over MCP" % size,
                  json.dumps(n)[:300])
            shot(shots, "tiles_%d" % size)
        bad = mcp("content_browser_navigation", tile=500)
        check(not bad.get("ok") and "48..192" in str(bad.get("error")), "C a size outside the slider is refused",
              json.dumps(bad)[:300])
        mcp("content_browser_navigation", tile=96)
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for the tiles at 48 and 192")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="xfined_iconsproj_")
    copy = os.path.join(work, "QaIcons")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git"))
    print("running against a copy: " + copy, flush=True)
    # project.ltx names this machine's paths: the copy must name its own folder
    pl = os.path.join(copy, "project.ltx")
    if os.path.isfile(pl):
        with open(pl, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        low, at = text.lower(), text.lower().find(src.lower())
        while at >= 0:
            text = text[:at] + copy + text[at + len(src):]
            low = text.lower()
            at = low.find(src.lower(), at + len(copy))
        with open(pl, "w", encoding="utf-8", newline="") as f:
            f.write(text)
    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()
    try:
        run(copy, args.shots)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
    ok = sum(1 for good, _ in results if good)
    print(f"ASSET ICONS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
