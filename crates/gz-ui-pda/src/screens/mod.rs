//! Screens & Music (`EditorScreens.cpp`): the tab ([`doc`]), `screens.view` ([`cmd`]) and what
//! both keep - which surface the tab edits, what is selected, the last status - plus the one way
//! an edit is saved: the manifest is read again, the change made to what it holds and written
//! through `gz-mod`'s store (`Commit`), and the one way a file arrives: `gz_screens::import` off
//! the UI thread, then put where it was dropped (`TakeFiles`).

pub mod cmd;
pub mod doc;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gz_base::Origin;
use gz_editor::{Editor, ProjectSession};
use gz_mod::ManifestService;
use gz_project::Project;
use gz_screens::model::{LOADING, MENU, PAUSE};
use gz_screens::{FileKind, Layer, Screen, Screens, Track, file_kind};
use parking_lot::Mutex;

/// What the tab shows and `screens.view` reports.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub open: bool,
    /// 0 the loading screen, 1 the menu.
    pub surface: usize,
    pub sel_screen: String,
    pub sel_track: String,
    /// -1: no layer's settings open.
    pub sel_layer: i64,
    /// The last save's or import's failure; empty when all is well.
    pub status: String,
}

/// The screens tab's state of the open project (`session.services`).
pub struct ScreensTab {
    pub root: String,
    pub v: Mutex<View>,
    /// Edits and drops still on their way to `mod.ltx`.
    pub busy: AtomicUsize,
}

impl ScreensTab {
    #[must_use]
    pub fn of_session(session: &ProjectSession) -> Arc<Self> {
        let root = session.root.display().to_string();
        session.services.get_or_init(|| ScreensTab {
            root,
            v: Mutex::new(View { sel_layer: -1, ..View::default() }),
            busy: AtomicUsize::new(0),
        })
    }
}

/// The surfaces the tab's side means: the loading screen, or the main and pause menus.
#[must_use]
pub fn surface_mask(surface: usize) -> u32 {
    if surface == 0 { LOADING } else { MENU | PAUSE }
}

/// Where a new screen or track shows (`DefaultShow`): the tab's surface.
#[must_use]
pub fn default_show(surface: usize) -> u32 {
    surface_mask(surface)
}

/// What a change selects afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub screen: Option<String>,
    pub track: Option<String>,
    pub layer: Option<i64>,
}

impl Selection {
    fn apply(self, v: &mut View) {
        if let Some(s) = self.screen {
            v.sel_screen = s;
            v.sel_track.clear();
        }
        if let Some(t) = self.track {
            v.sel_track = t;
            v.sel_screen.clear();
        }
        if let Some(l) = self.layer {
            v.sel_layer = l;
        }
    }
}

/// A change to the screens: made to the tab's own copy at once and to what the manifest holds
/// when it is saved.
pub type Change = Arc<dyn Fn(&mut Screens) -> Selection + Send + Sync>;

/// Files from anywhere, imported, put where they were dropped (`TakeFiles`): music into the
/// tracks, anything else on the screen `screen` (its layer `layer`, or a new one), or each as a
/// new screen of its own. `err` gets the last refusal.
pub fn take_files(
    s: &mut Screens,
    rels: &[String],
    screen: &str,
    layer: i64,
    to_music: bool,
    show: u32,
    err: &mut String,
) -> Selection {
    let mut sel = Selection::default();
    for rel in rels {
        if file_kind(rel) == FileKind::Music {
            let id = s.new_track_id(&gz_screens::import::stem(rel));
            s.tracks.push(Track { id: id.clone(), file: rel.clone(), play: show, ..Track::default() });
            sel = Selection { screen: None, track: Some(id), layer: sel.layer };
            continue;
        }
        if to_music {
            "Only .ogg files go into the music.".clone_into(err);
            continue;
        }
        let found = if screen.is_empty() { None } else { s.screen_mut(screen) };
        match found {
            None => {
                let id = s.new_screen_id(&gz_screens::import::stem(rel));
                s.screens.push(Screen {
                    id: id.clone(),
                    show,
                    layers: vec![Layer { file: rel.clone(), ..Layer::default() }],
                    ..Screen::default()
                });
                sel = Selection { screen: Some(id), track: None, layer: Some(0) };
            }
            Some(d) => {
                let index = usize::try_from(layer).ok().filter(|&i| i < d.layers.len());
                match index.and_then(|i| d.layers.get_mut(i)) {
                    Some(l) => l.file.clone_from(rel),
                    None => {
                        d.layers.push(Layer { file: rel.clone(), ..Layer::default() });
                        sel.layer = Some(i64::try_from(d.layers.len()).unwrap_or(1) - 1);
                    }
                }
                sel.screen = Some(d.id.clone());
                sel.track = None;
            }
        }
    }
    sel
}

/// The `[ok]` side of a refused save: `"Not saved: ..."` (`Commit`).
fn not_saved(why: &str) -> String {
    format!("Not saved: {why}")
}

/// Saves a change (`Commit`): the manifest is read again first, so nothing another tab or an
/// agent saved is written over. The status says what went wrong, empty when nothing did.
pub async fn commit(editor: &Arc<Editor>, tab: &Arc<ScreensTab>, change: Change) {
    tab.busy.fetch_add(1, Ordering::SeqCst);
    let status = save(editor, change).await;
    {
        let mut v = tab.v.lock();
        match status {
            Ok(sel) => {
                v.status.clear();
                sel.apply(&mut v);
            }
            Err(e) => v.status = not_saved(&e),
        }
    }
    tab.busy.fetch_sub(1, Ordering::SeqCst);
    editor.exec.wake().ping();
}

async fn save(editor: &Arc<Editor>, change: Change) -> Result<Selection, String> {
    let manifest = ManifestService::of(editor).await.map_err(|e| e.message)?;
    let _ = manifest.sync().await;
    let snap = manifest.snapshot();
    let Some(doc) = snap.doc() else { return Err("mod.ltx not found in the project".to_owned()) };
    let mut s = Screens::parse(doc);
    let sel = change(&mut s);
    let edits = s.edits(doc);
    manifest
        .edit(edits, Origin::Ui)
        .await
        .map_err(|e| if e.code == "io" { "can't write mod.ltx".to_owned() } else { e.message })?;
    Ok(sel)
}

/// Files dropped on the tab or picked for it (`TakeDrop`): imported off the UI thread, in
/// parallel, then put where they landed and saved. A refused import is the status.
pub async fn take_drop(
    editor: &Arc<Editor>,
    tab: &Arc<ScreensTab>,
    files: Vec<String>,
    screen: String,
    layer: i64,
    to_music: bool,
) {
    tab.busy.fetch_add(1, Ordering::SeqCst);
    let root = tab.root.clone();
    let exec = editor.exec.clone();
    let imported: Vec<Result<String, String>> =
        editor.exec.io(move || exec.par_io(files, |f| gz_screens::import(&root, &f))).await;
    if let Ok((_, project)) = Project::of(editor) {
        project.tree.mark_fs(&std::path::Path::new(&tab.root).join("screens"));
    }
    let mut err = String::new();
    let rels: Vec<String> = imported
        .into_iter()
        .filter_map(|r| match r {
            Ok(rel) => Some(rel),
            Err(e) => {
                err = e;
                None
            }
        })
        .collect();
    let show = default_show(tab.v.lock().surface);
    if !rels.is_empty() {
        let refused = Arc::new(Mutex::new(String::new()));
        let r2 = refused.clone();
        let change: Change = Arc::new(move |s: &mut Screens| {
            let mut e = String::new();
            let sel = take_files(s, &rels, &screen, layer, to_music, show, &mut e);
            *r2.lock() = e;
            sel
        });
        commit(editor, tab, change).await;
        let e = refused.lock().clone();
        if !e.is_empty() {
            err = e;
        }
    }
    if !err.is_empty() {
        tab.v.lock().status = err;
    }
    tab.busy.fetch_sub(1, Ordering::SeqCst);
    editor.exec.wake().ping();
}

/// Where an imported file goes in the screens: the model and the file's project path.
pub type PutFile = Arc<dyn Fn(&mut Screens, &str) + Send + Sync>;

/// A file picked for one place (a layer's file or texture, a track's file): imported, then set
/// by `put`.
pub async fn take_one(editor: &Arc<Editor>, tab: &Arc<ScreensTab>, file: String, put: PutFile) {
    tab.busy.fetch_add(1, Ordering::SeqCst);
    let root = tab.root.clone();
    let imported = editor.exec.io(move || gz_screens::import(&root, &file)).await;
    if let Ok((_, project)) = Project::of(editor) {
        project.tree.mark_fs(&std::path::Path::new(&tab.root).join("screens"));
    }
    match imported {
        Ok(rel) => {
            let change: Change = Arc::new(move |s: &mut Screens| {
                put(s, &rel);
                Selection::default()
            });
            commit(editor, tab, change).await;
        }
        Err(e) => tab.v.lock().status = e,
    }
    tab.busy.fetch_sub(1, Ordering::SeqCst);
    editor.exec.wake().ping();
}
