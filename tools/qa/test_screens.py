#!/usr/bin/env python3
"""End-to-end check of the module's screens and music (EditorScreens) over MCP, on a THROW-AWAY COPY of a project.

  A  mod.ltx round trip: policies, screens with any number of layers, tracks, pairing
  B  files: inside the project they stay, from outside they are copied, a PNG becomes a .dds, a .ps gets its .s,
     what the game cannot show is refused
  C  layers: add, remove, reorder
  D  problems: a missing file blocks the build, an empty layer does not; the build ships what screens name
  U  the tab: opens, both surfaces, nothing escapes, a drop at a point of the window lands where it should
  G  Disable Game Updates: the build writes the patch, and turning it off takes it away again

    python tools/qa/test_screens.py --project <project> [--shots <dir>]
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zlib

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = qa_target.MCP

results: list[tuple[bool, str]] = []


def check(ok, text, detail="") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + str(detail)) if detail and not ok else ""), flush=True)
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


def start_editor(project: str) -> subprocess.Popen:
    exe = qa_target.exe()
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
    qa_target.guard_project(project)
    proc = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe), startupinfo=startup)
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        try:
            if mcp("mod_manifest", timeout=30).get("ok"):
                mcp("ui.list")
                return proc
        except OSError:
            pass
        if proc.poll() is not None:
            raise RuntimeError("the editor exited during startup")
        time.sleep(1)
    raise RuntimeError("the editor never opened the project")


def png(path: str, w: int = 30, h: int = 18) -> None:
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))
    rows = b"".join(b"\0" + bytes((10, 90, 200)) * w for _ in range(h))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def write(path: str, data: bytes = b"x") -> str:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    return path


def text(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def screens(answer: dict) -> dict:
    return {s["id"]: s for s in answer.get("screens", [])}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--project", required=True)
    parser.add_argument("--shots", default="")
    args = parser.parse_args()

    # every project opened joins the list of the Start page: a throw-away copy must not stay there
    projects_ini = os.path.join(REPO, "_projects.ini")
    projects_before = open(projects_ini, "rb").read() if os.path.exists(projects_ini) else None

    work = tempfile.mkdtemp(prefix="grizzly_screens_")
    project = os.path.join(work, "project")
    outside = os.path.join(work, "outside")
    target = os.path.join(work, "game")
    qa_target.copy_project(args.project, project, ignore=(".git", "releases", "temp", "backup",))
    os.makedirs(outside)
    os.makedirs(target)
    ltx = os.path.join(project, "project.ltx")
    if os.path.exists(ltx):
        lines = [l for l in text(ltx).splitlines() if not l.startswith("last_scene") and not l.startswith("export_target")]
        lines += ["[xms]", "export_target=" + target] if not any(l.strip() == "[xms]" for l in lines) else []
        with open(ltx, "w", encoding="utf-8", newline="\r\n") as f:
            f.write("\n".join(l if not l.strip() == "[xms]" else "[xms]\nexport_target=" + target for l in lines) + "\n")

    editor = start_editor(project)
    try:
        mod = os.path.join(project, "mod.ltx")

        # ---- A: the format
        a = mcp("screens_set", action="policy", surface="loading", screens="only", end="next")
        check(a.get("ok") and a["policy"]["loading"] == {"screens": "only", "music": "mix", "end": "next"}, "A a policy is saved per surface", a)
        check("loading_screens   = only" in text(mod) and "loading_music_end = next" in text(mod), "A the policy reaches mod.ltx")

        pic = write(os.path.join(project, "art", "back.dds"), b"DDS ")
        a = mcp("screens_set", action="screen_add", id="night", show="loading,menu", levels="l01_escape", weight=2, file="art/back.dds", fit="contain", opacity=0.5, rect="10 20 30 40")
        s = screens(a).get("night", {})
        check(s.get("show") == "loading,menu" and s.get("weight") == 2 and s.get("levels") == ["l01_escape"], "A a screen keeps what was asked", s)
        layer = (s.get("layers") or [{}])[0]
        check(layer.get("kind") == "Picture" and layer.get("fit") == "contain" and layer.get("opacity") == 0.5 and layer.get("rect") == [10, 20, 30, 40], "A a layer keeps fit, opacity and place", layer)
        check(layer.get("file") == "art\\back.dds", "A a file of the project stays where it is", layer)
        body = text(mod)
        check("[screen.night]" in body and "layer" in body and "rect" in body and "fit               = contain" in body, "A the screen reaches mod.ltx")

        # ---- B: files
        ogg = write(os.path.join(outside, "rain theme.ogg"), b"OggS")
        a = mcp("screens_set", action="track_add", id="rain", file=ogg, play="loading", volume=0.7)
        t = next((t for t in a.get("tracks", []) if t["id"] == "rain"), {})
        check(t.get("file") == "screens\\rain theme.ogg" and os.path.exists(os.path.join(project, "screens", "rain theme.ogg")), "B a file from outside is copied into the project", t)
        a = mcp("screens_set", action="screen_set", id="night", music="rain")
        check(screens(a)["night"]["music"] == "tracks" and screens(a)["night"]["tracks"] == ["rain"], "A a track is tied to a screen", screens(a)["night"])

        png(os.path.join(outside, "title.png"))
        a = mcp("screens_set", action="layer_add", id="night", file=os.path.join(outside, "title.png"))
        check(a.get("ok") and screens(a)["night"]["layers"][1]["file"] == "screens\\title.dds", "B a PNG becomes a .dds", a)
        dds = os.path.join(project, "screens", "title.dds")
        if check(os.path.exists(dds), "B the .dds is on disk"):
            head = open(dds, "rb").read(20)
            check(head[:4] == b"DDS " and struct.unpack("<I", head[12:16])[0] % 4 == 0 and struct.unpack("<I", head[16:20])[0] % 4 == 0, "B its size is a multiple of four")

        ps = write(os.path.join(outside, "qa_fog.ps"), b"float4 main() : SV_Target { return 1; }")
        a = mcp("screens_set", action="layer_add", id="night", file=ps)
        check(a.get("ok") and screens(a)["night"]["layers"][2]["kind"] == "Shader", "B a shader is a layer", a)
        script = os.path.join(project, "screens", "qa_fog.s")
        check(os.path.exists(script) and '"qa_fog"' in text(script), "B a .ps gets its .s")

        html = write(os.path.join(outside, "page.html"), b"<html><body>x</body></html>")
        a = mcp("screens_set", action="layer_add", id="night", file=html)
        check(screens(a)["night"]["layers"][3]["kind"] == "HTML Page", "B a page is a layer", a)

        exe = write(os.path.join(outside, "bad.exe"), b"MZ")
        a = mcp("screens_set", action="layer_add", id="night", file=exe)
        check(not a.get("ok") and "bad.exe" in a.get("error", ""), "B what the game cannot show is refused", a)

        # ---- C: layers
        a = mcp("screens_set", action="layer_add", id="night")
        count = len(screens(a)["night"]["layers"])
        check(count == 5 and screens(a)["night"]["layers"][4]["kind"] == "Empty", "C a layer starts empty, and there may be as many as wanted", count)
        a = mcp("screens_set", action="layer_move", id="night", index=3, to=0)
        check(screens(a)["night"]["layers"][0]["kind"] == "HTML Page", "C a layer moves", a)
        a = mcp("screens_set", action="layer_remove", id="night", index=0)
        check(len(screens(a)["night"]["layers"]) == 4 and screens(a)["night"]["layers"][0]["kind"] == "Picture", "C a layer goes", a)

        # ---- D: problems and the build
        check(any(p.startswith("W405") for p in a.get("problems", [])), "D an empty layer is a warning", a.get("problems"))
        a = mcp("screens_set", action="screen_add", id="broken", file="art/back.dds")
        a = mcp("screens_set", action="layer_set", id="broken", index=0, file="")
        os.remove(pic)
        built = mcp("mod_export", timeout=600, clean=False, target=target)
        check(not built.get("ok") and "E401" in json.dumps(built), "D a missing file stops the build", built)
        write(pic, b"DDS ")
        mcp("screens_set", action="screen_remove", id="broken")
        mcp("screens_set", action="layer_remove", id="night", index=3)
        built = mcp("mod_export", timeout=600, clean=False, target=target)
        if check(built.get("ok"), "D the build takes them", built):
            module = built.get("module_dir") or built.get("target") or ""
            found = [os.path.join(r, f) for r, _, fs in os.walk(target) for f in fs]
            check(any(p.endswith(os.path.join("screens", "rain theme.ogg")) for p in found), "D the track is shipped", module)
            check(any(p.endswith(os.path.join("screens", "qa_fog.ps")) for p in found) and any(p.endswith(os.path.join("screens", "qa_fog.s")) for p in found), "D both halves of the shader are shipped")

        # ---- U: the tab
        v = mcp("screens_view", open=True, surface="loading", screen="night", layer=0)
        check(v.get("ok") and v.get("open"), "U the tab opens", v)
        shot = mcp("shot.window")
        check(shot.get("active_tab") == "screens", "U it is the active tab", shot.get("active_tab"))
        audit = mcp("ui.audit")
        escaped = [e for e in audit.get("escaped", []) if "screens" in json.dumps(e).lower()]
        check(not escaped, "U nothing escapes it", escaped)
        if args.shots:
            os.makedirs(args.shots, exist_ok=True)
            image = shot.get("png") or shot.get("image") or ""
            if image:
                with open(os.path.join(args.shots, "screens_loading.png"), "wb") as f:
                    f.write(base64.b64decode(image))
        mcp("screens_view", surface="menu")
        check(mcp("shot.window").get("active_tab") == "screens", "U the menu surface draws")
        mcp("screens_view", surface="loading")

        png(os.path.join(outside, "dropped.png"))
        v = mcp("screens_view", simulate_drop=os.path.join(outside, "dropped.png"), x=1000.0, y=500.0)
        mcp("ui.list")
        mcp("shot.window")
        a = mcp("screens_get")
        check("dropped" in screens(a), "U a file dropped on the empty part of the tab becomes a screen", list(screens(a)))

        # ---- G: the last resort
        a = mcp("mod_set_manifest", disable_game_updates=True)
        check(a.get("ok") and "disable_game_updates" in text(mod), "G the option is saved", a)
        built = mcp("mod_export", timeout=600, clean=False, target=target)
        shipped = [os.path.join(r, "mod.ltx") for r, _, fs in os.walk(target) if "mod.ltx" in fs]
        check(bool(shipped) and "disable_game_updates" in text(shipped[0]), "G the module carries the key, and that is all the build does about it", shipped)
        check(not [f for f in os.listdir(os.path.join(project, "patch")) if "game_updates" in f] if os.path.isdir(os.path.join(project, "patch")) else True, "G no file is generated for it")
        check("updates" in json.dumps(built.get("warnings", "")).lower(), "G the build says so", built.get("warnings"))
        mcp("mod_set_manifest", disable_game_updates=False)
        check("disable_game_updates" not in text(mod), "G turning it off takes the key away")
    finally:
        try:
            mcp("app.quit")
        except OSError:
            pass
        try:
            editor.wait(timeout=120)
        except subprocess.TimeoutExpired:
            print("! the editor did not close on its own", flush=True)
        if projects_before is not None:
            with open(projects_ini, "wb") as f:
                f.write(projects_before)
        qa_target.remove_tree(work)

    failed = [t for ok, t in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
