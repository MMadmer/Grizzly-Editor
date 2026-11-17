//! What the items area hangs on the editor: the templates of a new item, trader, loot table and
//! recipes file, their kinds of `gz-asset` (the positioned check and the reference of each), the
//! build's check and its publisher of the icon atlas, the roster made when a project opens; and
//! the commands `items.list`, `items.info` and `economy.list`. The files are the author's text:
//! created, read and changed with file tools and checked with `asset.check`.

use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps, Kinds};
use gz_base::{Origin, Problem, RelPath};
use gz_build::{
    BuildCheck, BuildChecks, BuildFile, BuildWalk, FileFrom, MediaPlan, MediaPublisher, Publication,
    source_key,
};
use gz_editor::{CmdError, Editor, ProjectSession, Registry, Reply, Spec};
use gz_project::{AssetKind, AssetTemplates, ProjectHooks, TemplateKind};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::atlas::{self, Plan};
use crate::model::{self, Issue};
use crate::resolve::{SectionView, kind_word};
use crate::roster::Roster;
use crate::service::{Items, build_errors, table_issues_of};
use crate::tables::{self, Rows, TableKind};

/// The item format, every key with what it takes and what the game does with it.
const ITEM_MD: &str = include_str!("../../../docs/assets/ITEM.md");

/// The formats of the three table kinds.
const TRADER_MD: &str = include_str!("../../../docs/assets/TRADER.md");
const LOOT_MD: &str = include_str!("../../../docs/assets/LOOT.md");
const RECIPES_MD: &str = include_str!("../../../docs/assets/RECIPES.md");

/// What the build says while the items' check runs.
const STAGE: &str = "Checking the items";

/// Moves with every change of what a bake makes of the same pictures.
const BAKE_VERSION: &[u8] = b"gz-items atlas 1";

/// Hangs the area on the editor.
pub fn install(editor: &Editor) {
    let templates = AssetTemplates::of(editor);
    templates.provide(TemplateKind::Item, |cx| Ok(model::template(cx.stem).into_bytes()));
    templates.provide(TemplateKind::Trader, |_| Ok(tables::template(TableKind::Trade).into_bytes()));
    templates.provide(TemplateKind::Loot, |_| Ok(tables::template(TableKind::Loot).into_bytes()));
    templates.provide(TemplateKind::Recipes, |_| Ok(tables::template(TableKind::Recipes).into_bytes()));
    let checks = BuildChecks::of(editor);
    checks.add(Arc::new(ItemsCheck));
    checks.add_publisher(Arc::new(ItemsPublisher));
    let kinds = Kinds::of(editor);
    kinds.add(kind());
    for k in TableKind::ALL {
        kinds.add(table_kind(k));
    }
    kinds.covers_build_stage(STAGE);
    ProjectHooks::of(editor).on_open(|ed, _session| {
        let ed = ed.clone();
        ed.exec.clone().spawn(async move {
            // a project without items costs a look at its file tree
            if let Ok(items) = Items::of(&ed).await {
                items.roster(&ed).await;
            }
        });
    });
}

/// The item kind of `gz-asset`.
#[must_use]
pub fn kind() -> KindOps {
    let mut k = KindOps::of(AssetKind::Item);
    k.known = model::KNOWN;
    k.reference_doc = "docs/assets/ITEM.md";
    k.check = check;
    k.reference = |ed| {
        Box::pin(async move {
            let mut out = ITEM_MD.to_owned();
            if let Ok(items) = Items::of(&ed).await {
                let read = items.catalog(&ed).await;
                let from = match read.provenance {
                    Some(gz_game::Provenance::Game) => "the linked game",
                    Some(gz_game::Provenance::Bundled) => "the editor (no game is linked, or it has none)",
                    None => "nobody",
                };
                out.push_str(&crate::doc::catalog_reference(&read.catalog, from));
            }
            out
        })
    };
    k.diff_rules = model::diff_rules;
    k
}

/// The kind of `gz-asset` of a trader, a loot table or a recipes file.
#[must_use]
pub fn table_kind(kind: TableKind) -> KindOps {
    let mut k = KindOps::of(AssetKind::from(kind));
    k.known = tables::known(kind);
    k.check = check_table;
    match kind {
        TableKind::Trade => {
            k.reference_doc = "docs/assets/TRADER.md";
            k.reference = |_| Box::pin(async { TRADER_MD.to_owned() });
            k.diff_rules = tables::trade_rules;
        }
        TableKind::Loot => {
            k.reference_doc = "docs/assets/LOOT.md";
            k.reference = |_| Box::pin(async { LOOT_MD.to_owned() });
            k.diff_rules = tables::loot_rules;
        }
        TableKind::Recipes => {
            k.reference_doc = "docs/assets/RECIPES.md";
            k.reference = |_| Box::pin(async { RECIPES_MD.to_owned() });
            k.diff_rules = tables::recipe_rules;
        }
    }
    k
}

fn check_table(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let wire = file.path.as_str().to_owned();
        let Some(kind) = TableKind::of_file(&wire) else { return Vec::new() };
        let Ok(items) = Items::of(&ed).await else { return Vec::new() };
        let issues = match items.table_problems(&ed, &wire).await {
            Some(issues) => issues,
            // a file the build leaves out is no table of the module: its own shape alone
            None => match tables::from_value(kind, &file.doc.value()) {
                Ok((_, issues)) => issues,
                Err(e) => vec![Issue::error(crate::roster::UNREADABLE, Vec::new(), e)],
            },
        };
        issues.iter().map(|i| placed(i, &file)).collect()
    })
}

/// An issue as a problem placed in its file.
fn placed(i: &Issue, file: &AssetFile) -> Problem {
    let p = Problem::new(i.code, i.severity, i.message.clone());
    if i.path.is_empty() { file.place_head(p) } else { file.place(p, &i.path) }
}

fn check(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let wire = file.path.as_str().to_owned();
        let Ok(items) = Items::of(&ed).await else { return Vec::new() };
        let issues = match items.problems(&ed, &wire).await {
            Some(issues) => issues,
            // a file the build leaves out is no item of the module: its own shape alone
            None => match model::from_value(&file.doc.value()) {
                Ok((_, issues)) => issues,
                Err(e) => vec![Issue::error(crate::roster::UNREADABLE, Vec::new(), e)],
            },
        };
        let mut seen: Vec<(&'static str, String)> = Vec::new();
        let mut out = Vec::new();
        for i in &issues {
            let key = (i.code, i.message.clone());
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            out.push(placed(i, &file));
        }
        out
    })
}

/// The build makes the icon atlas and nothing else of an item - the game reads the assets as
/// they are; what is left is saying no to a module whose items the game would leave out or
/// stop on. The refusal is the first error.
struct ItemsCheck;

impl BuildCheck for ItemsCheck {
    fn stage(&self) -> &'static str {
        STAGE
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        _session: Arc<ProjectSession>,
        _walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            let Ok(items) = Items::of(&editor).await else { return Vec::new() };
            let roster = items.fresh_roster(&editor).await;
            if roster.entries.is_empty() && roster.tables.is_empty() {
                return Vec::new();
            }
            let catalog = items.catalog(&editor).await;
            let root = items.root().to_path_buf();
            let errors = editor.exec.io(move || build_errors(&roster, &catalog.catalog, &root)).await;
            errors
                .into_iter()
                .next()
                .map(|(file, i)| vec![Problem::error(i.code, format!("{file}: {}", i.message))])
                .unwrap_or_default()
        })
    }
}

/// The items' half of a build: the atlas of their pictures and the texts that name it.
struct ItemsPublisher;

/// The project files the atlas replaces: the pictures and the assets whose text ships
/// rewritten.
fn sources_of(root: &Path, plan: &Plan) -> gz_build::Sources {
    let mut out = gz_build::Sources::default();
    for p in &plan.placed {
        out.insert(source_key(&p.file));
        if let Some(rel) = RelPath::from_fs(root, &p.picture) {
            out.insert(source_key(rel.as_str()));
        }
    }
    out
}

/// The atlas' file in the project's `temp\media`, baked when it is not there yet. Blocking.
fn baked_atlas(root: &Path, plan: &Plan) -> Result<std::path::PathBuf, String> {
    let mut h = gz_media::files::fnv(gz_media::files::FNV0, BAKE_VERSION);
    h = gz_media::files::fnv(h, plan.texture.as_bytes());
    h = gz_media::files::fnv(h, &[plan.width.to_le_bytes(), plan.height.to_le_bytes()].concat());
    for p in &plan.placed {
        h = gz_media::files::fnv(h, p.section.as_bytes());
        for n in [p.x, p.y, p.w, p.h] {
            h = gz_media::files::fnv(h, &n.to_le_bytes());
        }
        let bytes =
            std::fs::read(&p.picture).map_err(|e| format!("{}: {}", p.file, gz_base::io_reason(&e)))?;
        h = gz_media::files::fnv(h, &bytes);
    }
    let dir = gz_media::publish::cache_dir(root);
    let path = dir.join(format!("{h:016x}_icons.dds"));
    if path.is_file() {
        return Ok(path);
    }
    let dds = atlas::bake(plan)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), gz_base::io_reason(&e)))?;
    gz_editor::fsx::write_atomic(&path, &dds)
        .map_err(|e| format!("{}: {}", path.display(), gz_base::io_reason(&e)))?;
    Ok(path)
}

/// What the atlas publishes: the atlas and the rewritten texts. Blocking.
#[must_use]
pub fn publication(root: &Path, roster: &Roster) -> Publication {
    let plan = &roster.plan;
    let mut out = Publication { sources: sources_of(root, plan), ..Publication::default() };
    for (file, why) in &plan.left_out {
        out.errors.push(format!("{file}: the icon cannot go into the atlas: {why}"));
    }
    if plan.is_empty() {
        return out;
    }
    let atlas = match baked_atlas(root, plan) {
        Ok(p) => p,
        Err(e) => {
            out.errors.push(format!("the icon atlas could not be made: {e}"));
            return out;
        }
    };
    let made = |rel: &str, from: FileFrom, size: u64| -> Result<BuildFile, String> {
        let rel = RelPath::new(&rel.replace('\\', "/")).map_err(|e| e.to_string())?;
        Ok(BuildFile { rel, size, modified: None, from })
    };
    let size = std::fs::metadata(&atlas).map(|m| m.len()).unwrap_or(0);
    match made(&plan.path, FileFrom::Made(atlas), size) {
        Ok(f) => out.files.push(f),
        Err(e) => out.errors.push(e),
    }
    for p in &plan.placed {
        let abs = root.join(p.file.replace('/', "\\"));
        let shipped = std::fs::read_to_string(&abs)
            .map_err(|e| gz_base::io_reason(&e))
            .and_then(|t| atlas::shipped_text(t.strip_prefix('\u{feff}').unwrap_or(&t), &plan.texture, p));
        match shipped.and_then(|t| {
            let bytes: Arc<[u8]> = Arc::from(t.into_bytes());
            let n = bytes.len() as u64;
            made(&p.file, FileFrom::Bytes(bytes), n)
        }) {
            Ok(f) => out.files.push(f),
            Err(e) => out.errors.push(format!("{}: {e}", p.file)),
        }
    }
    out
}

impl MediaPublisher for ItemsPublisher {
    fn plan(&self, editor: Arc<Editor>, session: Arc<ProjectSession>) -> BoxFuture<'static, MediaPlan> {
        Box::pin(async move {
            let Ok(items) = Items::of(&editor).await else { return MediaPlan::default() };
            let roster = items.fresh_roster(&editor).await;
            let plan = &roster.plan;
            let mut published: Vec<String> = Vec::new();
            if !plan.is_empty() {
                published.push(plan.path.clone());
                published.extend(plan.placed.iter().map(|p| p.file.replace('/', "\\")));
            }
            MediaPlan { sources: sources_of(&session.root, plan), published }
        })
    }

    fn publish(&self, editor: Arc<Editor>, session: Arc<ProjectSession>) -> BoxFuture<'static, Publication> {
        Box::pin(async move {
            let Ok(items) = Items::of(&editor).await else { return Publication::default() };
            let roster = items.fresh_roster(&editor).await;
            let root = session.root.clone();
            editor.exec.io(move || publication(&root, &roster)).await
        })
    }
}

// ---- commands ---------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListParams {
    /// Only the items whose file, section or name holds this text, any case.
    #[serde(default)]
    pub query: Option<String>,
    /// How many items to answer (default 100); `total` says how many there are.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many to skip first.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InfoParams {
    /// The item: its file (project-relative) or the section it makes or changes.
    pub item: String,
    /// How many keys of its section to answer (default 200); `total_keys` says how many it has.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many keys to skip first.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EconomyParams {
    /// Only the files of this kind: `trader`, `loot` or `recipes`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only the rows that name this item (a section), in any role.
    #[serde(default)]
    pub item: Option<String>,
}

/// The `items` area.
pub fn register(reg: &mut Registry) {
    reg.area("economy", "The module's traders, loot tables and recipes (.tradeasset, .lootasset, .recipeasset): who sells an item, who carries it and what makes it, each in the file of the side that owns it");
    reg.cmd(
        Spec::read(
            "economy.list",
            "The module's traders, loot tables and recipes: every file with its rows and its problems.",
        )
        .example("kind=trader item=wpn_sil_diy"),
        economy_list,
    );
    reg.area("items", "The module's inventory items (.itemasset): what they are and what each resolves to in the game's configs");
    reg.cmd(
        Spec::read(
            "items.list",
            "The module's items: file, new or change, section, family, name, base or target, problems.",
        )
        .example("query=wpn limit=20"),
        items_list,
    );
    reg.cmd(
        Spec::read(
            "items.info",
            "What an item resolves to: every key of its section with its value and where it comes from, its HUD section, the game's values before a change.",
        )
        .example("item=items/sil_diy.itemasset"),
        items_info,
    );
}

fn issue_json(i: &Issue) -> Value {
    json!({
        "code": i.code,
        "severity": match i.severity {
            gz_base::Severity::Error => "error",
            gz_base::Severity::Warning => "warning",
            _ => "note",
        },
        "at": gz_luadata::path::show(&i.path),
        "text": i.message,
    })
}

async fn items_list(ed: Arc<Editor>, p: ListParams, _o: Origin) -> Reply {
    let items = Items::of(&ed).await?;
    let roster = items.fresh_roster(&ed).await;
    let catalog = items.catalog(&ed).await;
    let lookups = items.lookups(&ed).await;
    let root = items.root().to_path_buf();
    let query = p.query.as_deref().map(str::to_lowercase).unwrap_or_default();
    let limit = p.limit.unwrap_or(100).max(1);
    let offset = p.offset.unwrap_or(0);
    let answer = ed
        .exec
        .io(move || {
            let rows: Vec<Value> = roster
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let a = roster.composition.applied.get(i)?;
                    let name = gz_characters::text_in(&e.item.name, "eng");
                    let hay = format!("{} {} {}", e.item.file, a.section, name).to_lowercase();
                    if !query.is_empty() && !hay.contains(&query) {
                        return None;
                    }
                    Some((i, e, a, name))
                })
                .skip(offset)
                .take(limit)
                .map(|(i, e, a, name)| {
                    let issues = crate::service::issues_of(&roster, i, &catalog.catalog, &*lookups, &root);
                    let errors = issues.iter().filter(|x| x.is_error()).count();
                    let res = roster.resolution(i, Some(&catalog.catalog));
                    json!({
                        "file": e.item.file,
                        "kind": kind_word(e.item.kind()),
                        "section": a.section,
                        "family": res.as_ref().and_then(|r| r.family.clone()),
                        "name": name,
                        "base": e.item.base,
                        "edit": e.item.edit,
                        "abstract": e.item.is_abstract(),
                        "left_out": a.left_out,
                        "errors": errors,
                        "problems": issues.iter().filter(|x| x.severity != gz_base::Severity::Note).count(),
                    })
                })
                .collect();
            let total = roster.entries.len();
            json!({
                "total": total,
                "module": roster.module,
                "atlas": (!roster.plan.is_empty()).then(|| json!({
                    "texture": roster.plan.texture,
                    "path": roster.plan.path.replace('\\', "/"),
                    "size": [roster.plan.width, roster.plan.height],
                    "icons": roster.plan.placed.len(),
                })),
                "items": rows,
            })
        })
        .await;
    Ok(answer)
}

fn view_json(v: &SectionView, offset: usize, limit: usize) -> Value {
    let rows: Vec<Value> = v.rows.iter().skip(offset).take(limit).map(|r| json!(r)).collect();
    json!({
        "name": v.name,
        "exists": v.exists,
        "parents": v.parents,
        "total_keys": v.rows.len(),
        "keys": rows,
    })
}

async fn items_info(ed: Arc<Editor>, p: InfoParams, _o: Origin) -> Reply {
    let items = Items::of(&ed).await?;
    let roster = items.fresh_roster(&ed).await;
    let catalog = items.catalog(&ed).await;
    let Some(i) = roster.find(&p.item) else {
        return Err(CmdError::new(
            "not_found",
            format!(
                "'{}' is no item file of the module and no section one of its items makes or changes (items.list lists them)",
                p.item
            ),
        ));
    };
    let Some(res) = roster.resolution(i, Some(&catalog.catalog)) else {
        return Err(CmdError::internal("the item has no resolution"));
    };
    let lookups = items.lookups(&ed).await;
    let root = items.root().to_path_buf();
    let r2 = roster.clone();
    let issues =
        ed.exec.io(move || crate::service::issues_of(&r2, i, &catalog.catalog, &*lookups, &root)).await;
    let (offset, limit) = (p.offset.unwrap_or(0), p.limit.unwrap_or(200).max(1));
    let icon = roster
        .plan
        .of_file(&res.file)
        .map(|pl| json!({ "texture": roster.plan.texture, "x": pl.x, "y": pl.y, "w": pl.w, "h": pl.h }));
    Ok(json!({
        "file": res.file,
        "kind": res.kind,
        "section": res.section,
        "class": res.class,
        "family": res.family,
        "left_out": res.left_out,
        "atlas_icon": icon,
        "main": view_json(&res.main, offset, limit),
        "before": res.before.as_ref().map(|b| view_json(b, offset, limit)),
        "hud": res.hud.as_ref().map(|h| view_json(h, 0, 500)),
        "hud_before": res.hud_before.as_ref().map(|h| view_json(h, 0, 500)),
        "sections": res.sections.iter().map(|s| view_json(s, 0, 500)).collect::<Vec<_>>(),
        "file_patches": res.file_patches,
        "found": roster.found(&res.section),
        "problems": issues.iter().map(issue_json).collect::<Vec<_>>(),
    }))
}

/// The rows of a table file as an answer gives them: what the file says, names as written.
fn rows_json(rows: &Rows, only: Option<&str>) -> Vec<Value> {
    let is = |s: &str| only.is_none_or(|o| s.trim().eq_ignore_ascii_case(o));
    match rows {
        Rows::Trade { stock, .. } => stock
            .iter()
            .enumerate()
            .filter(|(_, s)| is(&s.item))
            .map(|(i, s)| {
                json!({ "row": i, "item": s.item, "supplies": s.sections(), "count": s.count.unwrap_or(1.0),
                        "chance": s.chance.unwrap_or(1.0), "removes": s.removes() })
            })
            .collect(),
        Rows::Loot(drops) => drops
            .iter()
            .enumerate()
            .filter(|(_, d)| is(&d.item))
            .map(|(i, d)| {
                let count = match d.count {
                    None => Value::Null,
                    Some(model::Count::One(n)) => json!(n),
                    Some(model::Count::Range(a, b)) => json!([a, b]),
                };
                let by: serde_json::Map<String, Value> = d.by.iter().map(|(k, c)| (k.clone(), json!(c))).collect();
                json!({ "row": i, "item": d.item, "count": count, "chance": d.chance, "by": by })
            })
            .collect(),
        Rows::Recipes(list) => list
            .iter()
            .enumerate()
            .filter(|(_, c)| is(&c.makes) || is(&c.at) || c.parts.iter().any(|p| is(p)))
            .map(|(i, c)| {
                json!({ "row": i, "makes": c.makes, "count": c.count.unwrap_or(1), "at": c.at, "parts": c.parts })
            })
            .collect(),
    }
}

async fn economy_list(ed: Arc<Editor>, p: EconomyParams, _o: Origin) -> Reply {
    let only_kind = match p.kind.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        None => None,
        Some(word) => Some(
            TableKind::ALL
                .into_iter()
                .find(|k| {
                    word.eq_ignore_ascii_case(k.name())
                        || word.eq_ignore_ascii_case(k.header())
                        || word.eq_ignore_ascii_case(k.rows())
                })
                .ok_or_else(|| CmdError::invalid(format!("kind is trader, loot or recipes, not '{word}'")))?,
        ),
    };
    let items = Items::of(&ed).await?;
    let roster = items.fresh_roster(&ed).await;
    let lookups = items.lookups(&ed).await;
    let item = p.item.map(|i| i.trim().to_ascii_lowercase()).filter(|i| !i.is_empty());
    let answer = ed
        .exec
        .io(move || {
            let mut files = Vec::new();
            for (i, t) in roster.tables.iter().enumerate() {
                let kind = t.table.kind();
                if only_kind.is_some_and(|k| k != kind) {
                    continue;
                }
                let rows = rows_json(&t.table.rows, item.as_deref());
                if item.is_some() && rows.is_empty() {
                    continue;
                }
                let issues = table_issues_of(&roster, i, &*lookups);
                let mut f = json!({
                    "file": t.table.file,
                    "kind": kind.name(),
                    "readable": t.readable,
                    "rows": rows,
                    "total_rows": t.table.rows.len(),
                    "errors": issues.iter().filter(|x| x.is_error()).count(),
                    "problems": issues.iter().map(issue_json).collect::<Vec<_>>(),
                });
                if let (Rows::Trade { trader, .. }, Some(o)) = (&t.table.rows, f.as_object_mut()) {
                    o.insert("trader".into(), json!(trader));
                    o.insert("config".into(), json!(tables::trader_file(trader)));
                }
                files.push(f);
            }
            json!({ "module": roster.module, "total": roster.tables.len(), "files": files })
        })
        .await;
    Ok(answer)
}
