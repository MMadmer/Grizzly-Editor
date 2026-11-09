#!/usr/bin/env python3
"""Every asset kind of the editor's own has its drawn icon in the content browser
(EditorAssetIcons), over MCP, on a THROW-AWAY COPY of a project.

What it proves:
  A  one asset of every kind - quest, behaviour graph, character, character template, PDA
     interface - is made the way an author makes it (content_create, characters_write)
  B  selected in the content browser, each one's tile draws its own icon
     (content_browser_selection 'icons'), and a file of no kind of the editor's own draws none
  C  the tile size answers over MCP (content_browser_navigation tile=, in pt) at both ends of its
     range, shows tiles, and refuses a size outside it; with no size picked (tile_fit) the tiles fit
     the host, 48 pt up to its default; view=list turns the grid into the list and view=tiles back,
     anything else is refused; with --shots the tiles are captured fitted, at 48 and 192, and the list
  D  every tile draws a vector glyph while it has no picture - a folder its folder, a text file its
     file type, one of the editor's own assets its own icon (content_browser_selection 'glyphs');
     the search narrows the view and the Sources pane turns off and on over MCP; Show in Explorer
     answers the path it would select (dry run); the toolbar's Add is off, saying why, on a
     read-only source and on in the project; with --shots the Add menu is captured

    python tools/qa/test_asset_icons.py --project <project> [--shots <folder>]
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
        contract = mcp("ui_contract")
        low, high = (contract.get("tile_range") or [48, 192])[:2]
        # the host shown keeps a size and a view of its own (the docked panel starts as a list, the drawer as tiles):
        # put back those
        before = mcp("content_browser_navigation")
        kept = before.get("tile") or contract.get("tile_default", 96)
        kept_fit = before.get("tile_fit")
        kept_view = before.get("view") or "tiles"
        check(kept_view in ("tiles", "list"), "C the answer says how the grid shows its entries", json.dumps(before)[:300])
        check(isinstance(kept_fit, bool), "C the answer says whether the tiles fit the host", json.dumps(before)[:300])
        # with no size picked the tiles fit the host: two rows where they can, never over its default, never under 48
        fitted = mcp("content_browser_navigation", tile_fit=True)
        most = contract.get("tile_default_panel", 64) if fitted.get("host") == "panel" else contract.get("tile_default", 96)
        check(fitted.get("ok") and fitted.get("tile_fit") is True and fitted.get("view") == "tiles"
              and low <= (fitted.get("tile") or 0) <= most,
              "C tile_fit fits the tiles to the host (48..%d pt), and shows tiles" % most, json.dumps(fitted)[:300])
        shot(shots, "tiles_fit")
        bad = mcp("content_browser_navigation", tile=high, tile_fit=True)
        check(not bad.get("ok") and "tile_fit" in str(bad.get("error")), "C a size and tile_fit=true together are refused",
              json.dumps(bad)[:300])
        for size in (low, high):
            n = mcp("content_browser_navigation", tile=size)
            check(n.get("ok") and n.get("tile") == size and n.get("tile_unit") == "pt" and n.get("view") == "tiles"
                  and n.get("tile_fit") is False,
                  "C the tile size goes to %d pt over MCP, shows tiles and is no longer fitted" % size, json.dumps(n)[:300])
            shot(shots, "tiles_%d" % size)
        bad = mcp("content_browser_navigation", tile=500)
        check(not bad.get("ok") and "48..192" in str(bad.get("error")), "C a size outside the range is refused",
              json.dumps(bad)[:300])
        listed = mcp("content_browser_navigation", view="list")
        check(listed.get("ok") and listed.get("view") == "list" and listed.get("tile") == high,
              "C view=list shows the list and keeps the tile size", json.dumps(listed)[:300])
        shot(shots, "list")
        bad = mcp("content_browser_navigation", view="grid")
        check(not bad.get("ok") and "tiles or list" in str(bad.get("error")), "C a view other than tiles or list is refused",
              json.dumps(bad)[:300])
        tiled = mcp("content_browser_navigation", view="tiles")
        check(tiled.get("ok") and tiled.get("view") == "tiles", "C view=tiles shows the tiles again", json.dumps(tiled)[:300])
        if kept_fit:
            mcp("content_browser_navigation", tile_fit=True)
        else:
            mcp("content_browser_navigation", tile=kept)
        mcp("content_browser_navigation", view=kept_view)

        # ------------------------------------------------------------ D: glyphs, search, pane, reveal
        mcp("content_mkdir", path="qa_icons/qa_sub")
        mcp("content_browser_navigation", action="navigate", source="project", folder="qa_icons")
        want = {"qa_icons\\qa_sub": "folder", plain: "file_text", made["quest"].replace("/", "\\"): "quest"}
        for entry, glyph in want.items():
            s = mcp("content_browser_selection", action="select", name=entry)
            key = entry + ("\\" if glyph == "folder" else "")
            check(s.get("ok") and (s.get("glyphs") or {}).get(key) == glyph,
                  "D a %s tile draws the '%s' glyph" % (entry.rsplit("\\", 1)[-1], glyph), json.dumps(s)[:400])
        mcp("content_browser_selection", action="clear")

        n = mcp("content_browser_navigation", search="notes")
        hit = mcp("content_browser_selection", action="select", name=plain)
        miss = mcp("content_browser_selection", action="select", name=made["quest"].replace("/", "\\"))
        check(n.get("ok") and n.get("search") == "notes" and hit.get("ok") and not miss.get("ok"),
              "D the search lists what matches and nothing else",
              json.dumps(n)[:200] + " | " + json.dumps(miss)[:200])
        n = mcp("content_browser_navigation", search="")
        check(n.get("ok") and n.get("search") == "", "D an empty search lists the folder again", json.dumps(n)[:200])
        mcp("content_browser_selection", action="clear")

        off = mcp("content_browser_navigation", sources_pane=False)
        on = mcp("content_browser_navigation", sources_pane=True)
        check((off.get("sources_pane") or {}).get("on") is False and (on.get("sources_pane") or {}).get("on") is True,
              "D the Sources pane turns off and on over MCP", json.dumps(off)[:200] + " | " + json.dumps(on)[:200])

        r = mcp("content_browser_selection", action="reveal", name=plain, dry_run=True)
        check(r.get("ok") and not r.get("opened") and str(r.get("path", "")).lower().endswith(plain.lower()),
              "D Show in Explorer answers the file it would select, and opens nothing on a dry run", json.dumps(r)[:300])

        # Add works in the project's own source only: on a read-only one its button is off and says why. The audit is
        # the last frame's, and a background editor draws only when asked: a screenshot draws one first.
        def add_button() -> dict:
            mcp("screenshot_editor")
            return next((c for c in mcp("ui_audit").get("controls", [])
                         if c.get("label") == "Add" and "Content" in str(c.get("window", ""))), {})

        ro = mcp("content_browser_navigation", action="home", source="editor")
        add = add_button()
        check(ro.get("ok") and ro.get("can_add") is False and add and add.get("enabled") is False and add.get("why"),
              "D Add is off on a read-only source, and says why", json.dumps(ro)[:200] + " | " + json.dumps(add)[:300])
        rw = mcp("content_browser_navigation", action="navigate", source="project", folder="qa_icons")
        add = add_button()
        check(rw.get("ok") and rw.get("can_add") is True and add.get("enabled") is True,
              "D and on in the project's own source", json.dumps(rw)[:200] + " | " + json.dumps(add)[:300])

        if shots:
            p = mcp("ui_popup", show="cb_add")
            check(p.get("ok"), "D the Add menu comes up over MCP", json.dumps(p)[:200])
            shot(shots, "add_menu")
            mcp("ui_popup", show="none")
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for the tiles at 48 and 192 and the list")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_iconsproj_")
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
