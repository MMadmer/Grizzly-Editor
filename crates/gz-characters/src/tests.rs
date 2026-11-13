#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use gz_base::Rev;
use gz_luadata::Value;
use gz_nq::PickerType;

use crate::game::{model_path, ogf_bone_names, trade_categories};
use crate::model::{
    Character, Supply, TradeRow, diff_rules, load_text, parse, template, text_collapse, text_in, text_raw,
    text_set, to_value, valid_id, write,
};
use crate::resolve::resolve;
use crate::service::Roster;
use crate::validate::{Cx, Lookups, Models, validate};

fn ch(text: &str, file: &str) -> Character {
    load_text(text, file).unwrap()
}

// ---- the file ------------------------------------------------------------------------------

#[test]
fn parse_reads_every_field_and_keeps_unset_unset() {
    let c = ch(
        r#"return { npc = 1, id = "guide", base = "guard", name = { rus = "Проводник", eng = "Guide" },
            rank = 300, money = { min = 900, max = 100 }, squad = false,
            supplies = { { section = "medkit", count = 2 }, "bread", { section = "x", count = -3 }, { count = 5 } },
            trade = { sells = "food", buys = { "weapon", "" }, markup = 1.5, stock = { "vodka", { section = "ammo", count = 3, chance = 0.5 }, 7 } } }"#,
        "characters/guide.npcasset",
    );
    assert_eq!(c.id, "guide");
    assert_eq!(c.base, "guard");
    assert!(c.community.is_empty() && c.visual.is_empty() && !c.has_reputation);
    assert!(c.has_rank && c.rank == 300);
    assert!(c.has_money && c.money_min == 900 && c.money_max == 900);
    assert!(c.has_squad && !c.squad);
    assert_eq!(c.supplies, vec![Supply::new("medkit", 2), Supply::new("bread", 1), Supply::new("x", 0)]);
    assert!(c.trade.is_trader());
    assert_eq!(c.trade.sells, ["food"]);
    assert_eq!(c.trade.buys, ["weapon"]);
    assert_eq!(c.trade.pays, 0.3);
    assert_eq!(c.trade.restock, 24);
    assert_eq!(c.trade.stock.len(), 3);
    assert_eq!(c.trade.stock[1], TradeRow { section: "ammo".into(), count: 3, chance: 0.5 });
    // a row that is neither a string nor a table is kept empty, for E118 to name
    assert!(c.trade.stock[2].section.is_empty());
}

#[test]
fn the_file_name_is_the_id_when_the_file_names_none() {
    let c = ch("return { npc = 1 }", "chars/Old Guard.npcasset");
    assert_eq!(c.id, "old_guard");
    assert_eq!(c.file, "chars/Old Guard.npcasset");
    assert!(parse("return 5", "x").is_err());
    let err = parse("return {\n  id = \n}", "x").unwrap_err();
    assert!(err.starts_with("line "), "{err}");
}

#[test]
fn trade_false_takes_a_shop_away() {
    let c = ch("return { trade = false }", "a.npcasset");
    assert!(c.trade.declared && c.trade.off && !c.trade.is_trader());
    let c = ch("return { trade = true }", "a.npcasset");
    assert!(!c.trade.declared);
}

#[test]
fn a_new_file_is_written_canonically() {
    let text = template("Courier 2");
    assert_eq!(
        text,
        "-- A character of this module. The game reads this file as it is - see docs/MOD_QUICKSTART.md.\n\
         return {\n\
         \tnpc       = 1,\n\
         \tid        = \"courier_2\",\n\
         \tname      = \"New character\",\n\
         \tcommunity = \"stalker\",\n\
         \tvisual    = \"actors\\\\stalker_neutral\\\\stalker_neutral_1\",\n\
         \ticon      = \"ui_inGame2_neutral_2\",\n\
         \trank      = 300,\n\
         }\n"
    );
    assert!(template("").contains("\"new_character\""));

    let full = ch(
        r#"return { id = "t", abstract = true, reputation = -5, money = { min = 1, max = 2, infinite = true }, squad = false,
            supplies = { { section = "a", count = 3 } }, start_dialog = "d",
            trade = { sells = { "food" }, buys = {}, pays = 0.25, restock = 12, stock = { { section = "v", chance = 0.3 } } } }"#,
        "t.npcasset",
    );
    let out = write(&full);
    assert!(out.contains("\tabstract  = true,\n"));
    assert!(out.contains("\treputation = -5,\n"));
    assert!(out.contains("\tmoney     = { min = 1, max = 2, infinite = true },\n"));
    assert!(out.contains("\t\t{ section = \"a\", count = 3 },\n"));
    assert!(out.contains("\t\tsells   = { \"food\" },\n\t\tbuys    = {},\n\t\tmarkup  = 1,\n\t\tpays    = 0.25,\n\t\trestock = 12,\n"));
    assert!(out.contains("\t\t\t{ section = \"v\", chance = 0.3 },\n"));
    assert!(out.contains("\tstart_dialog = \"d\",\n\tsquad     = false,\n}\n"));
}

#[test]
fn a_save_writes_only_what_changed() {
    let text = "-- the guide\nreturn {\n  npc = 1,\n  id = \"guide\",\n  name = \"Guide\", -- shown to the player\n  future_key = { 1 },\n  rank = 300,\n  supplies = { \"bread\" },\n}\n";
    let c = ch(text, "guide.npcasset");
    let mut after = c.clone();
    after.rank = 500;
    after.community = "dolg".into();
    let mut doc = gz_luadata::LuaDoc::parse(text).unwrap();
    assert!(!doc.apply_diff(&to_value(&c), &to_value(&c), &diff_rules()).unwrap());
    assert!(doc.apply_diff(&to_value(&c), &to_value(&after), &diff_rules()).unwrap());
    assert_eq!(
        doc.text(),
        "-- the guide\nreturn {\n  npc = 1,\n  id = \"guide\",\n  name = \"Guide\", -- shown to the player\n  community = \"dolg\",\n  future_key = { 1 },\n  rank = 500,\n  supplies = { \"bread\" },\n}\n"
    );
    // a newer format is refused, saying so
    let e = parse("return { npc = 2, id = \"x\" }", "x").unwrap_err();
    assert!(e.contains("newer Grizzly Editor"), "{e}");
}

// ---- texts ---------------------------------------------------------------------------------

#[test]
fn a_text_with_rus_alone_collapses_to_a_string() {
    let mut v = Value::Nil;
    text_set(&mut v, "rus", "Курьер");
    assert_eq!(v, Value::from("Курьер"));
    // a second language turns it into a table, the first one kept as rus
    text_set(&mut v, "eng", "Courier");
    assert_eq!(text_raw(&v, "rus"), "Курьер");
    assert_eq!(text_raw(&v, "eng"), "Courier");
    // the form reads what is stored, the game falls back
    let only_rus = Value::from("Курьер");
    assert_eq!(text_raw(&only_rus, "eng"), "");
    assert_eq!(text_in(&only_rus, "eng"), "Курьер");
    // taking the second language away leaves a table of rus alone, which collapses
    text_set(&mut v, "eng", "");
    assert!(v.is_table());
    text_collapse(&mut v);
    assert_eq!(v, Value::from("Курьер"));
    // and the writer does the same with what it is given
    let mut table = Value::table();
    table.set("rus", "Курьер");
    let c = Character { id: "c".into(), name: table, ..Character::default() };
    assert!(write(&c).contains("\tname      = \"Курьер\",\n"));
    text_set(&mut v, "rus", "");
    assert!(v.is_nil());
}

#[test]
fn ids() {
    assert!(valid_id("guard_2"));
    assert!(!valid_id("2guard"));
    assert!(!valid_id("Guard"));
    assert!(!valid_id(""));
    assert!(!valid_id(&"a".repeat(49)));
    assert_eq!(crate::model::game_id("od", "courier"), "od_courier");
    assert_eq!(crate::model::game_id("", "courier"), "courier");
}

// ---- inheritance ---------------------------------------------------------------------------

#[test]
fn resolve_inherits_unset_fields_and_merges_kits() {
    let mut all = vec![
        ch(
            r#"return { id = "guard", abstract = true, community = "dolg", rank = 500,
                supplies = { { section = "wpn_ak74", count = 1 }, { section = "medkit", count = 2 }, { section = "bread", count = 1 } },
                trade = { sells = { "food" } } }"#,
            "guard.npcasset",
        ),
        ch(
            r#"return { id = "captain", base = "guard", name = "Капитан", visual = "v",
                supplies = { { section = "MEDKIT", count = 5 }, { section = "bread", count = 0 }, { section = "vodka" } },
                trade = { buys = { "weapon" } } }"#,
            "captain.npcasset",
        ),
        ch(r#"return { id = "private", base = "captain", rank = 100, trade = false }"#, "private.npcasset"),
    ];
    let mut problems = Vec::new();
    resolve(&mut all, &mut problems);
    assert!(problems.is_empty(), "{problems:?}");
    let captain = &all[1];
    assert_eq!(captain.community, "dolg");
    assert_eq!(captain.rank, 500);
    assert!(!captain.is_abstract);
    // a repeated section takes the new count and keeps its place, count 0 drops a row
    assert_eq!(
        captain.supplies,
        vec![Supply::new("wpn_ak74", 1), Supply::new("medkit", 5), Supply::new("vodka", 1)]
    );
    // a shop is replaced as a whole
    assert!(captain.trade.sells.is_empty());
    assert_eq!(captain.trade.buys, ["weapon"]);
    let private = &all[2];
    assert_eq!(private.rank, 100);
    assert_eq!(text_in(&private.name, "rus"), "Капитан");
    assert_eq!(private.supplies.len(), 3);
    assert!(!private.trade.is_trader());
    // the defaults are applied once, after the chain
    assert_eq!(private.spec_rank, "novice");
    assert_eq!(all[0].community, "dolg");
}

#[test]
fn resolve_reports_missing_bases_and_loops() {
    let mut all = vec![
        ch(r#"return { id = "a", base = "b" }"#, "a.npcasset"),
        ch(r#"return { id = "b", base = "a" }"#, "b.npcasset"),
        ch(
            r#"return { id = "c", base = "nobody", supplies = { { section = "x", count = 0 } } }"#,
            "c.npcasset",
        ),
        ch(r#"return { id = "d", base = "d" }"#, "d.npcasset"),
    ];
    let mut problems = Vec::new();
    resolve(&mut all, &mut problems);
    assert_eq!(
        problems,
        [
            "E112 a.npcasset: 'a' inherits from itself",
            "E112 b.npcasset: 'b' inherits from itself",
            "E111 c.npcasset: there is no character 'nobody' to inherit from",
            "E112 d.npcasset: 'd' inherits from itself",
        ]
    );
    // a broken chain keeps its own fields, its empty rows dropped and no defaults applied
    assert!(all[2].supplies.is_empty());
    assert!(all[2].community.is_empty());
}

#[test]
fn a_chain_that_loops_further_up_is_e112() {
    let mut all = vec![
        ch(r#"return { id = "a", base = "b" }"#, "a.npcasset"),
        ch(r#"return { id = "b", base = "c" }"#, "b.npcasset"),
        ch(r#"return { id = "c", base = "b" }"#, "c.npcasset"),
    ];
    let mut problems = Vec::new();
    resolve(&mut all, &mut problems);
    assert_eq!(problems[0], "E112 a.npcasset: the base chain of 'a' comes back to where it started");
}

#[test]
fn the_roster_reports_unreadable_files_and_shared_ids() {
    let r = Roster::build(
        Rev(1),
        vec![
            ("b.npcasset".into(), Ok(r#"return { id = "same", name = "Б" }"#.into())),
            ("a.npcasset".into(), Ok(r#"return { id = "same", abstract = true }"#.into())),
            ("bad.npcasset".into(), Ok("return { id = ".into())),
            ("gone.npcasset".into(), Err("cannot open file".into())),
            ("z.npcasset".into(), Ok(r#"return { id = "alpha" }"#.into())),
        ],
    );
    assert!(r.problems[0].starts_with("E100 bad.npcasset: line "));
    assert_eq!(r.problems[1], "E100 gone.npcasset: cannot open file");
    assert_eq!(r.problems[2], "E101 two characters share the id 'same': b.npcasset and a.npcasset");
    assert_eq!(r.raw.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["alpha", "same", "same"]);
    assert_eq!(r.briefs.len(), 3);
    assert_eq!(r.briefs[1].name, "Б");
    assert!(r.briefs[2].template);
}

// ---- validation ----------------------------------------------------------------------------

struct FakeLookups;

impl Lookups for FakeLookups {
    fn available(&self) -> bool {
        true
    }

    fn has_any(&self, _t: PickerType) -> bool {
        true
    }

    fn known(&self, t: PickerType, id: &str) -> bool {
        match t {
            PickerType::Community => id == "stalker",
            PickerType::Icon => id == "ui_face",
            PickerType::Dialog => id == "hello",
            PickerType::Item => id == "medkit",
            _ => false,
        }
    }
}

struct FakeModels {
    damage: Vec<String>,
}

impl Models for FakeModels {
    fn exists(&self, rel: &str) -> bool {
        rel.starts_with("meshes\\actors\\")
    }

    fn bones(&self, rel: &str) -> Arc<[String]> {
        let names: &[&str] = if rel.contains("forester") {
            &["root", "spine"]
        } else {
            &["root", "spine", "eye_left", "jaw_1"]
        };
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    fn damage_bones(&self) -> &[String] {
        &self.damage
    }
}

fn problems_of(text: &str, linked: bool) -> Vec<String> {
    let mut all = vec![ch(text, "characters/x.npcasset")];
    let mut problems = Vec::new();
    resolve(&mut all, &mut problems);
    let models = FakeModels { damage: vec!["eye_left".into(), "jaw_1".into()] };
    let cats = vec!["food".to_owned(), "weapon".to_owned()];
    let cx = Cx { linked, pickers: Some(&FakeLookups), trade_categories: &cats, models: Some(&models) };
    validate(&all[0], &cx, &mut problems);
    problems
}

fn codes(problems: &[String]) -> Vec<&str> {
    problems.iter().map(|p| p.split(' ').next().unwrap_or("")).collect()
}

const OK: &str = r#"return { id = "x", name = "Икс", visual = "actors\\stalker", icon = "ui_face", start_dialog = "hello", supplies = { "medkit" } }"#;

#[test]
fn a_good_character_has_no_problem() {
    assert!(problems_of(OK, true).is_empty(), "{:?}", problems_of(OK, true));
}

#[test]
fn every_code_on_its_fixture() {
    let p = problems_of(r#"return { id = "2x" , visual = "v" }"#, false);
    assert_eq!(codes(&p), ["E102", "E103"]);
    assert_eq!(
        p[0],
        "E102 characters/x.npcasset: id '2x' is not usable (a-z, 0-9 and _, not starting with a digit, up to 48)"
    );
    assert_eq!(p[1], "E103 characters/x.npcasset: no name - it is what the player reads");

    let p = problems_of(r#"return { id = "x", name = "a" }"#, false);
    assert_eq!(p, ["E104 characters/x.npcasset: no visual - an NPC without a model cannot spawn"]);
    // a template owes the player nothing
    assert!(problems_of(r#"return { id = "x", abstract = true }"#, true).is_empty());

    let p = problems_of(r#"return { id = "x", name = "a", visual = "other\\m" }"#, true);
    assert_eq!(
        p,
        [
            "W105 characters/x.npcasset: the linked game has no model at 'meshes\\other\\m.ogf' - check it is staged in the project"
        ]
    );

    let p = problems_of(r#"return { id = "x", name = "a", visual = "actors\\forester.ogf" }"#, true);
    assert_eq!(
        p,
        [
            "E123 characters/x.npcasset: the model has no bone eye_left, jaw_1 - an NPC's damage names them and the game stops the moment it comes online; pick a model on the stalker skeleton"
        ]
    );

    let p =
        problems_of(r#"return { id = "x", name = "a", visual = "actors\\s", community = "aliens" }"#, true);
    assert_eq!(codes(&p), ["W114"]);
    assert!(
        problems_of(r#"return { id = "x", name = "a", visual = "actors\\s", community = "aliens" }"#, false)
            .is_empty()
    );

    let p = problems_of(
        r#"return { id = "x", name = "a", visual = "actors\\s", icon = "nope", start_dialog = "nope", supplies = { "nope" } }"#,
        true,
    );
    assert_eq!(codes(&p), ["W115", "W116", "W117"]);
    assert_eq!(p[1], "W116 characters/x.npcasset: no dialog 'nope' in the linked game or the project");

    let p = problems_of(r#"return { id = "x", name = "Привет ★", visual = "v" }"#, false);
    assert_eq!(p, ["W107 characters/x.npcasset: '★' does not exist in cp1251 and will not reach the player"]);

    let mut c = ch(OK, "characters/x.npcasset");
    c.supplies = vec![Supply::new("", 1), Supply::new("medkit", -1)];
    c.has_money = true;
    c.money_min = 5;
    c.money_max = 1;
    let mut p = Vec::new();
    validate(&c, &Cx::default(), &mut p);
    assert_eq!(codes(&p), ["E108", "E109", "E110"]);
    assert_eq!(p[1], "E109 characters/x.npcasset: supply 'medkit' asks for less than one item");
}

#[test]
fn trade_codes() {
    let p = problems_of(
        r#"return { id = "x", name = "a", visual = "v",
            trade = { markup = 0, pays = -1, restock = 0, sells = { "toys" }, buys = { "food" },
                      stock = { { count = 1 }, { section = "a", count = 0 }, { section = "b", chance = 2 }, { section = "medkit" }, { section = "MEDKIT" } } } }"#,
        true,
    );
    assert_eq!(
        codes(&p),
        ["W105", "E118", "E118", "E118", "E118", "E119", "E119", "E119", "W120", "W121", "W121", "W121"]
    );
    assert_eq!(p[1], "E118 characters/x.npcasset: a stock row without an item");
    assert_eq!(p[3], "E118 characters/x.npcasset: stock 'b' has a chance outside 0..1");
    assert_eq!(p[4], "E118 characters/x.npcasset: 'MEDKIT' is in the stock twice - one row per item");
    assert_eq!(
        p[8],
        "W120 characters/x.npcasset: the linked game has no trade category 'toys' the trader sells - the trade presets name weapon, drugs, food and the like"
    );
    let p =
        problems_of(r#"return { id = "x", name = "a", visual = "v", trade = { buys = { "food" } } }"#, false);
    assert_eq!(
        p,
        ["W122 characters/x.npcasset: the trader has nothing to sell - give it a stock or a category"]
    );
}

// ---- the linked game -----------------------------------------------------------------------

#[test]
fn trade_categories_are_sections_that_list_items() {
    let text = "; presets\n[weapon]\nwpn_ak74 = 1, 1\n[unused]\nx\n[medic]:unused,weapon\n[empty]\n[food] ; comment\nbread\n# not a line\n";
    assert_eq!(trade_categories(text), ["weapon", "food"]);
}

fn ogf(chunks: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, body) in chunks {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
    }
    out
}

#[test]
fn bone_names_come_from_their_chunk() {
    let mut names = 2u32.to_le_bytes().to_vec();
    for (bone, parent) in [("root", ""), ("eye_left", "root")] {
        names.extend_from_slice(bone.as_bytes());
        names.push(0);
        names.extend_from_slice(parent.as_bytes());
        names.push(0);
        names.extend_from_slice(&[0u8; 60]);
    }
    let data = ogf(&[(1, vec![0; 44]), (13, names.clone())]);
    assert_eq!(ogf_bone_names(&data).unwrap(), ["root", "eye_left"]);
    // a static model, or a cut one, tells nothing
    assert!(ogf_bone_names(&ogf(&[(1, vec![0; 44])])).is_none());
    names.truncate(names.len() - 10);
    assert!(ogf_bone_names(&ogf(&[(13, names)])).is_none());
    assert_eq!(model_path("actors\\a"), "meshes\\actors\\a.ogf");
    assert_eq!(model_path("actors\\a.ogf"), "meshes\\actors\\a.ogf");
}
