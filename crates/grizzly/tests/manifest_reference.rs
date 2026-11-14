//! The manifest's reference describes the sections that other areas read (`[world]`,
//! `[start_kit]`, `[screens]`, `[screen.<id>]`, `[track.<id>]`): `gz-mod` cannot see their
//! readers, this crate sees every one. A key an area learns without its words in the page, and a
//! key in the page its reader does not know, fail here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use gz_mod::reference::{Reader, SECTIONS, section_named};
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
    for s in ["screens", "screen.night", "track.rain"] {
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
