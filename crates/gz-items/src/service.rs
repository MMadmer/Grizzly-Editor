//! What the items area keeps while a project is open: the project's items as one [`Roster`],
//! made once per state of their files, the module's configs, the manifest and the game; the
//! catalog, once per game revision; what the check looks up of the game ([`GameLookups`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gz_base::{RelPath, Rev, RevCounter};
use gz_editor::{CmdError, Editor, Executors};
use gz_game::{Bundled, GameLink, Provenance};
use gz_ltx::{Ltx, config_key};
use gz_mod::ManifestService;
use gz_nq::{NqService, PickerType};
use gz_project::{EntryKind, FileTree, Filter, Project};
use gz_query::Query;
use gz_vfs::GameFiles;
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::catalog::{self, Catalog};
use crate::compose::{ConfigFile, ModuleConfigs};
use crate::model::{self, Issue, Item};
use crate::resolve::Resolution;
use crate::roster::Roster;
use crate::tables::{self, TableFile, TableKind};
use crate::validate::{self, Cx, Lookups, NoGame, TableCx};

/// The extensions whose changes the roster hears about: the items, the tables, the configs, the
/// pictures.
const WATCHED: [&str; 12] = [
    "itemasset",
    "tradeasset",
    "lootasset",
    "recipeasset",
    "ltx",
    "ltxp",
    "png",
    "tga",
    "jpg",
    "jpeg",
    "bmp",
    "dds",
];

/// The catalog the editor read, and from where.
#[derive(Clone, Debug, Default)]
pub struct CatalogRead {
    pub catalog: Catalog,
    /// The linked game's, or the editor's bundled copy; `None`: neither has one.
    pub provenance: Option<Provenance>,
}

/// FNV-1a: stable on every machine.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

/// The items of the open project (`session.services`).
pub struct Items {
    exec: Executors,
    root: PathBuf,
    project: Arc<Project>,
    manifest: Arc<ManifestService>,
    roster: Query<(), Roster>,
    catalog: Query<(), CatalogRead>,
    lookups: Query<(), GameLookups>,
    /// Moves with every change of a watched file the project's watcher reports.
    changes: Arc<RevCounter>,
}

/// The files a roster reads, project-relative: the items, the tables, the module's configs, its
/// patches.
struct Files {
    items: Vec<RelPath>,
    /// The traders, the loot tables and the recipes.
    tables: Vec<RelPath>,
    /// `(project path, path under configs)`.
    configs: Vec<(RelPath, String)>,
    patches: Vec<RelPath>,
}

impl Items {
    /// The items of the open project, made on first use.
    pub async fn of(editor: &Editor) -> Result<Arc<Self>, CmdError> {
        let (session, project) = Project::of(editor)?;
        if let Some(found) = session.services.get::<Self>() {
            return Ok(found);
        }
        let manifest = ManifestService::of_session(editor, &session).await?;
        Ok(session.services.get_or_init(|| Self {
            exec: editor.exec.clone(),
            root: PathBuf::from(project.root()),
            manifest,
            roster: Query::new(editor.exec.clone()),
            catalog: Query::new(editor.exec.clone()),
            lookups: Query::new(editor.exec.clone()),
            changes: follow_files(editor, &project),
            project,
        }))
    }

    /// The items of the open project when they were made already.
    #[must_use]
    pub fn get(editor: &Editor) -> Option<Arc<Self>> {
        editor.project_opt()?.services.get::<Self>()
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn rules(&self) -> Vec<String> {
        self.manifest.snapshot().manifest().map(|m| m.exclude.clone()).unwrap_or_default()
    }

    /// The module's id, as the manifest says it.
    #[must_use]
    pub fn module(&self) -> String {
        self.manifest.snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default()
    }

    fn files(&self, tree: &FileTree, rules: &[String]) -> Files {
        let shipped = |r: &RelPath| !gz_mod::outside_build(rules, r.as_str());
        let items: Vec<RelPath> =
            tree.files_with_extension("itemasset").into_iter().filter(shipped).collect();
        let tables: Vec<RelPath> = TableKind::ALL
            .into_iter()
            .flat_map(|k| tree.files_with_extension(k.extension().trim_start_matches('.')))
            .filter(shipped)
            .collect();
        let mut configs: Vec<(RelPath, String)> = Vec::new();
        let is_ltx = |r: &RelPath| r.as_str().to_ascii_lowercase().ends_with(".ltx");
        if let Ok(under) = RelPath::new("gamedata/configs") {
            for (rel, _) in tree.files_under(&under) {
                if is_ltx(&rel) && shipped(&rel) {
                    let virt = config_key(rel.as_str());
                    configs.push((rel, virt));
                }
            }
        }
        let snap = self.manifest.snapshot();
        if let Some(doc) = snap.doc() {
            for (virtual_path, inside) in doc.section_entries("vfs") {
                let v = config_key(virtual_path);
                let v_full = virtual_path.trim().replace('/', "\\").to_ascii_lowercase();
                if !(v_full == "configs" || v_full.starts_with("configs\\")) {
                    continue;
                }
                let Ok(rel) = RelPath::new(inside.trim()) else { continue };
                if tree.entry(&rel).is_some_and(|e| e.is_file()) {
                    if shipped(&rel) {
                        configs.push((rel, v));
                    }
                    continue;
                }
                for (f, _) in tree.files_under(&rel) {
                    if is_ltx(&f) && shipped(&f) {
                        let tail = f.as_str().get(rel.as_str().len()..).unwrap_or("").trim_start_matches('/');
                        let virt = if v == "configs" || v.is_empty() {
                            config_key(tail)
                        } else {
                            format!("{v}\\{}", config_key(tail))
                        };
                        configs.push((f, virt));
                    }
                }
            }
        }
        configs.sort_by(|a, b| a.1.cmp(&b.1));
        configs.dedup_by(|a, b| a.1 == b.1);
        let patches: Vec<RelPath> = tree
            .files_with_extension("ltxp")
            .into_iter()
            .filter(|r| {
                r.depth() == 2 && r.segments().next().is_some_and(|s| s.eq_ignore_ascii_case("patch"))
            })
            .filter(shipped)
            .collect();
        Files { items, tables, configs, patches }
    }

    /// The stamp of the files: the watcher's count of their changes and their list, or - without
    /// a watcher - their sizes and write times as the tree has them.
    fn stamp_of(&self, tree: &FileTree, files: &Files, rules: &[String], module: &str) -> Rev {
        let mut h = fnv(FNV0, module.as_bytes());
        for r in rules {
            h = fnv(fnv(h, r.as_bytes()), b"\n");
        }
        let live = self.project.tree.changes().live();
        if live {
            h = fnv(h, &self.changes.get().0.to_le_bytes());
        }
        let all = files
            .items
            .iter()
            .chain(&files.tables)
            .chain(files.configs.iter().map(|(r, _)| r))
            .chain(&files.patches);
        for rel in all {
            h = fnv(h, rel.key().as_bytes());
            if live {
                continue;
            }
            if let Some(EntryKind::File { size, modified, .. }) = tree.entry(rel).map(|e| e.kind) {
                h = fnv(h, &size.to_le_bytes());
                let t = modified
                    .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                h = fnv(h, &t.to_le_bytes());
            }
        }
        Rev(h)
    }

    /// The game's tree the items are laid over: the linked game's, else the editor's own; and
    /// its revision.
    async fn game_tree(&self, editor: &Editor) -> (Arc<Ltx>, Rev) {
        if let Ok(link) = GameLink::of(editor)
            && let Ok(ltx) = link.configs().await
        {
            return (ltx, link.rev());
        }
        match Bundled::of(editor).config_tree().await {
            Ok(t) => (t.ltx.clone(), Rev(1)),
            Err(_) => (Arc::new(Ltx::default()), Rev::ZERO),
        }
    }

    /// The roster of the files as the tree has them now: read on the IO pool, composed once per
    /// state of the files, the manifest and the game.
    pub async fn roster(&self, editor: &Editor) -> Arc<Roster> {
        let tree = self.project.tree.snapshot();
        let rules = self.rules();
        let module = self.module();
        let files = self.files(&tree, &rules);
        let (game, game_rev) = self.game_tree(editor).await;
        let stamp = self.stamp_of(&tree, &files, &rules, &module).mix(game_rev);
        if let Some(r) = self.roster.peek_at(&(), stamp) {
            return r;
        }
        let root = self.root.clone();
        let exec = self.exec.clone();
        // the work is the query's own computation: however many ask at once - a check of a folder
        // asks once per file - one of them builds and the rest wait for it
        self.roster
            .fresh(&(), stamp, move || {
                let read = |rel: &RelPath| -> Result<String, String> { model::read_file(&rel.to_fs(&root)) };
                let mut items: Vec<(String, Result<String, String>)> =
                    exec.par_io(files.items.clone(), |rel| (rel.as_str().to_owned(), read(&rel)));
                // the order the game takes them in: of two changes of one key the later path wins
                items.sort_by_cached_key(|(f, _)| gz_base::text::game_order_key(f));
                let table_files: Vec<(String, Result<String, String>)> =
                    exec.par_io(files.tables.clone(), |rel| (rel.as_str().to_owned(), read(&rel)));
                let text_of = |rel: &RelPath| -> Option<String> {
                    gz_editor::fsx::read_opt(&rel.to_fs(&root))
                        .ok()
                        .flatten()
                        .map(|b| gz_base::text::decode_guess(&b).into_owned())
                };
                let mut cfg = ModuleConfigs::default();
                let game_files: FxHashSet<String> = game.files().iter().map(|f| config_key(f)).collect();
                for (rel, virt) in &files.configs {
                    let Some(text) = text_of(rel) else { continue };
                    let f = ConfigFile { rel: rel.as_str().to_owned(), virt: virt.clone(), text };
                    let flat_xms = virt.strip_prefix("xms\\").is_some_and(|rest| !rest.contains('\\'));
                    if flat_xms {
                        cfg.xms.push(f.clone());
                    } else if game_files.contains(virt) {
                        cfg.replaced.push(f.clone());
                    }
                    cfg.all.push(f);
                }
                for rel in &files.patches {
                    if let Some(text) = text_of(rel) {
                        cfg.patches.push(ConfigFile {
                            rel: rel.as_str().to_owned(),
                            virt: String::new(),
                            text,
                        });
                    }
                }
                cfg.xms.sort_by_cached_key(|f| gz_base::text::game_order_key(&f.rel));
                cfg.patches.sort_by_cached_key(|p| gz_base::text::game_order_key(&p.rel));
                Roster::build(
                    stamp,
                    &module,
                    items,
                    table_files,
                    game,
                    &cfg,
                    &root,
                    &crate::atlas::picture_size,
                )
            })
            .await
    }

    /// The roster once the file tree holds every change marked so far: what a command reads
    /// after a file was written.
    pub async fn fresh_roster(&self, editor: &Editor) -> Arc<Roster> {
        self.project.tree.refresh().await;
        self.roster(editor).await
    }

    /// The item catalog: the linked game's, else the editor's bundled copy.
    pub async fn catalog(&self, editor: &Editor) -> Arc<CatalogRead> {
        let rev = GameLink::of(editor).map(|l| l.rev()).unwrap_or(Rev::ZERO);
        if let Some(c) = self.catalog.peek_at(&(), rev) {
            return c;
        }
        let read = gz_game::read_game_or_bundled(editor, catalog::PATH).await;
        self.catalog
            .fresh(&(), rev, move || match read {
                Ok((bytes, provenance)) => CatalogRead {
                    catalog: Catalog::parse(&gz_base::text::decode_guess(&bytes)),
                    provenance: Some(provenance),
                },
                Err(_) => CatalogRead::default(),
            })
            .await
    }

    /// What the check looks up of the linked game and the project now.
    pub async fn lookups(&self, editor: &Arc<Editor>) -> Arc<GameLookups> {
        let roster = self.roster(editor).await;
        let link = GameLink::of(editor).ok();
        let snapshot = match &link {
            Some(l) => l.snapshot().await.ok(),
            None => None,
        };
        let media = match gz_media::Media::of(editor).await {
            Ok(m) => Some(m.index().await),
            Err(_) => None,
        };
        let rev = snapshot
            .as_ref()
            .map_or(Rev::ZERO, |(_, r)| *r)
            .mix(roster.stamp)
            .mix(media.as_ref().map_or(Rev::ZERO, |m| m.stamp));
        if let Some(l) = self.lookups.peek_at(&(), rev) {
            return l;
        }
        let Some((files, _)) = snapshot else {
            let unlinked = GameLookups { root: self.root.clone(), ..GameLookups::default() };
            return self.lookups.fresh(&(), rev, move || unlinked).await;
        };
        // the tables are shared; the set of their ids is made once, by the query below
        let mut tables = Vec::new();
        if let Some(link) = &link {
            let lang = link.language().await.unwrap_or_else(|_| "rus".into());
            let mut langs = vec!["rus".to_owned(), "eng".to_owned()];
            if !langs.contains(&lang) {
                langs.push(lang);
            }
            for l in langs {
                if let Ok(t) = link.strings(&l).await {
                    tables.push(t);
                }
            }
        }
        let (mut communities, mut ranks) = (Vec::new(), Vec::new());
        if let Ok(nq) = NqService::of(editor) {
            let p = nq.pickers(editor).await;
            communities =
                p.entries(PickerType::Community).iter().map(|e| e.id.to_ascii_lowercase()).collect();
            ranks = p.entries(PickerType::Rank).iter().map(|e| e.id.to_ascii_lowercase()).collect();
        }
        let tree = self.project.tree.snapshot();
        let root = self.root.clone();
        let module_atlas = (!roster.plan.is_empty()).then(|| roster.plan.texture.to_ascii_lowercase());
        // computed by the query itself, so many askers at once share one walk of the files
        self.lookups
            .fresh(&(), rev, move || {
                let mut strings: FxHashSet<String> = FxHashSet::default();
                for t in &tables {
                    strings.extend(t.iter().map(|(id, _)| id.to_ascii_lowercase()));
                }
                strings.extend(project_strings(&root, &tree));
                let particles = particle_names(&files, &tree);
                GameLookups {
                    linked: true,
                    files: Some(files),
                    root,
                    media,
                    particles,
                    strings,
                    communities,
                    ranks,
                    module_atlas,
                    motions: OnceMap::default(),
                    sizes: OnceMap::default(),
                }
            })
            .await
    }

    /// The problems of every item of a roster, by entry: what a list shows its dots by. Blocking
    /// parts run on the IO pool.
    pub async fn all_problems(&self, editor: &Arc<Editor>, roster: Arc<Roster>) -> Vec<Vec<Issue>> {
        let catalog = self.catalog(editor).await;
        let lookups = self.lookups(editor).await;
        let root = self.root.clone();
        self.exec
            .io(move || {
                (0..roster.entries.len())
                    .map(|i| issues_of(&roster, i, &catalog.catalog, &*lookups, &root))
                    .collect()
            })
            .await
    }

    /// What an item resolves to: `item` a file of the project or a section an item makes or
    /// changes.
    pub async fn resolution(&self, editor: &Editor, item: &str) -> Option<Resolution> {
        let roster = self.fresh_roster(editor).await;
        let catalog = self.catalog(editor).await;
        let i = roster.find(item)?;
        roster.resolution(i, Some(&catalog.catalog))
    }

    /// Every problem of an item file of the roster, positioned by path: the reader's, the
    /// composition's and the check's, the game looked up. Blocking parts run on the IO pool.
    pub async fn problems(&self, editor: &Arc<Editor>, file: &str) -> Option<Vec<Issue>> {
        let roster = self.fresh_roster(editor).await;
        let i = roster.index_of_file(file)?;
        let catalog = self.catalog(editor).await;
        let lookups = self.lookups(editor).await;
        let root = self.root.clone();
        Some(self.exec.io(move || issues_of(&roster, i, &catalog.catalog, &*lookups, &root)).await)
    }

    /// Every problem of a table file of the roster - a trader, a loot table, recipes -,
    /// positioned by path: the reader's and the check's, the game looked up.
    pub async fn table_problems(&self, editor: &Arc<Editor>, file: &str) -> Option<Vec<Issue>> {
        let roster = self.fresh_roster(editor).await;
        let i = roster.table_index(file)?;
        let lookups = self.lookups(editor).await;
        Some(self.exec.io(move || table_issues_of(&roster, i, &*lookups)).await)
    }

    /// The problems of every table file of a roster, by entry.
    pub async fn all_table_problems(&self, editor: &Arc<Editor>, roster: Arc<Roster>) -> Vec<Vec<Issue>> {
        let lookups = self.lookups(editor).await;
        self.exec
            .io(move || (0..roster.tables.len()).map(|i| table_issues_of(&roster, i, &*lookups)).collect())
            .await
    }

    /// Saves an edited table file: the difference between the file as it was read (`before`) and
    /// as it is now, written into the file's own text - comments, layout and keys the editor does
    /// not know stay. A file that does not exist yet is written canonically. Answers the file,
    /// project-relative.
    pub async fn save_table(
        &self,
        editor: &Editor,
        before: Option<&TableFile>,
        after: &TableFile,
    ) -> Result<String, CmdError> {
        let file = after.file.as_str();
        let kind = after.kind();
        if TableKind::of_file(file) != Some(kind) {
            return Err(CmdError::invalid(format!(
                "{file}: a {} is a {} file",
                kind.name(),
                kind.extension()
            )));
        }
        let rel = RelPath::new(file).map_err(|e| CmdError::invalid(format!("{file}: {e}")))?;
        let abs = rel.to_fs(&self.root);
        let exists = self.exec.io(move || abs.is_file()).await;
        if exists {
            let mut f = gz_asset::read(editor, &rel).await?;
            let old = match before {
                Some(b) => tables::to_value(b),
                None => tables::from_value(kind, &f.doc.value())
                    .map(|(t, _)| tables::to_value(&t))
                    .map_err(CmdError::invalid)?,
            };
            f.apply(&old, &tables::to_value(after))?;
            gz_asset::write(editor, &mut f).await?;
        } else {
            gz_asset::write_text(editor, &rel, tables::write(after)).await?;
        }
        self.project.tree.refresh().await;
        self.changes.bump();
        Ok(rel.as_str().to_owned())
    }

    /// What an item a form edits resolves to and every problem of it, as the build would see the
    /// module with the item written into its file (`item.file`, a file of the roster or a new
    /// one): that one file read and the composition made again on the CPU pool, what the game is
    /// asked for the problems on the IO pool.
    pub async fn check_edit(&self, editor: &Arc<Editor>, item: &Item) -> (Option<Resolution>, Vec<Issue>) {
        let roster = self.roster(editor).await;
        let catalog = self.catalog(editor).await;
        let lookups = self.lookups(editor).await;
        let root = self.root.clone();
        let (file, text) = (item.file.clone(), model::write(item));
        let edited = {
            let (file, root) = (file.clone(), root.clone());
            self.exec.cpu(move || roster.with_edit(&file, text, &root, &crate::atlas::picture_size)).await
        };
        self.exec
            .io(move || {
                let Some(i) = edited.index_of_file(&file) else { return (None, Vec::new()) };
                let res = edited.resolution(i, Some(&catalog.catalog));
                (res, issues_of(&edited, i, &catalog.catalog, &*lookups, &root))
            })
            .await
    }

    /// Saves an edited item into its file: the difference between the item as it was read
    /// (`before`) and as it is now, written into the file's own text - comments, layout and keys
    /// the editor does not know stay. A file that does not exist yet is written canonically.
    /// Answers the file, project-relative.
    pub async fn save(
        &self,
        editor: &Editor,
        file: &str,
        before: Option<&Item>,
        after: &Item,
    ) -> Result<String, CmdError> {
        let rel = RelPath::new(file).map_err(|e| CmdError::invalid(format!("{file}: {e}")))?;
        let abs = rel.to_fs(&self.root);
        let exists = self.exec.io(move || abs.is_file()).await;
        if exists {
            let mut f = gz_asset::read(editor, &rel).await?;
            let old = match before {
                Some(b) => model::to_value(b),
                None => model::from_value(&f.doc.value())
                    .map(|(i, _)| model::to_value(&i))
                    .map_err(CmdError::invalid)?,
            };
            f.apply(&old, &model::to_value(after))?;
            gz_asset::write(editor, &mut f).await?;
        } else {
            gz_asset::write_text(editor, &rel, model::write(after)).await?;
        }
        self.project.tree.refresh().await;
        // the roster that follows reads the file as written, before the watcher reports it
        self.changes.bump();
        Ok(rel.as_str().to_owned())
    }
}

/// The problems of entry `i`: the reader's, then the check's (which carries the
/// composition's). Blocking.
#[must_use]
pub fn issues_of(
    roster: &Roster,
    i: usize,
    catalog: &Catalog,
    lookups: &dyn Lookups,
    root: &Path,
) -> Vec<Issue> {
    let Some(e) = roster.entries.get(i) else { return Vec::new() };
    let mut out = e.issues.clone();
    // what every item is read over: said once, on the first item
    if i == 0 {
        for (file, why) in &roster.composition.problems {
            out.push(Issue::warning(
                crate::compose::BAD_CONFIG,
                Vec::new(),
                format!("the module's config {file} does not compose, so what the items resolve to may differ: {why}"),
            ));
        }
    }
    if !e.readable {
        return out;
    }
    let Some(applied) = roster.composition.applied.get(i) else { return out };
    let cx = Cx {
        catalog,
        lookups,
        root,
        module: &roster.module,
        module_strings: &roster.strings,
        game: Some(&roster.game),
    };
    out.extend(validate::validate(&e.item, applied, &roster.composition, &cx));
    out
}

/// The problems of table entry `i`: the reader's, then the check's. Blocking.
#[must_use]
pub fn table_issues_of(roster: &Roster, i: usize, lookups: &dyn Lookups) -> Vec<Issue> {
    let Some(t) = roster.tables.get(i) else { return Vec::new() };
    let mut out = t.issues.clone();
    if !t.readable || out.iter().any(Issue::is_error) {
        return out;
    }
    let counted = roster.counted();
    let cx = TableCx { lookups, game: Some(&roster.game), counted: &counted };
    out.extend(validate::validate_table(&t.table, &roster.composition, &cx));
    out
}

/// The problems the build refuses a module for: the first error of any item or table file, the
/// game not looked up (what a build needs is in the module). Blocking.
#[must_use]
pub fn build_errors(roster: &Roster, catalog: &Catalog, root: &Path) -> Vec<(String, Issue)> {
    let mut out = Vec::new();
    for (i, e) in roster.entries.iter().enumerate() {
        for issue in issues_of(roster, i, catalog, &NoGame, root).into_iter().filter(Issue::is_error) {
            out.push((e.item.file.clone(), issue));
        }
    }
    for (i, t) in roster.tables.iter().enumerate() {
        for issue in table_issues_of(roster, i, &NoGame).into_iter().filter(Issue::is_error) {
            out.push((t.table.file.clone(), issue));
        }
    }
    out
}

/// A counter of the watched files' changes, moved by the project's watcher once the tree holds
/// them.
fn follow_files(editor: &Editor, project: &Arc<Project>) -> Arc<RevCounter> {
    let counter = Arc::new(RevCounter::new());
    let mut rx =
        project.tree.changes().subscribe(Filter::Extensions(WATCHED.iter().map(|e| (*e).into()).collect()));
    let (weak, tree) = (Arc::downgrade(&counter), Arc::downgrade(project));
    editor.exec.spawn(async move {
        while rx.recv().await.is_some() {
            if let Some(p) = tree.upgrade() {
                p.tree.refresh().await;
            }
            match weak.upgrade() {
                Some(c) => {
                    c.bump();
                }
                None => break,
            }
        }
    });
    counter
}

// ---- the lookups ---------------------------------------------------------------------------

/// The string ids of the project's own string tables (`gamedata\configs\text\<lang>\*.xml`).
fn project_strings(root: &Path, tree: &FileTree) -> Vec<String> {
    let Ok(under) = RelPath::new("gamedata/configs/text") else { return Vec::new() };
    let mut out = Vec::new();
    for (rel, _) in tree.files_under(&under) {
        if !rel.as_str().to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        let Ok(Some(bytes)) = gz_editor::fsx::read_opt(&rel.to_fs(root)) else { continue };
        let text = gz_fmt_xml::decode(&bytes).into_owned();
        let name = rel.as_str().to_owned();
        let (table, _) =
            gz_fmt_xml::strings::StringTable::build(&[(&name, &text)], gz_fmt_xml::strings::Mode::Editor);
        out.extend(table.iter().map(|(id, _)| id.to_ascii_lowercase()));
    }
    out
}

/// The particle effects and groups the game's `particles.xr` holds, and the loose ones of the
/// game and the project (`particles\<name>.pe` / `.pg`), lower case.
fn particle_names(files: &GameFiles, tree: &FileTree) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    if let Ok(bytes) = files.read("particles.xr")
        && let Ok(lib) = gz_fmt_xr::Particles::read(&bytes)
    {
        out.extend(lib.effects.iter().map(|e| e.name.to_ascii_lowercase()));
        out.extend(lib.groups.iter().map(|g| g.name.to_ascii_lowercase()));
    }
    let loose = |path: &str| {
        let p = path.replace('/', "\\").to_ascii_lowercase();
        let p = p.strip_prefix("gamedata\\").unwrap_or(&p).to_owned();
        let p = p.strip_prefix("particles\\")?.to_owned();
        p.strip_suffix(".pe").or_else(|| p.strip_suffix(".pg")).map(str::to_owned)
    };
    for f in files.find("particles", "pe").into_iter().chain(files.find("particles", "pg")) {
        out.extend(loose(f.path));
    }
    if let Ok(under) = RelPath::new("gamedata/particles") {
        for (rel, _) in tree.files_under(&under) {
            out.extend(loose(rel.as_str()));
        }
    }
    out
}

/// A game name as a path keys it: back slashes, lower case, without the extension `ext`.
fn name_key(name: &str, ext: &str) -> String {
    let n = name.trim().replace('/', "\\").to_ascii_lowercase();
    let n = n.trim_start_matches('\\');
    n.strip_suffix(ext).unwrap_or(n).to_owned()
}

/// A value worked out once per name: the first asker computes it and the rest wait for it, so
/// many checks at once read a file of the game once.
struct OnceMap<V>(Mutex<FxHashMap<String, Arc<std::sync::OnceLock<V>>>>);

impl<V> Default for OnceMap<V> {
    fn default() -> Self {
        Self(Mutex::default())
    }
}

impl<V: Clone> OnceMap<V> {
    fn get(&self, key: &str, make: impl FnOnce() -> V) -> V {
        let cell = self.0.lock().entry(key.to_owned()).or_default().clone();
        cell.get_or_init(make).clone()
    }
}

/// What the check looks up of the linked game and the project.
#[derive(Default)]
pub struct GameLookups {
    pub linked: bool,
    files: Option<Arc<GameFiles>>,
    root: PathBuf,
    media: Option<Arc<gz_media::Index>>,
    particles: FxHashSet<String>,
    strings: FxHashSet<String>,
    communities: Vec<String>,
    ranks: Vec<String>,
    /// The module's own atlas, when its items make one.
    module_atlas: Option<String>,
    motions: OnceMap<Option<Vec<String>>>,
    sizes: OnceMap<Option<(u32, u32)>>,
}

impl GameLookups {
    fn game_has(&self, path: &str) -> bool {
        self.files.as_ref().is_some_and(|f| f.exists(path))
    }

    fn project_has(&self, path: &str) -> bool {
        self.root.join("gamedata").join(path).is_file()
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let local = self.root.join("gamedata").join(path);
        if let Ok(Some(b)) = gz_editor::fsx::read_opt(&local) {
            return Some(b);
        }
        self.files.as_ref().and_then(|f| f.read(path).ok())
    }

    fn asset(&self, kind: gz_project::asset_kind::MediaKind, name: &str) -> bool {
        self.media.as_ref().is_some_and(|m| m.by_name(kind, name).is_some())
    }

    fn motions_of_model(&self, visual: &str) -> Option<Vec<String>> {
        let key = name_key(visual, ".ogf");
        let bytes = self.read(&format!("meshes\\{key}.ogf"))?;
        let v = gz_fmt_ogf::Visual::read(&bytes).ok()?;
        let mut out: Vec<String> = v
            .motions
            .as_ref()
            .map(|m| m.motions.iter().map(|x| x.name.to_string()).collect())
            .unwrap_or_default();
        for r in &v.motion_refs {
            let omf = name_key(r, ".omf");
            if let Some(b) = self.read(&format!("meshes\\{omf}.omf"))
                && let Ok(set) = gz_fmt_ogf::read_omf(&b)
            {
                out.extend(set.motions.iter().map(|x| x.name.to_string()));
            }
        }
        // a model without motions of its own says nothing about the names
        (!out.is_empty() || !v.motion_refs.is_empty()).then_some(out)
    }
}

impl Lookups for GameLookups {
    fn linked(&self) -> bool {
        self.linked
    }

    fn model(&self, name: &str) -> bool {
        let key = name_key(name, ".ogf");
        let path = format!("meshes\\{key}.ogf");
        self.game_has(&path)
            || self.project_has(&path)
            || self.asset(gz_project::asset_kind::MediaKind::Model, &key)
    }

    fn texture(&self, name: &str) -> bool {
        let key = name_key(name, ".dds");
        let path = format!("textures\\{key}.dds");
        self.game_has(&path)
            || self.project_has(&path)
            || self.asset(gz_project::asset_kind::MediaKind::Texture, &key)
            || self.module_atlas.as_deref() == Some(key.as_str())
    }

    fn texture_size(&self, name: &str) -> Option<(u32, u32)> {
        let key = name_key(name, ".dds");
        // every item asks for the size of the sheet its icon lies on, and a sheet is tens of
        // megabytes: read once, and of a file of the project only its head
        self.sizes.get(&key, || {
            let path = format!("textures\\{key}.dds");
            let head = match std::fs::File::open(self.root.join("gamedata").join(&path)) {
                Ok(f) => {
                    let mut head = Vec::new();
                    std::io::Read::read_to_end(&mut std::io::Read::take(f, 256), &mut head).ok()?;
                    head
                }
                Err(_) => self.files.as_ref()?.read(&path).ok()?,
            };
            gz_fmt_tex::read_dds_info(&head).ok().map(|i| (i.width, i.height))
        })
    }

    fn sound(&self, name: &str) -> bool {
        let key = name_key(name, ".ogg");
        let path = format!("sounds\\{key}.ogg");
        self.game_has(&path)
            || self.project_has(&path)
            || self.asset(gz_project::asset_kind::MediaKind::Sound, &key)
    }

    fn particle(&self, name: &str) -> bool {
        self.particles.contains(&name_key(name, ".pe"))
    }

    fn string_id(&self, id: &str) -> bool {
        self.strings.contains(&id.trim().to_ascii_lowercase())
    }

    fn community(&self, name: &str) -> bool {
        let n = name.trim().to_ascii_lowercase();
        if self.communities.contains(&n) {
            return true;
        }
        self.communities.iter().any(|c| {
            n.strip_prefix(c.as_str())
                .and_then(|r| r.strip_prefix('_'))
                .is_some_and(|rank| self.ranks.iter().any(|x| x == rank))
        })
    }

    fn config_sections(&self, file: &str) -> Option<Vec<String>> {
        let path = format!("configs\\{}", config_key(file));
        let bytes = self.read(&path)?;
        let text = gz_base::text::decode_guess(&bytes);
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim_start();
            if let Some(rest) = line.strip_prefix('[')
                && let Some(end) = rest.find(']')
            {
                out.push(rest.get(..end).unwrap_or("").trim().to_ascii_lowercase());
            }
        }
        Some(out)
    }

    fn motions(&self, visual: &str) -> Option<Vec<String>> {
        let key = name_key(visual, ".ogf");
        self.motions.get(&key, || self.motions_of_model(&key))
    }

    fn picture(&self, path: &Path) -> Result<(u32, u32), String> {
        crate::atlas::picture_size(path)
    }
}
