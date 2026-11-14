#![allow(clippy::unwrap_used)]

use glam::Vec3;
use gz_chunk::Writer;
use gz_editor::Args;
use gz_mod::{Edit, ManifestDoc};
use serde_json::{Value, json};

use crate::cmd::{set_kit, set_world};
use crate::game::{GameTransition, GameWorld, is_level_changer, walk_destination};
use crate::model::{KitItem, StartKit, World, base_kit};
use crate::starts::{Start, read_starts};
use crate::validate::validate;

fn doc(text: &str) -> ManifestDoc {
    ManifestDoc::parse(text)
}

fn applied(text: &str, edits: Vec<Edit>) -> String {
    let mut d = doc(text);
    for e in edits {
        gz_mod::edit::apply(&mut d, e).unwrap();
    }
    d.to_string()
}

fn args(v: Value) -> Args {
    match v {
        Value::Object(m) => m,
        _ => Args::new(),
    }
}

const MANIFEST: &str = "[module]\r\nid = od\r\n\r\n[world]\r\n; the author's note\r\noriginal_npcs = no\r\nMUTANTS = 0\r\nplayer_choice = mutants, nonsense, npc_quests\r\nlevels = l01_escape , l02_garbage\r\nlocked = esc_to_gar\r\nweather = rain\r\n\r\n[start_kit]\r\nmoney = 500\r\nperks = off\r\nwpn_knife\r\nbandage = 3\r\n";

// ---- [world] and [start_kit] ---------------------------------------------------------------

#[test]
fn world_reads_the_way_the_engine_does() {
    let w = World::parse(&doc(MANIFEST));
    assert_eq!(w.keep, [false, true, false, true, true]);
    assert_eq!(w.player_choice, [false, false, true, true, false]);
    assert_eq!(w.levels, ["l01_escape", "l02_garbage"]);
    assert_eq!(w.locked, ["esc_to_gar"]);
    assert_eq!(w.extra, ["weather = rain"]);
    assert!(!w.is_empty());
    // a rule changes only on an explicit no
    let garbled = World::from_lines(["original_npcs = nope", "mutants"]);
    assert_eq!(garbled.keep, [true; 5]);
    assert!(World::parse(&doc("[module]\nid = x\n")).is_empty());
}

#[test]
fn world_round_trips_through_the_manifest() {
    let w = World::parse(&doc(MANIFEST));
    let text = applied(MANIFEST, vec![w.edit()]);
    assert!(text.contains(
        "[world]\r\n; the author's note\r\noriginal_npcs   = false\r\nrandom_stalkers = true\r\nmutants         = false\r\nnpc_quests      = true\r\nstory_quests    = true\r\nplayer_choice   = mutants, npc_quests\r\nlevels          = l01_escape, l02_garbage\r\nlocked          = esc_to_gar\r\nweather = rain\r\n"
    ));
    assert_eq!(World::parse(&doc(&text)), w);
    // the stock world takes the section out; the comment of the author stays
    let stock = applied(&text, vec![World::default().edit()]);
    assert_eq!(World::parse(&doc(&stock)), World::default());
    assert!(!stock.contains("original_npcs"));
    let bare = "[module]\r\nid = od\r\n\r\n[world]\r\nmutants = false\r\n";
    assert_eq!(applied(bare, vec![World::default().edit()]), "[module]\r\nid = od\r\n");
}

#[test]
fn start_kit_round_trips_through_the_manifest() {
    let k = StartKit::parse(&doc(MANIFEST));
    assert!(k.declared);
    assert_eq!(k.money, 500);
    assert!(!k.perks);
    assert_eq!(
        k.items,
        [KitItem { section: "wpn_knife".into(), count: 1 }, KitItem { section: "bandage".into(), count: 3 }]
    );
    let d = doc(MANIFEST);
    let text = applied(MANIFEST, vec![k.edit(&d).unwrap()]);
    assert!(text.ends_with("[start_kit]\r\nmoney           = 500\r\nperks           = false\r\nwpn_knife       = 1\r\nbandage         = 3\r\n"));
    assert_eq!(StartKit::parse(&doc(&text)), k);

    // an empty kit is still a declared one
    let empty = StartKit { declared: true, ..StartKit::default() };
    let text = applied(MANIFEST, vec![empty.edit(&d).unwrap()]);
    let back = StartKit::parse(&doc(&text));
    assert!(back.declared && back.items.is_empty() && back.perks && back.money == -1);
    // and an undeclared one takes the section out
    let gone = applied(MANIFEST, vec![StartKit::default().edit(&d).unwrap()]);
    assert!(!StartKit::parse(&doc(&gone)).declared);
    assert!(StartKit::default().edit(&doc("[module]\nid = x\n")).is_none());
}

#[test]
fn the_json_of_both() {
    let d = doc(MANIFEST);
    let w = World::parse(&d).json();
    assert_eq!(
        w,
        json!({
            "rules": { "original_npcs": false, "random_stalkers": true, "mutants": false, "npc_quests": true, "story_quests": true },
            "player_choice": ["mutants", "npc_quests"],
            "levels": ["l01_escape", "l02_garbage"],
            "area": [],
            "locked_levels": [],
            "locked": ["esc_to_gar"],
            "declared": true,
        })
    );
    let keys: Vec<&String> = w.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["rules", "player_choice", "levels", "area", "locked_levels", "locked", "declared"]);
    assert_eq!(
        StartKit::default().json(),
        json!({ "declared": false, "money": null, "perks": true, "items": [] })
    );
}

#[test]
fn world_set_changes_only_what_it_is_given() {
    let w = World::parse(&doc(MANIFEST));
    let empty = set_world(&args(json!({ "preset": "empty" })), w.clone()).unwrap();
    assert_eq!(empty.keep, [false; 5]);
    assert_eq!(empty.levels, w.levels);
    let stock = set_world(&args(json!({ "preset": "stock", "mutants": false })), w.clone()).unwrap();
    assert_eq!(stock.keep, [true, true, false, true, true]);
    assert!(stock.levels.is_empty());
    let lists = set_world(&args(json!({ "levels": "", "area": "a, b", "lock": "x,esc_to_gar", "unlock": "esc_to_gar", "area_remove": "a" })), w.clone()).unwrap();
    assert!(lists.levels.is_empty());
    assert_eq!(lists.area, ["b"]);
    assert_eq!(lists.locked, ["x"]);
    assert!(
        set_world(&args(json!({ "preset": "nothing" })), w.clone())
            .unwrap_err()
            .starts_with("preset must be")
    );
    assert_eq!(
        set_world(&args(json!({ "player_choice": "mutants, aliens" })), w.clone()).unwrap_err(),
        "player_choice: unknown rule 'aliens' (original_npcs, random_stalkers, mutants, npc_quests, story_quests)"
    );
    assert_eq!(
        set_world(&args(json!({ "player_choice": "mutants,npc_quests,story_quests,original_npcs" })), w)
            .unwrap_err(),
        "player_choice: the new-game screen has room for 3 rules"
    );
}

#[test]
fn world_set_kit_fields_declare_the_kit() {
    let k = StartKit::default();
    let set = set_kit(
        &args(json!({ "start_kit": "device_torch, wpn_knife ,bandage=2", "start_money": "-5" })),
        k.clone(),
        None,
    )
    .unwrap();
    assert!(set.declared);
    assert_eq!(set.money, -1);
    assert_eq!(set.items.len(), 3);
    assert_eq!(set.items[2], KitItem { section: "bandage".into(), count: 2 });
    assert_eq!(
        set_kit(&args(json!({ "start_kit": "bandage=0" })), k.clone(), None).unwrap_err(),
        "start_kit: 'bandage=0' needs an item section and a count of at least 1 (section or section=N)"
    );
    let perks = set_kit(&args(json!({ "start_perks": false })), k.clone(), None).unwrap();
    assert!(perks.declared && !perks.perks);
    let base = StartKit {
        declared: true,
        money: 1000,
        perks: true,
        items: vec![KitItem { section: "bread".into(), count: 1 }],
    };
    let from_base = set_kit(
        &args(json!({ "start_kit_remove": true, "start_perks": false })),
        set.clone(),
        Some(Some(base)),
    )
    .unwrap();
    assert_eq!(from_base.money, 1000);
    assert!(!from_base.perks);
    assert!(set_kit(&args(json!({})), k.clone(), Some(None)).unwrap_err().starts_with("start_kit_base: "));
    assert_eq!(set_kit(&args(json!({ "start_kit_remove": "1" })), set, None).unwrap(), StartKit::default());
}

#[test]
fn the_game_base_kit_is_shared() {
    let text = "[stalker]\nwpn_pm = 1\n[shared] ; every faction\nmoney = 1000,3000\nbread\nbandage = 0\n[bandit]\nvodka\n";
    let k = base_kit(text).unwrap();
    assert_eq!(k.money, 1000);
    assert_eq!(
        k.items,
        [KitItem { section: "bread".into(), count: 1 }, KitItem { section: "bandage".into(), count: 1 }]
    );
    assert!(base_kit("[stalker]\nx\n").is_none());
}

// ---- starts --------------------------------------------------------------------------------

#[test]
fn starts_are_the_start_ops() {
    let text = "[obj:player_start]\nop = start ; the only one\nlevel = l03_agroprom\nposition = -144.2, -5.8 ,73.9\ndirection = 0,1.57\n[obj:other]\nop = spawn\n[meta]\nop = start\n";
    let s = read_starts(text, "spawn/l03.xspawn");
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].name, "player_start");
    assert_eq!(s[0].level, "l03_agroprom");
    assert_eq!(s[0].source, "spawn/l03.xspawn");
    assert_eq!(s[0].position, [-144.2, -5.8, 73.9]);
    assert_eq!(s[0].direction, [0.0, 1.57, 0.0]);
}

// ---- validation ----------------------------------------------------------------------------

fn game() -> GameWorld {
    GameWorld {
        ok: true,
        error: String::new(),
        levels: vec!["l01_escape".into(), "l02_garbage".into()],
        transitions: vec![GameTransition {
            name: "esc_to_gar".into(),
            level: "l01_escape".into(),
            to_level: "l02_garbage".into(),
            position: [0.0; 3],
        }],
        patrols: vec!["esc_guard".into()],
    }
}

fn start(level: &str, name: &str) -> Start {
    Start { level: level.into(), name: name.into(), source: "spawn/a.xspawn".into(), ..Start::default() }
}

#[test]
fn every_world_code() {
    let mut w = World::from_lines([
        "player_choice = original_npcs, random_stalkers, mutants, npc_quests",
        "levels = L01_ESCAPE, nowhere",
        "locked = esc_to_gar, gone",
    ]);
    w.area = vec!["l02_garbage".into()];
    let k = StartKit::from_lines(["wpn_knife = 0", "bread", "bread", "toy"]);
    let starts = [start("l01_escape", "a"), start("moon", "b")];
    let known = |s: &str| s != "toy";
    let p = validate(&w, &k, &starts, Some(&game()), Some(&known));
    assert_eq!(
        p,
        [
            "E201 world: 2 Player Starts - a module has one: l01_escape:a (spawn/a.xspawn), moon:b (spawn/a.xspawn)",
            "E202 world: 4 rules offered to the player - the new-game screen has room for 3",
            "W203 world: levels names 'nowhere', a level the linked game does not have",
            "W204 world: locked names 'gone', a transition the linked game does not have",
            "W203 world: the Player Start b stands on 'moon', a level the linked game does not have",
            "W205 world: the Player Start stands on l01_escape, outside the playable area",
            "W205 world: the Player Start stands on moon, outside the playable area",
            "E207 world: start kit item 'wpn_knife' has a count of 0 - at least 1",
            "E207 world: start kit item 'bread' is listed twice - one row says how many",
            "W208 world: start kit item 'toy' is an item of neither the linked game nor the module - a new game leaves it out",
        ]
    );
    let area_only = World { area: vec!["l01_escape".into()], ..World::default() };
    let p = validate(&area_only, &StartKit::default(), &[], None, None);
    assert_eq!(
        p,
        [
            "W206 world: the playable area is limited, but there is no Player Start - the player begins at the faction's start, which may lie outside it"
        ]
    );
    let row = StartKit {
        declared: true,
        items: vec![KitItem { section: String::new(), count: 1 }],
        ..StartKit::default()
    };
    assert_eq!(
        validate(&World::default(), &row, &[], None, None),
        ["E207 world: a start kit row names no item"]
    );
    assert!(
        validate(&World::default(), &StartKit::default(), &[start("l01_escape", "a")], Some(&game()), None)
            .is_empty()
    );
}

#[test]
fn transitions_close_in_the_engines_order() {
    let w = World {
        locked: vec!["a".into()],
        locked_levels: vec!["l1".into()],
        area: vec!["l2".into()],
        ..World::default()
    };
    assert_eq!(w.closed_by("a", "l2", "l2"), "locked");
    assert_eq!(w.closed_by("b", "l1", "l2"), "level");
    assert_eq!(w.closed_by("b", "l3", "l4"), "area");
    assert_eq!(w.closed_by("b", "l3", "l2"), "");
    let g = game();
    assert!(g.level_exists("L02_GARBAGE"));
    assert!(g.patrol_exists("esc_guard") && !g.patrol_exists("esc"));
    assert_eq!(g.transitions_on("l01_escape"), 1);
}

// ---- the linked game -----------------------------------------------------------------------

/// A level changer's spawn packet as `Spawn_Write` and the state chain lay it out.
fn changer_packet(to_level: &str, version: u16) -> Vec<u8> {
    let mut w = Writer::new();
    w.w_u16(1);
    w.w_stringz("level_changer");
    w.w_stringz("esc_to_gar");
    w.w_u8(0);
    w.w_u8(0xFE);
    w.w_vec3(Vec3::new(1.0, 2.0, 3.0));
    w.w_vec3(Vec3::ZERO);
    w.w_u16(0);
    w.w_u16(7);
    w.w_u16(u16::MAX);
    w.w_u16(u16::MAX);
    w.w_u16(1 << 5);
    w.w_u16(version);
    w.w_u16(u16::MAX);
    w.w_u16(12);
    w.w_u16(2);
    w.w_u8(9);
    w.w_u8(9);
    w.w_u16(3);
    w.w_u16(0);
    // CSE_ALifeObject
    w.w_u16(4);
    w.w_f32(0.0);
    w.w_u32(1);
    w.w_u32(5);
    w.w_u32(0);
    w.w_stringz("");
    w.w_u32(u32::MAX);
    w.w_u32(u32::MAX);
    // one sphere, one box
    w.w_u8(2);
    w.w_u8(0);
    w.w_f32(0.0);
    w.w_f32(0.0);
    w.w_f32(0.0);
    w.w_f32(1.0);
    w.w_u8(1);
    for _ in 0..12 {
        w.w_f32(0.0);
    }
    w.w_u8(0);
    w.w_u16(9);
    w.w_u32(10);
    w.w_vec3(Vec3::ZERO);
    w.w_vec3(Vec3::ZERO);
    w.w_stringz(to_level);
    w.w_stringz("start");
    w.into_bytes()
}

#[test]
fn a_level_changers_destination_is_walked() {
    assert_eq!(walk_destination(&changer_packet("l02_garbage", 128)).as_deref(), Some("l02_garbage"));
    // a format older than every linked game is not read
    assert!(walk_destination(&changer_packet("l02_garbage", 100)).is_none());
    let p = changer_packet("l02_garbage", 128);
    assert!(walk_destination(&p[..p.len() - 20]).is_none());
    assert!(is_level_changer("level_changer_esc", None));
    assert!(!is_level_changer("stalker", None));
}

#[test]
fn a_problem_carries_its_section_and_key_and_the_document_puts_it_at_the_line() {
    let text = "[module]\r\nid = od\r\n\r\n[world]\r\nmutants = false\r\nplayer_choice = original_npcs, random_stalkers, mutants, npc_quests\r\nlevels = nowhere\r\nlocked = gone\r\narea = l02_garbage\r\n\r\n[start_kit]\r\nmoney = 10\r\nwpn_knife = 0\r\nbread\r\nbread\r\ntoy\r\n";
    let doc = gz_mod::ManifestDoc::parse(text);
    let (w, k) = (World::parse(&doc), StartKit::parse(&doc));
    let known = |s: &str| s != "toy";
    let found = crate::validate::issues(&w, &k, &[], Some(&game()), Some(&known));
    let at = |code: &str, text: &str| {
        let i = found
            .iter()
            .find(|i| i.code == code && i.text.contains(text))
            .unwrap_or_else(|| panic!("{code} {text}: {found:?}"));
        doc.locate(&i.spot).map(|p| p.line)
    };
    assert_eq!(at("E202", "rules offered"), Some(6));
    assert_eq!(at("W203", "levels names"), Some(7));
    assert_eq!(at("W204", "locked names"), Some(8));
    assert_eq!(at("W206", "no Player Start"), Some(9));
    // the start kit's rows are at their own lines, not at [world]
    assert_eq!(at("E207", "'wpn_knife' has a count"), Some(13));
    assert_eq!(at("E207", "'bread' is listed twice"), Some(15));
    assert_eq!(at("W208", "'toy'"), Some(16));
    let row = StartKit {
        declared: true,
        items: vec![KitItem { section: String::new(), count: 1 }],
        ..StartKit::default()
    };
    let found = crate::validate::issues(&World::default(), &row, &[], None, None);
    assert_eq!(doc.locate(&found[0].spot).map(|p| p.line), Some(11));
    // the same problems as lines are what the answers have always said
    let lines: Vec<String> = crate::validate::issues(&w, &k, &[], Some(&game()), Some(&known))
        .iter()
        .map(gz_mod::Issue::line)
        .collect();
    assert_eq!(lines, validate(&w, &k, &[], Some(&game()), Some(&known)));
}
