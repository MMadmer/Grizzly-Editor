#!/usr/bin/env python3
"""A project stands alone (docs/ASSETS.md 8, docs/assets/PROJECT.md): moved anywhere and opened in a home that
never saw it, it opens by its project file, finds its game, brings its scene up and writes no absolute path into
itself; a second start of the editor hands its project to the first. Over MCP, on a COPY of a project in a temp
folder, with homes of its own.

What it proves:
  A  the first open of the copy converts it: <Folder>.grizzly with an id and the scene, project.ltx read once and
     gone, AGENTS.md written, the game the old project.ltx named is the default one now
  M  the folder is moved and renamed; a fresh home; the editor started with the project FILE as its one argument
     (as a double click starts it) opens the project at its new place
  G  the game is found by itself in that home (the installer's entry, the only place it knew), and the scene is up
  W  nothing the editor wrote inside the project names the old place, the new place, a home or this computer;
     the project file is untouched, AGENTS.md was not written again, there is no project.ltx
  H  a second start with another project hands it to the running editor and ends at once (exit code 0, its own
     port never opened); a start without a project only brings the window forward
  C  with a library of the test's own (Preferences > Library) and a library list an agent wrote into the project
     file, project.collect dry_run says what it would copy and copies nothing; project.collect copies it in and
     leaves on the list only what no library has, which asset.check names (W403)
  Z  no UI pass stalled; the editor closes over MCP and lets go of its home

    python tools/qa/test_portable.py --project <project> [--keep]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
SCENE = "l01_escape"

results: list[tuple[bool, str]] = []


def check(ok, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def short(data) -> str:
    return json.dumps(data)[:600]


def mcp(port: int, cmd: str, timeout: float = 300.0, **extra) -> dict:
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(("127.0.0.1", port), timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def answers(port: int) -> bool:
    try:
        return bool(mcp(port, "ping", timeout=5).get("ok"))
    except OSError:
        return False


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def start(args: list[str], env: dict) -> subprocess.Popen:
    exe = qa_target.exe()
    if not os.path.isfile(exe):
        raise RuntimeError("build the editor first: " + exe)
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
    return subprocess.Popen([exe, *args], cwd=os.path.dirname(exe), startupinfo=startup, env=env)


def wait_open(port: int, proc: subprocess.Popen, seconds: float = 300) -> dict:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            info = mcp(port, "project.info", timeout=30)
            if info.get("ok"):
                return info
        except OSError:
            pass
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup (code %s)" % proc.returncode)
        time.sleep(0.1)
    raise RuntimeError("the editor never opened the project")


def wait_scene(port: int, seconds: float = 300) -> str:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        # a minimised editor draws only when asked: the frame that reopens the scene is drawn here
        mcp(port, "ui.list")
        info = mcp(port, "scene.info", timeout=60)
        if SCENE in (info.get("file") or "").lower():
            return info.get("file") or ""
        time.sleep(1)
    return ""


def quit_editor(port: int, proc: subprocess.Popen) -> bool:
    try:
        mcp(port, "app.quit", discard=True)
    except OSError:
        pass
    try:
        proc.wait(timeout=120)
        return True
    except subprocess.TimeoutExpired:
        print("! the editor did not close on its own", flush=True)
        return False


def project_file(folder: str) -> str:
    names = sorted(n for n in os.listdir(folder) if n.lower().endswith(".grizzly"))
    return os.path.join(folder, names[0]) if names else ""


def legacy_value(path: str, key: str) -> str:
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return ""
    for line in text.splitlines():
        k, _, v = line.partition("=")
        if k.strip().lower() == key:
            return v.strip()
    return ""


def set_legacy(path: str, key: str, value: str) -> None:
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    lines = [l for l in lines if l.partition("=")[0].strip().lower() != key]
    at = next((i + 1 for i, l in enumerate(lines) if l.strip().lower() == "[project]"), None)
    if at is None:
        lines, at = ["[project]"] + lines, 1
    lines.insert(at, "%s=%s" % (key, value))
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write("\r\n".join(lines) + "\r\n")


def written_since(folder: str, since: float) -> list[str]:
    """Every file under `folder` changed at or after `since`."""
    out = []
    for base, dirs, files in os.walk(folder):
        dirs[:] = [d for d in dirs if d != ".git"]
        for n in files:
            p = os.path.join(base, n)
            try:
                if os.path.getmtime(p) >= since - 1:
                    out.append(p)
            except OSError:
                pass
    return out


def names_outside(path: str, words: list[str]) -> list[str]:
    """Which of `words` (places, names) the file's bytes hold, any case, either slash."""
    try:
        if os.path.getsize(path) > 8 << 20:
            return []
        data = open(path, "rb").read()
    except OSError:
        return []
    texts = [data.decode("latin-1").lower()]
    try:
        texts.append(data.decode("utf-16-le", "ignore").lower())
    except UnicodeDecodeError:
        pass
    found = []
    for w in words:
        forms = {w.lower(), w.lower().replace("\\", "/"), w.lower().replace("\\", "\\\\")}
        if any(f and f in t for f in forms for t in texts):
            found.append(w)
    return found


def run(src: str, work: str) -> None:
    port = qa_target.PORT
    home1 = qa_target.HOME
    env1 = dict(os.environ)
    a = os.path.join(work, "QaPortable")
    qa_target.copy_project(src, a)
    print("copy: " + a, flush=True)
    legacy = os.path.join(a, "project.ltx")
    scene_rel = "rawdata\\levels\\%s.level" % SCENE
    if not os.path.isfile(legacy) and not project_file(a):
        check(False, "A the copy is a project", a)
        return
    if os.path.isfile(legacy):
        set_legacy(legacy, "last_scene", scene_rel)
    else:
        # a project converted already: its project file names the scene
        pf = project_file(a)
        text = open(pf, encoding="utf-8").read()
        text = re.sub(r'\n\s*scene\s*=\s*"[^"]*",?', "", text)
        text = text.replace("\n}", '\n  scene = "rawdata/levels/%s.level",\n}' % SCENE, 1)
        open(pf, "w", encoding="utf-8", newline="").write(text)

    # ------------------------------------------------------------------ A: the first open converts it
    ed1 = start([*qa_target.ARGS, "-nodlg", "-project", a], env1)
    try:
        info = wait_open(port, ed1)
        pf = project_file(a)
        text = open(pf, encoding="utf-8").read() if pf else ""
        check(pf and os.path.basename(pf) == "QaPortable.grizzly" and re.search(r'id = "[0-9a-f-]{36}"', text) and
              ("rawdata/levels/%s.level" % SCENE) in text,
              "A the project file is written, named after the folder, with its id and the scene", text[:300])
        check(not os.path.exists(legacy), "A project.ltx was read once and is gone")
        check(os.path.isfile(os.path.join(a, "AGENTS.md")), "A AGENTS.md is written")
        game = (info.get("game") or {})
        check(game.get("valid") and game.get("root"), "A the game the old project.ltx named is the game in use",
              short(info))
        game_root = (game.get("root") or "").replace("/", "\\")
        project_id = info.get("id", "")
        guide_time = os.path.getmtime(os.path.join(a, "AGENTS.md")) if os.path.isfile(os.path.join(a, "AGENTS.md")) else 0
        pf_text = text
    finally:
        mcp(port, "project.close", discard=True) if answers(port) else None
        quit_editor(port, ed1)

    # ------------------------------------------------------------------ M: moved, a fresh home, a double click
    moved_parent = os.path.join(work, "elsewhere", "deeper")
    os.makedirs(moved_parent)
    b = os.path.join(moved_parent, "QaPortableMoved")
    os.rename(a, b)
    moved_at = time.time()
    home2 = tempfile.mkdtemp(prefix="grizzly_home_portable_")
    env2 = dict(os.environ)
    # the home never saw the project or the game; the game's installer entry is what the finder is given
    env2["GRIZZLY_HOME"] = home2
    env2["GRIZZLY_GAME_SEARCH"] = "installed:" + game_root
    pf_moved = project_file(b)
    ed2 = start(["-mcp_port", str(port), "-home", home2, "-nodlg", pf_moved], env2)
    try:
        info = wait_open(port, ed2)
        check(fwd(info.get("path", "")).lower() == fwd(b).lower() and info.get("id") == project_id,
              "M the project opens at its new place by its project file, the same id", short(info))

        # -------------------------------------------------------------- G: the game is found, the scene is up
        g = mcp(port, "game.info")
        check(g.get("valid") and fwd(g.get("root", "")).lower() == fwd(game_root).lower() and
              g.get("source") == "installed" and g.get("default_found") == "installed",
              "G the game is found by itself in a home that never saw it", short(g))
        scene = wait_scene(port)
        check(SCENE in scene.lower(), "G the project's scene is up", scene)

        # -------------------------------------------------------------- W: nothing inside names a place outside
        words = [a, b, os.path.dirname(b), home1, home2, os.environ.get("COMPUTERNAME", ""), REPO]
        words = [w for w in words if w]
        bad = {}
        for path in written_since(b, moved_at):
            hit = names_outside(path, words)
            if hit:
                bad[os.path.relpath(path, b)] = hit
        check(not bad, "W nothing the editor wrote inside the project names a place outside it or this computer",
              json.dumps(bad)[:600])
        hit = names_outside(pf_moved, words)
        check(not hit and open(pf_moved, encoding="utf-8").read() == pf_text,
              "W the project file names no place and was not rewritten by the open", json.dumps(hit))
        guide = os.path.join(b, "AGENTS.md")
        check(os.path.isfile(guide) and os.path.getmtime(guide) == guide_time and not names_outside(guide, words),
              "W AGENTS.md was written once, and names no place outside the project")
        check(not os.path.exists(os.path.join(b, "project.ltx")), "W there is no project.ltx")
        state = os.path.join(home2, "projects", project_id + ".ltx")
        check(os.path.isfile(state) and fwd(b).lower() in fwd(open(state, encoding="utf-8").read()).lower(),
              "W what is this machine's went into the new home, by the project's id", state)

        # -------------------------------------------------------------- C: collect, dry run, then for real
        # a library of the test's own (harmless files: an object with no surfaces and its thumbnail) and a
        # project file that names what the scenes take from it - as an agent would write it
        library = os.path.join(work, "Library")
        for rel, data in (("rawdata/objects/qa/crate.object", "qa crate"), ("rawdata/objects/qa/crate.thm", "thumb")):
            os.makedirs(os.path.dirname(os.path.join(library, rel)), exist_ok=True)
            with open(os.path.join(library, rel), "w", encoding="utf-8", newline="") as f:
                f.write(data)
        lib = mcp(port, "project.set_library", path=library)
        check(lib.get("ok") and fwd(lib.get("path", "")).lower() == fwd(library).lower(),
              "C the library is a preference of the home", short(lib))
        text = open(pf_moved, encoding="utf-8").read()
        with open(pf_moved, "w", encoding="utf-8", newline="") as f:
            f.write(text.replace("\n}", '\n  library = { "qa\\\\crate", "qa\\\\nowhere" },\n}', 1))
        before = sorted(written_since(b, time.time() - 1e9))
        c = mcp(port, "project.collect", dry_run=True)
        after = sorted(written_since(b, time.time() - 1e9))
        check(c.get("ok") and c.get("dry_run") is True and c.get("files") == 2 and c.get("bytes") == 13 and
              c.get("missing") == ["qa\\nowhere"] and before == after,
              "C project.collect dry_run says what it would copy (2 files, 13 bytes, one ref nowhere), and copies nothing",
              short(c))
        c = mcp(port, "project.collect")
        landed = os.path.join(b, "rawdata", "objects", "qa", "crate.object")
        listed = mcp(port, "project.info").get("library")
        check(c.get("ok") and os.path.isfile(landed) and os.path.isfile(landed[:-len("object")] + "thm") and
              listed == ["qa\\nowhere"],
              "C project.collect copies it in; the list keeps only what no library has", short(c) + " " + json.dumps(listed))
        problems = mcp(port, "asset.check", path=os.path.basename(pf_moved))
        check(any(p.get("code") == "W403" and "qa\\nowhere" in p.get("text", "") for p in problems.get("problems", [])),
              "C a reference that resolves nowhere is a problem of the project file", short(problems))
        mcp(port, "project.set_library", path="")

        # -------------------------------------------------------------- H: a second start hands over
        other = os.path.join(work, "QaSecond")
        os.makedirs(other)
        with open(os.path.join(other, "mod.ltx"), "w", encoding="utf-8", newline="") as f:
            f.write("[module]\r\nid = qa_second\r\nname = QaSecond\r\nversion = 1.0.0\r\n")
        qa_target.guard_project(other)
        port3 = free_port()
        t = time.monotonic()
        second = start(["-mcp_port", str(port3), "-home", home2, "-nodlg", other], env2)
        try:
            code = second.wait(timeout=120)
        except subprocess.TimeoutExpired:
            code = None
        took = time.monotonic() - t
        check(code == 0 and not answers(port3), "H a second start hands its project over and ends (code %s, %.1f s)"
              % (code, took))
        info = mcp(port, "project.info")
        check(fwd(info.get("path", "")).lower() == fwd(other).lower(),
              "H the running editor opened the project the second start was given", short(info))
        third = start(["-mcp_port", str(port3), "-home", home2, "-nodlg"], env2)
        try:
            code = third.wait(timeout=120)
        except subprocess.TimeoutExpired:
            code = None
        check(code == 0 and answers(port) and fwd(mcp(port, "project.info").get("path", "")).lower() == fwd(other).lower(),
              "H a start without a project only brings the window forward", "code %s" % code)
        lock = os.path.join(home2, "editor.lock")
        check(os.path.isfile(lock) and ("pid=%d" % ed2.pid) in open(lock, encoding="utf-8").read(),
              "H the home's lock names the running editor", lock)
        stalls = mcp(port, "perf.stalls")
        check(stalls.get("ok") and not stalls.get("stalls"),
              "Z no UI pass held the UI thread over 100 ms (worst %s ms)" % stalls.get("worst_ms"),
              json.dumps(stalls.get("stalls"))[:600])
    finally:
        closed = quit_editor(port, ed2)
        check(closed, "Z the editor closes over MCP")
        check(not os.path.exists(os.path.join(home2, "editor.lock")), "Z and lets go of its home")
        qa_target.remove_tree(home2)


def fwd(p: str) -> str:
    return p.replace("\\", "/").rstrip("/")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY; it needs rawdata\\levels\\%s.level" % SCENE)
    ap.add_argument("--keep", action="store_true", help="keep the throw-away folder")
    args = ap.parse_args()
    src = os.path.abspath(args.project)
    if not os.path.isfile(os.path.join(src, "rawdata", "levels", SCENE + ".level")):
        print("the project has no rawdata\\levels\\%s.level: %s" % (SCENE, src))
        return 2
    if answers(qa_target.PORT):
        print(qa_target.busy_text())
        return 2
    work = os.path.realpath(tempfile.mkdtemp(prefix="grizzly_portable_"))
    try:
        run(src, work)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if not args.keep:
            qa_target.remove_tree(work)
    ok = sum(1 for good, _ in results if good)
    print(f"PORTABLE: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
