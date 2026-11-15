//! The manifest's reference describes the sections that other areas read (`[world]`,
//! `[start_kit]`, `[screens]`, `[screen.<id>]`, `[track.<id>]`): `gz-mod` cannot see their
//! readers, this crate sees every one. A key an area learns without its words in the page, and a
//! key in the page its reader does not know, fail here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use gz_mod::reference::{Check, Reader, SECTIONS, WORLD_RULES, key_doc, section_named};
use gz_mod::{Manifest, ManifestDoc};

const WORLD_SRC: &str = include_str!("../../gz-world/src/model.rs");
const SCREENS_SRC: &str = include_str!("../../gz-screens/src/model.rs");

fn documented(section: &str) -> BTreeSet<String> {
    section_named(section)
        .unwrap_or_else(|| panic!("the page has no section {section}"))
        .keys
        .iter()
        .filter(|k| !k.key.starts_with('<'))
        .map(|k| k.key.to_owned())
        .collect()
}

/// The words after `key == "` anywhere in a source.
fn key_literals(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("key == \"") {
            let tail = rest.get(at + 8..).unwrap_or("");
            let Some(end) = tail.find('"') else { break };
            out.insert(tail.get(..end).unwrap_or("").to_owned());
            rest = tail.get(end..).unwrap_or("");
        }
    }
    out
}

/// The literals that open a match arm (`"x" =>`) on lines indented by exactly one of `indents`,
/// between two markers of a source.
fn arms(src: &str, from: &str, to: &str, indents: &[usize]) -> BTreeSet<String> {
    let start = src.find(from).unwrap_or_else(|| panic!("{from} is not in the source"));
    let end = src.get(start..).and_then(|s| s.find(to)).map_or(src.len(), |e| start + e);
    let mut out = BTreeSet::new();
    for line in src.get(start..end).unwrap_or("").lines() {
        let indent = line.len() - line.trim_start().len();
        let Some(rest) = line.trim_start().strip_prefix('"') else { continue };
        let Some(close) = rest.find('"') else { continue };
        if indents.contains(&indent) && rest.get(close + 1..).is_some_and(|a| a.starts_with(" =>")) {
            out.insert(rest.get(..close).unwrap_or("").to_owned());
        }
    }
    out
}

#[test]
fn the_world_and_the_start_kit_are_the_readers_keys() {
    let scanned = key_literals(WORLD_SRC);
    let mut world: BTreeSet<String> =
        scanned.iter().filter(|k| !matches!(k.as_str(), "money" | "perks")).cloned().collect();
    world.extend(gz_world::RULES.iter().map(|r| (*r).to_owned()));
    assert_eq!(documented("world"), world, "[world]: the page and gz-world's reader differ");
    let kit: BTreeSet<String> =
        scanned.iter().filter(|k| matches!(k.as_str(), "money" | "perks")).cloned().collect();
    assert_eq!(documented("start_kit"), kit, "[start_kit]: the page and gz-world's reader differ");

    // what the page lists is read, not kept as an unknown line
    let lines: Vec<String> =
        documented("world")
            .iter()
            .map(|k| {
                if gz_world::rule_index(k).is_some() { format!("{k} = false") } else { format!("{k} = a, b") }
            })
            .collect();
    let w = gz_world::World::from_lines(lines.iter().map(String::as_str));
    assert!(w.extra.is_empty(), "unknown to the reader: {:?}", w.extra);
    let kit = gz_world::StartKit::from_lines(["money = 5", "perks = false", "bandage = 2"]);
    assert_eq!((kit.money, kit.perks, kit.items.len()), (5, false, 1));
}

#[test]
fn the_screens_are_the_readers_keys() {
    let policy: BTreeSet<String> = SCREENS_SRC
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix('"'))
        .filter_map(|r| r.split_once('"'))
        .filter(|(name, after)| {
            after.starts_with(" =>")
                && ["loading", "menu"].iter().any(|p| {
                    name.strip_prefix(p)
                        .and_then(|s| s.strip_prefix('_'))
                        .is_some_and(|s| matches!(s, "screens" | "music" | "music_end"))
                })
        })
        .map(|(name, _)| name.to_owned())
        .collect();
    assert_eq!(documented("screens"), policy, "[screens]: the page and gz-screens' reader differ");

    // the outer arms of the screen's reader, and the layer's, which nest one level deeper
    let screen = arms(SCREENS_SRC, "fn parse_screen_key", "fn parse_track_key", &[8, 16]);
    assert_eq!(documented("screen.<id>"), screen, "[screen.<id>]: the page and gz-screens' reader differ");
    let track = arms(SCREENS_SRC, "fn parse_track_key", "\n}\n", &[8]);
    assert_eq!(documented("track.<id>"), track, "[track.<id>]: the page and gz-screens' reader differ");
    let playlist = arms(SCREENS_SRC, "fn parse_playlist_key", "\n}\n", &[8]);
    assert_eq!(
        documented("playlist.<id>"),
        playlist,
        "[playlist.<id>]: the page and gz-screens' reader differ"
    );
    let from = SCREENS_SRC.find("fn parse_radio_key").expect("the radio's reader");
    let to = SCREENS_SRC.get(from..).and_then(|s| s.find("\n}\n")).map_or(SCREENS_SRC.len(), |e| from + e);
    let radio = key_literals(SCREENS_SRC.get(from..to).unwrap_or(""));
    assert_eq!(documented("radio"), radio, "[radio]: the page and gz-screens' reader differ");
    for s in ["screens", "screen.night", "track.rain", "playlist.road", "radio"] {
        assert_eq!(section_named(s).map(|d| d.reader), Some(Reader::Screens), "{s}");
    }
}

#[test]
fn every_section_of_the_page_has_a_reader_and_the_example_parses() {
    for s in SECTIONS {
        assert!(!s.summary.is_empty() && s.keys.iter().all(|k| !k.meaning.is_empty()), "[{}]", s.name);
    }
    // the readers of the model's sections are the model: nothing here is kept as an unknown line
    let m = Manifest::parse(&ManifestDoc::parse(gz_mod::reference::EXAMPLE));
    assert_eq!(m.id, "night_watch");
}

/// The words a `Check::Choice` or `Check::Words` of a documented key takes.
fn words(section: &str, key: &str) -> Vec<&'static str> {
    match key_doc(section, key).unwrap_or_else(|| panic!("[{section}] {key} is not in the page")).1.check {
        Check::Choice(w) | Check::Words(w) => w.to_vec(),
        other => panic!("[{section}] {key} is judged by {other:?}"),
    }
}

#[test]
fn the_values_the_check_takes_are_the_values_the_readers_give_a_meaning() {
    // [world]: the five rules, and what the engine reads as no and as yes
    assert_eq!(WORLD_RULES, gz_world::RULES);
    for w in ["false", "0", "no", "off"] {
        assert!(gz_mod::accepts(Check::Flag, w));
        assert!(!gz_world::World::from_lines([format!("mutants = {w}").as_str()]).keep[2], "{w}");
        assert!(!gz_world::StartKit::from_lines([format!("perks = {w}").as_str()]).perks, "{w}");
    }
    for w in ["true", "1", "yes", "on"] {
        assert!(gz_mod::accepts(Check::Flag, w));
        assert!(gz_world::World::from_lines([format!("mutants = {w}").as_str()]).keep[2], "{w}");
    }
    // player_choice names the five rules, and a word that is none is not a choice
    for r in words("world", "player_choice") {
        assert!(gz_world::rule_index(r).is_some(), "{r}");
    }
    assert!(gz_world::World::from_lines(["player_choice = mutant"]).choices().is_empty());

    // [screens]: only and next are the words that change anything
    let screens = |text: &str| gz_screens::Screens::parse(&ManifestDoc::parse(text));
    for (key, at) in [("loading_screens", 0), ("menu_screens", 1)] {
        assert_eq!(words("screens", key), ["mix", "only"]);
        let on = screens(&format!("[screens]\r\n{key} = only\r\n"));
        let off = screens(&format!("[screens]\r\n{key} = mix\r\n"));
        assert!(on.only_screens[at] && !off.only_screens[at], "{key}");
    }
    for (key, at) in [("loading_music_end", 0), ("menu_music_end", 1)] {
        assert_eq!(words("screens", key), ["loop", "next"]);
        assert!(screens(&format!("[screens]\r\n{key} = next\r\n")).next_track[at], "{key}");
    }
    // the surfaces, the hidden parts and the fits: every word is read, any other is not
    for w in words("screen.<id>", "show") {
        let s = screens(&format!("[screen.a]\r\nshow = {w}\r\n[track.a]\r\nplay = {w}\r\n"));
        assert!(s.screens[0].show != 0 && s.tracks[0].play != 0, "{w}");
    }
    let none = screens("[screen.a]\r\nshow = nowhere\r\nhide = nothing\r\n");
    assert_eq!((none.screens[0].show, none.screens[0].hide), (0, 0));
    for w in words("screen.<id>", "hide") {
        assert!(screens(&format!("[screen.a]\r\nhide = {w}\r\n")).screens[0].hide != 0, "{w}");
    }
    for w in words("screen.<id>", "fit") {
        assert_eq!(gz_screens::Fit::parse(w).name(), w);
    }
    // a track plays where a screen shows, and on the radio
    let mut plays = words("screen.<id>", "show");
    plays.push("radio");
    assert_eq!(words("track.<id>", "play"), plays);
    for w in words("track.<id>", "play") {
        assert!(screens(&format!("[track.a]\r\nplay = {w}\r\n")).tracks[0].play != 0, "{w}");
    }
    assert_eq!(screens("[screen.a]\r\nshow = radio\r\n").screens[0].show, 0);
    assert!(screens("[track.a]\r\ncopyrighted = true\r\n").tracks[0].copyrighted);
    assert!(!screens("[track.a]\r\ncopyrighted = false\r\n").tracks[0].copyrighted);
    // the playlist's orders and the radio's policy are the words the reader gives a meaning
    for w in words("playlist.<id>", "order") {
        assert_eq!(gz_screens::Order::parse(w).name(), w);
    }
    assert_eq!(words("radio", "music"), ["mix", "only"]);
    assert!(
        screens("[radio]\r\nmusic = only\r\n").only_radio
            && !screens("[radio]\r\nmusic = mix\r\n").only_radio
    );
}

#[test]
fn the_tags_of_the_check_are_the_tags_of_the_hook() {
    assert_eq!(gz_release::browser::TAG_VOCABULARY, gz_mod::TAG_VOCABULARY);
    for tag in gz_mod::TAG_VOCABULARY {
        assert!(gz_release::browser::parse_tags(tag).is_ok());
    }
    // a tag the hook refuses is one the check names
    assert!(gz_release::browser::parse_tags("banana").is_err() && !gz_mod::accepts(Check::Tags, "banana"));
}

#[test]
fn every_documented_key_of_the_areas_with_a_value_it_takes_passes_the_check() {
    // one line per documented key of the sections an area reads, with the first value it takes
    let mut text = String::from("[module]\r\nid = od\r\ntarget = default\r\n");
    for s in SECTIONS.iter().filter(|s| matches!(s.reader, Reader::World | Reader::Screens)) {
        text.push_str(&format!("[{}]\r\n", s.name.replace("<id>", "x")));
        for k in s.keys.iter().filter(|k| !k.key.starts_with('<')) {
            let value = match k.check {
                Check::Flag | Check::FlagAnyCase => "true".to_owned(),
                Check::Choice(w) | Check::Words(w) => w[0].to_owned(),
                Check::Number | Check::Count | Check::Range(..) => "1".to_owned(),
                Check::Rect => "0 0 100 100".to_owned(),
                _ => "a".to_owned(),
            };
            text.push_str(&format!("{} = {value}\r\n", k.key));
        }
    }
    let found = gz_mod::validate(&ManifestDoc::parse(&text));
    assert!(found.is_empty(), "{found:?}\n{text}");
}
