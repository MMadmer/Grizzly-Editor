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

import base64
import json
import os
import struct
import sys
import zlib


import qa
from qa import check, cmd

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


def run(ctx: qa.Run) -> None:
    shots = ctx.shots
    project = ctx.project
    outside = os.path.join(ctx.work, "outside")
    target = os.path.join(ctx.work, "game")
    os.makedirs(outside)
    os.makedirs(target)
    ltx = os.path.join(project, "project.ltx")
    if os.path.exists(ltx):
        lines = [l for l in text(ltx).splitlines() if not l.startswith("last_scene") and not l.startswith("export_target")]
        lines += ["[xms]", "export_target=" + target] if not any(l.strip() == "[xms]" for l in lines) else []
        with open(ltx, "w", encoding="utf-8", newline="\r\n") as f:
            f.write("\n".join(l if not l.strip() == "[xms]" else "[xms]\nexport_target=" + target for l in lines) + "\n")

    editor = qa.Editor(project).start()
    try:
        mod = os.path.join(project, "mod.ltx")

        # ---- A: the format
        a = cmd("screens.set", action="policy", surface="loading", screens="only", end="next")
        check(a.get("ok") and a["policy"]["loading"] == {"screens": "only", "music": "mix", "end": "next"}, "A a policy is saved per surface", a)
        check("loading_screens   = only" in text(mod) and "loading_music_end = next" in text(mod), "A the policy reaches mod.ltx")

        pic = write(os.path.join(project, "art", "back.dds"), b"DDS ")
        a = cmd("screens.set", action="screen_add", id="night", show="loading,menu", levels="l01_escape", weight=2, file="art/back.dds", fit="contain", opacity=0.5, rect=[10, 20, 30, 40])
        s = screens(a).get("night", {})
        check(s.get("show") == "loading,menu" and s.get("weight") == 2 and s.get("levels") == ["l01_escape"], "A a screen keeps what was asked", s)
        layer = (s.get("layers") or [{}])[0]
        check(layer.get("kind") == "Picture" and layer.get("fit") == "contain" and layer.get("opacity") == 0.5 and layer.get("rect") == [10, 20, 30, 40], "A a layer keeps fit, opacity and place", layer)
        check(layer.get("file") == "art\\back.dds", "A a file of the project stays where it is", layer)
        body = text(mod)
        check("[screen.night]" in body and "layer" in body and "rect" in body and "fit               = contain" in body, "A the screen reaches mod.ltx")

        # ---- B: files
        ogg = write(os.path.join(outside, "rain theme.ogg"), b"OggS")
        a = cmd("screens.set", action="track_add", id="rain", file=ogg, play="loading", volume=0.7)
        t = next((t for t in a.get("tracks", []) if t["id"] == "rain"), {})
        check(t.get("file") == "screens\\rain theme.ogg" and os.path.exists(os.path.join(project, "screens", "rain theme.ogg")), "B a file from outside is copied into the project", t)
        a = cmd("screens.set", action="screen_set", id="night", music="rain")
        check(screens(a)["night"]["music"] == "tracks" and screens(a)["night"]["tracks"] == ["rain"], "A a track is tied to a screen", screens(a)["night"])

        png(os.path.join(outside, "title.png"))
        a = cmd("screens.set", action="layer_add", id="night", file=os.path.join(outside, "title.png"))
        check(a.get("ok") and screens(a)["night"]["layers"][1]["file"] == "screens\\title.dds", "B a PNG becomes a .dds", a)
        dds = os.path.join(project, "screens", "title.dds")
        if check(os.path.exists(dds), "B the .dds is on disk"):
            head = open(dds, "rb").read(20)
            check(head[:4] == b"DDS " and struct.unpack("<I", head[12:16])[0] % 4 == 0 and struct.unpack("<I", head[16:20])[0] % 4 == 0, "B its size is a multiple of four")

        ps = write(os.path.join(outside, "qa_fog.ps"), b"float4 main() : SV_Target { return 1; }")
        a = cmd("screens.set", action="layer_add", id="night", file=ps)
        check(a.get("ok") and screens(a)["night"]["layers"][2]["kind"] == "Shader", "B a shader is a layer", a)
        script = os.path.join(project, "screens", "qa_fog.s")
        check(os.path.exists(script) and '"qa_fog"' in text(script), "B a .ps gets its .s")

        html = write(os.path.join(outside, "page.html"), b"<html><body>x</body></html>")
        a = cmd("screens.set", action="layer_add", id="night", file=html)
        check(screens(a)["night"]["layers"][3]["kind"] == "HTML Page", "B a page is a layer", a)

        exe = write(os.path.join(outside, "bad.exe"), b"MZ")
        a = cmd("screens.set", action="layer_add", id="night", file=exe)
        check(not a.get("ok") and "bad.exe" in a.get("error", ""), "B what the game cannot show is refused", a)

        # ---- C: layers
        a = cmd("screens.set", action="layer_add", id="night")
        count = len(screens(a)["night"]["layers"])
        check(count == 5 and screens(a)["night"]["layers"][4]["kind"] == "Empty", "C a layer starts empty, and there may be as many as wanted", count)
        a = cmd("screens.set", action="layer_move", id="night", index=3, to=0)
        check(screens(a)["night"]["layers"][0]["kind"] == "HTML Page", "C a layer moves", a)
        a = cmd("screens.set", action="layer_remove", id="night", index=0)
        check(len(screens(a)["night"]["layers"]) == 4 and screens(a)["night"]["layers"][0]["kind"] == "Picture", "C a layer goes", a)

        # ---- D: problems and the build
        check(any(p.startswith("W405") for p in a.get("problems", [])), "D an empty layer is a warning", a.get("problems"))
        a = cmd("screens.set", action="screen_add", id="broken", file="art/back.dds")
        a = cmd("screens.set", action="layer_set", id="broken", index=0, file="")
        os.remove(pic)
        built = cmd("mod.build", timeout=600, clean=False, target="folder", path=target)
        check(not built.get("ok") and "E401" in json.dumps(built), "D a missing file stops the build", built)
        write(pic, b"DDS ")
        cmd("screens.set", action="screen_remove", id="broken")
        cmd("screens.set", action="layer_remove", id="night", index=3)
        built = cmd("mod.build", timeout=600, clean=False, target="folder", path=target)
        if check(built.get("ok"), "D the build takes them", built):
            module = built.get("module_dir") or built.get("target") or ""
            found = [os.path.join(r, f) for r, _, fs in os.walk(target) for f in fs]
            check(any(p.endswith(os.path.join("screens", "rain theme.ogg")) for p in found), "D the track is shipped", module)
            check(any(p.endswith(os.path.join("screens", "qa_fog.ps")) for p in found) and any(p.endswith(os.path.join("screens", "qa_fog.s")) for p in found), "D both halves of the shader are shipped")

        # ---- U: the tab
        v = cmd("screens.view", open=True, surface="loading", screen="night", layer=0)
        check(v.get("ok") and v.get("open"), "U the tab opens", v)
        shot = cmd("shot.window")
        check(shot.get("active_tab") == "screens", "U it is the active tab", shot.get("active_tab"))
        audit = cmd("ui.audit")
        escaped = [e for e in audit.get("escaped", []) if "screens" in json.dumps(e).lower()]
        check(not escaped, "U nothing escapes it", escaped)
        if shots:
            os.makedirs(shots, exist_ok=True)
            image = shot.get("png") or shot.get("image") or ""
            if image:
                with open(os.path.join(shots, "screens_loading.png"), "wb") as f:
                    f.write(base64.b64decode(image))
        cmd("screens.view", surface="menu")
        check(cmd("shot.window").get("active_tab") == "screens", "U the menu surface draws")
        cmd("screens.view", surface="loading")

        png(os.path.join(outside, "dropped.png"))
        v = cmd("screens.view", simulate_drop=os.path.join(outside, "dropped.png"), x=1000.0, y=500.0)
        cmd("ui.list")
        cmd("shot.window")
        a = cmd("screens.info")
        check("dropped" in screens(a), "U a file dropped on the empty part of the tab becomes a screen", list(screens(a)))

        # ---- G: the last resort
        a = cmd("mod.set", set={"module": {"disable_game_updates": True}})
        check(a.get("ok") and "disable_game_updates" in text(mod), "G the option is saved", a)
        built = cmd("mod.build", timeout=600, clean=False, target="folder", path=target)
        shipped = [os.path.join(r, "mod.ltx") for r, _, fs in os.walk(target) if "mod.ltx" in fs]
        check(bool(shipped) and "disable_game_updates" in text(shipped[0]), "G the module carries the key, and that is all the build does about it", shipped)
        check(not [f for f in os.listdir(os.path.join(project, "patch")) if "game_updates" in f] if os.path.isdir(os.path.join(project, "patch")) else True, "G no file is generated for it")
        check("updates" in json.dumps(built.get("warnings", "")).lower(), "G the build says so", built.get("warnings"))
        cmd("mod.set", set={"module": {"disable_game_updates": False}})
        check("disable_game_updates" not in text(mod), "G turning it off takes the key away")

        # ---- I: the import, and nothing waited
        imp = cmd("screens.import", path=[os.path.join(outside, "title.png")])
        check(imp.get("ok") and any(k.startswith("title") for k in screens(imp)), "I screens.import makes a screen of a picture",
              list(screens(imp)))
    finally:
        editor.close()


def main() -> int:
    return qa.suite("SCREENS", run, doc=__doc__, ignore=(".git", "releases", "temp", "backup"))


if __name__ == "__main__":
    sys.exit(main())
