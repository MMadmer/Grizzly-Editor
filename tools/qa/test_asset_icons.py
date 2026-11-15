#!/usr/bin/env python3
"""Every asset kind of the editor's own has its drawn icon in the content browser, over MCP, on a
THROW-AWAY COPY of a project.

What it proves:
  A  one asset of every kind - quest, behaviour graph, character, character template, PDA
     interface, model, texture, sound - is made the way an author makes it (file.create, a character
     file written beside it)
  B  selected in the content browser, each one's tile draws its own icon
     (content.view 'icons'), and a file of no kind of the editor's own draws none
  C  the tile size answers over MCP (content.view tile=, in pt) at both ends of its range - under
     64 pt the folder is the list - and refuses a size outside it; with no size picked (tile_fit) the
     tiles fit the host, up to its default; view=list turns the grid into the list and view=tiles back,
     anything else is refused; with --shots the tiles are captured fitted, at 48 and 192, and the list
  D  every tile draws a vector glyph while it has no picture - a folder its folder, a text file its
     file type, one of the editor's own assets its own icon (content.view 'glyphs');
     the search narrows the view and the Sources pane turns off and on over MCP; Show in Explorer
     answers the path it would select (dry run); the toolbar's Add is off, saying why, on a
     read-only source and on in the project; with --shots the Add menu is captured
  E  a media asset's source is its companion, never a tile; a model written as OBJ text comes in
     through asset.import as a model asset, its tile draws the model asset's icon and glyph, a drag of
     it places an object by the asset's path, it opens in the model preview with its bounds, size and
     surfaces, and the Place panel lists it among the objects with the model asset's icon

    python tools/qa/test_asset_icons.py --project <project> [--shots <folder>]
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
import time

import qa
from qa import check, cmd, shot

KINDS = {
    "quest": "quest",
    "behaviour": "behaviour graph",
    "character": "character",
    "pda": "PDA interface",
    "model": "model",
    "texture": "texture",
    "sound": "sound",
}
# the glyph a tile of each media kind draws while it has no picture
MEDIA_GLYPHS = {"model": "model_asset", "texture": "texture_asset", "sound": "sound_asset"}
# a crate of a metre, as OBJ text: the test writes it, nothing binary is checked in
CRATE_OBJ = """o crate
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
v 0 0 1
v 1 0 1
v 1 1 1
v 0 1 1
vt 0 0
vt 1 0
vt 1 1
vt 0 1
f 1/1 2/2 3/3 4/4
f 5/1 8/4 7/3 6/2
f 1/1 5/2 6/3 2/4
f 2/1 6/2 7/3 3/4
f 3/1 7/2 8/3 4/4
f 4/1 8/2 5/3 1/4
"""
TEMPLATE = """return {
    npc      = 1,
    id       = "qa_icon_template",
    abstract = true,
    community = "stalker",
}
"""

def run(ctx: qa.Run) -> None:
    project, shots = ctx.project, ctx.shots
    ed = qa.Editor(project).start()
    try:
        # a folder of their own: the tiles are all there is to see in it, at any size
        m = cmd("file.mkdir", path="qa_icons")
        nav = cmd("content.view", action="go", source="project", folder="qa_icons")
        check(m.get("ok") and nav.get("ok"), "A the content browser is in a folder of the test's own",
              json.dumps(nav)[:300])

        # ------------------------------------------------------------ A: one of every kind
        made: dict[str, str] = {}
        for kind in KINDS:
            c = cmd("file.create", kind=kind)
            made[kind] = c.get("path", "")
            check(c.get("ok") and made[kind].replace("/", "\\").lower().startswith("qa_icons\\"),
                  "A Create makes a " + KINDS[kind] + " in the open folder", json.dumps(c)[:300])
        # a template is a character that says abstract = true: the file is all it takes
        template = "qa_icons\\QaTemplate.npcasset"
        with open(os.path.join(project, template), "w", encoding="utf-8", newline="\n") as f:
            f.write(TEMPLATE)
        plain = "qa_icons\\qa_icon_notes.txt"
        with open(os.path.join(project, plain), "w", encoding="utf-8") as f:
            f.write("a file of no kind of the editor's own\n")
        # the pickers' list follows the files a moment later: it never makes a caller wait
        deadline = time.monotonic() + 10
        while True:
            t = cmd("game.lookup", type="character_id", limit=500)
            if any(c.get("id") == "qa_icon_template" and "template" in c.get("extra", "") for c in t.get("entries", [])):
                break
            if time.monotonic() > deadline:
                break
            time.sleep(0.1)
        check(any(c.get("id") == "qa_icon_template" and "template" in c.get("extra", "") for c in t.get("entries", [])),
              "A the template is a character template to the editor too", json.dumps(t)[:300])

        # ------------------------------------------------------------ B: each tile's icon
        def icon_of(entry: str) -> tuple[str, dict]:
            folder = os.path.dirname(entry).replace("/", "\\")
            n = cmd("content.view", action="go", source="project", folder=folder)
            s = cmd("content.view", action="select", path=entry.replace("/", "\\"))
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
        cmd("content.view", action="go", source="project", folder="qa_icons")
        cmd("content.view", action="clear")
        contract = cmd("ui.contract")
        low, high = (contract.get("tile_range") or [48, 192])[:2]
        # the host shown keeps a size and a view of its own (the docked panel starts as a list, the drawer as tiles):
        # put back those
        before = cmd("content.view")
        kept = before.get("tile") or contract.get("tile_default", 96)
        kept_fit = before.get("tile_fit")
        kept_view = before.get("view") or "tiles"
        check(kept_view in ("tiles", "list"), "C the answer says how the grid shows its entries", json.dumps(before)[:300])
        check(isinstance(kept_fit, bool), "C the answer says whether the tiles fit the host", json.dumps(before)[:300])
        # with no size picked the tiles fit the host: two rows where they can, never over its default, never under 48
        fitted = cmd("content.view", tile_fit=True)
        most = contract.get("tile_default_panel", 64) if fitted.get("host") == "panel" else contract.get("tile_default", 96)
        list_below = fitted.get("list_below", 64)
        fit_view = "list" if (fitted.get("tile") or 0) < list_below else "tiles"
        check(fitted.get("ok") and fitted.get("tile_fit") is True and fitted.get("view") == fit_view
              and low <= (fitted.get("tile") or 0) <= most,
              "C tile_fit fits the tiles to the host (48..%d pt), a list under %d" % (most, list_below), json.dumps(fitted)[:300])
        shot(shots, "tiles_fit")
        bad = cmd("content.view", tile=high, tile_fit=True)
        check(not bad.get("ok") and "tile_fit" in str(bad.get("error")), "C a size and tile_fit=true together are refused",
              json.dumps(bad)[:300])
        for size in (low, high):
            n = cmd("content.view", tile=size)
            # a tile too small for a folder's name is a row of the list
            view = "list" if size < n.get("list_below", 64) else "tiles"
            check(n.get("ok") and n.get("tile") == size and n.get("view") == view and n.get("tile_fit") is False,
                  "C the tile size goes to %d pt over MCP, shows %s and is no longer fitted" % (size, view), json.dumps(n)[:300])
            shot(shots, "tiles_%d" % size)
        bad = cmd("content.view", tile=500)
        check(not bad.get("ok") and "48..192" in str(bad.get("error")), "C a size outside the range is refused",
              json.dumps(bad)[:300])
        listed = cmd("content.view", view="list")
        check(listed.get("ok") and listed.get("view") == "list" and listed.get("tile") == high,
              "C view=list shows the list and keeps the tile size", json.dumps(listed)[:300])
        shot(shots, "list")
        bad = cmd("content.view", view="grid")
        check(not bad.get("ok") and "tiles" in str(bad.get("error")), "C a view other than tiles or list is refused",
              json.dumps(bad)[:300])
        tiled = cmd("content.view", view="tiles")
        check(tiled.get("ok") and tiled.get("view") == "tiles", "C view=tiles shows the tiles again", json.dumps(tiled)[:300])
        if kept_fit:
            cmd("content.view", tile_fit=True)
        else:
            cmd("content.view", tile=kept)
        cmd("content.view", view=kept_view)

        # ------------------------------------------------------------ D: glyphs, search, pane, reveal
        cmd("file.mkdir", path="qa_icons/qa_sub")
        cmd("content.view", action="go", source="project", folder="qa_icons")
        want = {"qa_icons\\qa_sub": "folder", plain: "file_text", made["quest"].replace("/", "\\"): "quest"}
        for kind, glyph in MEDIA_GLYPHS.items():
            want[made[kind].replace("/", "\\")] = glyph
        for entry, glyph in want.items():
            s = cmd("content.view", action="select", path=entry)
            key = entry + ("\\" if glyph == "folder" else "")
            check(s.get("ok") and (s.get("glyphs") or {}).get(key) == glyph,
                  "D a %s tile draws the '%s' glyph" % (entry.rsplit("\\", 1)[-1], glyph), json.dumps(s)[:400])
        cmd("content.view", action="clear")

        n = cmd("content.view", query="notes")
        hit = cmd("content.view", action="select", path=plain)
        miss = cmd("content.view", action="select", path=made["quest"].replace("/", "\\"))
        check(n.get("ok") and n.get("query") == "notes" and hit.get("ok") and not miss.get("ok"),
              "D the search lists what matches and nothing else",
              json.dumps(n)[:200] + " | " + json.dumps(miss)[:200])
        n = cmd("content.view", query="")
        check(n.get("ok") and n.get("query") == "", "D an empty search lists the folder again", json.dumps(n)[:200])
        cmd("content.view", action="clear")

        off = cmd("content.view", sources_pane=False)
        on = cmd("content.view", sources_pane=True)
        check((off.get("sources_pane") or {}).get("on") is False and (on.get("sources_pane") or {}).get("on") is True,
              "D the Sources pane turns off and on over MCP", json.dumps(off)[:200] + " | " + json.dumps(on)[:200])

        r = cmd("ui.reveal", path=plain, dry_run=True)
        check(r.get("ok") and not r.get("opened") and str(r.get("shown", "")).lower().endswith(plain.lower()),
              "D Show in Explorer answers the file it would select, and opens nothing on a dry run", json.dumps(r)[:300])

        # Add works in the project's own source only: on a read-only one its button is off and says why. The audit is
        # the last frame's, and a background editor draws only when asked: a screenshot draws one first.
        def add_button() -> dict:
            cmd("shot.window")
            return next((c for c in cmd("ui.audit").get("controls", [])
                         if c.get("label") == "Add" and "Content" in str(c.get("window", ""))), {})

        ro = cmd("content.view", action="go", source="library", kind="home")
        add = add_button()
        check(ro.get("ok") and ro.get("can_add") is False and add and add.get("enabled") is False and add.get("why"),
              "D Add is off on a read-only source, and says why", json.dumps(ro)[:200] + " | " + json.dumps(add)[:300])
        rw = cmd("content.view", action="go", source="project", folder="qa_icons")
        add = add_button()
        check(rw.get("ok") and rw.get("can_add") is True and add.get("enabled") is True,
              "D and on in the project's own source", json.dumps(rw)[:200] + " | " + json.dumps(add)[:300])

        if shots:
            p = cmd("ui.popup", show="cb_add")
            check(p.get("ok"), "D the Add menu comes up over MCP", json.dumps(p)[:200])
            shot(shots, "add_menu")
            cmd("ui.popup", show="none")

        # a folder that is not there yet is made with the asset: nobody has to run file.mkdir first
        deep = cmd("file.create", kind="quest", path="qa_made/deeper")
        where = deep.get("path", "").replace("\\", "/")
        check(deep.get("ok") and where.startswith("qa_made/deeper/") and os.path.isfile(os.path.join(project, where)),
              "D file.create makes the folder it is asked to put the asset into", json.dumps(deep)[:300])
        up = cmd("file.create", kind="quest", path="../outside")
        check(not up.get("ok"), "D and a folder outside the project is refused", json.dumps(up)[:300])
        # the new asset's name is taken once in the project: its id comes from the name
        other = cmd("file.create", kind="quest", path="qa_made/elsewhere")
        names = {os.path.basename(where).lower(), os.path.basename(other.get("path", "")).lower()}
        check(other.get("ok") and len(names) == 2, "D two quests made in two folders get two names (two ids)",
              json.dumps(other)[:300])
        # a folder the project never lists, a name Windows keeps, a file in the way: refused in a sentence
        for bad, word in ((".git/hooks", "tooling"), ("releases/x", "editor's own"), ("temp/x", "editor's own"),
                          ("qa_made/con", "device"), ("mod.ltx/x", "is a file")):
            r = cmd("file.create", kind="quest", path=bad)
            check(not r.get("ok") and word in str(r.get("error")) and "win32" not in str(r.get("error"))
                  and not os.path.isdir(os.path.join(project, *bad.split("/"))),
                  "D file.create path=" + bad + " is refused in a sentence, nothing made", json.dumps(r)[:300])
        media(project, made)
    finally:
        ed.close()


def media(project: str, made: dict[str, str]) -> None:
    """E: the media kinds in the browser, the model preview and the Place panel."""
    # a texture asset names its picture: the picture beside it is its companion, no tile
    texture = made["texture"].replace("/", "\\")
    picture = texture[: -len(".texasset")] + ".png"
    with open(os.path.join(project, picture), "w", encoding="utf-8") as f:
        f.write("not a picture: the companion rule goes by the name\n")
    cmd("content.view", action="go", source="project", folder="qa_icons")
    seen = qa.until(lambda: cmd("content.view", action="select", path=picture).get("ok") is False, 10)
    tile = cmd("content.view", action="select", path=texture)
    check(seen and tile.get("ok"), "E a texture asset's picture is its companion, not a tile of its own",
          json.dumps(tile)[:300])
    cmd("content.view", action="clear")

    # a model of OBJ text comes in as a model asset with its source beside it
    work = tempfile.mkdtemp(prefix="qa_media_")
    try:
        obj = os.path.join(work, "qa_crate.obj")
        with open(obj, "w", encoding="utf-8", newline="\n") as f:
            f.write(CRATE_OBJ)
        imp = cmd("asset.import", **{"from": [obj], "to": "qa_icons/imported"})
    finally:
        qa.remove_tree(work)
    asset = "qa_icons\\imported\\qa_crate.meshasset"
    check(imp.get("ok") and any(a.replace("/", "\\").lower() == asset.lower() for a in imp.get("assets", [])),
          "E asset.import makes a model asset of an OBJ", json.dumps(imp)[:300])
    cmd("content.view", action="go", source="project", folder="qa_icons/imported")
    shown = qa.until(lambda: cmd("content.view", action="select", path=asset).get("ok"), 10)
    s = cmd("content.view", action="select", path=asset)
    entry = next((e for e in s.get("entries", []) if e.get("path", "").lower() == asset.lower()), {})
    names = [e.get("path") for e in s.get("entries", [])]
    check(shown and (s.get("icons") or {}).get(asset) == "model" and (s.get("glyphs") or {}).get(asset) == "model_asset",
          "E the imported model asset's tile draws the model asset's icon", json.dumps(s)[:400])
    check(not any(str(n).lower().endswith(".obj") for n in names), "E its OBJ is its companion, not a tile", names)
    check(entry.get("picture") == "model" and (entry.get("payload") or {}).get("kind") == "object"
          and (entry.get("place") or {}).get("ref", "").lower() == asset.lower(),
          "E it is pictured as a model and placed as an object by its project path", json.dumps(entry)[:400])
    cmd("content.view", action="clear")

    p = cmd("preview.model", path=asset.replace("\\", "/"))
    size = p.get("size") or []
    check(p.get("ok") and not p.get("error") and len(size) == 3 and all(abs(v - 1.0) < 0.01 for v in size)
          and p.get("bounds") and p.get("surfaces"),
          "E the model preview shows the model asset with its bounds, size (1 x 1 x 1 m) and surfaces",
          json.dumps(p)[:400])
    cmd("ui.tab", action="activate", doc="level")
    v = cmd("level.view", place_search="qa_crate")
    rows = [r for r in v.get("place", v).get("rows", []) if r.get("item")] if v.get("ok") else []
    found = qa.until(lambda: any(r.get("icon") == "model_asset" for r in
                                 cmd("level.view", place_search="qa_crate").get("place", {}).get("rows", [])), 10)
    check(found, "E the Place panel lists the model asset among the objects", json.dumps(rows)[:300])
    cmd("level.view", place_search="")


def main() -> int:
    return qa.suite("ASSET ICONS", run, doc=__doc__)


if __name__ == "__main__":
    sys.exit(main())
