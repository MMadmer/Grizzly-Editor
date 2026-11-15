//! The sound preview: the `sound` tab (docs/UI_DESIGN.md, section 6, "Sound Preview").
//!
//! One sound asset: the sound itself (Play / Stop, where it is and how long it lasts, its file
//! and format) and what the asset says, as the kit's property grid made from `gz-media`'s sound
//! table - the keys an author meets on the surface, the rest under one More, a key left out
//! showing its default as the field's hint, a Reset arrow taking a key out of the file. Copyrighted
//! is the one place the mark is said in the editor, Plays Instead right under it.
//!
//! It saves as it is edited: the change is written into the file's own text (`gz_asset`'s save,
//! the difference of the model), read from the disk again first, so a comment, a key the editor
//! does not know and another writer's change stay. The project's watcher brings the tab what
//! anybody writes. A frame never reads a file: loads, decodes, checks and writes run on the pools.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use egui::{Align, CornerRadius, Layout, Rect, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_app::{DocCx, Document, OpenRequest};
use gz_base::{Problem, RelPath, Severity};
use gz_editor::Editor;
use gz_fmt_sound::Pcm;
use gz_luadata::{Seg, Value};
use gz_media::fields::{Field, SOUND, Ty};
use gz_media::sound::{FreeSound, MARK_KEY, REPLACEMENT_KEY, SoundAsset};
use gz_project::asset_kind::MediaKind;
use gz_ui::grid::{Grid, format_number, parse_number, read_only, switch};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Combo, Font, Icon, Kind, PickItem, PickList, PropFlags, PropertyGrid, Tone, Toolbar, Tx, palette, state,
    tr,
};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde_json::{Map, json};

use crate::audio::Player;
use crate::local::{local_facts, local_heading};
use crate::sound_pick;

/// pt (docs/UI_DESIGN.md, section 6).
pub const COLUMN_W: f32 = 760.0;
const BAR_H: f32 = 4.0;
/// A number's field.
const NUMBER_W: f32 = 140.0;

/// What the source of the sound is, read once per state of the file.
#[derive(Clone, Debug, Default)]
pub struct SourceFacts {
    /// "Ogg Vorbis" or "WAV".
    pub format: &'static str,
    pub seconds: f64,
    pub channels: u32,
    pub rate: u32,
    pub bytes: u64,
}

/// The asset as it was read last.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    /// Why the file does not read; empty when it does.
    pub error: String,
    /// The file's value as read (a table), `Nil` when it does not read.
    pub value: Value,
    pub asset: SoundAsset,
    /// The name the game knows it by.
    pub name: String,
    /// The source, and its project path.
    pub source: Option<PathBuf>,
    pub source_rel: String,
    pub facts: Option<SourceFacts>,
    /// Why the source does not read.
    pub source_error: String,
    /// `asset.check`'s problems of the file.
    pub problems: Vec<Problem>,
    /// Where each key is written: its first and last line.
    pub spans: Vec<(&'static str, u32, u32)>,
}

impl Loaded {
    /// The value the file writes for a key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.value.get(key)
    }

    /// The key a problem is about, by the line it stands at.
    #[must_use]
    pub fn key_of(&self, p: &Problem) -> Option<&'static str> {
        let line = p.line?;
        self.spans.iter().find(|(_, a, b)| *a <= line && line <= *b).map(|(k, _, _)| *k)
    }

    /// The problems a key's row shows.
    pub fn problems_of<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Problem> + 'a {
        self.problems.iter().filter(move |p| self.key_of(p) == Some(key))
    }
}

/// A change of the file: keys set (`Some`) or taken out (`None`).
type Change = Vec<(&'static str, Option<Value>)>;

/// A sound decoded for the player, or why it is not.
type Decoded = Result<Arc<Pcm>, String>;

/// The sounds that may play instead, and the state of the files they were read at.
type FreeRead = (u64, Arc<Vec<FreeSound>>);

/// The sound tab.
pub struct SoundDoc {
    editor: Arc<Editor>,
    /// Project-relative, `\`.
    rel: String,
    loaded: Option<Arc<Loaded>>,
    inbox: Arc<Mutex<Option<Arc<Loaded>>>>,
    reading: Arc<AtomicBool>,
    /// Moves when the asset or its source changes on the disk.
    follow: Arc<AtomicU64>,
    /// The source the follower watches beside the asset, project-relative.
    source_watch: Arc<Mutex<String>>,
    follow_seen: u64,
    /// Edits on their way to the file.
    busy: Arc<AtomicUsize>,
    /// The last write's failure; empty when all is well.
    status: Arc<Mutex<String>>,
    /// The sound decoded for the player, made when it is first played.
    pcm: Option<Arc<Pcm>>,
    pcm_in: Arc<Mutex<Option<Decoded>>>,
    decoding: bool,
    /// What may play instead, read off the UI thread while the sound is copyrighted.
    free: Option<FreeRead>,
    free_in: Arc<Mutex<Option<FreeRead>>>,
    free_asked: Option<u64>,
    /// `preview.sound show=streamer`: the list goes up at the next frame.
    pub open_picker: bool,
    /// A view command that asks for no list: the one up goes.
    pub close_picker: bool,
    /// A field being typed: its key and its text.
    typing: FxHashMap<&'static str, String>,
    drawn_frame: Option<u64>,
}

/// The source of a sound read for its facts. Blocking.
fn source_facts(path: &std::path::Path) -> Result<SourceFacts, String> {
    let bytes = std::fs::read(path).map_err(|e| gz_base::io_reason(&e))?;
    let size = bytes.len() as u64;
    if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("ogg")) {
        let i = gz_fmt_sound::ogg_info(&bytes).map_err(|e| e.to_string())?;
        return Ok(SourceFacts {
            format: "Ogg Vorbis",
            seconds: i.seconds(),
            channels: u32::from(i.channels),
            rate: i.rate,
            bytes: size,
        });
    }
    let w = gz_fmt_sound::read_wav(&bytes).map_err(|e| e.to_string())?;
    Ok(SourceFacts {
        format: "WAV",
        seconds: w.frames() as f64 / f64::from(w.rate.max(1)),
        channels: u32::try_from(w.channels.len()).unwrap_or(0),
        rate: w.rate,
        bytes: size,
    })
}

/// The asset at `rel` read, checked and its source measured.
pub async fn load(editor: Arc<Editor>, rel: String) -> Loaded {
    let Ok(path) = RelPath::new(&rel.replace('\\', "/")) else {
        return Loaded { error: format!("{rel} is no path of the project"), ..Loaded::default() };
    };
    let file = match gz_asset::read(&editor, &path).await {
        Ok(f) => f,
        Err(f) => {
            let problem = *f.problem;
            return Loaded { error: problem.message.clone(), problems: vec![problem], ..Loaded::default() };
        }
    };
    let value = file.doc.value();
    let (asset, _) = SoundAsset::from_value(&value);
    let doc = file.place_doc();
    let spans = SOUND
        .iter()
        .filter_map(|f| doc.span(&[Seg::key(f.key)]).map(|s| (f.key, s.line, s.end_line)))
        .collect();
    let mut problems = file.notes_of(Severity::Note);
    let media = gz_media::Media::of(&editor).await.ok();
    let (name, root) = match &media {
        Some(m) => {
            let index = m.fresh_index().await;
            let name = index.at(&rel).map(|e| e.name.clone()).unwrap_or_else(|| {
                gz_media::files::published_name(asset.name.as_deref(), &rel, &index.module)
            });
            (name, m.root().to_path_buf())
        }
        None => (String::new(), PathBuf::new()),
    };
    if media.is_some() {
        problems.extend(gz_media::check::check(editor.clone(), Arc::new(file), MediaKind::Sound).await);
    }
    let r = rel.clone();
    let (source, facts, source_error) = editor
        .exec
        .io(move || match gz_media::resolve::sound(&root, &r) {
            Ok((_, source, _)) => match source_facts(&source) {
                Ok(f) => (Some(source), Some(f), String::new()),
                Err(e) => (Some(source), None, e),
            },
            Err(f) => (None, None, f.to_string()),
        })
        .await;
    let source_rel = match (&source, editor.project_root()) {
        (Some(s), Some(root)) => s.strip_prefix(&root).unwrap_or(s).display().to_string(),
        _ => String::new(),
    };
    Loaded {
        error: String::new(),
        value,
        asset,
        name,
        source,
        source_rel,
        facts,
        source_error,
        problems,
        spans,
    }
}

/// Whether a change of the project's file `changed` concerns the sound asset `rel` (both
/// project-relative, either slash): the asset itself or its source.
#[must_use]
pub fn touches(rel: &str, source: Option<&str>, changed: &str) -> bool {
    let key = |p: &str| p.replace('\\', "/").to_lowercase();
    let changed = key(changed);
    key(rel) == changed || source.is_some_and(|s| key(s) == changed)
}

/// A counter that moves whenever the asset `rel` or its source changes on the disk; it follows
/// the project's watcher until the counter is dropped.
fn follow_asset(editor: &Arc<Editor>, rel: &str, source: Arc<Mutex<String>>) -> Arc<AtomicU64> {
    let counter = Arc::new(AtomicU64::new(0));
    let Ok((_, project)) = gz_project::Project::of(editor) else { return counter };
    let mut rx = project.tree.changes().subscribe(gz_project::Filter::All);
    let (weak, ed, asset) = (Arc::downgrade(&counter), Arc::downgrade(editor), rel.to_owned());
    editor.exec.spawn(async move {
        while let Some(change) = rx.recv().await {
            let (Some(c), Some(ed)) = (weak.upgrade(), ed.upgrade()) else { break };
            let src = source.lock().clone();
            if touches(&asset, (!src.is_empty()).then_some(src.as_str()), change.path.as_str()) {
                c.fetch_add(1, Ordering::AcqRel);
                ed.exec.wake().ping();
            }
        }
    });
    counter
}

/// A value of the file as the window writes it in a field.
#[must_use]
pub fn shown(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(*n, 4),
        Value::String(s) => s.to_string(),
        Value::Table(t) => t.arr.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        Value::Nil => String::new(),
    }
}

/// A value of the file as the answer gives it.
fn value_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Bool(b) => json!(b),
        Value::Number(n) => json!(n),
        Value::String(s) => json!(s.as_str()),
        Value::Table(t) => json!(t.arr.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Value::Nil => serde_json::Value::Null,
    }
}

/// The words of a word list, each once, in its order.
fn words(list: &[(&'static str, u32)]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (w, _) in list {
        if !out.contains(w) {
            out.push(w);
        }
    }
    out
}

/// `1:05`, `0:03.2` for a short one.
#[must_use]
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0);
    if s < 10.0 {
        return format!("0:{s:04.1}");
    }
    let whole = s.round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

impl SoundDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, rel: String) -> Self {
        let source = Arc::new(Mutex::new(String::new()));
        let follow = follow_asset(editor, &rel, source.clone());
        let mut doc = Self {
            editor: editor.clone(),
            rel,
            loaded: None,
            inbox: Arc::new(Mutex::new(None)),
            reading: Arc::new(AtomicBool::new(false)),
            follow,
            source_watch: source,
            follow_seen: 0,
            busy: Arc::new(AtomicUsize::new(0)),
            status: Arc::new(Mutex::new(String::new())),
            pcm: None,
            pcm_in: Arc::new(Mutex::new(None)),
            decoding: false,
            free: None,
            free_in: Arc::new(Mutex::new(None)),
            free_asked: None,
            open_picker: false,
            close_picker: false,
            typing: FxHashMap::default(),
            drawn_frame: None,
        };
        doc.read();
        doc
    }

    #[must_use]
    pub fn rel(&self) -> &str {
        &self.rel
    }

    #[must_use]
    pub fn loaded(&self) -> Option<Arc<Loaded>> {
        self.loaded.clone()
    }

    /// Whether a read or a write is under way.
    #[must_use]
    pub fn settling(&self) -> bool {
        self.reading.load(Ordering::Acquire) || self.busy.load(Ordering::Acquire) > 0
    }

    #[must_use]
    pub fn drawn_lately(&self, frame: u64) -> bool {
        self.drawn_frame.is_some_and(|f| f + 1 >= frame)
    }

    /// The file read again, off the UI thread.
    fn read(&mut self) {
        if self.reading.swap(true, Ordering::AcqRel) {
            return;
        }
        let (ed, rel, inbox, flag, watch) = (
            self.editor.clone(),
            self.rel.clone(),
            self.inbox.clone(),
            self.reading.clone(),
            self.source_watch.clone(),
        );
        self.editor.exec.spawn(async move {
            let loaded = load(ed.clone(), rel).await;
            loaded.source_rel.replace('\\', "/").clone_into(&mut watch.lock());
            *inbox.lock() = Some(Arc::new(loaded));
            flag.store(false, Ordering::Release);
            ed.exec.wake().ping();
        });
    }

    /// What arrived, and a read when the file moved.
    fn refresh(&mut self) {
        if let Some(l) = self.inbox.lock().take() {
            // a sound made of another source is decoded again when it is played
            if self.loaded.as_ref().is_none_or(|o| {
                o.source != l.source || o.facts.as_ref().map(|f| f.bytes) != l.facts.as_ref().map(|f| f.bytes)
            }) {
                self.pcm = None;
            }
            self.loaded = Some(l);
        }
        if let Some(p) = self.pcm_in.lock().take() {
            self.decoding = false;
            match p {
                Ok(pcm) => {
                    Player::of(&self.editor).play(pcm.clone());
                    self.pcm = Some(pcm);
                }
                Err(e) => self.editor.notices.error(e),
            }
        }
        let seen = self.follow.load(Ordering::Acquire);
        if seen != self.follow_seen && !self.reading.load(Ordering::Acquire) {
            self.follow_seen = seen;
            self.read();
        }
    }

    /// Writes a change into the file, then reads it again; a refusal is the tab's status.
    fn change(&mut self, change: Change) {
        if change.is_empty() {
            return;
        }
        self.busy.fetch_add(1, Ordering::AcqRel);
        let (ed, rel, busy, status, follow) = (
            self.editor.clone(),
            self.rel.clone(),
            self.busy.clone(),
            self.status.clone(),
            self.follow.clone(),
        );
        self.editor.exec.spawn(async move {
            let done = write_change(&ed, &rel, change).await;
            *status.lock() = done.err().unwrap_or_default();
            busy.fetch_sub(1, Ordering::AcqRel);
            follow.fetch_add(1, Ordering::AcqRel);
            ed.exec.wake().ping();
        });
    }

    /// Plays the sound from its start: decoded once on the pools.
    pub fn play(&mut self) -> Result<(), String> {
        let player = Player::of(&self.editor);
        if let Some(pcm) = &self.pcm {
            player.play(pcm.clone());
            return Ok(());
        }
        let source =
            self.loaded.as_ref().and_then(|l| l.source.clone()).ok_or("The sound has no source to play.")?;
        if self.decoding {
            return Ok(());
        }
        self.decoding = true;
        let (ed, out) = (self.editor.clone(), self.pcm_in.clone());
        self.editor.exec.spawn(async move {
            let decoded = ed.exec.io(move || crate::cmd::decode_file(&source)).await;
            *out.lock() = Some(decoded.map(Arc::new));
            ed.exec.wake().ping();
        });
        Ok(())
    }

    pub fn stop(&self) {
        Player::of(&self.editor).stop();
    }

    /// Where the sound is while it plays, in seconds.
    #[must_use]
    pub fn position(&self) -> Option<f64> {
        let pcm = self.pcm.as_ref()?;
        Player::of(&self.editor).progress(pcm)
    }

    fn copyrighted(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.asset.is_copyrighted())
    }

    /// The sounds that may play instead: read off the UI thread once per state of the files,
    /// and only while the sound is copyrighted.
    fn free_rows(&mut self) -> Option<Arc<Vec<FreeSound>>> {
        if let Some(read) = self.free_in.lock().take() {
            self.free = Some(read);
        }
        let tree = gz_project::Project::of(&self.editor).map_or(0, |(_, p)| p.tree.rev().0);
        if self.free.as_ref().is_none_or(|(k, _)| *k != tree) && self.free_asked != Some(tree) {
            self.free_asked = Some(tree);
            let (out, ed, me) =
                (self.free_in.clone(), self.editor.clone(), self.rel.replace('\\', "/").to_lowercase());
            self.editor.exec.spawn(async move {
                let Ok(media) = gz_media::Media::of(&ed).await else { return };
                let index = media.fresh_index().await;
                let game = media.game_lists(&ed).await;
                let root = media.root().to_path_buf();
                let rows = ed
                    .exec
                    .io(move || {
                        let mut all = gz_media::sound::free_sounds(&root, &index, &game);
                        // a sound never plays instead of itself
                        all.retain(|f| f.asset.as_ref().is_none_or(|a| a.to_lowercase() != me));
                        all
                    })
                    .await;
                *out.lock() = Some((tree, Arc::new(rows)));
                ed.exec.wake().ping();
            });
        }
        self.free.as_ref().map(|(_, r)| r.clone())
    }

    /// `preview.sound`'s fields after `tab` and `active`.
    #[must_use]
    pub fn state_json(&self) -> Map<String, serde_json::Value> {
        let mut out = Map::new();
        out.insert("open".into(), json!(true));
        out.insert("path".into(), json!(self.rel.replace('\\', "/")));
        let Some(l) = &self.loaded else {
            out.insert("reading".into(), json!(true));
            return out;
        };
        out.insert("reading".into(), json!(self.settling()));
        out.insert("error".into(), json!(l.error));
        out.insert("status".into(), json!(self.status.lock().clone()));
        out.insert("name".into(), json!(l.name));
        out.insert("source".into(), json!(l.source_rel.replace('\\', "/")));
        out.insert("source_error".into(), json!(l.source_error));
        if let Some(f) = &l.facts {
            out.insert("format".into(), json!(f.format));
            out.insert("seconds".into(), json!((f.seconds * 1000.0).round() / 1000.0));
            out.insert("channels".into(), json!(f.channels));
            out.insert("rate".into(), json!(f.rate));
            out.insert("bytes".into(), json!(f.bytes));
        }
        out.insert("copyrighted".into(), json!(l.asset.is_copyrighted()));
        out.insert("streamer".into(), json!(l.asset.streamer.clone().unwrap_or_default()));
        let position = self.position();
        out.insert("playing".into(), json!(position.is_some()));
        out.insert("position".into(), json!(position.map(|p| (p * 1000.0).round() / 1000.0)));
        let problem = |p: &Problem| json!({ "code": p.code.as_str(), "severity": format!("{:?}", p.severity).to_lowercase(), "text": p.message, "line": p.line, "key": l.key_of(p) });
        let rows: Vec<serde_json::Value> = SOUND
            .iter()
            .filter(|f| !f.title.is_empty())
            .map(|f| {
                json!({
                    "key": f.key,
                    "title": f.title,
                    "value": l.get(f.key).map_or(serde_json::Value::Null, value_json),
                    "default": hint_of(f, l),
                    "set": l.get(f.key).is_some(),
                    "surface": f.surface,
                    "enabled": f.key != REPLACEMENT_KEY || l.asset.is_copyrighted(),
                    "problems": l.problems_of(f.key).map(problem).collect::<Vec<_>>(),
                })
            })
            .collect();
        out.insert("rows".into(), json!(rows));
        out.insert("problems".into(), json!(l.problems.iter().map(problem).collect::<Vec<_>>()));
        out
    }

    /// Plays Instead's rows as the answer names them: `null` while the sound is not copyrighted.
    #[must_use]
    pub fn picker_json(&self, limit: usize) -> serde_json::Value {
        if !self.copyrighted() {
            return serde_json::Value::Null;
        }
        let Some((_, rows)) = &self.free else { return json!({ "reading": true }) };
        let first: Vec<serde_json::Value> = rows
            .iter()
            .take(limit)
            .map(|r| json!({ "name": r.name, "from": if r.asset.is_some() { "sound_asset" } else { "game" }, "asset": r.asset }))
            .collect();
        json!({ "count": rows.len(), "first": sound_pick::NOTHING, "rows": first })
    }

    /// Whether Plays Instead's rows are read (or not wanted).
    #[must_use]
    pub fn picker_ready(&self) -> bool {
        !self.copyrighted() || self.free.is_some()
    }

    /// The tab's own edits, as a person makes them in it (`preview.sound set=`): a key set to a
    /// value or taken out (`null`), each the way its row writes it - Copyrighted off takes the
    /// replacement out with it, and Plays Instead takes only a row of its list, while the sound is
    /// copyrighted. A key the tab shows no row of, or a value its row does not take, is refused
    /// and nothing is written.
    pub fn apply_set(&mut self, set: &Map<String, serde_json::Value>) -> Result<(), String> {
        let Some(l) = self.loaded.clone() else {
            return Err("the sound asset is still being read".to_owned());
        };
        if !l.error.is_empty() {
            return Err(l.error.clone());
        }
        let mut change: Change = Vec::new();
        let mut copyrighted = l.asset.is_copyrighted();
        for (key, v) in set {
            let Some(f) = SOUND.iter().find(|f| f.key == key && !f.title.is_empty()) else {
                let keys: Vec<&str> = SOUND.iter().filter(|f| !f.title.is_empty()).map(|f| f.key).collect();
                return Err(format!("the Sound tab has no row '{key}': its keys are {}", keys.join(", ")));
            };
            let value = match v {
                serde_json::Value::Null => None,
                serde_json::Value::Bool(b) if f.ty == Ty::Bool => Some(Value::from(*b)),
                serde_json::Value::Number(n) if matches!(f.ty, Ty::Number { .. } | Ty::Integer { .. }) => {
                    typed_value(f, &n.to_string()).map_err(|()| format!("{key} takes a number"))?
                }
                serde_json::Value::String(s) => match f.ty {
                    Ty::Words(list) | Ty::WordSet(list) if !words(list).contains(&s.as_str()) => {
                        return Err(format!(
                            "{key} takes one of the game's words: {}",
                            words(list).join(", ")
                        ));
                    }
                    Ty::Bool | Ty::Number { .. } | Ty::Integer { .. } => {
                        return Err(format!("{key} takes {}", f.ty.describe()));
                    }
                    _ => typed_value(f, s).map_err(|()| format!("{key} takes {}", f.ty.describe()))?,
                },
                _ => return Err(format!("{key} takes {}", f.ty.describe())),
            };
            if f.key == MARK_KEY {
                copyrighted = value.as_ref().and_then(Value::as_bool) == Some(true);
                // the switch writes the mark or takes it out, never `false`
                change.push((MARK_KEY, copyrighted.then(|| Value::from(true))));
                if !copyrighted {
                    change.push((REPLACEMENT_KEY, None));
                }
                continue;
            }
            change.push((f.key, value));
        }
        if let Some((_, Some(v))) = change.iter().find(|(k, v)| *k == REPLACEMENT_KEY && v.is_some()) {
            let name = v.as_str().unwrap_or_default();
            if !copyrighted {
                return Err(
                    "Plays Instead is live only while the sound is copyrighted: set copyrighted first"
                        .to_owned(),
                );
            }
            let rows = self.free.as_ref().map(|(_, r)| r.clone()).unwrap_or_default();
            if !rows.iter().any(|r| r.name.eq_ignore_ascii_case(name)) {
                return Err(format!(
                    "'{name}' is no row of Plays Instead: it offers the module's sound assets without the mark and the game's sounds its own list does not name"
                ));
            }
        }
        self.change(change);
        Ok(())
    }

    fn toolbar_items(&mut self, tb: &mut Toolbar<'_, '_>) {
        let can_play = self.loaded.as_ref().is_some_and(|l| l.source.is_some());
        let playing = self.position().is_some();
        tb.group(2);
        if tb.action(
            "play",
            Icon::Play,
            Tx::ctx("playback", "Play"),
            if can_play { "Play the sound from its start" } else { "The sound has no source to play" },
            can_play,
            false,
            Kind::Ghost,
        ) && let Err(e) = self.play()
        {
            self.editor.notices.error(e);
        }
        if tb.action(
            "stop",
            Icon::Stop,
            Tx::ctx("playback", "Stop"),
            "Stop the sound",
            playing,
            false,
            Kind::Ghost,
        ) {
            self.stop();
        }
        tb.separator();
        tb.group(1);
        let rel = self.rel.clone();
        if tb.action(
            "reveal",
            Icon::Reveal,
            "Show in Content Browser",
            "Select it in the Content Browser",
            true,
            false,
            Kind::Ghost,
        ) {
            self.editor.post(
                "content.view",
                gz_app::args(&[("action", "reveal"), ("source", "project"), ("path", &rel)]),
            );
        }
        tb.group(0);
        tb.separator();
        let status = self.status.lock().clone();
        if status.is_empty() {
            tb.status("Saved to the file as you edit", Tone::Muted);
        } else {
            tb.status(Tx::ui(status), Tone::Error);
        }
    }

    /// The sound itself: its name, its file and format, Play / Stop with where it is.
    fn head(&mut self, ui: &mut Ui, l: &Loaded) {
        let ctx = ui.ctx().clone();
        let facts = l.facts.as_ref().map(|f| {
            let channels = match f.channels {
                1 => Tx::from("Mono"),
                2 => Tx::from("Stereo"),
                n => tr!("{n} channels", n = n),
            };
            local_facts(
                &ctx,
                &[
                    Tx::raw(f.format),
                    Tx::raw(clock(f.seconds)),
                    tr!("{khz} kHz", khz = format_number(f64::from(f.rate) / 1000.0, 1)),
                    channels,
                    Tx::raw(crate::texture_load::human_bytes(f.bytes)),
                ],
            )
        });
        let file = Tx::raw(l.source_rel.clone());
        let line = match &facts {
            Some(f) => f.clone(),
            None if !l.source_error.is_empty() => Tx::ui(l.source_error.clone()),
            None => Tx::EMPTY,
        };
        local_heading(ui, Tx::raw(l.name.clone()), Clip::Start, &[(file, Clip::Start), (line, Clip::End)]);
        ui.add_space(6.0);
        let seconds = l.facts.as_ref().map_or(0.0, |f| f.seconds);
        let position = self.position();
        let mut play = false;
        let mut stop = false;
        ui.horizontal(|ui| {
            let on = l.source.is_some();
            play = gz_ui::Button::new(Tx::ctx("playback", "Play"))
                .icon(Icon::Play)
                .enabled(on, "The sound has no source to play.")
                .show(ui)
                .clicked();
            stop = gz_ui::Button::new(Tx::ctx("playback", "Stop"))
                .icon(Icon::Stop)
                .enabled(position.is_some(), "Nothing is playing.")
                .show(ui)
                .clicked();
            let p = palette(ui.ctx());
            let at = position.unwrap_or(0.0).min(seconds);
            text::label(
                ui,
                Tx::raw(format!("{} / {}", clock(at), clock(seconds))),
                Font::Mono,
                p.text_secondary,
                Clip::End,
            );
            // how far it is: a bar the rest of the row wide
            let w = ui.available_width().max(40.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, metrics_frame()), Sense::hover());
            let bar = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), BAR_H));
            ui.painter().rect_filled(bar, CornerRadius::same(2), p.inset);
            if seconds > 0.0 {
                let done = (at / seconds).clamp(0.0, 1.0) as f32;
                let filled = Rect::from_min_size(bar.min, Vec2::new(bar.width() * done, BAR_H));
                ui.painter().rect_filled(filled, CornerRadius::same(2), p.accent);
            }
        });
        if play && let Err(e) = self.play() {
            self.editor.notices.error(e);
        }
        if stop {
            self.stop();
        }
        if position.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// What the asset says: the surface's rows, then More.
    fn keys(&mut self, ui: &mut Ui, l: &Loaded) -> Change {
        let mut out: Change = Vec::new();
        let rows = self.free_rows_if(l);
        let open = std::mem::take(&mut self.open_picker);
        if std::mem::take(&mut self.close_picker) && !open {
            sound_pick::close(ui.ctx(), REPLACEMENT_KEY);
        }
        let typing = &mut self.typing;
        let mut play: Option<FreeSound> = None;
        PropertyGrid::new("sound").label_frac(0.34).show(ui, |g| {
            for f in SOUND.iter().filter(|f| f.surface && !f.title.is_empty()) {
                row(g, f, l, typing, &mut out, rows.as_ref().map(|r| r.as_slice()), open, &mut play);
            }
            g.section("More", false, None, |g| {
                for f in SOUND.iter().filter(|f| !f.surface && !f.title.is_empty()) {
                    row(g, f, l, typing, &mut out, rows.as_ref().map(|r| r.as_slice()), false, &mut play);
                }
            });
        });
        if let Some(r) = play {
            play_free(&self.editor, &r);
        }
        out
    }

    fn free_rows_if(&mut self, l: &Loaded) -> Option<Arc<Vec<FreeSound>>> {
        if l.asset.is_copyrighted() { self.free_rows() } else { None }
    }
}

/// A frame's height, the kit's.
fn metrics_frame() -> f32 {
    gz_ui::metrics::FRAME
}

/// What a key left out is: its default as the table says it, the value of the key it names, the
/// file the asset would read, the name it would publish.
#[must_use]
pub fn hint_of(f: &Field, l: &Loaded) -> String {
    let clean = |t: &str| t.replace(['`', '"'], "");
    match f.ty {
        Ty::File => {
            let leaf = l.source_rel.rsplit(['\\', '/']).next().unwrap_or("").to_owned();
            if leaf.is_empty() { clean(f.default) } else { leaf }
        }
        Ty::GameName if f.key != REPLACEMENT_KEY => l.name.clone(),
        _ => {
            let d = clean(f.default);
            // a default that is another key of the table: that key's value, or its own default
            match SOUND.iter().find(|o| o.key == d) {
                Some(o) => l.get(o.key).map_or_else(|| clean(o.default), shown),
                None => d,
            }
        }
    }
}

/// The value written for a typed text, or `None` to take the key out; `Err` for a text the key
/// does not take.
fn typed_value(f: &Field, text_: &str) -> Result<Option<Value>, ()> {
    let t = text_.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match f.ty {
        Ty::Number { min, max } => parse_number(t).map(|v| Some(Value::from(v.clamp(min, max)))).ok_or(()),
        Ty::Integer { min, max } => {
            parse_number(t).map(|v| Some(Value::from((v.round() as i64).clamp(min, max) as f64))).ok_or(())
        }
        Ty::GameName => Ok(Some(Value::from(t.replace('/', "\\")))),
        _ => Ok(Some(Value::from(t))),
    }
}

// one row of the grid: every argument is what the grid draws from or writes into
#[allow(clippy::too_many_arguments)]
fn row(
    g: &mut Grid<'_>,
    f: &'static Field,
    l: &Loaded,
    typing: &mut FxHashMap<&'static str, String>,
    out: &mut Change,
    rows: Option<&[FreeSound]>,
    open: bool,
    play: &mut Option<FreeSound>,
) {
    let set = l.get(f.key).is_some();
    let problems: Vec<&Problem> = l.problems_of(f.key).collect();
    let error = problems.iter().any(|p| p.severity == Severity::Error);
    let mut flags = if set { PropFlags::MODIFIED } else { PropFlags::NONE };
    if error {
        flags = flags | PropFlags::INVALID;
    }
    let copyrighted = l.asset.is_copyrighted();
    let live = f.key != REPLACEMENT_KEY || copyrighted;
    if f.ty == Ty::Bool {
        flags = flags | PropFlags::INLINE;
    }
    let title = Tx::ui(f.title);
    let tip = Tx::ui(f.shown_tip());
    let hint = hint_of(f, l);
    g.row(title, tip, flags, 0, |ui| {
        ui.add_enabled_ui(live, |ui| {
            if f.key == REPLACEMENT_KEY {
                let current = l.asset.streamer.clone().unwrap_or_default();
                let Some(rows) = rows.filter(|_| copyrighted) else {
                    if copyrighted {
                        state::busy_line(ui, "Reading the sounds...");
                    } else {
                        read_only(ui, sound_pick::NOTHING, Clip::End);
                    }
                    return;
                };
                let w = ui.available_width();
                ui.horizontal(|ui| {
                    ui.set_width(w);
                    let field_w = (w - gz_ui::metrics::ICON_BUTTON - 6.0).max(80.0);
                    let asked = ui
                        .scope(|ui| {
                            ui.set_width(field_w);
                            sound_pick::picker(ui, f.key, &current, rows, open)
                        })
                        .inner;
                    if let Some(r) = asked.play.and_then(|i| rows.get(i)) {
                        *play = Some(r.clone());
                    }
                    if let Some(r) = asked.picked.and_then(|i| rows.get(i)) {
                        out.push((REPLACEMENT_KEY, Some(Value::from(r.name.as_str()))));
                    }
                    if asked.clear {
                        out.push((REPLACEMENT_KEY, None));
                    }
                    let chosen = rows.iter().find(|r| r.name.eq_ignore_ascii_case(&current));
                    if gz_ui::icon_button(ui, Icon::Play, "Play what plays instead", false, chosen.is_some())
                        .clicked()
                        && let Some(r) = chosen
                    {
                        *play = Some(r.clone());
                    }
                });
                return;
            }
            match f.ty {
                Ty::Bool => {
                    let mut on = l.get(f.key).and_then(Value::as_bool).unwrap_or(f.default == "true");
                    if switch(ui, &mut on).changed() {
                        if f.key == MARK_KEY {
                            // off takes the replacement out with the mark: it has nothing to replace
                            out.push((MARK_KEY, on.then(|| Value::from(true))));
                            if !on {
                                out.push((REPLACEMENT_KEY, None));
                            }
                        } else {
                            out.push((f.key, Some(Value::from(on))));
                        }
                    }
                }
                Ty::Words(list) | Ty::WordSet(list) => {
                    let all = words(list);
                    let items: Vec<PickItem> = all.iter().map(|w| PickItem::new(*w, Tx::raw(*w))).collect();
                    let now = l.get(f.key).map(shown).unwrap_or_default();
                    let shown_tx = if now.is_empty() { Tx::raw(hint.clone()) } else { Tx::raw(now.clone()) };
                    let picked = Combo::new(("gz-ui-previews.sound_words", f.key), shown_tx)
                        .placeholder(now.is_empty())
                        .show(
                            ui,
                            PickList::new(("gz-ui-previews.sound_words_list", f.key), &items).current(&now),
                        );
                    if let Some(w) = picked.and_then(|i| all.get(i)) {
                        out.push((f.key, Some(Value::from(*w))));
                    }
                }
                _ => {
                    let id = egui::Id::unique(("gz-ui-previews.sound_field", f.key));
                    let now = l.get(f.key).map(shown).unwrap_or_default();
                    let focused = ui.ctx().memory(|m| m.has_focus(id));
                    let mut text_ =
                        if focused { typing.get(f.key).cloned().unwrap_or(now.clone()) } else { now.clone() };
                    let unit_w = if f.unit.is_empty() { 0.0 } else { 22.0 };
                    // a number is short: its field is too, its unit right after it
                    let room = (ui.available_width() - unit_w).max(40.0);
                    let width = if matches!(f.ty, Ty::Number { .. } | Ty::Integer { .. }) {
                        room.min(NUMBER_W)
                    } else {
                        room
                    };
                    let resp = gz_ui::form::hint_field(ui, id, &mut text_, Tx::raw(hint.clone()), width);
                    if !f.unit.is_empty() {
                        let p = palette(ui.ctx());
                        text::label(ui, Tx::ui(f.unit), Font::Body, p.text_muted, Clip::End);
                    }
                    if resp.has_focus() {
                        typing.insert(f.key, text_.clone());
                    }
                    if resp.lost_focus() {
                        typing.remove(f.key);
                        if text_.trim() != now
                            && let Ok(v) = typed_value(f, &text_)
                            && (v.is_some() || set)
                        {
                            out.push((f.key, v));
                        }
                    }
                }
            }
        });
    });
    if g.was_reset() {
        out.push((f.key, None));
        if f.key == MARK_KEY {
            out.push((REPLACEMENT_KEY, None));
        }
    }
    for p in problems {
        let tone = match p.severity {
            Severity::Error => Tone::Error,
            Severity::Warning | Severity::Required => Tone::Warning,
            Severity::Note => Tone::Muted,
        };
        g.message(tone, Tx::ui(p.message.clone()));
    }
}

/// Plays a row of Plays Instead: a sound asset of the module, or the game's sound by its name.
fn play_free(editor: &Arc<Editor>, r: &FreeSound) {
    let args = match &r.asset {
        Some(a) => gz_app::args(&[("action", "play"), ("path", a.as_str())]),
        None => gz_app::args(&[("action", "play"), ("name", r.name.as_str())]),
    };
    editor.post("preview.sound", args);
}

/// Writes a change into the file as it is on the disk now: the difference of the model, so the
/// file's own text stays.
async fn write_change(editor: &Arc<Editor>, rel: &str, change: Change) -> Result<(), String> {
    let path = RelPath::new(&rel.replace('\\', "/")).map_err(|e| e.to_string())?;
    let mut file = gz_asset::read(editor, &path).await.map_err(|f| f.problem.message.clone())?;
    let old = file.doc.value();
    let mut new = old.clone();
    let Value::Table(t) = &mut new else { return Err(format!("{rel} holds no table")) };
    for (key, value) in change {
        match value {
            Some(v) => match t.get_mut(key) {
                Some(slot) => *slot = v,
                None => t.put(key, v),
            },
            None => t.fields.retain(|(k, _)| k.as_str() != key),
        }
    }
    gz_asset::save(editor, &mut file, &old, &new).await.map(|_| ()).map_err(|e| e.message)
}

impl Document for SoundDoc {
    fn id(&self) -> &str {
        "sound"
    }
    fn kind(&self) -> &'static str {
        "sound"
    }
    fn label(&self) -> Tx<'static> {
        Tx::raw(crate::paths::label_of(&self.rel))
    }
    fn icon(&self) -> Icon {
        Icon::SoundAsset
    }
    fn path(&self) -> Option<String> {
        Some(self.rel.clone())
    }
    fn tooltip(&self) -> Option<Tx<'static>> {
        Some(Tx::raw(self.rel.clone()))
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, _cx: &mut DocCx<'_>) {
        self.toolbar_items(tb);
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        self.drawn_frame = Some(ui.ctx().cumulative_frame_nr());
        self.refresh();
        let full = ui.max_rect();
        let Some(l) = self.loaded.clone() else {
            state::busy_line(ui, tr!("Reading {name}...", name = crate::paths::leaf(&self.rel)));
            return;
        };
        if !l.error.is_empty() {
            let mut child =
                ui.new_child(UiBuilder::new().max_rect(full).layout(Layout::top_down(Align::Center)));
            state::error_state(&mut child, "This sound asset cannot be read", Tx::ui(l.error.clone()), None);
            ui.allocate_rect(full, Sense::hover());
            return;
        }
        let mut change: Change = Vec::new();
        ScrollArea::vertical().id_salt("gz-ui-previews.sound_scroll").auto_shrink([false, false]).show(
            ui,
            |ui| {
                let avail = ui.available_width();
                let column = avail.min(COLUMN_W);
                let x0 = ui.cursor().min.x + ((avail - column) * 0.5).max(0.0);
                let col = Rect::from_min_size(pos2(x0, ui.cursor().min.y), Vec2::new(column, f32::INFINITY));
                ui.scope_builder(UiBuilder::new().max_rect(col).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.set_width(column);
                    self.head(ui, &l);
                    ui.add_space(10.0);
                    // what reading the file found that is about no key of it
                    for p in l.problems.iter().filter(|p| l.key_of(p).is_none()) {
                        let tone = match p.severity {
                            Severity::Error => Tone::Error,
                            Severity::Note => Tone::Muted,
                            _ => Tone::Warning,
                        };
                        text::status(ui, tone, Tx::ui(p.message.clone()));
                    }
                    change = self.keys(ui, &l);
                });
            },
        );
        self.change(change);
    }

    fn restore_arg(&self) -> Option<String> {
        Some(self.rel.clone())
    }

    fn mcp_state(&self) -> serde_json::Value {
        serde_json::Value::Object(self.state_json())
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        if self.position().is_some() {
            self.stop();
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `sound` factory: `path` names the sound asset, project-relative.
pub fn factory(editor: &Arc<Editor>, req: &OpenRequest) -> Result<Box<dyn Document>, String> {
    let rel = req.path.as_deref().unwrap_or("").replace('/', "\\");
    if rel.is_empty() || MediaKind::of_file(&rel) != Some(MediaKind::Sound) {
        return Err("open takes path= naming a sound asset of the project (.sndasset)".to_owned());
    }
    Ok(Box::new(SoundDoc::new(editor, rel)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_left_out_key_shows_its_default_and_a_key_it_names() {
        let mut l =
            Loaded { name: "od\\rain".into(), source_rel: "sounds\\rain.ogg".into(), ..Loaded::default() };
        let key = |k: &str| SOUND.iter().find(|f| f.key == k).unwrap();
        assert_eq!(hint_of(key("max_distance"), &l), "300");
        assert_eq!(hint_of(key("ai_distance"), &l), "300");
        assert_eq!(hint_of(key("name"), &l), "od\\rain");
        assert_eq!(hint_of(key("source"), &l), "rain.ogg");
        assert_eq!(hint_of(key("type"), &l), "no_sound");
        l.value = gz_luadata::parse("return { snd = 1, max_distance = 50 }").unwrap();
        assert_eq!(hint_of(key("ai_distance"), &l), "50");
        assert_eq!(typed_value(key("volume"), "20").unwrap(), Some(Value::from(10.0)));
        assert_eq!(typed_value(key("volume"), " ").unwrap(), None);
        assert!(typed_value(key("volume"), "loud").is_err());
        assert_eq!((clock(3.31), clock(65.0)), ("0:03.3".to_owned(), "1:05".to_owned()));
        assert!(touches("sounds\\rain.sndasset", Some("sounds/rain.ogg"), "Sounds/Rain.ogg"));
        assert!(!touches("sounds\\rain.sndasset", None, "sounds/other.ogg"));
        // every row the window shows has its title, and the two of the mark are on the surface
        assert!(
            SOUND.iter().filter(|f| f.ty != Ty::Header).all(|f| !f.title.is_empty()),
            "a key without a title"
        );
        assert!(SOUND.iter().any(|f| f.key == MARK_KEY && f.surface));
        assert!(SOUND.iter().any(|f| f.key == REPLACEMENT_KEY && f.surface));
    }
}
