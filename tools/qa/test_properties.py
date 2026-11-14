#!/usr/bin/env python3
"""Details, Level Settings, the choosers and Preferences over MCP (docs/UI_DESIGN.md, section 6), on a THROW-AWAY
COPY of a project.

What it proves:
  A  Level Settings: scene.get world=true lists the level's properties by their keys, with a label in Title Case, a
     unit and a help that says the range ("Sky Light Quality", "Range: 0 to 3."); ui.panel raises the panel
  B  scene.set writes a property the way editing it does and scene.get reads it back; a value a property does not
     take is refused with what it takes, and nothing changes; an object's angles go in degrees
  C  a spawn's properties: its playback buttons go by their names; scene.choices lists what a pick takes - the
     models of Visual, Fixed Bones the model's bones (several), an anomaly's Attack Animation its model's motions, a
     helicopter's Motion the game's .anm files - and an empty one says why
  D  Preferences opens as a tab, ui_audit finds nothing escaping it, and it closes
  E  shortcuts: app.keys lists the actions, bind (moved from the action that had it), clear - and every shortcut is back as it was
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
PROJECTS = qa_target.projects_ini()

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
        qa_target.guard_project(project)
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                if mcp("mod.info", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("app.quit", discard=True)
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
            if level in (mcp("scene.info", timeout=60).get("file") or "").lower():
                return True
        except OSError:
            pass
        time.sleep(2)
    return False


def props(answer: dict) -> dict:
    """The properties of an answer of scene.get (the level's, or the first object's), by key."""
    if not isinstance(answer, dict):
        return {}
    if "properties" in answer:
        found = answer.get("properties") or []
    else:
        found = ((answer.get("objects") or [{}])[0]).get("properties") or []
    return {r.get("key"): r for r in found}


def world() -> dict:
    return props(mcp("scene.get", world=True))


def mine(name: str, keys=None) -> dict:
    extra = {"keys": keys} if keys else {}
    return props(mcp("scene.get", objects=[name], **extra))


def shot() -> None:
    # the editor redraws on demand: a picture makes it draw the frame a request waits for
    mcp("shot.window", timeout=120)


def placed_spawn(section: str) -> str:
    """A spawn of `section` placed and selected; its name, or "" (a FAIL says why)."""
    placed = mcp("scene.place", kind="spawn", section=section, position=[3, 0, 3])
    name = (placed.get("object") or {}).get("name", "")
    check(placed.get("ok") and bool(name), f"C a {section} to pick for", json.dumps(placed)[:300])
    return name


def choosers(spawn: str) -> None:
    """The lists a pick takes: a model's bones, a model's motions, the game's .anm files - picked, never typed."""
    # the physic object wears a model now: its bones, several at once
    bones = mcp("scene.choices", object=spawn, key="props.physic.fixed_bones")
    items = bones.get("choices") or []
    check(bones.get("ok") and bones.get("list") == "skeletonbones" and bones.get("total", 0) > 1
          and bones.get("several") is True,
          f"C Fixed Bones lists the bones of the spawn's model ({bones.get('total')}), several to pick",
          json.dumps(bones)[:300])
    if len(items) > 1:
        two = ",".join(items[:2])
        mcp("scene.set", objects=[spawn], key="props.physic.fixed_bones", value=two)
        fixed = mine(spawn, ["props.physic.fixed_bones"]).get("props.physic.fixed_bones") or {}
        check(fixed.get("value") == two, "C the picked bones are the property's value", json.dumps(fixed)[:300])
        mcp("scene.set", objects=[spawn], key="props.physic.fixed_bones", value="")
    # an object without a model has nothing to pick, and says so
    bare = placed_spawn("physic_object")
    if bare:
        empty = mcp("scene.choices", object=bare, key="props.physic.fixed_bones")
        check(empty.get("total") == 0 and empty.get("empty") == "The object has no model.",
              "C an object without a model says it has none", json.dumps(empty)[:300])
        mcp("scene.delete", objects=[bare])
    # an anomaly's Attack Animation: the motions of its model
    zone = placed_spawn("zone_burning_fuzz_weak")
    if zone:
        motions = mcp("scene.choices", object=zone, key="props.zone.attack_animation")
        check(motions.get("list") == "skeletonanims" and "idle" in (motions.get("choices") or []),
              f"C Attack Animation lists the motions of the anomaly's model ({motions.get('total')})",
              json.dumps(motions)[:300])
        mcp("scene.delete", objects=[zone])
    # a helicopter's Motion: the game's .anm animations
    heli = placed_spawn("helicopter")
    if heli:
        anims = mcp("scene.choices", object=heli, key="props.motion", limit=5)
        listed = anims.get("choices") or []
        check(anims.get("list") == "gameanim" and anims.get("total", 0) > 0
              and all(i.lower().endswith((".anm", ".anms")) for i in listed),
              f"C Motion lists the game's .anm animations ({anims.get('total')})", json.dumps(anims)[:300])
        mcp("scene.delete", objects=[heli])


def run(project: str) -> None:
    ed = Editor()
    try:
        ed.start(project)
        if "l01_escape" not in (mcp("scene.info").get("file") or "").lower():
            opened = mcp("scene.open", timeout=900, path="rawdata/levels/l01_escape.level")
            if not check(opened.get("ok"), "the project's Escape scene opens", json.dumps(opened)[:200]):
                # everything below stands on that scene: waiting for one that did not open only burns minutes
                raise RuntimeError("the Escape scene did not open")
        if not check(wait_scene("l01_escape"), "the project's Escape scene is open"):
            raise RuntimeError("the Escape scene did not open")

        # ------------------------------------------------------------ A: Level Settings
        opened = mcp("ui.panel", id="level_settings", action="open")
        check(opened.get("ok"), "A ui.panel opens Level Settings", json.dumps(opened)[:300])
        shot()
        panels = {p.get("id"): p for p in mcp("ui.list").get("panels", [])}
        check(panels.get("level_settings", {}).get("visible"), "A and raises it on the Level tab", json.dumps(panels.get("level_settings")))
        level = world()
        sky = level.get("build.sky_quality", {})
        check(sky.get("label") == "Sky Light Quality" and sky.get("section") == "Build", "A a property has a plain label and its section",
              json.dumps(sky))
        check("Range: 0 to 3." in sky.get("help", ""), "A and its help says its range", json.dumps(sky))
        quality = level.get("build.quality", {})
        check(bool(quality.get("help")) and quality.get("choices") == ["Draft", "High", "Custom"],
              "A Quality says what it does and offers its words", json.dumps(quality))
        weld = level.get("build.weld_distance", {})
        check(weld.get("label") == "Weld Distance" and weld.get("unit") == "m", "A a unit is the value's, not the label's",
              json.dumps(weld))
        check(all(r.get("label") and r["label"][0].isupper() for r in level.values()), "A every label is in Title Case",
              json.dumps([r.get("label") for r in level.values() if not (r.get("label") or " ")[0].isupper()]))
        audit = mcp("ui.audit", id="level_settings")
        check(audit.get("ok") and not audit.get("escaped"), "A nothing escapes Level Settings", json.dumps(audit.get("escaped"))[:400])

        # ------------------------------------------------------------ B: scene.set
        key = "level.map_version"
        before = level.get(key, {}).get("value", "")
        set_ = mcp("scene.set", world=True, key=key, value="7.5")
        check(set_.get("ok") and (set_.get("set") or {}).get(key) == "7.5", "B scene.set writes a text property", json.dumps(set_))
        again = world().get(key, {})
        check(again.get("value") == "7.5", "B scene.get reads it back", json.dumps(again))
        # the lighting values take a value under the Custom quality only; Escape is built at High
        mcp("scene.set", world=True, key="build.quality", value="Custom")
        bad = mcp("scene.set", world=True, key="build.sky_quality", value=9)
        check(not bad.get("ok") and "0 to 3" in bad.get("error", ""), "B a number out of range is refused with the range",
              json.dumps(bad))
        bad = mcp("scene.set", world=True, key="file", value="x")
        check(not bad.get("ok") and "cannot be set" in bad.get("error", ""), "B what only shows a value is not set", json.dumps(bad))
        mcp("scene.set", world=True, key=key, value=before)
        # an object's rotation goes in degrees, both ways
        lamp = (mcp("scene.place", kind="light", position=[2, 1, 2]).get("object") or {}).get("name", "")
        turned = mcp("scene.set", objects=[lamp], rotation=[0, 90, 0])
        rot = ((turned.get("objects") or [{}])[0]).get("rotation") or []
        check(turned.get("ok") and len(rot) == 3 and abs(rot[1] - 90) < 1e-3, "B an object turns by degrees", json.dumps(turned)[:300])
        mcp("scene.delete", objects=[lamp])

        # ------------------------------------------------------------ C: a spawn's properties and its lists
        roster = mcp("game.lookup", type="spawn_section", query="physic_object", limit=20).get("entries") or []
        sections = [r.get("id", "") if isinstance(r, dict) else str(r) for r in roster]
        section = "physic_object" if "physic_object" in sections else next((s for s in sections if s), "")
        placed = mcp("scene.place", kind="spawn", section=section, position=[1, 0, 1]) if section else {"error": "no physic_object section"}
        name = (placed.get("object") or {}).get("name", "")
        if check(placed.get("ok") and bool(name), "C a spawn to look at", json.dumps(placed)[:300]):
            mcp("scene.select", objects=[name])
            mcp("ui.panel", id="details", action="open")
            shot()
            details = mine(name)
            check(bool(details), "C its properties are listed", "")
            visuals = mcp("scene.choices", object=name, key="props.visual", limit=3)
            check(visuals.get("ok") and visuals.get("list") == "visual" and visuals.get("total", 0) > 0,
                  f"C Visual picks from the models ({visuals.get('total')})", json.dumps(visuals)[:300])
            # a model of the game given: the spawn wears it, and its playback buttons go by their names
            tried = []
            # a model of many bones first: Fixed Bones picks several of them below
            for visual in ("actors\\stalker_neutral\\stalker_neutral_1", "dynamics\\box\\box_wood_01", "physics\\box\\box_1a"):
                r = mcp("scene.set", objects=[name], key="props.visual", value=visual)
                tried.append(r)
                if r.get("ok"):
                    break
            worn = mine(name, ["props.visual", "spawn.animation"])
            check(any(t.get("ok") for t in tried) and worn.get("props.visual", {}).get("value"),
                  "C scene.set gives the spawn a model of the game", json.dumps(tried)[:300])
            anim = worn.get("spawn.animation") or {}
            check(anim.get("choices") == ["First Frame", "Play", "Pause", "Stop", "Last Frame"],
                  "C its playback buttons go by their names", json.dumps(anim))
            pressed = mcp("scene.set", objects=[name], key="spawn.animation", value="Pause")
            check(pressed.get("ok"), "C a playback button is pressed by its name", json.dumps(pressed))
            refused = mcp("scene.set", objects=[name], key="props.visual", value="no\\such\\model")
            check(not refused.get("ok") and refused.get("code") == "not_found", "C a model nobody has is refused",
                  json.dumps(refused)[:300])
            # the spawn's playback over MCP: read, stopped, at its first frame; nothing of the scene changes
            sa = mcp("spawn_animation", object=name)
            check(sa.get("ok") and sa.get("object") == name and sa.get("state") in ("playing", "paused", "stopped"),
                  "C spawn_animation reads the spawn's playback", json.dumps(sa)[:300])
            stopped = mcp("spawn_animation", object=name, action="stop")
            first = mcp("spawn_animation", object=name, action="first")
            check(stopped.get("state") == "stopped" and first.get("state") == "paused" and first.get("time") == 0,
                  "C spawn_animation stops the model and puts it at its first frame",
                  json.dumps(stopped)[:200] + " / " + json.dumps(first)[:200])
            none = mcp("spawn_animation", object="qa_no_such_object")
            check(not none.get("ok"), "C spawn_animation refuses an object that is not there", json.dumps(none)[:200])
            choosers(name)
            mcp("scene.delete", objects=[name])

        # ------------------------------------------------------------ D: Preferences
        tab = mcp("ui.tab", action="open", doc="preferences")
        check(tab.get("ok"), "D Preferences opens as a tab", json.dumps(tab)[:300])
        shot()
        audit = mcp("ui.audit", id="preferences")
        check(audit.get("ok") and not audit.get("escaped"), "D nothing escapes Preferences", json.dumps(audit.get("escaped"))[:400])
        labels = {c.get("label") for c in audit.get("controls", [])}
        check("Search settings" in labels, "D it has its search", json.dumps(sorted(x for x in labels if x))[:400])
        prefs = mcp("app.prefs", action="get", key="bp_cull")
        check(prefs.get("ok") and prefs.get("value") in (True, False), "D bp_cull is a preference of its own", json.dumps(prefs))
        # the snap steps of Preferences ARE the viewport toolbar's
        before = mcp("app.prefs", action="get", key="snap_angle_step")
        mcp("app.prefs", action="set", key="snap_angle_step", value="11.25")
        angle = mcp("viewport_controls").get("angle", {})
        check(abs(float(angle.get("step", 0)) - 11.25) < 1e-3, "D the angle snap of Preferences is the toolbar's",
              json.dumps(angle))
        mcp("app.prefs", action="set", key="snap_angle_step", value=str(before.get("value", "5.625")))
        closed = mcp("ui.tab", action="close", doc="preferences")
        check(closed.get("ok"), "D and closes", json.dumps(closed)[:300])

        # ------------------------------------------------------------ E: shortcuts
        listed = mcp("app.keys")
        key_rows = {s.get("id"): s for s in listed.get("keys", [])}
        every = {i: ", ".join(r.get("keys") or []) for i, r in key_rows.items()}
        check(listed.get("ok") and len(every) > 10, "E app.keys lists the actions", json.dumps(listed)[:300])
        names = {i: (r.get("group"), r.get("title")) for i, r in key_rows.items()}
        check(names.get('COMMAND_ZOOM_EXTENTS."Selected"') == ("View", "Frame Selection") and
              names.get("app.preferences") == ("Edit", "Preferences"), "E actions go by their official names",
              json.dumps({k: names.get(k) for k in ('COMMAND_ZOOM_EXTENTS."Selected"', "app.preferences")}))
        undo = "edit.undo" if "edit.undo" in every else ""
        about = "help.about" if "help.about" in every else ""
        if check(bool(undo) and bool(about), "E Undo and About take shortcuts", json.dumps(sorted(every))[:300]):
            combo = "Ctrl+Shift+Alt+F12"
            first = mcp("app.keys", action="bind", id=about, keys=combo)
            check(first.get("ok") and combo in (first.get("row") or {}).get("keys", []), "E bind gives an action a shortcut",
                  json.dumps(first))
            moved = mcp("app.keys", action="bind", id=undo, keys=combo)
            check(about in (moved.get("moved_from") or []), "E the action that had it lets it go", json.dumps(moved))
            bad = mcp("app.keys", action="bind", id=undo, keys="Ctrl+NoSuchKey")
            check(not bad.get("ok"), "E a key nobody names is refused", json.dumps(bad))
            # as they were
            for action in (undo, about):
                if every[action]:
                    mcp("app.keys", action="bind", id=action, keys=every[action])
                else:
                    mcp("app.keys", action="clear", id=action)
            after = {s.get("id"): ", ".join(s.get("keys") or []) for s in mcp("app.keys").get("keys", [])}
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
    qa_target.copy_project(src, copy, ignore=())
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
            qa_target.remove_tree(work)

    ok = sum(1 for good, _ in results if good)
    print(f"PROPERTIES: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
