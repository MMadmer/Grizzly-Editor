//! The screens' commands, the build's check and the slot for the linked game's levels:
//! `screens.info` (the screens and music as data, with their problems), `screens.set` (one change
//! of them, checked) and `screens.import` (files brought from anywhere, put where they go).
//! `screens.view` is the tab's. An agent may equally edit `mod.ltx` itself.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Origin, Problem};
use gz_build::{BuildCheck, BuildChecks, BuildWalk};
use gz_editor::{Args, CmdError, Editor, Executors, NoParams, ProjectSession, Registry, Reply, Spec};
use gz_mod::ManifestService;
use gz_project::Project;
use parking_lot::RwLock;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::import::{FileKind, file_kind, import, stem};
use crate::model::{
    Fit, HIDE_BAR, HIDE_PERCENT, HIDE_STAGE, HIDE_TIP, LOADING, Layer, MENU, PAUSE, Screen, Screens, Track,
    atof, sanitize, scan_floats, split,
};
use crate::validate::validate;

pub fn register(reg: &mut Registry) {
    reg.area("screens", "The module's loading and menu screens and their music ([screens] of mod.ltx)");
    reg.cmd(
        Spec::read(
            "screens.info",
            "The module's screens, layers and music tracks as data, with their problems.",
        ),
        screens_info,
    );
    reg.cmd(
        Spec::write(
            "screens.set",
            "One change of the screens or the music: a screen, a layer, a track, the policy.",
        )
        .example("action=layer_set id=night index=0 opacity=0.5"),
        screens_set,
    );
    reg.cmd(
        Spec::write(
            "screens.import",
            "Brings files into the screens: a picture or a video becomes a screen or a layer, an .ogg a track.",
        )
        .example("path=D:/Art/night.mp4 screen=night"),
        screens_import,
    );
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SetAction {
    /// What a surface's screens and music do with the game's own: `surface`, `screens` (only or
    /// mix), `music` (only or mix), `end` (loop or next).
    Policy,
    /// A new screen `id` (made unique) with the screen's fields, and a first layer with `file`.
    ScreenAdd,
    /// Changes the screen `id`: show, levels, modes, weight, music, hide, new_id.
    ScreenSet,
    /// Removes the screen `id`.
    ScreenRemove,
    /// A new layer of the screen `id` at `index` (left out: on top) with the layer's fields.
    LayerAdd,
    /// Changes the layer `index` of the screen `id`: file, texture, fit, opacity, rect, sound.
    LayerSet,
    /// Removes the layer `index` of the screen `id`.
    LayerRemove,
    /// Moves the layer `index` of the screen `id` to `to`.
    LayerMove,
    /// A new track `id` with `file`, play, weight, volume.
    TrackAdd,
    /// Changes the track `id`.
    TrackSet,
    /// Removes the track `id`.
    TrackRemove,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetParams {
    action: SetAction,
    /// The screen or the track; for screen_add and track_add the wanted id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// Policy: loading or menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface: Option<String>,
    /// Policy: only (the module's screens alone) or mix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    screens: Option<String>,
    /// Policy: only or mix. A screen: any, none, video, or the tracks it plays (comma separated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    music: Option<String>,
    /// Policy: loop or next (what the music does at the end of a track).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<String>,
    /// A screen: the surfaces it shows on (loading, menu, pause).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show: Option<String>,
    /// A screen: the levels it shows for; empty: every level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    levels: Option<Vec<String>>,
    /// A screen: the game modes it shows in; empty: every mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modes: Option<Vec<String>>,
    /// A screen or a track: how often it is picked beside the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight: Option<f32>,
    /// A screen: what of the game's loading screen it hides (bar, percent, stage, tip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hide: Option<String>,
    /// A screen: its new id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new_id: Option<String>,
    /// A layer or a track: its file, from anywhere (brought into screens\ when it is outside the
    /// project); empty clears a layer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// A shader layer: its texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    texture: Option<String>,
    /// A layer: cover, contain, stretch or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fit: Option<String>,
    /// A layer: 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opacity: Option<f32>,
    /// A layer: its rectangle x, y, w, h in shares of the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rect: Option<[f32; 4]>,
    /// A video layer: whether its sound plays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sound: Option<bool>,
    /// A layer: its index, 0 the bottom one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<i64>,
    /// layer_move: where the layer goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<i64>,
    /// A track: the surfaces it plays on (loading, menu, pause).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    play: Option<String>,
    /// A track: 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    volume: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportParams {
    /// The files, from anywhere: absolute or project-relative.
    path: Vec<String>,
    /// The screen they land on (each a layer of it); left out: each becomes a new screen.
    #[serde(default)]
    screen: Option<String>,
    /// On that screen: the layer whose file they replace; left out: new layers on top.
    #[serde(default)]
    layer: Option<i64>,
    /// Into the music: only .ogg files are taken.
    #[serde(default)]
    music: bool,
    /// What a new screen or track shows on: loading or menu (left out: what the open tab shows).
    #[serde(default)]
    surface: Option<String>,
}

/// The linked game's levels, as the world area reads them.
pub type LevelsHook = Arc<dyn Fn(Arc<Editor>) -> BoxFuture<'static, Option<Vec<String>>> + Send + Sync>;

/// Where the world area hangs what W407 asks: the levels of the linked game. Until it does, or
/// while it answers `None`, levels are not checked.
/// `EditorWorld::Available()` was false. Lives in `editor.services`.
#[derive(Default)]
pub struct GameLevels {
    hook: RwLock<Option<LevelsHook>>,
}

impl GameLevels {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    pub fn provide(
        &self,
        f: impl Fn(Arc<Editor>) -> BoxFuture<'static, Option<Vec<String>>> + Send + Sync + 'static,
    ) {
        *self.hook.write() = Some(Arc::new(f));
    }

    pub async fn levels(&self, editor: &Arc<Editor>) -> Option<Vec<String>> {
        let hook = self.hook.read().clone()?;
        hook(editor.clone()).await
    }
}

/// The problems of `s` in the open project, on the IO pool.
pub async fn problems(
    editor: &Arc<Editor>,
    root: PathBuf,
    manifest: &Arc<ManifestService>,
    s: Screens,
) -> Vec<String> {
    let levels = GameLevels::of(editor).levels(editor).await;
    let manifest = manifest.clone();
    editor.exec.io(move || validate(&root, &s, &|rel| manifest.outside_build(rel), levels.as_deref())).await
}

/// A coded line as a problem: its code, the rest its message.
fn as_problem(line: &str) -> Problem {
    let (code, message) = line.split_once(' ').unwrap_or((line, ""));
    if code.starts_with('E') { Problem::error(code, message) } else { Problem::warning(code, message) }
}

struct ScreensCheck;

impl BuildCheck for ScreensCheck {
    fn stage(&self) -> &'static str {
        "Checking the screens and music"
    }

    fn headline(&self) -> Option<&'static str> {
        Some("The screens and music have errors:")
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        session: Arc<ProjectSession>,
        _walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            // screens and music name files of the project: one the module would not hold is a
            // screen the game cannot show
            let Ok(manifest) = ManifestService::of_session(&editor, &session).await else {
                return Vec::new();
            };
            let _ = manifest.sync().await;
            let snap = manifest.snapshot();
            let Some(doc) = snap.doc() else { return Vec::new() };
            let s = Screens::parse(doc);
            problems(&editor, session.root.clone(), &manifest, s)
                .await
                .iter()
                .map(|l| as_problem(l))
                .collect()
        })
    }
}

/// Hangs the area on the editor: the build's check.
pub fn install(editor: &Editor) {
    BuildChecks::of(editor).add(Arc::new(ScreensCheck));
}

// ---- the answer -----------------------------------------------------------------------------

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A number as a JSON number: a whole one as an integer, any other `%.15g`.
fn json_number(v: f32) -> Value {
    let n = f64::from(v);
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        return json!(n as i64);
    }
    gz_luadata::format_g(n, 15).parse::<f64>().map_or(json!(n), |p| json!(p))
}

/// `loading,menu,pause` without spaces, as the answer names a mask.
fn mask_text(mask: u32, names: &[(u32, &str)]) -> String {
    names.iter().filter(|(b, _)| mask & b != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(",")
}

fn surfaces_json(mask: u32) -> String {
    mask_text(mask, &[(LOADING, "loading"), (MENU, "menu"), (PAUSE, "pause")])
}

fn layer_json(l: &Layer) -> Value {
    let kind = if l.file.is_empty() { FileKind::None } else { file_kind(&l.file) };
    let mut o = Map::new();
    o.insert("file".into(), json!(l.file));
    o.insert("kind".into(), json!(kind.label()));
    o.insert("fit".into(), json!(l.fit.name()));
    o.insert("opacity".into(), json_number(l.opacity));
    o.insert(
        "rect".into(),
        Value::Array(vec![json_number(l.x), json_number(l.y), json_number(l.w), json_number(l.h)]),
    );
    o.insert("texture".into(), json!(l.texture));
    o.insert("sound".into(), json!(l.sound));
    Value::Object(o)
}

/// The answer of both commands (`Answer`).
#[must_use]
pub fn answer(s: &Screens, problems: &[String]) -> Value {
    let side = |i: usize| {
        let pick = |v: &[bool; 2], yes: &str, no: &str| {
            if v.get(i).copied().unwrap_or(false) { yes.to_owned() } else { no.to_owned() }
        };
        json!({ "screens": pick(&s.only_screens, "only", "mix"), "music": pick(&s.only_music, "only", "mix"), "end": pick(&s.next_track, "next", "loop") })
    };
    let screens: Vec<Value> = s
        .screens
        .iter()
        .map(|d| {
            let mut o = Map::new();
            o.insert("id".into(), json!(d.id));
            o.insert("show".into(), json!(surfaces_json(d.show)));
            o.insert("levels".into(), json!(d.levels));
            o.insert("modes".into(), json!(d.modes));
            o.insert("weight".into(), json_number(d.weight));
            o.insert("music".into(), json!(d.music));
            o.insert("tracks".into(), json!(d.tracks));
            o.insert(
                "hide".into(),
                json!(mask_text(
                    d.hide,
                    &[(HIDE_BAR, "bar"), (HIDE_PERCENT, "percent"), (HIDE_STAGE, "stage"), (HIDE_TIP, "tip")]
                )),
            );
            o.insert("layers".into(), Value::Array(d.layers.iter().map(layer_json).collect()));
            Value::Object(o)
        })
        .collect();
    let tracks: Vec<Value> = s
        .tracks
        .iter()
        .map(|t| {
            let mut o = Map::new();
            o.insert("id".into(), json!(t.id));
            o.insert("file".into(), json!(t.file));
            o.insert("play".into(), json!(surfaces_json(t.play)));
            o.insert("weight".into(), json_number(t.weight));
            o.insert("volume".into(), json_number(t.volume));
            Value::Object(o)
        })
        .collect();
    let mut out = Map::new();
    out.insert("policy".into(), json!({ "loading": side(0), "menu": side(1) }));
    out.insert("screens".into(), Value::Array(screens));
    out.insert("tracks".into(), Value::Array(tracks));
    out.insert("problems".into(), json!(problems));
    Value::Object(out)
}

// ---- arguments the way GrizzlyMCP read them -------------------------------------------------

/// Present means given, even empty; a bare number or flag is its text.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(a) => Some(a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(",")),
        _ => None,
    }
}

/// A number, or text that starts like one.
fn arg_float(args: &Args, key: &str) -> Option<f32> {
    match args.get(key)? {
        Value::Number(n) => n.as_f64().map(|v| v as f32),
        Value::String(s) => {
            let t = s.trim_start_matches([' ', '\t']);
            t.starts_with(|c: char| c == '-' || c == '.' || c.is_ascii_digit()).then(|| atof(t) as f32)
        }
        _ => None,
    }
}

/// A number (cut toward zero) or text that starts like one; `def` otherwise.
fn arg_int(args: &Args, key: &str, def: i64) -> i64 {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|v| v as i64)).unwrap_or(def),
        Some(Value::String(s)) => {
            let t = s.trim_start_matches([' ', '\t']);
            if t.starts_with(|c: char| c == '-' || c.is_ascii_digit()) { atof(t) as i64 } else { def }
        }
        _ => def,
    }
}

/// `true` / `1` / `false` / `0` at the start of the value; `def` otherwise.
fn arg_bool(args: &Args, key: &str) -> Option<bool> {
    match args.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.to_string().chars().next() {
            Some('1') => Some(true),
            Some('0') => Some(false),
            _ => None,
        },
        Value::String(s) => {
            let t = s.trim_start_matches(' ');
            if t.starts_with("true") || t.starts_with('1') {
                Some(true)
            } else if t.starts_with("false") || t.starts_with('0') {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Every surface named anywhere in the text.
fn parse_surfaces(text: &str) -> u32 {
    [("loading", LOADING), ("menu", MENU), ("pause", PAUSE)]
        .into_iter()
        .filter(|(n, _)| text.contains(n))
        .fold(0, |m, (_, b)| m | b)
}

/// What a screen or a track made by a drop shows on: the surface the Screens
/// tab shows - the loading screen, or the menu and the pause screen.
fn default_show(menu: bool) -> u32 {
    if menu { MENU | PAUSE } else { LOADING }
}

/// The surface a drop lands on: `surface=` when given, else the one the open Screens tab shows
/// (`s_Surface`), else the loading screen.
async fn drop_surface(ed: &Arc<Editor>, args: &Args) -> bool {
    if let Some(v) = arg(args, "surface") {
        return v == "menu";
    }
    if !ed.registry().has("screens.view") {
        return false;
    }
    let view = ed.call("screens.view", Args::new(), Origin::Mcp).await;
    view.get("open").and_then(Value::as_bool) == Some(true)
        && view.get("surface").and_then(Value::as_str) == Some("menu")
}

/// Everything an action needs while it runs on the IO pool.
struct Cx<'a> {
    root: &'a str,
    /// What a screen or a track made by a drop shows on.
    show: u32,
    args: &'a Args,
    exec: &'a Executors,
    err: String,
}

impl Cx<'_> {
    fn import(&mut self, given: &str) -> Option<String> {
        match import(self.root, given) {
            Ok(rel) => Some(rel),
            Err(e) => {
                self.err = e;
                None
            }
        }
    }

    fn apply_layer(&mut self, l: &mut Layer) {
        if let Some(v) = arg(self.args, "file") {
            if v.is_empty() {
                l.file.clear();
            } else if let Some(rel) = self.import(&v) {
                l.file = rel;
            }
        }
        if let Some(v) = arg(self.args, "texture") {
            if v.is_empty() {
                l.texture.clear();
            } else if let Some(rel) = self.import(&v) {
                l.texture = rel;
            }
        }
        if let Some(v) = arg(self.args, "fit") {
            l.fit = Fit::parse(&v);
        }
        if let Some(f) = arg_float(self.args, "opacity") {
            l.opacity = f.clamp(0.0, 1.0);
        }
        if let Some(v) = arg(self.args, "rect") {
            for (slot, n) in [&mut l.x, &mut l.y, &mut l.w, &mut l.h].into_iter().zip(scan_floats(&v, 4)) {
                *slot = n;
            }
        }
        if let Some(b) = arg_bool(self.args, "sound") {
            l.sound = b;
        }
    }

    fn apply_screen(&mut self, d: &mut Screen, others: &Screens) {
        if let Some(v) = arg(self.args, "show") {
            d.show = parse_surfaces(&v);
        }
        if let Some(v) = arg(self.args, "levels") {
            d.levels = split(&v, ',');
        }
        if let Some(v) = arg(self.args, "modes") {
            d.modes = split(&v, ',');
        }
        if let Some(f) = arg_float(self.args, "weight") {
            d.weight = f.max(0.0);
        }
        if let Some(v) = arg(self.args, "music") {
            d.tracks.clear();
            if v.is_empty() || v == "any" || v == "none" || v == "video" {
                d.music = if v.is_empty() { "any".to_owned() } else { v };
            } else {
                "tracks".clone_into(&mut d.music);
                d.tracks = split(&v, ',');
            }
        }
        if let Some(v) = arg(self.args, "hide") {
            d.hide = [("bar", HIDE_BAR), ("percent", HIDE_PERCENT), ("stage", HIDE_STAGE), ("tip", HIDE_TIP)]
                .into_iter()
                .filter(|(n, _)| v.contains(n))
                .fold(0, |m, (_, b)| m | b);
        }
        if let Some(v) = arg(self.args, "new_id") {
            let wanted = sanitize(&v);
            if wanted != d.id {
                if others.screen(&wanted).is_some() {
                    self.err = format!("a screen '{wanted}' already exists");
                } else {
                    d.id = wanted;
                }
            }
        }
    }

    fn apply_track(&mut self, t: &mut Track) {
        if let Some(v) = arg(self.args, "file")
            && let Some(rel) = self.import(&v)
        {
            t.file = rel;
        }
        if let Some(v) = arg(self.args, "play") {
            t.play = parse_surfaces(&v);
        }
        if let Some(f) = arg_float(self.args, "weight") {
            t.weight = f.max(0.0);
        }
        if let Some(f) = arg_float(self.args, "volume") {
            t.volume = f.clamp(0.0, 1.0);
        }
    }

    /// Files brought from anywhere: imported in parallel, then put where they go -
    /// music into the tracks, anything else on the screen `screen` (its layer `layer`, or a new
    /// one), or as a new screen of its own.
    fn take_files(&mut self, s: &mut Screens, files: Vec<String>, screen: &str, layer: i64, to_music: bool) {
        let root = self.root.to_owned();
        let imported = self.exec.par_io(files, |f| import(&root, &f));
        for result in imported {
            let rel = match result {
                Ok(rel) => rel,
                Err(e) => {
                    self.err = e;
                    continue;
                }
            };
            if file_kind(&rel) == FileKind::Music {
                let id = s.new_track_id(&stem(&rel));
                s.tracks.push(Track { id, file: rel, play: self.show, ..Track::default() });
                continue;
            }
            if to_music {
                "Only .ogg files go into the music.".clone_into(&mut self.err);
                continue;
            }
            let found = if screen.is_empty() { None } else { s.screen_mut(screen) };
            match found {
                None => {
                    let id = s.new_screen_id(&stem(&rel));
                    s.screens.push(Screen {
                        id,
                        show: self.show,
                        layers: vec![Layer { file: rel, ..Layer::default() }],
                        ..Screen::default()
                    });
                }
                Some(d) => match usize::try_from(layer).ok().and_then(|i| d.layers.get_mut(i)) {
                    Some(l) => l.file = rel,
                    None => d.layers.push(Layer { file: rel, ..Layer::default() }),
                },
            }
        }
    }
}

const ACTIONS: &str = "action must be policy, screen_add, screen_set, screen_remove, layer_add, layer_set, layer_remove, layer_move, track_add, track_set, track_remove or drop";

/// Runs one `screens.set` action on the model. `Err`: the call is refused before anything is
/// done to the model (`surface`, `action`); `Ok(err)`: the action ran, and a non-empty `err`
/// refuses the save.
fn run(cx: &mut Cx<'_>, s: &mut Screens) -> Result<(), String> {
    let action = arg(cx.args, "action").unwrap_or_default();
    let id = arg(cx.args, "id").unwrap_or_default();
    let no_screen = |id: &str| format!("no screen '{id}'");
    match action.as_str() {
        "policy" => {
            let surface = arg(cx.args, "surface").unwrap_or_default();
            let side = match surface.as_str() {
                "loading" => 0,
                "menu" => 1,
                _ => return Err("surface must be loading or menu".to_owned()),
            };
            if let Some(v) = arg(cx.args, "screens")
                && let Some(slot) = s.only_screens.get_mut(side)
            {
                *slot = v == "only";
            }
            if let Some(v) = arg(cx.args, "music")
                && let Some(slot) = s.only_music.get_mut(side)
            {
                *slot = v == "only";
            }
            if let Some(v) = arg(cx.args, "end")
                && let Some(slot) = s.next_track.get_mut(side)
            {
                *slot = v == "next";
            }
        }
        "screen_add" => {
            let mut d = Screen {
                id: s.new_screen_id(if id.is_empty() { "screen" } else { &id }),
                ..Screen::default()
            };
            cx.apply_screen(&mut d, s);
            if arg(cx.args, "file").is_some_and(|v| !v.is_empty()) {
                let mut l = Layer::default();
                cx.apply_layer(&mut l);
                d.layers.push(l);
            }
            s.screens.push(d);
        }
        "screen_set" => match s.screens.iter().position(|d| d.id == id) {
            Some(i) => {
                let others = s.clone();
                if let Some(d) = s.screens.get_mut(i) {
                    cx.apply_screen(d, &others);
                }
            }
            None => cx.err = no_screen(&id),
        },
        "screen_remove" => {
            if s.screen(&id).is_some() {
                s.screens.retain(|d| d.id != id);
            } else {
                cx.err = no_screen(&id);
            }
        }
        "layer_add" => match s.screens.iter().position(|d| d.id == id) {
            Some(i) => {
                let mut l = Layer::default();
                cx.apply_layer(&mut l);
                if let Some(d) = s.screens.get_mut(i) {
                    let len = i64::try_from(d.layers.len()).unwrap_or(i64::MAX);
                    let at = usize::try_from(arg_int(cx.args, "index", len).clamp(0, len)).unwrap_or(0);
                    d.layers.insert(at.min(d.layers.len()), l);
                }
            }
            None => cx.err = no_screen(&id),
        },
        "layer_set" | "layer_remove" | "layer_move" => match s.screens.iter().position(|d| d.id == id) {
            Some(i) => {
                let index = arg_int(cx.args, "index", -1);
                let Some(d) = s.screens.get_mut(i) else { return Ok(()) };
                let len = d.layers.len();
                match usize::try_from(index).ok().filter(|&k| k < len) {
                    None => "no layer at that index (0 is the bottom layer)".clone_into(&mut cx.err),
                    Some(k) => match action.as_str() {
                        "layer_set" => {
                            if let Some(l) = d.layers.get_mut(k) {
                                cx.apply_layer(l);
                            }
                        }
                        "layer_remove" => {
                            d.layers.remove(k);
                        }
                        _ => {
                            let last = i64::try_from(len - 1).unwrap_or(0);
                            let to =
                                usize::try_from(arg_int(cx.args, "to", index).clamp(0, last)).unwrap_or(0);
                            let moved = d.layers.remove(k);
                            d.layers.insert(to.min(d.layers.len()), moved);
                        }
                    },
                }
            }
            None => cx.err = no_screen(&id),
        },
        "track_add" => {
            let mut t =
                Track { id: s.new_track_id(if id.is_empty() { "track" } else { &id }), ..Track::default() };
            cx.apply_track(&mut t);
            if t.file.is_empty() && cx.err.is_empty() {
                "a track needs a file".clone_into(&mut cx.err);
            }
            if cx.err.is_empty() {
                s.tracks.push(t);
            }
        }
        "track_set" => match s.track_mut(&id) {
            Some(t) => cx.apply_track(t),
            None => cx.err = format!("no track '{id}'"),
        },
        "track_remove" => s.remove_track(&id),
        "drop" => {
            // the path of a drop in the tab: files separated by |; on a screen, a layer of it
            // (index), the music, or the tab
            let files = split(&arg(cx.args, "files").unwrap_or_default(), '|');
            let index = arg_int(cx.args, "index", -1);
            let music = arg_bool(cx.args, "music").unwrap_or(false);
            cx.take_files(s, files, &id, index, music);
        }
        _ => return Err(ACTIONS.to_owned()),
    }
    Ok(())
}

async fn open(ed: &Arc<Editor>) -> Result<(Arc<Project>, Arc<ManifestService>), CmdError> {
    let (session, project) = Project::of(ed)?;
    let service = ManifestService::of_session(ed, &session).await?;
    let _ = service.sync().await;
    Ok((project, service))
}

async fn screens_info(ed: Arc<Editor>, _p: NoParams, _origin: Origin) -> Reply {
    let (project, manifest) = open(&ed).await?;
    let snap = manifest.snapshot();
    let Some(doc) = snap.doc() else { return Err(refuse("the project has no mod.ltx: it is no module")) };
    let s = Screens::parse(doc);
    let found = problems(&ed, PathBuf::from(project.root()), &manifest, s.clone()).await;
    Ok(answer(&s, &found))
}

/// The parameters as the request map the actions read.
fn as_args(value: impl Serialize) -> Args {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map,
        _ => Args::new(),
    }
}

async fn screens_set(ed: Arc<Editor>, p: SetParams, origin: Origin) -> Reply {
    let mut args = as_args(&p);
    if let Some(r) = p.rect {
        args.insert("rect".into(), json!(r.iter().map(f32::to_string).collect::<Vec<_>>().join(" ")));
    }
    apply(ed, args, origin).await
}

async fn screens_import(ed: Arc<Editor>, p: ImportParams, origin: Origin) -> Reply {
    if p.path.is_empty() {
        return Err(CmdError::invalid("screens.import needs path: one file or a list"));
    }
    let mut args = Args::new();
    args.insert("action".into(), json!("drop"));
    args.insert("files".into(), json!(p.path.join("|")));
    if let Some(id) = p.screen {
        args.insert("id".into(), json!(id));
    }
    if let Some(layer) = p.layer {
        args.insert("index".into(), json!(layer));
    }
    args.insert("music".into(), json!(p.music));
    if let Some(surface) = p.surface {
        args.insert("surface".into(), json!(surface));
    }
    apply(ed, args, origin).await
}

async fn apply(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let (project, manifest) = open(&ed).await?;
    let snap = manifest.snapshot();
    let Some(doc) = snap.doc() else { return Err(refuse("the project has no mod.ltx: it is no module")) };
    let mut s = Screens::parse(doc);
    let root = project.root().to_owned();
    let exec = ed.exec.clone();
    let dropping = arg(&args, "action").as_deref() == Some("drop");
    let show = default_show(dropping && drop_surface(&ed, &args).await);
    // imports are file work: the whole action runs on the IO pool
    let (s, outcome) = ed
        .exec
        .io(move || {
            let mut cx = Cx { root: &root, show, args: &args, exec: &exec, err: String::new() };
            let outcome = run(&mut cx, &mut s).map(|()| cx.err);
            (s, outcome)
        })
        .await;
    let err = outcome.map_err(refuse)?;
    if !err.is_empty() {
        return Err(refuse(err));
    }
    // a file brought in from outside landed in screens\: the tree lists it again
    project.tree.mark_fs(&PathBuf::from(project.root()).join("screens"));
    let edits = {
        let now = manifest.snapshot();
        let Some(doc) = now.doc() else { return Err(refuse("the project has no mod.ltx: it is no module")) };
        s.edits(doc)
    };
    manifest
        .edit(edits, origin)
        .await
        .map_err(|e| if e.code == "io" { refuse("can't write mod.ltx") } else { e })?;
    let found = problems(&ed, PathBuf::from(project.root()), &manifest, s.clone()).await;
    Ok(answer(&s, &found))
}
