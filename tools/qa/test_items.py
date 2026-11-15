#!/usr/bin/env python3
"""The module's items and the Items tab (Mod > Items), over MCP on a THROW-AWAY COPY of a project.

  K  the kind: file.create kind=item, asset.kinds and asset.reference kind=item know it, asset.check passes a good
     file and names the problem of each one an author is likely to make (no base, both a base and an edit, an unknown
     base, a value of the wrong type, a value no config can carry, an icon picture that is not there)
  L  items.list and items.info answer every item: file, kind, section, family, what each key resolves to
  T  the tab through items.view: it opens, lists the items under New and Changes, selects one; the form of a change of
     a weapon and of a new consumable carries the catalog's groups and rows (key, title, type, value, where it comes
     from); an edit of a catalog key, of a text and an added loot row are saved as exactly that difference - the
     author's comments and order kept; Revert drops an edit; + makes a new item from a game item and a change of one;
     the Delete Item question comes up (dlg.delete_item) and goes again, and a deleted file leaves the list
  I  the icon: a picture beside the asset is what the form shows, and it is drawn; a cell of a texture the project holds is
     drawn too; the module built to a folder carries the atlas, the asset's shipped text names its cell, and the picture
     itself does not ship
  H  In the Hands: a weapon's change finds the hands and the HUD model and shows its idle with a length; a motion row
     plays (the time moves) and stops where it is; an offset edited moves the answered transform at once; a HUD model
     nobody has says so; the item chooser's rows carry the items' icons; a row of More Sections that takes a key away
     and changes a list answers both
  F  the frame: ui.audit finds nothing cut to nothing or escaping at 1707x912 / 1.5 and 1280x720 / 1.0, in English and in
     Russian (every text of the editor's own translated), in the dark and the light theme; perf.stalls is empty

    python tools/qa/test_items.py --project <project> [--only K,L,T,I,H,F] [--shots <folder>]
"""
from __future__ import annotations

import json
import os
import struct
import sys
import tempfile
import time

import qa
from qa import check, cmd

PROJECT = [""]
# what the copy starts with: a change of a weapon written by hand, with its own comments and order
CHANGE = """-- QA: the AK as this module has it
return {
  item = 1,
  edit = "wpn_ak74",
  -- the shot stays as the game has it; the price is the module's
  set = {
    inv_weight = 3.2, -- lighter than the game's
  },
}
"""
CONSUMABLE = """return {
  item = 1,
  id = "qa_ration",
  base = "bread",
  name = { rus = "Paek", eng = "Ration" },
  description = "A QA ration.",
  set = { cost = 50 },
}
"""
BAD = {
    "no base": ("return { item = 1, id = \"qa_nobase\" }\n", "I405"),
    "both": ("return { item = 1, edit = \"bread\", base = \"bread\" }\n", "I402"),
    "unknown base": ("return { item = 1, base = \"qa_nothing_at_all\" }\n", "I408"),
    "wrong type": ("return { item = 1, edit = \"bread\", set = { inv_weight = \"heavy\" } }\n", "I412"),
    "value no config can carry": ("return { item = 1, edit = \"bread\", set = { cost = { a = 1 } } }\n", "I427"),
    "missing picture": ("return { item = 1, edit = \"bread\", icon = \"icons/qa_nowhere.png\" }\n", "I421"),
}
SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))
# a new item whose HUD section names a model nobody has
NO_HUD_MODEL = r'''return {
  item = 1,
  id = "qa_nohud",
  base = "wpn_pm",
  hud = { set = { item_visual = [[dynamics\weapons\qa_nowhere\qa_nowhere_hud]] } },
  sections = {
    { edit = "wpn_pm_hud", unset = { "anm_bore" }, add = { qa_list = { "a", "b" } }, drop = { qa_other = "c" } },
  },
}
'''


def path_of(rel: str) -> str:
    return os.path.join(PROJECT[0], *rel.split("/"))


def text_of(rel: str) -> str:
    with open(path_of(rel), encoding="utf-8") as f:
        return f.read()


def codes(answer: dict) -> list[str]:
    return [p.get("code", "") for p in answer.get("problems", [])]


def tga(path: str, w: int, h: int) -> None:
    """A picture of one colour as a 32-bit TGA: what an author's paint program writes."""
    head = struct.pack("<BBBHHBHHHHBB", 0, 0, 2, 0, 0, 0, 0, 0, w, h, 32, 8)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(head + bytes([40, 160, 220, 255]) * (w * h))


def dds(path: str, w: int, h: int) -> None:
    # a picture of one colour as an uncompressed 32-bit DDS: a texture an icon is a cell of
    head = struct.pack("<4sIIIIIII44xIIIIIIII", b"DDS ", 124, 0x100F, h, w, w * 4, 0, 1,
                       32, 0x41, 0, 32, 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000)
    head += struct.pack("<IIIII", 0x1000, 0, 0, 0, 0)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(head + bytes([220, 160, 40, 255]) * (w * h))


def rows_of(form: dict) -> dict[str, dict]:
    """Every key row of a form, the item's own section's and the HUD's (as hud.<key>)."""
    out = {}
    for g in form.get("groups") or []:
        for r in g.get("rows", []):
            out[r["key"]] = r
    for g in ((form.get("hud") or {}).get("groups") or []):
        for r in g.get("rows", []):
            out["hud." + r["key"]] = r
    return out


def view(**params) -> dict:
    return cmd("items.view", **params)


# ---- K: the kind ------------------------------------------------------------------------------------------------


def phase_k() -> None:
    made = cmd("file.create", kind="item", path="qa_items")
    rel = str(made.get("path", "")).replace("\\", "/")
    check(made.get("ok") and rel.endswith(".itemasset") and os.path.isfile(path_of(rel)), "K file.create kind=item makes an item file",
          qa.short(made))
    kinds = cmd("asset.kinds")
    names = json.dumps(kinds)
    check(kinds.get("ok") and "itemasset" in names, "K asset.kinds knows the item kind", names[:300])
    ref = cmd("asset.reference", kind="item")
    check(ref.get("ok") and "| `edit` |" in str(ref.get("text", "")), "K asset.reference kind=item lists the keys", qa.short(ref))
    good = cmd("asset.check", path=rel)
    errors = [p for p in good.get("problems", []) if p.get("severity") == "error"]
    check(good.get("ok") and not errors, "K the template checks without an error", qa.short(good))
    for what, (text, code) in BAD.items():
        bad_rel = "qa_items/qa_bad_%s.itemasset" % what.replace(" ", "_")
        qa.write(path_of(bad_rel), text)
        a = cmd("asset.check", path=bad_rel)
        check(code in codes(a), "K %s is named (%s)" % (what, code), qa.short(a))
        os.remove(path_of(bad_rel))
    os.remove(path_of(rel))


# ---- L: the commands of the data --------------------------------------------------------------------------------


def phase_l() -> None:
    qa.write(path_of("items/qa_ak.itemasset"), CHANGE)
    qa.write(path_of("items/qa_ration.itemasset"), CONSUMABLE)
    listed = qa.until(lambda: len(cmd("items.list").get("items", [])) >= 2 and cmd("items.list"), 10) or {}
    by_file = {i.get("file"): i for i in listed.get("items", [])}
    ak, ration = by_file.get("items/qa_ak.itemasset", {}), by_file.get("items/qa_ration.itemasset", {})
    check(ak.get("kind") == "change" and ak.get("section") == "wpn_ak74", "L items.list names the change and what it changes", qa.short(ak))
    check(ration.get("kind") == "new" and ration.get("section", "").endswith("qa_ration"), "L and the new item with its section",
          qa.short(ration))
    info = cmd("items.info", item="items/qa_ak.itemasset")
    keys = {k.get("key"): k for k in (info.get("main") or {}).get("keys", [])}
    check(info.get("ok") and keys.get("inv_weight", {}).get("from") == "this", "L items.info says the weight is the change's own",
          qa.short(keys.get("inv_weight")))
    check(keys.get("class", {}).get("from") in ("game", "base"), "L and the class is the game's", qa.short(keys.get("class")))


# ---- T: the tab -------------------------------------------------------------------------------------------------


def phase_t(shots: str | None) -> None:
    v = view()
    check(v.get("ok") and v.get("tab") == "items", "T items.view answers", qa.short(v))
    v = view(select="items/qa_ak.itemasset")
    check(v.get("ok") and v.get("active") and v.get("selected") == "items/qa_ak.itemasset", "T it opens the tab on an item", qa.short(v))
    check(cmd("ui.list").get("active_tab") == "items", "T Items is the active tab")
    kinds = {i["file"]: i["kind"] for i in v.get("items", [])}
    check(kinds.get("items/qa_ak.itemasset") == "change" and kinds.get("items/qa_ration.itemasset") == "new",
          "T the list holds both, a change and a new item", qa.short(kinds))
    form = v.get("form") or {}
    rows = rows_of(form)
    check(form.get("kind") == "change" and form.get("family") and len(form.get("groups") or []) >= 3,
          "T a weapon's change shows the catalog's groups of its family", qa.short({k: form.get(k) for k in ("kind", "family")}))
    weight = rows.get("inv_weight", {})
    check(weight.get("sets") and weight.get("value") == "3.2" and weight.get("from") == "this" and weight.get("title"),
          "T its weight row is the item's own, titled by the catalog", qa.short(weight))
    cost = rows.get("cost", {})
    check(cost and not cost.get("sets") and cost.get("value") and cost.get("type"), "T and the cost is the game's, shown with its type",
          qa.short(cost))
    check(bool(form.get("hud")), "T a weapon shows In the Hands", qa.short(form.get("hud"))[:200])
    audit = cmd("ui.audit", id="items")
    labels = {c.get("label") for c in audit.get("controls", [])}
    for want in ("Identity", "Name and Description", "Icon"):
        check(want in labels, "T the form draws " + want, qa.short(sorted(x for x in labels if x)))
    toolbar = {c.get("label") for c in cmd("ui.audit", id="##toolbar").get("controls", [])}
    check({"Save", "Revert", "Delete..."} <= toolbar, "T its toolbar holds Save, Revert and Delete...", qa.short(sorted(x for x in toolbar if x)))
    qa.shot(shots, "items_change")

    # an edit of a catalog key, saved as exactly that line
    before = text_of("items/qa_ak.itemasset")
    v = view(set={"cost": 1500})
    check(v.get("dirty") and rows_of(v.get("form") or {}).get("cost", {}).get("value") == "1500", "T an edit of a key is in the form, unsaved",
          qa.short(rows_of(v.get("form") or {}).get("cost")))
    check(text_of("items/qa_ak.itemasset") == before, "T and nothing is written before Save")
    v = view(save=True)
    after = text_of("items/qa_ak.itemasset")
    added = [l for l in after.splitlines() if l not in before.splitlines()]
    gone = [l for l in before.splitlines() if l not in after.splitlines()]
    check(v.get("ok") and not v.get("dirty") and len(added) == 1 and "cost" in added[0] and "1500" in added[0] and not gone,
          "T Save writes the one line that changed, comments and order kept", qa.short({"added": added, "gone": gone}))
    # Revert drops an edit
    v = view(set={"inv_weight": 9})
    v = view(discard=True)
    check(not v.get("dirty") and rows_of(v.get("form") or {}).get("inv_weight", {}).get("value") == "3.2"
          and text_of("items/qa_ak.itemasset") == after, "T Revert drops the edit, the file untouched", qa.short(v.get("dirty")))

    # a new consumable: its texts and a loot row
    v = view(select="items/qa_ration.itemasset")
    form = v.get("form") or {}
    check(form.get("kind") == "new" and (form.get("identity") or {}).get("base") == ["bread"] and form.get("groups"),
          "T a new consumable's form starts from its base", qa.short(form.get("identity")))
    texts = form.get("texts") or {}
    check(texts.get("name.eng", {}).get("value") == "Ration", "T its texts are the file's, a row per language", qa.short(texts))
    before = text_of("items/qa_ration.itemasset")
    v = view(text={"name.eng": "Field Ration"}, loot={"count": [0, 2], "chance": 0.05, "by": {"bandit": 0.1}}, save=True)
    after = text_of("items/qa_ration.itemasset")
    check(v.get("ok") and '"Field Ration"' in after and "Ration\"" not in after.replace("Field Ration", ""),
          "T the text is saved in its language", after[:400])
    check("loot" in after and "bandit" in after and "0.05" in after and "cost = 50" in after,
          "T the loot row is added and the rest kept", after[:500])
    found = ((v.get("form") or {}).get("found") or {}).get("loot") or {}
    check(found.get("chance") == 0.05 and found.get("by"), "T Where It Is Found shows the loot", qa.short(found))
    qa.shot(shots, "items_new")

    # + makes a new item from a game item and a change of one
    v = view(new="vodka")
    sel = v.get("selected", "")
    check(v.get("ok") and sel.startswith("items/") and os.path.isfile(path_of(sel)) and (v.get("form") or {}).get("kind") == "new",
          "T + New Item makes the file and selects it", qa.short({"selected": sel, "status": v.get("status")}))
    check("base = \"vodka\"" in text_of(sel) if os.path.isfile(path_of(sel)) else False, "T and it starts from the game item", sel)
    v = view(change="medkit")
    sel2 = v.get("selected", "")
    check(v.get("ok") and os.path.isfile(path_of(sel2)) and (v.get("form") or {}).get("kind") == "change",
          "T + Change a Game Item makes a change of it", qa.short({"selected": sel2}))
    for show in ("new", "base"):
        p = view(show=show)
        chooser = (p.get("picker") or {}).get("chooser") or {}
        check(p.get("ok") and (p.get("picker") or {}).get("show") == show and chooser.get("shown", 0) > 10,
              "T show=%s puts the chooser over the game's items up" % show, qa.short(p.get("picker")))
        qa.shot(shots, "items_chooser_" + show)
        cmd("ui.dialog", action="close", id="dlg.choose")
    bad = view(show="nowhere")
    check(not bad.get("ok") and "new" in str(bad.get("error")), "T an unknown pick list is refused, naming them", qa.short(bad))

    # Delete: the question names the file, and the file deleted leaves the list
    v = view(select=sel, ask_delete=True)
    up = qa.until(lambda: "dlg.delete_item" in json.dumps(cmd("ui.dialog").get("dialogs")), 5)
    check(up, "T Delete... asks through dlg.delete_item", qa.short(cmd("ui.dialog")))
    qa.shot(shots, "items_delete")
    cmd("ui.dialog", action="close", id="dlg.delete_item")
    check(os.path.isfile(path_of(sel)), "T and Cancel keeps the file")
    for f in (sel, sel2):
        cmd("file.delete", path=f)
    v = qa.until(lambda: (lambda a: a if all(i["file"] not in (sel, sel2) for i in a.get("items", [])) else None)(view()), 10) or {}
    check(bool(v) and v.get("count") == 2, "T the deleted files leave the list", qa.short(v.get("items")))
    gone = view(select="items/qa_nobody.itemasset")
    check(not gone.get("ok") and "qa_nobody" in str(gone.get("error")), "T an item that is not there is refused by name", qa.short(gone))


# ---- I: the icon ------------------------------------------------------------------------------------------------


def phase_i(module: str) -> None:
    tga(path_of("items/icons/qa_ration.tga"), 100, 50)
    text = text_of("items/qa_ration.itemasset").replace("  set = {", '  icon = "icons/qa_ration.tga",\n  set = {', 1)
    qa.write(path_of("items/qa_ration.itemasset"), text)
    v = qa.until(lambda: (lambda a: a if ((a.get("form") or {}).get("icon") or {}).get("own", {}).get("form") == "picture" else None)(
        view(select="items/qa_ration.itemasset")), 10) or {}
    icon = ((v.get("form") or {}).get("icon") or {})
    check(icon.get("own", {}).get("file") == "icons/qa_ration.tga" and "qa_ration.tga" in str(icon.get("shown")),
          "I the form shows the picture beside the asset", qa.short(icon))
    drawn = qa.until(lambda: ((view().get("form") or {}).get("icon") or {}).get("drawn"), 10)
    check(drawn, "I and the picture is drawn")
    # a cell of a texture the project holds: drawn from the project, not looked for in the game
    dds(path_of("textures/ui/qa_icons.dds"), 100, 100)
    cell = text_of("items/qa_ak.itemasset").replace("  set = {", '  icon = { texture = [[ui\\qa_icons]], x = 0, y = 0, w = 2, h = 1 },\n  set = {', 1)
    qa.write(path_of("items/qa_ak.itemasset"), cell)
    v = qa.until(lambda: (lambda a: a if ((a.get("form") or {}).get("icon") or {}).get("own", {}).get("form") == "cell" else None)(
        view(select="items/qa_ak.itemasset")), 10) or {}
    shown = ((v.get("form") or {}).get("icon") or {}).get("shown") or {}
    check(shown.get("texture") == "ui\\qa_icons" and shown.get("w") == 2, "I a cell icon names the project's texture", qa.short(shown))
    drawn = qa.until(lambda: ((view().get("form") or {}).get("icon") or {}).get("drawn"), 10)
    check(drawn, "I and a cell of a texture the project holds is drawn")
    qa.write(path_of("items/qa_ak.itemasset"), cell.replace('  icon = { texture = [[ui\\qa_icons]], x = 0, y = 0, w = 2, h = 1 },\n', ""))
    out = tempfile.mkdtemp(prefix="grizzly_items_")
    try:
        built = cmd("mod.build", target="folder", path=out)
        check(built.get("ok"), "I the module builds to a folder", qa.short(built))
        root = os.path.join(out, "modules", module)
        atlas = os.path.join(root, "gamedata", "textures", "ui", module + "_icons.dds")
        check(os.path.isfile(atlas), "I the atlas ships", atlas)
        shipped = os.path.join(root, "items", "qa_ration.itemasset")
        body = open(shipped, encoding="utf-8").read() if os.path.isfile(shipped) else ""
        check("ui\\\\%s_icons" % module in body or ("%s_icons" % module) in body and "x =" in body,
              "I the shipped text names its cell of the atlas", body[:300])
        check(not os.path.exists(os.path.join(root, "items", "icons", "qa_ration.tga")), "I and the picture itself does not ship")
    finally:
        qa.remove_tree(out)


# ---- H: In the Hands --------------------------------------------------------------------------------------------


def hands_of(**params) -> dict:
    return view(**params).get("hands") or {}


def phase_h(shots: str | None) -> None:
    view(select="items/qa_ak.itemasset")
    # the hands' model and its motion sets are read the first time: seconds, not frames
    h = qa.until(lambda: (lambda x: x if x.get("status") in ("ready", "missing") else None)(hands_of()), 120) or {}
    check(h.get("status") == "ready" and h.get("hands_found") and h.get("item_found"),
          "H a weapon's change finds the hands and its HUD model", qa.short(h))
    check(h.get("line") == "anm_idle" and h.get("motion") and (h.get("length") or 0) > 0 and not h.get("playing"),
          "H a row that is no motion shows the idle, still, with its length", qa.short({k: h.get(k) for k in ("line", "motion", "length", "playing")}))
    check(h.get("wide") and h.get("anchor") and (h.get("fov") or 0) > 10, "H the picture is wide, hung on the hands' bone, at the HUD's fov",
          qa.short({k: h.get(k) for k in ("wide", "anchor", "fov")}))
    pos = ((h.get("item_transform") or {}).get("position") or [0, 0, 0])
    check(pos[2] > 0.05 and pos[1] < 0, "H the weapon is in front of the camera and below its eyes", qa.short(pos))
    cmd("ui.scroll", id="items.preview", to="bottom")
    qa.shot(shots, "items_hands_idle")
    # a motion row plays; Stop holds it where it is
    h = hands_of(hud_row="anm_reload")
    t1 = h.get("time", 0)
    check(h.get("line") == "anm_reload" and h.get("playing"), "H a motion row plays its line", qa.short({k: h.get(k) for k in ("line", "playing")}))
    time.sleep(0.6)
    t2 = hands_of().get("time", 0)
    check(abs(t2 - t1) > 0.2, "H and the time moves while it plays", qa.short([t1, t2]))
    h = hands_of(hands_play=False, hands_time=1.5)
    t3 = h.get("time", 0)
    time.sleep(0.5)
    t4 = hands_of().get("time", 0)
    check(not h.get("playing") and abs(t3 - 1.5) < 0.01 and t3 == t4, "H stopped, the time stays where it was put", qa.short([t3, t4]))
    cmd("ui.scroll", id="items.preview", to="bottom")
    qa.shot(shots, "items_hands_reload")
    # an offset edited moves the picture at once, before any save
    before = hands_of()
    v = view(set={"hud.hands_position_16x9": [0.1, -0.05, 0.2]})
    after = v.get("hands") or {}
    hp = ((after.get("hands_transform") or {}).get("position") or [])
    check(v.get("dirty") and [round(x, 4) for x in hp] == [0.1, -0.05, 0.2]
          and (after.get("item_transform") or {}).get("position") != (before.get("item_transform") or {}).get("position"),
          "H an offset edited moves the hands and the item at once", qa.short({"hands": hp, "item": (after.get("item_transform") or {}).get("position")}))
    view(discard=True)
    # a HUD model nobody has
    qa.write(path_of("items/qa_nohud.itemasset"), NO_HUD_MODEL)
    v = qa.until(lambda: (lambda a: a if (a.get("hands") or {}).get("status") == "missing" else None)(
        view(select="items/qa_nohud.itemasset")), 60) or {}
    missing = (v.get("hands") or {}).get("missing") or {}
    check(missing.get("what") == "item_model" and "qa_nowhere" in str(missing.get("name")),
          "H a HUD model nobody has says so in the picture's place", qa.short(v.get("hands")))
    cmd("ui.scroll", id="items.preview", to="bottom")
    qa.shot(shots, "items_hands_missing")
    # a row of More Sections that takes a key away and changes a list
    rows = ((v.get("form") or {}).get("sections") or [{}])[0]
    check(rows.get("unset") == ["anm_bore"] and rows.get("add") and rows.get("drop"),
          "H a row of More Sections answers what it takes away and does to lists", qa.short(rows))
    cmd("ui.scroll", id="items", to="bottom")
    qa.shot(shots, "items_more_sections")
    cmd("ui.scroll", id="items", to="top")
    os.remove(path_of("items/qa_nohud.itemasset"))
    # the item chooser carries the items' icons
    p = view(show="new")
    pictured = qa.until(lambda: ((view().get("picker") or {}).get("chooser") or {}).get("pictured", 0) > 0 and view(), 20) or {}
    chooser = (pictured.get("picker") or {}).get("chooser") or {}
    check(((p.get("picker") or {}).get("chooser") or {}).get("icons") and chooser.get("pictured", 0) > 0,
          "H the item chooser's rows carry the items' icons", qa.short({k: chooser.get(k) for k in ("icons", "pictured", "shown")}))
    qa.shot(shots, "items_chooser_icons")
    cmd("ui.dialog", action="close", id="dlg.choose")
    view(select="items/qa_ak.itemasset")


# ---- F: the frame -----------------------------------------------------------------------------------------------


def phase_f(shots: str | None) -> None:
    view(select="items/qa_ak.itemasset")
    seen: dict[str, set] = {}
    for theme in ("", "grizzly-light"):
        if theme:
            check(cmd("app.prefs", action="set", key="theme", value=theme).get("ok"), "F the window takes the light theme")
        for lang in ("en", "ru"):
            cmd("app.prefs", action="set", key="language", value=lang)
            for width, height, scale in SIZES:
                tag = "%s %s %dx%d%s" % ("F", lang, width, height, " light" if theme else "")
                cmd("ui.scale", value=scale)
                cmd("app.window", action="resize", width=width, height=height)
                view(select="items/qa_ak.itemasset")
                qa.until(lambda: not cmd("ui.audit", id="items").get("loading"), 10)
                for where in ("top", "bottom"):
                    cmd("ui.scroll", id="items", to=where)
                    cmd("ui.scroll", id="items.preview", to=where)
                    a = cmd("ui.audit", id="items")
                    cut = a.get("truncated") or []
                    vanished = [t for t in cut if not t.get("shown") and (t.get("rect") or [0, 0, 0])[2] > 0]
                    check(a.get("ok") and not vanished and not a.get("escaped"),
                          "%s %s: nothing is cut to nothing, nothing escapes" % (tag, where),
                          qa.short({"vanished": vanished[:3], "escaped": (a.get("escaped") or [])[:3]}))
                    key = "%dx%d" % (width, height)
                    if lang == "en":
                        seen.setdefault(key, set()).update(str(t.get("label")) for t in cut)
                    else:
                        missing = [m.get("label") for m in a.get("untranslated") or []]
                        check(not missing, "%s %s: every text of the editor's own reads in Russian" % (tag, where), qa.short(missing[:12]))
                        longer = [t.get("label") for t in cut if t.get("en") and str(t["en"]) not in seen.get(key, set())]
                        check(not longer, "%s %s: no Russian text is cut where its English fits" % (tag, where), qa.short(longer[:6]))
                    qa.shot(shots, "items_%s_%s_%d%s" % (lang, where, width, "_light" if theme else ""))
                cmd("ui.scroll", id="items", to="top")
    cmd("app.prefs", action="set", key="language", value="en")
    cmd("app.prefs", action="set", key="theme", value="grizzly-dark")
    cmd("ui.scale", value=1.5)
    closed = cmd("ui.tab", action="close", doc="items")
    check(closed.get("ok"), "F the tab closes when nothing is unsaved", qa.short(closed))
    check(view().get("open") is False, "F and items.view says it is closed")


def run(ctx: qa.Run) -> None:
    PROJECT[0] = ctx.project
    ed = qa.Editor(ctx.project).start()
    try:
        module = cmd("mod.info").get("manifest", {}).get("id", "")
        check(bool(module), "the project has a module id", module)
        # the checks count items: an author's project may bring items of its own
        for b, _, fs in os.walk(ctx.project):
            for f in fs:
                if f.lower().endswith(".itemasset"):
                    os.remove(os.path.join(b, f))
        qa.run_phases(
            {"K": phase_k, "L": phase_l, "T": lambda: phase_t(ctx.shots), "I": lambda: phase_i(module),
             "H": lambda: phase_h(ctx.shots), "F": lambda: phase_f(ctx.shots)},
            ctx.only,
            {"T": "L", "I": "L", "H": "L", "F": "L"},
        )
        qa.stalls()
    finally:
        ed.close()


def main() -> int:
    return qa.suite("ITEMS", run, doc=__doc__, phases="KLTIHF")


if __name__ == "__main__":
    sys.exit(main())
