//! The screens' commands, the build's check and the slot for the linked game's levels:
//! `screens.info` (the screens, the music and the radio as data, with their problems),
//! `screens.set` (one change of them, checked) and `screens.import` (files brought from anywhere,
//! put where they go).
//! `screens.view` is the tab's. An agent may equally edit `mod.ltx` itself.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Origin, Problem};
use gz_build::{
    BuildCheck, BuildChecks, BuildWalk, MediaPlan, MediaPublisher, Publication, Sources, source_key,
};
use gz_editor::{Args, CmdError, Editor, Executors, NoParams, ProjectSession, Registry, Reply, Spec};
use gz_media::sound::Mark;
use gz_mod::{Issue, ManifestService};
use gz_project::Project;
use parking_lot::RwLock;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::import::{FileKind, file_kind, import, is_music, stem};
use crate::model::{
    Fit, HIDE_BAR, HIDE_PERCENT, HIDE_STAGE, HIDE_TIP, LOADING, Layer, MENU, Order, PAUSE, Playlist, RADIO,
    Screen, Screens, Track, atof, sanitize, scan_floats, split,
};

pub fn register(reg: &mut Registry) {
    reg.area(
        "screens",
        "The module's loading and menu screens, their music and its radio music ([screens], [track.*], [playlist.*], [radio] of mod.ltx)",
    );
    reg.cmd(
        Spec::read(
            "screens.info",
            "The module's screens, layers, music tracks, radio tracks and playlists as data, with their problems.",
        ),
        screens_info,
    );
    reg.cmd(
        Spec::write(
            "screens.set",
            "One change of the screens, the music or the radio: a screen, a layer, a track, a playlist, the policy.",
        )
        .example("action=track_add file=D:/Music/rain.ogg play=menu,radio"),
        screens_set,
    );
    reg.cmd(
        Spec::write(
            "screens.import",
            "Brings files into the screens: a picture or a video becomes a screen or a layer, music a sound asset and a track that plays it.",
        )
        .example("path=D:/Art/night.mp4 screen=night"),
        screens_import,
    );
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SetAction {
    /// What a surface's screens and music do with the game's own: `surface`, `screens` (only or
    /// mix), `music` (only or mix), `end` (loop or next). `surface=radio` takes `music` alone:
    /// only makes this module's radio tracks the radios' whole common music.
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
    /// A new track `id` with `sound` or `file`, play, weight, volume.
    TrackAdd,
    /// Changes the track `id`: sound, file, play, weight, volume.
    TrackSet,
    /// Make It a Sound: the track `id`'s plain file becomes a sound asset (written beside it) and
    /// the track names its sound - only a sound can carry the copyrighted mark.
    TrackSound,
    /// Removes the track `id`, from every screen and playlist too.
    TrackRemove,
    /// A new playlist `id` (made unique; left out: made of the title) with title, tracks, order.
    PlaylistAdd,
    /// Changes the playlist `id`: title, tracks (the whole list, in its order), order.
    PlaylistSet,
    /// Removes the playlist `id`. Its tracks stay.
    PlaylistRemove,
    /// Puts `track` into the playlist `id` at `index` (left out: at its end).
    PlaylistTrackAdd,
    /// Moves `track` of the playlist `id` to `to`.
    PlaylistTrackMove,
    /// Takes `track` out of the playlist `id`. The track stays in the music.
    PlaylistTrackRemove,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetParams {
    action: SetAction,
    /// The screen, the track or the playlist; for screen_add, track_add and playlist_add the wanted
    /// id, made of a-z, 0-9 and _ (anything else becomes _).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// Policy: loading, menu or radio.
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
    /// A screen or a track: how often it is picked beside the others, 0 to 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight: Option<f32>,
    /// A screen: what of the game's loading screen it hides (bar, percent, stage, tip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hide: Option<String>,
    /// A screen: its new id, made of a-z, 0-9 and _ (anything else becomes _).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new_id: Option<String>,
    /// A layer or a track: its file, from anywhere; empty clears a layer's. A layer takes a picture,
    /// a GIF, a video, a shader or an HTML page (brought into screens\ when it is outside the
    /// project; a PNG, JPG, TGA or BMP becomes a .dds). A track takes music (.ogg, .wav), brought in
    /// as a sound asset the way asset.import brings a sound, and the track names that sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// A shader layer: its texture (a .dds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    texture: Option<String>,
    /// A layer: cover (cut to fit), contain (whole) or stretch (pulled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fit: Option<String>,
    /// A layer: 0 to 1; a number outside it is cut to the nearest end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opacity: Option<f32>,
    /// A layer: its place as x, y, width, height in per cent of the screen; [0, 0, 100, 100] (the
    /// whole screen) when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rect: Option<[f32; 4]>,
    /// A video layer: whether its sound plays (true or false). A track: the sound it plays, by its
    /// name (a sound asset of the module or a sound of the game), or a sound asset of the project by
    /// its path (`sounds/rain.sndasset`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sound: Option<Value>,
    /// A layer: its index, 0 the bottom one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<i64>,
    /// layer_move: where the layer goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<i64>,
    /// A track: the shuffles it joins (loading, menu, pause, radio; comma separated); empty:
    /// only where a screen or a playlist names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    play: Option<String>,
    /// A track: how loud, 0 to 1; a number outside it is cut to the nearest end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    volume: Option<f32>,
    /// Refused: a track carries no copyrighted mark. Whether music is copyrighted is its sound's to
    /// say - Copyrighted in the sound asset (the Sound tab, or its `copyrighted` key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    copyrighted: Option<bool>,
    /// Refused: what plays instead in the streamer mode is the sound's too (its `streamer` key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    streamer: Option<String>,
    /// A playlist: what the player reads in the list of stations (text or a string table id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// A playlist: its track ids, in the order they play.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracks: Option<Vec<String>>,
    /// A playlist: shuffle (every track once a pass, a new order each pass) or list (in order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<String>,
    /// playlist_track_add, playlist_track_move, playlist_track_remove: the track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    track: Option<String>,
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
    /// Into the music: only music (.ogg, .wav) and sound assets are taken.
    #[serde(default)]
    music: bool,
    /// What a new screen or track shows on: loading, menu or radio (a track the radios play; left
    /// out: what the open tab shows).
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

/// The problems of `s` in the open project, on the IO pool, each with the section and the key
/// it is about.
pub async fn issues(
    editor: &Arc<Editor>,
    root: PathBuf,
    manifest: &Arc<ManifestService>,
    s: Screens,
) -> Vec<Issue> {
    let levels = GameLevels::of(editor).levels(editor).await;
    let manifest = manifest.clone();
    let sounds = Sounds::now(editor).await;
    editor
        .exec
        .io(move || {
            crate::validate::issues(
                &root,
                &s,
                &|rel| manifest.outside_build(rel),
                levels.as_deref(),
                &|name| sounds.facts(name),
            )
        })
        .await
}

/// The sounds a track names, as `gz-media` reads them: the module's sound assets and the linked
/// game's sounds with its own list - what one is to the streamer mode, whose it is, and the way
/// music comes in as a sound asset.
pub struct Sounds {
    look: Option<(PathBuf, Arc<gz_media::index::Index>, Arc<gz_media::GameLists>)>,
    guard: Option<gz_editor::fsx::WriteGuard>,
}

impl Sounds {
    /// No project: every sound is free and nothing comes in.
    #[must_use]
    pub fn none() -> Arc<Self> {
        Arc::new(Self { look: None, guard: None })
    }

    /// The media of the open project, its files as the tree holds every change so far.
    pub async fn now(editor: &Arc<Editor>) -> Arc<Self> {
        let Ok(media) = gz_media::Media::of(editor).await else { return Self::none() };
        let index = media.fresh_index().await;
        let game = media.game_lists(editor).await;
        let guard = Project::of(editor).ok().map(|(_, p)| p.write_guard(None));
        Arc::new(Self { look: Some((media.root().to_path_buf(), index, game)), guard })
    }

    /// What a sound name is to the streamer mode and whose it is. Blocking: a sound asset of the
    /// module is read.
    #[must_use]
    pub fn facts(&self, name: &str) -> gz_media::sound::Facts {
        match &self.look {
            Some((root, index, game)) => gz_media::sound::facts(root, index, game, name),
            None => gz_media::sound::Facts { mark: Mark::Free, instead: String::new(), asset: None },
        }
    }

    /// Blocking: a sound asset of the module is read.
    #[must_use]
    pub fn mark(&self, name: &str) -> Mark {
        self.facts(name).mark
    }

    /// How long a sound lasts, in seconds: the module's asset's source, else the game's
    /// `sounds\<name>.ogg`. Blocking: the file is read.
    #[must_use]
    pub fn seconds(&self, name: &str) -> Option<f64> {
        use gz_project::asset_kind::MediaKind;
        let (root, index, game) = self.look.as_ref()?;
        let length = |bytes: &[u8], ogg: bool| -> Option<f64> {
            if ogg {
                gz_fmt_sound::ogg_info(bytes).ok().map(|i| i.seconds())
            } else {
                gz_fmt_sound::read_wav(bytes).ok().map(|w| w.frames() as f64 / f64::from(w.rate.max(1)))
            }
        };
        if let Some(e) = index.by_name(MediaKind::Sound, name) {
            let (_, source, _) = gz_media::resolve::sound(root, e.path.as_str()).ok()?;
            let ogg = source.extension().is_some_and(|x| x.eq_ignore_ascii_case("ogg"));
            return length(&std::fs::read(&source).ok()?, ogg);
        }
        let files = game.files.as_ref()?;
        let bytes = files.read(&format!("sounds\\{}.ogg", gz_media::read::name_key(name))).ok()?;
        length(&bytes, true)
    }

    /// Every sound that plays in the streamer mode. Blocking.
    #[must_use]
    pub fn free(&self) -> Vec<gz_media::sound::FreeSound> {
        match &self.look {
            Some((root, index, game)) => gz_media::sound::free_sounds(root, index, game),
            None => Vec::new(),
        }
    }

    /// Music from anywhere (or a sound asset of the project) as a sound asset of the module:
    /// [`gz_media::import::sound_asset`], the names the project's sound assets publish taken.
    /// `made` holds the names made by this call so far. Blocking file work.
    pub fn import(
        &self,
        given: &str,
        made: &mut Vec<String>,
    ) -> Result<gz_media::import::SoundImport, String> {
        use gz_project::asset_kind::MediaKind;
        let Some((root, index, _)) = &self.look else { return Err("no project is open".to_owned()) };
        let path = std::path::Path::new(given);
        let from = if path.is_absolute() { path.to_path_buf() } else { root.join(given.replace('/', "\\")) };
        let taken = |n: &str| {
            index.by_name(MediaKind::Sound, n).is_some() || made.iter().any(|m| m.eq_ignore_ascii_case(n))
        };
        let got = gz_media::import::sound_asset(root, &from, &index.module, &taken, self.guard.as_ref())?;
        made.push(got.name.clone());
        Ok(got)
    }
}

/// The problems of `s` in the open project as lines, each starting with its code.
pub async fn problems(
    editor: &Arc<Editor>,
    root: PathBuf,
    manifest: &Arc<ManifestService>,
    s: Screens,
) -> Vec<String> {
    issues(editor, root, manifest, s).await.iter().map(Issue::line).collect()
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
            issues(&editor, session.root.clone(), &manifest, s).await.iter().map(Issue::problem).collect()
        })
    }
}

/// The files the screens and the music ship where they lie (`docs/ASSETS.md` section 6): every
/// layer's file and its shader's picture, the folder of a page or a sequence, and
/// a track's plain file. A track that names its sound ships it through the sound's asset.
#[must_use]
pub fn ships_of(s: &Screens) -> Sources {
    let mut out = Sources::default();
    for l in s.screens.iter().flat_map(|sc| &sc.layers) {
        for f in [&l.file, &l.texture].into_iter().filter(|f| !f.trim().is_empty()) {
            out.insert(source_key(f.trim()));
        }
        // a page reads the files beside it, a sequence names its frames
        if matches!(file_kind(&l.file), FileKind::Page)
            || l.file.trim().to_ascii_lowercase().ends_with(".seq")
        {
            let file = source_key(l.file.trim());
            if let Some((folder, _)) = file.rsplit_once('/') {
                out.insert(folder.to_owned());
            }
        }
    }
    out.extend(
        s.tracks
            .iter()
            .filter(|t| t.sound.trim().is_empty() && !t.file.trim().is_empty())
            .map(|t| source_key(t.file.trim())),
    );
    out
}

/// Tells the build which raw files of the project the screens and the music ship.
struct ScreensFiles;

async fn screen_ships(editor: Arc<Editor>, session: Arc<ProjectSession>) -> Sources {
    let Ok(manifest) = ManifestService::of_session(&editor, &session).await else {
        return Sources::default();
    };
    let _ = manifest.sync().await;
    let snap = manifest.snapshot();
    snap.doc().map(|doc| ships_of(&Screens::parse(doc))).unwrap_or_default()
}

impl MediaPublisher for ScreensFiles {
    fn plan(&self, editor: Arc<Editor>, session: Arc<ProjectSession>) -> BoxFuture<'static, MediaPlan> {
        Box::pin(
            async move { MediaPlan { ships: screen_ships(editor, session).await, ..MediaPlan::default() } },
        )
    }

    fn publish(&self, editor: Arc<Editor>, session: Arc<ProjectSession>) -> BoxFuture<'static, Publication> {
        Box::pin(async move {
            Publication { ships: screen_ships(editor, session).await, ..Publication::default() }
        })
    }
}

/// Hangs the area on the editor: the build's check and what the screens ship.
pub fn install(editor: &Editor) {
    let checks = BuildChecks::of(editor);
    checks.add(Arc::new(ScreensCheck));
    checks.add_publisher(Arc::new(ScreensFiles));
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
    mask_text(mask, &[(LOADING, "loading"), (MENU, "menu"), (PAUSE, "pause"), (RADIO, "radio")])
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

/// What the answer says of a track's music: its sound's facts, read where the game reads them.
pub type TrackFacts = std::collections::HashMap<String, gz_media::sound::Facts>;

/// The facts of every sound the tracks name. Blocking: the module's sound assets are read.
#[must_use]
pub fn track_facts(s: &Screens, sounds: &Sounds) -> TrackFacts {
    let mut out = TrackFacts::default();
    for t in s.tracks.iter().filter(|t| t.is_sound()) {
        out.entry(t.sound.clone()).or_insert_with(|| sounds.facts(&t.sound));
    }
    out
}

/// The answer of both commands (`Answer`). `facts` says what the tracks' sounds are
/// ([`track_facts`]); a track whose sound it lacks answers it free.
#[must_use]
pub fn answer(s: &Screens, problems: &[String], facts: &TrackFacts) -> Value {
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
            // what it plays: its sound, else its file - and what that sound says of itself
            let f = t.is_sound().then(|| facts.get(&t.sound)).flatten();
            o.insert("plays".into(), json!(if t.is_sound() { "sound" } else { "file" }));
            o.insert("sound".into(), json!(t.sound));
            o.insert("file".into(), json!(t.file));
            o.insert("asset".into(), json!(f.and_then(|f| f.asset.clone())));
            o.insert("copyrighted".into(), json!(f.is_some_and(|f| f.mark == Mark::Copyrighted)));
            o.insert(
                "streamer".into(),
                json!(f.filter(|f| f.mark == Mark::Copyrighted).map_or("", |f| f.instead.as_str())),
            );
            o.insert("missing".into(), json!(f.is_some_and(|f| f.mark == Mark::Missing)));
            o.insert("play".into(), json!(surfaces_json(t.play)));
            o.insert("weight".into(), json_number(t.weight));
            o.insert("volume".into(), json_number(t.volume));
            Value::Object(o)
        })
        .collect();
    let playlists: Vec<Value> = s
        .playlists
        .iter()
        .map(|p| json!({ "id": p.id, "title": p.title, "tracks": p.tracks, "order": p.order.name() }))
        .collect();
    let radio_tracks: Vec<&str> =
        s.tracks.iter().filter(|t| t.play & RADIO != 0).map(|t| t.id.as_str()).collect();
    let mut out = Map::new();
    out.insert("policy".into(), json!({ "loading": side(0), "menu": side(1) }));
    out.insert("screens".into(), Value::Array(screens));
    out.insert("tracks".into(), Value::Array(tracks));
    out.insert(
        "radio".into(),
        json!({ "music": if s.only_radio { "only" } else { "mix" }, "tracks": radio_tracks }),
    );
    out.insert("playlists".into(), Value::Array(playlists));
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

/// What a screen or a track made by a drop shows on: `surface=` when given (loading, menu - the
/// main and the pause menu -, radio: a track the radios play), else the surface the open Screens
/// tab shows, else the loading screen.
async fn drop_show(ed: &Arc<Editor>, args: &Args) -> u32 {
    let surface = match arg(args, "surface") {
        Some(v) => v,
        None if ed.registry().has("screens.view") => {
            let view = ed.call("screens.view", Args::new(), Origin::Mcp).await;
            let open = view.get("open").and_then(Value::as_bool) == Some(true);
            let shown = view.get("surface").and_then(Value::as_str).unwrap_or_default();
            if open { shown.to_owned() } else { String::new() }
        }
        None => String::new(),
    };
    match surface.as_str() {
        "menu" => MENU | PAUSE,
        "radio" => RADIO,
        _ => LOADING,
    }
}

/// A refusal when `value` names something `key` does not take: the words are separated by commas
/// and read without case, as the game reads them; the game would drop the others silently.
fn words_refusal(key: &str, value: &str, allowed: &[&str]) -> Option<String> {
    let bad = split(value, ',').into_iter().find(|w| !allowed.iter().any(|a| a.eq_ignore_ascii_case(w)))?;
    Some(format!("{key} takes {} (comma separated), not '{bad}'", allowed.join(", ")))
}

/// The last part of a path as given.
fn leaf(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Writes the mark into a sound asset just made: `copyrighted = true` and, when one is named,
/// what plays instead. Blocking.
fn mark_asset(root: &str, asset: &str, instead: Option<&str>) -> Result<(), String> {
    use gz_luadata::{Seg, Value};
    let abs = std::path::Path::new(root).join(asset.replace('/', "\\"));
    let text = std::fs::read_to_string(&abs).map_err(|e| format!("{asset}: {}", gz_base::io_reason(&e)))?;
    let mut doc = gz_luadata::LuaDoc::parse(&text).map_err(|e| format!("{asset}: {}", e.message))?;
    let set = |doc: &mut gz_luadata::LuaDoc, key: &str, v: Value| {
        doc.set(&[Seg::key(key)], &v).map_err(|e| format!("{asset}: {}", e.message))
    };
    set(&mut doc, gz_media::sound::MARK_KEY, Value::from(true))?;
    if let Some(r) = instead {
        set(&mut doc, gz_media::sound::REPLACEMENT_KEY, Value::from(r))?;
    }
    gz_editor::fsx::write_atomic(&abs, doc.text().as_bytes())
        .map_err(|e| format!("{asset}: {}", gz_base::io_reason(&e)))
}

/// Everything an action needs while it runs on the IO pool.
struct Cx<'a> {
    root: &'a str,
    /// What a screen or a track made by a drop shows on.
    show: u32,
    args: &'a Args,
    exec: &'a Executors,
    /// The sounds the tracks name, and the way music comes in as a sound asset.
    sounds: &'a Sounds,
    /// The names of the sound assets this action made.
    made: Vec<String>,
    /// The sound assets written, project-relative: the tree lists their folders again.
    written: Vec<String>,
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
            } else if file_kind(&v) == FileKind::Music {
                self.err = format!(
                    "a layer shows a picture, a GIF, a video, a shader or an HTML page, not {}: music goes into a track (action=track_add)",
                    leaf(&v)
                );
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
            if matches!(v.as_str(), "cover" | "contain" | "stretch") {
                l.fit = Fit::parse(&v);
            } else {
                self.err = format!("fit takes cover, contain or stretch, not '{v}'");
            }
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
            match words_refusal("show", &v, &["loading", "menu", "pause"]) {
                Some(why) => self.err = why,
                None => d.show = parse_surfaces(&v),
            }
        }
        if let Some(v) = arg(self.args, "levels") {
            d.levels = split(&v, ',');
        }
        if let Some(v) = arg(self.args, "modes") {
            d.modes = split(&v, ',');
        }
        if let Some(f) = arg_float(self.args, "weight") {
            d.weight = f.clamp(0.0, 1000.0);
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
        if let Some(why) = arg(self.args, "hide")
            .and_then(|v| words_refusal("hide", &v, &["bar", "percent", "stage", "tip"]))
        {
            self.err = why;
        } else if let Some(v) = arg(self.args, "hide") {
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

    /// Music from anywhere, or a sound asset of the project, as the module's sound: its name.
    fn import_sound(&mut self, given: &str) -> Option<String> {
        match self.sounds.import(given, &mut self.made) {
            Ok(got) => {
                if got.written {
                    self.written.push(got.asset.clone());
                }
                Some(got.name)
            }
            Err(e) => {
                self.err = e;
                None
            }
        }
    }

    fn apply_track(&mut self, t: &mut Track) {
        // the mark is the sound's: refused before anything is copied, naming where it is set
        for key in gz_mod::reference::TRACK_MARK_KEYS {
            if self.args.contains_key(*key) {
                let asset = if t.is_sound() { self.sounds.facts(&t.sound).asset } else { None };
                self.err = crate::validate::mark_moved(key, t, asset.as_deref());
                return;
            }
        }
        // before anything is copied or converted: a picture is no track
        if let Some(v) = arg(self.args, "file") {
            if !is_music(&v) {
                self.err = format!("a track takes music (.ogg, .wav) or a sound asset, not {}", leaf(&v));
            } else if let Some(name) = self.import_sound(&v) {
                t.sound = name;
                t.file.clear();
            }
        }
        if let Some(v) = arg(self.args, "sound").map(|v| v.trim().to_owned()) {
            if v.to_ascii_lowercase().ends_with(".sndasset") {
                if let Some(name) = self.import_sound(&v) {
                    t.sound = name;
                }
            } else if self.sounds.mark(&v) == Mark::Missing {
                self.err = format!(
                    "sound: nobody has the sound '{v}' - no sound asset of the project publishes it, and the linked game has none of that name"
                );
            } else {
                t.sound = v.replace('/', "\\");
            }
        }
        if let Some(v) = arg(self.args, "play") {
            match words_refusal("play", &v, &["loading", "menu", "pause", "radio"]) {
                Some(why) => self.err = why,
                None => t.play = crate::model::plays(&v.to_ascii_lowercase()),
            }
        }
        if let Some(f) = arg_float(self.args, "weight") {
            t.weight = f.clamp(0.0, 1000.0);
        }
        if let Some(f) = arg_float(self.args, "volume") {
            t.volume = f.clamp(0.0, 1.0);
        }
    }

    /// Make It a Sound: a plain file's track names a sound asset written over that file; the mark
    /// the track carried goes into the asset it is now said in.
    fn make_sound(&mut self, t: &mut Track, others: &Screens) {
        if t.is_sound() {
            self.err = format!("the track '{}' plays the sound {} already", t.id, t.sound);
            return;
        }
        if t.file.trim().is_empty() {
            self.err = format!("the track '{}' has no file to make a sound of", t.id);
            return;
        }
        let Ok(got) = self.sounds.import(&t.file.clone(), &mut self.made).map_err(|e| self.err = e) else {
            return;
        };
        if got.written {
            self.written.push(got.asset.clone());
        }
        let copyrighted = t.extra("copyrighted").is_some_and(|v| gz_mod::rules::parse_flag(v) == Some(true));
        if got.written && copyrighted {
            // a replacement named by a track of the module is that track's sound
            let instead = t.extra("streamer").map(|r| match others.track(r) {
                Some(o) if o.is_sound() => o.sound.clone(),
                _ => r.to_owned(),
            });
            if let Err(e) = mark_asset(self.root, &got.asset, instead.as_deref().filter(|r| !r.is_empty())) {
                self.err = e;
                return;
            }
        }
        t.extra.retain(|(k, _)| !gz_mod::reference::TRACK_MARK_KEYS.contains(&k.as_str()));
        t.sound = got.name;
        t.file.clear();
    }

    /// Track ids of a playlist: every one a track of the module, refused by name otherwise.
    fn playlist_tracks(&mut self, s: &Screens, given: &str) -> Option<Vec<String>> {
        let ids = split(given, ',');
        if let Some(bad) = ids.iter().find(|id| s.track(id).is_none()) {
            self.err = format!("a playlist plays the module's tracks, and there is no track '{bad}'");
            return None;
        }
        Some(ids)
    }

    /// The playlist's fields from the request: title, tracks, order.
    fn apply_playlist(&mut self, p: &mut Playlist, s: &Screens) {
        if let Some(v) = arg(self.args, "title") {
            p.title = v.trim().to_owned();
        }
        if let Some(v) = arg(self.args, "tracks")
            && let Some(ids) = self.playlist_tracks(s, &v)
        {
            p.tracks = ids;
        }
        if let Some(v) = arg(self.args, "order") {
            match v.as_str() {
                "shuffle" | "list" => p.order = Order::parse(&v),
                _ => self.err = format!("order takes shuffle or list, not '{v}'"),
            }
        }
    }

    /// Files brought from anywhere: imported in parallel, then put where they go - music as a
    /// sound asset and a track that names it, anything else on the screen `screen` (its layer
    /// `layer`, or a new one), or as a new screen of its own.
    fn take_files(&mut self, s: &mut Screens, files: Vec<String>, screen: &str, layer: i64, to_music: bool) {
        let (music, rest): (Vec<String>, Vec<String>) = files.into_iter().partition(|f| is_music(f));
        for f in music {
            let Some(name) = self.import_sound(&f) else { continue };
            // the same sound twice is one track
            if s.tracks.iter().any(|t| t.sound.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let id = s.new_track_id(&stem(&f));
            s.tracks.push(Track { id, sound: name, play: self.show, ..Track::default() });
        }
        let root = self.root.to_owned();
        let imported = self.exec.par_io(rest, |f| import(&root, &f));
        for result in imported {
            let rel = match result {
                Ok(rel) => rel,
                Err(e) => {
                    self.err = e;
                    continue;
                }
            };
            if to_music || self.show == RADIO {
                "Only music (.ogg, .wav) and sound assets go into the music.".clone_into(&mut self.err);
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

const ACTIONS: &str = "action must be policy, screen_add, screen_set, screen_remove, layer_add, layer_set, layer_remove, layer_move, track_add, track_set, track_sound, track_remove, playlist_add, playlist_set, playlist_remove, playlist_track_add, playlist_track_move, playlist_track_remove or drop";

/// Runs one `screens.set` action on the model. `Err`: the call is refused before anything is
/// done to the model (`surface`, `action`); `Ok(err)`: the action ran, and a non-empty `err`
/// refuses the save.
fn run(cx: &mut Cx<'_>, s: &mut Screens) -> Result<(), String> {
    let action = arg(cx.args, "action").unwrap_or_default();
    let id = arg(cx.args, "id").unwrap_or_default();
    let no_screen = |id: &str| format!("no screen '{id}'");
    match action.as_str() {
        "policy" if arg(cx.args, "surface").as_deref() == Some("radio") => {
            if arg(cx.args, "screens").is_some() || arg(cx.args, "end").is_some() {
                return Err("the radio's policy is music alone (only or mix): it has no screens and no end"
                    .to_owned());
            }
            match arg(cx.args, "music").as_deref() {
                Some("only") => s.only_radio = true,
                Some("mix") => s.only_radio = false,
                Some(v) => return Err(format!("music takes only or mix, not '{v}'")),
                None => {}
            }
        }
        "policy" => {
            let surface = arg(cx.args, "surface").unwrap_or_default();
            let side = match surface.as_str() {
                "loading" => 0,
                "menu" => 1,
                _ => return Err("surface must be loading, menu or radio".to_owned()),
            };
            for (key, words) in
                [("screens", ["only", "mix"]), ("music", ["only", "mix"]), ("end", ["loop", "next"])]
            {
                if let Some(v) = arg(cx.args, key)
                    && !words.contains(&v.as_str())
                {
                    return Err(format!("{key} takes {} or {}, not '{v}'", words[0], words[1]));
                }
            }
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
            if t.file.is_empty() && !t.is_sound() && cx.err.is_empty() {
                "a track needs its music: sound=<a sound's name> or file=<music from anywhere>"
                    .clone_into(&mut cx.err);
            }
            if cx.err.is_empty() {
                s.tracks.push(t);
            }
        }
        "track_set" => match s.track_mut(&id) {
            Some(t) => cx.apply_track(t),
            None => cx.err = format!("no track '{id}'"),
        },
        "track_sound" => {
            let others = s.clone();
            match s.track_mut(&id) {
                Some(t) => cx.make_sound(t, &others),
                None => cx.err = format!("no track '{id}'"),
            }
        }
        "track_remove" => s.remove_track(&id),
        "playlist_add" => {
            let title = arg(cx.args, "title").unwrap_or_default();
            let base = if !id.is_empty() {
                id.clone()
            } else if !title.trim().is_empty() {
                title.trim().to_owned()
            } else {
                "playlist".to_owned()
            };
            let mut p = Playlist { id: s.new_playlist_id(&base), ..Playlist::default() };
            cx.apply_playlist(&mut p, s);
            if cx.err.is_empty() {
                s.playlists.push(p);
            }
        }
        "playlist_set" => match s.playlists.iter().position(|p| p.id == id) {
            Some(i) => {
                let others = s.clone();
                if let Some(p) = s.playlists.get_mut(i) {
                    cx.apply_playlist(p, &others);
                }
            }
            None => cx.err = format!("no playlist '{id}'"),
        },
        "playlist_remove" => {
            if s.playlist(&id).is_some() {
                s.playlists.retain(|p| p.id != id);
            } else {
                cx.err = format!("no playlist '{id}'");
            }
        }
        "playlist_track_add" | "playlist_track_move" | "playlist_track_remove" => {
            let track = arg(cx.args, "track").unwrap_or_default();
            if s.track(&track).is_none() {
                cx.err = format!("no track '{track}'");
                return Ok(());
            }
            let Some(p) = s.playlist_mut(&id) else {
                cx.err = format!("no playlist '{id}'");
                return Ok(());
            };
            let at = p.tracks.iter().position(|t| *t == track);
            match (action.as_str(), at) {
                ("playlist_track_add", _) => {
                    let len = i64::try_from(p.tracks.len()).unwrap_or(i64::MAX);
                    let to = usize::try_from(arg_int(cx.args, "index", len).clamp(0, len)).unwrap_or(0);
                    p.tracks.insert(to.min(p.tracks.len()), track);
                }
                ("playlist_track_move", Some(from)) => {
                    let last = i64::try_from(p.tracks.len().saturating_sub(1)).unwrap_or(0);
                    let to = usize::try_from(arg_int(cx.args, "to", last).clamp(0, last)).unwrap_or(0);
                    let moved = p.tracks.remove(from);
                    p.tracks.insert(to.min(p.tracks.len()), moved);
                }
                ("playlist_track_remove", Some(from)) => {
                    p.tracks.remove(from);
                }
                _ => cx.err = format!("the playlist '{id}' does not play the track '{track}'"),
            }
        }
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
    Ok(answered(&ed, &s, &found).await)
}

/// [`answer`] with the facts of the tracks' sounds, read on the IO pool.
async fn answered(ed: &Arc<Editor>, s: &Screens, found: &[String]) -> Value {
    let sounds = Sounds::now(ed).await;
    let model = s.clone();
    let facts = ed.exec.io(move || track_facts(&model, &sounds)).await;
    answer(s, found, &facts)
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
    let show = if dropping { drop_show(&ed, &args).await } else { LOADING };
    let sounds = Sounds::now(&ed).await;
    // imports are file work: the whole action runs on the IO pool
    let (s, outcome, wrote) = ed
        .exec
        .io(move || {
            let mut cx = Cx {
                root: &root,
                show,
                args: &args,
                exec: &exec,
                sounds: &sounds,
                made: Vec::new(),
                written: Vec::new(),
                err: String::new(),
            };
            let outcome = run(&mut cx, &mut s).map(|()| cx.err.clone());
            (s, outcome, cx.written)
        })
        .await;
    // a file brought in from outside landed in screens\ or sounds\: the tree lists it again
    let base = PathBuf::from(project.root());
    project.tree.mark_fs(&base.join("screens"));
    for asset in &wrote {
        let abs = base.join(asset.replace('/', "\\"));
        project.tree.mark_fs(abs.parent().unwrap_or(&base));
    }
    if !wrote.is_empty() {
        project.tree.refresh().await;
    }
    let err = outcome.map_err(refuse)?;
    if !err.is_empty() {
        return Err(refuse(err));
    }
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
    Ok(answered(&ed, &s, &found).await)
}
