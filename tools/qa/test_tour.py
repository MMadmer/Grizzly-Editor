#!/usr/bin/env python3
"""A short tour of the window on a COPY of a project: every kind of tab, every dialog the contract names and every
menu is brought up over MCP at two sizes, and each is audited and pictured once - in English, then in Russian.

For each of them:
  - the editor says it is there (`ui.tab` / `ui.dialog` / `ui.popup` answer ok and `ui.list` names it); a surface
    the project cannot show (no GitHub repository for Releases, say) must refuse in a sentence, never crash;
  - `ui.audit` finds nothing cut short and nothing that escaped its window (kit/audit.rs), no busy line or busy
    status (`loading`: a surface's busy line, a dialog footer's busy status, a card's busy line) and no error state in
    place of the content (`errors`);
  - `shot.window` is a picture with the editor's size, nothing outside the window, and the dialog on top as `modal`.
In Russian (docs/UI_DESIGN.md, section 9) each is audited again: `untranslated` is empty - every text of the
editor's own reads in Russian - and no text is cut whose English the English pass showed whole (Russian runs about
a quarter longer: a width follows its text).

The two sizes are the ones the design is held to (docs/UI_DESIGN.md): 1707 x 912 pt at UI scale 1.5 and 1280 x 720 pt
at 1.0. --shots keeps the pictures: the first size in the folder, the second in `small` inside it, the Russian ones
in `ru` and `ru/small`.

    python tools/qa/test_tour.py --project <project> [--shots <folder>] [--only A,B,R] [--theme <id>]

--theme takes the tour in a theme (`app.prefs key=theme` lists them; a light one, solarized-light say, holds the
light half of the kit): the editor is set to it over MCP before the first surface and must answer it back.

  A  every tab kind of ui.contract's document_kinds opens, comes up active and is audited; a library tab (light
     animations, textures, sounds) has its first entry picked first, so what an entry shows is pictured too
  B  every dialog of ui.contract opens, is the window's modal and is audited
  C  every menu of ui.contract opens to be looked at and is audited
  R  A, B and C again with the window in Russian (`app.prefs key=language value=ru`); the English pass of the same
     phases runs first (quietly when it was not asked for): it is what a Russian cut is weighed against
"""
from __future__ import annotations

import os
import sys

import qa
from qa import check, cmd

SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))
# what a tab of each kind opens on: nothing for a tool tab, a file of the linked game for the rest
FILES = {"model": "dynamics\\weapons\\wpn_ak74\\wpn_ak74"}
# the item the Items tab is opened on, as a change of it
ITEM = "wpn_ak74"

# the language the window shows now, and the English texts the English pass saw cut, per size
PASS = {"lang": "en", "tag": "max", "cut": {}}


def settled(label: str) -> None:
    """The surface shows its content, not a busy line: it is audited and pictured as the author sees it."""
    ready = qa.until(lambda: not cmd("ui.audit").get("loading"), 20)
    check(ready, "%s: shows its content within 20 s, not a busy line" % label,
          qa.short(cmd("ui.audit").get("loading")))


def shows_content(label: str) -> None:
    """The surface shows what it is for, not an error state in its place (`ui.audit` `errors`: the kit's full-surface
    error state, "This model cannot be shown ..."). A picture of an error screen proves nothing about the surface."""
    errors = cmd("ui.audit").get("errors") or []
    check(not errors, "%s: shows no error state in place of its content" % label, qa.short(errors))


def audited(label: str) -> None:
    """Nothing escapes its window and no text is cut to nothing. A text cut with an ellipsis that still shows its
    end - a path, a status line, which the tooltip holds whole - is the design, and is only noted. In Russian every
    text of the editor's own has its translation, and none is cut whose English the English pass showed whole."""
    a = cmd("ui.audit")
    cut, escaped = a.get("truncated") or [], a.get("escaped") or []
    vanished = [t for t in cut if not t.get("shown") and (t.get("rect") or [0, 0, 0])[2] > 0]
    check(a.get("ok") and not vanished and not escaped, "%s: no text is cut to nothing and nothing escapes its window" % label,
          qa.short({"vanished": vanished[:3], "escaped": escaped[:3]}))
    seen = PASS["cut"].setdefault(PASS["tag"], set())
    if PASS["lang"] == "en":
        seen.update(str(t.get("label")) for t in cut)
    else:
        check(a.get("language") == PASS["lang"], "%s: the window speaks %s" % (label, PASS["lang"]), a.get("language"))
        missing = a.get("untranslated") or []
        check(not missing, "%s: every text of the editor's own reads in Russian (%d without an entry)" % (label, len(missing)),
              qa.short([m.get("label") for m in missing[:12]]))
        # a translated text carries its English (`en`); data and the log's lines read the same in both
        longer = [t for t in cut if t.get("en") and str(t["en"]) not in seen]
        check(not longer, "%s: no Russian text is cut where its English fits" % label,
              qa.short([(t.get("label"), t.get("shown")) for t in longer[:6]]))
    for t in cut:
        if t not in vanished:
            print("NOTE %s: '%s' is shown cut short" % (label, str(t.get("label"))[:60]), flush=True)


def kept(shots: str | None, tag: str) -> str | None:
    """Where the pictures of a size go: the folder itself for the first size, `small` inside it for the other; the
    Russian ones in `ru` beside them."""
    if not shots:
        return None
    base = shots if PASS["lang"] == "en" else os.path.join(shots, PASS["lang"])
    return base if tag == "max" else os.path.join(base, "small")


def pictured(label: str, shots: str | None, name: str, modal: str | None = None) -> None:
    pic = qa.shot(shots, name)
    ok = pic.get("ok") and bool(pic.get("png_base64")) and not pic.get("outside")
    if modal:
        ok = ok and (pic.get("modal") or {}).get("id") == modal
    check(ok, "%s: the window is drawn%s, nothing lives outside it" % (label, " with its dialog on top" if modal else ""),
          qa.short(pic))


def open_tab(kind: str, project: str) -> dict:
    if kind == "level":
        return cmd("ui.tab", action="activate", doc="level")
    if kind in ("quest", "beh"):
        made = cmd("file.create", kind="quest" if kind == "quest" else "behaviour")
        return cmd("ui.tab", action="open", kind=kind, path=made.get("path", ""))
    if kind == "pda":
        return cmd("pda.open")
    if kind == "texture":
        # a picture the tab can show: a texture of the linked game, found by the editor's own listing
        listed = cmd("file.list", source="game", kind="dds", query="textures", limit=1)
        files = listed.get("files", []) if listed.get("ok") else []
        check(bool(files and files[0].get("path")), "A the linked game has a texture the tour can open", qa.short(listed))
        return cmd("ui.tab", action="open", kind=kind, path=files[0]["path"] if files else "", source="game")
    if kind in FILES:
        return cmd("ui.tab", action="open", kind=kind, path=FILES[kind])
    if kind == "sound":
        # a sound asset of the project from the template, its .wav beside it: the Sound tab on it
        made = cmd("file.create", kind="sound", path="qa_tour_sound")
        rel = made.get("path", "")
        if rel:
            import wave
            with wave.open(os.path.join(project, rel.replace("/", os.sep)).rsplit(".", 1)[0] + ".wav", "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(22050)
                w.writeframes(b"\0\0" * 4410)
        return cmd("preview.sound", open=True, path=rel)
    if kind == "items":
        # the tab is pictured with a form up: a change of an item of the linked game
        shown = cmd("items.view", select=ITEM)
        if not shown.get("ok"):
            shown = cmd("items.view", change=ITEM)
        return shown if shown.get("ok") else cmd("ui.tab", action="open", kind=kind)
    return cmd("ui.tab", action="open", kind=kind)


LIBRARIES = ("light_animations", "texture_library", "sound_library")


def media_tabs(project: str, tag: str, shots: str | None) -> list[str]:
    """The model and the texture tab on a media asset of the project: a model asset made of its parts (the
    template), a texture asset over a picture of the linked game brought into the project. Each is audited and
    pictured as the tabs on a file are."""
    opened: list[str] = []
    model = cmd("file.create", kind="model", path="qa_tour")
    label = "A %s %s model asset" % (PASS["lang"], tag)
    shown = cmd("preview.model", path=model.get("path", "")) if model.get("ok") else model
    check(shown.get("ok") and not shown.get("error") and shown.get("size"), label + " opens in the model preview",
          qa.short(shown))
    if shown.get("ok"):
        settled(label)
        shows_content(label)
        audited(label)
        pictured(label, kept(shots, tag), "tab_model_asset")
        opened.append(cmd("ui.list").get("active_tab", ""))
    listed = cmd("file.list", source="game", kind="dds", query="textures", limit=1)
    files = listed.get("files", []) if listed.get("ok") else []
    if not files:
        print("NOTE the linked game has no texture: no texture asset is toured", flush=True)
        return opened
    got = cmd("file.import", **{"from": "game", "path": files[0]["path"]})
    label = "A %s %s texture asset" % (PASS["lang"], tag)
    picture = str(got.get("path", "")).replace("\\", "/")
    # the pass before brought it in already: the file is there to wear
    check((got.get("ok") or got.get("exists")) and picture, label + ": a picture of the game is in the project",
          qa.short(got))
    if not picture:
        return opened
    folder, leaf = picture.rsplit("/", 1) if "/" in picture else ("", picture)
    asset = (folder + "/" if folder else "") + "qa_tour_" + leaf.rsplit(".", 1)[0] + ".texasset"
    qa.write(os.path.join(project, *asset.split("/")), 'return {\n  tex = 1,\n  source = "%s",\n}\n' % leaf)
    shown = cmd("preview.texture", action="open", path=asset, source="project")
    check(shown.get("ok") and (shown.get("asset") or {}).get("name"), label + " opens in the texture preview with its keys",
          qa.short(shown))
    if shown.get("ok"):
        settled(label)
        shows_content(label)
        audited(label)
        pictured(label, kept(shots, tag), "tab_texture_asset")
        opened.append(cmd("ui.list").get("active_tab", ""))
    return opened


def pick_first(kind: str, label: str) -> None:
    """A library tab is pictured with an entry picked: the timeline, the key actions, the description and the
    attenuation curve are only drawn for one. A library with no entry is said so."""
    opened = cmd("library.view", kind=kind, action="open")
    first = opened.get("first") if opened.get("ok") else None
    if not first:
        print("NOTE %s: the library holds no entry, so no entry is pictured" % label, flush=True)
        return
    picked = cmd("library.view", kind=kind, action="select", item=first)
    check(picked.get("ok"), label + " picks its first entry before it is pictured", qa.short(picked))


def phase_a(project: str, tag: str, shots: str | None, contract: dict) -> None:
    opened = []
    for kind in contract.get("document_kinds", []):
        label = "A %s %s %s" % (PASS["lang"], tag, kind)
        o = open_tab(kind, project)
        if not o.get("ok"):
            # the project cannot show it: it says why, in a sentence
            check(isinstance(o.get("error"), str) and len(o["error"]) > 12, label + " is refused in a sentence", qa.short(o))
            continue
        active = cmd("ui.list").get("active_tab", "")
        check(active.split(":")[0] == kind.split(":")[0] or kind in ("pda", "beh") and active.startswith(kind[:2]),
              label + " comes up as the active tab", active)
        if kind in LIBRARIES:
            pick_first(kind, label)
        settled(label)
        shows_content(label)
        audited(label)
        pictured(label, kept(shots, tag), "tab_%s" % kind.replace(":", "_"))
        opened.append(active)
    opened.extend(media_tabs(project, tag, shots))
    cmd("ui.tab", action="activate", doc="level")
    for doc in opened:
        if doc and doc != "level":
            cmd("ui.tab", action="close", doc=doc, discard=True)


def phase_b(tag: str, shots: str | None, contract: dict) -> None:
    for dialog_id, window in (contract.get("dialogs") or {}).items():
        label = "B %s %s %s" % (PASS["lang"], tag, dialog_id)
        o = cmd("ui.dialog", action="open", id=dialog_id)
        if not o.get("ok"):
            check(isinstance(o.get("error"), str) and len(o["error"]) > 12, label + " is refused in a sentence", qa.short(o))
            continue
        shown = [d.get("id") for d in cmd("ui.list").get("dialogs", [])]
        check(dialog_id in shown, label + " is listed among the open dialogs", shown)
        settled(label)
        shows_content(label)
        audited(label)
        pictured(label, kept(shots, tag), "dialog_%s" % dialog_id, modal=dialog_id)
        cmd("ui.dialog", action="close", id=dialog_id)
        check(dialog_id not in [d.get("id") for d in cmd("ui.list").get("dialogs", [])], label + " closes")


def phase_c(tag: str, shots: str | None, contract: dict) -> None:
    for menu in contract.get("menus") or []:
        label = "C %s %s menu %s" % (PASS["lang"], tag, menu)
        o = cmd("ui.popup", show="menu:" + menu)
        if not o.get("ok"):
            check(isinstance(o.get("error"), str) and len(o["error"]) > 12, label + " is refused in a sentence", qa.short(o))
            continue
        audited(label)
        pictured(label, kept(shots, tag), "menu_%s" % menu)
        cmd("ui.popup", show="none")


def themed(theme: str, contract: dict) -> None:
    """The editor takes the theme over MCP, answers it back, and the contract names it."""
    check(theme in (contract.get("themes") or []), "ui.contract names the theme %s" % theme, contract.get("themes"))
    check(cmd("app.prefs", action="set", key="theme", value=theme).get("ok"), "app.prefs sets the theme %s" % theme)
    got = cmd("app.prefs", key="theme")
    listed = [t.get("id") for t in got.get("themes") or []]
    check(got.get("value") == theme and theme in listed, "app.prefs answers the theme it set, among the themes",
          qa.short(got))


def run(ctx: qa.Run) -> None:
    asked = {c for c in (ctx.only or "").upper() if c.isalpha()} or set("ABCR")
    # the phases each pass tours; the Russian pass weighs its cuts against the English one of the same phases
    base = sorted(asked & set("ABC")) or ["A", "B", "C"]
    ed = qa.Editor(ctx.project).start()
    try:
        contract = cmd("ui.contract")
        if ctx.args.theme:
            themed(ctx.args.theme, contract)
        for lang in ["en"] + (["ru"] if "R" in asked else []):
            PASS["lang"] = lang
            if lang != "en":
                check(cmd("app.prefs", action="set", key="language", value=lang).get("ok"),
                      "R the window takes the language %s at once, without a restart" % lang)
            for width, height, scale in SIZES:
                tag = "max" if width == SIZES[0][0] else "small"
                PASS["tag"] = tag
                cmd("ui.scale", value=scale)
                check(cmd("app.window", action="resize", width=width, height=height).get("ok"),
                      "%s: the window takes %d x %d pt at scale %g" % (tag, width, height, scale))
                steps = {"A": lambda: phase_a(ctx.project, tag, ctx.shots, contract),
                         "B": lambda: phase_b(tag, ctx.shots, contract),
                         "C": lambda: phase_c(tag, ctx.shots, contract)}
                steps = {k: v for k, v in steps.items() if k in base}
                if lang == "en" and not asked & set("ABC"):
                    # asked for the Russian pass alone: the English one runs quietly, for its cuts
                    qa.run_phases(steps, "R", {"R": "".join(base)})
                else:
                    qa.run_phases(steps, ",".join(base))
        if "R" in asked:
            cmd("app.prefs", action="set", key="language", value="en")
        cmd("ui.scale", value=1.5)
    finally:
        ed.close()


def main() -> int:
    return qa.suite("TOUR", run, doc=__doc__, phases="ABCR",
                    add_args=lambda ap: ap.add_argument("--theme", default="", help="a theme's id to take the tour in"))


if __name__ == "__main__":
    sys.exit(main())
