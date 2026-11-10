//! The PDA preview (`EditorPdaPreview.cpp`): the model one tab and `pda_preview` share, the
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
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Pos2, Vec2};
use gz_base::Origin;
use gz_editor::{DirtyDoc, DirtySource, Editor, ProjectSession, SmolStr};
use gz_mod::ManifestService;
use gz_pda::{Layout, PdaService, Preview, ReadError, base_name, project_relative};
use gz_project::Project;
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
    /// One point per game unit at the window's corner: what a preview no frame drew answers in
    /// (`EnsureGeometry`).
    #[must_use]
    pub fn unit(screen: FRect) -> Self {
        Self { origin: Pos2::ZERO, kx: 1.0, ky: 1.0, screen }
    }

    /// A game point (`VPx`).
    #[must_use]
    pub fn at(&self, vx: f32, vy: f32) -> Pos2 {
        Pos2::new(self.origin.x + vx * self.kx, self.origin.y + vy * self.ky)
    }

    /// Where the canvas' (0, 0) is.
    #[must_use]
    pub fn root(&self) -> Pos2 {
        self.at(self.screen.x1, self.screen.y1)
    }

    /// Canvas units to points, per axis (the game's `m_scale`, then the view's).
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
    pub text_dirty: bool,
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
            text_dirty: false,
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

    /// Back to the start page with every var and panel as the file declares them
    /// (`ResetState`).
    pub fn reset_state(&mut self) {
        self.preview.reset();
        self.hovered = None;
        self.pressed = None;
    }

    /// What a reading of the text gives (`Reload`): the layout when it read; the page stays when
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
}

#[derive(Default)]
struct DirtyRegistered(AtomicBool);

impl PdaTab {
    /// The preview of the session, made on first use.
    pub fn of_session(editor: &Editor, session: &ProjectSession) -> Arc<Self> {
        let flag = editor.services.get_or_init(DirtyRegistered::default);
        if !flag.0.swap(true, Ordering::SeqCst) {
            editor.add_dirty_source(Arc::new(PdaDirty));
        }
        let root = session.root.display().to_string();
        session.services.get_or_init(|| PdaTab { root, m: Mutex::new(Model::new()) })
    }
}

fn wake(editor: &Editor) {
    editor.exec.wake().ping();
}

/// Reads the Source pane's text with the game's reader (`Reload`); a newer reading started
/// meanwhile wins.
pub async fn read(editor: &Arc<Editor>, tab: &Arc<PdaTab>, keep: bool) {
    let Ok(session) = editor.project() else { return };
    let (seq, text, chunk, lang) = {
        let mut m = tab.m.lock();
        m.read_seq += 1;
        (m.read_seq, Arc::<str>::from(m.text.as_str()), base_name(&m.path), m.preview.lang.clone())
    };
    let service = PdaService::of_session(editor, &session);
    let res = service.evaluate(editor, text, &chunk, &lang).await;
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

/// Reads a file into the preview (`LoadFile`): the Source pane gets its text, clean. The page
/// stays when it is the same file. `false` when the file cannot be read.
pub async fn load_file(editor: &Arc<Editor>, tab: &Arc<PdaTab>, abs: &str) -> bool {
    let p = abs.to_owned();
    let Ok(text) = editor.exec.io(move || gz_pda::read_text(Path::new(&p))).await else { return false };
    let same = {
        let mut m = tab.m.lock();
        let same = m.path == abs;
        m.path = abs.to_owned();
        m.rel = project_relative(&tab.root, abs);
        m.text.clone_from(&text);
        m.disk = text;
        m.text_dirty = false;
        same
    };
    read(editor, tab, same).await;
    true
}

/// Writes the Source pane's text to the file (`SaveText`).
pub async fn save_text(editor: &Arc<Editor>, tab: &Arc<PdaTab>) -> Result<(), String> {
    let (path, text) = {
        let m = tab.m.lock();
        (m.path.clone(), m.text.clone())
    };
    if path.is_empty() {
        return Err("no PDA layout is open".to_owned());
    }
    let (p, t) = (path.clone(), text.clone());
    editor.exec.io(move || gz_editor::fsx::write_atomic(Path::new(&p), t.as_bytes())).await.map_err(|_| format!("can't write {path}"))?;
    {
        let mut m = tab.m.lock();
        if m.path == path {
            if m.text == text {
                m.text_dirty = false;
            }
            m.disk = text;
        }
    }
    if let Ok((_, project)) = Project::of(editor) {
        project.tree.mark_fs(Path::new(&path));
    }
    wake(editor);
    Ok(())
}

/// Saves the text and reads it again: the toolbar's Save, Ctrl+S, the shell's save.
pub async fn save_and_read(editor: &Arc<Editor>, tab: &Arc<PdaTab>) -> Result<(), String> {
    save_text(editor, tab).await?;
    read(editor, tab, true).await;
    Ok(())
}

/// The game's textures, fonts and reader are read again; the file too, unless the Source pane
/// holds edits (`ReloadAll`).
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
/// read again unless the Source pane holds edits; gone, the tab goes (`FileWritten`).
pub async fn follow(editor: &Arc<Editor>, tab: &Arc<PdaTab>) {
    let (path, disk) = {
        let m = tab.m.lock();
        if m.path.is_empty() || m.text_dirty {
            return;
        }
        (m.path.clone(), m.disk.clone())
    };
    let p = path.clone();
    let now = editor.exec.io(move || if Path::new(&p).is_file() { Some(gz_pda::read_text(Path::new(&p)).ok()) } else { None }).await;
    match now {
        None => {
            let mut m = tab.m.lock();
            if m.path == path && !m.text_dirty {
                m.open = false;
                m.path.clear();
                m.rel.clear();
            }
            drop(m);
            wake(editor);
        }
        Some(Some(text)) if text != disk => {
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

/// The layout Mod > PDA Interface opens (`ShowModuleLayout`): the one the module uses, else the
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
    let (id, name) = manifest.snapshot().manifest().map(|m| (m.id.clone(), m.name.clone())).unwrap_or_default();
    let id = if id.is_empty() { "app".to_owned() } else { id };
    let safe: String = id.bytes().map(|c| if c.is_ascii_alphanumeric() { char::from(c.to_ascii_lowercase()) } else { '_' }).collect();
    let folder = Path::new(project.root()).join("pda");
    let file = folder.join(format!("{safe}.pdaasset"));
    let text = gz_pda::template(&safe, if name.is_empty() { &safe } else { &name });
    let target = file.clone();
    // the C++ wrote over a file of that name the build leaves out; it is used as it is instead
    let made = editor
        .exec
        .io(move || {
            let _ = std::fs::create_dir_all(&folder);
            target.is_file() || gz_editor::fsx::write_atomic(&target, text.as_bytes()).is_ok()
        })
        .await;
    if !made {
        return None;
    }
    project.tree.mark_fs(&file);
    let rel = project_relative(project.root(), &file.display().to_string());
    let _ = service.use_layout(editor, &rel, Origin::Ui).await;
    Some(rel)
}
