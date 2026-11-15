#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use gz_base::{Rev, Severity};
use gz_ltx::{Dialect, Ltx, ParsedFile, parse_file};
use gz_luadata::{LuaDoc, Value};
use rustc_hash::FxHashMap;

use crate::catalog::Catalog;
use crate::compose::{ConfigFile, ModuleConfigs};
use crate::model::{
    Count, Icon, Kind, SetValue, diff_rules, from_value, load_text, template, to_value, write,
};
use crate::resolve::From;
use crate::roster::Roster;
use crate::service::issues_of;
use crate::validate::{Lookups, NoGame};

// ---- fixtures ----------------------------------------------------------------------------------

const GAME: &str = "\
[base_item]\nclass = II_FOOD\ncost = 10\ninv_weight = 0.1\n\
[bread]:base_item\ninv_name = st_bread\n\
[wpn_base]\nclass = WP_AK74\ncost = 100\ninv_weight = 3\nammo_class = ammo_a, ammo_b\nhud = wpn_ak74_hud\nicons_texture = ui\\ui_icon_weapons\n\
[wpn_bare]\nclass = WP_AK74\ncost = 1\ninv_weight = 1\n\
[wpn_ak74]:wpn_base\nvisual = dynamics\\weapons\\wpn_ak74\nsnd_shoot = weapons\\ak74_shoot\n\
[wpn_ak74_kobra]:wpn_ak74\nscope = kobra\n\
[wpn_ak74_hud]\nitem_visual = dynamics\\weapons\\wpn_ak74_hud\nanm_idle = ak74_idle\n\
[nameless]\ncost = 5\n\
[itm_repairkit]:base_item\n";

const CATALOG: &str = "\
[catalog]\nversion = 1\n\
[families]\norder = weapon, item\n\
[family.weapon]\ntitle = Weapon\nclasses = WP_AK74\nlike = wpn_ak74\n\
[family.item]\ntitle = Item\nclasses = II_FOOD\nlike = bread\n\
[item.cost]\nfamilies = *\ntype = int\nrequired = true\ntitle = Cost\nhelp = What a trader asks for it.\n\
[item.inv_weight]\nfamilies = *\ntype = float\nrequired = true\ntitle = Weight\n\
[item.class]\nfamilies = *\ntype = string\nrequired = true\n\
[item.visual]\ntype = visual\n\
[item.snd_shoot]\nfamilies = weapon\ntype = sound\npattern = snd_shoot<N>\n\
[item.snd_reload]\nfamilies = weapon\ntype = sound\n\
[item.icons_texture]\ntype = texture\n\
[item.flame_particles]\nfamilies = weapon\ntype = particles\n\
[item.ammo_class]\nfamilies = weapon\ntype = section\n\
[item.inv_name]\ntype = string_id\n\
[item.fire_modes]\nfamilies = weapon\ntype = token\nvalues = single, auto\n\
[hud.anm_idle]\nfamilies = weapon\ntype = motion\n\
[hud.hands_position]\nfamilies = weapon\ntype = vec3\n\
[hud.item_visual]\nfamilies = weapon\ntype = visual\nrequired = true\n";

fn game() -> Arc<Ltx> {
    let files: FxHashMap<String, ParsedFile> =
        [("configs\\system.ltx".to_owned(), parse_file(GAME, Dialect::Editor))].into_iter().collect();
    Arc::new(Ltx::assemble_map("configs\\system.ltx", &files, Dialect::Editor).unwrap().ltx)
}

fn roster_in(root: &Path, files: &[(&str, &str)], cfg: &ModuleConfigs) -> Roster {
    let files: Vec<(String, Result<String, String>)> =
        files.iter().map(|(f, t)| ((*f).to_owned(), Ok((*t).to_owned()))).collect();
    Roster::build(Rev(1), "we", files, game(), cfg, root, &crate::atlas::picture_size)
}

/// A picture of one colour, written where the test wants it.
fn picture(path: &Path, w: u32, h: u32) {
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 40, 200, 255]).collect();
    let img = gz_fmt_tex::Image::new(w, h, px).unwrap();
    std::fs::write(path, gz_fmt_tex::write_tga(&img, gz_fmt_tex::TgaDepth::Bgra32).unwrap()).unwrap();
}

fn roster(files: &[(&str, &str)]) -> Roster {
    roster_in(Path::new("D:\\nowhere"), files, &ModuleConfigs::default())
}

/// What the check looks up: a game that has what these lists hold.
#[derive(Default)]
struct Game {
    models: Vec<&'static str>,
    sounds: Vec<&'static str>,
    textures: Vec<(&'static str, (u32, u32))>,
    particles: Vec<&'static str>,
    strings: Vec<&'static str>,
    communities: Vec<&'static str>,
    configs: Vec<(&'static str, Vec<&'static str>)>,
    motions: Vec<&'static str>,
}

impl Lookups for Game {
    fn linked(&self) -> bool {
        true
    }
    fn model(&self, name: &str) -> bool {
        self.models.iter().any(|m| m.eq_ignore_ascii_case(name))
    }
    fn texture(&self, name: &str) -> bool {
        self.textures.iter().any(|(t, _)| t.eq_ignore_ascii_case(name))
    }
    fn texture_size(&self, name: &str) -> Option<(u32, u32)> {
        self.textures.iter().find(|(t, _)| t.eq_ignore_ascii_case(name)).map(|(_, s)| *s)
    }
    fn sound(&self, name: &str) -> bool {
        self.sounds.iter().any(|s| s.eq_ignore_ascii_case(name))
    }
    fn particle(&self, name: &str) -> bool {
        self.particles.iter().any(|s| s.eq_ignore_ascii_case(name))
    }
    fn string_id(&self, id: &str) -> bool {
        self.strings.iter().any(|s| s.eq_ignore_ascii_case(id))
    }
    fn community(&self, name: &str) -> bool {
        self.communities.iter().any(|s| s.eq_ignore_ascii_case(name))
    }
    fn config_sections(&self, file: &str) -> Option<Vec<String>> {
        self.configs
            .iter()
            .find(|(f, _)| *f == file)
            .map(|(_, s)| s.iter().map(|x| (*x).to_owned()).collect())
    }
    fn motions(&self, _visual: &str) -> Option<Vec<String>> {
        Some(self.motions.iter().map(|m| (*m).to_owned()).collect())
    }
    fn picture(&self, path: &Path) -> Result<(u32, u32), String> {
        crate::atlas::picture_size(path)
    }
}

fn full_game() -> Game {
    Game {
        models: vec!["dynamics\\weapons\\wpn_ak74", "dynamics\\weapons\\wpn_ak74_hud"],
        sounds: vec!["weapons\\ak74_shoot"],
        textures: vec![("ui\\ui_icon_equipment", (2048, 1024))],
        particles: vec!["weapons\\ak74_flame"],
        strings: vec!["st_bread"],
        communities: vec!["bandit", "stalker", "stalker_veteran"],
        configs: vec![
            ("misc\\trade\\trade_military.ltx", vec!["supplies_1", "trader"]),
            ("misc\\death_generic.ltx", vec!["base"]),
            ("plugins\\itms.ltx", vec!["list"]),
        ],
        motions: vec!["ak74_idle", "ak74_reload"],
    }
}

fn codes(roster: &Roster, file: &str, lookups: &dyn Lookups) -> Vec<&'static str> {
    let catalog = Catalog::parse(CATALOG);
    let i = roster.index_of_file(file).unwrap();
    issues_of(roster, i, &catalog, lookups, Path::new("D:\\nowhere")).into_iter().map(|x| x.code).collect()
}

fn issues(roster: &Roster, file: &str, lookups: &dyn Lookups) -> Vec<crate::model::Issue> {
    let catalog = Catalog::parse(CATALOG);
    let i = roster.index_of_file(file).unwrap();
    issues_of(roster, i, &catalog, lookups, Path::new("D:\\nowhere"))
}

const NEW: &str = r#"return {
  item = 1,
  id = "sil_diy",
  section = "wpn_sil_diy",
  base = "wpn_ak74",
  name = { rus = "R", eng = "DIY" },
  description = "D",
  icon = { texture = [[ui\ui_icon_equipment]], x = 3, y = 0, w = 2, h = 1 },
  set = { cost = 1200, inv_weight = 0.4, fire_modes = { "single" } },
  hud = { base = "wpn_ak74_hud", set = { hands_position = { 0.02, -0.01, 0.1 } } },
  sections = { { name = "wpn_sil_diy_sounds", base = "wpn_base", set = { snd_shoot = "x" } } },
  craft = { { at = "itm_repairkit", parts = { "bread", "nameless" } } },
  loot = { count = { 0, 1 }, chance = 0.01, by = { bandit = 0.03 } },
  sold = { { by = "trade_military", count = 2, chance = 0.5 } },
  lists = { { file = [[plugins\itms.ltx]], section = "list", key = "items", add = "wpn_sil_diy" } },
}
"#;

const CHANGE: &str = r#"return {
  item = 1,
  edit = "wpn_ak74",
  set = { snd_shoot = [[weapons\ak74_shoot]], cost = 300 },
  unset = { "visual" },
  add = { ammo_class = { "ammo_c" } },
  drop = { ammo_class = "ammo_a" },
  hud = { set = { anm_idle = "ak74_reload" } },
}
"#;

// ---- the file ----------------------------------------------------------------------------------

#[test]
fn a_new_item_reads_every_key_and_keeps_unset_unset() {
    let (item, issues) = load_text(NEW, "items/sil.itemasset").unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(item.kind(), Kind::New);
    assert_eq!(item.game_section("we"), "wpn_sil_diy");
    assert_eq!(item.base, ["wpn_ak74"]);
    assert_eq!(
        item.icon,
        Some(Icon::Cell {
            texture: Some("ui\\ui_icon_equipment".into()),
            x: 3,
            y: 0,
            w: Some(2),
            h: Some(1)
        })
    );
    assert_eq!(item.changes.get("cost"), Some(&SetValue::Number(1200.0)));
    assert_eq!(item.changes.get("fire_modes").map(SetValue::ltx).as_deref(), Some("single"));
    assert_eq!(item.hud.as_ref().unwrap().changes.get("hands_position").unwrap().ltx(), "0.02, -0.01, 0.1");
    assert_eq!(item.craft[0].parts, ["bread", "nameless"]);
    assert_eq!(item.loot.as_ref().unwrap().count, Some(Count::Range(0.0, 1.0)));
    assert_eq!(item.sold[0].file().as_deref(), Some("misc\\trade\\trade_military.ltx"));
    assert_eq!(item.sold[0].sections(), ["supplies_1"]);
    // unset stays unset
    assert!(item.is_abstract.is_none() && item.short_name.is_nil() && item.scene.is_none());
    assert_eq!(item.sold[0].count, Some(2.0));

    let (change, _) = load_text(CHANGE, "items/ak.itemasset").unwrap();
    assert_eq!(change.kind(), Kind::Change);
    assert_eq!(change.game_section("we"), "wpn_ak74");
    assert_eq!(change.changes.drop, [("ammo_class".to_owned(), vec!["ammo_a".to_owned()])]);
    // a file without an id is named by its file; without a section, `<module>_<id>`
    let (plain, _) =
        load_text("return { item = 1, base = \"bread\" }", "items/Dried Fish.itemasset").unwrap();
    assert_eq!(plain.game_section("we"), "we_dried_fish");
}

#[test]
fn the_icon_has_three_forms() {
    let icon = |t: &str| {
        load_text(&format!("return {{ item = 1, base = \"bread\", icon = {t} }}"), "a.itemasset")
            .unwrap()
            .0
            .icon
    };
    assert_eq!(
        icon("\"icons/a.png\""),
        Some(Icon::Picture { file: "icons/a.png".into(), w: None, h: None, plain: true })
    );
    assert_eq!(
        icon("{ file = \"icons/a.png\", w = 2 }"),
        Some(Icon::Picture { file: "icons/a.png".into(), w: Some(2), h: None, plain: false })
    );
    assert_eq!(icon("{ x = 1, y = 2 }"), Some(Icon::Cell { texture: None, x: 1, y: 2, w: None, h: None }));
}

#[test]
fn a_value_outside_what_its_key_takes_is_at_its_key() {
    let (_, issues) = load_text(
        "return { item = 1, base = \"bread\", set = { a = { x = 1 } }, craft = { { at = \"x\", parts = { \"a\", \"b\", \"c\", \"d\", \"e\" } } }, icon = { x = \"left\" }, loot = { chance = 2 } }",
        "a.itemasset",
    )
    .unwrap();
    let at: Vec<String> = issues.iter().map(|i| gz_luadata::path::show(&i.path)).collect();
    assert!(issues.iter().all(|i| i.code == "I427"), "{issues:?}");
    // a chance of 2 is the game's to take; a cell without its place is not
    assert_eq!(at, ["set.a", "icon.x", "icon", "craft[0].parts"]);
    assert!(load_text("return { item = 2 }", "a.itemasset").unwrap_err().contains("newer Grizzly Editor"));
    assert!(load_text("return 5", "a.itemasset").is_err());
}

#[test]
fn a_new_file_is_written_canonically_and_reads_back() {
    let (item, _) = load_text(NEW, "items/sil.itemasset").unwrap();
    let text = write(&item);
    let (back, issues) = load_text(&text, "items/sil.itemasset").unwrap();
    assert!(issues.is_empty());
    assert_eq!(back, item);
    let t = template("Sil DIY");
    let (fresh, issues) = load_text(&t, "items/Sil DIY.itemasset").unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(fresh.kind(), Kind::New);
    assert!(t.contains("\"<module>_sil_diy\""), "{t}");
    assert!(t.is_ascii());
}

#[test]
fn a_save_writes_the_difference_alone() {
    let text =
        "-- mine\nreturn {\n  item = 1,\n  edit = \"wpn_ak74\", -- the rifle\n  set = { cost = 300 },\n}\n";
    let mut doc = LuaDoc::parse(text).unwrap();
    let (before, _) = from_value(&doc.value()).unwrap();
    let mut after = before.clone();
    after.changes.set[0].1 = SetValue::Number(350.0);
    after.changes.unset.push("visual".into());
    doc.apply_diff(&to_value(&before), &to_value(&after), &diff_rules()).unwrap();
    let out = doc.text();
    assert!(
        out.starts_with(
            "-- mine\nreturn {\n  item = 1,\n  edit = \"wpn_ak74\", -- the rifle\n  set = { cost = 350 },\n"
        ),
        "{out}"
    );
    assert!(out.contains("unset = { \"visual\" }"), "{out}");
}

// ---- what an item resolves to -------------------------------------------------------------------

#[test]
fn a_new_item_and_a_change_resolve_in_the_games_order() {
    let r = roster(&[("items/ak.itemasset", CHANGE), ("items/sil.itemasset", NEW)]);
    let catalog = Catalog::parse(CATALOG);
    let sil = r.resolution(r.find("wpn_sil_diy").unwrap(), Some(&catalog)).unwrap();
    assert_eq!(sil.kind, "new");
    assert_eq!(sil.family.as_deref(), Some("weapon"));
    let row = |s: &crate::SectionView, k: &str| s.get(k).cloned().unwrap();
    assert_eq!(row(&sil.main, "cost").from, From::This);
    assert_eq!(row(&sil.main, "cost").value.as_deref(), Some("1200"));
    // the change of its base reaches it: it does not set the key itself (1.2)
    assert_eq!(row(&sil.main, "ammo_class").value.as_deref(), Some("ammo_b,ammo_c"));
    assert_eq!(row(&sil.main, "class").from, From::Base);
    assert_eq!(row(&sil.main, "class").base.as_deref(), Some("wpn_ak74"));
    assert_eq!(row(&sil.main, "inv_name").value.as_deref(), Some("wpn_sil_diy_name"));
    assert_eq!(row(&sil.main, "inv_grid_width").value.as_deref(), Some("2"));
    assert_eq!(row(&sil.main, "hud").value.as_deref(), Some("wpn_sil_diy_hud"));
    assert_eq!(sil.main.parents, ["wpn_ak74"]);
    let hud = sil.hud.as_ref().unwrap();
    assert_eq!(hud.name, "wpn_sil_diy_hud");
    assert_eq!(row(hud, "item_visual").from, From::Base);
    assert_eq!(sil.sections[0].name, "wpn_sil_diy_sounds");
    let files: Vec<&str> = sil.file_patches.iter().map(|p| p.file.as_str()).collect();
    assert!(files.contains(&"misc\\death_generic.ltx") && files.contains(&"misc\\trade\\trade_military.ltx"));
    assert!(
        sil.file_patches
            .iter()
            .any(|p| p.section == "supplies_1" && p.key == "wpn_sil_diy" && p.value == "2, 0.5")
    );
    assert!(sil.file_patches.iter().any(|p| p.file == "plugins\\itms.ltx" && p.op == "add"));

    let ak = r.resolution(r.find("items/ak.itemasset").unwrap(), Some(&catalog)).unwrap();
    assert_eq!(ak.kind, "change");
    assert_eq!(row(&ak.main, "cost").from, From::This);
    assert!(ak.main.get("visual").is_none());
    assert_eq!(
        ak.before.as_ref().unwrap().get("visual").unwrap().value.as_deref(),
        Some("dynamics\\weapons\\wpn_ak74")
    );
    assert_eq!(row(ak.hud.as_ref().unwrap(), "anm_idle").value.as_deref(), Some("ak74_reload"));
    assert_eq!(row(ak.hud_before.as_ref().unwrap(), "anm_idle").value.as_deref(), Some("ak74_idle"));
    // a change reaches what inherits it and does not set it itself
    let kobra = crate::resolve::view(&r.composition.ltx, "wpn_ak74_kobra", &r.composition, "", &[]);
    assert_eq!(row(&kobra, "ammo_class").value.as_deref(), Some("ammo_b,ammo_c"));
    assert_eq!(row(&kobra, "cost").from, From::Item);
}

#[test]
fn a_module_config_and_patch_come_after_the_items() {
    let cfg = ModuleConfigs {
        xms: vec![ConfigFile {
            rel: "gamedata/configs/xms/we.ltx".into(),
            virt: "xms\\we.ltx".into(),
            text: "[wpn_sil_diy]\ncost = 9000\n".into(),
        }],
        patches: vec![ConfigFile {
            rel: "patch/we.ltxp".into(),
            virt: String::new(),
            text: "[wpn_base]\ninv_weight = 7\n".into(),
        }],
        ..ModuleConfigs::default()
    };
    let r = roster_in(Path::new("D:\\nowhere"), &[("items/sil.itemasset", NEW)], &cfg);
    let sil = r.resolution(0, None).unwrap();
    let cost = sil.main.get("cost").unwrap();
    assert_eq!((cost.value.as_deref(), cost.from), (Some("9000"), From::Module));
    assert_eq!(cost.file, "gamedata/configs/xms/we.ltx");
    // the patch of a base reaches what inherits it, but not the item's own line
    assert_eq!(sil.main.get("inv_weight").unwrap().value.as_deref(), Some("0.4"));
    let base = crate::resolve::view(&r.composition.ltx, "wpn_ak74", &r.composition, "", &[]);
    assert_eq!(base.get("inv_weight").unwrap().value.as_deref(), Some("7"));
}

// ---- the check: each code trips, and a clean item does not ---------------------------------------

#[test]
fn a_clean_item_has_no_problem() {
    let r = roster(&[("items/sil.itemasset", NEW), ("items/ak.itemasset", CHANGE)]);
    let g = full_game();
    let found = issues(&r, "items/sil.itemasset", &g);
    assert!(found.is_empty(), "{found:?}");
    assert!(issues(&r, "items/ak.itemasset", &g).is_empty());
}

#[test]
fn identity_problems() {
    let g = full_game();
    let r = roster(&[
        ("a.itemasset", "return { item = 1, set = { cost = 1 } }"),
        ("b.itemasset", "return { item = 1, edit = \"bread\", base = \"bread\" }"),
        ("c.itemasset", "return { item = 1, id = \"9lives\", base = \"bread\" }"),
        ("d.itemasset", "return { item = 1, section = \"Big Gun\", base = \"bread\" }"),
        ("e.itemasset", "return { item = 1, section = \"x\" }"),
        ("f1.itemasset", "return { item = 1, section = \"same\", base = \"bread\" }"),
        ("f2.itemasset", "return { item = 1, section = \"same\", base = \"bread\" }"),
        ("g.itemasset", "return { item = 1, section = \"wpn_ak74\", base = \"bread\" }"),
        ("h.itemasset", "return { item = 1, section = \"h\", base = \"nobody\" }"),
        ("i.itemasset", "return { item = 1, edit = \"nobody\" }"),
        ("j1.itemasset", "return { item = 1, section = \"j1\", base = \"j2\" }"),
        ("j2.itemasset", "return { item = 1, section = \"j2\", base = \"j1\" }"),
    ]);
    assert_eq!(codes(&r, "a.itemasset", &g), ["I401"]);
    assert_eq!(codes(&r, "b.itemasset", &g), ["I402"]);
    assert!(codes(&r, "c.itemasset", &g).contains(&"I403"));
    assert!(codes(&r, "d.itemasset", &g).contains(&"I404"));
    assert_eq!(codes(&r, "e.itemasset", &g), ["I405"]);
    assert!(codes(&r, "f1.itemasset", &g).contains(&"I406"));
    assert!(codes(&r, "f2.itemasset", &g).contains(&"I406"));
    assert_eq!(codes(&r, "g.itemasset", &g), ["I407"]);
    assert_eq!(codes(&r, "h.itemasset", &g), ["I408"]);
    assert_eq!(codes(&r, "i.itemasset", &g), ["I409"]);
    assert_eq!(codes(&r, "j1.itemasset", &g), ["I410"]);
    assert_eq!(codes(&r, "j2.itemasset", &g), ["I410"]);
    // a base that is another item of the module, whatever the order of their paths
    let r = roster(&[
        ("a.itemasset", "return { item = 1, section = \"a\", base = \"z\" }"),
        ("z.itemasset", "return { item = 1, section = \"z\", base = \"bread\" }"),
    ]);
    assert!(codes(&r, "a.itemasset", &g).is_empty());
}

#[test]
fn what_the_game_stops_without_and_values_of_a_type() {
    let g = full_game();
    let r = roster(&[
        // a base that works without a key shows the game does not stop without it: taking it away does
        (
            "a.itemasset",
            "return { item = 1, section = \"a\", base = \"bread\", unset = { \"inv_weight\", \"class\" } }",
        ),
        ("n.itemasset", "return { item = 1, section = \"n\", base = \"nameless\" }"),
        ("t.itemasset", "return { item = 1, section = \"t\", base = \"nameless\", abstract = true }"),
        (
            "b.itemasset",
            "return { item = 1, section = \"b\", base = \"wpn_ak74\", set = { cost = 1.5, fire_modes = \"burst\" }, hud = { base = \"wpn_ak74_hud\", set = { hands_position = { 1, 2 } } } }",
        ),
        (
            "c.itemasset",
            "return { item = 1, section = \"c\", base = \"wpn_bare\", hud = { set = { anm_idle = \"x\" } } }",
        ),
    ]);
    let a = issues(&r, "a.itemasset", &g);
    let required: Vec<&str> = a.iter().filter(|i| i.code == "I411").map(|i| i.message.as_str()).collect();
    assert_eq!(required.len(), 2, "{a:?}");
    assert!(required[0].contains("'inv_weight'") && required[1].contains("'class'"), "{required:?}");
    assert_eq!(gz_luadata::path::show(&a[0].path), "base");
    assert!(codes(&r, "n.itemasset", &g).is_empty());
    // a template is no item of the game: nothing is required of it
    assert!(codes(&r, "t.itemasset", &g).is_empty());
    let b = issues(&r, "b.itemasset", &g);
    let bad: Vec<String> =
        b.iter().filter(|i| i.code == "I412").map(|i| gz_luadata::path::show(&i.path)).collect();
    assert_eq!(bad, ["set.cost", "set.fire_modes", "hud.set.hands_position"]);
    assert!(b.iter().all(|i| i.severity == Severity::Warning));
    // a HUD section made of nothing: a warning, and what the game stops without
    let c = codes(&r, "c.itemasset", &g);
    assert!(c.contains(&"I429") && c.contains(&"I411"), "{c:?}");
}

#[test]
fn what_a_key_names_is_looked_up() {
    let g = full_game();
    let text = r#"return { item = 1, section = "n", base = "wpn_ak74",
        set = { visual = [[nowhere\gun]], snd_shoot2 = "none", snd_reload = "$no_sound", icons_texture = [[no\tex]],
                flame_particles = [[no\fx]], ammo_class = "ammo_a, no_ammo", inv_name = "st_nothing" },
        hud = { base = "wpn_ak74_hud", set = { anm_idle = "ak74_idle, ak74_nothing" } } }"#;
    let r = roster(&[("n.itemasset", text)]);
    let found = issues(&r, "n.itemasset", &g);
    let pairs: Vec<(&str, String)> =
        found.iter().map(|i| (i.code, gz_luadata::path::show(&i.path))).collect();
    for want in [
        ("I413", "set.visual"),
        ("I414", "set.snd_shoot2"),
        ("I415", "set.icons_texture"),
        ("I416", "set.flame_particles"),
        ("I417", "set.ammo_class"),
        ("I418", "set.inv_name"),
        ("I419", "hud.set.anm_idle"),
    ] {
        assert!(pairs.iter().any(|(c, p)| *c == want.0 && p == want.1), "{want:?} in {pairs:?}");
    }
    // `$no_sound` is no file
    assert!(!pairs.iter().any(|(_, p)| p == "set.snd_reload"));
    // without a game nothing is looked up, and the check says so once
    let unlinked = codes(&r, "n.itemasset", &NoGame);
    assert!(unlinked.contains(&"I426") && !unlinked.contains(&"I413"), "{unlinked:?}");
    // a section a key names is the composition's: it needs no game
    assert!(unlinked.contains(&"I417"));
}

#[test]
fn texts_icons_and_where_it_is_found() {
    let g = full_game();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("items")).unwrap();
    picture(&dir.path().join("items").join("good.tga"), 100, 50);
    std::fs::write(dir.path().join("items").join("bad.png"), b"not a picture").unwrap();
    let files = [
        (
            "items/t.itemasset",
            "return { item = 1, section = \"t\", base = \"bread\", name = { eng = \"Star \u{2605}\" } }",
        ),
        (
            "items/p.itemasset",
            "return { item = 1, section = \"p\", base = \"bread\", icon = \"missing.png\" }",
        ),
        ("items/q.itemasset", "return { item = 1, section = \"q\", base = \"bread\", icon = \"bad.png\" }"),
        (
            "items/o.itemasset",
            "return { item = 1, section = \"o\", base = \"bread\", icon = \"../../outside.png\" }",
        ),
        ("items/g.itemasset", "return { item = 1, section = \"g\", base = \"bread\", icon = \"good.tga\" }"),
        (
            "items/r.itemasset",
            "return { item = 1, section = \"r\", base = \"bread\", icon = { texture = [[ui\\ui_icon_equipment]], x = 40, y = 20, w = 2, h = 2 } }",
        ),
        (
            "items/w.itemasset",
            "return { item = 1, section = \"w\", base = \"bread\", craft = { { at = \"no_kit\", parts = { \"bread\", \"no_part\" } } }, sold = { { by = \"trade_nobody\" }, { by = \"trade_military\", supplies = \"supplies_9\" } }, loot = { by = { bandit = 0.1, stalker_veteran = 0.1, monolith = 0.1 } }, lists = { { file = \"no\\\\file.ltx\", section = \"x\" } } }",
        ),
    ];
    let r = roster_in(dir.path(), &files, &ModuleConfigs::default());
    let check = |f: &str| {
        let catalog = Catalog::parse(CATALOG);
        let i = r.index_of_file(f).unwrap();
        issues_of(&r, i, &catalog, &g, dir.path())
    };
    let t = check("items/t.itemasset");
    assert_eq!(
        t.iter().map(|i| (i.code, gz_luadata::path::show(&i.path))).collect::<Vec<_>>(),
        [("I420", "name.eng".to_owned())]
    );
    for f in ["items/p.itemasset", "items/q.itemasset", "items/o.itemasset"] {
        assert_eq!(check(f).iter().map(|i| i.code).collect::<Vec<_>>(), ["I421"], "{f}");
    }
    assert!(check("items/g.itemasset").is_empty());
    assert_eq!(r.plan.placed.len(), 1);
    assert_eq!((r.plan.placed[0].w, r.plan.placed[0].h), (2, 1));
    assert_eq!(check("items/r.itemasset").iter().map(|i| i.code).collect::<Vec<_>>(), ["I422"]);
    let w = check("items/w.itemasset");
    let w: Vec<(&str, String)> = w.iter().map(|i| (i.code, gz_luadata::path::show(&i.path))).collect();
    assert_eq!(
        w,
        [
            ("I423", "craft[0].at".to_owned()),
            ("I423", "craft[0].parts".to_owned()),
            ("I424", "sold[0]".to_owned()),
            ("I424", "sold[1]".to_owned()),
            ("I424", "lists[0].file".to_owned()),
            ("I425", "loot.by.monolith".to_owned()),
            ("I430", "loot".to_owned()),
        ]
    );
    // the picture's item resolves to the module's atlas
    let gi = r.resolution(r.find("g").unwrap(), None).unwrap();
    assert_eq!(gi.main.get("icons_texture").unwrap().value.as_deref(), Some("ui\\we_icons"));
}

#[test]
fn a_config_of_the_module_that_does_not_compose_is_said() {
    let cfg = ModuleConfigs {
        xms: vec![ConfigFile {
            rel: "gamedata/configs/xms/a.ltx".into(),
            virt: "xms\\a.ltx".into(),
            text: "[x]:nobody\n".into(),
        }],
        ..ModuleConfigs::default()
    };
    let r = roster_in(
        Path::new("D:\\nowhere"),
        &[
            ("a.itemasset", "return { item = 1, edit = \"bread\" }"),
            ("b.itemasset", "return { item = 1, edit = \"bread\" }"),
        ],
        &cfg,
    );
    assert_eq!(r.composition.problems.len(), 1);
    assert_eq!(r.composition.problems[0].0, "gamedata/configs/xms/a.ltx");
    // said once, on the first item
    assert_eq!(codes(&r, "a.itemasset", &full_game()), ["I428"]);
    assert!(codes(&r, "b.itemasset", &full_game()).is_empty());
}

#[test]
fn the_build_refuses_on_the_first_error_and_ships_the_atlas() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("items")).unwrap();
    picture(&dir.path().join("items").join("a.tga"), 50, 50);
    picture(&dir.path().join("items").join("b.tga"), 100, 100);
    let a = "-- a\nreturn {\n  item = 1,\n  section = \"a\",\n  base = \"bread\",\n  icon = \"a.tga\",\n}\n";
    let b =
        "return { item = 1, section = \"b\", base = \"bread\", icon = { file = \"b.tga\", w = 1, h = 1 } }\n";
    std::fs::write(dir.path().join("items").join("a.itemasset"), a).unwrap();
    std::fs::write(dir.path().join("items").join("b.itemasset"), b).unwrap();
    let r = roster_in(
        dir.path(),
        &[("items/a.itemasset", a), ("items/b.itemasset", b)],
        &ModuleConfigs::default(),
    );
    assert!(crate::build_errors(&r, &Catalog::parse(CATALOG), dir.path()).is_empty());
    let p = crate::publication(dir.path(), &r);
    assert!(p.errors.is_empty(), "{:?}", p.errors);
    let paths: Vec<&str> = p.files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(paths, ["gamedata/textures/ui/we_icons.dds", "items/a.itemasset", "items/b.itemasset"]);
    assert!(p.sources.contains("items/a.tga") && p.sources.contains("items/a.itemasset"));
    let gz_build::FileFrom::Bytes(text) = &p.files[1].from else { panic!("text") };
    let text = std::str::from_utf8(text).unwrap();
    assert!(
        text.starts_with("-- a\nreturn {\n  item = 1,\n  section = \"a\",\n  base = \"bread\",\n  icon = {"),
        "{text}"
    );
    let v = gz_luadata::parse(text).unwrap();
    assert_eq!(v.get("icon").and_then(|i| i.get("texture")).and_then(Value::as_str), Some("ui\\we_icons"));
    // the same pictures: the same atlas, from the cache the second time
    let again = crate::publication(dir.path(), &r);
    let made = |p: &gz_build::Publication| match &p.files[0].from {
        gz_build::FileFrom::Made(path) => std::fs::read(path).unwrap(),
        _ => Vec::new(),
    };
    assert_eq!(made(&p), made(&again));
    // a broken item refuses the build, with its first error
    let broken = roster_in(
        dir.path(),
        &[("items/x.itemasset", "return { item = 1, section = \"x\" }")],
        &ModuleConfigs::default(),
    );
    let errors = crate::build_errors(&broken, &Catalog::parse(CATALOG), dir.path());
    assert_eq!(errors[0].1.code, "I405");
}

#[test]
fn the_game_makes_what_the_contract_says() {
    let text = r#"return { item = 1, section = "k", base = "wpn_ak74", name = "N", set = { inv_name = "st_x" },
        icon = { x = 1, y = 2 }, hud = { set = { anm_idle = "a" } },
        craft = { { at = "itm_repairkit", parts = { "bread" } }, { at = "ITM_repairkit", count = 2, parts = { "bread", "nameless" } } },
        scene = { like = "Vodka", sound = "s", hud = { base = "wpn_ak74_hud", set = { x = 1 } } },
        loot = { chance = 0.5 }, sold = { by = "trade_military", count = 3 } }"#;
    let r = roster(&[
        ("k.itemasset", text),
        (
            "bad.itemasset",
            "return { item = 1, section = \"bad\", base = \"bread\", set = { [\"a b\"] = 1 } }",
        ),
    ]);
    let k = r.resolution(r.find("k").unwrap(), None).unwrap();
    let val = |s: &crate::SectionView, key: &str| s.get(key).and_then(|x| x.value.clone());
    // the name's string ids win over a key `set` names; the short name is the name's without one
    assert_eq!(val(&k.main, "inv_name").as_deref(), Some("k_name"));
    assert_eq!(val(&k.main, "inv_name_short").as_deref(), Some("k_name"));
    // an icon without a texture is the game's atlas: the base's icons_texture goes, w and h stay
    assert!(k.main.get("icons_texture").is_none());
    assert_eq!(val(&k.main, "inv_grid_x").as_deref(), Some("1"));
    assert!(k.main.get("inv_grid_width").is_none());
    // a HUD without a base sits over the HUD its bases name
    let hud = k.hud.as_ref().unwrap();
    assert_eq!((hud.name.as_str(), hud.parents.as_slice()), ("k_hud", &["wpn_ak74_hud".to_owned()][..]));
    assert_eq!(val(hud, "item_visual").as_deref(), Some("dynamics\\weapons\\wpn_ak74_hud"));
    // the recipes, numbered on from what [xms_craft] holds, no part padded
    let craft = crate::resolve::view(&r.composition.ltx, "xms_craft", &r.composition, "", &[]);
    assert_eq!(val(&craft, "000001").as_deref(), Some("itm_repairkit, k, 1, bread"));
    assert_eq!(val(&craft, "000002").as_deref(), Some("itm_repairkit, k, 2, bread, nameless"));
    // the scene: its config's section over the item it is like, the names the scene script reads
    let lines: Vec<(&str, &str, &str, &str)> = k
        .file_patches
        .iter()
        .map(|p| (p.file.as_str(), p.section.as_str(), p.key.as_str(), p.value.as_str()))
        .collect();
    assert!(lines.contains(&("items\\items\\animations_settings.ltx", "k", "snd", "s")), "{lines:?}");
    assert!(lines.contains(&("items\\items\\animations_settings.ltx", "k", "anm", "item_ea_k")));
    assert!(lines.contains(&("misc\\death_items_by_communities.ltx", "base", "k", "0.5")));
    assert!(lines.contains(&("misc\\trade\\trade_military.ltx", "supplies_1", "k", "3, 1")));
    assert!(!lines.iter().any(|l| l.1 == "item_count"));
    let scene_hud = crate::resolve::view(&r.composition.ltx, "item_ea_k_hud", &r.composition, "", &[]);
    assert_eq!(scene_hud.parents, ["wpn_ak74_hud"]);
    assert_eq!(val(&scene_hud, "x").as_deref(), Some("1"));
    // a death loot without a count never drops
    assert!(codes(&r, "k.itemasset", &full_game()).contains(&"I430"));
    // any malformed part leaves the whole item out
    let bad = r.resolution(r.find("bad.itemasset").unwrap(), None).unwrap();
    assert!(bad.left_out && !bad.main.exists);
    assert_eq!(codes(&r, "bad.itemasset", &full_game()), ["I427"]);
}

#[test]
fn a_template_needs_no_base_and_a_change_refuses_a_hud_base() {
    let r = roster(&[
        ("t.itemasset", "return { item = 1, section = \"tpl\", abstract = true, set = { cost = 1 } }"),
        ("e.itemasset", "return { item = 1, edit = \"wpn_ak74\", hud = { base = \"x\" } }"),
        ("h.itemasset", "return { item = 1, section = \"wpn_ak74\", base = \"bread\" }"),
        ("hh.itemasset", "return { item = 1, section = \"wpn_ak74\", base = \"bread\", hud = { } }"),
    ]);
    let t = r.resolution(r.find("tpl").unwrap(), None).unwrap();
    assert!(t.main.exists && t.main.parents.is_empty());
    assert_eq!(codes(&r, "e.itemasset", &full_game()), ["I427"]);
    let r = roster(&[
        ("n.itemasset", "return { item = 1, section = \"wpn_ak74\", base = \"bread\", hud = {} }"),
        ("m.itemasset", "return { item = 1, section = \"wpn\", base = \"bread\", hud = {} }"),
    ]);
    assert_eq!(codes(&r, "n.itemasset", &full_game()), ["I407"]);
    // `<section>_hud` there already refuses a new item with a hud
    let r = roster(&[
        ("w.itemasset", "return { item = 1, section = \"wpn_ak74\", base = \"bread\" }"),
        ("x.itemasset", "return { item = 1, section = \"wpn_ak74x\", base = \"bread\", hud = {} }"),
    ]);
    assert!(codes(&r, "x.itemasset", &full_game()).iter().all(|c| *c != "I407"));
}
