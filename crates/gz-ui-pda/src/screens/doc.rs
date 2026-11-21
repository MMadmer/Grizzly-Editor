//! The `screens` tab (docs/UI_DESIGN.md, section 6, "Screens & Music"; section 1a holds it up
//! as the reference): the screens and music of one surface on the left, the selected one on the
//! right - a 16:9 preview, its layers, its music, the rest folded under More - saved to
//! `mod.ltx` as it is edited. Files arrive by drag and drop: on a layer, on Layers, on a list or
//! anywhere on the tab, each where it belongs. Under the music the radio: Radio Music (the tracks
//! the radios play) and Playlists (stations of the module's tracks, a card each on the right).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use egui::{
    Align, Color32, CornerRadius, DragAndDrop, Id, LayerId, Layout, Order, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, TextureHandle, TextureId, TextureOptions, Ui, UiBuilder, Vec2, pos2,
};
use gz_app::{DocCx, Document, OpenRequest, ShellBuilder};
use gz_base::{RelPath, Rev};
use gz_editor::Editor;
use gz_mod::ManifestService;
use gz_project::Project;
use gz_project::tree::EntryKind;
use gz_screens::model::{
    HIDE_BAR, HIDE_PERCENT, HIDE_STAGE, HIDE_TIP, LOADING, MENU, PAUSE, RADIO, sanitize,
};
use gz_screens::{FileKind, Fit, Layer, Playlist, Screen, Screens, Track, file_kind};
use gz_ui::file_dialog::FileRequest;
use gz_ui::grid::{Grid, chips, number, pick_field, read_only, switch, text_field};
use gz_ui::list::{NavRowSpec, nav_row};
use gz_ui::state::{busy_line, empty_state};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Combo, Font, Icon, Kind, MenuItem, PickAction, PickItem, PickList, PropFlags, PropertyGrid, TipExt, Tone,
    Toolbar, Tx, icon_button_sized, icons, menu_item, metrics, palette, segmented, tr,
};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::local::{local_drop_mark, local_list_header};
use crate::screens::{
    Change, DropAt, ScreensTab, Selection, TrackInfo, View, commit, default_show, mark_line, play_track,
    surface_mask, take_drop, take_drop_at, take_one, track_infos,
};

/// pt (docs/UI_DESIGN.md, section 6).
const LEFT_W: f32 = 260.0;
const COLUMN_W: f32 = 1040.0;
const PREVIEW_W: f32 = 520.0;
const LAYER_ROW: f32 = 28.0;
/// A picture is uploaded at most this wide: the preview never shows it larger.
const THUMB_MAX: u32 = 1280;

/// The screens as the manifest last said them.
#[derive(Clone)]
struct Local {
    rev: Rev,
    have: bool,
    s: Screens,
    /// The mode this module's games run in; empty for the plain game or every mode.
    mode: String,
}

/// A file's write stamp, as the project tree knows it.
type Stamp = (u64, Option<SystemTime>);

enum ThumbSlot {
    Loading,
    Ready(Option<(TextureHandle, Vec2)>),
}

/// Pictures of the layers, decoded on the pools and uploaded once per file version.
/// A picture decoded for a file at its stamp: the texture and its size, `None` when it did not
/// decode.
/// The problems read for the manifest and the tree at their revisions.
type ProblemsRead = ((Rev, Rev), Vec<String>);

/// What the tracks' sounds are, by name, read for the manifest and the tree at their revisions.
type Infos = Arc<FxHashMap<String, TrackInfo>>;
type InfosRead = ((Rev, Rev), Infos);

/// What a dragged row of the Music list carries.
const TRACK_ROW: &str = "screens.track";

type ThumbArrival = (String, Option<Stamp>, Option<(TextureHandle, Vec2)>);

#[derive(Default)]
struct Thumbs {
    map: FxHashMap<String, (Option<Stamp>, ThumbSlot)>,
    inbox: Arc<Mutex<Vec<ThumbArrival>>>,
}

impl Thumbs {
    fn get(
        &mut self,
        editor: &Arc<Editor>,
        ctx: &egui::Context,
        root: &str,
        rel: &str,
        stamp: Option<Stamp>,
    ) -> Option<(TextureId, Vec2)> {
        for (k, st, made) in self.inbox.lock().drain(..) {
            self.map.insert(k, (st, ThumbSlot::Ready(made)));
        }
        if let Some((st, slot)) = self.map.get(rel)
            && *st == stamp
        {
            return match slot {
                ThumbSlot::Ready(Some((h, size))) => Some((h.id(), *size)),
                _ => None,
            };
        }
        self.map.insert(rel.to_owned(), (stamp, ThumbSlot::Loading));
        let (inbox, ctx, ed) = (self.inbox.clone(), ctx.clone(), editor.clone());
        let (path, key, ext) =
            (Path::new(root).join(rel), rel.to_owned(), gz_screens::import::extension(rel));
        let _task = editor.exec.spawn(async move {
            let bytes = ed.exec.io(move || std::fs::read(path).ok()).await;
            let name = format!("screens:{key}");
            let made = match bytes {
                Some(bytes) => {
                    ed.exec
                        .cpu(move || {
                            let mut img = gz_fmt_tex::load_image(&bytes, Some(&ext)).ok()?.image;
                            if img.width > THUMB_MAX {
                                let h = (u64::from(img.height) * u64::from(THUMB_MAX) / u64::from(img.width))
                                    .max(1);
                                img = gz_fmt_tex::resize(
                                    &img,
                                    THUMB_MAX,
                                    u32::try_from(h).ok()?,
                                    gz_fmt_tex::MipFilter::Default,
                                )
                                .ok()?;
                            }
                            let size = [usize::try_from(img.width).ok()?, usize::try_from(img.height).ok()?];
                            let handle = ctx.load_texture(
                                name,
                                egui::ColorImage::from_rgba_unmultiplied(size, &img.rgba),
                                TextureOptions::LINEAR,
                            );
                            Some((handle, Vec2::new(img.width as f32, img.height as f32)))
                        })
                        .await
                }
                None => None,
            };
            inbox.lock().push((key, stamp, made));
            ed.exec.wake().ping();
        });
        None
    }
}

/// A layer being dragged to another place in its list.
#[derive(Clone)]
struct LayerDrag {
    screen: String,
    index: usize,
}

/// What a frame asked for, done once it is drawn.
#[derive(Default)]
struct Out {
    changes: Vec<Change>,
    /// The files and where they landed.
    drops: Vec<(Vec<String>, DropAt)>,
    /// A file to pick: the filter's name and extensions, and where the file goes.
    picks: Vec<(&'static str, &'static [&'static str], Pick)>,
    open_mod_settings: bool,
    /// Open Sound: the sound asset to open in the Sound tab.
    open_sound: Option<String>,
    /// Make It a Sound: the track whose file becomes a sound asset.
    make_sound: Option<String>,
}

#[derive(Clone)]
enum Pick {
    LayerFile(String, usize),
    LayerTexture(String, usize),
    /// A new track.
    Music,
    /// A new track the radios play.
    RadioMusic,
    /// A new track at the end of a playlist.
    Playlist(String),
}

/// What the tab keeps that is not the module's data.
#[derive(Default)]
struct Edit {
    /// A number being dragged: its key and value, saved when it is let go of.
    drag: Option<(String, f32)>,
    /// A screen's name being typed.
    rename: Option<(String, String)>,
    /// A level being typed (no list of the game's levels).
    level: String,
    /// The title of a playlist being made, typed in place.
    new_playlist: Option<String>,
    /// A playlist's title being typed: its id and the text.
    title: Option<(String, String)>,
}

pub struct ScreensDoc {
    editor: Arc<Editor>,
    tab: Arc<ScreensTab>,
    local: Option<Local>,
    incoming: Arc<Mutex<Option<Local>>>,
    asked: Option<Rev>,
    manifest_asked: bool,
    problems: Vec<String>,
    problems_key: Option<(Rev, Rev)>,
    problems_in: Arc<Mutex<Option<ProblemsRead>>>,
    checking: Arc<AtomicBool>,
    checked_at: Option<Instant>,
    levels: Arc<Mutex<Option<Option<Vec<String>>>>>,
    levels_asked: bool,
    thumbs: Thumbs,
    edit: Edit,
    picking: Arc<AtomicBool>,
    /// What the tracks' sounds are, read off the UI thread once per state of the manifest and
    /// the files.
    infos: Option<InfosRead>,
    infos_in: Arc<Mutex<Option<InfosRead>>>,
    infos_asked: Option<(Rev, Rev)>,
}

fn mask_check(ui: &mut Ui, label: &'static str, mask: &mut u32, bit: u32, invert: bool) -> bool {
    let mut on = if invert { *mask & bit == 0 } else { *mask & bit != 0 };
    let flipped = ui
        .push_id(("gz-ui-pda.check", label), |ui| gz_ui::Switch::new(&mut on, label).show(ui).changed())
        .inner;
    if !flipped {
        return false;
    }
    if on != invert {
        *mask |= bit;
    } else {
        *mask &= !bit;
    }
    true
}

fn kind_of(file: &str) -> FileKind {
    if file.is_empty() { FileKind::None } else { file_kind(file) }
}

fn kind_icon(k: FileKind) -> Icon {
    match k {
        FileKind::Picture | FileKind::Gif => Icon::Picture,
        FileKind::Video => Icon::Play,
        FileKind::Shader => Icon::FileScript,
        FileKind::Page => Icon::FileXml,
        _ => Icon::File,
    }
}

fn leaf(file: &str) -> &str {
    file.rsplit(['\\', '/']).next().unwrap_or(file)
}

/// The problems of one screen or track, without their code and tag: "screen <id>:" after the
/// code.
fn problems_of<'a>(problems: &'a [String], what: &str, id: &str) -> Vec<(bool, &'a str)> {
    let tag = format!("{what} {id}:");
    problems
        .iter()
        .filter_map(|p| {
            let rest = p.get(5..)?;
            rest.strip_prefix(&tag).map(|msg| (p.starts_with('E'), msg.trim_start()))
        })
        .collect()
}

fn row_dot(problems: &[String], what: &str, id: &str) -> Option<Tone> {
    let list = problems_of(problems, what, id);
    if list.is_empty() {
        None
    } else if list.iter().any(|(e, _)| *e) {
        Some(Tone::Error)
    } else {
        Some(Tone::Warning)
    }
}

/// What a screen's music is called.
fn music_label(d: &Screen) -> Tx<'static> {
    match d.music.as_str() {
        "none" => Tx::from("Silence"),
        "video" => Tx::from("The Video's Own Sound"),
        "tracks" => Tx::raw(d.tracks.join(", ")),
        _ => Tx::from("Any Track"),
    }
}

fn change(f: impl Fn(&mut Screens) -> Selection + Send + Sync + 'static) -> Change {
    Arc::new(f)
}

fn set_screen(id: &str, f: impl Fn(&mut Screen) + Send + Sync + 'static) -> Change {
    let id = id.to_owned();
    change(move |s| {
        if let Some(d) = s.screen_mut(&id) {
            f(d);
        }
        Selection::default()
    })
}

fn set_layer(id: &str, index: usize, f: impl Fn(&mut Layer) + Send + Sync + 'static) -> Change {
    set_screen(id, move |d| {
        if let Some(l) = d.layers.get_mut(index) {
            f(l);
        }
    })
}

fn set_track(id: &str, f: impl Fn(&mut Track) + Send + Sync + 'static) -> Change {
    let id = id.to_owned();
    change(move |s| {
        if let Some(t) = s.track_mut(&id) {
            f(t);
        }
        Selection::default()
    })
}

/// A number edited by dragging or typing: shown from the drag while it lasts, saved when it is
/// let go of. Returns the value to save.
// every argument is one the caller decides at each call; a struct of them would only rename them
#[allow(clippy::too_many_arguments)]
fn edited_number(
    ui: &mut Ui,
    edit: &mut Edit,
    key: &str,
    value: f32,
    speed: f32,
    unit: &'static str,
    lo: f32,
    hi: f32,
) -> Option<f32> {
    let mut v = match &edit.drag {
        Some((k, v)) if k == key => *v,
        _ => value,
    };
    let resp = number(ui, &mut v, speed, unit);
    v = v.clamp(lo, hi);
    if resp.changed() {
        edit.drag = Some((key.to_owned(), v));
    }
    let done = resp.drag_stopped() || (resp.changed() && !resp.dragged());
    if done {
        let out = match &edit.drag {
            Some((k, v)) if k == key => Some(*v),
            _ => Some(v),
        };
        edit.drag = None;
        return out;
    }
    None
}

impl ScreensDoc {
    fn new(editor: Arc<Editor>, tab: Arc<ScreensTab>) -> Self {
        Self {
            editor,
            tab,
            local: None,
            incoming: Arc::new(Mutex::new(None)),
            asked: None,
            manifest_asked: false,
            problems: Vec::new(),
            problems_key: None,
            problems_in: Arc::new(Mutex::new(None)),
            checking: Arc::new(AtomicBool::new(false)),
            checked_at: None,
            levels: Arc::new(Mutex::new(None)),
            levels_asked: false,
            thumbs: Thumbs::default(),
            edit: Edit::default(),
            picking: Arc::new(AtomicBool::new(false)),
            infos: None,
            infos_in: Arc::new(Mutex::new(None)),
            infos_asked: None,
        }
    }

    /// What the tracks' sounds are: asked for off the UI thread once per state of the manifest and
    /// the files; the last answer while a newer one is read.
    fn infos(&mut self, s: &Screens, rev: Rev) -> Option<Infos> {
        if let Some(read) = self.infos_in.lock().take() {
            self.infos = Some(read);
        }
        let tree = Project::of(&self.editor).map_or(Rev::ZERO, |(_, p)| p.tree.rev());
        let key = (rev, tree);
        if self.infos.as_ref().is_none_or(|(k, _)| *k != key) && self.infos_asked != Some(key) {
            self.infos_asked = Some(key);
            let (out, ed, model) = (self.infos_in.clone(), self.editor.clone(), s.clone());
            let _task = self.editor.exec.spawn(async move {
                let infos = track_infos(&ed, model).await;
                *out.lock() = Some((key, Arc::new(infos)));
                ed.exec.wake().ping();
            });
        }
        self.infos.as_ref().map(|(_, i)| i.clone())
    }

    /// The model follows the manifest: read again off the UI thread when its
    /// revision moved, checked again at most once a second when the manifest or the files moved.
    fn refresh(&mut self) {
        let Some(session) = self.editor.project_opt() else { return };
        let Some(manifest) = ManifestService::get(&session) else {
            if !self.manifest_asked {
                self.manifest_asked = true;
                let (ed, s) = (self.editor.clone(), session.clone());
                let _task = self.editor.exec.spawn(async move {
                    let _ = ManifestService::of_session(&ed, &s).await;
                    ed.exec.wake().ping();
                });
            }
            return;
        };
        if let Some(l) = self.incoming.lock().take() {
            self.local = Some(l);
        }
        let rev = manifest.rev();
        if self.local.as_ref().is_none_or(|l| l.rev != rev) && self.asked != Some(rev) {
            self.asked = Some(rev);
            let (inbox, ed, m) = (self.incoming.clone(), self.editor.clone(), manifest.clone());
            let _task = self.editor.exec.spawn(async move {
                let snap = m.snapshot();
                let local = ed
                    .exec
                    .cpu(move || {
                        let s = snap.doc().map(Screens::parse).unwrap_or_default();
                        // the modes the module is for, its own first; one for every game or for the plain
                        // game too is no mode's alone, and its screens show everywhere
                        let mode = snap
                            .manifest()
                            .map(|mf| {
                                let named = gz_mod::mode_list(&mf.mode);
                                if named.iter().any(|m| m == "*" || m == gz_mod::PLAIN_MODE) {
                                    return String::new();
                                }
                                let mut ids: Vec<String> = mf
                                    .provides_modes
                                    .iter()
                                    .map(|p| p.id.clone())
                                    .filter(|i| !i.is_empty())
                                    .collect();
                                for m in named {
                                    if !ids.contains(&m) {
                                        ids.push(m);
                                    }
                                }
                                ids.join(", ")
                            })
                            .unwrap_or_default();
                        Local { rev, have: snap.exists(), s, mode }
                    })
                    .await;
                *inbox.lock() = Some(local);
                ed.exec.wake().ping();
            });
        }
        if let Some((key, list)) = self.problems_in.lock().take() {
            self.problems_key = Some(key);
            self.problems = list;
        }
        let tree_rev = Project::of(&self.editor).map_or(Rev::ZERO, |(_, p)| p.tree.rev());
        let key = (rev, tree_rev);
        let due = self.checked_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        if let Some(l) = &self.local
            && l.rev == rev
            && l.have
            && self.problems_key != Some(key)
            && due
            && !self.checking.swap(true, Ordering::SeqCst)
        {
            self.checked_at = Some(Instant::now());
            let (out, flag, ed, m, s, root) = (
                self.problems_in.clone(),
                self.checking.clone(),
                self.editor.clone(),
                manifest.clone(),
                l.s.clone(),
                session.root.clone(),
            );
            let _task = self.editor.exec.spawn(async move {
                let list = gz_screens::problems(&ed, root, &m, s).await;
                *out.lock() = Some((key, list));
                flag.store(false, Ordering::SeqCst);
                ed.exec.wake().ping();
            });
        }
        if !self.levels_asked {
            self.levels_asked = true;
            let (out, ed) = (self.levels.clone(), self.editor.clone());
            let _task = self.editor.exec.spawn(async move {
                let levels = gz_screens::GameLevels::of(&ed).levels(&ed).await;
                *out.lock() = Some(levels);
            });
        }
    }

    /// An edit: made to the tab's copy at once, then saved.
    fn apply(&mut self, c: Change) {
        if let Some(l) = &mut self.local {
            let sel = c(&mut l.s);
            sel.apply(&mut self.tab.v.lock());
        }
        let (ed, tab) = (self.editor.clone(), self.tab.clone());
        self.spawn_busy(async move { commit(&ed, &tab, c).await });
    }

    /// Work on its way to `mod.ltx`, counted from before it starts: `screens.view` waits for it.
    fn spawn_busy(&self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        self.tab.busy.fetch_add(1, Ordering::SeqCst);
        let tab = self.tab.clone();
        let _task = self.editor.exec.spawn(async move {
            fut.await;
            tab.busy.fetch_sub(1, Ordering::SeqCst);
        });
    }

    fn finish(&mut self, out: Out, cx: &mut DocCx<'_>) {
        for c in out.changes {
            self.apply(c);
        }
        for (files, at) in out.drops {
            let (ed, tab) = (self.editor.clone(), self.tab.clone());
            self.spawn_busy(async move { take_drop_at(&ed, &tab, files, at).await });
        }
        for (filter, exts, pick) in out.picks {
            self.pick_file(filter, exts, pick);
        }
        if out.open_mod_settings {
            cx.requests.open(OpenRequest::kind("mod_settings"));
        }
        if let Some(asset) = out.open_sound {
            cx.requests.open(OpenRequest::file("sound", &asset.replace('/', "\\")));
        }
        if let Some(id) = out.make_sound {
            // the command's one path: the window and an agent make a sound the same way
            let (ed, tab) = (self.editor.clone(), self.tab.clone());
            self.spawn_busy(async move {
                let mut args = gz_editor::Args::new();
                args.insert("action".into(), serde_json::json!("track_sound"));
                args.insert("id".into(), serde_json::json!(id));
                let reply = ed.call("screens.set", args, gz_base::Origin::Ui).await;
                let mut v = tab.v.lock();
                match reply.get("error").and_then(serde_json::Value::as_str) {
                    Some(e) if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(false) => {
                        v.status = format!("Not saved: {e}");
                    }
                    _ => v.status.clear(),
                }
                drop(v);
                ed.exec.wake().ping();
            });
        }
    }

    /// The system's file dialog, off the UI thread; the file picked is imported where it goes.
    fn pick_file(&self, filter: &'static str, exts: &'static [&'static str], pick: Pick) {
        if self.picking.swap(true, Ordering::SeqCst) {
            return;
        }
        let (ed, tab) = (self.editor.clone(), self.tab.clone());
        let release = ReleasePick(self.picking.clone());
        let request =
            FileRequest::open("Choose a File").filter(filter, exts).in_folder(self.tab.root.clone());
        let asker = if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
        let picked = gz_ui::file_dialog::pick(&self.editor.exec, None, asker, request, move |file| {
            drop(release);
            let file = file.display().to_string();
            let _task = ed.exec.clone().spawn(async move {
                match pick {
                    Pick::Music => take_drop(&ed, &tab, vec![file], String::new(), -1, true).await,
                    Pick::RadioMusic => {
                        let at = DropAt { radio: true, ..DropAt::default() };
                        take_drop_at(&ed, &tab, vec![file], at).await;
                    }
                    Pick::Playlist(id) => {
                        let at = DropAt { playlist: id, ..DropAt::default() };
                        take_drop_at(&ed, &tab, vec![file], at).await;
                    }
                    Pick::LayerFile(id, i) => {
                        take_one(
                            &ed,
                            &tab,
                            file,
                            Arc::new(move |s: &mut Screens, rel: &str| set_file(s, &id, i, rel, false)),
                        )
                        .await
                    }
                    Pick::LayerTexture(id, i) => {
                        take_one(
                            &ed,
                            &tab,
                            file,
                            Arc::new(move |s: &mut Screens, rel: &str| set_file(s, &id, i, rel, true)),
                        )
                        .await
                    }
                }
            });
        });
        if let Err(line) = picked {
            self.editor.log.finding(&line);
        }
    }
}

fn set_file(s: &mut Screens, id: &str, index: usize, rel: &str, texture: bool) {
    if let Some(l) = s.screen_mut(id).and_then(|d| d.layers.get_mut(index)) {
        if texture {
            rel.clone_into(&mut l.texture);
        } else {
            rel.clone_into(&mut l.file);
        }
    }
}

/// Everything a frame of the tab draws from and writes into.
struct Frame<'a> {
    editor: &'a Arc<Editor>,
    root: &'a str,
    s: &'a Screens,
    mode: &'a str,
    problems: &'a [String],
    levels: Option<&'a [String]>,
    view: &'a mut View,
    out: &'a mut Out,
    edit: &'a mut Edit,
    thumbs: &'a mut Thumbs,
    tree: Option<Arc<gz_project::tree::FileTree>>,
    /// What the tracks' sounds are; `None` until they are first read.
    infos: Option<Infos>,
}

impl Frame<'_> {
    fn stamp(&self, rel: &str) -> Option<Stamp> {
        let tree = self.tree.as_ref()?;
        let path = RelPath::new(rel).ok()?;
        match tree.entry(&path)?.kind {
            EntryKind::File { size, modified, .. } => Some((size, modified)),
            _ => None,
        }
    }

    fn screens_list(&mut self, ui: &mut Ui) {
        let from = ui.cursor().min;
        let mask = surface_mask(self.view.surface);
        if local_list_header(ui, "Screens", "Add a screen") {
            let show = default_show(self.view.surface);
            self.out.changes.push(change(move |s| {
                let id = s.new_screen_id("screen");
                s.screens.push(Screen { id: id.clone(), show, ..Screen::default() });
                Selection { screen: Some(id), layer: Some(-1), ..Selection::default() }
            }));
        }
        let mut any = false;
        for d in &self.s.screens {
            if d.show & mask == 0 {
                continue;
            }
            any = true;
            let count = d.layers.len().to_string();
            let spec = NavRowSpec {
                icon: Icon::Picture,
                count: Some(Tx::raw(count.as_str())),
                dot: row_dot(self.problems, "screen", &d.id),
                action: Icon::Delete,
                action_tip: "Remove this screen. Its files stay in the project.".into(),
                ..NavRowSpec::default()
            };
            let r = nav_row(
                ui,
                &format!("screen.{}", d.id),
                Tx::raw(d.id.as_str()),
                self.view.sel_screen == d.id,
                &spec,
            );
            if r.clicked {
                self.view.sel_screen.clone_from(&d.id);
                self.view.sel_track.clear();
                self.view.sel_playlist.clear();
                self.view.sel_layer = -1;
            }
            if r.action {
                let id = d.id.clone();
                if self.view.sel_screen == id {
                    self.view.sel_screen.clear();
                }
                self.out.changes.push(change(move |s| {
                    s.screens.retain(|x| x.id != id);
                    Selection::default()
                }));
            }
        }
        if !any {
            text::status(
                ui,
                Tone::Muted,
                "Drop pictures, videos or shaders here. Without a screen the game shows its own.",
            );
        }
        let to = pos2(from.x + ui.available_width(), ui.cursor().min.y + 8.0);
        if let Some(files) = take_files(ui, "drop_screens", Rect::from_min_max(from, to), false) {
            self.out.drops.push((files, DropAt { layer: -1, ..DropAt::default() }));
        }
    }

    fn music_list(&mut self, ui: &mut Ui) {
        let from = ui.cursor().min;
        let mask = surface_mask(self.view.surface);
        if local_list_header(ui, "Music", "Add music (.ogg, .wav)") {
            self.out.picks.push(("Music", MUSIC_FILES, Pick::Music));
        }
        let s = self.s;
        for t in &s.tracks {
            let mark = self.mark_of(t);
            let tip: Tx<'_> = match &mark {
                Some(line) => tr!("{mark}. Drag it onto a playlist to put it there.", mark = line.clone()),
                None => "Drag it onto a playlist to put it there.".into(),
            };
            let spec = NavRowSpec {
                icon: if t.is_sound() { Icon::SoundAsset } else { Icon::FileSound },
                dim: t.play & mask == 0,
                dot: row_dot(self.problems, "track", &t.id),
                count: mark.as_ref().map(|_| Tx::raw(COPYRIGHT)),
                action: Icon::Delete,
                action_tip: "Remove this track. Its sound stays in the project.".into(),
                tip,
            };
            let top = ui.cursor().min.y;
            let r = nav_row(
                ui,
                &format!("track.{}", t.id),
                Tx::raw(t.id.as_str()),
                self.view.sel_track == t.id,
                &spec,
            );
            drag_track(ui, top, "music", &t.id);
            if r.clicked {
                self.select_track(&t.id);
            }
            if r.action {
                let id = t.id.clone();
                if self.view.sel_track == id {
                    self.view.sel_track.clear();
                }
                self.out.changes.push(change(move |s| {
                    s.remove_track(&id);
                    Selection::default()
                }));
            }
        }
        if self.s.tracks.is_empty() {
            text::status(ui, Tone::Muted, "Drop music here (.ogg, .wav) or sound assets.");
        }
        let to = pos2(from.x + ui.available_width(), ui.cursor().min.y + 8.0);
        if let Some(files) = take_files(ui, "drop_music", Rect::from_min_max(from, to), false) {
            self.out.drops.push((files, DropAt { layer: -1, music: true, ..DropAt::default() }));
        }
        if !self.s.tracks.is_empty() {
            ui.add_space(4.0);
            let side = self.view.surface.min(1);
            let mut cur = usize::from(self.s.next_track.get(side).copied().unwrap_or(false));
            let resp = ui.scope(|ui| segmented(ui, &mut cur, &["Repeat", "Next Track"], None));
            if resp.inner {
                let next = cur == 1;
                self.out.changes.push(change(move |s| {
                    if let Some(slot) = s.next_track.get_mut(side) {
                        *slot = next;
                    }
                    Selection::default()
                }));
            }
            resp.response.tip("What happens when a track ends.");
        }
    }

    fn select_track(&mut self, id: &str) {
        id.clone_into(&mut self.view.sel_track);
        self.view.sel_screen.clear();
        self.view.sel_playlist.clear();
        self.view.sel_layer = -1;
    }

    /// Radio Music: the tracks the radios of the world play among the game's own, a drop zone
    /// for more, and Only Mine under More.
    fn radio_list(&mut self, ui: &mut Ui) {
        let from = ui.cursor().min;
        if local_list_header(ui, "Radio Music", "Add music the radios play (.ogg, .wav)") {
            self.out.picks.push(("Music", MUSIC_FILES, Pick::RadioMusic));
        }
        let mut any = false;
        let s = self.s;
        for t in s.tracks.iter().filter(|t| t.play & RADIO != 0) {
            any = true;
            let mark = self.mark_of(t);
            let spec = NavRowSpec {
                icon: Icon::SoundSource,
                dot: row_dot(self.problems, "track", &t.id),
                count: mark.as_ref().map(|_| Tx::raw(COPYRIGHT)),
                tip: mark.unwrap_or(Tx::EMPTY),
                action: Icon::Close,
                action_tip: "Take it off the radio. The track stays in the music.".into(),
                ..NavRowSpec::default()
            };
            let top = ui.cursor().min.y;
            let r = nav_row(
                ui,
                &format!("radio.{}", t.id),
                Tx::raw(t.id.as_str()),
                self.view.sel_track == t.id,
                &spec,
            );
            drag_track(ui, top, "radio", &t.id);
            if r.clicked {
                self.select_track(&t.id);
            }
            if r.action {
                self.out.changes.push(set_track(&t.id, |t| t.play &= !RADIO));
            }
        }
        if !any {
            text::status(
                ui,
                Tone::Muted,
                "Drop music here (.ogg, .wav) or sound assets: the radios play it among the game's music.",
            );
        }
        let to = pos2(from.x + ui.available_width(), ui.cursor().min.y + 8.0);
        if let Some(files) = take_files(ui, "drop_radio", Rect::from_min_max(from, to), false) {
            self.out.drops.push((files, DropAt { layer: -1, radio: true, ..DropAt::default() }));
        }
        let key = "gz-ui-pda.radio_more";
        let (_, _) = gz_ui::Section::new(key, "More").default_open(false).body(ui, |ui| {
            let mut only = self.s.only_radio;
            let resp = gz_ui::Switch::new(&mut only, "Only Mine")
                .tip("The radios play this module's radio music alone, not the game's and other mods'. The player may still pick the game's music or a playlist.")
                .show(ui);
            if resp.changed() {
                self.out.changes.push(change(move |s| {
                    s.only_radio = only;
                    Selection::default()
                }));
            }
        });
    }

    /// Playlists: stations of the module's tracks. + makes one, its title typed in place; a track
    /// dragged onto a row joins that playlist.
    fn playlists_list(&mut self, ui: &mut Ui) {
        if local_list_header(ui, "Playlists", "Make a playlist") && self.edit.new_playlist.is_none() {
            self.edit.new_playlist = Some(String::new());
        }
        if let Some(mut title) = self.edit.new_playlist.take() {
            let resp = text_field(ui, &mut title);
            if !resp.has_focus() && !resp.lost_focus() && title.is_empty() {
                resp.request_focus();
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if resp.lost_focus() && enter && !title.trim().is_empty() {
                let t = title.trim().to_owned();
                self.out.changes.push(change(move |s| {
                    let id = s.new_playlist_id(&t);
                    s.playlists.push(Playlist { id: id.clone(), title: t.clone(), ..Playlist::default() });
                    Selection { playlist: Some(id), ..Selection::default() }
                }));
            } else if !(resp.lost_focus() && (escape || title.trim().is_empty())) {
                self.edit.new_playlist = Some(title);
            }
            resp.tip("The playlist's title, as the player reads it among the stations. Enter makes it.");
        }
        let s = self.s;
        for p in &s.playlists {
            let count = p.tracks.len().to_string();
            let spec = NavRowSpec {
                icon: Icon::Menu,
                count: Some(Tx::raw(count.as_str())),
                dot: row_dot(self.problems, "playlist", &p.id),
                action: Icon::Delete,
                action_tip: "Remove this playlist. Its tracks stay in the music.".into(),
                ..NavRowSpec::default()
            };
            let top = ui.cursor().min.y;
            let label = if p.title.is_empty() { p.id.as_str() } else { p.title.as_str() };
            let r = nav_row(
                ui,
                &format!("playlist.{}", p.id),
                Tx::raw(label),
                self.view.sel_playlist == p.id,
                &spec,
            );
            if r.clicked {
                self.view.sel_playlist.clone_from(&p.id);
                self.view.sel_screen.clear();
                self.view.sel_track.clear();
                self.view.sel_layer = -1;
            }
            if r.action {
                let id = p.id.clone();
                if self.view.sel_playlist == id {
                    self.view.sel_playlist.clear();
                }
                self.out.changes.push(change(move |s| {
                    s.playlists.retain(|x| x.id != id);
                    Selection::default()
                }));
            }
            let row = Rect::from_min_max(
                pos2(ui.cursor().min.x, top),
                pos2(ui.cursor().min.x + ui.available_width(), ui.cursor().min.y),
            );
            self.take_into_playlist(ui, &format!("drop_playlist.{}", p.id), row, &p.id);
        }
        if self.s.playlists.is_empty() && self.edit.new_playlist.is_none() {
            text::status(ui, Tone::Muted, "A playlist is a station of its own: + makes one.");
        }
    }

    /// A zone over `rect` that puts a track dragged from the Music list, or dropped files, into
    /// the playlist `id`.
    fn take_into_playlist(&mut self, ui: &Ui, name: &str, rect: Rect, id: &str) {
        let zone = gz_ui::drop::Zone::new(name, rect, &accepts_tracks);
        match zone.take(ui) {
            Some(gz_ui::drop::Payload::Row { key, .. }) => {
                let id = id.to_owned();
                self.out.changes.push(change(move |s| {
                    if s.track(&key).is_some()
                        && let Some(p) = s.playlist_mut(&id)
                        && !p.tracks.contains(&key)
                    {
                        p.tracks.push(key.clone());
                    }
                    Selection::default()
                }));
            }
            Some(other) => {
                let files: Vec<String> = other.files().iter().map(|f| f.display().to_string()).collect();
                self.out
                    .drops
                    .push((files, DropAt { layer: -1, playlist: id.to_owned(), ..DropAt::default() }));
            }
            None => {}
        }
    }

    /// A playlist's card: its title, Shuffle, its tracks (a play button each, dragged to reorder,
    /// a cross to leave), a drop zone for tracks and files.
    fn playlist_details(&mut self, ui: &mut Ui, pl: &Playlist) {
        let from = ui.cursor().min;
        let id = pl.id.clone();
        let heading = if pl.title.is_empty() { pl.id.clone() } else { pl.title.clone() };
        let problems = problems_of(self.problems, "playlist", &pl.id);
        let attention = !problems.is_empty();
        gz_ui::card::card(ui, "playlist", Tx::raw(heading.as_str()), Tone::Warning, attention, |ui| {
            for (error, msg) in &problems {
                text::status(ui, if *error { Tone::Error } else { Tone::Warning }, Tx::ui(*msg));
            }
            PropertyGrid::new("playlist").label_frac(0.28).show(ui, |g: &mut Grid<'_>| {
                g.row(
                    "Title",
                    "What the player reads among the radio stations: text or a string table id.",
                    PropFlags::NONE,
                    0,
                    |ui| {
                        let mut t = match &self.edit.title {
                            Some((tid, t)) if *tid == id => t.clone(),
                            _ => pl.title.clone(),
                        };
                        let resp = text_field(ui, &mut t);
                        if resp.changed() {
                            self.edit.title = Some((id.clone(), t.clone()));
                        }
                        if resp.lost_focus() {
                            self.edit.title = None;
                            let t = t.trim().to_owned();
                            if t != pl.title {
                                let pid = id.clone();
                                self.out.changes.push(change(move |s| {
                                    if let Some(p) = s.playlist_mut(&pid) {
                                        p.title.clone_from(&t);
                                    }
                                    Selection::default()
                                }));
                            }
                        }
                    },
                );
                g.row(
                    "Shuffle",
                    "Every track once a pass, in a new order each pass. Off: in this order, round and round.",
                    PropFlags::INLINE,
                    0,
                    |ui| {
                        let mut on = pl.order == gz_screens::Order::Shuffle;
                        if switch(ui, &mut on).changed() {
                            let pid = id.clone();
                            self.out.changes.push(change(move |s| {
                                if let Some(p) = s.playlist_mut(&pid) {
                                    p.order =
                                        if on { gz_screens::Order::Shuffle } else { gz_screens::Order::List };
                                }
                                Selection::default()
                            }));
                        }
                    },
                );
            });
            ui.add_space(6.0);
            const COLS: [gz_ui::Col<'static>; 1] = [gz_ui::Col::new("Track", 200.0)];
            let tracks = &pl.tracks;
            let mut play: Option<usize> = None;
            let edit = gz_ui::ItemRows::new(("gz-ui-pda.playlist_rows", &id), &COLS, tracks.len())
                .add_label("Add Tracks")
                .empty_text("Drop tracks here from the Music list, or music files.")
                .show(ui, |ui, row, _| {
                    let Some(t) = tracks.get(row) else { return };
                    let track = self.s.track(t);
                    let audible = track.is_some_and(|x| x.is_sound() || !x.file.is_empty());
                    if icon_button_sized(ui, Icon::Play, "Play it", false, audible, 22.0, metrics::GLYPH_ROW)
                        .clicked()
                    {
                        play = Some(row);
                    }
                    let p = palette(ui.ctx());
                    text::label(
                        ui,
                        Tx::raw(t.as_str()),
                        Font::Body,
                        if track.is_some() { p.text } else { p.tone(Tone::Error) },
                        Clip::Start,
                    );
                    if let Some(line) = track.and_then(|x| self.mark_of(x)) {
                        text::label(ui, Tx::raw(COPYRIGHT), Font::Small, p.text_muted, Clip::End).tip(line);
                    }
                });
            if let Some(row) = play
                && let Some(t) = tracks.get(row).and_then(|t| self.s.track(t))
            {
                play_track(self.editor, t, self.info_of(t));
            }
            if let Some(row) = edit.removed {
                let pid = id.clone();
                self.out.changes.push(change(move |s| {
                    if let Some(p) = s.playlist_mut(&pid)
                        && row < p.tracks.len()
                    {
                        p.tracks.remove(row);
                    }
                    Selection::default()
                }));
            }
            if let Some((a, b)) = edit.moved {
                let pid = id.clone();
                self.out.changes.push(change(move |s| {
                    if let Some(p) = s.playlist_mut(&pid) {
                        gz_ui::rows::apply_move(&mut p.tracks, a, b);
                    }
                    Selection::default()
                }));
            }
            if edit.added {
                self.out.picks.push(("Music", MUSIC_FILES, Pick::Playlist(id.clone())));
            }
        });
        let to = pos2(from.x + ui.available_width(), ui.cursor().min.y);
        self.take_into_playlist(ui, "drop_playlist_card", Rect::from_min_max(from, to), &id);
    }

    /// Where the layers go: pictures as they are, a GIF's first frame, the rest
    /// marked where it goes.
    fn preview(&mut self, ui: &mut Ui, d: &Screen) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let w = ui.available_width().min(PREVIEW_W);
        let h = w * 9.0 / 16.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        resp.tip("Where the layers go. Pictures and a GIF's first frame are shown as they are; videos, shaders and pages are marked, and move in the game.");
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p.base);
        for (i, l) in d.layers.iter().enumerate() {
            let kind = kind_of(&l.file);
            let mut r = Rect::from_min_size(
                pos2(rect.min.x + l.x * 0.01 * w, rect.min.y + l.y * 0.01 * h),
                Vec2::new(l.w * 0.01 * w, l.h * 0.01 * h),
            );
            let picture = (kind == FileKind::Picture && gz_screens::import::extension(&l.file) == ".dds")
                || kind == FileKind::Gif;
            let stamp = if picture { self.stamp(&l.file) } else { None };
            let thumb =
                if picture { self.thumbs.get(self.editor, &ctx, self.root, &l.file, stamp) } else { None };
            if let Some((tid, size)) = thumb.filter(|(_, s)| s.x > 0.0 && s.y > 0.0) {
                let mut uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                let (bw, bh) = (r.width(), r.height());
                match l.fit {
                    Fit::Cover => {
                        let k = (bw / size.x).max(bh / size.y);
                        let (u, v) = (bw / k / size.x, bh / k / size.y);
                        uv = Rect::from_min_max(
                            pos2((1.0 - u) * 0.5, (1.0 - v) * 0.5),
                            pos2((1.0 + u) * 0.5, (1.0 + v) * 0.5),
                        );
                    }
                    Fit::Contain => {
                        let k = (bw / size.x).min(bh / size.y);
                        let (ix, iy) = ((bw - size.x * k) * 0.5, (bh - size.y * k) * 0.5);
                        r = Rect::from_min_max(
                            pos2(r.min.x + ix, r.min.y + iy),
                            pos2(r.max.x - ix, r.max.y - iy),
                        );
                    }
                    Fit::Stretch => {}
                }
                // a picture's own tint: its opacity
                let a = (l.opacity.clamp(0.0, 1.0) * 255.0) as u8;
                painter.image(tid, r, uv, Color32::from_rgba_unmultiplied(255, 255, 255, a));
            } else if kind != FileKind::None {
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, p.border), StrokeKind::Inside);
                let c = pos2(r.center().x, r.center().y - 16.0 + i as f32 * 18.0);
                icons::draw(
                    &painter,
                    kind_icon(kind),
                    Rect::from_center_size(c, Vec2::splat(16.0)),
                    p.text_secondary,
                );
            }
        }
        ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, p.border), StrokeKind::Inside);
    }

    fn pick(&mut self, ui: &mut Ui, id_salt: &str, value: &str, on_clear: Change, browse: Pick) {
        // a file of the project: shown as it is
        match pick_field(ui, id_salt, Tx::raw(value), !value.is_empty()) {
            PickAction::Clear => self.out.changes.push(on_clear),
            PickAction::Browse => self.out.picks.push((
                "Screens and music",
                &[
                    "dds", "gif", "mp4", "webm", "wmv", "ogm", "ps", "s", "html", "htm", "png", "jpg",
                    "jpeg", "tga", "bmp", "ogg",
                ],
                browse,
            )),
            PickAction::None => {}
        }
    }

    fn layer_details(&mut self, ui: &mut Ui, d: &Screen, i: usize, l: &Layer) {
        let kind = kind_of(&l.file);
        let id = d.id.clone();
        PropertyGrid::new("screen_layer").label_frac(0.28).show(ui, |g: &mut Grid<'_>| {
            g.row("File", "A picture, a GIF, a video, a shader or an HTML page. Drop one here from anywhere.", PropFlags::NONE, 0, |ui| {
                self.pick(ui, "layer_file", &l.file, set_layer(&id, i, |l| l.file.clear()), Pick::LayerFile(id.clone(), i));
            });
            if matches!(kind, FileKind::Picture | FileKind::Gif | FileKind::Video) {
                g.row(Tx::ctx("screens", "Fit"), "Cover fills the place and cuts the picture to it. Contain shows it whole. Stretch fills the place whatever the proportions.", PropFlags::NONE, 0, |ui| {
                    let mut cur = match l.fit {
                        Fit::Cover => 0,
                        Fit::Contain => 1,
                        Fit::Stretch => 2,
                    };
                    if segmented(ui, &mut cur, &["Cover", "Contain", "Stretch"], None) {
                        let fit = [Fit::Cover, Fit::Contain, Fit::Stretch].get(cur).copied().unwrap_or_default();
                        self.out.changes.push(set_layer(&id, i, move |l| l.fit = fit));
                    }
                });
            }
            if kind != FileKind::None {
                g.row("Opacity", "", PropFlags::NONE, 0, |ui| {
                    let key = format!("{id}.{i}.opacity");
                    if let Some(v) = edited_number(ui, self.edit, &key, l.opacity * 100.0, 1.0, "%", 0.0, 100.0) {
                        self.out.changes.push(set_layer(&id, i, move |l| l.opacity = v * 0.01));
                    }
                });
            }
            if kind == FileKind::Shader {
                g.row("Texture", "The picture the shader receives as s_base. Optional.", PropFlags::NONE, 0, |ui| {
                    self.pick(ui, "layer_texture", &l.texture, set_layer(&id, i, |l| l.texture.clear()), Pick::LayerTexture(id.clone(), i));
                });
            }
            if kind == FileKind::Video && d.music == "video" {
                g.row("Its Sound", "This video is heard while the screen shows.", PropFlags::INLINE, 0, |ui| {
                    let mut on = l.sound;
                    if switch(ui, &mut on).changed() {
                        self.out.changes.push(set_layer(&id, i, move |l| l.sound = on));
                    }
                });
            }
            if kind != FileKind::None {
                g.section("Position", false, None, |g| {
                    for (label, which, value) in [("Left", 0usize, l.x), ("Top", 1, l.y), ("Width", 2, l.w), ("Height", 3, l.h)] {
                        g.row(label, "", PropFlags::NONE, 0, |ui| {
                            let key = format!("{id}.{i}.rect{which}");
                            if let Some(v) = edited_number(ui, self.edit, &key, value, 0.1, "%", -100.0, 200.0) {
                                self.out.changes.push(set_layer(&id, i, move |l| match which {
                                    0 => l.x = v,
                                    1 => l.y = v,
                                    2 => l.w = v,
                                    _ => l.h = v,
                                }));
                            }
                        });
                    }
                });
            }
        });
    }

    fn layers(&mut self, ui: &mut Ui, d: &Screen) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let from = ui.cursor().min;
        if local_list_header(ui, "Layers", "Add a layer") {
            let id = d.id.clone();
            self.out.changes.push(change(move |s| {
                let mut sel = Selection::default();
                if let Some(d) = s.screen_mut(&id) {
                    d.layers.push(Layer::default());
                    sel.layer = Some(i64::try_from(d.layers.len()).unwrap_or(1) - 1);
                }
                sel
            }));
        }
        if d.layers.is_empty() {
            text::status(ui, Tone::Muted, "Add a layer, or drop a file here.");
        }
        let dragged = DragAndDrop::payload::<LayerDrag>(&ctx).filter(|p| p.screen == d.id);
        let released = ctx.input(|i| i.pointer.any_released());
        let pointer = ctx.pointer_latest_pos();
        // the top row is the top layer
        for i in (0..d.layers.len()).rev() {
            let Some(l) = d.layers.get(i) else { continue };
            let kind = kind_of(&l.file);
            let width = ui.available_width();
            let (at, resp) = ui.allocate_exact_size(Vec2::new(width, LAYER_ROW), Sense::click_and_drag());
            let selected = self.view.sel_layer == i64::try_from(i).unwrap_or(-1);
            if resp.clicked() {
                self.view.sel_layer = if selected { -1 } else { i64::try_from(i).unwrap_or(-1) };
            }
            if resp.drag_started() {
                DragAndDrop::set_payload(&ctx, LayerDrag { screen: d.id.clone(), index: i });
            }
            if let Some(drag) = &dragged
                && pointer.is_some_and(|pt| at.contains(pt))
            {
                if released {
                    let (id, from_i) = (d.id.clone(), drag.index);
                    DragAndDrop::clear_payload(&ctx);
                    if from_i != i {
                        if self.view.sel_layer == i64::try_from(from_i).unwrap_or(-1) {
                            self.view.sel_layer = i64::try_from(i).unwrap_or(-1);
                        }
                        self.out.changes.push(set_screen(&id, move |d| {
                            if from_i < d.layers.len() {
                                let moved = d.layers.remove(from_i);
                                d.layers.insert(i.min(d.layers.len()), moved);
                            }
                        }));
                    }
                } else if drag.index != i {
                    local_drop_mark(ui, at);
                }
            }
            let painter = ui.painter();
            if selected {
                painter.rect_filled(at, CornerRadius::same(3), p.selection);
            } else if resp.hovered() {
                painter.rect_filled(at, CornerRadius::same(3), p.hover);
            }
            let cy = at.center().y;
            icons::draw(
                painter,
                Icon::Grip,
                Rect::from_min_size(pos2(at.min.x + 4.0, cy - 6.0), Vec2::splat(12.0)),
                p.text_disabled,
            );
            icons::draw(
                painter,
                kind_icon(kind),
                Rect::from_min_size(pos2(at.min.x + 20.0, cy - 7.0), Vec2::splat(14.0)),
                p.text_secondary,
            );
            let right = at.min.x + width - 26.0;
            let name =
                if l.file.is_empty() { Tx::from("Empty: drop a file here") } else { Tx::raw(leaf(&l.file)) };
            text::paint(
                painter,
                &Font::Body.id(),
                pos2(at.min.x + 40.0, cy),
                egui::Align2::LEFT_CENTER,
                if kind == FileKind::None { p.text_secondary } else { p.text },
                name,
                Some((right - at.min.x - 120.0).max(0.0)),
                Clip::Start,
            );
            text::paint(
                painter,
                &Font::Small.id(),
                pos2(right - 68.0, cy),
                egui::Align2::LEFT_CENTER,
                p.text_secondary,
                kind.label(),
                None,
                Clip::End,
            );
            let b = Rect::from_min_size(pos2(right, at.min.y + 3.0), Vec2::splat(22.0));
            let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui-pda.layer_remove", i)).max_rect(b));
            if icon_button_sized(
                &mut child,
                Icon::Close,
                "Remove this layer. Its file stays in the project.",
                false,
                true,
                22.0,
                metrics::GLYPH_ROW,
            )
            .clicked()
            {
                if selected {
                    self.view.sel_layer = -1;
                }
                let id = d.id.clone();
                self.out.changes.push(set_screen(&id, move |d| {
                    if i < d.layers.len() {
                        d.layers.remove(i);
                    }
                }));
            }
            if let Some(files) = take_files(ui, &format!("drop_layer.{i}"), at, false)
                && let Some(first) = files.into_iter().next()
            {
                let at = DropAt {
                    screen: d.id.clone(),
                    layer: i64::try_from(i).unwrap_or(-1),
                    ..DropAt::default()
                };
                self.out.drops.push((vec![first], at));
            }
            if selected {
                ui.indent(("gz-ui-pda.layer_details", i), |ui| self.layer_details(ui, d, i, l));
            }
        }
        // the dragged row follows the pointer
        if let (Some(drag), Some(pt)) = (&dragged, pointer)
            && let Some(l) = d.layers.get(drag.index)
        {
            let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("gz-ui-pda.layer_drag")));
            let name = if l.file.is_empty() { Tx::from("Empty layer") } else { Tx::raw(leaf(&l.file)) };
            let r = text::bounds(
                &ctx,
                &Font::Body.id(),
                pos2(pt.x + 14.0, pt.y + 4.0),
                egui::Align2::LEFT_TOP,
                &name,
            )
            .expand2(Vec2::new(8.0, 4.0));
            painter.rect_filled(r, CornerRadius::same(metrics::RADIUS), Palette::alpha(p.overlay, 0.92));
            text::paint(
                &painter,
                &Font::Body.id(),
                pos2(pt.x + 14.0, pt.y + 4.0),
                egui::Align2::LEFT_TOP,
                p.text,
                name,
                None,
                Clip::End,
            );
        }
        let to = pos2(from.x + ui.available_width(), ui.cursor().min.y + 8.0);
        if let Some(files) = take_files(ui, "drop_layers", Rect::from_min_max(from, to), false) {
            self.out.drops.push((files, DropAt { screen: d.id.clone(), layer: -1, ..DropAt::default() }));
        }
    }

    /// What plays with the screen: the kit's combo over Any Track, Silence, the video's own sound
    /// and the tracks of the surface; a track is ticked on and off.
    fn music_combo(&mut self, ui: &mut Ui, d: &Screen) {
        let id = d.id.clone();
        let loading = self.view.surface == 0;
        let mut items = vec![PickItem::new("any", "Any Track"), PickItem::new("none", "Silence")];
        if loading {
            items.push(PickItem::new("video", "The Video's Own Sound"));
        }
        let simple = items.len();
        for t in &self.s.tracks {
            let on = d.music == "tracks" && d.tracks.contains(&t.id);
            let label = if on { format!("\u{2713} {}", t.id) } else { format!("   {}", t.id) };
            items.push(PickItem::new(t.id.clone(), Tx::raw(label)).icon(Icon::FileSound).detail(if on {
                "Plays with this screen: pick to take it off"
            } else {
                "Pick to play it with this screen"
            }));
        }
        let current = if d.music == "tracks" { String::new() } else { d.music.clone() };
        let picked = Combo::new(("gz-ui-pda.music", &id), music_label(d)).show(
            ui,
            PickList::new(("gz-ui-pda.music_list", &id), &items).current(&current).without_search(),
        );
        let Some(i) = picked else { return };
        let change = if i < simple {
            let value = items.get(i).map(|it| it.id.clone()).unwrap_or_default();
            set_screen(&id, move |d| {
                d.music.clone_from(&value);
                d.tracks.clear();
            })
        } else {
            let Some(t) = items.get(i).map(|it| it.id.clone()) else { return };
            let on = d.music == "tracks" && d.tracks.contains(&t);
            set_screen(&id, move |d| {
                if d.music != "tracks" {
                    d.tracks.clear();
                }
                if on {
                    d.tracks.retain(|x| *x != t);
                } else if !d.tracks.contains(&t) {
                    d.tracks.push(t.clone());
                }
                d.music = if d.tracks.is_empty() { "any" } else { "tracks" }.to_owned();
            })
        };
        self.out.changes.push(change);
    }

    /// What the tab knows of a track's sound.
    fn info_of(&self, t: &Track) -> Option<&TrackInfo> {
        t.is_sound().then(|| self.infos.as_deref().and_then(|i| i.get(&t.sound))).flatten()
    }

    /// The quiet mark of a copyrighted track's sound: what it does in the streamer mode.
    fn mark_of(&self, t: &Track) -> Option<Tx<'static>> {
        self.info_of(t).filter(|i| i.copyrighted()).map(mark_line)
    }

    fn levels_row(&mut self, ui: &mut Ui, d: &Screen) {
        let items: Vec<Tx<'_>> = d.levels.iter().map(|l| Tx::raw(l.as_str())).collect();
        if let Some(i) = chips(ui, "screen_levels", &items, true) {
            let id = d.id.clone();
            self.out.changes.push(set_screen(&id, move |d| {
                if i < d.levels.len() {
                    d.levels.remove(i);
                }
            }));
        }
        match self.levels {
            // values from a list are picked
            Some(known) => {
                let free: Vec<String> = known
                    .iter()
                    .filter(|k| !d.levels.iter().any(|l| l.eq_ignore_ascii_case(k)))
                    .cloned()
                    .collect();
                let mut add: Option<String> = None;
                let resp = gz_ui::Button::new("Add Level")
                    .kind(Kind::Ghost)
                    .icon(Icon::Plus)
                    .enabled(!free.is_empty(), "Every level of the linked game is on the list")
                    .show(ui);
                let popup = ui.make_persistent_id("gz-ui-pda.add_level");
                if resp.clicked() {
                    egui::Popup::toggle_id(ui.ctx(), popup);
                }
                gz_ui::menu::menu_popup(ui.ctx(), popup, resp.rect, ui.layer_id(), |ui| {
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for k in &free {
                            if menu_item(ui, Tx::raw(k.as_str()), MenuItem::default()) {
                                add = Some(k.clone());
                            }
                        }
                    });
                });
                if let Some(level) = add {
                    let id = d.id.clone();
                    self.out.changes.push(set_screen(&id, move |d| d.levels.push(level.clone())));
                }
            }
            // no list of the game's levels: typed
            None => {
                let resp = text_field(ui, &mut self.edit.level);
                if resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.edit.level.trim().is_empty()
                {
                    let level = std::mem::take(&mut self.edit.level).trim().to_owned();
                    let id = d.id.clone();
                    self.out.changes.push(set_screen(&id, move |d| d.levels.push(level.clone())));
                }
            }
        }
    }

    fn screen_details(&mut self, ui: &mut Ui, d: &Screen) {
        for (error, msg) in problems_of(self.problems, "screen", &d.id) {
            text::status(ui, if error { Tone::Error } else { Tone::Warning }, Tx::ui(msg));
        }
        self.preview(ui, d);
        ui.add_space(6.0);
        self.layers(ui, d);
        ui.add_space(6.0);
        let id = d.id.clone();
        let mode = self.mode.to_owned();
        PropertyGrid::new("screen").label_frac(0.28).show(ui, |g: &mut Grid<'_>| {
            g.row("Music", "What plays with this screen. Any Track is the shuffle of this surface; pick tracks to tie them to the screen.", PropFlags::NONE, 0, |ui| self.music_combo(ui, d));
            g.section("More", false, None, |g| {
                g.row("Name", "How this screen is called in the list and in other mods' eyes. Letters, digits and _.", PropFlags::NONE, 0, |ui| {
                    let mut text_ = match &self.edit.rename {
                        Some((rid, t)) if *rid == id => t.clone(),
                        _ => id.clone(),
                    };
                    let resp = text_field(ui, &mut text_);
                    if resp.changed() {
                        self.edit.rename = Some((id.clone(), text_.clone()));
                    }
                    if resp.lost_focus() {
                        self.edit.rename = None;
                        let wanted = sanitize(&text_);
                        if wanted != id && self.s.screen(&wanted).is_none() {
                            let old = id.clone();
                            self.view.sel_screen.clone_from(&wanted);
                            self.out.changes.push(change(move |s| {
                                if s.screen(&wanted).is_none()
                                    && let Some(d) = s.screen_mut(&old)
                                {
                                    d.id.clone_from(&wanted);
                                }
                                Selection { screen: Some(wanted.clone()), ..Selection::default() }
                            }));
                        }
                    }
                });
                g.row("Show On", "", PropFlags::NONE, 0, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let mut show = d.show;
                        let mut changed = mask_check(ui, "Loading", &mut show, LOADING, false);
                        changed |= mask_check(ui, "Main Menu", &mut show, MENU, false);
                        changed |= mask_check(ui, "Pause Menu", &mut show, PAUSE, false);
                        if changed {
                            self.out.changes.push(set_screen(&id, move |d| d.show = show));
                        }
                    });
                });
                g.row("Levels", "Only while these levels load or run. Empty: everywhere.", PropFlags::NONE, 0, |ui| {
                    ui.vertical(|ui| self.levels_row(ui, d));
                });
                // a module for every game or for the plain game too has no modes of its own to limit a screen
                // to, but a screen the file limits already is shown, so the limit can be taken off
                if (!mode.is_empty() || !d.modes.is_empty()) && d.show & (LOADING | PAUSE) != 0 {
                    let named = if mode.is_empty() { d.modes.join(", ") } else { mode.clone() };
                    let help = tr!("Shown only in games of this module's modes ({mode}). The main menu belongs to no game.", mode = named.as_str());
                    g.row("Only In Its Mode", help, PropFlags::INLINE, 0, |ui| {
                        let mut only = !d.modes.is_empty();
                        if switch(ui, &mut only).changed() {
                            let m = mode.clone();
                            self.out.changes.push(set_screen(&id, move |d| {
                                d.modes.clear();
                                if only {
                                    d.modes.extend(gz_mod::mode_list(&m));
                                }
                            }));
                        }
                    });
                }
                if d.show & LOADING != 0 {
                    g.row("Show Over It", "What of the loading screen's interface stays on top of this screen.", PropFlags::NONE, 0, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut hide = d.hide;
                            let mut changed = mask_check(ui, "Bar", &mut hide, HIDE_BAR, true);
                            changed |= mask_check(ui, "Per Cent", &mut hide, HIDE_PERCENT, true);
                            changed |= mask_check(ui, "Stage", &mut hide, HIDE_STAGE, true);
                            changed |= mask_check(ui, "Tip", &mut hide, HIDE_TIP, true);
                            if changed {
                                self.out.changes.push(set_screen(&id, move |d| d.hide = hide));
                            }
                        });
                    });
                }
                g.row("Weight", "How often this screen comes up against the others. The game's own screen weighs 1.", PropFlags::NONE, 0, |ui| {
                    let key = format!("{id}.weight");
                    if let Some(v) = edited_number(ui, self.edit, &key, d.weight, 0.05, "", 0.0, 100.0) {
                        self.out.changes.push(set_screen(&id, move |d| d.weight = v));
                    }
                });
            });
        });
    }

    fn track_details(&mut self, ui: &mut Ui, t: &Track) {
        for (error, msg) in problems_of(self.problems, "track", &t.id) {
            text::status(ui, if error { Tone::Error } else { Tone::Warning }, Tx::ui(msg));
        }
        let info = self.info_of(t).cloned();
        ui.horizontal(|ui| {
            if gz_ui::Button::new(Tx::ctx("playback", "Play"))
                .icon(Icon::Play)
                .enabled(t.is_sound() || !t.file.is_empty(), "The track has no music.")
                .show(ui)
                .clicked()
            {
                play_track(self.editor, t, info.as_ref());
            }
            if gz_ui::Button::new(Tx::ctx("playback", "Stop")).icon(Icon::Stop).show(ui).clicked() {
                crate::screens::stop(self.editor);
            }
        });
        if let Some(line) = info.as_ref().filter(|i| i.copyrighted()).map(mark_line) {
            ui.add_space(2.0);
            text::status(ui, Tone::Muted, line);
        }
        ui.add_space(4.0);
        let id = t.id.clone();
        PropertyGrid::new("track").label_frac(0.28).show(ui, |g: &mut Grid<'_>| {
            if t.is_sound() {
                let asset = info.as_ref().and_then(|i| i.facts.asset.clone());
                g.row("Sound", "The sound the track plays, by its name. Whether it is copyrighted is the sound's to say: Open Sound.", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, Tx::raw(t.sound.as_str()), Clip::Start);
                });
                let length = info.as_ref().and_then(|i| i.seconds).map(clock);
                g.row("Length", "How long the sound lasts.", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, length.map_or(Tx::from("Unknown"), Tx::raw), Clip::End);
                });
                g.block(|ui| {
                    let why = if info.is_none() { "Reading the sound..." } else { "A sound of the game has no sound asset of the module." };
                    if gz_ui::Button::new("Open Sound")
                        .icon(Icon::SoundAsset)
                        .kind(Kind::Secondary)
                        .enabled(asset.is_some(), why)
                        .tip("Opens the sound asset: its keys, and whether it is copyrighted.")
                        .show(ui)
                        .clicked()
                        && let Some(a) = &asset
                    {
                        self.out.open_sound = Some(a.clone());
                    }
                });
            } else {
                g.row("File", "A plain .ogg of the project: it has no description, so it cannot be marked copyrighted.", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, Tx::raw(t.file.as_str()), Clip::Start);
                });
                g.block(|ui| {
                    if gz_ui::Button::new("Make It a Sound")
                        .icon(Icon::SoundAsset)
                        .kind(Kind::Secondary)
                        .enabled(!t.file.is_empty(), "The track has no file.")
                        .tip("Only a sound can carry the copyrighted mark: a sound asset is written over the file and the track plays that sound.")
                        .show(ui)
                        .clicked()
                    {
                        self.out.make_sound = Some(id.clone());
                    }
                });
            }
            g.row("Shuffle On", "Where this track is played at random: on the surfaces, and on the radios of the world. Leave all off to play it only with the screens and playlists that name it.", PropFlags::NONE, 0, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut play = t.play;
                    let mut changed = mask_check(ui, "Loading", &mut play, LOADING, false);
                    changed |= mask_check(ui, "Main Menu", &mut play, MENU, false);
                    changed |= mask_check(ui, "Pause Menu", &mut play, PAUSE, false);
                    changed |= mask_check(ui, "Radio", &mut play, RADIO, false);
                    if changed {
                        self.out.changes.push(set_track(&id, move |t| t.play = play));
                    }
                });
            });
            g.row("Volume", "", PropFlags::NONE, 0, |ui| {
                let key = format!("{id}.volume");
                if let Some(v) = edited_number(ui, self.edit, &key, t.volume * 100.0, 1.0, "%", 0.0, 100.0) {
                    self.out.changes.push(set_track(&id, move |t| t.volume = v * 0.01));
                }
            });
            g.row("Weight", "How often this track comes up against the others.", PropFlags::NONE, 0, |ui| {
                let key = format!("track.{id}.weight");
                if let Some(v) = edited_number(ui, self.edit, &key, t.weight, 0.05, "", 0.0, 100.0) {
                    self.out.changes.push(set_track(&id, move |t| t.weight = v));
                }
            });
        });
    }
}

/// `1:05`, the length of a track; `0:03.2` for a short one.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0);
    if s < 10.0 {
        return format!("0:{s:04.1}");
    }
    let whole = s.round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// What a track's Add picks: music, made a sound asset of the module.
const MUSIC_FILES: &[&str] = &["ogg", "wav"];

/// The quiet mark of a copyrighted sound on its rows.
const COPYRIGHT: &str = "\u{a9}";

use gz_ui::Palette;

/// A track of the Music list or of Radio Music dragged out of its row (from `top` to the cursor):
/// a playlist takes it.
fn drag_track(ui: &mut Ui, top: f32, list: &str, id: &str) {
    let rect = Rect::from_min_max(
        pos2(ui.cursor().min.x, top),
        pos2(ui.cursor().min.x + ui.available_width(), ui.cursor().min.y),
    );
    let resp = ui.interact(rect, Id::unique(("gz-ui-pda.track_drag", list, id)), Sense::drag());
    gz_ui::drop::start(
        &resp,
        gz_ui::drop::Payload::Row { surface: TRACK_ROW.to_owned(), key: id.to_owned() },
    );
    if resp.dragged()
        && let Some(pt) = ui.ctx().pointer_latest_pos()
    {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let painter =
            ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("gz-ui-pda.track_drag_label")));
        let at = pos2(pt.x + 14.0, pt.y + 4.0);
        let r = text::bounds(&ctx, &Font::Body.id(), at, egui::Align2::LEFT_TOP, Tx::raw(id))
            .expand2(Vec2::new(8.0, 4.0));
        painter.rect_filled(r, CornerRadius::same(metrics::RADIUS), Palette::alpha(p.overlay, 0.92));
        text::paint(
            &painter,
            &Font::Body.id(),
            at,
            egui::Align2::LEFT_TOP,
            p.text,
            Tx::raw(id),
            None,
            Clip::End,
        );
    }
}

/// What a playlist takes: a track of the Music list, or files.
fn accepts_tracks(p: &gz_ui::drop::Payload) -> bool {
    matches!(p, gz_ui::drop::Payload::Row { surface, .. } if surface == TRACK_ROW)
        || gz_ui::drop::accepts_files(p)
}

/// The files dropped on `rect` this frame, from Explorer or from tiles of the Content Browser: the
/// kit's drop board, so a drop is taken once whoever asks first. `catch_all` is the outermost zone
/// of the tab.
fn take_files(ui: &Ui, name: &str, rect: Rect, catch_all: bool) -> Option<Vec<String>> {
    let zone = gz_ui::drop::Zone::new(name, rect, &gz_ui::drop::accepts_files);
    let zone = if catch_all { zone.catch_all() } else { zone };
    zone.take(ui).map(|p| p.files().iter().map(|f| f.display().to_string()).collect())
}

impl Document for ScreensDoc {
    fn id(&self) -> &str {
        "screens"
    }

    fn kind(&self) -> &'static str {
        "screens"
    }

    fn label(&self) -> Tx<'static> {
        Tx::from("Screens & Music")
    }

    fn icon(&self) -> Icon {
        Icon::Picture
    }

    fn tooltip(&self) -> Option<Tx<'static>> {
        Some(Tx::from("The module's loading and menu screens and their music - saved to mod.ltx as you edit"))
    }

    fn alive(&self) -> bool {
        self.tab.v.lock().open
            && self.editor.project_opt().is_some_and(|s| s.root.display().to_string() == self.tab.root)
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let ctx = cx.ctx.clone();
        let (surface, status) = {
            let v = self.tab.v.lock();
            (v.surface.min(1), v.status.clone())
        };
        let (have, only_s, only_m) = self.local.as_ref().map_or((false, false, false), |l| {
            (
                l.have,
                l.s.only_screens.get(surface).copied().unwrap_or(false),
                l.s.only_music.get(surface).copied().unwrap_or(false),
            )
        });
        tb.group(2);
        let mut sfc = surface;
        if tb.segmented("surface", "Surface", &mut sfc, &["Loading", "Menu"], "Which screens to edit", true) {
            let mut v = self.tab.v.lock();
            v.surface = sfc;
            v.sel_layer = -1;
        }
        tb.separator();
        tb.group(1);
        caption(tb, &ctx, "screens_caption", "Screens");
        let mut screens = usize::from(only_s);
        if tb.segmented(
            "policy_screens",
            "Screens",
            &mut screens,
            &["Mix In", "Only Mine"],
            "Mix In: yours come up among the game's and other mods'. Only Mine: nobody else's are shown.",
            have,
        ) {
            let only = screens == 1;
            self.apply(change(move |s| {
                if let Some(slot) = s.only_screens.get_mut(surface) {
                    *slot = only;
                }
                Selection::default()
            }));
        }
        tb.group(1);
        caption(tb, &ctx, "music_caption", "Music");
        let mut music = usize::from(only_m);
        if tb.segmented(
            "policy_music",
            "Music",
            &mut music,
            &["Mix In", "Only Mine"],
            "Mix In: yours plays among the game's and other mods'. Only Mine: nobody else's is played, and no tracks of yours means silence.",
            have,
        ) {
            let only = music == 1;
            self.apply(change(move |s| {
                if let Some(slot) = s.only_music.get_mut(surface) {
                    *slot = only;
                }
                Selection::default()
            }));
        }
        tb.group(0);
        tb.separator();
        if status.is_empty() {
            tb.status("Saved to mod.ltx as you edit", Tone::Muted);
        } else {
            tb.status(Tx::ui(status), Tone::Error);
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let full = ui.max_rect();
        if self.editor.project_opt().is_none() {
            empty_state(
                ui,
                Icon::Picture,
                "No project",
                "Screens and music live in the mod.ltx of the open project.",
                None,
            );
            return;
        }
        self.refresh();
        let Some(local) = self.local.take() else {
            busy_line(ui, "Reading mod.ltx...");
            return;
        };
        if !local.have {
            if empty_state(
                ui,
                Icon::Picture,
                "No mod.ltx yet",
                "Screens and music live in the module's mod.ltx. Mod Settings makes it.",
                Some("Open Mod Settings".into()),
            ) {
                cx.requests.open(OpenRequest::kind("mod_settings"));
            }
            self.local = Some(local);
            return;
        }
        let tree = Project::of(&self.editor).ok().map(|(_, p)| p.tree.snapshot());
        let levels = self.levels.lock().clone().flatten();
        let mut view = self.tab.v.lock().clone();
        let infos = self.infos(&local.s, local.rev);
        let before = view.clone();
        let mut out = Out::default();
        {
            let root = self.tab.root.clone();
            let mut f = Frame {
                editor: &self.editor,
                root: &root,
                s: &local.s,
                mode: &local.mode,
                problems: &self.problems,
                levels: levels.as_deref(),
                view: &mut view,
                out: &mut out,
                edit: &mut self.edit,
                thumbs: &mut self.thumbs,
                tree,
                infos,
            };
            let gap = 8.0;
            let left = Rect::from_min_max(full.min, pos2((full.min.x + LEFT_W).min(full.max.x), full.max.y));
            let right = Rect::from_min_max(pos2((left.max.x + gap).min(full.max.x), full.min.y), full.max);
            let mut l_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-pda.screens_left")
                    .max_rect(left)
                    .layout(Layout::top_down(Align::Min)),
            );
            ScrollArea::vertical().id_salt("gz-ui-pda.screens_left_scroll").auto_shrink([false, false]).show(
                &mut l_ui,
                |ui| {
                    f.screens_list(ui);
                    ui.add_space(12.0);
                    f.music_list(ui);
                    ui.add_space(12.0);
                    f.radio_list(ui);
                    ui.add_space(12.0);
                    f.playlists_list(ui);
                },
            );
            let mut r_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-pda.screens_right")
                    .max_rect(right)
                    .layout(Layout::top_down(Align::Min)),
            );
            ScrollArea::vertical()
                .id_salt("gz-ui-pda.screens_right_scroll")
                .auto_shrink([false, false])
                .show(&mut r_ui, |ui| {
                    let avail = ui.available_width();
                    let column = avail.min(COLUMN_W);
                    let x0 = ui.cursor().min.x + ((avail - column) * 0.5).max(0.0);
                    let col =
                        Rect::from_min_size(pos2(x0, ui.cursor().min.y), Vec2::new(column, f32::INFINITY));
                    ui.scope_builder(
                        UiBuilder::new().max_rect(col).layout(Layout::top_down(Align::Min)),
                        |ui| {
                            ui.set_width(column);
                            let screen = (!f.view.sel_screen.is_empty())
                                .then(|| f.s.screen(&f.view.sel_screen))
                                .flatten()
                                .cloned();
                            let track = (!f.view.sel_track.is_empty())
                                .then(|| f.s.track(&f.view.sel_track))
                                .flatten()
                                .cloned();
                            let playlist = (!f.view.sel_playlist.is_empty())
                                .then(|| f.s.playlist(&f.view.sel_playlist))
                                .flatten()
                                .cloned();
                            if let Some(d) = screen {
                                f.screen_details(ui, &d);
                            } else if let Some(t) = track {
                                f.track_details(ui, &t);
                            } else if let Some(p) = playlist {
                                f.playlist_details(ui, &p);
                            } else {
                                empty_state(
                                    ui,
                                    Icon::Picture,
                                    "Pick a screen, a track or a playlist",
                                    "Or drop files anywhere on this tab.",
                                    None,
                                );
                            }
                        },
                    );
                });
            // what no list or layer took falls on the tab: sorted by what it is
            if let Some(files) = take_files(ui, "drop_tab", full, true) {
                f.out.drops.push((files, DropAt { layer: -1, ..DropAt::default() }));
            }
        }
        ui.allocate_rect(full, Sense::hover());
        self.local = Some(local);
        {
            let mut v = self.tab.v.lock();
            if view.surface != before.surface {
                v.surface = view.surface;
            }
            if view.sel_screen != before.sel_screen
                || view.sel_track != before.sel_track
                || view.sel_playlist != before.sel_playlist
                || view.sel_layer != before.sel_layer
            {
                v.sel_screen = view.sel_screen;
                v.sel_track = view.sel_track;
                v.sel_playlist = view.sel_playlist;
                v.sel_layer = view.sel_layer;
            }
        }
        self.finish(out, cx);
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        self.tab.v.lock().open = false;
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Plugs the tab and Mod > Screens & Music into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.document("screens", |editor, _req| {
        let session = editor.project().map_err(|e| e.message)?;
        let tab = ScreensTab::of_session(&session);
        tab.v.lock().open = true;
        Ok(Box::new(ScreensDoc::new(editor.clone(), tab)) as Box<dyn Document>)
    });
    b.action(
        gz_app::Action::new(gz_ui_mod::menu::SCREENS, "Screens & Music", "Mod", "ui.tab")
            .icon(Icon::Picture)
            .args(|_| gz_app::args(&[("action", "open"), ("kind", "screens")]))
            .enabled(gz_ui_mod::menu::with_project),
    );
}

/// Frees the one file dialog's place when its answer is dropped, picked or not.
struct ReleasePick(Arc<AtomicBool>);

impl Drop for ReleasePick {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A toolbar caption before its segmented control: muted, with the room of a group gap after it
/// so the two read as one ("Screens  [Mix In | Only Mine]"). The overflow names the control by
/// the same caption.
pub(crate) fn caption(tb: &mut Toolbar<'_, '_>, ctx: &egui::Context, id: &str, text_: &'static str) {
    let font = Font::Small.id();
    let w = gz_ui::text::width(ctx, &font, text_) + 8.0;
    tb.widget(id, w, |ui| {
        let p = gz_ui::palette(ui.ctx());
        let r = ui.max_rect();
        gz_ui::text::paint(
            ui.painter(),
            &font,
            pos2(r.min.x, r.center().y),
            egui::Align2::LEFT_CENTER,
            p.text_muted,
            text_,
            None,
            gz_ui::Clip::End,
        );
    });
}
