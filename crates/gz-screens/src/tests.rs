use gz_media::sound::Mark;
use gz_mod::{ManifestDoc, edit};

use super::*;
use crate::model::{HIDE_BAR, HIDE_TIP, LOADING, MENU, Order, PAUSE, Playlist, RADIO};

const TEXT: &str = "[module]\r\nid          = od\r\n\r\n\
[screens]\r\nloading_screens   = only\r\nmenu_music_end    = next\r\n\r\n\
[Screen.Night] ; the author's\r\nshow              = loading, pause\r\nlevels            = l01_escape, l02_garbage\r\nweight            = 2.5\r\nmusic             = rain, wind\r\nhide              = bar, tip\r\n\
layer             = screens\\night.dds\r\nfit               = contain\r\nopacity           = 0.5\r\nrect              = 10 20 30 40\r\n\
layer             = screens\\noise.ps\r\ntexture           = screens\\grain.dds\r\nsound             = off\r\n\r\n\
[track.rain]\r\nfile              = screens\\rain.ogg\r\nplay              = menu\r\nvolume            = 0.8\r\n\r\n\
[track.wind]\r\nfile              = screens\\wind.ogg\r\n";

fn apply(doc: &mut ManifestDoc, s: &Screens) {
    for e in s.edits(doc) {
        edit::apply(doc, e).unwrap();
    }
}

#[test]
fn the_model_reads_the_file() {
    let s = Screens::parse(&ManifestDoc::parse(TEXT));
    assert_eq!(s.only_screens, [true, false]);
    assert_eq!(s.next_track, [false, true]);
    let night = s.screen("night").unwrap();
    assert_eq!(night.show, LOADING | PAUSE);
    assert_eq!(night.levels, ["l01_escape", "l02_garbage"]);
    assert_eq!(night.weight, 2.5);
    assert_eq!(
        (night.music.as_str(), night.tracks.clone()),
        ("tracks", vec!["rain".to_owned(), "wind".to_owned()])
    );
    assert_eq!(night.hide, HIDE_BAR | HIDE_TIP);
    let (a, b) = (&night.layers[0], &night.layers[1]);
    assert_eq!(
        (a.file.as_str(), a.fit, a.opacity, a.x, a.y, a.w, a.h),
        ("screens\\night.dds", Fit::Contain, 0.5, 10.0, 20.0, 30.0, 40.0)
    );
    assert_eq!((b.texture.as_str(), b.sound), ("screens\\grain.dds", false));
    assert_eq!(s.track("rain").map(|t| (t.play, t.volume)), Some((MENU, 0.8)));
}

#[test]
fn the_model_round_trips() {
    let mut doc = ManifestDoc::parse(TEXT);
    let s = Screens::parse(&doc);
    apply(&mut doc, &s);
    assert_eq!(Screens::parse(&doc), s);
    // the same model writes the same file: in place, comments kept
    let once = doc.to_string();
    apply(&mut doc, &s);
    assert_eq!(doc.to_string(), once);
    assert!(once.contains("[Screen.Night] ; the author's"));
    assert!(once.contains("rect              = 10 20 30 40\r\n"));
    assert!(once.contains("[module]\r\nid          = od\r\n"));
}

#[test]
fn a_changed_model_writes_its_sections_in_its_order() {
    let mut doc = ManifestDoc::parse(TEXT);
    let mut s = Screens::parse(&doc);
    let id = s.new_screen_id("Night");
    assert_eq!(id, "night_2");
    s.screens.insert(
        0,
        Screen {
            id,
            layers: vec![Layer { file: "screens\\b.gif".into(), ..Layer::default() }],
            ..Screen::default()
        },
    );
    s.remove_track("rain");
    s.only_screens = [false, false];
    s.next_track = [false, false];
    apply(&mut doc, &s);
    let text = doc.to_string();
    assert!(!text.contains("[screens]"));
    assert!(!text.contains("[track.rain]"));
    assert!(text.find("[screen.night_2]").unwrap() < text.find("[screen.night]").unwrap());
    assert!(
        text.contains("show              = loading, menu, pause\r\nlayer             = screens\\b.gif\r\n")
    );
    let back = Screens::parse(&doc);
    assert_eq!(back, s);
    assert_eq!(back.screen("night").map(|d| d.tracks.clone()), Some(vec!["wind".to_owned()]));
}

#[test]
fn the_kind_is_the_extension() {
    assert_eq!(file_kind("a\\b.DDS"), FileKind::Picture);
    assert_eq!(file_kind("x.seq"), FileKind::Picture);
    assert_eq!(file_kind("x.gif"), FileKind::Gif);
    for v in ["x.mp4", "x.webm", "x.wmv", "x.ogm"] {
        assert_eq!(file_kind(v), FileKind::Video);
    }
    assert_eq!(file_kind("x.s"), FileKind::Shader);
    assert_eq!(file_kind("x.ps"), FileKind::Shader);
    assert_eq!(file_kind("x.htm"), FileKind::Page);
    assert_eq!(file_kind("x.ogg"), FileKind::Music);
    for v in ["x.png", "x.jpg", "x.jpeg", "x.tga", "x.bmp"] {
        assert_eq!(file_kind(v), FileKind::Convert);
    }
    assert_eq!(file_kind("x.txt"), FileKind::None);
    assert_eq!(file_kind("folder.png\\x"), FileKind::None);
}

#[test]
fn import_decisions() {
    let root = "D:\\Mods\\OD";
    let inside = plan(root, "screens/night.png").unwrap();
    assert_eq!(inside.source, "D:\\Mods\\OD\\screens\\night.png");
    assert!(!inside.copy && inside.convert && !inside.shader_script);
    let outside = plan(root, "C:\\Pictures\\night.dds").unwrap();
    assert!(outside.copy && !outside.convert);
    let shader = plan(root, "c:/shaders//glow.ps").unwrap();
    assert_eq!(shader.source, "c:\\shaders\\glow.ps");
    assert!(shader.copy && shader.shader_script && shader.kind == FileKind::Shader);
    // the root's own case does not make a file foreign
    assert!(!plan(root, "d:\\mods\\od\\a.ogg").unwrap().copy);
    assert_eq!(
        plan(root, "notes.txt").unwrap_err(),
        "The game shows pictures, GIFs, videos, shaders and HTML pages, and plays .ogg music - not notes.txt."
    );
}

#[test]
fn import_copies_from_outside_and_reuses_a_twin() {
    let base = std::env::temp_dir().join(format!("gz-screens-import-{}", std::process::id()));
    let (root, outside) = (base.join("project"), base.join("outside"));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let ogg = outside.join("song.ogg");
    std::fs::write(&ogg, b"OggS one").unwrap();
    let r = root.to_string_lossy().into_owned();
    assert_eq!(import(&r, &ogg.to_string_lossy()).unwrap(), "screens\\song.ogg");
    // the same file again is the same copy; another with the name gets a number
    assert_eq!(import(&r, &ogg.to_string_lossy()).unwrap(), "screens\\song.ogg");
    let other = base.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("song.ogg"), b"OggS two").unwrap();
    assert_eq!(import(&r, &other.join("song.ogg").to_string_lossy()).unwrap(), "screens\\song_2.ogg");
    // a .ps gets its .s
    std::fs::write(outside.join("glow.ps"), b"float4 main() : COLOR { return 1; }").unwrap();
    assert_eq!(import(&r, &outside.join("glow.ps").to_string_lossy()).unwrap(), "screens\\glow.ps");
    assert!(root.join("screens").join("glow.s").is_file());
    assert!(import(&r, "missing.dds").unwrap_err().starts_with("The file does not exist: "));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn validation_says_what_is_wrong() {
    let base = std::env::temp_dir().join(format!("gz-screens-validate-{}", std::process::id()));
    std::fs::create_dir_all(base.join("screens")).unwrap();
    std::fs::write(base.join("screens").join("a.dds"), b"DDS ").unwrap();
    std::fs::write(base.join("screens").join("glow.s"), b"x").unwrap();
    let s = Screens {
        screens: vec![
            Screen { id: "empty".into(), ..Screen::default() },
            Screen {
                id: "one".into(),
                music: "video".into(),
                levels: vec!["l99_nowhere".into()],
                layers: vec![
                    Layer::default(),
                    Layer { file: "screens\\a.png".into(), ..Layer::default() },
                    Layer { file: "screens\\gone.dds".into(), ..Layer::default() },
                    Layer { file: "screens\\glow.s".into(), ..Layer::default() },
                    Layer { file: "screens\\a.dds".into(), ..Layer::default() },
                ],
                ..Screen::default()
            },
            Screen {
                id: "two".into(),
                music: "tracks".into(),
                tracks: vec!["nope".into()],
                layers: vec![Layer { file: "screens\\a.dds".into(), ..Layer::default() }],
                ..Screen::default()
            },
        ],
        tracks: vec![Track { id: "t".into(), file: "screens\\t.wav".into(), ..Track::default() }],
        ..Screens::default()
    };
    let levels = vec!["l01_escape".to_owned()];
    let out = validate(&base, &s, &|_| false, Some(&levels), &|_| Mark::Free);
    let codes: Vec<&str> = out.iter().map(|l| l.get(..4).unwrap()).collect();
    assert_eq!(codes, ["W406", "W405", "E402", "E401", "E404", "W411", "W407", "E403", "E409", "W410"]);
    assert_eq!(out[0], "W406 screen empty: it has no layers, so it is never shown");
    assert_eq!(out[2], "E402 screen one: layer 2: the game shows no .png file");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_problem_carries_its_section_and_key_and_the_document_puts_it_at_the_line() {
    let text = "[module]\r\nid = od\r\n\r\n[screens]\r\nloading_screens = only\r\n\r\n[screen.night]\r\nshow = loading\r\nlayer = art/a.dds\r\nopacity = 0.5\r\nlayer = art/gone.dds\r\ntexture = art/none.dds\r\nmusic = nope, rain\r\n\r\n[screen.bare]\r\nshow = menu\r\n\r\n[track.t1]\r\nfile = music/t.ogg\r\n\r\n[track.rain]\r\nfile = music/rain.wav\r\nplay = loading\r\n";
    let doc = gz_mod::ManifestDoc::parse(text);
    let s = Screens::parse(&doc);
    let base = std::env::temp_dir().join(format!("gz-screens-spots-{}", std::process::id()));
    std::fs::create_dir_all(base.join("art")).unwrap();
    std::fs::write(base.join("art").join("a.dds"), b"DDS ").unwrap();
    let found = crate::validate::issues(&base, &s, &|_| false, None, &|_| Mark::Free);
    let at = |code: &str, text: &str| {
        let i = found
            .iter()
            .find(|i| i.code == code && i.text.contains(text))
            .unwrap_or_else(|| panic!("{code} {text}: {found:?}"));
        (i.spot.clone(), doc.locate(&i.spot).map(|p| p.line))
    };
    // a layer's file at its `layer` line, its texture at its `texture` line
    assert_eq!(at("E401", "layer 2: the file").1, Some(11));
    assert_eq!(at("E401", "layer 2: the shader's texture").1, Some(12));
    // a screen named by a track that is not there at its `music` line
    assert_eq!(at("E403", "nope").1, Some(13));
    assert_eq!(at("E403", "nope").0, gz_mod::Spot::key("screen.night", "music"));
    // a screen with no layers and a track that plays nowhere at their headers
    assert_eq!(at("W406", "it has no layers").1, Some(15));
    assert_eq!(at("W410", "track t1").1, Some(18));
    // a track that is no .ogg and a track whose file is missing at their `file` line
    assert_eq!(at("E409", "track rain").1, Some(22));
    assert_eq!(at("E401", "track t1").1, Some(19));
    // the same problems as lines are what the answers have always said
    let lines: Vec<String> = found.iter().map(gz_mod::Issue::line).collect();
    assert_eq!(lines, validate(&base, &s, &|_| false, None, &|_| Mark::Free));
    // and as problems of the manifest's check they carry the spot
    let p = found[0].problem();
    assert_eq!(gz_mod::Spot::of(&p), Some(found[0].spot.clone()));
    let _ = std::fs::remove_dir_all(&base);
}

const RADIO_TEXT: &str = "[module]\r\nid = od\r\n\r\n\
[track.night]\r\nfile = music/night.ogg\r\nplay = menu, radio\r\ncopyrighted = true\r\nstreamer = calm\r\n\r\n\
[track.calm]\r\nfile = music/calm.ogg\r\nplay = radio\r\n\r\n\
[playlist.road] ; the author's\r\ntitle = \"Road; Songs\"\r\ntracks = night, calm\r\norder = list\r\n\r\n\
[radio]\r\nmusic = only\r\n";

#[test]
fn the_radio_reads_writes_and_keeps_its_place() {
    let mut doc = ManifestDoc::parse(RADIO_TEXT);
    let s = Screens::parse(&doc);
    let night = s.track("night").unwrap();
    assert_eq!((night.play, night.copyrighted, night.streamer.as_str()), (MENU | RADIO, true, "calm"));
    assert!(!s.track("calm").unwrap().copyrighted);
    let road = s.playlist("road").unwrap();
    assert_eq!(
        (road.title.as_str(), road.tracks.clone(), road.order),
        ("Road; Songs", vec!["night".to_owned(), "calm".to_owned()], Order::List)
    );
    assert!(s.only_radio);
    // the same model writes the same file, the author's comment kept
    apply(&mut doc, &s);
    let once = doc.to_string();
    assert!(once.contains("[playlist.road] ; the author's"), "{once}");
    assert!(once.contains("title             = \"Road; Songs\"\r\n"), "{once}");
    assert!(
        once.contains("play              = menu, radio\r\n") && once.contains("copyrighted       = true\r\n")
    );
    assert_eq!(Screens::parse(&doc), s);
    // a track taken out leaves the playlists and the replacements that named it
    let mut s2 = s.clone();
    s2.remove_track("calm");
    assert_eq!(s2.playlist("road").unwrap().tracks, ["night"]);
    assert!(s2.track("night").unwrap().streamer.is_empty());
    // the mix, a shuffled playlist and a new one write their defaults away
    let mut s3 = s.clone();
    s3.only_radio = false;
    s3.playlists[0].order = Order::Shuffle;
    let id = s3.new_playlist_id("Road");
    assert_eq!(id, "road_2");
    s3.playlists.push(Playlist { id, ..Playlist::default() });
    apply(&mut doc, &s3);
    let text = doc.to_string();
    assert!(
        !text.contains("[radio]") && !text.contains("order") && text.contains("[playlist.road_2]"),
        "{text}"
    );
    assert_eq!(Screens::parse(&doc), s3);
    // radio is a word of a track's play and no surface a screen shows on
    assert_eq!(crate::model::plays("radio, loading"), RADIO | LOADING);
    assert_eq!(crate::model::surfaces("radio"), 0);
}

#[test]
fn the_radio_says_what_is_wrong_at_its_lines() {
    let text = "[module]\r\nid = od\r\n\r\n\
[track.a]\r\nfile = a.ogg\r\nplay = radio\r\nstreamer = b\r\n\r\n\
[track.b]\r\nfile = b.ogg\r\nplay = radio\r\ncopyrighted = true\r\nstreamer = a\r\n\r\n\
[track.c]\r\nfile = c.ogg\r\ncopyrighted = true\r\nstreamer = game\\song\r\n\r\n\
[track.d]\r\nfile = d.ogg\r\ncopyrighted = true\r\nstreamer = nowhere\r\n\r\n\
[playlist.p]\r\ntitle = Дорога ★\r\ntracks = a, gone\r\n\r\n\
[playlist.empty]\r\ntitle = x\r\n";
    let doc = ManifestDoc::parse(text);
    let s = Screens::parse(&doc);
    let base = std::env::temp_dir().join(format!("gz-screens-radio-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    for f in ["a.ogg", "b.ogg", "c.ogg", "d.ogg"] {
        std::fs::write(base.join(f), b"OggS").unwrap();
    }
    let sounds = |n: &str| match n {
        "game\\song" => Mark::Copyrighted,
        "nowhere" => Mark::Missing,
        _ => Mark::Free,
    };
    let found = crate::validate::issues(&base, &s, &|_| false, None, &sounds);
    let at = |code: &str, text: &str| {
        let i = found
            .iter()
            .find(|i| i.code == code && i.text.contains(text))
            .unwrap_or_else(|| panic!("{code} {text}: {found:?}"));
        doc.locate(&i.spot).map(|p| p.line)
    };
    // a replacement on a track that is not copyrighted; a copyrighted one, a track or a sound
    assert_eq!(at("W415", "track a:"), Some(7));
    assert_eq!(at("E416", "track a:"), Some(7));
    assert_eq!(at("E416", "track c:"), Some(18));
    assert_eq!(at("E417", "track d:"), Some(23));
    // a playlist's missing track, its title outside cp1251, an empty one at its header
    assert_eq!(at("E412", "gone"), Some(27));
    assert_eq!(at("W414", "★"), Some(26));
    assert_eq!(at("W413", "playlist empty:"), Some(29));
    // a track the radio plays plays somewhere
    assert!(!found.iter().any(|i| i.code == "W410" && i.text.starts_with("track a:")), "{found:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_playlist_written_by_hand_takes_a_change() {
    let text = "[module]\r\nid = od\r\n\r\n[track.night]\r\nfile              = a.ogg\r\n\r\n[track.calm]\r\nfile              = b.ogg\r\n\r\n[playlist.road_songs]\r\ntitle             = Road Songs\r\ntracks            = night, gone\r\n\r\n[radio]\r\nmusic             = only\r\n";
    let mut doc = ManifestDoc::parse(text);
    let mut s = Screens::parse(&doc);
    s.playlist_mut("road_songs").unwrap().tracks = vec!["night".into(), "calm".into()];
    apply(&mut doc, &s);
    assert!(doc.to_string().contains("tracks            = night, calm"), "{}", doc);
}
