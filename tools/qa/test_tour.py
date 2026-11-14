#!/usr/bin/env python3
"""A short tour of the window on a COPY of a project: every kind of tab, every dialog the contract names and every
menu is brought up over MCP at two sizes, and each is audited and pictured once.

For each of them:
  - the editor says it is there (`ui.tab` / `ui.dialog` / `ui.popup` answer ok and `ui.list` names it); a surface
    the project cannot show (no GitHub repository for Releases, say) must refuse in a sentence, never crash;
  - `ui.audit` finds nothing cut short and nothing that escaped its window (kit/audit.rs);
  - `shot.window` is a picture with the editor's size, nothing outside the window, and the dialog on top as `modal`.

The two sizes are the ones the design is held to (docs/UI_DESIGN.md): 1707 x 912 pt at UI scale 1.5 and 1280 x 720 pt
at 1.0. --shots keeps the pictures of the first size.

    python tools/qa/test_tour.py --project <project> [--shots <folder>] [--only A,B]

  A  every tab kind of ui.contract's document_kinds opens, comes up active and is audited
  B  every dialog of ui.contract opens, is the window's modal and is audited
  C  every menu of ui.contract opens to be looked at and is audited
"""
from __future__ import annotations

import sys

import qa
from qa import check, cmd

SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))
# what a tab of each kind opens on: nothing for a tool tab, a file of the project or the linked game for the rest
FILES = {"texture": "ui\\ui_icon_equipment", "model": "dynamics\\weapons\\wpn_ak74\\wpn_ak74"}


def audited(label: str) -> None:
    """Nothing escapes its window and no text is cut to nothing. A text cut with an ellipsis that still shows its
    end - a path, a status line, which the tooltip holds whole - is the design, and is only noted."""
    a = cmd("ui.audit")
    cut, escaped = a.get("truncated") or [], a.get("escaped") or []
    vanished = [t for t in cut if not t.get("shown") and (t.get("rect") or [0, 0, 0])[2] > 0]
    check(a.get("ok") and not vanished and not escaped, "%s: no text is cut to nothing and nothing escapes its window" % label,
          qa.short({"vanished": vanished[:3], "escaped": escaped[:3]}))
    for t in cut:
        if t not in vanished:
            print("NOTE %s: '%s' is shown cut short" % (label, str(t.get("label"))[:60]), flush=True)


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
    if kind in FILES:
        return cmd("ui.tab", action="open", kind=kind, path=FILES[kind])
    return cmd("ui.tab", action="open", kind=kind)


def phase_a(project: str, tag: str, shots: str | None, contract: dict) -> None:
    opened = []
    for kind in contract.get("document_kinds", []):
        label = "A %s %s" % (tag, kind)
        o = open_tab(kind, project)
        if not o.get("ok"):
            # the project cannot show it: it says why, in a sentence
            check(isinstance(o.get("error"), str) and len(o["error"]) > 12, label + " is refused in a sentence", qa.short(o))
            continue
        active = cmd("ui.list").get("active_tab", "")
        check(active.split(":")[0] == kind.split(":")[0] or kind in ("pda", "beh") and active.startswith(kind[:2]),
              label + " comes up as the active tab", active)
        audited(label)
        pictured(label, shots if tag == "max" else None, "tab_%s" % kind.replace(":", "_"))
        opened.append(active)
    cmd("ui.tab", action="activate", doc="level")
    for doc in opened:
        if doc and doc != "level":
            cmd("ui.tab", action="close", doc=doc, discard=True)


def phase_b(tag: str, shots: str | None, contract: dict) -> None:
    for dialog_id, window in (contract.get("dialogs") or {}).items():
        label = "B %s %s" % (tag, dialog_id)
        o = cmd("ui.dialog", action="open", id=dialog_id)
        if not o.get("ok"):
            check(isinstance(o.get("error"), str) and len(o["error"]) > 12, label + " is refused in a sentence", qa.short(o))
            continue
        shown = [d.get("id") for d in cmd("ui.list").get("dialogs", [])]
        check(dialog_id in shown, label + " is listed among the open dialogs", shown)
        audited(label)
        pictured(label, shots if tag == "max" else None, "dialog_%s" % dialog_id, modal=dialog_id)
        cmd("ui.dialog", action="close", id=dialog_id)
        check(dialog_id not in [d.get("id") for d in cmd("ui.list").get("dialogs", [])], label + " closes")


def phase_c(tag: str, shots: str | None, contract: dict) -> None:
    for menu in contract.get("menus") or []:
        label = "C %s menu %s" % (tag, menu)
        o = cmd("ui.popup", show="menu:" + menu)
        if not o.get("ok"):
            check(isinstance(o.get("error"), str) and len(o["error"]) > 12, label + " is refused in a sentence", qa.short(o))
            continue
        audited(label)
        pictured(label, shots if tag == "max" else None, "menu_%s" % menu)
        cmd("ui.popup", show="none")


def run(ctx: qa.Run) -> None:
    ed = qa.Editor(ctx.project).start()
    try:
        contract = cmd("ui.contract")
        for width, height, scale in SIZES:
            tag = "max" if width == SIZES[0][0] else "small"
            cmd("ui.scale", value=scale)
            check(cmd("app.window", action="resize", width=width, height=height).get("ok"),
                  "%s: the window takes %d x %d pt at scale %g" % (tag, width, height, scale))
            qa.run_phases({"A": lambda: phase_a(ctx.project, tag, ctx.shots, contract),
                           "B": lambda: phase_b(tag, ctx.shots, contract),
                           "C": lambda: phase_c(tag, ctx.shots, contract)}, ctx.only)
        cmd("ui.scale", value=1.5)
    finally:
        ed.close()


def main() -> int:
    return qa.suite("TOUR", run, doc=__doc__, phases="ABC")


if __name__ == "__main__":
    sys.exit(main())
