#!/usr/bin/env python3
"""What a build ships, and what it must not.

A project folder is the author's working place as well as the module: a git
repository, design notes, reference pictures, tool settings. This drives the editor
over MCP on a THROW-AWAY COPY of a project filled with all of that and checks:

  * tooling (.git, .gitignore, .github, .vscode, Thumbs.db, desktop.ini, Office
    locks) is never listed, never walked for assets and never shipped - at any depth;
  * mod.ltx [exclude] (mod.exclude / mod.set) leaves out what the author
    names - folders, files, patterns - from the module AND from the release package,
    and the quests and characters inside it are no part of the build;
  * the build says what it left out (`left_out`);
  * the clean build does not follow a link inside the target, refuses a target that
    is a link, and refuses a target that overlaps the project;
  * a git working copy gets a .gitignore for the editor's own files, once;
  * the Mod Settings tab opens on each of its pages (mod.view), follows a
    save made elsewhere without turning dirty, and its Relations and Mod Browser pages
    are the same label-left grid as the rest; the build dialogs (Build to Folder,
    Export Flat, Package Release) open, show and close over MCP by their kit ids, and
    a Writes To path that does not fit keeps its end.

    python tools/qa/test_build_contents.py --project <project>
"""
from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import tempfile
import time
import zipfile

import qa
from qa import check, cmd


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


SETTINGS_PAGES = ["general", "mod_browser", "game_mode", "relations", "build_contents", "pda", "commands"]
BUILD_DIALOGS = ["build_to_folder", "export_flat", "package_release"]


def labels_of(audit: dict) -> dict:
    return {c.get("label"): c for c in audit.get("controls", [])}


def run_ui_checks() -> None:
    """Mod Settings is a tab that follows mod.ltx; the build dialogs open, show and close over MCP."""
    v = cmd("mod.view")
    check(v.get("ok") and v.get("tab") == "mod_settings", "U mod.view answers", json.dumps(v)[:300])
    check([p.get("id") for p in v.get("pages", [])] == SETTINGS_PAGES, "U Mod Settings has its seven pages",
          json.dumps(v.get("pages"))[:300])
    for page in SETTINGS_PAGES:
        v = cmd("mod.view", page=page)
        check(v.get("ok") and v.get("active") and v.get("page") == page, "U the tab opens on " + page, json.dumps(v)[:300])
        audit = cmd("ui.audit", id="mod_settings")
        check(audit.get("ok") and not audit.get("escaped"), "U nothing escapes the " + page + " page",
              json.dumps(audit.get("escaped"))[:400])
    controls = labels_of(cmd("ui.audit", id="mod_settings"))
    check("Revert" not in controls and "Save" not in controls, "U the tab saves as it is edited: no Revert, no Save",
          json.dumps(sorted(x for x in controls if x))[:400])
    frame = cmd("ui.frame")
    check("as you edit" in str(frame.get("toolbar_status", "")), "U the toolbar row says how the tab saves - no footer",
          json.dumps(frame.get("toolbar_status"))[:200])
    # a save made elsewhere reaches the form and leaves it clean
    probe = cmd("mod.exclude", path="qa_ui_probe")
    v = cmd("mod.view")
    check(probe.get("ok") and v.get("dirty") is False, "U an outside save does not make the tab dirty", json.dumps(v)[:300])
    cmd("mod.exclude", path="qa_ui_probe", on=False)
    # Relations are picked from the linked game's modules, never typed into boxes
    cmd("mod.view", page="relations")
    audit = cmd("ui.audit", id="mod_settings")
    controls = labels_of(audit)
    check("Add Module..." in controls, "U Relations offers Add Module...", json.dumps(sorted(x for x in controls if x))[:400])
    # the same label-left grid as the other pages: a row per relation, its Add Module... in the value column
    rows = [controls.get(k) for k in ("Requires", "Load After", "Load Before", "Conflicts")]
    adds = sorted((c for c in audit.get("controls", []) if c.get("label") == "Add Module..."), key=lambda c: c["rect"][1])
    grid = all(rows) and len(adds) == 4 and all(a["rect"][0] >= r["rect"][0] + 60 and a["rect"][1] >= r["rect"][1]
                                                for a, r in zip(adds, rows))
    check(grid, "U Relations is a grid: each row's Add Module... sits in the value column",
          json.dumps({"rows": rows, "adds": adds})[:400])
    # Mod Browser too: Picture and Tags are rows, the picture's buttons in the value column
    cmd("mod.view", page="mod_browser")
    audit = cmd("ui.audit", id="mod_settings")
    controls = labels_of(audit)
    picture, tags, choose = controls.get("Picture"), controls.get("Tags"), controls.get("Choose Picture...")
    grid = bool(picture and tags and choose) and choose["rect"][0] >= picture["rect"][0] + 60 and \
        choose["rect"][1] >= picture["rect"][1] and tags["rect"][1] > choose["rect"][1]
    check(grid, "U Mod Browser is a grid: Picture and Tags are rows, Choose Picture... sits in the value column",
          json.dumps({"picture": picture, "tags": tags, "choose": choose})[:400])
    both = cmd("mod.set", set={"requires": ["qa.other"], "conflicts": ["qa.other"]})
    v = cmd("mod.view")
    rel = [p for p in v.get("problems", []) if p.get("page") == "relations"]
    check(both.get("ok") and any("qa.other" in p.get("text", "") and p.get("tone") == "warning" for p in rel),
          "U a module required and in conflict at once is named on Relations", json.dumps(rel)[:300])
    cmd("mod.set", set={"requires": [], "conflicts": []})
    bad = cmd("mod.view", page="nowhere")
    check(not bad.get("ok") and "general" in bad.get("error", ""), "U an unknown page is refused, naming the pages",
          json.dumps(bad))
    closed = cmd("ui.tab", action="close", doc="mod_settings")
    check(closed.get("ok"), "U the tab closes when nothing is unsaved", json.dumps(closed)[:300])

    windows = cmd("ui.contract").get("dialogs", {})
    module_id = cmd("mod.info").get("manifest", {}).get("id", "")
    for dialog in BUILD_DIALOGS:
        window = windows.get(dialog, "")
        check(window.endswith("###dlg." + dialog), "U the contract names " + dialog + " by its kit id", window)
        opened = cmd("ui.dialog", action="open", id=dialog)
        check(opened.get("ok"), "U ui.dialog opens " + dialog, json.dumps(opened)[:300])
        audit = cmd("ui.audit", id="dlg." + dialog)
        check(audit.get("ok") and not audit.get("escaped"), "U nothing escapes " + dialog, json.dumps(audit.get("escaped"))[:400])
        controls = labels_of(audit)
        check("Cancel" in controls, "U " + dialog + " has Cancel", json.dumps(sorted(x for x in controls if x))[:300])
        # Writes To exists to show the folder the build fills: cut, it keeps its end
        tail = {"build_to_folder": "\\modules\\", "export_flat": "\\gamedata_"}.get(dialog)
        if tail and module_id:
            cut = [t for t in audit.get("truncated", []) if t.get("label", "").endswith(tail + module_id)]
            check(all(t.get("shown", "").endswith(tail + module_id) for t in cut),
                  "U " + dialog + " keeps the end of the path it writes to", json.dumps(cut)[:300])
        listed = [d.get("id") for d in cmd("ui.list").get("dialogs", [])]
        check(dialog in listed, "U ui.list names " + dialog + " as the dialog on top", json.dumps(listed))
        shut = cmd("ui.dialog", action="close", id=dialog)
        check(shut.get("ok"), "U ui.dialog closes " + dialog, json.dumps(shut)[:300])
        listed = [d.get("id") for d in cmd("ui.list").get("dialogs", [])]
        check(dialog not in listed, "U and it is gone", json.dumps(listed))


def run(ctx: qa.Run) -> None:
    project = ctx.project
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

    ed = qa.Editor(project)
    target = tempfile.mkdtemp(prefix="grizzly_build_target_")
    pkg = tempfile.mkdtemp(prefix="grizzly_build_pkg_")
    linked = tempfile.mkdtemp(prefix="grizzly_build_linked_")
    try:
        ed.start()

        # ------------------------------------------------------------------ A: git
        ignore = os.path.join(project, ".gitignore")
        text = io.open(ignore, encoding="utf-8").read() if os.path.isfile(ignore) else ""
        check("/releases/" in text and "/preview.png" in text and "/temp/" in text,
              "A a git working copy without a .gitignore gets one for the editor's own files", text[:200])
        check("/design" not in text and "LICENSE" not in text, "A and it names nothing of the author's")
        write(project, ".gitignore", "# the author's own now\n")

        # ------------------------------------------------------------------ B: never walked, never listed
        quests = [q.get("extra", "") for q in cmd("game.lookup", type="quest_id", limit=500).get("entries", [])]
        check(not any("qa_hidden_in_git" in q for q in quests), "B a quest inside .git is no quest of the project",
              json.dumps(quests))
        chars = [c.get("id") for c in cmd("game.lookup", type="character_id", limit=500).get("entries", [])]
        check("qa_hidden" not in chars, "B nor a character inside .vscode", json.dumps(chars))
        check("qa_courier" in chars, "B the project's own character is listed")
        for rel in (".git", ".gitignore", ".vscode"):
            r = cmd("content.view", action="reveal", source="project", path=rel)
            check(not r.get("ok"), "B the content browser does not show " + rel, json.dumps(r)[:160])
        r = cmd("content.view", action="reveal", source="project", path="gamedata/textures/qa/")
        sel = cmd("content.view", action="go", source="project", folder="gamedata/textures/qa") and cmd("content.view", action="select", path="gamedata\\textures\\qa\\Thumbs.db")
        check(r.get("ok") and not sel.get("ok"), "B nor a Thumbs.db deep in gamedata", json.dumps(sel)[:160])

        # ------------------------------------------------------------------ C: the author's exclusions
        r = cmd("mod.set", set={"exclude": ["docs", "*.psd", "wip_char"]})
        check(r.get("ok") and r.get("manifest", {}).get("exclude") == ["docs", "*.psd", "wip_char"],
              "C mod.set takes the whole list", json.dumps(r.get("manifest", {}).get("exclude")))
        r = cmd("mod.exclude", path="design")
        check(r.get("ok") and r.get("exclude") == ["docs", "*.psd", "wip_char", "design"],
              "C mod.exclude adds one entry", json.dumps(r)[:200])
        r = cmd("mod.exclude", path="design/story.md")
        check(r.get("ok") and len(r.get("exclude", [])) == 4, "C an entry below an excluded folder adds nothing",
              json.dumps(r)[:200])
        r = cmd("mod.exclude", path="design/story.md", on=False)
        check(not r.get("ok") and "design" in str(r.get("error")),
              "C including it names the rule that still takes it out", json.dumps(r)[:200])
        r = cmd("mod.exclude", path="mod.ltx")
        check(not r.get("ok"), "C mod.ltx cannot be excluded", json.dumps(r)[:160])
        info = cmd("mod.info")
        m = info.get("manifest", {})
        check(m.get("exclude") == ["docs", "*.psd", "wip_char", "design"], "C mod.info reports the list",
              json.dumps(m.get("exclude")))
        left = [x.replace("\\", "/") for x in info.get("build", {}).get("left_out", [])]
        check("design" in left and info.get("build", {}).get("ships", 0) > 0,
              "C and what a build would ship and leave out", json.dumps(info.get("build"))[:300])
        ltx = io.open(os.path.join(project, "mod.ltx"), encoding="utf-8", errors="replace").read()
        check("[exclude]" in ltx and "wip_char" in ltx, "C the list lives in mod.ltx [exclude]")

        # excluded quests and characters are no part of the build
        quests = [q.get("extra", "") for q in cmd("game.lookup", type="quest_id", limit=500).get("entries", [])]
        check(not any("wip.nqasset" in q for q in quests), "C a quest in an excluded folder is not the build's",
              json.dumps(quests))
        # the pickers' list follows the manifest a moment later: it never makes a caller wait
        deadline = time.monotonic() + 10
        while True:
            chars = [c.get("id") for c in cmd("game.lookup", type="character_id", limit=500).get("entries", [])]
            if "qa_ghost" not in chars or time.monotonic() > deadline:
                break
            time.sleep(0.1)
        check("qa_ghost" not in chars, "C nor a character in one", json.dumps(chars))

        # ------------------------------------------------------------------ D: the build
        built = cmd("mod.build", target="folder", path=target.replace("\\", "/"))
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

        # ------------------------------------------------------------------ U: Mod Settings and the build dialogs
        run_ui_checks()

        # ------------------------------------------------------------------ F: the clean build and links
        keep = os.path.join(linked, "keep.txt")
        write(linked, "keep.txt", "not yours\n")
        inner = os.path.join(module, "pointer")
        if check(junction(inner, linked), "F a junction sits inside the module folder"):
            again = cmd("mod.build", target="folder", path=target.replace("\\", "/"))
            check(again.get("ok") and os.path.isfile(keep) and not os.path.exists(inner),
                  "F the clean build removes the link and leaves what it pointed at", json.dumps(again)[:200])
        qa.remove_tree(module)
        if check(junction(module, linked), "F the module folder itself is a junction"):
            refused = cmd("mod.build", target="folder", path=target.replace("\\", "/"))
            check(not refused.get("ok") and "link" in str(refused.get("error")) and os.path.isfile(keep),
                  "F a build into a link is refused and the linked folder is untouched", json.dumps(refused)[:240])
            os.rmdir(module)
        refused = cmd("mod.build", target="folder", path=project.replace("\\", "/"))
        check(not refused.get("ok") and "overlap" in str(refused.get("error")),
              "F a target inside the project is refused", json.dumps(refused)[:240])
        check(not os.path.exists(os.path.join(project, "modules")),
              "F and not even an empty folder was made there")

        # ------------------------------------------------------------------ G: the release
        pk = cmd("mod.package", path=pkg.replace("\\", "/"))
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
        check("design" in [x.replace("\\", "/") for x in pk.get("left_out", [])], "G mod.package says what it left out",
              json.dumps(pk.get("left_out")))

        # ------------------------------------------------------------------ H: the jobs and the UI thread
    finally:
        ed.close()
        for d in (target, pkg, linked):
            qa.remove_tree(d)


def main() -> int:
    return qa.suite("BUILD CONTENTS", run, doc=__doc__, ignore=(".git", ".gitignore"))


if __name__ == "__main__":
    sys.exit(main())
