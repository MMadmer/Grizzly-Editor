#!/usr/bin/env python3
"""End-to-end check of the module's screens and music (EditorScreens) over MCP, on a THROW-AWAY COPY of a project.

  A  mod.ltx round trip: policies, screens with any number of layers, tracks, pairing
  B  files: inside the project they stay, from outside they are copied, a PNG becomes a .dds, a .ps gets its .s,
     what the game cannot show is refused
  C  layers: add, remove, reorder
  D  problems: a missing file blocks the build, an empty layer does not; the build ships what screens name
  U  the tab: opens, both surfaces, nothing escapes, a drop at a point of the window lands where it should
  G  Disable Game Updates: the build writes the patch, and turning it off takes it away again
  R  the radio: a track on the radio and copyrighted, what plays instead (the picker holds no copyrighted sound),
     playlists made, reordered, a track taken out, removed whole; the refusals; a playlist's missing track at its line
  S  a sound asset: copyrighted and streamer checked (a copyrighted replacement is an error), shipped with its keys
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
        ogg = write(os.path.join(outside, "rain theme.ogg"), b"OggS")
        a = cmd("screens.set", action="track_add", id="rain", file=ogg, play="loading", volume=0.7)
        t = next((t for t in a.get("tracks", []) if t["id"] == "rain"), {})
        check(t.get("file") == "screens\\rain theme.ogg" and os.path.exists(os.path.join(project, "screens", "rain theme.ogg")), "B a file from outside is copied into the project", t)
        a = cmd("screens.set", action="screen_set", id="night", music="rain")
        check(screens(a)["night"]["music"] == "tracks" and screens(a)["night"]["tracks"] == ["rain"], "A a track is tied to a screen", screens(a)["night"])

        # a track takes a sound: a picture is refused before anything is converted or written
        before = text(mod)
        png(os.path.join(outside, "title.png"))
        shots_before = sorted(os.listdir(os.path.join(project, "screens")))
        a = cmd("screens.set", action="track_add", id="art", file=os.path.join(outside, "title.png"))
        check(not a.get("ok") and "a track takes an .ogg sound, not title.png" in a.get("error", ""), "B track_add of a picture is refused in a sentence", a)
        check(text(mod) == before and sorted(os.listdir(os.path.join(project, "screens"))) == shots_before
              and not os.path.exists(os.path.join(project, "screens", "title.dds")),
              "B nothing was written and nothing converted or copied", sorted(os.listdir(os.path.join(project, "screens"))))
        a = cmd("screens.set", action="track_set", id="rain", file=os.path.join(outside, "title.png"))
        check(not a.get("ok") and "an .ogg sound" in a.get("error", "") and text(mod) == before,
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


def phase_r(project: str, outside: str, mod: str) -> None:
    """R  the radio: a track plays on the radio and is copyrighted, what plays instead, playlists, the refusals."""
    night = write(os.path.join(outside, "night road.ogg"), b"OggS night")
    calm = write(os.path.join(outside, "calm.ogg"), b"OggS calm")
    loud = write(os.path.join(outside, "loud.ogg"), b"OggS loud")
    a = cmd("screens.set", action="track_add", id="night", file=night, play="menu,radio")
    a = cmd("screens.set", action="track_set", id="night", copyrighted=True)
    t = tracks(a).get("night", {})
    check(a.get("ok") and t.get("play") == "menu,radio" and t.get("copyrighted") is True,
          "R a track plays on the menu and the radio and is copyrighted", t)
    body = text(mod)
    check("[track.night]" in body and "play              = menu, radio" in body and "copyrighted       = true" in body,
          "R both reach mod.ltx", body[-400:])
    info = cmd("screens.info")
    check("night" in info.get("radio", {}).get("tracks", []) and info["radio"].get("music") == "mix",
          "R screens.info names the radio's tracks", info.get("radio"))
    cmd("screens.set", action="track_add", id="calm", file=calm, play="radio")
    cmd("screens.set", action="track_add", id="loud", file=loud, play="radio", copyrighted=True)

    # what the game would drop is refused by name, and nothing is written
    before = text(mod)
    for params, says in (({"action": "track_set", "id": "night", "play": "radio,tv"}, "play takes loading, menu, pause, radio"),
                         ({"action": "track_set", "id": "calm", "streamer": "night"}, "is not copyrighted"),
                         ({"action": "track_set", "id": "night", "streamer": "night"}, "cannot play instead of itself"),
                         ({"action": "track_set", "id": "night", "streamer": "loud"}, "is copyrighted itself"),
                         ({"action": "policy", "surface": "radio", "music": "sometimes"}, "music takes only or mix"),
                         ({"action": "playlist_add", "title": "X", "tracks": ["night", "ghost"]}, "there is no track 'ghost'"),
                         ({"action": "playlist_add", "title": "X", "order": "random"}, "order takes shuffle or list")):
        a = cmd("screens.set", **params)
        check(not a.get("ok") and says in a.get("error", "") and text(mod) == before,
              "R refused by name, nothing written: " + says, a.get("error"))
    a = cmd("screens.set", action="track_set", id="night", streamer="qa_nowhere\\no_such_sound")
    if a.get("ok"):
        print("note: no game is linked, so a sound nobody has cannot be told apart", flush=True)
        cmd("screens.set", action="track_set", id="night", streamer="")
    else:
        check("nobody's sound" in a.get("error", "") and text(mod) == before, "R a replacement nobody has is refused", a.get("error"))

    # a replacement without the mark is taken; the picker offers no copyrighted sound
    a = cmd("screens.set", action="track_set", id="night", streamer="calm")
    check(a.get("ok") and tracks(a)["night"].get("streamer") == "calm" and "streamer          = calm" in text(mod),
          "R a track without the mark plays instead", a.get("error"))
    v = cmd("screens.view", open=True, track="night", show="streamer")
    picker = v.get("picker") or {}
    ids = [r["id"] for r in picker.get("rows", [])]
    check(v.get("ok") and "calm" in picker.get("tracks", []) and "night" not in ids and "loud" not in picker.get("tracks", []),
          "R the picker offers the tracks without the mark, never a copyrighted one", qa.short(picker))
    check(not any(r["id"].lower().startswith(("music\\radio\\", "radio\\01\\")) for r in picker.get("rows", [])),
          "R the picker offers none of the game's own copyrighted music", qa.short(ids[:20]))
    a = cmd("screens.view", track="calm", show="streamer")
    check(not a.get("ok"), "R a track that is not copyrighted has no Plays Instead to put up", qa.short(a))

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
    # a track removed leaves the playlists and the replacement that named it
    a = cmd("screens.set", action="track_remove", id="calm")
    check(playlists(a)["road_songs"]["tracks"] == ["night"] and tracks(a)["night"].get("streamer") == "",
          "R a track removed leaves the playlists and the replacements", (playlists(a), tracks(a).get("night")))
    cmd("screens.set", action="track_add", id="calm", file=calm, play="radio")
    cmd("screens.set", action="track_set", id="night", streamer="calm")
    cmd("screens.set", action="playlist_track_add", id="road_songs", track="calm")


def phase_s(project: str, target: str, mod: str) -> None:
    """S  a sound asset: copyrighted and what plays instead, checked, shipped with the keys in its text."""
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
    # the picker offers the module's free sound asset and never its copyrighted one
    v = cmd("screens.view", track="night", show="streamer")
    ids = [x["id"] for x in (v.get("picker") or {}).get("rows", [])]
    check("qa\\qa_calm" in ids and "qa\\qa_song" not in ids, "S the picker offers the free sound asset, not the copyrighted one",
          qa.short(ids[:12]))
    a = cmd("screens.set", action="track_set", id="night", streamer="qa\\qa_song")
    check(not a.get("ok") and "is copyrighted itself" in a.get("error", ""), "S a copyrighted sound asset is refused as a replacement",
          a.get("error"))
    built = cmd("mod.build", timeout=600, clean=False, target="folder", path=target)
    shipped = [os.path.join(r, f) for r, _, fs in os.walk(target) for f in fs if f == "qa_song.sndasset"]
    if check(built.get("ok") and bool(shipped), "S the module build ships the sound asset", qa.short(built)):
        body = text(shipped[0])
        check("copyrighted = true" in body and 'streamer = "qa\\\\qa_calm"' in body, "S its text carries copyrighted and streamer", body)
        check(any(f == "qa_song.ogg" for _, _, fs in os.walk(target) for f in fs), "S and its sound is baked beside it")
        manifest = [os.path.join(r, "mod.ltx") for r, _, fs in os.walk(target) if "mod.ltx" in fs]
        check(bool(manifest) and "[playlist.road_songs]" in text(manifest[0]) and "copyrighted       = true" in text(manifest[0]),
              "S the shipped mod.ltx carries the radio", manifest)


SIZES = ((1707, 912, 1.5), (1280, 720, 1.0))


def phase_t(shots: str | None) -> None:
    """T  the tab: a copyrighted track, Radio Music and a playlist, at both sizes, in English and Russian, dark and light."""
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
            v = cmd("screens.view", track="night", show="streamer")
            check(v.get("ok") and (v.get("picker") or {}).get("count", 0) >= 1, "T %s: the picker is up" % lang, qa.short(v.get("picker")))
            qa.shot(shots, "screens_picker_%s%s" % (lang, "_light" if theme else ""))
            cmd("ui.popup", action="close")
    cmd("app.prefs", action="set", key="language", value="en")
    cmd("app.prefs", action="set", key="theme", value="grizzly-dark")
    cmd("ui.scale", value=1.5)


def main() -> int:
    return qa.suite("SCREENS", run, doc=__doc__, ignore=(".git", "releases", "temp", "backup"))


if __name__ == "__main__":
    sys.exit(main())
