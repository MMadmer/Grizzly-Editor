//! What the module adds to the game's shared tables, each in a file of the side that owns the
//! rows: a trader's shelf (`.tradeasset`), what the dead carry (`.lootasset`), recipes
//! (`.recipeasset`). An item never says any of it (`docs/ASSETS.md`, "One thing, one place").
//!
//! One [`TableFile`] per file: read with every problem of its own shape at the key it is about
//! ([`from_value`]), projected back for a save to compare ([`to_value`]), written canonically for
//! a new file ([`write`], [`template`]). The game takes a file whole or not at all, so any
//! problem of its shape leaves the file out.

use gz_luadata::{DiffRules, Seg, Table, Value, write_asset};
pub use gz_project::asset_kind::TableKind;

use crate::fields;
use crate::model::{BAD_VALUE, Count, Cx, Issue, plain_value, stem};

/// The format version of each of the three kinds.
pub const VERSION: u32 = 1;

/// A row of a trader's stock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stock {
    pub item: String,
    pub supplies: Vec<String>,
    pub count: Option<f64>,
    pub chance: Option<f64>,
}

impl Stock {
    /// The stock sections, lower case; `supplies_1` when the row names none.
    #[must_use]
    pub fn sections(&self) -> Vec<String> {
        if self.supplies.is_empty() {
            vec!["supplies_1".to_owned()]
        } else {
            self.supplies.iter().map(|s| s.to_ascii_lowercase()).collect()
        }
    }

    /// A count of 0 takes the item off the shelf.
    #[must_use]
    pub fn removes(&self) -> bool {
        self.count == Some(0.0)
    }
}

/// A row of the death loot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drop {
    pub item: String,
    pub count: Option<Count>,
    pub chance: Option<f64>,
    pub by: Vec<(String, f64)>,
}

/// A recipe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recipe {
    pub makes: String,
    pub count: Option<u32>,
    pub at: String,
    pub parts: Vec<String>,
}

/// The rows of a file, by its kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Rows {
    Trade { trader: String, stock: Vec<Stock> },
    Loot(Vec<Drop>),
    Recipes(Vec<Recipe>),
}

impl Rows {
    #[must_use]
    pub fn empty(kind: TableKind) -> Self {
        match kind {
            TableKind::Trade => Rows::Trade { trader: String::new(), stock: Vec::new() },
            TableKind::Loot => Rows::Loot(Vec::new()),
            TableKind::Recipes => Rows::Recipes(Vec::new()),
        }
    }

    #[must_use]
    pub fn kind(&self) -> TableKind {
        match self {
            Rows::Trade { .. } => TableKind::Trade,
            Rows::Loot(_) => TableKind::Loot,
            Rows::Recipes(_) => TableKind::Recipes,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Rows::Trade { stock, .. } => stock.len(),
            Rows::Loot(d) => d.len(),
            Rows::Recipes(r) => r.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One file of a table kind.
#[derive(Clone, Debug, PartialEq)]
pub struct TableFile {
    /// Where it was read from: project-relative with `/`.
    pub file: String,
    pub rows: Rows,
    /// Kept as it is: the editor's notes.
    pub editor: Value,
}

impl TableFile {
    #[must_use]
    pub fn new(kind: TableKind, file: &str) -> Self {
        Self { file: file.to_owned(), rows: Rows::empty(kind), editor: Value::Nil }
    }

    #[must_use]
    pub fn kind(&self) -> TableKind {
        self.rows.kind()
    }
}

/// The trader's config under `configs`: `misc\trade\<trader>.ltx`, or the path a name ending in
/// `.ltx` is; `None` when it names no config under `configs`.
#[must_use]
pub fn trader_file(trader: &str) -> Option<String> {
    let name = trader.trim().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let path =
        if name.len() > 4 && name.ends_with(".ltx") { name } else { format!("misc\\trade\\{name}.ltx") };
    gz_ltx::patch::config_path(&path).filter(|p| !p.is_empty() && !p.contains(':') && plain_value(p))
}

/// The name a trader's config goes by in a file: `trade_military` for `misc\trade\trade_military.ltx`,
/// the path itself for a config elsewhere.
#[must_use]
pub fn trader_name(config: &str) -> String {
    let key = gz_ltx::config_key(config);
    match key.strip_prefix("misc\\trade\\").and_then(|rest| rest.strip_suffix(".ltx")) {
        Some(name) if !name.contains('\\') => name.to_owned(),
        _ => key,
    }
}

// ---- reading ---------------------------------------------------------------------------------

/// The item a row is about: a section a config can hold. A row without one is refused.
fn item_of(cx: &mut Cx, row: &Value, key: &str, noun: &str, missing: &str) -> String {
    match cx.name(row, key, noun) {
        Some(s) => s,
        None => {
            if row.get(key).is_none() {
                cx.bad(key, format!("{missing}: {key}"));
            }
            String::new()
        }
    }
}

fn stock(cx: &mut Cx, row: &Value) -> Stock {
    let s = Stock {
        item: item_of(cx, row, "item", "the item of a stock row", "a row of stock names its item"),
        supplies: cx.sections(row, "supplies", "stock.supplies"),
        count: cx.any_number(row, "count"),
        chance: cx.any_number(row, "chance"),
    };
    if s.count.is_some_and(|c| c < 0.0) {
        cx.bad("count", "stock.count is a number not below 0 (0 takes the item off the shelf)");
    }
    s
}

fn drop_row(cx: &mut Cx, row: &Value) -> Drop {
    let item = item_of(cx, row, "item", "the item of a drop", "a row of drops names its item");
    let count = match row.get("count") {
        None => None,
        Some(Value::Number(n)) => Some(Count::One(*n)),
        Some(Value::Table(t))
            if t.fields.is_empty() && t.arr.len() == 2 && t.arr.iter().all(|x| x.as_number().is_some()) =>
        {
            let n = |i: usize| t.arr.get(i).and_then(Value::as_number).unwrap_or(0.0);
            Some(Count::Range(n(0), n(1)))
        }
        Some(_) => {
            cx.bad("count", "drops.count is a number or { min, max }");
            None
        }
    };
    let chance = cx.any_number(row, "chance");
    let mut by = Vec::new();
    match row.get("by") {
        None => {}
        Some(Value::Table(t)) if t.arr.is_empty() => {
            for (k, x) in &t.fields {
                match x.as_number().filter(|_| crate::model::plain_name(&k.to_ascii_lowercase())) {
                    Some(n) => by.push((k.to_string(), n)),
                    None => {
                        let mut p = cx.at("by");
                        p.push(Seg::Key(k.clone()));
                        cx.bad_at(p, "drops.by is { <community or community_rank> = chance }");
                    }
                }
            }
        }
        Some(_) => cx.bad("by", "drops.by is { <community or community_rank> = chance }"),
    }
    Drop { item, count, chance, by }
}

fn recipe(cx: &mut Cx, row: &Value) -> Recipe {
    let makes = item_of(cx, row, "makes", "what a recipe makes", "a recipe names what it makes");
    let at = item_of(cx, row, "at", "a recipe's at", "a recipe names the tool or workbench it is made at");
    let parts = cx.sections(row, "parts", "a part of a recipe");
    if parts.is_empty() || parts.len() > 4 {
        cx.bad("parts", "a recipe takes one to four parts");
    }
    Recipe { makes, count: cx.integer(row, fields::RECIPE, "count"), at, parts }
}

/// A file of the kind from its value, with the problems of its own shape: any of them makes the
/// game leave the whole file out. A file of a newer format version is refused.
pub fn from_value(kind: TableKind, v: &Value) -> Result<(TableFile, Vec<Issue>), String> {
    if !v.is_table() {
        return Err(format!("a {} file returns one table", kind.name()));
    }
    let header = kind.header();
    if let Some(Value::Number(n)) = v.get(header)
        && *n > f64::from(VERSION)
    {
        return Err(format!(
            "made by a newer Grizzly Editor ({header} = {n}; this one reads up to {VERSION}): update the editor to open it"
        ));
    }
    let mut cx = Cx::default();
    let key = kind.rows();
    let what = match kind {
        TableKind::Trade => "stock is a list of { item, supplies, count, chance }",
        TableKind::Loot => "drops is a list of { item, count, chance, by }",
        TableKind::Recipes => "recipes is a list of { makes, count, at, parts }",
    };
    let list = cx.rows(v, key, what).to_vec();
    let read = |cx: &mut Cx, i: usize, row: &Value| -> bool {
        if row.is_table() {
            return true;
        }
        cx.bad_at(vec![Seg::key(key), Seg::Index(i)], format!("a row of {key} is a table"));
        false
    };
    let rows = match kind {
        TableKind::Trade => {
            let trader = match v.get("trader").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
                Some(s) => s.to_owned(),
                None => {
                    cx.bad("trader", "a trader file names the trader: trader");
                    String::new()
                }
            };
            if !trader.is_empty() && trader_file(&trader).is_none() {
                cx.bad(
                    "trader",
                    "trader is a trader's config (trade_military), or a config's path under configs",
                );
            }
            let mut out = Vec::new();
            for (i, row) in list.iter().enumerate() {
                if read(&mut cx, i, row) {
                    out.push(cx.within(&[Seg::key(key), Seg::Index(i)], |cx| stock(cx, row)));
                }
            }
            Rows::Trade { trader, stock: out }
        }
        TableKind::Loot => {
            let mut out = Vec::new();
            for (i, row) in list.iter().enumerate() {
                if read(&mut cx, i, row) {
                    out.push(cx.within(&[Seg::key(key), Seg::Index(i)], |cx| drop_row(cx, row)));
                }
            }
            Rows::Loot(out)
        }
        TableKind::Recipes => {
            let mut out = Vec::new();
            for (i, row) in list.iter().enumerate() {
                if read(&mut cx, i, row) {
                    out.push(cx.within(&[Seg::key(key), Seg::Index(i)], |cx| recipe(cx, row)));
                }
            }
            Rows::Recipes(out)
        }
    };
    let editor = v.get("editor").cloned().unwrap_or_default();
    Ok((TableFile { file: String::new(), rows, editor }, cx.issues))
}

/// Reads the text of a file of the kind. The error is a sentence, `line N: <message>` when the
/// reader names a line.
pub fn load_text(kind: TableKind, text: &str, file: &str) -> Result<(TableFile, Vec<Issue>), String> {
    let v = gz_luadata::parse_named(text, stem(file))
        .map_err(|e| if e.line != 0 { format!("line {}: {}", e.line, e.message) } else { e.message })?;
    let (mut table, issues) = from_value(kind, &v)?;
    file.clone_into(&mut table.file);
    Ok((table, issues))
}

// ---- writing ---------------------------------------------------------------------------------

fn names_value(list: &[String]) -> Value {
    match list {
        [one] => Value::from(one.as_str()),
        many => Value::from(many.iter().map(|n| Value::from(n.as_str())).collect::<Vec<_>>()),
    }
}

/// A file as a save compares it with what was read, and as a new file holds it: the keys in the
/// reference's order, nothing the file did not say.
#[must_use]
pub fn to_value(table: &TableFile) -> Value {
    let kind = table.kind();
    let mut t = Table::new();
    t.put(kind.header(), VERSION);
    let rows: Vec<Value> = match &table.rows {
        Rows::Trade { trader, stock } => {
            t.put("trader", trader.as_str());
            stock
                .iter()
                .map(|s| {
                    let mut r = Table::new();
                    r.put("item", s.item.as_str());
                    if !s.supplies.is_empty() {
                        r.put("supplies", names_value(&s.supplies));
                    }
                    if let Some(n) = s.count {
                        r.put("count", n);
                    }
                    if let Some(c) = s.chance {
                        r.put("chance", c);
                    }
                    Value::Table(r)
                })
                .collect()
        }
        Rows::Loot(drops) => drops
            .iter()
            .map(|d| {
                let mut r = Table::new();
                r.put("item", d.item.as_str());
                match d.count {
                    None => {}
                    Some(Count::One(n)) => r.put("count", n),
                    Some(Count::Range(a, b)) => {
                        r.put("count", Value::from(vec![Value::from(a), Value::from(b)]));
                    }
                }
                if let Some(c) = d.chance {
                    r.put("chance", c);
                }
                if !d.by.is_empty() {
                    let mut by = Table::new();
                    for (k, c) in &d.by {
                        by.put(k, *c);
                    }
                    r.put("by", by);
                }
                Value::Table(r)
            })
            .collect(),
        Rows::Recipes(recipes) => recipes
            .iter()
            .map(|c| {
                let mut r = Table::new();
                r.put("makes", c.makes.as_str());
                if let Some(n) = c.count {
                    r.put("count", n);
                }
                r.put("at", c.at.as_str());
                r.put(
                    "parts",
                    Value::from(c.parts.iter().map(|p| Value::from(p.as_str())).collect::<Vec<_>>()),
                );
                Value::Table(r)
            })
            .collect(),
    };
    t.put(kind.rows(), Value::from(rows));
    if !table.editor.is_nil() {
        t.put("editor", table.editor.clone());
    }
    Value::Table(t)
}

fn head(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Trade => {
            "-- What this module puts on a trader's shelf. `Grizzly-Cmd.exe reference trader` lists every key.\n"
        }
        TableKind::Loot => "-- What the dead carry. `Grizzly-Cmd.exe reference loot` lists every key.\n",
        TableKind::Recipes => {
            "-- Recipes of this module. `Grizzly-Cmd.exe reference recipes` lists every key.\n"
        }
    }
}

/// The canonical text of a new file. An existing file is never written this way: a save writes
/// only what changed into it ([`to_value`]).
#[must_use]
pub fn write(table: &TableFile) -> String {
    match to_value(table) {
        Value::Table(t) => write_asset(head(table.kind()), &t),
        _ => String::new(),
    }
}

/// The start of a new file of the kind: commented, with one row to change.
#[must_use]
pub fn template(kind: TableKind) -> String {
    match kind {
        TableKind::Trade => "\
-- What this module puts on the shelf of a trader of the game. The trader is its config's name:\n\
-- `trade_military` is misc\\trade\\trade_military.ltx. A character of the module keeps its shop in\n\
-- its own .npcasset. `Grizzly-Cmd.exe reference trader` lists every key.\n\
return {\n  trade = 1,\n  trader = \"trade_military\",\n  stock = {\n    -- supplies = \"supplies_1\" unless a row says otherwise; count = 0 takes an item off the shelf\n    { item = \"bread\", count = 2, chance = 1 },\n  },\n}\n"
            .to_owned(),
        TableKind::Loot => "\
-- What the dead carry: an item, how many, and the chance - for everybody (`chance`) and for a\n\
-- community or a community and a rank (`by`). `Grizzly-Cmd.exe reference loot` lists every key.\n\
return {\n  loot = 1,\n  drops = {\n    { item = \"bread\", count = { 0, 1 }, chance = 0.05, by = { bandit = 0.1 } },\n  },\n}\n"
            .to_owned(),
        TableKind::Recipes => "\
-- Recipes: what is made (`makes`), at which tool or workbench (`at`), from one to four parts.\n\
-- `Grizzly-Cmd.exe reference recipes` lists every key.\n\
return {\n  recipe = 1,\n  recipes = {\n    { makes = \"bread\", count = 1, at = \"itm_repairkit_tier_1\", parts = { \"jar\", \"rope\" } },\n  },\n}\n"
            .to_owned(),
    }
}

/// The keys each level of a file of the kind knows: any other is the author's, kept and noted.
/// The tables of [`fields`] say the same (a test holds them to it).
#[must_use]
pub fn known(kind: TableKind) -> &'static [(&'static str, &'static [&'static str])] {
    match kind {
        TableKind::Trade => &[
            ("", &["trade", "trader", "stock", "editor"]),
            ("stock.*", &["item", "supplies", "count", "chance"]),
        ],
        TableKind::Loot => {
            &[("", &["loot", "drops", "editor"]), ("drops.*", &["item", "count", "chance", "by"])]
        }
        TableKind::Recipes => {
            &[("", &["recipe", "recipes", "editor"]), ("recipes.*", &["makes", "count", "at", "parts"])]
        }
    }
}

/// The tables of a kind's levels.
#[must_use]
pub fn levels(kind: TableKind) -> &'static [(&'static str, &'static [fields::Field])] {
    match kind {
        TableKind::Trade => fields::TRADE_LEVELS,
        TableKind::Loot => fields::LOOT_LEVELS,
        TableKind::Recipes => fields::RECIPE_LEVELS,
    }
}

fn rules(kind: TableKind) -> DiffRules {
    known(kind).iter().fold(DiffRules::new(), |rules, (pattern, keys)| rules.order(pattern, keys))
}

/// How a trader file's values map onto its file: new keys in the reference's order.
#[must_use]
pub fn trade_rules() -> DiffRules {
    rules(TableKind::Trade)
}

#[must_use]
pub fn loot_rules() -> DiffRules {
    rules(TableKind::Loot)
}

#[must_use]
pub fn recipe_rules() -> DiffRules {
    rules(TableKind::Recipes)
}

/// A value outside what its key takes (the items' code): re-exported for the checks of a table.
pub const BAD: &str = BAD_VALUE;

#[cfg(test)]
mod tests {
    use super::*;

    fn read(kind: TableKind, text: &str) -> (TableFile, Vec<Issue>) {
        load_text(kind, text, "t/x").expect("the file reads")
    }

    #[test]
    fn a_trader_reads_its_stock_and_keeps_unset_unset() {
        let (t, issues) = read(
            TableKind::Trade,
            "return { trade = 1, trader = \"trade_military\", stock = {\n  { item = \"wpn_x\", count = 1, chance = 0.5 },\n  { item = \"ammo_x\", supplies = { \"supplies_2\", \"Supplies_3\" } },\n  { item = \"vodka\", count = 0 },\n} }",
        );
        assert!(issues.is_empty(), "{issues:?}");
        let Rows::Trade { trader, stock } = &t.rows else { panic!("a trader") };
        assert_eq!(trader, "trade_military");
        assert_eq!(stock.len(), 3);
        assert_eq!(
            (stock[0].count, stock[0].chance, stock[0].sections()),
            (Some(1.0), Some(0.5), vec!["supplies_1".to_owned()])
        );
        assert_eq!(
            (stock[1].count, stock[1].sections()),
            (None, vec!["supplies_2".to_owned(), "supplies_3".to_owned()])
        );
        assert!(stock[2].removes() && !stock[0].removes());
        assert_eq!(trader_file(trader).as_deref(), Some("misc\\trade\\trade_military.ltx"));
        assert_eq!(
            trader_file("Misc/Trade/Trade_Barman.LTX").as_deref(),
            Some("misc\\trade\\trade_barman.ltx")
        );
        assert_eq!(trader_file("..\\x.ltx"), None);
        assert_eq!(trader_name("misc\\trade\\trade_military.ltx"), "trade_military");
        assert_eq!(trader_name("misc\\other\\shop.ltx"), "misc\\other\\shop.ltx");
    }

    #[test]
    fn what_the_game_refuses_is_an_issue_at_its_key() {
        let at = |kind, text: &str| -> Vec<String> {
            read(kind, text).1.iter().map(|i| gz_luadata::path::show(&i.path)).collect()
        };
        assert_eq!(at(TableKind::Trade, "return { trade = 1, stock = {} }"), ["trader"]);
        assert_eq!(at(TableKind::Trade, "return { trade = 1, trader = \"..\\\\x.ltx\" }"), ["trader"]);
        assert_eq!(
            at(TableKind::Trade, "return { trade = 1, trader = \"t\", stock = { { count = 1 } } }"),
            ["stock[0].item"]
        );
        assert_eq!(
            at(
                TableKind::Trade,
                "return { trade = 1, trader = \"t\", stock = { { item = \"a\", count = -1 } } }"
            ),
            ["stock[0].count"]
        );
        assert_eq!(
            at(TableKind::Loot, "return { loot = 1, drops = { { item = \"a\", count = \"many\" } } }"),
            ["drops[0].count"]
        );
        assert_eq!(
            at(
                TableKind::Recipes,
                "return { recipe = 1, recipes = { { makes = \"x\", at = \"k\", parts = { \"a\", \"b\", \"c\", \"d\", \"e\" } } } }"
            ),
            ["recipes[0].parts"]
        );
        assert_eq!(
            at(TableKind::Recipes, "return { recipe = 1, recipes = { { at = \"k\", parts = \"a\" } } }"),
            ["recipes[0].makes"]
        );
        assert!(
            load_text(TableKind::Loot, "return { loot = 2 }", "x").is_err(),
            "a newer version is refused"
        );
    }

    #[test]
    fn a_file_is_written_canonically_and_reads_back() {
        for kind in TableKind::ALL {
            let (t, issues) = read(kind, &template(kind));
            assert!(issues.is_empty() && t.rows.len() == 1, "{kind:?}: {issues:?}");
            let (again, issues) = read(kind, &write(&t));
            assert!(issues.is_empty(), "{kind:?}: {issues:?}");
            assert_eq!(again.rows, t.rows, "{kind:?}");
        }
    }

    #[test]
    fn the_known_keys_are_the_tables() {
        for kind in TableKind::ALL {
            let (known, levels) = (known(kind), levels(kind));
            assert_eq!(known.len(), levels.len());
            for ((pattern, keys), (level, fields)) in known.iter().zip(levels) {
                assert_eq!(pattern, level);
                assert_eq!(*keys, fields::keys(fields).as_slice(), "{kind:?} {pattern}");
            }
            assert_eq!(known[0].1[0], kind.header());
            assert!(known[0].1.contains(&kind.rows()));
        }
    }
}
