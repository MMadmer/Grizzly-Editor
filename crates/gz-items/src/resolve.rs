//! What one item resolves to: its section as the game ends up holding it, every key with its
//! value and where the value comes from; the same for its HUD section and the sections it owns;
//! for a change, the game's values before it. What `items.info` answers and a form shows.

use gz_ltx::{FilePatch, Ltx, Op, Section};
use serde::Serialize;

use crate::catalog::Catalog;
use crate::compose::{Applied, Composition, Origin};
use crate::model::{Item, Kind};

/// Where a value of a resolved section comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum From {
    /// This item says it.
    This,
    /// A base of this item gives it (`base` names which).
    Base,
    /// The game's configs (or a module installed before), not through a base this item names.
    Game,
    /// The module's own configs or patches.
    Module,
    /// Another item of the module.
    Item,
}

/// One key of a resolved section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    pub key: String,
    /// As the game reads it; `None` for `key =` or a bare key.
    pub value: Option<String>,
    pub from: From,
    /// The base it comes through, for a new item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Where the value was written: the game's config, or a project file.
    pub file: String,
    pub line: u32,
}

/// A section as the game holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SectionView {
    pub name: String,
    /// The game has it after the module's composition.
    pub exists: bool,
    pub parents: Vec<String>,
    pub rows: Vec<Row>,
}

impl SectionView {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.key == key)
    }
}

/// One line an item gives another config file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PatchLine {
    /// The file under `configs`.
    pub file: String,
    pub section: String,
    /// `set`, `add`, `drop` or `delete`.
    pub op: &'static str,
    pub key: String,
    pub value: String,
}

/// What an item resolves to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Resolution {
    pub file: String,
    /// `new`, `change`, `neither` or `both`.
    pub kind: &'static str,
    pub section: String,
    /// The engine class its section resolves to.
    pub class: Option<String>,
    /// The catalog's family of that class.
    pub family: Option<String>,
    /// The game leaves the item out (the check says why).
    pub left_out: bool,
    pub main: SectionView,
    /// A change: the section as the game has it without the module.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<SectionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hud: Option<SectionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hud_before: Option<SectionView>,
    pub sections: Vec<SectionView>,
    pub file_patches: Vec<PatchLine>,
}

/// The name a kind of file goes by in `kind`.
#[must_use]
pub fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::New => "new",
        Kind::Change => "change",
        Kind::Neither => "neither",
        Kind::Both => "both",
    }
}

fn rows_of(s: &Section, comp: &Composition, this: &str, bases: &[&Section]) -> Vec<Row> {
    s.items
        .iter()
        .map(|it| {
            let (origin, file) = comp.origin_of(it.file);
            let mut from = match origin {
                Origin::Game => From::Game,
                Origin::Module => From::Module,
                Origin::Item if file.eq_ignore_ascii_case(this) => From::This,
                Origin::Item => From::Item,
            };
            let mut base = None;
            if from != From::This && !bases.is_empty() {
                // the last base gives what the item has from its bases, as `[s]:a, b` reads them
                let exact = bases.iter().rev().find(|b| {
                    b.get(&it.key)
                        .is_some_and(|x| x.value == it.value && x.file == it.file && x.line == it.line)
                });
                let any = exact.or_else(|| bases.iter().rev().find(|b| b.get(&it.key).is_some()));
                if let Some(b) = any {
                    base = Some(b.name.to_string());
                    if from == From::Game {
                        from = From::Base;
                    }
                }
            }
            Row {
                key: it.key.to_string(),
                value: it.value.as_deref().map(str::to_owned),
                from,
                base,
                file,
                line: it.line,
            }
        })
        .collect()
}

/// A section of a tree, as a view; `this` is the item's file, `bases` what it starts from.
#[must_use]
pub fn view(ltx: &Ltx, name: &str, comp: &Composition, this: &str, bases: &[String]) -> SectionView {
    let lc = name.trim().to_ascii_lowercase();
    let Some(s) = ltx.section(&lc) else {
        return SectionView { name: lc, ..SectionView::default() };
    };
    let base_sections: Vec<&Section> =
        bases.iter().filter_map(|b| ltx.section(&b.trim().to_ascii_lowercase())).collect();
    SectionView {
        name: lc,
        exists: true,
        parents: s.parents.iter().map(|p| p.to_string()).collect(),
        rows: rows_of(s, comp, this, &base_sections),
    }
}

/// A section of the game's own tree (before the module), as a view.
#[must_use]
pub fn game_view(game: &Ltx, name: &str) -> SectionView {
    let lc = name.trim().to_ascii_lowercase();
    let Some(s) = game.section(&lc) else {
        return SectionView { name: lc, ..SectionView::default() };
    };
    SectionView {
        name: lc,
        exists: true,
        parents: s.parents.iter().map(|p| p.to_string()).collect(),
        rows: s
            .items
            .iter()
            .map(|it| Row {
                key: it.key.to_string(),
                value: it.value.as_deref().map(str::to_owned),
                from: From::Game,
                base: None,
                file: game.files().get(it.file as usize).map(|f| f.to_string()).unwrap_or_default(),
                line: it.line,
            })
            .collect(),
    }
}

/// The lines of `#file` patches.
#[must_use]
pub fn patch_lines(patches: &[FilePatch]) -> Vec<PatchLine> {
    let mut out = Vec::new();
    for p in patches {
        let mut section = String::new();
        for d in &p.directives {
            let (op, key, value) = match &d.op {
                Op::Section { name, .. } => {
                    section.clone_from(name);
                    continue;
                }
                Op::DropSection(_) => continue,
                Op::Set { key, value } => ("set", key, value.clone()),
                Op::Append { key, value } => ("add", key, value.clone()),
                Op::Remove { key, value } => ("drop", key, value.clone()),
                Op::Delete(key) => ("delete", key, String::new()),
            };
            out.push(PatchLine {
                file: p.file.clone(),
                section: section.clone(),
                op,
                key: key.clone(),
                value,
            });
        }
    }
    out
}

/// What `item` resolves to in the composition, against `game` (the tree without the module).
#[must_use]
pub fn resolve(
    item: &Item,
    applied: &Applied,
    comp: &Composition,
    game: &Ltx,
    catalog: Option<&Catalog>,
) -> Resolution {
    let kind = item.kind();
    let bases: Vec<String> = if kind == Kind::New { item.base.clone() } else { Vec::new() };
    let main = view(&comp.ltx, &applied.section, comp, &item.file, &bases);
    let class = main.get("class").and_then(|r| r.value.clone()).map(|c| c.trim().to_owned());
    let family =
        class.as_deref().and_then(|c| catalog.and_then(|cat| cat.family_of_class(c))).map(|f| f.id.clone());
    let hud_bases: Vec<String> = item.hud.as_ref().map(|h| h.base.clone()).unwrap_or_default();
    let hud = applied.hud.as_ref().map(|h| view(&comp.ltx, h, comp, &item.file, &hud_bases));
    let change = kind == Kind::Change;
    Resolution {
        file: item.file.clone(),
        kind: kind_word(kind),
        section: applied.section.clone(),
        class,
        family,
        left_out: applied.left_out,
        before: change.then(|| game_view(game, &applied.section)),
        hud_before: if change { applied.hud.as_ref().map(|h| game_view(game, h)) } else { None },
        main,
        hud,
        sections: applied.owned.iter().map(|s| view(&comp.ltx, s, comp, &item.file, &[])).collect(),
        file_patches: patch_lines(&applied.file_patches),
    }
}
