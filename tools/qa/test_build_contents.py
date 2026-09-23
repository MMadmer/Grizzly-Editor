#!/usr/bin/env python3
"""What a build ships, and what it must not.

A project folder is the author's working place as well as the module: a git
repository, design notes, reference pictures, tool settings. This drives the editor
over MCP on a THROW-AWAY COPY of a project filled with all of that and checks:

  * tooling (.git, .gitignore, .github, .vscode, Thumbs.db, desktop.ini, Office
    locks) is never listed, never walked for assets and never shipped - at any depth;
  * mod.ltx [exclude] (mod_exclude / mod_set_manifest) leaves out what the author
    names - folders, files, patterns - from the module AND from the release package,
    and the quests and characters inside it are no part of the build;
  * a rule that would take out a file the build generates is refused;
  * the build says what it left out (`left_out`);
  * the clean build does not follow a link inside the target, refuses a target that
    is a link, and refuses a target that overlaps the project;
  * a git working copy gets a .gitignore for the editor's own files, once;
  * a folder with mod.ltx and no project.ltx still counts as a project.

    python tools/qa/test_build_contents.py --project D:\\XFinedProjects\\Test
"""
from __future__ import annotations

import argparse
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
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7
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


def write(root: str, rel: str, text: str) -> None:
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def junction(link: str, target: str) -> bool:
    r = subprocess.run(["cmd", "/c", "mklink", "/J", link, target], capture_output=True)
    return r.returncode == 0 and os.path.isdir(link)


def tree(root: str) -> list[str]:
    out = []
    for base, dirs, files in os.walk(root):
        for d in dirs:
            out.append(os.path.relpath(os.path.join(base, d), root).replace("\\", "/") + "/")
        for f in files:
            out.append(os.path.relpath(os.path.join(base, f), root).replace("\\", "/"))
    return sorted(out)


GOOD_QUEST = """return {
    nq = 1, id = "qa_hidden_in_git", title = "hidden",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "finish" } },
        { id = "finish", kind = "flow.end" },
    },
}
"""

BROKEN_QUEST = """return {
    nq = 1, id = "qa_wip", title = "WIP",
    nodes = {
        { id = "start", kind = "trigger.start", out = { next = "nowhere" } },
    },
}
"""

CHARACTER = """return {
  npc       = 1,
  id        = "%s",
  name      = "%s",
  community = "stalker",
  visual    = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
}
"""

# never content, wherever it sits
TOOLING = [
    ".gitignore", ".gitattributes", ".github/workflows/release.yml", ".vscode/settings.json",
    "gamedata/textures/qa/Thumbs.db", "gamedata/textures/qa/desktop.ini", "gamedata/.git/HEAD",
    "design/~$story.docx",
]
# the author's working files the author takes out
AUTHORED_OUT = ["design/story.md", "docs/notes.md", "docs/index.bleve/store/root.bolt", "art/cover.psd",
                "art/sheet.psd", "wip_char/ghost.npcasset", "design/wip.nqasset"]
# what must ship
SHIPS = ["LICENSE", "README.md", "art/cover.png", "gamedata/configs/qa_keep.ltx", "gamedata/textures/qa/qa_tex.dds",
         "mod.ltx"]


def run(project: str) -> None:
    # the working copy is a git repository before the editor sees it
    write(project, ".git/HEAD", "ref: refs/heads/main\n")
    write(project, ".git/config", "[core]\n")
    write(project, ".git/objects/ab/cdef0123456789", "blob")
    # a VALID quest and a character where no walk may look
    write(project, ".git/qa_hidden_in_git.nqasset", GOOD_QUEST)
    write(project, ".vscode/qa_hidden.npcasset", CHARACTER % ("qa_hidden", "Hidden"))
    for rel in TOOLING:
        if not rel.startswith(".gitignore"):
            write(project, rel, "tooling\n")
    write(project, "design/story.md", "# the plot\n")
    write(project, "docs/notes.md", "notes\n")
    write(project, "docs/index.bleve/store/root.bolt", "bolt\n")
    write(project, "art/cover.psd", "psd\n")
    write(project, "art/sheet.psd", "psd\n")
    write(project, "art/cover.png", "png\n")
    write(project, "wip_char/ghost.npcasset", CHARACTER % ("qa_ghost", "Ghost"))
    write(project, "design/wip.nqasset", BROKEN_QUEST)
    write(project, "chars/qa_courier.npcasset", CHARACTER % ("qa_courier", "Courier"))
    write(project, "LICENSE", "MIT\n")
    write(project, "README.md", "readme\n")
    write(project, "gamedata/configs/qa_keep.ltx", "[qa]\n")
    write(project, "gamedata/textures/qa/qa_tex.dds", "DDS \n")

    ed = Editor()
    target = tempfile.mkdtemp(prefix="xfined_build_target_")
    pkg = tempfile.mkdtemp(prefix="xfined_build_pkg_")
    linked = tempfile.mkdtemp(prefix="xfined_build_linked_")
    try:
        ed.start(project)

        # ------------------------------------------------------------------ A: git
        ignore = os.path.join(project, ".gitignore")
        text = io.open(ignore, encoding="utf-8").read() if os.path.isfile(ignore) else ""
        check("/releases/" in text and "/project.ltx" in text and "/temp/" in text,
              "A a git working copy without a .gitignore gets one for the editor's own files", text[:200])
        check("/design" not in text and "LICENSE" not in text, "A and it names nothing of the author's")
        write(project, ".gitignore", "# the author's own now\n")

        # ------------------------------------------------------------------ B: never walked, never listed
        quests = [q.get("path", "") for q in mcp("quest_list").get("quests", [])]
        check(not any("qa_hidden_in_git" in q for q in quests), "B a quest inside .git is no quest of the project",
              json.dumps(quests))
        chars = [c.get("id") for c in mcp("characters_list").get("characters", [])]
        check("qa_hidden" not in chars, "B nor a character inside .vscode", json.dumps(chars))
        check("qa_courier" in chars, "B the project's own character is listed")
        for rel in (".git", ".gitignore", ".vscode"):
            r = mcp("content_browser_open", name=rel)
            check(not r.get("ok"), "B the content browser does not show " + rel, json.dumps(r)[:160])
        r = mcp("content_browser_open", name="gamedata/textures/qa")
        sel = mcp("content_browser_selection", action="select", name="gamedata\\textures\\qa\\Thumbs.db")
        check(r.get("ok") and not sel.get("ok"), "B nor a Thumbs.db deep in gamedata", json.dumps(sel)[:160])

        # ------------------------------------------------------------------ C: the author's exclusions
        r = mcp("mod_set_manifest", exclude="docs\n*.psd\nwip_char")
        check(r.get("ok") and r.get("exclude") == ["docs", "*.psd", "wip_char"],
              "C mod_set_manifest takes the whole list, one entry per line", json.dumps(r.get("exclude")))
        r = mcp("mod_exclude", path="design")
        check(r.get("ok") and r.get("exclude") == ["docs", "*.psd", "wip_char", "design"],
              "C mod_exclude adds one entry", json.dumps(r)[:200])
        r = mcp("mod_exclude", path="design/story.md")
        check(r.get("ok") and len(r.get("exclude", [])) == 4, "C an entry below an excluded folder adds nothing",
              json.dumps(r)[:200])
        r = mcp("mod_exclude", path="design/story.md", excluded=False)
        check(not r.get("ok") and "design" in str(r.get("error")),
              "C including it names the rule that still takes it out", json.dumps(r)[:200])
        r = mcp("mod_exclude", path="mod.ltx")
        check(not r.get("ok"), "C mod.ltx cannot be excluded", json.dumps(r)[:160])
        m = mcp("mod_manifest")
        check(m.get("exclude") == ["docs", "*.psd", "wip_char", "design"], "C mod_manifest reports the list",
              json.dumps(m.get("exclude")))
        ltx = io.open(os.path.join(project, "mod.ltx"), encoding="utf-8", errors="replace").read()
        check("[exclude]" in ltx and "wip_char" in ltx, "C the list lives in mod.ltx [exclude]")

        # excluded quests and characters are no part of the build
        quests = [q.get("path", "") for q in mcp("quest_list").get("quests", [])]
        check(not any("wip.nqasset" in q for q in quests), "C a quest in an excluded folder is not the build's",
              json.dumps(quests))
        chars = [c.get("id") for c in mcp("characters_list").get("characters", [])]
        check("qa_ghost" not in chars, "C nor a character in one", json.dumps(chars))

        # ------------------------------------------------------------------ D: the build
        built = mcp("mod_export", target=target.replace("\\", "/"))
        check(built.get("ok"), "D the module builds - a broken quest in an excluded folder does not stop it",
              json.dumps(built)[:300])
        module = os.path.join(target, "modules", m.get("id", ""))
        got = tree(module)
        leaked = [p for p in got if any(p.startswith(x) or p == x for x in
                  (".git", ".vscode", ".github", "design", "docs", "wip_char", "art/cover.psd", "art/sheet.psd"))
                  or os.path.basename(p.rstrip("/")) in (".gitignore", ".gitattributes", "Thumbs.db", "desktop.ini",
                                                          ".git", "~$story.docx")]
        check(not leaked, "D nothing of git, the tools or the author's exclusions is in the module", json.dumps(leaked))
        missing = [p for p in SHIPS if p not in got]
        check(not missing, "D and everything else is", json.dumps(missing))
        left = [p.replace("\\", "/") for p in built.get("left_out", [])]
        for want in (".git", ".gitignore", ".vscode", "design", "docs", "art/cover.psd", "wip_char",
                     "gamedata/textures/qa/Thumbs.db", "gamedata/.git"):
            check(want in left, "D left_out names " + want, json.dumps(left))
        check("LICENSE" not in left and "README.md" not in left, "D and does not name what shipped")

        # ------------------------------------------------------------------ E: a rule that breaks the module
        gen = mcp("mod_exclude", path="gamedata/configs/gameplay")
        refused = mcp("mod_export", target=target.replace("\\", "/"))
        check(gen.get("ok") and not refused.get("ok") and "generates" in str(refused.get("error")),
              "E excluding a file the build generates is refused at build time", json.dumps(refused)[:300])
        back = mcp("mod_exclude", path="gamedata/configs/gameplay", excluded=False)
        check(back.get("ok"), "E and Include in Build takes it back", json.dumps(back)[:200])

        # ------------------------------------------------------------------ F: the clean build and links
        keep = os.path.join(linked, "keep.txt")
        write(linked, "keep.txt", "not yours\n")
        inner = os.path.join(module, "pointer")
        if check(junction(inner, linked), "F a junction sits inside the module folder"):
            again = mcp("mod_export", target=target.replace("\\", "/"))
            check(again.get("ok") and os.path.isfile(keep) and not os.path.exists(inner),
                  "F the clean build removes the link and leaves what it pointed at", json.dumps(again)[:200])
        shutil.rmtree(module, ignore_errors=True)
        if check(junction(module, linked), "F the module folder itself is a junction"):
            refused = mcp("mod_export", target=target.replace("\\", "/"))
            check(not refused.get("ok") and "link" in str(refused.get("error")) and os.path.isfile(keep),
                  "F a build into a link is refused and the linked folder is untouched", json.dumps(refused)[:240])
            os.rmdir(module)
        refused = mcp("mod_export", target=project.replace("\\", "/"))
        check(not refused.get("ok") and "overlap" in str(refused.get("error")),
              "F a target inside the project is refused", json.dumps(refused)[:240])
        check(not os.path.exists(os.path.join(project, "modules")),
              "F and not even an empty folder was made there")

        # ------------------------------------------------------------------ G: the release
        pk = mcp("mod_package", out_dir=pkg.replace("\\", "/"))
        check(pk.get("ok"), "G the release packages", json.dumps(pk)[:300])
        names = []
        for p in pk.get("packages", []):
            with zipfile.ZipFile(os.path.join(pkg, p.get("name", ""))) as z:
                names += [i.filename for i in z.infolist()]
        prefix = "modules/%s/" % m.get("id", "")
        rel = [n[len(prefix):] for n in names if n.startswith(prefix)]
        bad = [n for n in rel if n.startswith((".git", ".vscode", ".github", "design/", "docs/", "wip_char/"))
               or n.endswith((".psd", "Thumbs.db", "desktop.ini", ".gitignore", ".gitattributes"))]
        check(rel and not bad, "G the package holds none of it", json.dumps(bad))
        check("LICENSE" in rel and "gamedata/configs/qa_keep.ltx" in rel, "G and the module's files are all there",
              json.dumps(rel)[:300])
        index = io.open(os.path.join(pkg, pk.get("index", {}).get("name", "")), encoding="utf-8").read()
        check(".git" not in index and "story.md" not in index and ".psd" not in index,
              "G the file index names none of it")
        check("design" in [x.replace("\\", "/") for x in pk.get("left_out", [])], "G mod_package says what it left out",
              json.dumps(pk.get("left_out")))

        # ------------------------------------------------------------------ H: a clone without project.ltx
        os.remove(os.path.join(project, "project.ltx"))
        listed = json.dumps(mcp("list_projects"))
        check(os.path.basename(project) in listed, "H a folder with mod.ltx and no project.ltx is still a project",
              listed[:300])
    finally:
        ed.close()
        for d in (target, pkg, linked):
            shutil.rmtree(d, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="xfined_buildproj_")
    copy = os.path.join(work, "QaBuildContents")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git", ".gitignore"))
    print("running against a copy: " + copy, flush=True)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()

    try:
        run(copy)
    except Exception as exc:  # noqa: BLE001
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"BUILD CONTENTS: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
