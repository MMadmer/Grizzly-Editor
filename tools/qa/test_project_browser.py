#!/usr/bin/env python3
"""The project browser's tile menu - Show in Explorer, Delete..., Remove from List - over MCP
(EditorProject::CheckDelete / DeleteProject), on THROW-AWAY projects in a temp folder only.

Nothing here touches a folder the test did not make: _projects.ini is replaced for the run by a list
of the test's own projects (so the editor lists nothing else) and put back byte for byte at the end,
and every path the test hands the editor lies inside its temp folder.

What it proves:
  A  the list: every entry is a tile, a gone folder included, each with its state
  B  the menu and the confirmation on screen, to look at only: the question names what goes (the
     counts match the deletion), a refusal says why, a gone tile offers Remove from List; nothing
     is deleted by looking
  C  a deletion: the folder is gone - a read-only file, a read-only folder, a hidden .git and the
     packaged releases with it -, a junction inside goes as a link while the folder it points at
     keeps its files, the entry leaves the list, the answer counts what went
  D  refusals, each deleting nothing: not a project, a path outside the list, a relative path, a
     folder holding another project (named), the project folder being a junction (named where it
     points), a folder holding a listed project's game, a folder inside one; a missing folder only
     leaves the list
  E  a file in use: named, the folder and its manifest stay, the entry stays; once the file is free
     a second call finishes the job
  F  Remove from List: refused for a project, done for a folder without one, nothing on disk changes
  G  Show in Explorer: refusals, the folder it resolves, and one real window, closed again
  H  the open project is refused, and the menu is not offered while a project is open
  Z  _projects.ini is back byte for byte

    python tools/qa/test_project_browser.py [--shots <folder>] [--no-explorer] [--keep]
"""
from __future__ import annotations

import _winapi
import argparse
import base64
import json
import os
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = ("127.0.0.1", 28016)
INI = os.path.join(REPO, "_projects.ini")

results: list[tuple[bool, str]] = []


def check(ok, text: str, detail: str = "") -> bool:
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


def short(data: dict) -> str:
    return json.dumps(data)[:600]


def port_in_use() -> bool:
    try:
        with socket.create_connection(MCP, timeout=1):
            return True
    except OSError:
        return False


class Editor:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self) -> dict:
        exe = os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        # no -project: the project browser is what this test is about
        self.proc = subprocess.Popen([exe, "-nodlg"], cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                st = mcp("state", timeout=30)
                if st.get("ok"):
                    return st
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(1)
        raise RuntimeError("the editor never answered")

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
    """The editor as it looks now. Always taken - it is what makes a background editor draw the frames
    the answers after it report on -, saved only when --shots names a folder."""
    pic = mcp("screenshot_editor")
    if folder and pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)
    elif not pic.get("ok"):
        print("! screenshot %s: %s" % (name, pic.get("error")), flush=True)


def fwd(path: str) -> str:
    """How the editor's answers spell a path."""
    return path.replace("\\", "/")


def same(a: str, b: str) -> bool:
    return fwd(a).rstrip("/").lower() == fwd(b).rstrip("/").lower()


def write(path: str, text: str) -> int:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    return os.path.getsize(path)


def set_attributes(path: str, readonly: bool = False, hidden: bool = False) -> None:
    import ctypes
    attrs = ctypes.windll.kernel32.GetFileAttributesW(path)
    if readonly:
        attrs |= 0x1
    if hidden:
        attrs |= 0x2
    ctypes.windll.kernel32.SetFileAttributesW(path, attrs)


def is_junction(path: str) -> bool:
    try:
        st = os.lstat(path)
    except OSError:
        return False
    return bool(st.st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT) and \
        st.st_reparse_tag == stat.IO_REPARSE_TAG_MOUNT_POINT


def listed(state: dict | None = None) -> dict:
    """list_projects as {lowered forward-slash path: entry}."""
    data = state or mcp("list_projects")
    return {fwd(p.get("path", "")).lower(): p for p in data.get("projects", [])}


class Work:
    """The throw-away projects, all inside one temp folder."""

    def __init__(self) -> None:
        self.root = os.path.realpath(tempfile.mkdtemp(prefix="xfined_projbrowser_"))
        self.p = {}
        self.expect = {}

    def path(self, *parts: str) -> str:
        return os.path.join(self.root, *parts)

    def inside(self, path: str) -> bool:
        full = os.path.abspath(path)
        return os.path.commonpath([self.root.lower(), full.lower()]) == self.root.lower()

    def build(self) -> None:
        r = self.root
        mod = lambda i: "[module]\nid = %s\nname = %s\nversion = 1.0.0\n" % (i, i)

        # C: the project that is deleted, with everything a real one gathers
        d = self.p["delete"] = self.path("QaDelete")
        sizes = [
            write(os.path.join(d, "project.ltx"), "[project]\nname = QaDelete\n"),
            write(os.path.join(d, "mod.ltx"), mod("qa_delete")),
            write(os.path.join(d, "readonly.txt"), "git and CDs leave files read-only\n"),
            write(os.path.join(d, "gamedata", "configs", "qa_delete.ltx"), "[qa_delete]\nvalue = 1\n"),
            write(os.path.join(d, ".git", "HEAD"), "ref: refs/heads/main\n"),
            write(os.path.join(d, ".git", "config"), "[core]\n\trepositoryformatversion = 0\n"),
            write(os.path.join(d, ".git", "objects", "ab", "cdef0123456789abcdef0123456789abcdef01"), "x" * 300),
            write(os.path.join(d, "releases", "1.0.0", "qa_delete-1.0.0.zip"), "PK" + "z" * 2000),
            write(os.path.join(d, "releases", "1.0.0", "qa_delete.update.ltx"), "[update]\nversion = 1.0.0\n"),
        ]
        set_attributes(os.path.join(d, "readonly.txt"), readonly=True)
        set_attributes(os.path.join(d, ".git", "objects", "ab", "cdef0123456789abcdef0123456789abcdef01"), readonly=True)
        set_attributes(os.path.join(d, ".git", "objects", "ab"), readonly=True)
        set_attributes(os.path.join(d, ".git"), hidden=True)
        # the junction inside points at a folder of the test's own, OUTSIDE the project
        self.p["target"] = self.path("JunctionTarget")
        write(os.path.join(self.p["target"], "keep.txt"), "a junction's target is somebody else's\n")
        _winapi.CreateJunction(self.p["target"], os.path.join(d, "linked"))
        self.expect["delete"] = {"files": 9, "folders": 8, "links": 1, "bytes": sum(sizes),
                                 "release_files": 2, "release_bytes": sizes[7] + sizes[8]}

        # D: refusals
        self.p["notproject"] = self.path("QaNotProject")
        write(os.path.join(self.p["notproject"], "readme.txt"), "no manifest here\n")
        self.p["outside"] = self.path("QaOutside")
        write(os.path.join(self.p["outside"], "mod.ltx"), mod("qa_outside"))
        self.p["nested"] = self.path("QaNested")
        write(os.path.join(self.p["nested"], "mod.ltx"), mod("qa_nested"))
        write(os.path.join(self.p["nested"], "inner", "mod.ltx"), mod("qa_inner"))
        write(os.path.join(self.p["nested"], "inner", "gamedata", "x.ltx"), "[x]\n")
        self.p["real"] = self.path("QaJunctionReal")
        write(os.path.join(self.p["real"], "mod.ltx"), mod("qa_junction"))
        write(os.path.join(self.p["real"], "file.txt"), "the real project behind the junction\n")
        self.p["junction"] = self.path("QaJunction")
        _winapi.CreateJunction(self.p["real"], self.p["junction"])
        self.p["holdsgame"] = self.path("QaHoldsGame")
        write(os.path.join(self.p["holdsgame"], "mod.ltx"), mod("qa_holds_game"))
        write(os.path.join(self.p["holdsgame"], "project.ltx"),
              "[project]\nname = QaHoldsGame\ngame_root = %s\n" % os.path.join(self.p["holdsgame"], "game"))
        write(os.path.join(self.p["holdsgame"], "game", "fsgame.ltx"), "; a game\n")
        self.p["game"] = self.path("GameX")
        write(os.path.join(self.p["game"], "fsgame.ltx"), "; a game\n")
        self.p["ingame"] = os.path.join(self.p["game"], "modules", "QaInGame")
        write(os.path.join(self.p["ingame"], "mod.ltx"), mod("qa_in_game"))
        self.p["linker"] = self.path("QaLinker")
        write(os.path.join(self.p["linker"], "mod.ltx"), mod("qa_linker"))
        write(os.path.join(self.p["linker"], "project.ltx"),
              "[project]\nname = QaLinker\ngame_root = %s\n" % self.p["game"])
        self.p["gone"] = self.path("QaGone")
        self.p["missing"] = self.path("QaMissing")

        # E: a file in use
        self.p["locked"] = self.path("QaLocked")
        write(os.path.join(self.p["locked"], "mod.ltx"), mod("qa_locked"))
        write(os.path.join(self.p["locked"], "other.txt"), "goes at once\n")
        write(os.path.join(self.p["locked"], "data", "locked.bin"), "held open by the test\n")

        # H: the project that is opened
        self.p["open"] = self.path("QaOpen")
        write(os.path.join(self.p["open"], "mod.ltx"), mod("qa_open"))

        for p in self.p.values():
            assert self.inside(p), p

    def ini(self) -> bytes:
        order = ["delete", "notproject", "nested", "junction", "gone", "missing", "holdsgame", "ingame", "linker",
                 "locked", "open"]
        lines = ["[recent]"] + ["p%d=%s" % (i, self.p[k]) for i, k in enumerate(order)]
        lines += ["p%d=" % i for i in range(len(order), 12)]
        lines += ["create_parent=%s" % self.root, ""]
        return "\r\n".join(lines).encode("ascii")

    def remove(self) -> None:
        # links first, as links: nothing may follow them out of the test's folder
        for base, dirs, _ in os.walk(self.root):
            for name in list(dirs):
                full = os.path.join(base, name)
                if is_junction(full):
                    os.rmdir(full)
                    dirs.remove(name)

        def writable(func, path, _):
            os.chmod(path, stat.S_IWRITE)
            func(path)

        shutil.rmtree(self.root, onerror=writable)


def close_explorer(folder: str, seconds: float = 15.0) -> int:
    """Closes the Explorer windows showing `folder`; answers how many it closed."""
    target = folder.replace("'", "''")
    script = (
        "$target = '%s'.TrimEnd('\\');"
        "$deadline = (Get-Date).AddSeconds(%d); $closed = 0;"
        "do {"
        "  $shell = New-Object -ComObject Shell.Application;"
        "  foreach ($w in @($shell.Windows())) {"
        "    $p = $null; try { $p = $w.Document.Folder.Self.Path } catch {};"
        "    if ($p -and ($p.TrimEnd('\\') -ieq $target)) { try { $w.Quit(); $closed++ } catch {} }"
        "  };"
        "  if ($closed) { break };"
        "  Start-Sleep -Milliseconds 250"
        "} while ((Get-Date) -lt $deadline);"
        "$closed" % (target, int(seconds)))
    exe = shutil.which("pwsh") or shutil.which("powershell")
    done = subprocess.run([exe, "-NoProfile", "-NonInteractive", "-Command", script], capture_output=True, text=True,
                          timeout=seconds + 60)
    try:
        return int((done.stdout or "0").strip().splitlines()[-1])
    except (ValueError, IndexError):
        return 0


def run(w: Work, shots: str | None, explorer: bool) -> None:
    ed = Editor()
    p = w.p
    try:
        st = ed.start()
        check(not st.get("project_active"), "A the editor starts on the project browser", short(st))

        # ------------------------------------------------------------------ A: the list
        lst = listed()
        states = {k: (lst.get(fwd(p[k]).lower()) or {}).get("state") for k in
                  ("delete", "notproject", "nested", "junction", "gone", "missing", "locked", "open")}
        check(states == {"delete": "project", "notproject": "not_a_project", "nested": "project",
                         "junction": "project", "gone": "missing", "missing": "missing", "locked": "project",
                         "open": "project"},
              "A every entry is listed with its state, gone folders included", json.dumps(states))

        # ------------------------------------------------------------------ B: to look at, never to act
        m = mcp("project_browser_menu", show="menu", path=p["delete"])
        check(m.get("ok") and m.get("browser") and same((m.get("menu") or {}).get("path", ""), p["delete"]),
              "B show=menu asks for the tile's menu", short(m))
        shot(shots, "menu")
        m = mcp("project_browser_menu")
        menu = m.get("menu") or {}
        check(menu.get("shown") and [(i.get("label"), i.get("enabled")) for i in menu.get("items", [])] ==
              [("Show in Explorer", True), ("Delete...", True)],
              "B the menu is drawn: Show in Explorer and Delete...", short(m))

        m = mcp("project_browser_menu", show="menu", path=p["gone"])
        shot(shots, "menu_missing")
        m = mcp("project_browser_menu")
        menu = m.get("menu") or {}
        check(menu.get("shown") and menu.get("state") == "missing" and
              [(i.get("label"), i.get("enabled")) for i in menu.get("items", [])] ==
              [("Show in Explorer", False), ("Remove from List", True)],
              "B a gone folder's menu: Show in Explorer disabled, Remove from List", short(m))

        m = mcp("project_browser_menu", show="confirm", path=p["delete"])
        c = m.get("confirm") or {}
        e = w.expect["delete"]
        check(m.get("ok") and c.get("view") == "question" and not c.get("refused") and
              (c.get("files"), c.get("folders"), c.get("links"), c.get("bytes")) ==
              (e["files"], e["folders"], e["links"], e["bytes"]) and c.get("git") and c.get("releases") and
              (c.get("release_files"), c.get("release_bytes")) == (e["release_files"], e["release_bytes"]),
              "B the confirmation counts what goes: files, folders, the link, bytes, .git, releases", short(m))
        shot(shots, "confirm")
        m = mcp("project_browser_menu")
        check((m.get("confirm") or {}).get("shown") and m.get("menu") is None,
              "B the confirmation is drawn, and the menu is down", short(m))

        m = mcp("project_browser_menu", show="confirm", path=p["nested"])
        c = m.get("confirm") or {}
        check(c.get("view") == "refused" and "another project" in c.get("reason", "") and
              any(same(n, os.path.join(p["nested"], "inner")) for n in c.get("nested", [])),
              "B a refusal is what the confirmation shows, naming the project inside", short(m))
        shot(shots, "confirm_refused")
        bad = mcp("project_browser_menu", show="confirm", path=p["gone"])
        check(not bad.get("ok"), "B a gone folder has no confirmation: Remove from List asks nothing", short(bad))
        bad = mcp("project_browser_menu", show="menu", path=p["outside"])
        check(not bad.get("ok"), "B a folder that is not a tile has no menu", short(bad))
        bad = mcp("project_browser_menu", show="everything")
        check(not bad.get("ok"), "B an unknown show is refused", short(bad))
        m = mcp("project_browser_menu", show="none")
        shot(shots, "browser")
        m = mcp("project_browser_menu")
        check(m.get("ok") and m.get("menu") is None and m.get("confirm") is None,
              "B show=none takes both down", short(m))
        check(os.path.isfile(os.path.join(p["delete"], "mod.ltx")) and os.path.isfile(os.path.join(p["nested"], "mod.ltx")),
              "B looking deleted nothing")

        # ------------------------------------------------------------------ C: a deletion
        d = mcp("project_delete", path=p["delete"])
        gone = d.get("deleted") or {}
        check(d.get("ok") and not d.get("refused") and d.get("removed_from_list") and
              (gone.get("files"), gone.get("folders"), gone.get("links"), gone.get("bytes")) ==
              (e["files"], e["folders"], e["links"], e["bytes"]) and d.get("git") and d.get("releases") and
              not d.get("failed"),
              "C the project is deleted, and the answer counts what went", short(d))
        check(not os.path.lexists(p["delete"]), "C the folder is gone - read-only files and folders, .git, releases")
        keep = os.path.join(p["target"], "keep.txt")
        check(os.path.isfile(keep) and open(keep, encoding="utf-8").read().startswith("a junction's target"),
              "C the junction went as a link: the folder it pointed at keeps its files")
        check(fwd(p["delete"]).lower() not in listed(), "C the entry left the list")
        bad = mcp("project_browser_menu", show="menu", path=p["delete"])
        check(not bad.get("ok"), "C and its tile is gone", short(bad))

        # ------------------------------------------------------------------ D: refusals
        def refused(key_or_path: str, words: str, what: str, extra=None) -> dict:
            path = p.get(key_or_path, key_or_path)
            assert not os.path.isabs(path) or w.inside(path), path
            before = sorted(os.walk(path)) if os.path.isdir(path) else None
            r = mcp("project_delete", path=path)
            after = sorted(os.walk(path)) if os.path.isdir(path) else None
            ok = not r.get("ok") and r.get("refused") and words.lower() in r.get("error", "").lower() and \
                before == after and not r.get("removed_from_list")
            if extra:
                ok = ok and extra(r)
            check(ok, "D refused: " + what + ", nothing deleted", short(r))
            return r

        refused("notproject", "not a project", "a folder with no project.ltx or mod.ltx")
        refused("outside", "not in the project list", "a project outside the list")
        refused("QaDelete", "full path", "a relative path")
        refused(w.path("QaNoSuchFolder"), "no such folder", "a folder that does not exist and is not listed")
        refused("nested", "another project", "a folder holding another project",
                lambda r: any(same(n, os.path.join(p["nested"], "inner")) for n in r.get("nested", [])))
        refused("junction", "is a link to", "the project folder is a junction",
                lambda r: fwd(p["real"]).lower() in r.get("error", "").lower())
        check(is_junction(p["junction"]) and os.path.isfile(os.path.join(p["real"], "file.txt")),
              "D the junction and the project behind it are both still there")
        refused("holdsgame", "holds the linked game of the project QaHoldsGame", "a folder holding a listed project's game")
        refused("ingame", "lies inside the linked game of the project QaLinker", "a folder inside a listed project's game")

        d = mcp("project_delete", path=p["missing"])
        check(d.get("ok") and d.get("missing") and d.get("removed_from_list") and
              (d.get("deleted") or {}).get("files") == 0 and not os.path.lexists(p["missing"]),
              "D a missing folder only leaves the list", short(d))
        check(fwd(p["missing"]).lower() not in listed(), "D and it is gone from the list")

        # ------------------------------------------------------------------ E: a file in use
        held = open(os.path.join(p["locked"], "data", "locked.bin"), "rb")
        try:
            d = mcp("project_delete", path=p["locked"])
            failed = d.get("failed") or []
            check(not d.get("ok") and not d.get("refused") and d.get("failed_total") == 1 and
                  failed and same(failed[0].get("path", ""), os.path.join(p["locked"], "data", "locked.bin")) and
                  "in use" in failed[0].get("reason", "") and not d.get("removed_from_list"),
                  "E a file in use is named, and the deletion does not claim success", short(d))
            check(os.path.isfile(os.path.join(p["locked"], "mod.ltx")) and
                  not os.path.exists(os.path.join(p["locked"], "other.txt")),
                  "E the rest went, the manifest stayed: it is still a project")
            check((listed().get(fwd(p["locked"]).lower()) or {}).get("state") == "project",
                  "E and it is still in the list")
        finally:
            held.close()
        d = mcp("project_delete", path=p["locked"])
        check(d.get("ok") and d.get("removed_from_list") and not os.path.lexists(p["locked"]),
              "E once the file is free, a second call finishes it", short(d))

        # ------------------------------------------------------------------ F: Remove from List
        f = mcp("project_forget", path=p["nested"])
        check(not f.get("ok") and os.path.isdir(p["nested"]) and fwd(p["nested"]).lower() in listed(),
              "F a folder that holds a project is not taken off the list", short(f))
        f = mcp("project_forget", path=p["notproject"])
        check(f.get("ok") and fwd(p["notproject"]).lower() not in listed() and
              os.path.isfile(os.path.join(p["notproject"], "readme.txt")),
              "F a folder without a project leaves the list, and nothing on disk changes", short(f))
        f = mcp("project_forget", path=p["outside"])
        check(not f.get("ok"), "F a folder outside the list is refused", short(f))

        # ------------------------------------------------------------------ G: Show in Explorer
        r = mcp("project_reveal", path=p["outside"], dry_run=True)
        check(not r.get("ok") and "not in the project list" in r.get("error", ""),
              "G a folder outside the list is not shown", short(r))
        r = mcp("project_reveal", path=p["gone"], dry_run=True)
        check(not r.get("ok") and "does not exist" in r.get("error", ""), "G a gone folder is not shown", short(r))
        r = mcp("project_reveal", path=p["nested"], dry_run=True)
        check(r.get("ok") and not r.get("opened") and same(r.get("path", ""), p["nested"]),
              "G dry_run resolves the listed folder and opens nothing", short(r))
        if explorer:
            r = mcp("project_reveal", path=p["nested"])
            closed = close_explorer(p["nested"]) if r.get("ok") else 0
            check(r.get("ok") and r.get("opened") and closed >= 1,
                  "G a real reveal opens an Explorer window on the folder (closed again: %d)" % closed, short(r))
            if r.get("ok") and not closed:
                print("! an Explorer window on %s may still be open" % p["nested"], flush=True)

        # ------------------------------------------------------------------ H: the open project
        o = mcp("open_project", path=p["open"])
        check(o.get("ok"), "H a listed project opens", short(o))
        d = mcp("project_delete", path=p["open"])
        check(not d.get("ok") and d.get("refused") and "open project" in d.get("error", "") and
              os.path.isfile(os.path.join(p["open"], "mod.ltx")),
              "H the open project is refused", short(d))
        m = mcp("project_browser_menu", show="menu", path=p["nested"])
        check(not m.get("ok") and "not on screen" in m.get("error", ""),
              "H no tile menu while a project is open", short(m))
        m = mcp("project_browser_menu")
        check(m.get("ok") and m.get("browser") is False, "H the report still answers, browser:false", short(m))
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--shots", help="a folder for screenshots of the menu and the confirmation")
    ap.add_argument("--no-explorer", action="store_true", help="skip the one real Explorer window")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away folder")
    args = ap.parse_args()

    if port_in_use():
        print("an editor already answers on %s:%d - close it first; this test starts its own" % MCP)
        return 2
    w = Work()
    print("throw-away projects in: " + w.root, flush=True)
    ini_backup = None
    if os.path.isfile(INI):
        with open(INI, "rb") as f:
            ini_backup = f.read()
    try:
        w.build()
        # the list is the test's own: the editor offers no real project to anything below
        with open(INI, "wb") as f:
            f.write(w.ini())
        run(w, os.path.abspath(args.shots) if args.shots else None, not args.no_explorer)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is None:
            if os.path.isfile(INI):
                os.remove(INI)
        else:
            with open(INI, "wb") as f:
                f.write(ini_backup)
        restored = ini_backup is None and not os.path.exists(INI) or \
            ini_backup is not None and open(INI, "rb").read() == ini_backup
        check(restored, "Z _projects.ini is back byte for byte")
        if not args.keep:
            try:
                w.remove()
            except OSError as exc:
                print("! could not remove %s: %s" % (w.root, exc), flush=True)
    ok = sum(1 for good, _ in results if good)
    print(f"PROJECT BROWSER: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
