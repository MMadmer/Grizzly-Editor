#!/usr/bin/env python3
"""Details, Level Settings, the choosers and Preferences over MCP (docs/UI_DESIGN.md, section 6), on a THROW-AWAY
COPY of a project.

What it proves:
  A  Level Settings: property_inspector target=world lists its rows with the grid's label and help - a range left
     the label for the help ("Hemisphere quality [0-3]" reads "Hemisphere quality", its help says "Range: 0-3.");
     action=open raises the panel on the Level tab
  B  property_set writes a row the way editing it does and property_inspector reads it back; a value a row does not
     take is refused with what it takes, and nothing changes
  C  a spawn's Details: its Visual row opens the chooser (###dlg.choose), ui_dialog closes it as Cancel would
  D  Preferences opens as a tab, ui_audit finds nothing escaping it, and it closes
  E  shortcuts: list, set (moved from the command that had it), clear - and every shortcut is back as it was
  F  the editor's preferences file and layout come back byte for byte

    python tools/qa/test_properties.py --project <project>

The project must have the Escape scene (rawdata\\levels\\l01_escape.level), as test_world.py does: a project without it
is refused at once, and a scene that does not open ends the run.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP
LAYOUT = os.path.join(REPO, "level_imgui.ini")
PREFS = os.path.join(REPO, "level.ini")
PROJECTS = os.path.join(REPO, "_projects.ini")

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
        exe = qa_target.exe()
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
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
        try:
            mcp("window_control", action="close", discard=True)
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def wait_scene(level: str, seconds: float = 120) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            if level in (mcp("scene_info", timeout=60).get("scene") or "").lower():
                return True
        except OSError:
            pass
        time.sleep(2)
    return False


def rows(answer: dict) -> dict:
    return {r.get("key"): r for r in answer.get("rows", [])} if isinstance(answer, dict) else {}


def shot() -> None:
    # the editor redraws on demand: a picture makes it draw the frame a request waits for
    mcp("screenshot_editor", timeout=120)


def run(project: str) -> None:
    ed = Editor()
    try:
        ed.start(project)
        if "l01_escape" not in (mcp("scene_info").get("scene") or "").lower():
            opened = mcp("open_scene", timeout=900, scene="l01_escape.level")
            if not check(opened.get("ok"), "the project's Escape scene opens", json.dumps(opened)[:200]):
                # everything below stands on that scene: waiting for one that did not open only burns minutes
                raise RuntimeError("the Escape scene did not open")
        if not check(wait_scene("l01_escape"), "the project's Escape scene is open"):
            raise RuntimeError("the Escape scene did not open")

        # ------------------------------------------------------------ A: Level Settings
        opened = mcp("property_inspector", target="world", action="open", limit=500)
        check(opened.get("ok") and opened.get("open"), "A property_inspector opens Level Settings", json.dumps(opened)[:300])
        shot()
        panels = {p.get("id"): p for p in mcp("ui_list").get("panels", [])}
        check(panels.get("level_settings", {}).get("visible"), "A and raises it on the Level tab", json.dumps(panels.get("level_settings")))
        world = rows(opened)
        hemi = world.get("Scene\\Build options\\Lighting\\Hemisphere quality [0-3]", {})
        check(hemi.get("label") == "Hemisphere quality", "A a range leaves the label", json.dumps(hemi))
        check("Range: 0-3." in hemi.get("help", ""), "A and goes into the help", json.dumps(hemi))
        quality = world.get("Scene\\Build options\\Quality", {})
        check(bool(quality.get("help")), "A Quality says what it does", json.dumps(quality))
        weld = world.get("Scene\\Build options\\Optimizing\\Weld distance (m)", {})
        check(weld.get("label") == "Weld distance" and weld.get("unit") == "m", "A a unit leaves the label for the value",
              json.dumps(weld))
        audit = mcp("ui_audit", id="level_settings")
        check(audit.get("ok") and not audit.get("escaped"), "A nothing escapes Level Settings", json.dumps(audit.get("escaped"))[:400])

        # ------------------------------------------------------------ B: property_set
        key = "Scene\\Map version"
        before = world.get(key, {}).get("value", "")
        set_ = mcp("property_set", target="world", key=key, value="7.5")
        check(set_.get("ok") and set_.get("value") == "7.5", "B property_set writes a text row", json.dumps(set_))
        again = rows(mcp("property_inspector", target="world", limit=500)).get(key, {})
        check(again.get("value") == "7.5", "B property_inspector reads it back", json.dumps(again))
        bad = mcp("property_set", target="world", key="Scene\\Build options\\Lighting\\Hemisphere quality [0-3]", value="9")
        check(not bad.get("ok") and "0 to 3" in bad.get("error", ""), "B a number out of range is refused with the range",
              json.dumps(bad))
        bad = mcp("property_set", target="world", key="Scene\\Name", value="x")
        check(not bad.get("ok") and "cannot be set" in bad.get("error", ""), "B a caption is not set by text", json.dumps(bad))
        mcp("property_set", target="world", key=key, value=before)

        # ------------------------------------------------------------ C: the chooser
        # a physics object has a Visual row: the one that opens the chooser
        roster = mcp("list_spawn_sections", filter="physic_object", limit=20).get("sections") or []
        sections = [r.get("section", "") if isinstance(r, dict) else str(r) for r in roster]
        section = "physic_object" if "physic_object" in sections else next((s for s in sections if s), "")
        placed = mcp("place_spawn", section=section, x=1, y=0, z=1) if section else {"error": "no physic_object section"}
        name = (placed.get("object") or {}).get("name", "")
        if check(placed.get("ok") and bool(name), "C a spawn to look at", json.dumps(placed)[:300]):
            mcp("focus_object", name=name, select=True)
            mcp("property_inspector", target="selection", action="open")
            shot()
            details = rows(mcp("property_inspector", target="selection", limit=500))
            check(bool(details), "C its Details list rows", "")
            # a spawn with a visual: its playback controls go by their names, and one is pressed by its name
            anim = next((r for k, r in details.items() if k and (k.endswith("\\Model\\Animation") or
                                                                   k.endswith("\\Model\\AnimationControl"))), None)
            if anim:
                check(anim.get("value") == "First Frame,Play,Pause,Stop,Last Frame", "C playback controls go by their names",
                      json.dumps(anim))
                pressed = mcp("property_set", target="selection", key=anim.get("key"), value="Pause")
                check(pressed.get("ok"), "C a playback control is pressed by its name", json.dumps(pressed))
            chooser = mcp("property_choose", action="open", key="Visual")
            if check(chooser.get("ok"), "C the Visual row opens the chooser", json.dumps(chooser)):
                shot()
                dialogs = json.dumps(mcp("ui_dialog", action="list"))
                check("dlg.choose" in dialogs, "C the chooser is the kit dialog ###dlg.choose", dialogs[:400])
                closed = mcp("ui_dialog", action="close", id="choose")
                check(closed.get("ok"), "C ui_dialog closes it", json.dumps(closed))
                shot()
                shot()
                status = mcp("property_choose", action="status", key="Visual")
                check(status.get("form_alive") is False and status.get("pending") is False, "C as Cancel: no chooser, no edit left",
                      json.dumps(status))
            mcp("delete_selected")

        # ------------------------------------------------------------ D: Preferences
        tab = mcp("ui_tab", action="open", id="preferences")
        check(tab.get("ok"), "D Preferences opens as a tab", json.dumps(tab)[:300])
        shot()
        audit = mcp("ui_audit", id="preferences")
        check(audit.get("ok") and not audit.get("escaped"), "D nothing escapes Preferences", json.dumps(audit.get("escaped"))[:400])
        labels = {c.get("label") for c in audit.get("controls", [])}
        check("Search settings" in labels, "D it has its search", json.dumps(sorted(x for x in labels if x))[:400])
        prefs = mcp("preferences", action="get", key="bp_cull")
        check(prefs.get("ok") and prefs.get("value") in (True, False), "D bp_cull is a preference of its own", json.dumps(prefs))
        # the snap steps of Preferences ARE the viewport toolbar's
        before = mcp("preferences", action="get", key="snap_angle_step")
        mcp("preferences", action="set", key="snap_angle_step", value="11.25")
        angle = mcp("viewport_controls").get("angle", {})
        check(abs(float(angle.get("step", 0)) - 11.25) < 1e-3, "D the angle snap of Preferences is the toolbar's",
              json.dumps(angle))
        mcp("preferences", action="set", key="snap_angle_step", value=str(before.get("value", "5.625")))
        closed = mcp("ui_tab", action="close", id="preferences")
        check(closed.get("ok"), "D and closes", json.dumps(closed)[:300])

        # ------------------------------------------------------------ E: shortcuts
        listed = mcp("shortcuts")
        every = {s.get("id"): s.get("keys", "") for s in listed.get("shortcuts", [])}
        check(listed.get("ok") and len(every) > 10, "E shortcuts lists the commands", json.dumps(listed)[:300])
        names = {s.get("id"): s.get("label") for s in listed.get("shortcuts", [])}
        check(names.get('COMMAND_ZOOM_EXTENTS."Selected"') == "View · Frame Selection" and
              names.get("COMMAND_EDITOR_PREF") == "Edit · Preferences", "E commands go by their official names",
              json.dumps({k: names.get(k) for k in ('COMMAND_ZOOM_EXTENTS."Selected"', "COMMAND_EDITOR_PREF")}))
        undo = next((i for i in every if i == "COMMAND_UNDO"), "")
        safe = next((i for i in every if i == "COMMAND_TOGGLE_SAFE_RECT"), "")
        if check(bool(undo) and bool(safe), "E Undo and Toggle Safe Rect take shortcuts", json.dumps(sorted(every))[:300]):
            combo = "Ctrl+Shift+Alt+F12"
            first = mcp("shortcuts", action="set", command=safe, keys=combo)
            check(first.get("ok") and first.get("shortcut", {}).get("keys") == combo, "E set gives a command a shortcut",
                  json.dumps(first))
            moved = mcp("shortcuts", action="set", command=undo, keys=combo)
            check(moved.get("moved_from") == safe, "E the command that had it lets it go", json.dumps(moved))
            bad = mcp("shortcuts", action="set", command=undo, keys="Ctrl+NoSuchKey")
            check(not bad.get("ok"), "E a key nobody names is refused", json.dumps(bad))
            # as they were
            for command in (undo, safe):
                if every[command]:
                    mcp("shortcuts", action="set", command=command, keys=every[command])
                else:
                    mcp("shortcuts", action="clear", command=command)
            after = {s.get("id"): s.get("keys", "") for s in mcp("shortcuts").get("shortcuts", [])}
            check(after == every, "E every shortcut is back as it was",
                  json.dumps({k: (every.get(k), after.get(k)) for k in set(every) | set(after) if every.get(k) != after.get(k)}))
    finally:
        ed.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isfile(os.path.join(src, "rawdata", "levels", "l01_escape.level")):
        print("the project has no rawdata\\levels\\l01_escape.level: " + src)
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_propsproj_")
    copy = os.path.join(work, "QaProperties")
    shutil.copytree(src, copy)
    print("running against a copy: " + copy, flush=True)
    # the copy opens its OWN scene: project.ltx names the last one by absolute path
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

    # the editor rewrites these when it closes: they come back byte for byte
    backups = {}
    for path in (LAYOUT, PREFS, PROJECTS):
        if os.path.isfile(path):
            with open(path, "rb") as f:
                backups[path] = f.read()
    try:
        run(copy)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        for path, data in backups.items():
            with open(path, "wb") as f:
                f.write(data)
        check(all(open(p, "rb").read() == d for p, d in backups.items()), "F the preferences, the layout and the project list are back")
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"PROPERTIES: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
