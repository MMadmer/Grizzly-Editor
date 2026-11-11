use gz_mod::{ManifestDoc, edit};

use super::*;
use crate::model::{HIDE_BAR, HIDE_TIP, LOADING, MENU, PAUSE};

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
    let out = validate(&base, &s, &|_| false, Some(&levels));
    let codes: Vec<&str> = out.iter().map(|l| l.get(..4).unwrap()).collect();
    assert_eq!(codes, ["W406", "W405", "E402", "E401", "E404", "W411", "W407", "E403", "E409", "W410"]);
    assert_eq!(out[0], "W406 screen empty: it has no layers, so it is never shown");
    assert_eq!(out[2], "E402 screen one: layer 2: the game shows no .png file");
    let _ = std::fs::remove_dir_all(&base);
}
