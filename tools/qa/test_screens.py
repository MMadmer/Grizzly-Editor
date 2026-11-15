#!/usr/bin/env python3
"""End-to-end check of the module's screens and music (EditorScreens) over MCP, on a THROW-AWAY COPY of a project.

  A  mod.ltx round trip: policies, screens with any number of layers, tracks, pairing
  B  files: inside the project they stay, from outside they are copied, a PNG becomes a .dds, a .ps gets its .s,
     what the game cannot show is refused
  C  layers: add, remove, reorder
  D  problems: a missing file blocks the build, an empty layer does not; the build ships what screens name
  U  the tab: opens, both surfaces, nothing escapes, a drop at a point of the window lands where it should
  G  Disable Game Updates: the build writes the patch, and turning it off takes it away again
  R  the radio: music brought in is a sound asset and the track says sound; the mark is the sound's - screens.set
     refuses copyrighted and streamer naming the asset, and a track's copyrighted in screens.info follows the asset's
     file; Make It a Sound on a plain-file track (its old mark carried into the asset); a sound nobody has is an
     error and a hand-written copyrighted a warning, each at its line; playlists made, reordered, a track taken out,
     removed whole; the refusals; a playlist's missing track at its line
  S  a sound asset: copyrighted and streamer checked (a copyrighted replacement is an error); the module build ships
     the baked sound with its description holding both keys and a mod.ltx whose track says sound
  T  the tab: a copyrighted track and a playlist audited and pictured at both sizes, English and Russian, dark and light

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


def listing(folder: str) -> list:
    """What a folder holds, sorted; nothing when it is not there."""
    return sorted(os.listdir(folder)) if os.path.isdir(folder) else []


def module_id(mod: str) -> str:
    """`[module] id` of a mod.ltx."""
    section = ""
    for line in text(mod).splitlines():
        line = line.split(";")[0].strip()
        if line.startswith("["):
            section = line.strip("[]").strip().lower()
        elif section == "module" and line.split("=")[0].strip() == "id":
            return line.split("=", 1)[1].strip()
    return ""


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

        # the place is said in per cent by the card, the page and the file: one unit
        check("rect              = 10 20 30 40" in body, "A rect in per cent is written as the numbers it was given", body)
        h = cmd("help", about="screens.set")
        helped = json.dumps(h)
        check("per cent of the screen" in helped and "shares" not in helped and "or none" not in helped,
              "A the card says the unit of rect and the words of fit", helped[:600])
        page = cmd("asset.reference", kind="manifest").get("text", "")
        check("x y width height, per cent of the screen" in page and "0 0 100 100" in page,
              "A the manifest's page says the same unit and the default", page[-300:])
        a = cmd("screens.set", action="layer_set", id="night", index=0, rect=[0, 0, 50, 50])
        check(screens(a)["night"]["layers"][0]["rect"] == [0, 0, 50, 50] and "rect              = 0 0 50 50" in text(mod),
              "A half of the screen is 0 0 50 50 in the answer and in the file", a)
        before = text(mod)
        a = cmd("screens.set", action="layer_set", id="night", index=0, fit="none")
        check(not a.get("ok") and "cover, contain or stretch" in a.get("error", "") and text(mod) == before,
              "A a fit the page does not list is refused and nothing is written", a)
        for params, says in (({"action": "screen_set", "id": "night", "show": "loading,banana"}, "show takes loading, menu, pause"),
                             ({"action": "screen_set", "id": "night", "hide": "everything"}, "hide takes bar, percent, stage, tip"),
                             ({"action": "policy", "surface": "loading", "end": "forever"}, "end takes loop or next")):
            a = cmd("screens.set", **params)
            check(not a.get("ok") and says in a.get("error", "") and text(mod) == before,
                  "A a word the game would drop is refused by name: " + says, a)

        # ---- B: files
        modid = module_id(mod)
        music = write(os.path.join(outside, "rain theme.wav"), wav())
        a = cmd("screens.set", action="track_add", id="rain", file=music, play="loading", volume=0.7)
        t = next((t for t in a.get("tracks", []) if t["id"] == "rain"), {})
        check(t.get("sound") == modid + "\\rain theme" and t.get("plays") == "sound" and t.get("file") == ""
              and t.get("asset") == "sounds/rain theme.sndasset"
              and os.path.exists(os.path.join(project, "sounds", "rain theme.wav"))
              and os.path.exists(os.path.join(project, "sounds", "rain theme.sndasset")),
              "B music from outside is copied in as a sound asset under the module's name, and the track says sound", t)
        check("sound             = %s\\rain theme" % modid in text(mod), "B mod.ltx names the track's sound", text(mod)[-300:])
        ogg = write(os.path.join(outside, "song.ogg"), b"OggS")
        a = cmd("screens.set", action="screen_set", id="night", music="rain")
        check(screens(a)["night"]["music"] == "tracks" and screens(a)["night"]["tracks"] == ["rain"], "A a track is tied to a screen", screens(a)["night"])

        # a track takes a sound: a picture is refused before anything is converted or written
        before = text(mod)
        png(os.path.join(outside, "title.png"))
        shots_before = listing(os.path.join(project, "screens"))
        a = cmd("screens.set", action="track_add", id="art", file=os.path.join(outside, "title.png"))
        check(not a.get("ok") and "a track takes music (.ogg, .wav) or a sound asset, not title.png" in a.get("error", ""), "B track_add of a picture is refused in a sentence", a)
        check(text(mod) == before and listing(os.path.join(project, "screens")) == shots_before
              and not os.path.exists(os.path.join(project, "screens", "title.dds")),
              "B nothing was written and nothing converted or copied", listing(os.path.join(project, "screens")))
        a = cmd("screens.set", action="track_set", id="rain", file=os.path.join(outside, "title.png"))
        check(not a.get("ok") and "a track takes music" in a.get("error", "") and text(mod) == before,
              "B track_set of a picture is refused too", a)
        a = cmd("screens.set", action="layer_add", id="night", file=ogg)
        check(not a.get("ok") and "music goes into a track" in a.get("error", "") and text(mod) == before,
              "B a sound is no layer: refused and pointed at the track", a)

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
            check(any(p.endswith(os.path.join("sounds", modid, "rain theme.ogg")) for p in found), "D the track's sound is baked and shipped", module)
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

        phase_r(project, outside, mod)
        phase_s(project, target, mod)
        phase_t(shots)
        qa.stalls()
    finally:
        editor.close()


def tracks(answer: dict) -> dict:
    return {t["id"]: t for t in answer.get("tracks", [])}


def playlists(answer: dict) -> dict:
    return {p["id"]: p for p in answer.get("playlists", [])}


def wav() -> bytes:
    """A tenth of a second of silence: RIFF, PCM, 16 bits, mono, 22050 Hz."""
    data = b"\0\0" * 2205
    fmt = struct.pack("<HHIIHH", 1, 1, 22050, 44100, 2, 16)
    return b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(data)) + data


def set_lines(path: str, old: str, new: str) -> None:
    """Rewrites a text file the way an agent edits it: one replacement, the rest kept."""
    body = text(path)
    assert old in body, (old, body[-300:])
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(body.replace(old, new, 1))


def problems_at(path: str, code: str) -> list:
    return [p for p in cmd("asset.check", path=path).get("problems", []) if p.get("code") == code]


def phase_r(project: str, outside: str, mod: str) -> None:
    """R  the radio: a track is a sound, the mark is the sound's, Make It a Sound, playlists, the refusals."""
    modid = module_id(mod)
    night = write(os.path.join(outside, "night road.wav"), wav())
    calm = write(os.path.join(outside, "calm.wav"), wav())
    a = cmd("screens.set", action="track_add", id="night", file=night, play="menu,radio")
    t = tracks(a).get("night", {})
    check(a.get("ok") and t.get("play") == "menu,radio" and t.get("sound") == modid + "\\night road"
          and t.get("copyrighted") is False, "R a track plays on the menu and the radio, and names its sound", t)
    body = text(mod)
    check("[track.night]" in body and "play              = menu, radio" in body and "copyrighted" not in body,
          "R it reaches mod.ltx, no mark of its own", body[-400:])
    info = cmd("screens.info")
    check("night" in info.get("radio", {}).get("tracks", []) and info["radio"].get("music") == "mix",
          "R screens.info names the radio's tracks", info.get("radio"))
    a = cmd("screens.set", action="track_add", id="calm", file=calm, play="radio")
    calm_name = tracks(a).get("calm", {}).get("sound", "")

    # the mark is the sound's: refused in a sentence naming the asset to edit, nothing written
    before = text(mod)
    for params in ({"copyrighted": True}, {"streamer": calm_name}):
        a = cmd("screens.set", action="track_set", id="night", **params)
        check(not a.get("ok") and "is no key of a track" in a.get("error", "")
              and "sounds/night road.sndasset" in a.get("error", "") and text(mod) == before,
              "R screens.set %s= is refused, naming the sound asset" % list(params)[0], a.get("error"))
    # ticked in the asset (an agent edits the file), read back from the track
    asset = os.path.join(project, "sounds", "night road.sndasset")
    set_lines(asset, "  snd = 1,\n", "  snd = 1,\n  copyrighted = true,\n  streamer = \"%s\",\n" % calm_name.replace("\\", "\\\\"))
    t = tracks(cmd("screens.info")).get("night", {})
    check(t.get("copyrighted") is True and t.get("streamer") == calm_name and t.get("asset") == "sounds/night road.sndasset",
          "R a track's copyrighted in screens.info follows its sound asset's file", t)

    # what the game would drop is refused by name, and nothing is written
    before = text(mod)
    for params, says in (({"action": "track_set", "id": "night", "play": "radio,tv"}, "play takes loading, menu, pause, radio"),
                         ({"action": "policy", "surface": "radio", "music": "sometimes"}, "music takes only or mix"),
                         ({"action": "playlist_add", "title": "X", "tracks": ["night", "ghost"]}, "there is no track 'ghost'"),
                         ({"action": "playlist_add", "title": "X", "order": "random"}, "order takes shuffle or list")):
        a = cmd("screens.set", **params)
        check(not a.get("ok") and says in a.get("error", "") and text(mod) == before,
              "R refused by name, nothing written: " + says, a.get("error"))
    a = cmd("screens.set", action="track_set", id="night", sound="qa_nowhere\\no_such_sound")
    linked = not a.get("ok")
    if linked:
        check("nobody has the sound" in a.get("error", "") and text(mod) == before, "R a sound nobody has is refused", a.get("error"))
    else:
        print("note: no game is linked, so a sound nobody has cannot be told apart", flush=True)
        cmd("screens.set", action="track_set", id="night", sound=modid + "\\night road")

    # written by hand: a track that is a plain file with the mark it had, one whose sound nobody has
    plain = write(os.path.join(project, "music", "plain song.wav"), wav())
    with open(mod, "a", encoding="utf-8", newline="") as f:
        f.write("\r\n[track.plain]\r\nfile              = music\\plain song.wav\r\nplay              = radio\r\n"
                "copyrighted       = true\r\nstreamer          = calm\r\n"
                "\r\n[track.ghost]\r\nsound             = qa_nowhere\\no_such_sound\r\nplay              = radio\r\n")
    lines = text(mod).splitlines()
    marked = problems_at("mod.ltx", "W417")
    check(any(p.get("line") == lines.index("copyrighted       = true") + 1 for p in marked)
          and all(p.get("severity") == "warning" for p in marked) and "action=track_sound id=plain" in json.dumps(marked),
          "R a hand-written copyrighted on a track is a warning at its line that says where the mark goes", qa.short(marked))
    ghost = problems_at("mod.ltx", "E416")
    if linked:
        check(bool(ghost) and ghost[0].get("line") == lines.index("sound             = qa_nowhere\\no_such_sound") + 1
              and ghost[0].get("severity") == "error", "R a sound nobody has is the check's error at its line", qa.short(ghost))
    seen = qa.until(lambda: "plain" in tracks(cmd("screens.info")), 10)
    check(seen, "R the editor reads the tracks written by hand")
    # Make It a Sound: the asset over the file, the track says sound, the old mark carried into the asset
    a = cmd("screens.set", action="track_sound", id="plain")
    t = tracks(a).get("plain", {})
    made = os.path.join(project, "music", "plain song.sndasset")
    check(a.get("ok") and t.get("sound") == modid + "\\plain song" and t.get("file") == "" and os.path.isfile(made),
          "R Make It a Sound writes the asset over the file and the track names its sound", t)
    if os.path.isfile(made):
        body = text(made)
        check("copyrighted = true" in body and ('streamer = "%s"' % calm_name.replace("\\", "\\\\")) in body,
              "R the mark the track had is the sound's now (the replacement a track named is its sound)", body)
    check(t.get("copyrighted") is True and "copyrighted" not in text(mod).split("[track.plain]")[1].split("[")[0],
          "R the track carries no mark any more and screens.info reads it from the sound", t)
    cmd("screens.set", action="track_remove", id="plain")
    cmd("screens.set", action="track_remove", id="ghost")
    for f in (made, plain):
        if os.path.isfile(f):
            os.remove(f)

    # playlists: made, reordered, a track taken out, removed whole
    a = cmd("screens.set", action="playlist_add", title="Road Songs", tracks=["night", "calm"], order="list")
    p = playlists(a).get("road_songs", {})
    check(a.get("ok") and p.get("tracks") == ["night", "calm"] and p.get("order") == "list" and p.get("title") == "Road Songs",
          "R a playlist is made of its title", p)
    body = text(mod)
    check("[playlist.road_songs]" in body and "title             = Road Songs" in body
          and "tracks            = night, calm" in body and "order             = list" in body, "R it reaches mod.ltx", body[-300:])
    a = cmd("screens.set", action="playlist_track_move", id="road_songs", track="calm", to=0)
    check(playlists(a)["road_songs"]["tracks"] == ["calm", "night"] and "tracks            = calm, night" in text(mod),
          "R a track moves in the playlist", playlists(a)["road_songs"])
    a = cmd("screens.set", action="playlist_track_remove", id="road_songs", track="night")
    check(playlists(a)["road_songs"]["tracks"] == ["calm"] and "night" in tracks(a), "R a track leaves the playlist and stays in the music",
          playlists(a)["road_songs"])
    a = cmd("screens.set", action="playlist_set", id="road_songs", tracks=["night", "calm"], order="shuffle")
    check(playlists(a)["road_songs"]["tracks"] == ["night", "calm"] and "order             = list" not in text(mod),
          "R the whole list is set, shuffle is the default and not written", playlists(a)["road_songs"])
    a = cmd("screens.set", action="policy", surface="radio", music="only")
    check(a.get("radio", {}).get("music") == "only" and "[radio]" in text(mod) and "music             = only" in text(mod),
          "R Only Mine for the radio reaches mod.ltx", a.get("radio"))
    a = cmd("screens.set", action="playlist_add", title="Quiet")
    check(any(x.startswith("W413 playlist quiet:") for x in a.get("problems", [])), "R an empty playlist is a warning", a.get("problems"))
    a = cmd("screens.set", action="playlist_remove", id="quiet")
    check(a.get("ok") and "quiet" not in playlists(a) and "[playlist.quiet]" not in text(mod), "R a playlist is removed whole")
    lines = text(mod).replace("tracks            = night, calm", "tracks            = night, gone")
    with open(mod, "w", encoding="utf-8", newline="") as f:
        f.write(lines)
    r = cmd("asset.check", path="mod.ltx")
    gone = [x for x in r.get("problems", []) if x.get("code") == "E412"]
    check(bool(gone) and gone[0].get("line") == text(mod).splitlines().index("tracks            = night, gone") + 1,
          "R a playlist naming a track that is not there is an error at its line", qa.short(r.get("problems")))
    # the editor follows the file written by hand, and the next change goes over it
    seen = qa.until(lambda: "gone" in playlists(cmd("screens.info")).get("road_songs", {}).get("tracks", []), 10)
    check(seen, "R the editor reads mod.ltx written by hand", qa.short(playlists(cmd("screens.info"))))
    a = cmd("screens.set", action="playlist_set", id="road_songs", tracks=["night", "calm"])
    check(a.get("ok") and playlists(a)["road_songs"]["tracks"] == ["night", "calm"] and "tracks            = night, calm" in text(mod),
          "R a change after an edit by hand writes over the file as it is now", qa.short(a))
    # a track removed leaves the playlists
    a = cmd("screens.set", action="track_remove", id="calm")
    check(playlists(a)["road_songs"]["tracks"] == ["night"], "R a track removed leaves the playlists", playlists(a))
    cmd("screens.set", action="track_add", id="calm", sound=calm_name, play="radio")
    cmd("screens.set", action="playlist_track_add", id="road_songs", track="calm")


def phase_s(project: str, target: str, mod: str) -> None:
    """S  a sound asset: copyrighted and what plays instead, checked; a track plays it; shipped with its keys."""
    sounds = os.path.join(project, "sounds", "qa")
    for n in ("qa_song", "qa_calm", "qa_bad"):
        write(os.path.join(sounds, n + ".wav"), wav())
    write(os.path.join(sounds, "qa_song.sndasset"),
          b'return {\n  snd = 1,\n  name = "qa\\\\qa_song",\n  copyrighted = true,\n  streamer = "qa\\\\qa_calm",\n}\n')
    write(os.path.join(sounds, "qa_calm.sndasset"), b'return {\n  snd = 1,\n  name = "qa\\\\qa_calm",\n}\n')
    write(os.path.join(sounds, "qa_bad.sndasset"),
          b'return {\n  snd = 1,\n  name = "qa\\\\qa_bad",\n  copyrighted = true,\n  streamer = "qa\\\\qa_song",\n}\n')
    r = cmd("asset.check", path="sounds/qa/qa_song.sndasset")
    check(r.get("ok") and not [p for p in r.get("problems", []) if p.get("severity") in ("error", "warning")],
          "S a copyrighted sound with a free replacement passes its check", qa.short(r.get("problems")))
    r = cmd("asset.check", path="sounds/qa/qa_bad.sndasset")
    bad = [p for p in r.get("problems", []) if p.get("code") == "M115"]
    check(bool(bad) and bad[0].get("severity") == "error" and bad[0].get("line") == 5,
          "S a copyrighted replacement is an error at its streamer line", qa.short(r.get("problems")))
    write(os.path.join(sounds, "qa_bad.sndasset"), b'return {\n  snd = 1,\n  name = "qa\\\\qa_bad",\n  streamer = "qa\\\\qa_calm",\n}\n')
    r = cmd("asset.check", path="sounds/qa/qa_bad.sndasset")
    check([p.get("severity") for p in r.get("problems", []) if p.get("code") == "M114"] == ["warning"],
          "S a replacement on a sound that is not copyrighted is a warning", qa.short(r.get("problems")))
    os.remove(os.path.join(sounds, "qa_bad.sndasset"))
    os.remove(os.path.join(sounds, "qa_bad.wav"))
    # a track plays the sound by its name; what the sound says is what screens.info says
    a = cmd("screens.set", action="track_add", id="song", sound="qa\\qa_song", play="radio")
    t = tracks(a).get("song", {})
    check(a.get("ok") and t.get("copyrighted") is True and t.get("streamer") == "qa\\qa_calm"
          and t.get("asset") == "sounds/qa/qa_song.sndasset", "S a track that names the sound reads its mark from it", t)
    cmd("screens.set", action="playlist_track_add", id="road_songs", track="song")
    built = cmd("mod.build", timeout=600, clean=False, target="folder", path=target)
    shipped = [os.path.join(r, f) for r, _, fs in os.walk(target) for f in fs if f == "qa_song.sndasset"]
    if check(built.get("ok") and bool(shipped), "S the module build ships the sound asset", qa.short(built)):
        body = text(shipped[0])
        check("copyrighted = true" in body and 'streamer = "qa\\\\qa_calm"' in body, "S its text carries copyrighted and streamer", body)
        check(any(f == "qa_song.ogg" for _, _, fs in os.walk(target) for f in fs), "S and its sound is baked beside it")
        manifest = [os.path.join(r, "mod.ltx") for r, _, fs in os.walk(target) if "mod.ltx" in fs]
        shipped_mod = text(manifest[0]) if manifest else ""
        check("[playlist.road_songs]" in shipped_mod and "sound             = qa\\qa_song" in shipped_mod
              and "copyrighted" not in shipped_mod and "streamer" not in shipped_mod,
              "S the shipped mod.ltx's track says sound, and carries no mark", manifest)


SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))


def phase_t(shots: str | None) -> None:
    """T  the tab: a track whose sound is copyrighted (its mark, Open Sound), a playlist, at both sizes, in English and
    Russian, dark and light; Open Sound opens the Sound tab on the track's asset."""
    seen: dict[str, set] = {}
    views = (("track", {"track": "night"}), ("playlist", {"playlist": "road_songs"}))
    for theme in ("", "grizzly-light"):
        if theme:
            check(cmd("app.prefs", action="set", key="theme", value=theme).get("ok"), "T the window takes the light theme")
        for lang in ("en", "ru"):
            cmd("app.prefs", action="set", key="language", value=lang)
            for width, height, scale in SIZES:
                cmd("ui.scale", value=scale)
                cmd("app.window", action="resize", width=width, height=height)
                for name, sel in views:
                    tag = "T %s %s %dx%d%s" % (name, lang, width, height, " light" if theme else "")
                    cmd("screens.view", open=True, **sel)
                    qa.until(lambda: not cmd("ui.audit", id="screens").get("loading"), 10)
                    a = cmd("ui.audit", id="screens")
                    cut = a.get("truncated") or []
                    vanished = [t for t in cut if not t.get("shown") and (t.get("rect") or [0, 0, 0])[2] > 0]
                    check(a.get("ok") and not vanished and not a.get("escaped"), "%s: nothing is cut to nothing, nothing escapes" % tag,
                          qa.short({"vanished": vanished[:3], "escaped": (a.get("escaped") or [])[:3]}))
                    key = "%s %dx%d" % (name, width, height)
                    if lang == "en":
                        seen.setdefault(key, set()).update(str(t.get("label")) for t in cut)
                        if cut:
                            print("note: %s cut with its end shown: %s" % (tag, qa.short([t.get("label") for t in cut][:6])), flush=True)
                    else:
                        missing = [m.get("label") for m in a.get("untranslated") or []]
                        check(not missing, "%s: every text of the editor's own reads in Russian" % tag, qa.short(missing[:12]))
                        longer = [t.get("label") for t in cut if t.get("en") and str(t["en"]) not in seen.get(key, set())]
                        check(not longer, "%s: no Russian text is cut where its English fits" % tag, qa.short(longer[:6]))
                    qa.shot(shots, "screens_%s_%s_%d%s" % (name, lang, width, "_light" if theme else ""))
    cmd("app.prefs", action="set", key="language", value="en")
    cmd("app.prefs", action="set", key="theme", value="grizzly-dark")
    cmd("ui.scale", value=1.5)
    cmd("app.window", action="resize", width=1707, height=912)
    # the track's sound opens in the Sound tab, where its mark is said
    a = cmd("preview.sound", open=True, path="sounds/night road.sndasset")
    check(a.get("ok") and a.get("active") and a.get("copyrighted") is True, "T the track's sound opens in the Sound tab", qa.short(a))
    qa.shot(shots, "screens_open_sound")


def main() -> int:
    return qa.suite("SCREENS", run, doc=__doc__, ignore=(".git", "releases", "temp", "backup"))


if __name__ == "__main__":
    sys.exit(main())
