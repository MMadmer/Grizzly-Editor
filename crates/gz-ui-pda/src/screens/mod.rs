//! Screens & Music: the tab ([`doc`]), `screens.view` ([`cmd`]) and what
//! both keep - which surface the tab edits, what is selected, the last status - plus the one way
//! an edit is saved: the manifest is read again, the change made to what it holds and written
//! through `gz-mod`'s store, and the one way a file arrives: `gz_screens::import` off
//! the UI thread, then put where it was dropped.

pub mod cmd;
pub mod doc;
pub mod sounds;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gz_base::Origin;
use gz_editor::{Editor, ProjectSession};
use gz_mod::ManifestService;
use gz_project::Project;
use gz_screens::model::{LOADING, MENU, PAUSE, RADIO};
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
    pub sel_playlist: String,
    /// -1: no layer's settings open.
    pub sel_layer: i64,
    /// The last save's or import's failure; empty when all is well.
    pub status: String,
    /// The selected track's Plays Instead list is to be put up (`screens.view show=streamer`).
    pub open_picker: bool,
    /// The Plays Instead list of this track (and of the selected one) is to be taken down
    /// (`screens.view` without `show`).
    pub close_picker: Option<String>,
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

/// Where a new screen or track shows: the tab's surface.
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
    pub playlist: Option<String>,
}

impl Selection {
    fn apply(self, v: &mut View) {
        if let Some(s) = self.screen {
            v.sel_screen = s;
            v.sel_track.clear();
            v.sel_playlist.clear();
        }
        if let Some(t) = self.track {
            v.sel_track = t;
            v.sel_screen.clear();
            v.sel_playlist.clear();
        }
        if let Some(p) = self.playlist {
            v.sel_playlist = p;
            v.sel_screen.clear();
            v.sel_track.clear();
        }
        if let Some(l) = self.layer {
            v.sel_layer = l;
        }
    }
}

/// A change to the screens: made to the tab's own copy at once and to what the manifest holds
/// when it is saved.
pub type Change = Arc<dyn Fn(&mut Screens) -> Selection + Send + Sync>;

/// Where dropped files go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DropAt {
    /// The screen they land on; empty: new screens.
    pub screen: String,
    /// On that screen, the layer whose file they replace; -1: new layers.
    pub layer: i64,
    /// Music only.
    pub music: bool,
    /// The radio: music only, each track made to play on the radio.
    pub radio: bool,
    /// A playlist: music only, each track put at its end.
    pub playlist: String,
}

/// Files from anywhere, imported, put where they were dropped: music into the tracks (onto the
/// radio or into a playlist when they landed there), anything else as [`take_files`] puts it.
/// `err` gets the last refusal.
pub fn take_into(s: &mut Screens, rels: &[String], at: &DropAt, show: u32, err: &mut String) -> Selection {
    if !at.radio && at.playlist.is_empty() {
        return take_files(s, rels, &at.screen, at.layer, at.music, show, err);
    }
    let mut sel = Selection::default();
    for rel in rels {
        if file_kind(rel) != FileKind::Music {
            "Only .ogg files go into the music.".clone_into(err);
            continue;
        }
        // the same file twice is one track
        let id = match s.tracks.iter().find(|t| t.file.eq_ignore_ascii_case(rel)) {
            Some(t) => t.id.clone(),
            None => {
                let id = s.new_track_id(&gz_screens::import::stem(rel));
                s.tracks.push(Track { id: id.clone(), file: rel.clone(), ..Track::default() });
                id
            }
        };
        if at.radio
            && let Some(t) = s.track_mut(&id)
        {
            t.play |= RADIO;
            sel = Selection { track: Some(id.clone()), ..Selection::default() };
        }
        if let Some(p) = s.playlist_mut(&at.playlist)
            && !p.tracks.contains(&id)
        {
            p.tracks.push(id);
        }
    }
    sel
}

/// Files from anywhere, imported, put where they were dropped: music into the
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
            sel = Selection { track: Some(id), layer: sel.layer, ..Selection::default() };
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
                sel = Selection { screen: Some(id), layer: Some(0), ..Selection::default() };
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

/// Files dropped on the tab or picked for it: imported off the UI thread, in
/// parallel, then put where they landed and saved. A refused import is the status.
pub async fn take_drop(
    editor: &Arc<Editor>,
    tab: &Arc<ScreensTab>,
    files: Vec<String>,
    screen: String,
    layer: i64,
    to_music: bool,
) {
    let at = DropAt { screen, layer, music: to_music, ..DropAt::default() };
    take_drop_at(editor, tab, files, at).await;
}

/// [`take_drop`] onto any place of the tab: the radio or a playlist too.
pub async fn take_drop_at(editor: &Arc<Editor>, tab: &Arc<ScreensTab>, files: Vec<String>, at: DropAt) {
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
            let sel = take_into(s, &rels, &at, show, &mut e);
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

/// A row of the Plays Instead list: what may play in a copyrighted track's place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// What `streamer` is set to: a track id or a sound name.
    pub id: String,
    /// A track of the module, a sound asset of the module, or a sound of the game.
    pub from: ChoiceFrom,
    /// What plays it in the editor: the track's file or the sound asset, project-relative; empty
    /// for a sound of the game.
    pub file: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceFrom {
    Track,
    Asset,
    Game,
}

impl ChoiceFrom {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Asset => "sound_asset",
            Self::Game => "game",
        }
    }
}

/// Everything that may play instead of the track `id` in the streamer mode: the module's other
/// tracks without the mark, then `free` (the module's sound assets and the game's sounds that
/// play in the streamer mode). Never a copyrighted sound.
#[must_use]
pub fn choices(s: &Screens, id: &str, free: &[gz_media::sound::FreeSound]) -> Vec<Choice> {
    let mut out: Vec<Choice> = s
        .tracks
        .iter()
        .filter(|t| t.id != id && !t.copyrighted)
        .map(|t| Choice { id: t.id.clone(), from: ChoiceFrom::Track, file: t.file.clone() })
        .collect();
    out.extend(free.iter().filter(|f| s.track(&f.name).is_none()).map(|f| Choice {
        id: f.name.clone(),
        from: if f.asset.is_some() { ChoiceFrom::Asset } else { ChoiceFrom::Game },
        file: f.asset.clone().unwrap_or_default(),
    }));
    out
}

/// Plays a file of the project (a track's `.ogg`, a sound asset) or a sound of the linked game
/// by its name through the editor's player, off the UI thread.
pub fn play(editor: &Arc<Editor>, file: &str, game_sound: &str) {
    let ed = editor.clone();
    let (file, name) = (file.replace('\\', "/"), game_sound.to_owned());
    let _task = editor.exec.spawn(async move {
        let mut args = gz_editor::Args::new();
        args.insert("action".into(), serde_json::json!("play"));
        if file.is_empty() {
            args.insert("name".into(), serde_json::json!(name));
        } else {
            args.insert("path".into(), serde_json::json!(file));
        }
        let reply = ed.call("preview.sound", args, Origin::Ui).await;
        if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
            let why = reply.get("error").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
            ed.notices.error(why);
        }
    });
}

/// Stops what the editor's player plays.
pub fn stop(editor: &Arc<Editor>) {
    let ed = editor.clone();
    let _task = editor.exec.spawn(async move {
        let mut args = gz_editor::Args::new();
        args.insert("action".into(), serde_json::json!("stop"));
        let _ = ed.call("preview.sound", args, Origin::Ui).await;
    });
}
