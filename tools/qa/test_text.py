#!/usr/bin/env python3
"""The Text tab and Play without a scene, over MCP, on a THROW-AWAY COPY of a project (docs/UI_DESIGN.md
section 6 "Text"; CLAUDE.md "Text files").

What it proves:
  A  the Content Browser opens a script, a config, a patch, XML, Markdown and mod.ltx in the Text tab, a file whose
     name says nothing but that is text too, and a binary file in none (a note says why); Open as Text opens one of
     the editor's own assets as text
  B  text.view answers what the tab shows: the language, the encoding (UTF-8, Windows-1251 read as it is), the line
     endings, the lines; a Lua syntax error is a problem at its line and column, the same list asset.check gives;
     a file written outside is read again and its problems follow
  C  the Markdown view: preview, split, edit; Go to Symbol's list: a script's functions, a config's sections, a
     Markdown file's headings; find marks the matches and counts them; a line goes to the caret
  D  the file as it was: saving an untouched tab writes nothing (bytes, encoding, line endings the same); a tab
     with an edit made outside merges it; a file of the game opens read-only from script.lookup's answer
  E  script.lookup finds a declaration of the project's scripts and completes a prefix
  F  Play in a project without a scene builds the module into Play's root and starts a new game (dry run)

The suite makes its own project in its temp folder - a module that changes the game and has no scene, with the files
above - and links it to the game the author's editor links (read only: Play's dry run stages nothing in the game).

    python tools/qa/test_text.py [--shots <folder>]
"""
from __future__ import annotations

import json
import os
import sys

import qa
from qa import check, cmd

DIR = "qa_text"
SCRIPT = DIR + "/qa_text.script"
SCRIPT_TEXT = (
    "-- the tour's script\n"
    "local base = 10\n"
    "\n"
    "function qa_text_twice(x)\n"
    "\treturn x * 2\n"
    "end\n"
    "\n"
    "function on_game_start()\n"
    "\tlocal v = qa_text_twice(base)\n"
    "\tprintf(\"qa %s\", v)\n"
    "end\n"
)
BROKEN = SCRIPT_TEXT.replace("local v = qa_text_twice(base)", "local v = = qa_text_twice(base)")
CONFIG = DIR + "/qa_text.ltx"
# Windows-1251 with CRLF, as the game's configs are
CONFIG_BYTES = "[qa_section]:parent_a, parent_b\r\nname = Сталкер\r\ncost = 1200\r\n\r\n[qa_other]\r\nflag = true\r\n".encode("cp1251")
MIXED = DIR + "/qa_mixed.ltxp"
MIXED_BYTES = b"[a]\r\nk = 1\nm = 2\r\n"
NOTES = DIR + "/qa_notes.md"
NOTES_TEXT = "# Notes\n\nSome **text**, и по-русски.\n\n## Second\n\n- one\n- two\n"
PLAIN = DIR + "/qa_plain.seq"
BINARY = DIR + "/qa_blob.dat"
XML = DIR + "/qa_text.xml"
XML_TEXT = '<?xml version="1.0" encoding="windows-1251"?>\n<string_table>\n  <string id="qa"><text>QA</text></string>\n</string_table>\n'


def path_in(project: str, rel: str) -> str:
    return os.path.join(project, rel.replace("/", os.sep))


def write_bytes(project: str, rel: str, data: bytes) -> None:
    p = path_in(project, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb") as f:
        f.write(data)


def read_bytes(project: str, rel: str) -> bytes:
    with open(path_in(project, rel), "rb") as f:
        return f.read()


def view(**params) -> dict:
    return cmd("text.view", **params)


def phase_a(project: str, shots: str | None) -> None:
    opened = {}
    for rel, lang in ((SCRIPT, "lua"), (CONFIG, "ltx"), (MIXED, "ltx"), (XML, "xml"), (NOTES, "markdown")):
        r = cmd("content.view", action="reveal", path=rel.replace("/", "\\"), open=True)
        tab = "text:" + rel
        shown = qa.until(lambda: cmd("ui.tab", action="list").get("active_tab") == tab, 10)
        state = view() if shown else {}
        opened[rel] = state
        check(r.get("ok") and shown and state.get("language") == lang,
              "A %s opens from the Content Browser in the Text tab as %s" % (rel, lang), qa.short(state or r))
    # the browser keeps mod.ltx off its tiles (Mod Settings edits it): it opens as text by its path
    m = view(path="mod.ltx")
    check(m.get("ok") and m.get("language") == "ltx" and not m.get("read_only"), "A mod.ltx opens in the Text tab as ltx",
          qa.short(m))
    r = cmd("content.view", action="reveal", path=PLAIN.replace("/", "\\"), open=True)
    shown = qa.until(lambda: cmd("ui.tab", action="list").get("active_tab") == "text:" + PLAIN, 10)
    check(r.get("ok") and shown, "A a file whose name says nothing but that is text opens in the Text tab", qa.short(r))
    before = cmd("ui.tab", action="list").get("active_tab")
    r = cmd("content.view", action="reveal", path=BINARY.replace("/", "\\"), open=True)
    check(not r.get("ok") and "no text" in (r.get("error") or ""),
          "A a binary file opens in no tab, and the answer says it is no text", qa.short(r))
    check(cmd("ui.tab", action="list").get("active_tab") == before, "A the binary file left the tabs as they were")
    made = cmd("file.create", kind="quest", path=DIR + "/qa_text_quest")
    quest = made.get("path", "")
    r = view(path=quest)
    check(made.get("ok") and r.get("ok") and r.get("language") == "lua_data",
          "A one of the editor's own assets opens as text (Lua data)", qa.short(r))
    if shots:
        view(path=SCRIPT)
        qa.shot(shots, "text_script")


def phase_b(project: str, shots: str | None) -> None:
    # an ASCII script is in the encoding the game reads scripts in: its first other character goes in it
    s = view(path=SCRIPT)
    check(s.get("encoding") == "windows-1251" and s.get("line_endings") == "lf" and s.get("lines") == 12
          and not s.get("problems") and not s.get("dirty"),
          "B an ASCII script with LF: the game's encoding, its line endings and lines, no problem", qa.short(s))
    n = view(path=NOTES)
    check(n.get("encoding") == "utf-8", "B a UTF-8 Markdown file with Cyrillic is read as UTF-8", qa.short(n))
    c = view(path=CONFIG)
    check(c.get("encoding") == "windows-1251" and c.get("line_endings") == "crlf" and not c.get("dirty"),
          "B a Windows-1251 config with CRLF is read as it is", qa.short(c))
    write_bytes(project, SCRIPT, BROKEN.encode("utf-8"))
    got = qa.until(lambda: any(p.get("line") == 9 for p in view(path=SCRIPT).get("problems", [])), 20)
    s = view(path=SCRIPT)
    first = next((p for p in s.get("problems", []) if p.get("line") == 9), {})
    check(got and first.get("severity") == "error" and first.get("column"),
          "B a file written outside is read again: its syntax error is a problem at its line and column",
          qa.short(s))
    a = cmd("asset.check", path=SCRIPT)
    lines = [p.get("line") for p in a.get("problems", [])]
    check(a.get("ok") and 9 in lines, "B asset.check of the script gives the same problem", qa.short(a))
    write_bytes(project, SCRIPT, SCRIPT_TEXT.encode("utf-8"))
    fixed = qa.until(lambda: not view(path=SCRIPT).get("problems"), 20)
    check(fixed, "B the file fixed outside, its problems go", qa.short(view(path=SCRIPT)))
    if shots:
        write_bytes(project, SCRIPT, BROKEN.encode("utf-8"))
        qa.until(lambda: view(path=SCRIPT).get("problems"), 20)
        qa.shot(shots, "text_problem")
        write_bytes(project, SCRIPT, SCRIPT_TEXT.encode("utf-8"))
        qa.until(lambda: not view(path=SCRIPT).get("problems"), 20)


def phase_c(project: str, shots: str | None) -> None:
    for mode in ("preview", "split", "edit"):
        m = view(path=NOTES, mode=mode)
        check(m.get("mode") == mode, "C a Markdown file shows as %s" % mode, qa.short(m))
        if shots:
            qa.shot(shots, "text_markdown_" + mode)
    m = view(path=NOTES)
    heads = [s.get("name") for s in m.get("symbols", [])]
    check(heads == ["Notes", "Second"], "C Go to Symbol lists a Markdown file's headings", heads)
    s = view(path=SCRIPT, show="symbols")
    fns = [x.get("name") for x in s.get("symbols", [])]
    check("qa_text_twice" in fns and "on_game_start" in fns, "C Go to Symbol lists a script's functions", fns)
    if shots:
        qa.shot(shots, "text_symbols")
    view(path=SCRIPT, show="none")
    c = view(path=CONFIG)
    secs = [x.get("name") for x in c.get("symbols", [])]
    check(secs == ["qa_section", "qa_other"], "C Go to Symbol lists a config's sections", secs)
    f = view(path=SCRIPT, find="qa_text_twice")
    check((f.get("find") or {}).get("count") == 2, "C find marks the matches and counts them", qa.short(f.get("find")))
    g = view(path=SCRIPT, line=9, column=2, show="none")
    check((g.get("caret") or {}).get("line") == 9, "C a line goes to the caret", qa.short(g.get("caret")))


def phase_d(project: str, shots: str | None) -> None:
    before = read_bytes(project, CONFIG)
    view(path=CONFIG)
    saved = cmd("doc.save", doc="text:" + CONFIG)
    check(saved.get("ok") and read_bytes(project, CONFIG) == before,
          "D saving an untouched tab writes nothing: the Windows-1251 bytes stay", qa.short(saved))
    mixed = read_bytes(project, MIXED)
    view(path=MIXED)
    cmd("doc.save", doc="text:" + MIXED)
    check(read_bytes(project, MIXED) == mixed, "D a file with mixed line endings stays byte for byte")
    # an edit outside while the tab is clean: the tab follows it
    write_bytes(project, CONFIG, CONFIG_BYTES.replace(b"cost = 1200", b"cost = 1300"))
    follows = qa.until(lambda: view(path=CONFIG).get("encoding") == "windows-1251"
                       and not view(path=CONFIG).get("dirty"), 10)
    check(follows, "D a clean tab follows the file changed outside", qa.short(view(path=CONFIG)))
    look = cmd("script.lookup", name="xr_logic.pick_section_from_condlist")
    decl = (look.get("declared") or [{}])[0]
    if look.get("ok") and decl.get("source") == "game":
        g = view(path=decl.get("path", ""), source="game", line=decl.get("line", 1))
        check(g.get("ok") and g.get("read_only") and g.get("source") == "game" and not g.get("dirty"),
              "D a script of the game opens read-only from script.lookup's answer", qa.short(g))
        saved = cmd("doc.save", doc=g.get("tab", ""))
        check(not saved.get("ok") or not g.get("dirty"), "D a file of the game is never saved", qa.short(saved))
        if shots:
            qa.shot(shots, "text_game_script")
    else:
        check(True, "D no game is linked: the read-only open is not tried (%s)" % qa.short(look))


def phase_e(project: str, shots: str | None) -> None:
    look = cmd("script.lookup", name="qa_text.qa_text_twice")
    decl = (look.get("declared") or [{}])[0]
    check(look.get("ok") and decl.get("line") == 4 and decl.get("source") == "project",
          "E script.lookup finds a function of the project's script at its line", qa.short(look))
    comp = cmd("script.lookup", name="qa_text.qa_te", complete=True)
    texts = [o.get("text") for o in comp.get("offers", [])]
    check(comp.get("ok") and "qa_text_twice" in texts, "E a prefix after a script's dot completes to its names", texts)
    none = cmd("script.lookup", name="qa_text.nobody_declares_this")
    check(none.get("ok") and none.get("declared") == [], "E a name nobody declares answers an empty list", qa.short(none))


def phase_f(project: str, shots: str | None) -> None:
    st = cmd("state")
    if (st.get("scene") or {}).get("open"):
        check(True, "F the project has a scene: Play without one is not tried")
        return
    p = cmd("level.play", action="start", dry_run=True, timeout=900)
    check(p.get("ok") and p.get("way") == "module" and p.get("level") is None
          and (p.get("start") or {}).get("source") == "game" and "-start server" in (p.get("command") or ""),
          "F Play in a project without a scene builds the module and starts a new game at the game's start",
          qa.short(p, 800))


MANIFEST = "[module]\nid = qa_text\nname = QA Text\nversion = 1.0.0\napi = 1\nmode = *\ntarget = any\n"


def run(ctx: qa.Run) -> None:
    shots = ctx.shots
    project = os.path.join(ctx.work, "QaText")
    write_bytes(project, "mod.ltx", MANIFEST.encode("utf-8"))
    write_bytes(project, "scripts/qa_kept.script", b"function kept() end\n")
    write_bytes(project, SCRIPT, SCRIPT_TEXT.encode("utf-8"))
    write_bytes(project, CONFIG, CONFIG_BYTES)
    write_bytes(project, MIXED, MIXED_BYTES)
    write_bytes(project, NOTES, NOTES_TEXT.encode("utf-8"))
    write_bytes(project, PLAIN, b"frame_1\nframe_2\n")
    write_bytes(project, BINARY, b"\x00\x01binary\x00" * 64)
    write_bytes(project, XML, XML_TEXT.encode("cp1251"))
    ed = qa.Editor(project).start()
    try:
        game = qa._ltx_value(os.path.join(qa.REPO, "machine.ltx"), "game", "root")
        if game:
            link = cmd("game.link", path=game, scope="project")
            check(link.get("ok") and link.get("valid"), "the copy is linked to the author's game", qa.short(link))
        qa.run_phases({"A": phase_a, "B": phase_b, "C": phase_c, "D": phase_d, "E": phase_e, "F": phase_f},
                      ctx.only, {}, project, shots)
        qa.stalls()
    finally:
        ed.close()


def main() -> int:
    return qa.suite("TEXT", run, doc=__doc__, project=False)


if __name__ == "__main__":
    sys.exit(main())
