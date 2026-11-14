//! The PDA preview: the model one tab and `pda.preview` share, the
//! operations on it, the tab ([`doc`]), its canvas ([`canvas`]) and the command ([`cmd`]).
//!
//! The model is a service of the open project behind one lock: the file, the Source pane's
//! text, what the game's reader made of it (`gz_pda::evaluate`, on the CPU pool) and the
//! running interface (`gz_pda::Preview`), plus the tab's view and what its last frame drew. The
//! tab draws from it and MCP works on it without the window; every read of the file, every
//! write and every reading of the layout runs off the UI thread.

pub mod canvas;
pub mod cmd;
pub mod doc;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use egui::{Pos2, Vec2};
use gz_base::Origin;
use gz_editor::{DirtyDoc, DirtySource, Editor, ProjectSession, SmolStr};
use gz_mod::ManifestService;
use gz_pda::{Layout, PdaService, Preview, ReadError, base_name, project_relative};
use gz_project::{Filter, Project};
use parking_lot::Mutex;

use crate::assets::PdaAssets;
use crate::gamefile::FRect;

/// The Details tabs, as MCP names them.
pub const TAB_IDS: [&str; 4] = ["problems", "pages", "vars", "log"];

/// pt (docs/UI_DESIGN.md, sections 2 and 6): the Details pane, the Source pane and what the
/// canvas keeps.
pub const DETAILS_W: f32 = 320.0;
pub const SIDE_MIN: f32 = 220.0;
pub const CANVAS_MIN_W: f32 = 400.0;
pub const CANVAS_MIN_H: f32 = 260.0;
pub const SOURCE_H: f32 = 240.0;
pub const SOURCE_MIN: f32 = 120.0;

/// Where the device sat on the last frame: the screen's corner and the points per game unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    /// pt: where the game's (0, 0) is.
    pub origin: Pos2,
    /// pt per game unit (1024 x 768 units).
    pub kx: f32,
    pub ky: f32,
    /// The device's screen, in game units.
    pub screen: FRect,
}

impl Geom {
    /// One point per game unit at the window's corner: what a preview no frame drew answers in.
    #[must_use]
    pub fn unit(screen: FRect) -> Self {
        Self { origin: Pos2::ZERO, kx: 1.0, ky: 1.0, screen }
    }

    /// A game point.
    #[must_use]
    pub fn at(&self, vx: f32, vy: f32) -> Pos2 {
        Pos2::new(self.origin.x + vx * self.kx, self.origin.y + vy * self.ky)
    }

    /// Where the canvas' (0, 0) is.
    #[must_use]
    pub fn root(&self) -> Pos2 {
        self.at(self.screen.x1, self.screen.y1)
    }

    /// Canvas units to points, per axis (the game's scale, then the view's).
    #[must_use]
    pub fn cx(&self, canvas_w: f32) -> f32 {
        if canvas_w > 0.0 { self.screen.width() / canvas_w * self.kx } else { self.kx }
    }

    #[must_use]
    pub fn cy(&self, canvas_h: f32) -> f32 {
        if canvas_h > 0.0 { self.screen.height() / canvas_h * self.ky } else { self.ky }
    }
}

/// The preview of one layout: the file, the Source pane's text, the reader's answer and the
/// running interface; the tab's view; what the tab's last frame drew.
pub struct Model {
    /// The tab is there (or asked for).
    pub open: bool,
    /// The newest view of the tab: an older one closing does not close the tab.
    pub view_serial: u64,
    /// Absolute, `\`.
    pub path: String,
    /// Project-relative, `/`.
    pub rel: String,
    /// What the Source pane edits.
    pub text: String,
    /// What the file holds, as last read or written.
    pub disk: String,
    /// What stopped the save a command's caller asked for: that caller's answer.
    pub save_error: Option<gz_editor::CmdError>,
    pub text_dirty: bool,
    /// What the file on disk did that the Source pane's edits keep out of view: empty, or the
    /// sentence the tab shows with Take Theirs.
    pub disk_note: String,
    pub read_ok: bool,
    pub read_err: String,
    pub read_line: u32,
    /// Where the reader came from: `game:...` or `bundled:...`.
    pub reader_source: String,
    pub preview: Preview,
    pub read_seq: u64,

    pub show_source: bool,
    pub show_side: bool,
    /// 0 = fit, else x the 768 lines of the game's PDA in pixels.
    pub zoom: f32,
    /// pt: where a zoomed device sits off its centred place.
    pub pan: Vec2,
    /// The Details tab on show.
    pub tab: usize,
    /// A tab asked for over MCP, selected when Details draws next.
    pub want_tab: Option<usize>,
    /// The Source pane scrolls to this line when it draws next.
    pub goto_line: u32,
    /// The preview map's centre in world units (x, z).
    pub map_center: (f32, f32),
    pub hovered: Option<usize>,
    pub pressed: Option<usize>,

    /// The egui frame that drew the tab last.
    pub drawn_frame: Option<u64>,
    pub geom: Option<Geom>,
    /// Pixels per point of that frame.
    pub ppp: f32,
    /// pt, as the last frame laid them out.
    pub details_w: f32,
    pub source_h: f32,
    /// pt: the device's nearest distance to a canvas edge; below 0 when cut.
    pub device_gap: f32,
    /// The device's sides on the game's screen edge: 1 top, 2 bottom, 4 left, 8 right.
    pub screen_edges: u8,
}

impl Model {
    fn new() -> Self {
        Self {
            open: false,
            view_serial: 0,
            path: String::new(),
            rel: String::new(),
            text: String::new(),
            disk: String::new(),
            save_error: None,
            text_dirty: false,
            disk_note: String::new(),
            read_ok: false,
            read_err: String::new(),
            read_line: 0,
            reader_source: String::new(),
            preview: Preview::new(Arc::new(Layout::default())),
            read_seq: 0,
            show_source: false,
            show_side: true,
            zoom: 0.0,
            pan: Vec2::ZERO,
            tab: 0,
            want_tab: None,
            goto_line: 0,
            map_center: (0.0, 0.0),
            hovered: None,
            pressed: None,
            drawn_frame: None,
            geom: None,
            ppp: 1.0,
            details_w: 0.0,
            source_h: 0.0,
            device_gap: 0.0,
            screen_edges: 0,
        }
    }

    /// The file as the tab names it: project-relative, or its base name.
    #[must_use]
    pub fn shown_name(&self) -> String {
        if self.rel.is_empty() { base_name(&self.path) } else { self.rel.clone() }
    }

    /// Back to the start page with every var and panel as the file declares them.
    pub fn reset_state(&mut self) {
        self.preview.reset();
        self.hovered = None;
        self.pressed = None;
    }

    /// What a reading of the text gives: the layout when it read; the page stays when
    /// `keep` and the layout still has it - an edit should not throw the author back.
    pub fn apply_read(&mut self, res: Result<Arc<Layout>, ReadError>, keep: bool) {
        let page = self.preview.page.clone();
        match res {
            Ok(layout) => {
                self.read_ok = true;
                self.read_err.clear();
                self.read_line = 0;
                self.preview.layout = layout;
            }
            Err(e) => {
                self.read_ok = false;
                self.read_err = e.message;
                self.read_line = e.line;
            }
        }
        if !keep || self.preview.layout.find_page(&page).is_none() {
            self.reset_state();
        } else {
            // the item list may have changed shape: index-keyed state goes
            self.preview.vis.clear();
            self.preview.row_sel.clear();
            self.preview.scroll.clear();
        }
    }

    /// The geometry MCP answers in: the last frame's, or one point per game unit when no frame
    /// drew the tab.
    #[must_use]
    pub fn geometry(&self, screen: FRect) -> Geom {
        match self.geom {
            Some(g) if self.open && g.screen.width() > 0.0 => g,
            _ => Geom::unit(screen),
        }
    }
}

/// The preview of the open project (`session.services`).
pub struct PdaTab {
    /// The project folder.
    pub root: String,
    pub m: Mutex<Model>,
    /// Which follower of the file is the current one: a file loaded starts another.
    follow: AtomicU64,
}

/// The unsaved Source text, for everything that asks before work is lost.
struct PdaDirty;

impl DirtySource for PdaDirty {
    fn dirty(&self, editor: &Editor) -> Vec<DirtyDoc> {
        let Some(session) = editor.project_opt() else { return Vec::new() };
        let Some(tab) = session.services.get::<PdaTab>() else { return Vec::new() };
        let m = tab.m.lock();
        if !m.text_dirty || !m.open {
            return Vec::new();
        }
        vec![DirtyDoc { kind: SmolStr::new("pda"), name: m.shown_name(), path: Some(m.rel.clone()) }]
    }

    fn save<'a>(&'a self, editor: &'a Arc<Editor>) -> gz_editor::SettleFuture<'a> {
        Box::pin(async move {
            let Some(tab) = editor.project_opt().and_then(|s| s.services.get::<PdaTab>()) else {
                return Ok(());
            };
            let name = tab.m.lock().shown_name();
            save_and_read(editor, &tab).await.map_err(|e| format!("{name}: {e}"))
        })
    }

    fn discard<'a>(&'a self, editor: &'a Arc<Editor>) -> gz_editor::SettleFuture<'a> {
        Box::pin(async move {
            if let Some(tab) = editor.project_opt().and_then(|s| s.services.get::<PdaTab>()) {
                let mut m = tab.m.lock();
                m.text = m.disk.clone();
                m.text_dirty = false;
                m.disk_note.clear();
            }
            wake(editor);
            Ok(())
        })
    }
}

#[derive(Default)]
struct DirtyRegistered(AtomicBool);

impl PdaTab {
    /// The preview of the session, made on first use.
    pub fn of_session(editor: &Editor, session: &ProjectSession) -> Arc<Self> {
        let flag = editor.services.get_or_init(DirtyRegistered::default);
        if !flag.0.swap(true, Ordering::SeqCst) {
            editor.add_dirty_source(Arc::new(PdaDirty));
            // asset.take_theirs reaches the tab: the one open document of the kind
            let kinds = gz_asset::Kinds::of(editor);
            if let Some(ops) = kinds.of_file("layout.pdaasset") {
                let mut ops = (*ops).clone();
                ops.take_theirs = Some(|ed, rel, _| Box::pin(take_theirs_of(ed, rel)));
                kinds.add(ops);
            }
        }
        let root = session.root.display().to_string();
        session.services.get_or_init(|| PdaTab {
            root,
            m: Mutex::new(Model::new()),
            follow: AtomicU64::new(0),
        })
    }
}

fn wake(editor: &Editor) {
    editor.exec.wake().ping();
}

/// Reads the Source pane's text with the game's reader; a newer reading started
/// meanwhile wins.
pub async fn read(editor: &Arc<Editor>, tab: &Arc<PdaTab>, keep: bool) {
    let Ok(session) = editor.project() else { return };
    let (seq, text, chunk, lang) = {
        let mut m = tab.m.lock();
        m.read_seq += 1;
        (m.read_seq, Arc::<str>::from(m.text.as_str()), base_name(&m.path), m.preview.lang.clone())
    };
    let service = PdaService::of_session(editor, &session);
    // a layout of a newer format opens nothing: the read path says why
    let res = match newer_version(editor, &text) {
        Some(why) => Err(ReadError { message: why.0, line: why.1 }),
        None => service.evaluate(editor, text, &chunk, &lang).await,
    };
    let source = service.reader_source(editor).await;
    {
        let mut m = tab.m.lock();
        if m.read_seq != seq {
            return;
        }
        m.reader_source = source;
        m.apply_read(res, keep);
    }
    wake(editor);
}

/// The sentence of a text of a newer format than this editor reads, with its line.
fn newer_version(editor: &Editor, text: &str) -> Option<(String, u32)> {
    let ops = gz_asset::Kinds::of(editor).of_file("layout.pdaasset")?;
    let rel = gz_base::RelPath::new("layout.pdaasset").ok()?;
    match gz_asset::read_text(&ops, rel, std::path::PathBuf::new(), text, &[]) {
        Err(f) if f.code == "newer_version" => {
            let line = f.problem.line.unwrap_or(0);
            let why = f.problem.message.replacen("layout.pdaasset was", "this layout was", 1);
            Some((why, line))
        }
        _ => None,
    }
}

/// Reads a file into the preview: the Source pane gets its text, clean. The page
/// stays when it is the same file. `false` when the file cannot be read. A new file is
/// followed: the project's watcher tells the tab when it changes outside the editor.
pub async fn load_file(editor: &Arc<Editor>, tab: &Arc<PdaTab>, abs: &str) -> bool {
    let p = abs.to_owned();
    let Ok(text) = editor.exec.io(move || gz_pda::read_text(Path::new(&p))).await else { return false };
    let (same, rel) = {
        let mut m = tab.m.lock();
        let same = m.path == abs;
        m.path = abs.to_owned();
        m.rel = project_relative(&tab.root, abs);
        m.text.clone_from(&text);
        m.disk = text;
        m.text_dirty = false;
        m.disk_note.clear();
        (same, m.rel.clone())
    };
    if !same {
        follow_file(editor, tab, &rel);
    }
    read(editor, tab, same).await;
    true
}

/// Follows the tab's file through the project's watcher until another file is loaded.
fn follow_file(editor: &Arc<Editor>, tab: &Arc<PdaTab>, rel: &str) {
    let generation = tab.follow.fetch_add(1, Ordering::AcqRel) + 1;
    let Ok((_, project)) = Project::of(editor) else { return };
    let Ok(rel) = gz_base::RelPath::new(rel) else { return };
    let mut rx = project.tree.changes().subscribe(Filter::Path(rel));
    let (ed, weak) = (editor.clone(), Arc::downgrade(tab));
    editor.exec.spawn(async move {
        while rx.recv().await.is_some() {
            let Some(tab) = weak.upgrade() else { break };
            if tab.follow.load(Ordering::Acquire) != generation {
                break;
            }
            follow(&ed, &tab).await;
        }
    });
}

/// Writes the Source pane's text to the file.
pub async fn save_text(editor: &Arc<Editor>, tab: &Arc<PdaTab>) -> Result<(), String> {
    let point = save_point(tab);
    write_point(editor, tab, point).await
}

/// What a save writes - the file and the text - as they are now. Taken when the save is asked
/// for: by the time the write runs the tab may show another file.
#[must_use]
pub fn save_point(tab: &PdaTab) -> (String, String) {
    let m = tab.m.lock();
    (m.path.clone(), m.text.clone())
}

/// Writes a [`save_point`]; the tab is clean once its file holds the text it shows.
pub async fn write_point(
    editor: &Arc<Editor>,
    tab: &Arc<PdaTab>,
    (path, text): (String, String),
) -> Result<(), String> {
    if path.is_empty() {
        return Err("no PDA layout is open".to_owned());
    }
    // the author's text, written as it is: past the write guard, as the editor's own write
    let rel = gz_base::RelPath::new(&project_relative(&tab.root, &path))
        .map_err(|_| format!("{path} is outside the project: it cannot be saved from here"))?;
    // a file that holds the text already is not written again
    let (p, t) = (path.clone(), text.clone());
    let same = editor.exec.io(move || std::fs::read(&p).is_ok_and(|b| b == t.as_bytes())).await;
    if !same {
        gz_asset::write_text(editor, &rel, text.clone()).await.map_err(|e| e.message)?;
    }
    {
        let mut m = tab.m.lock();
        if m.path == path {
            if m.text == text {
                m.text_dirty = false;
            }
            m.disk = text;
            m.disk_note.clear();
        }
    }
    wake(editor);
    Ok(())
}

/// Saves the text and reads it again: the toolbar's Save, Ctrl+S, the shell's save.
pub async fn save_and_read(editor: &Arc<Editor>, tab: &Arc<PdaTab>) -> Result<(), String> {
    let point = save_point(tab);
    save_point_and_read(editor, tab, point).await
}

/// [`save_and_read`] of a [`save_point`] taken earlier.
pub async fn save_point_and_read(
    editor: &Arc<Editor>,
    tab: &Arc<PdaTab>,
    point: (String, String),
) -> Result<(), String> {
    write_point(editor, tab, point).await?;
    read(editor, tab, true).await;
    Ok(())
}

/// The game's textures, fonts and reader are read again; the file too, unless the Source pane
/// holds edits.
pub async fn reload_all(editor: &Arc<Editor>, tab: &Arc<PdaTab>) {
    if let Ok(session) = editor.project() {
        PdaService::of_session(editor, &session).drop_reader();
        PdaAssets::of_session(&session).invalidate();
    }
    let (dirty, path) = {
        let m = tab.m.lock();
        (m.text_dirty, m.path.clone())
    };
    if dirty || path.is_empty() {
        read(editor, tab, true).await;
    } else {
        load_file(editor, tab, &path).await;
    }
}

/// The file follows the disk: written behind the preview (an agent, another editor), it is
/// read again; with edits in the Source pane the two are merged ([`merge`]). Gone, the tab goes
/// - unless it holds edits, which Save writes again.
pub async fn follow(editor: &Arc<Editor>, tab: &Arc<PdaTab>) {
    let (path, disk, dirty) = {
        let m = tab.m.lock();
        if m.path.is_empty() {
            return;
        }
        (m.path.clone(), m.disk.clone(), m.text_dirty)
    };
    let p = path.clone();
    let now = editor
        .exec
        .io(move || if Path::new(&p).is_file() { Some(gz_pda::read_text(Path::new(&p)).ok()) } else { None })
        .await;
    match now {
        None => {
            let mut m = tab.m.lock();
            if m.path == path && !m.text_dirty {
                m.open = false;
                m.path.clear();
                m.rel.clear();
            } else if m.path == path {
                m.disk_note = format!("{} is gone from the disk. Save writes it again.", m.shown_name());
            }
            drop(m);
            wake(editor);
        }
        Some(Some(text)) if text != disk => {
            if dirty {
                merge(editor, tab, &path, disk, text).await;
                return;
            }
            let still = {
                let m = tab.m.lock();
                m.path == path && !m.text_dirty
            };
            if still {
                load_file(editor, tab, &path).await;
            }
        }
        Some(_) => {}
    }
}

/// The file changed on disk while the Source pane holds edits: the values the pane changed go
/// into the file's new text, and the file's other changes stay; a value both changed keeps the
/// pane's and is counted in the note. When one of the texts does not read nothing can be
/// merged: the pane keeps its text and the note says so. Take Theirs takes the file's text.
async fn merge(editor: &Arc<Editor>, tab: &Arc<PdaTab>, path: &str, base: String, theirs: String) {
    let mine = tab.m.lock().text.clone();
    let Some(ops) = gz_asset::Kinds::of(editor).of_file("layout.pdaasset") else { return };
    let (name, text) = (base_name(path), theirs.clone());
    let typed = mine.clone();
    let merged = editor
        .exec
        .cpu(move || -> Option<(String, usize)> {
            let limits = ops.limits;
            let base = gz_luadata::LuaDoc::parse_with(&base, &name, limits).ok()?.value();
            let mine = gz_luadata::LuaDoc::parse_with(&typed, &name, limits).ok()?.value();
            let (doc, conflicts) =
                gz_luadata::LuaDoc::rebase_with(&base, &mine, &text, &(ops.diff_rules)(), &name, limits)
                    .ok()?;
            Some((doc.into_text(), conflicts.len()))
        })
        .await;
    let read_again = {
        let mut m = tab.m.lock();
        // the author typed meanwhile, or the tab moved on: the next change merges again
        if m.path != path || !m.text_dirty || m.text != mine {
            return;
        }
        m.disk = theirs;
        let file = m.shown_name();
        if let Some((text, conflicts)) = merged {
            m.text = text;
            m.text_dirty = m.text != m.disk;
            m.disk_note = match conflicts {
                0 => String::new(),
                n => format!(
                    "Changed on disk too: {n} value{}. The Source pane keeps its own; Take Theirs takes the file's.",
                    if n == 1 { "" } else { "s" }
                ),
            };
            if conflicts == 0 {
                m.preview.push_log(format!("{file} changed on disk: its changes are in the Source pane"));
            }
            true
        } else {
            m.disk_note = format!(
                "{file} changed on disk, and a text that does not read cannot be merged. Save writes the Source pane's text; Take Theirs takes the file's."
            );
            false
        }
    };
    if read_again {
        read(editor, tab, true).await;
    } else {
        wake(editor);
    }
}

/// The Source pane drops its edits for the file's text (Take Theirs).
pub async fn take_theirs(editor: &Arc<Editor>, tab: &Arc<PdaTab>) {
    {
        let mut m = tab.m.lock();
        m.text = m.disk.clone();
        m.text_dirty = false;
        m.disk_note.clear();
    }
    read(editor, tab, true).await;
}

/// `asset.take_theirs` for a layout: the tab's, when it shows that file and the file changed.
async fn take_theirs_of(editor: Arc<Editor>, rel: gz_base::RelPath) -> Result<usize, String> {
    let session = editor.project().map_err(|e| e.message)?;
    let Some(tab) = session.services.get::<PdaTab>() else {
        return Err(format!("{rel} is not open in the PDA tab"));
    };
    let (same, changed) = {
        let m = tab.m.lock();
        (!m.rel.is_empty() && gz_pda::same_path(&m.rel, rel.as_str()), !m.disk_note.is_empty())
    };
    if !same {
        return Err(format!("{rel} is not open in the PDA tab"));
    }
    if !changed {
        return Err(format!("{rel} has no change on disk the PDA tab keeps out"));
    }
    take_theirs(&editor, &tab).await;
    Ok(0)
}

/// The layout Mod > PDA Interface opens: the one the module uses, else the
/// project's first, else a new one from the template, made the module's layout. Project-relative.
pub async fn module_layout(editor: &Arc<Editor>) -> Option<String> {
    let (session, project) = Project::of(editor).ok()?;
    let service = PdaService::of_session(editor, &session);
    let used = service.used_layout(editor).await;
    if !used.is_empty() {
        return Some(used);
    }
    if let Ok(list) = service.list(editor).await
        && let Some(first) = list.into_iter().next()
    {
        return Some(first);
    }
    // nothing yet: the template, made the module's layout
    let manifest = ManifestService::of_session(editor, &session).await.ok()?;
    let _ = manifest.sync().await;
    let (id, name) =
        manifest.snapshot().manifest().map(|m| (m.id.clone(), m.name.clone())).unwrap_or_default();
    let id = if id.is_empty() { "app".to_owned() } else { id };
    let safe: String = id
        .bytes()
        .map(|c| if c.is_ascii_alphanumeric() { char::from(c.to_ascii_lowercase()) } else { '_' })
        .collect();
    let file = Path::new(project.root()).join("pda").join(format!("{safe}.pdaasset"));
    let text = gz_pda::template(&safe, if name.is_empty() { &safe } else { &name });
    let rel = project_relative(project.root(), &file.display().to_string());
    // a file of that name is used as it is, never written over
    let exists = editor.exec.io(move || file.is_file()).await;
    if !exists {
        let rp = gz_base::RelPath::new(&rel).ok()?;
        gz_asset::write_text(editor, &rp, text).await.ok()?;
    }
    let _ = service.use_layout(editor, &rel, Origin::Ui).await;
    Some(rel)
}
