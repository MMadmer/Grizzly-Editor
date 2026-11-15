//! Screens & Music: the tab ([`doc`]), `screens.view` ([`cmd`]) and what
//! both keep - which surface the tab edits, what is selected, the last status - plus the one way
//! an edit is saved: the manifest is read again, the change made to what it holds and written
//! through `gz-mod`'s store, and the one way a file arrives: off the UI thread, music as a sound
//! asset of the module (`gz_screens::Sounds::import`, the way `asset.import` brings a sound), the
//! rest through `gz_screens::import`, then put where it was dropped.
//!
//! A track plays a sound; whether that sound is copyrighted is the sound asset's to say (the
//! Sound tab), never the track's: the tab shows it ([`TrackInfo`]) and edits none of it.

pub mod cmd;
pub mod doc;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gz_base::Origin;
use gz_editor::{Editor, ProjectSession};
use gz_media::sound::{Facts, Mark};
use gz_mod::ManifestService;
use gz_project::Project;
use gz_screens::model::{LOADING, MENU, PAUSE, RADIO};
use gz_screens::{FileKind, Layer, Screen, Screens, Track, file_kind, is_music};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

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

/// A sound that came in: the file's stem (what a new track's id is made of) and the sound's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrived {
    pub stem: String,
    pub name: String,
}

/// Sounds that came in, put where they were dropped: a track each that names its sound (one that
/// plays it already is that track), onto the radio or into a playlist when they landed there,
/// else playing on `show`.
pub fn take_sounds(s: &mut Screens, sounds: &[Arrived], at: &DropAt, show: u32) -> Selection {
    let mut sel = Selection::default();
    let elsewhere = !at.radio && at.playlist.is_empty();
    for a in sounds {
        let id = match s.tracks.iter().find(|t| t.sound.eq_ignore_ascii_case(&a.name)) {
            Some(t) => t.id.clone(),
            None => {
                let id = s.new_track_id(&a.stem);
                let play = if elsewhere { show } else { 0 };
                s.tracks.push(Track { id: id.clone(), sound: a.name.clone(), play, ..Track::default() });
                id
            }
        };
        if at.radio
            && let Some(t) = s.track_mut(&id)
        {
            t.play |= RADIO;
        }
        if let Some(p) = s.playlist_mut(&at.playlist)
            && !p.tracks.contains(&id)
        {
            p.tracks.push(id.clone());
        }
        if at.playlist.is_empty() {
            sel = Selection { track: Some(id), ..Selection::default() };
        }
    }
    sel
}

/// Files from anywhere that are no music, imported, put where they were dropped: on the screen
/// `screen` (its layer `layer`, or a new one), or each as a new screen of its own. Onto the music,
/// the radio or a playlist they are refused: `err` gets the refusal.
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
        if to_music || file_kind(rel) == FileKind::Music {
            "Only music (.ogg, .wav) and sound assets go into the music.".clone_into(err);
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

/// [`take_drop`] onto any place of the tab: the radio or a playlist too. Music becomes a sound
/// asset of the module and a track that names it; the rest is a layer or a screen.
pub async fn take_drop_at(editor: &Arc<Editor>, tab: &Arc<ScreensTab>, files: Vec<String>, at: DropAt) {
    tab.busy.fetch_add(1, Ordering::SeqCst);
    let (music, rest): (Vec<String>, Vec<String>) = files.into_iter().partition(|f| is_music(f));
    let mut err = String::new();
    // music first, one file after another: two of one stem get two names
    let sounds_in: Vec<Result<(Arrived, String, bool), String>> = if music.is_empty() {
        Vec::new()
    } else {
        let sounds = gz_screens::Sounds::now(editor).await;
        editor
            .exec
            .io(move || {
                let mut made = Vec::new();
                music
                    .iter()
                    .map(|f| {
                        sounds.import(f, &mut made).map(|g| {
                            let stem = gz_screens::import::stem(f);
                            (Arrived { stem, name: g.name }, g.asset, g.written)
                        })
                    })
                    .collect()
            })
            .await
    };
    let root = tab.root.clone();
    let exec = editor.exec.clone();
    let imported: Vec<Result<String, String>> =
        editor.exec.io(move || exec.par_io(rest, |f| gz_screens::import(&root, &f))).await;
    let mut sounds = Vec::new();
    if let Ok((_, project)) = Project::of(editor) {
        let base = std::path::Path::new(&tab.root);
        project.tree.mark_fs(&base.join("screens"));
        let mut wrote = false;
        for r in sounds_in {
            match r {
                Ok((a, asset, written)) => {
                    if written {
                        let abs = base.join(asset.replace('/', "\\"));
                        project.tree.mark_fs(abs.parent().unwrap_or(base));
                        wrote = true;
                    }
                    sounds.push(a);
                }
                Err(e) => err = e,
            }
        }
        if wrote {
            project.tree.refresh().await;
        }
    }
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
    if !rels.is_empty() || !sounds.is_empty() {
        let refused = Arc::new(Mutex::new(String::new()));
        let r2 = refused.clone();
        let change: Change = Arc::new(move |s: &mut Screens| {
            let mut e = String::new();
            let music_only = at.music || at.radio || !at.playlist.is_empty();
            let mut sel = take_files(s, &rels, &at.screen, at.layer, music_only, show, &mut e);
            let heard = take_sounds(s, &sounds, &at, show);
            if heard.track.is_some() {
                sel = heard;
            }
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

/// A file picked for one place (a layer's file or texture): imported, then set by `put`.
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

/// What the tab shows of a track's sound: what it says of itself, read where the game reads it,
/// and how long it lasts.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub facts: Facts,
    pub seconds: Option<f64>,
}

impl TrackInfo {
    #[must_use]
    pub fn copyrighted(&self) -> bool {
        self.facts.mark == Mark::Copyrighted
    }
}

/// The sounds the tracks of `s` name, by name. Off the UI thread: the module's sound assets and
/// the sounds' files are read.
pub async fn track_infos(editor: &Arc<Editor>, s: Screens) -> FxHashMap<String, TrackInfo> {
    if !s.tracks.iter().any(Track::is_sound) {
        return FxHashMap::default();
    }
    let sounds = gz_screens::Sounds::now(editor).await;
    editor
        .exec
        .io(move || {
            let mut out = FxHashMap::default();
            for t in s.tracks.iter().filter(|t| t.is_sound()) {
                out.entry(t.sound.clone()).or_insert_with(|| TrackInfo {
                    facts: sounds.facts(&t.sound),
                    seconds: sounds.seconds(&t.sound),
                });
            }
            out
        })
        .await
}

/// What a copyrighted sound does in the streamer mode, as a track's row and details say it.
#[must_use]
pub fn mark_line(info: &TrackInfo) -> gz_ui::Tx<'static> {
    if info.facts.instead.is_empty() {
        gz_ui::Tx::from("Copyrighted: it is left out in the streamer mode")
    } else {
        gz_ui::tr!(
            "Copyrighted: {instead} plays instead in the streamer mode",
            instead = info.facts.instead.clone()
        )
        .into_owned()
    }
}

/// Plays a track: its sound (the module's asset, or the game's sound by its name) or its file.
pub fn play_track(editor: &Arc<Editor>, t: &Track, info: Option<&TrackInfo>) {
    if t.is_sound() {
        match info.and_then(|i| i.facts.asset.as_deref()) {
            Some(asset) => play(editor, asset, ""),
            None => play(editor, "", &t.sound),
        }
    } else {
        play(editor, &t.file, "");
    }
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
