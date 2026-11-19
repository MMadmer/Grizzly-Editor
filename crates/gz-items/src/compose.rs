//! The module's items and tables laid over the linked game's configs, in the game's order
//! (`docs/assets/ITEM.md`, "Order"): the game's tree with the module's files that replace a game config, then
//! the items - new sections first (a base that is another item of the module before the item
//! made from it, else in path order), then the changes in path order -, then the module's traders,
//! loot tables and recipes (each kind in path order), then the module's `configs\xms\*.ltx`, then
//! its `patch\*.ltxp`. Every change reaches what inherits (`gz-ltx`'s `tree.rs`). An item's use
//! scene and a table's rows are `#file` patches of the configs they are about ([`FilePatch`]), as
//! the game makes them.
//!
//! The files of the tree are named so their origin can be told: the game's as the game names
//! them (`configs\...`), a module's config as `@` and its path in the game (`@configs\xms\...`),
//! an item by its project path (`items/wpn.itemasset`), a patch by its project path.

use gz_ltx::{Builder, Dialect, Directive, FilePatch, Loader, Ltx, Op, ParsedFile, config_key, parse_file};
use gz_luadata::{Limits, LuaDoc, Seg};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::atlas::Plan;
use crate::model::{Changes, Count, Icon, Issue, Item, Kind, Scene, valid_section};
use crate::tables::{Rows, TableFile, trader_file};

/// The death loot's counts: `[item_count]`.
pub const DEATH_COUNTS: &str = "misc\\death_generic.ltx";
/// The death loot's chances: `[base]`, which every community and rank inherits, and theirs.
pub const DEATH_CHANCES: &str = "misc\\death_items_by_communities.ltx";
/// The use scenes.
pub const SCENES: &str = "items\\items\\animations_settings.ltx";

pub const NEITHER: &str = "I401";
pub const BOTH: &str = "I402";
pub const BAD_ID: &str = "I403";
pub const BAD_SECTION: &str = "I404";
pub const NO_BASE: &str = "I405";
pub const TAKEN: &str = "I407";
pub const UNKNOWN_BASE: &str = "I408";
pub const UNKNOWN_EDIT: &str = "I409";
pub const LOOP: &str = "I410";
pub const BAD_CONFIG: &str = "I428";
pub const HUD_WITHOUT_BASE: &str = "I429";

/// A config file of the module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    /// Its project path, with `/`.
    pub rel: String,
    /// Its path in the game, as [`config_key`] writes it under `configs` (`xms\balance.ltx`).
    pub virt: String,
    pub text: String,
}

/// The module's config files as the game composes them.
#[derive(Clone, Debug, Default)]
pub struct ModuleConfigs {
    /// Files at a path of a game config: they replace it (read here as a merge over it).
    pub replaced: Vec<ConfigFile>,
    /// `configs\xms\*.ltx`, by name.
    pub xms: Vec<ConfigFile>,
    /// `patch\*.ltxp`, by name.
    pub patches: Vec<ConfigFile>,
    /// Every config of the module: what an include of one of them finds.
    pub all: Vec<ConfigFile>,
}

/// The module's configs, by the name the tree gives them.
struct ModuleFiles {
    files: FxHashMap<String, ParsedFile>,
    layer: u16,
}

impl Loader for ModuleFiles {
    fn file(&self, path: &str) -> Option<&ParsedFile> {
        self.files.get(&config_key(path.trim_start_matches('@')))
    }

    fn layer(&self, _path: &str) -> u16 {
        self.layer
    }
}

/// What the composition did with one item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// The section it makes or changes.
    pub section: String,
    /// The HUD section it makes or changes.
    pub hud: Option<String>,
    /// The other sections it makes or changes.
    pub owned: Vec<String>,
    /// What it says of other config files.
    pub file_patches: Vec<FilePatch>,
    /// The game leaves it out: the problem says why.
    pub left_out: bool,
    /// What the composition found: a base that is not there, a loop, a section taken.
    pub issues: Vec<Issue>,
}

/// What the composition did with one table file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppliedTable {
    /// What it says of other config files.
    pub file_patches: Vec<FilePatch>,
    /// The game leaves it out: its own shape is not one the game reads.
    pub left_out: bool,
}

/// The game's tree with the module laid over it.
#[derive(Clone, Debug, Default)]
pub struct Composition {
    /// Shared: the scene's settings read this very tree.
    pub ltx: std::sync::Arc<Ltx>,
    /// Per item of the roster, in its order.
    pub applied: Vec<Applied>,
    /// Per table file of the roster, in its order.
    pub tables: Vec<AppliedTable>,
    /// The module's `#file` patches (its `.ltxp` files') and the items', in order.
    pub file_patches: Vec<FilePatch>,
    /// A module config that does not compose: `(file, why)`.
    pub problems: Vec<(String, String)>,
    /// `@<path in the game>` -> the module file's project path.
    pub module_files: FxHashMap<String, String>,
}

impl Composition {
    /// Where a value of the tree was written: `(origin, file)`, the file project-relative for
    /// the module's.
    #[must_use]
    pub fn origin_of(&self, file: u32) -> (Origin, String) {
        let name = self.ltx.files().get(file as usize).map(|f| f.to_string()).unwrap_or_default();
        if let Some(rel) = self.module_files.get(&name) {
            return (Origin::Module, rel.clone());
        }
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".itemasset") {
            (Origin::Item, name)
        } else if lower.ends_with(".ltxp") || gz_project::asset_kind::TableKind::of_file(&lower).is_some() {
            (Origin::Module, name)
        } else {
            (Origin::Game, name)
        }
    }
}

/// Where a value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The game's configs (or a module installed before).
    Game,
    /// The module's own configs or patches.
    Module,
    /// An item asset (this one or another one of the module).
    Item,
}

/// The line a document's path stands at (1-based; 0 without one).
pub struct Lines(Option<LuaDoc>);

impl std::fmt::Debug for Lines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Lines(..)" } else { "Lines(none)" })
    }
}

impl Lines {
    #[must_use]
    pub fn of(text: Option<&str>) -> Self {
        Self(text.and_then(|t| LuaDoc::parse_with(t, "item", Limits::asset()).ok()))
    }

    #[must_use]
    pub fn at(&self, path: &[Seg]) -> u32 {
        self.0.as_ref().map_or(0, |d| d.nearest_span(path).line)
    }
}

fn d(op: Op, line: u32) -> Directive {
    Directive::new(op, line)
}

/// The directives of `set`, `unset`, `add`, `drop` under `at` (a path of the file), in that order.
fn changes(out: &mut Vec<Directive>, c: &Changes, at: &[Seg], lines: &Lines) {
    let line = |key: &str, sub: Option<&str>| {
        let mut p = at.to_vec();
        p.push(Seg::key(key));
        if let Some(s) = sub {
            p.push(Seg::key(s));
        }
        lines.at(&p)
    };
    for (k, v) in &c.set {
        out.push(d(Op::Set { key: k.clone(), value: v.ltx() }, line("set", Some(k))));
    }
    for k in &c.unset {
        out.push(d(Op::Delete(k.clone()), line("unset", None)));
    }
    // a list a change adds to or drops from is rewritten joined by `,`, as `+key` does
    for (k, m) in &c.add {
        out.push(d(Op::Append { key: k.clone(), value: m.join(",") }, line("add", Some(k))));
    }
    for (k, m) in &c.drop {
        out.push(d(Op::Remove { key: k.clone(), value: m.join(",") }, line("drop", Some(k))));
    }
}

/// The game's `number_text`: a whole number as one, else `%.15g`.
fn number(n: f64) -> String {
    gz_luadata::number_to_text(n)
}

/// A section an item makes or changes, with its directives.
struct SectionPlan {
    make: bool,
    name: String,
    parents: Vec<String>,
    ops: Vec<Directive>,
    at: Vec<Seg>,
}

/// One block of another config's (or the system config's, `file` empty) directives.
struct ListBlock {
    file: String,
    section: String,
    parents: Option<Vec<String>>,
    ops: Vec<Directive>,
    line: u32,
}

/// What the game makes of an item, before it looks at the config (`PlanItem`).
struct ItemPlan {
    section: String,
    edit: bool,
    parents: Vec<String>,
    own: Vec<Directive>,
    hud: bool,
    hud_parents_given: bool,
    hud_parents: Vec<String>,
    hud_ops: Vec<Directive>,
    sections: Vec<SectionPlan>,
    /// The use scene: blocks of the scenes' config and, for its HUD, of the system config.
    lists: Vec<ListBlock>,
}

fn lower(list: &[String]) -> Vec<String> {
    list.iter().map(|s| s.to_ascii_lowercase()).collect()
}

/// The game's plan of an item, or why it leaves the item out.
fn plan_item(item: &Item, module: &str, plan: &Plan, lines: &Lines) -> Result<ItemPlan, Issue> {
    let section = item.game_section(module);
    let mut p = ItemPlan {
        section: section.clone(),
        edit: false,
        parents: Vec::new(),
        own: Vec::new(),
        hud: false,
        hud_parents_given: false,
        hud_parents: Vec::new(),
        hud_ops: Vec::new(),
        sections: Vec::new(),
        lists: Vec::new(),
    };
    match item.kind() {
        Kind::Both => {
            return Err(Issue::error(
                BOTH,
                vec![Seg::key("edit")],
                "a change names its section with edit alone: id, section, base and abstract make a new item",
            ));
        }
        Kind::Change => p.edit = true,
        Kind::New | Kind::Neither => {
            let name = item.id_or_stem();
            // the id matters where it is written, or where it makes the section
            if (item.id.is_some() || item.section.is_none()) && !gz_characters::valid_id(&name) {
                let at = if item.id.is_some() { vec![Seg::key("id")] } else { Vec::new() };
                return Err(Issue::error(
                    BAD_ID,
                    at,
                    format!("the id '{name}' is not a-z, 0-9 and _, up to 48, not starting with a digit"),
                ));
            }
            if !valid_section(&section) {
                let at = if item.section.is_some() { vec![Seg::key("section")] } else { Vec::new() };
                return Err(Issue::error(
                    BAD_SECTION,
                    at,
                    format!("the section [{section}] is not a-z, 0-9, _, . and -, under 64 characters"),
                ));
            }
            p.parents = lower(&item.base);
            if p.parents.is_empty() && !item.is_abstract() {
                let code = if item.kind() == Kind::Neither { NEITHER } else { NO_BASE };
                return Err(Issue::error(
                    code,
                    Vec::new(),
                    "a new item starts from a base: write base (or edit, to change an item that exists)",
                ));
            }
        }
    }
    changes(&mut p.own, &item.changes, &[], lines);
    // a template is its section and nothing else
    if item.is_abstract() {
        return Ok(p);
    }

    // the game's own string ids, after `set`: they win over a key `set` names
    let has = |v: &gz_luadata::Value| !v.is_nil();
    if has(&item.name) {
        p.own.push(d(
            Op::Set { key: "inv_name".into(), value: format!("{section}_name") },
            lines.at(&[Seg::key("name")]),
        ));
    }
    if has(&item.short_name) {
        p.own.push(d(
            Op::Set { key: "inv_name_short".into(), value: format!("{section}_short") },
            lines.at(&[Seg::key("short_name")]),
        ));
    } else if has(&item.name) {
        p.own.push(d(
            Op::Set { key: "inv_name_short".into(), value: format!("{section}_name") },
            lines.at(&[Seg::key("name")]),
        ));
    }
    if has(&item.description) {
        p.own.push(d(
            Op::Set { key: "description".into(), value: format!("{section}_descr") },
            lines.at(&[Seg::key("description")]),
        ));
    }

    let icon_line = lines.at(&[Seg::key("icon")]);
    let cell = match &item.icon {
        None => None,
        Some(Icon::Cell { texture, x, y, w, h }) => Some((texture.clone(), *x, *y, *w, *h)),
        // a picture is the built module's rectangle of the atlas; unbuilt, the game keeps the icon
        Some(Icon::Picture { .. }) => {
            plan.of_file(&item.file).map(|c| (Some(plan.texture.clone()), c.x, c.y, Some(c.w), Some(c.h)))
        }
    };
    if let Some((texture, x, y, w, h)) = cell {
        p.own.push(d(
            match texture {
                Some(t) => Op::Set { key: "icons_texture".into(), value: t },
                None => Op::Delete("icons_texture".into()),
            },
            icon_line,
        ));
        p.own.push(d(Op::Set { key: "inv_grid_x".into(), value: x.to_string() }, icon_line));
        p.own.push(d(Op::Set { key: "inv_grid_y".into(), value: y.to_string() }, icon_line));
        for (k, v) in [("inv_grid_width", w), ("inv_grid_height", h)] {
            if let Some(v) = v {
                p.own.push(d(Op::Set { key: k.into(), value: v.to_string() }, icon_line));
            }
        }
    }

    if let Some(h) = &item.hud {
        let at = [Seg::key("hud")];
        p.hud_parents_given = !h.base.is_empty();
        p.hud_parents = lower(&h.base);
        changes(&mut p.hud_ops, &h.changes, &at, lines);
        p.hud = true;
        if !p.edit && item.changes.get("hud").is_none() {
            p.own.push(d(Op::Set { key: "hud".into(), value: format!("{section}_hud") }, lines.at(&at)));
        }
    }
    for (i, s) in item.sections.iter().enumerate() {
        let at = vec![Seg::key("sections"), Seg::Index(i)];
        let make = !s.name.is_empty();
        let name = if make { s.name.to_ascii_lowercase() } else { s.edit.to_ascii_lowercase() };
        if make && !valid_section(&name) {
            let mut path = at.clone();
            path.push(Seg::key("name"));
            return Err(Issue::error(
                BAD_SECTION,
                path,
                format!("the section [{name}] is not a-z, 0-9, _, . and -, under 64 characters"),
            ));
        }
        let mut ops = Vec::new();
        changes(&mut ops, &s.changes, &at, lines);
        p.sections.push(SectionPlan { make, name, parents: lower(&s.base), ops, at });
    }

    // the use scene: lines of the scenes' config
    let block = |file: &str, sec: &str, parents: Option<Vec<String>>, line: u32| ListBlock {
        file: file.to_owned(),
        section: sec.to_owned(),
        parents,
        ops: Vec::new(),
        line,
    };
    let set = |key: &str, value: String, line: u32| d(Op::Set { key: key.to_owned(), value }, line);
    if let Some(scene) = &item.scene {
        let line = lines.at(&[Seg::key("scene")]);
        let (like, keys, hud, own) = match scene {
            Scene::Like(l) => (Some(l.clone()), &[][..], None, &[][..]),
            Scene::Table { like, keys, hud, set } => {
                (like.clone(), keys.as_slice(), hud.as_ref(), set.as_slice())
            }
        };
        let mut b = block(SCENES, &section, like.map(|l| vec![l.to_ascii_lowercase()]), line);
        // the keys the game's scene script reads, under the names an author writes
        for (from, to) in [
            ("sound", "snd"),
            ("camera", "cam"),
            ("time", "tm"),
            ("helmet", "helm"),
            ("fit", "fit"),
            ("anm", "anm"),
            ("hud_anm", "hud_anm"),
        ] {
            if let Some((_, v)) = keys.iter().find(|(k, _)| k == from) {
                b.ops.push(set(to, v.ltx(), line));
            }
        }
        // the scene's other keys, as its section holds them
        for (k, v) in own {
            b.ops.push(d(
                Op::Set { key: k.clone(), value: v.ltx() },
                lines.at(&[Seg::key("scene"), Seg::key("set"), Seg::key(k)]),
            ));
        }
        if let Some(h) = hud {
            if !keys.iter().any(|(k, _)| k == "anm") {
                b.ops.push(set("anm", format!("item_ea_{section}"), line));
            }
            p.lists.push(b);
            // the scene plays <anm>_hud of the system config
            let parents = (!h.base.is_empty()).then(|| lower(&h.base));
            let mut hb = block("", &format!("item_ea_{section}_hud"), parents, line);
            changes(&mut hb.ops, &h.changes, &[Seg::key("scene"), Seg::key("hud")], lines);
            p.lists.push(hb);
        } else {
            p.lists.push(b);
        }
    }
    Ok(p)
}

/// A section's key as it stands.
fn value_of(b: &Builder, section: &str, key: &str) -> String {
    b.section(section).and_then(|s| s.get(key)).and_then(|it| it.value.as_deref()).unwrap_or("").to_owned()
}

/// The section a change's `hud` names once its own `set` is applied.
fn hud_of_change(b: &Builder, p: &ItemPlan) -> String {
    let mut hud = value_of(b, &p.section, "hud");
    for x in &p.own {
        if let Op::Set { key, value } = &x.op
            && key == "hud"
        {
            hud.clone_from(value);
        }
    }
    hud
}

enum Ready {
    Go,
    /// A base is not there yet: another item of the module may make it.
    Wait(Issue),
    Refuse(Issue),
}

/// Whether the game applies the item now (`ready`).
fn ready(b: &Builder, p: &ItemPlan) -> Ready {
    let missing = |parents: &[String]| parents.iter().find(|x| !b.has(x)).cloned();
    if !p.edit {
        if b.has(&p.section) {
            return Ready::Refuse(Issue::error(
                TAKEN,
                Vec::new(),
                format!("the section [{}] exists already: write it as a change, with edit", p.section),
            ));
        }
        if p.hud && b.has(&format!("{}_hud", p.section)) {
            return Ready::Refuse(Issue::error(
                TAKEN,
                vec![Seg::key("hud")],
                format!("the section [{}_hud] exists already", p.section),
            ));
        }
    } else if !b.has(&p.section) {
        return Ready::Refuse(Issue::error(
            UNKNOWN_EDIT,
            vec![Seg::key("edit")],
            format!("it changes [{}], which is not there", p.section),
        ));
    } else if p.hud {
        let hud = hud_of_change(b, p);
        if hud.is_empty() || !b.has(&hud) {
            return Ready::Refuse(Issue::error(
                UNKNOWN_EDIT,
                vec![Seg::key("hud")],
                format!("it changes the hud of [{}], which names no hud section", p.section),
            ));
        }
    }
    for s in &p.sections {
        if s.make && (b.has(&s.name) || s.name == p.section) {
            return Ready::Refuse(Issue::error(
                TAKEN,
                s.at.clone(),
                format!(
                    "the section [{}] exists already: change it with {{ edit = \"{}\" }}",
                    s.name, s.name
                ),
            ));
        }
        if !s.make && !b.has(&s.name) {
            return Ready::Refuse(Issue::error(
                UNKNOWN_EDIT,
                s.at.clone(),
                format!("it changes [{}], which is not there", s.name),
            ));
        }
    }
    if !p.edit
        && let Some(m) = missing(&p.parents)
    {
        return Ready::Wait(Issue::error(
            UNKNOWN_BASE,
            vec![Seg::key("base")],
            format!("an unknown base [{m}]"),
        ));
    }
    if p.hud
        && p.hud_parents_given
        && let Some(m) = missing(&p.hud_parents)
    {
        return Ready::Wait(Issue::error(
            UNKNOWN_BASE,
            vec![Seg::key("hud"), Seg::key("base")],
            format!("an unknown base [{m}] of its hud"),
        ));
    }
    // a row may start from what the item itself makes before it: its own section, its hud, an
    // earlier row
    let mut made: Vec<String> = Vec::new();
    if !p.edit {
        made.push(p.section.clone());
        if p.hud {
            made.push(format!("{}_hud", p.section));
        }
    }
    for s in p.sections.iter().filter(|s| s.make) {
        let unknown = s.parents.iter().find(|x| !b.has(x) && !made.contains(x)).cloned();
        made.push(s.name.clone());
        if let Some(m) = unknown {
            let mut at = s.at.clone();
            at.push(Seg::key("base"));
            return Ready::Wait(Issue::error(
                UNKNOWN_BASE,
                at,
                format!("an unknown base [{m}] of [{}]", s.name),
            ));
        }
    }
    Ready::Go
}

/// One section's changes under a header that makes it (with `parents`) where it is missing.
fn apply_to(
    b: &mut Builder,
    source: &str,
    section: &str,
    parents: Option<Vec<String>>,
    ops: &[Directive],
    line: u32,
) {
    let mut list = Vec::with_capacity(ops.len() + 1);
    list.push(d(Op::Section { name: section.to_owned(), parents }, line));
    list.extend(ops.iter().cloned());
    b.apply(source, &list, 1);
}

/// A recipe as the composition takes it: `(at, makes, count, parts, line)`.
type RecipeLine = (String, String, u32, Vec<String>, u32);

/// `[xms_craft] <n> = at, result, count, part, ...`: `n` goes on after the highest number the
/// section holds, padded so that the keys' order as text is the order the recipes came in.
/// `recipes` are `(at, makes, count, parts, line)`.
fn add_recipes(b: &mut Builder, source: &str, recipes: &[RecipeLine]) {
    if recipes.is_empty() {
        return;
    }
    let mut next: u64 = 1;
    if let Some(s) = b.section("xms_craft") {
        for it in &s.items {
            if !it.key.is_empty() && it.key.bytes().all(|c| c.is_ascii_digit()) {
                next = next.max(it.key.parse::<u64>().unwrap_or(0) + 1);
            }
        }
    }
    let mut ops = Vec::new();
    for (at, makes, count, parts, line) in recipes {
        let mut value = format!("{at}, {makes}, {count}");
        for part in parts {
            value.push_str(", ");
            value.push_str(part);
        }
        ops.push(d(Op::Set { key: format!("{next:06}"), value }, *line));
        next += 1;
    }
    let line = recipes.first().map_or(0, |r| r.4);
    apply_to(b, source, "xms_craft", None, &ops, line);
}

/// What the game makes of a table file (`PlanTable`): its blocks of other configs and its
/// recipes. `lines` are the file's own.
fn plan_table(table: &TableFile, lines: &Lines) -> (Vec<ListBlock>, Vec<RecipeLine>) {
    let mut blocks = Vec::new();
    let mut recipes = Vec::new();
    let mut push = |file: &str, section: &str, op: Op, line: u32| {
        blocks.push(ListBlock {
            file: file.to_owned(),
            section: section.to_owned(),
            parents: None,
            ops: vec![d(op, line)],
            line,
        });
    };
    let set = |key: &str, value: String| Op::Set { key: key.to_owned(), value };
    match &table.rows {
        Rows::Trade { trader, stock } => {
            let Some(file) = trader_file(trader) else { return (blocks, recipes) };
            for (i, s) in stock.iter().enumerate() {
                let line = lines.at(&[Seg::key("stock"), Seg::Index(i)]);
                let item = s.item.to_ascii_lowercase();
                let value =
                    format!("{}, {}", number(s.count.unwrap_or(1.0)), number(s.chance.unwrap_or(1.0)));
                for sup in s.sections() {
                    // a count of 0 takes the item off the shelf
                    let op = if s.removes() { Op::Delete(item.clone()) } else { set(&item, value.clone()) };
                    push(&file, &sup, op, line);
                }
            }
        }
        Rows::Loot(drops) => {
            for (i, l) in drops.iter().enumerate() {
                let line = lines.at(&[Seg::key("drops"), Seg::Index(i)]);
                let item = l.item.to_ascii_lowercase();
                let count = match l.count {
                    None => None,
                    Some(Count::One(n)) => Some(number(n)),
                    Some(Count::Range(a, b)) => Some(format!("{}, {}", number(a), number(b))),
                };
                if let Some(c) = count {
                    push(DEATH_COUNTS, "item_count", set(&item, c), line);
                }
                // every community and rank inherits [base]
                if let Some(c) = l.chance {
                    push(DEATH_CHANCES, "base", set(&item, number(c)), line);
                }
                for (who, c) in &l.by {
                    push(DEATH_CHANCES, &who.to_ascii_lowercase(), set(&item, number(*c)), line);
                }
            }
        }
        Rows::Recipes(list) => {
            for (i, c) in list.iter().enumerate() {
                recipes.push((
                    c.at.to_ascii_lowercase(),
                    c.makes.to_ascii_lowercase(),
                    c.count.unwrap_or(1),
                    lower(&c.parts),
                    lines.at(&[Seg::key("recipes"), Seg::Index(i)]),
                ));
            }
        }
    }
    (blocks, recipes)
}

/// The item applied as the game applies it (`apply`): what it made and changed.
fn apply_plan(b: &mut Builder, item: &Item, p: &ItemPlan, lines: &Lines, a: &mut Applied) {
    let source = item.file.as_str();
    let head = lines.at(&[]);
    if !p.edit {
        // the hud the bases name, read before the item's own keys may name another
        let mut base_hud = String::new();
        for base in &p.parents {
            let hud = value_of(b, base, "hud");
            if !hud.is_empty() {
                base_hud = hud;
            }
        }
        apply_to(b, source, &p.section, Some(p.parents.clone()), &p.own, head);
        if p.hud {
            let hud = format!("{}_hud", p.section);
            let parents = if p.hud_parents_given {
                p.hud_parents.clone()
            } else if b.has(&base_hud) {
                vec![base_hud.to_ascii_lowercase()]
            } else {
                Vec::new()
            };
            if parents.is_empty() {
                a.issues.push(Issue::warning(
                    HUD_WITHOUT_BASE,
                    vec![Seg::key("hud")],
                    format!("the HUD section [{hud}] has no base, and the item's bases name no HUD section: it holds only what hud.set says (hud.base names one)"),
                ));
            }
            apply_to(b, source, &hud, Some(parents), &p.hud_ops, lines.at(&[Seg::key("hud")]));
            a.hud = Some(hud);
        }
    } else {
        apply_to(b, source, &p.section, None, &p.own, head);
        if p.hud {
            let hud = value_of(b, &p.section, "hud").to_ascii_lowercase();
            apply_to(b, source, &hud, None, &p.hud_ops, lines.at(&[Seg::key("hud")]));
            a.hud = Some(hud);
        }
    }
    for s in &p.sections {
        apply_to(
            b,
            source,
            &s.name,
            if s.make { Some(s.parents.clone()) } else { None },
            &s.ops,
            lines.at(&s.at),
        );
        a.owned.push(s.name.clone());
    }
}

/// The table files of a module as the composition takes them: in the game's order (traders,
/// loot tables, recipes, each kind by path), with their files' lines and whether the game
/// refuses their shape.
#[derive(Clone, Copy, Default)]
pub struct Tables<'a> {
    pub files: &'a [TableFile],
    pub lines: &'a [std::sync::Arc<Lines>],
    pub malformed: &'a [bool],
}

/// Lays the module over `game` in the game's order. `items` in path order, `lines` their files'
/// own (the lines values are written at), `malformed` the items whose own shape the game
/// refuses (the reader's errors), `plan` the icon atlas, `tables` the module's table files.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn compose(
    game: &Ltx,
    cfg: &ModuleConfigs,
    items: &[Item],
    lines: &[std::sync::Arc<Lines>],
    malformed: &[bool],
    module: &str,
    plan: &Plan,
    tables: Tables<'_>,
) -> Composition {
    let mut out = Composition {
        applied: vec![Applied::default(); items.len()],
        tables: vec![AppliedTable::default(); tables.files.len()],
        ..Composition::default()
    };
    let mut b = Builder::from_ltx(game, Dialect::Editor);
    b.set_merge(true);
    let every = || cfg.all.iter().chain(&cfg.xms).chain(&cfg.replaced);
    let parsed: FxHashMap<String, ParsedFile> =
        every().map(|f| (f.virt.clone(), parse_file(&f.text, Dialect::Editor))).collect();
    for f in every() {
        out.module_files.insert(format!("@configs\\{}", f.virt), f.rel.clone());
    }
    let overlay =
        |b: &mut Builder, files: &[ConfigFile], layer: u16, problems: &mut Vec<(String, String)>| {
            let loader = ModuleFiles { files: parsed.clone(), layer };
            for f in files {
                if let Err(e) = b.overlay(&format!("@configs\\{}", f.virt), &loader) {
                    problems.push((f.rel.clone(), e.to_string()));
                }
            }
        };
    overlay(&mut b, &cfg.replaced, 1, &mut out.problems);

    let mut fresh: Vec<(usize, ItemPlan)> = Vec::new();
    let mut edits: Vec<(usize, ItemPlan)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let (Some(a), Some(l)) = (out.applied.get_mut(i), lines.get(i)) else { continue };
        a.section = item.game_section(module);
        if malformed.get(i).copied().unwrap_or(true) {
            a.left_out = true;
            continue;
        }
        match plan_item(item, module, plan, l) {
            Ok(p) if p.edit => edits.push((i, p)),
            Ok(p) => fresh.push((i, p)),
            Err(issue) => {
                a.left_out = true;
                a.issues.push(issue);
            }
        }
    }
    let mut lists: Vec<(usize, ListBlock)> = Vec::new();
    let mut finish = |b: &mut Builder, i: usize, p: ItemPlan, a: &mut Applied| {
        if let (Some(item), Some(l)) = (items.get(i), lines.get(i)) {
            apply_plan(b, item, &p, l, a);
        }
        lists.extend(p.lists.into_iter().map(|blk| (i, blk)));
    };
    // new sections first: a base may be another new item of the module, so each waits for its bases
    let mut progress = true;
    while progress && !fresh.is_empty() {
        progress = false;
        let mut waiting = Vec::new();
        for (i, p) in std::mem::take(&mut fresh) {
            match ready(&b, &p) {
                Ready::Wait(_) => waiting.push((i, p)),
                Ready::Refuse(issue) => {
                    if let Some(a) = out.applied.get_mut(i) {
                        a.left_out = true;
                        a.issues.push(issue);
                    }
                    progress = true;
                }
                Ready::Go => {
                    if let Some(a) = out.applied.get_mut(i) {
                        finish(&mut b, i, p, a);
                    }
                    progress = true;
                }
            }
        }
        fresh = waiting;
    }
    let waiting: FxHashSet<String> = fresh.iter().map(|(_, p)| p.section.clone()).collect();
    for (i, p) in fresh {
        let issue = match ready(&b, &p) {
            Ready::Wait(mut issue) | Ready::Refuse(mut issue) => {
                // the bases of items of the module that wait for each other
                if p.parents.iter().any(|x| waiting.contains(x)) {
                    issue.code = LOOP;
                    issue.message = format!(
                        "{}: the bases of the module's items lead back to [{}]",
                        issue.message, p.section
                    );
                }
                issue
            }
            Ready::Go => continue,
        };
        if let Some(a) = out.applied.get_mut(i) {
            a.left_out = true;
            a.issues.push(issue);
        }
    }
    // then the changes, in the order of their paths
    for (i, p) in edits {
        match ready(&b, &p) {
            Ready::Go => {
                if let Some(a) = out.applied.get_mut(i) {
                    finish(&mut b, i, p, a);
                }
            }
            Ready::Wait(issue) | Ready::Refuse(issue) => {
                if let Some(a) = out.applied.get_mut(i) {
                    a.left_out = true;
                    a.issues.push(issue);
                }
            }
        }
    }

    // a block of lines: the system config's applied now, another config's kept for whoever reads it
    let route = |b: &mut Builder, source: &str, blk: ListBlock| -> Option<FilePatch> {
        let mut ds =
            vec![d(Op::Section { name: blk.section.clone(), parents: blk.parents.clone() }, blk.line)];
        ds.extend(blk.ops);
        if blk.file.is_empty() || b.reads(&blk.file) {
            b.apply(source, &ds, 1);
        }
        (!blk.file.is_empty()).then(|| FilePatch {
            file: blk.file,
            source: source.to_owned(),
            layer: 1,
            directives: ds,
        })
    };
    // the items' use scenes
    for (i, blk) in lists {
        let source = items.get(i).map(|it| it.file.clone()).unwrap_or_default();
        if let Some(fp) = route(&mut b, &source, blk) {
            if let Some(a) = out.applied.get_mut(i) {
                a.file_patches.push(fp.clone());
            }
            out.file_patches.push(fp);
        }
    }
    // then the tables, which may name the items the module made: a trader's shelf, the death loot,
    // the recipes
    for (i, table) in tables.files.iter().enumerate() {
        let (Some(a), Some(l)) = (out.tables.get_mut(i), tables.lines.get(i)) else { continue };
        if tables.malformed.get(i).copied().unwrap_or(true) {
            a.left_out = true;
            continue;
        }
        let (blocks, recipes) = plan_table(table, l);
        add_recipes(&mut b, &table.file, &recipes);
        for blk in blocks {
            if let Some(fp) = route(&mut b, &table.file, blk) {
                a.file_patches.push(fp.clone());
                out.file_patches.push(fp);
            }
        }
    }

    overlay(&mut b, &cfg.xms, 2, &mut out.problems);
    for p in &cfg.patches {
        out.file_patches.extend(b.patch(&p.rel, &p.text, 2));
    }
    out.ltx = std::sync::Arc::new(b.finish().ltx);
    out
}
