//! The Sound Library: the `sound_library` tab over the descriptions of `rawdata\sounds`
//! (`UISoundEditorForm.cpp`, `SoundManager.cpp`). The selected sound's game file is read and
//! decoded on the pools - its waveform from the peak envelope - and played on the audio thread.
//! Save writes the changed descriptions and makes their game sounds again from the `.wav`;
//! Rebuild makes the selected one's.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{CornerRadius, Pos2, Sense, Stroke, Ui, Vec2, pos2};
use gz_app::{DocCx, Document, EditAction, OpenRequest};
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin};
use gz_fmt_sound::{
    OggParams, Pcm, Peak, SoundThm, decode_ogg, encode_game_sound, read_sound_thm, read_wav, write_sound_thm,
};
use gz_store::{Apply, State, Store, StoreOptions, imbl};
use gz_ui::grid::{Grid, combo, number, read_only};
use gz_ui::text::{self, Clip};
use gz_ui::{Button, Font, Icon, Kind, PropFlags, PropertyGrid, Tone, Toolbar, palette, state};
use serde_json::{Map, Value, json};

use super::list::{Empty, LibraryList, find, shown};
use super::{LibraryDoc, LibraryLists, Settle, can_write, folder_text, pane, panes};
use crate::audio::Player;
use crate::local::{local_ends, local_heading, local_section};
use crate::paths::{EditorDirs, file_age, list_files, set_file_age};
use crate::pending::Pending;

pub const KIND: &str = "sound_library";
/// The game's rolloff (`psSoundRolloff`): how fast a sound falls off past its minimum distance.
pub const ROLLOFF: f32 = 0.75;
const CURVE_POINTS: usize = 96;
const WAVE_BUCKETS: usize = 512;

/// `anomaly_type_token`: the sound types NPCs react to.
pub const GAME_TYPES: [(u32, &str); 21] = [
    (0, "undefined"),
    (0x4000_0000 | 0x0400_0000, "Item picking up"),
    (0x4000_0000 | 0x0200_0000, "Item dropping"),
    (0x4000_0000 | 0x0080_0000, "Item taking"),
    (0x4000_0000 | 0x0100_0000, "Item hiding"),
    (0x4000_0000 | 0x0040_0000, "Item using"),
    (0x8000_0000 | 0x0020_0000, "Weapon shooting"),
    (0x8000_0000 | 0x0010_0000, "Weapon empty clicking"),
    (0x8000_0000 | 0x0008_0000, "Weapon bullet hit"),
    (0x8000_0000 | 0x0004_0000, "Weapon recharging"),
    (0x2000_0000 | 0x0002_0000, "NPC dying"),
    (0x2000_0000 | 0x0001_0000, "NPC injuring"),
    (0x2000_0000 | 0x0000_8000, "NPC step"),
    (0x2000_0000 | 0x0000_4000, "NPC talking"),
    (0x2000_0000 | 0x0000_2000, "NPC attacking"),
    (0x2000_0000 | 0x0000_1000, "NPC eating"),
    (0x1000_0000 | 0x0000_0800, "Anomaly idle"),
    (0x0800_0000 | 0x0000_0400, "Object breaking"),
    (0x0800_0000 | 0x0000_0200, "Object colliding"),
    (0x0800_0000 | 0x0000_0100, "Object exploding"),
    (0x0800_0000 | 0x0000_0080, "World ambient"),
];

/// A sound of the list: its key and its description as read.
#[derive(Clone, Debug)]
pub struct SoundItem {
    pub name: String,
    pub thm: SoundThm,
}

/// Every `.wav` of `rawdata\sounds` with its `.thm` (`GetSounds`).
#[must_use]
pub fn list_sounds(dirs: &EditorDirs, pool: &gz_task::Executors) -> Vec<SoundItem> {
    let files = list_files(&dirs.sounds, &["wav"], false, true);
    let dir = dirs.sounds.clone();
    pool.io_pool().install(|| {
        use rayon::prelude::*;
        files
            .into_par_iter()
            .map(|(name, _)| {
                let thm = std::fs::read(dir.join(format!("{name}.thm")))
                    .ok()
                    .and_then(|b| read_sound_thm(&b).ok())
                    .unwrap_or_default();
                SoundItem { name, thm }
            })
            .collect()
    })
}

/// The descriptions edited since the last save.
#[derive(Clone, Default)]
pub struct SoundEdits {
    pub edits: imbl::OrdMap<String, SoundThm>,
}

pub enum SoundCmd {
    Set { name: String, thm: SoundThm, gesture: String },
    Reset,
}

impl State for SoundEdits {
    type Cmd = SoundCmd;
    type Reply = ();

    fn apply(&mut self, cmd: SoundCmd, cx: &mut Apply) -> Result<(), CmdError> {
        match cmd {
            SoundCmd::Set { name, thm, gesture } => {
                self.edits.insert(name, thm);
                cx.undoable_merged("Sound Description", gesture);
            }
            SoundCmd::Reset => {
                self.edits = imbl::OrdMap::new();
                cx.new_document();
            }
        }
        Ok(())
    }
}

/// The selected sound's game file: its size, its length, its waveform and its samples.
pub struct GameSound {
    pub bytes: u64,
    pub ms: u32,
    pub peaks: Vec<Peak>,
    pub pcm: Option<Arc<Pcm>>,
}

/// `Max from Min` (`OnAttClick` 0): the distance where the sound falls to 1%, and a tenth more.
#[must_use]
pub fn max_from_min(min: f32) -> f32 {
    let d = min / (0.01 * ROLLOFF);
    d + 0.1 * d
}

/// `Min from Max` (`OnAttClick` 1).
#[must_use]
pub fn min_from_max(max: f32) -> f32 {
    ROLLOFF * (max - (0.1 / 1.1) * max) * 0.01
}

/// How loud the sound is at `d` metres: full up to the minimum distance, falling with the
/// rolloff after it.
#[must_use]
pub fn volume_at(min: f32, d: f32) -> f32 {
    (min / (ROLLOFF.max(1e-6) * d.max(1e-6))).clamp(0.0, 1.0)
}

/// The Sound Library tab.
pub struct SoundLibDoc {
    editor: Arc<Editor>,
    dirs: EditorDirs,
    store: Store<SoundEdits>,
    list: LibraryList,
    pub selected: String,
    pub auto_play: bool,
    read_only: bool,
    saving: Arc<std::sync::atomic::AtomicBool>,
    game: Option<(String, Pending<Option<GameSound>>)>,
    /// A rebuild ended: the preview reads the new file.
    rebuilt: Arc<std::sync::atomic::AtomicBool>,
}

impl SoundLibDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        let dirs = EditorDirs::of(editor);
        let read_only = !can_write(&dirs.sounds);
        if read_only {
            editor.log.warn("The sound folder cannot be written: sound descriptions are read-only.");
        }
        Self {
            editor: editor.clone(),
            store: Store::spawn(
                "sound library",
                SoundEdits::default(),
                &editor.exec,
                StoreOptions::default(),
            ),
            dirs,
            list: LibraryList::default(),
            selected: String::new(),
            // a sound plays on a click only; picking one stays silent unless Auto Play is on
            auto_play: read_only,
            read_only,
            saving: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            game: None,
            rebuilt: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn items(&self) -> Option<Arc<Vec<SoundItem>>> {
        let lists = LibraryLists::of(&self.editor);
        let (dirs, exec) = (self.dirs.clone(), self.editor.exec.clone());
        lists.sounds.get(lists.sounds_rev.get(), move || list_sounds(&dirs, &exec)).value().cloned()
    }

    /// The list, awaited: what a command reads.
    pub async fn items_now(editor: &Editor) -> Arc<Vec<SoundItem>> {
        let lists = LibraryLists::of(editor);
        let (dirs, exec) = (EditorDirs::of(editor), editor.exec.clone());
        lists.sounds.fresh(lists.sounds_rev.get(), move || list_sounds(&dirs, &exec)).await
    }

    fn saving(&self) -> bool {
        self.saving.load(std::sync::atomic::Ordering::Acquire)
    }

    fn thm_of(&self, item: &SoundItem) -> SoundThm {
        self.store.snapshot().edits.get(&item.name).copied().unwrap_or(item.thm)
    }

    fn ogg_path(&self, name: &str) -> PathBuf {
        self.dirs.game_sounds.join(format!("{name}.ogg"))
    }

    fn game_sound(&self) -> Option<Arc<Option<GameSound>>> {
        self.game.as_ref().filter(|(n, _)| *n == self.selected).and_then(|(_, p)| p.get())
    }

    /// Picks a sound: its game file is read and decoded on the pools; Auto Play plays it then.
    fn select(&mut self, name: Option<&str>) {
        Player::of(&self.editor).stop();
        let Some(name) = name else {
            self.selected.clear();
            self.game = None;
            return;
        };
        self.selected = name.to_owned();
        let path = self.ogg_path(name);
        let (exec, editor, auto) = (self.editor.exec.clone(), self.editor.clone(), self.auto_play);
        let pending = Pending::spawn(&self.editor.exec, async move {
            let got = exec
                .io(move || {
                    let bytes = std::fs::read(&path).ok()?;
                    Some(bytes)
                })
                .await?;
            let size = got.len() as u64;
            let sound = exec
                .cpu(move || {
                    let ms =
                        gz_fmt_sound::ogg_info(&got).map_or(0, |i| (i.seconds() * 1000.0).floor() as u32);
                    let pcm = decode_ogg(&got).ok().map(Arc::new);
                    let peaks = pcm.as_ref().map_or_else(Vec::new, |p| p.peaks(WAVE_BUCKETS));
                    GameSound { bytes: size, ms, peaks, pcm }
                })
                .await;
            if auto && let Some(pcm) = &sound.pcm {
                Player::of(&editor).play(pcm.clone());
            }
            Some(sound)
        });
        self.game = Some((name.to_owned(), pending));
    }

    /// Writes the changed descriptions (`UpdateLib`) as a job: each `.thm`, and the parameters
    /// of its game sound rewritten in place.
    fn queue_save(&mut self) -> Result<(), String> {
        if self.read_only {
            return Err("The sound folder cannot be written.".to_owned());
        }
        if self.saving() {
            return Ok(());
        }
        let edits = self.store.snapshot().edits.clone();
        let (dirs, saving, store, editor) =
            (self.dirs.clone(), self.saving.clone(), self.store.clone(), self.editor.clone());
        Player::of(&self.editor).stop();
        saving.store(true, std::sync::atomic::Ordering::Release);
        let started = self.editor.jobs.start(
            "sound_library",
            Some("libraries"),
            "Saving the Sound Library",
            move |ctx| async move {
                let exec = ctx.exec.clone();
                let c = ctx.clone();
                let result = exec
                    .io(move || -> Result<(usize, Vec<String>), String> {
                        let age = super::texsync::now();
                        let (mut done, mut notes) = (0usize, Vec::new());
                        for (i, (name, thm)) in edits.iter().enumerate() {
                            c.check().map_err(|_| "Stopped.".to_owned())?;
                            c.stage(format!("Saving {name}"));
                            c.progress(i, edits.len());
                            match save_sound(&dirs, name, thm, age) {
                                Ok(()) => done += 1,
                                Err(e) => notes.push(e),
                            }
                        }
                        Ok((done, notes))
                    })
                    .await;
                saving.store(false, std::sync::atomic::Ordering::Release);
                LibraryLists::of(&editor).sounds_rev.bump();
                match result {
                    Ok((done, notes)) => {
                        for n in &notes {
                            editor.log.warn(n.clone());
                        }
                        let _ = store.send(SoundCmd::Reset, Origin::Ui).await;
                        Ok(json!({ "written": done, "notes": notes }))
                    }
                    Err(e) => Err(gz_task::JobError::new(e)),
                }
            },
        );
        if let Err(busy) = started {
            self.saving.store(false, std::sync::atomic::Ordering::Release);
            return Err(format!("The save could not be started: {busy}."));
        }
        Ok(())
    }

    /// Makes the selected sound's game file again from its `.wav` (`RebuildGameSound`), as a job.
    fn queue_rebuild(&mut self) -> Result<(), String> {
        let items = self.items().unwrap_or_default();
        let Some(item) = items.iter().find(|i| i.name == self.selected) else {
            return Err("Pick a sound first.".to_owned());
        };
        let (name, thm) = (item.name.clone(), self.thm_of(item));
        let (dirs, saving, rebuilt, editor) =
            (self.dirs.clone(), self.saving.clone(), self.rebuilt.clone(), self.editor.clone());
        Player::of(&self.editor).stop();
        saving.store(true, std::sync::atomic::Ordering::Release);
        let started = self.editor.jobs.start(
            "sound_library",
            Some("libraries"),
            "Rebuilding the Game Sound",
            move |ctx| async move {
                editor.log.info(format!(
                    "Rebuilding the game sound {}",
                    dirs.game_sounds.join(format!("{name}.ogg")).display()
                ));
                ctx.stage(format!("Encoding {name}"));
                let d = dirs.clone();
                let result = ctx.exec.cpu(move || make_game_sound(&d, &name, &thm, None)).await;
                saving.store(false, std::sync::atomic::Ordering::Release);
                rebuilt.store(true, std::sync::atomic::Ordering::Release);
                editor.exec.wake().ping();
                result.map(|()| json!({ "rebuilt": true })).map_err(gz_task::JobError::new)
            },
        );
        if let Err(busy) = started {
            self.saving.store(false, std::sync::atomic::Ordering::Release);
            return Err(format!("The rebuild could not be started: {busy}."));
        }
        Ok(())
    }

    fn draw_preview(&mut self, ui: &mut Ui, item: &SoundItem) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let thm = self.thm_of(item);
        let src = self.dirs.sounds.join(format!("{}.wav", item.name)).to_string_lossy().into_owned();
        let game = self.game_sound();
        let facts = match game.as_deref() {
            Some(Some(g)) => {
                format!("{:.1} KB \u{b7} {:.2} s", g.bytes as f32 / 1024.0, g.ms as f32 / 1000.0)
            }
            _ => String::new(),
        };
        local_heading(
            ui,
            &crate::paths::label_of(&item.name),
            Clip::End,
            &[(&src, Clip::Start), (&facts, Clip::End)],
        );
        match game.as_deref() {
            Some(None) => {
                text::status(ui, Tone::Warning, "No game sound (.ogg) was built from this description yet.");
            }
            None => state::busy_line(ui, "Reading the game sound..."),
            Some(Some(g)) => {
                ui.add_space(8.0);
                // the waveform: every column the lowest and the highest sample of its stretch
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(4), p.inset);
                let mid = rect.center().y;
                let half = rect.height() * 0.5 - 2.0;
                let n = g.peaks.len().max(1);
                for (i, pk) in g.peaks.iter().enumerate() {
                    let x = rect.min.x + rect.width() * (i as f32 + 0.5) / n as f32;
                    let hi = mid - f32::from(pk.max) / 32768.0 * half;
                    let lo = mid - f32::from(pk.min) / 32768.0 * half;
                    ui.painter().line_segment(
                        [pos2(x, hi), pos2(x, lo.max(hi + 1.0))],
                        Stroke::new(1.0, p.accent_bright),
                    );
                }
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, p.border_subtle),
                    egui::StrokeKind::Inside,
                );
            }
        }
        ui.add_space(12.0);
        if local_section(ui, "sound.attenuation", "Attenuation", true) {
            let h = (ui.available_height() - 28.0).clamp(96.0, 240.0);
            draw_attenuation(ui, &thm, h);
        }
    }

    fn draw_properties(&mut self, ui: &mut Ui, item: &SoundItem) {
        let before = self.thm_of(item);
        let mut t = before;
        let ro = if self.read_only || self.saving() { PropFlags::READ_ONLY } else { PropFlags::NONE };
        let game = self.game_sound();
        let mut gesture = String::new();
        let mut clamped = false;
        PropertyGrid::new("sound_library").show(ui, |g| {
            real(
                g,
                "Quality",
                "The Vorbis quality the game sound is encoded with",
                ro,
                &mut t.quality,
                0.01,
                "",
                &mut gesture,
            );
            real(
                g,
                "Min Dist",
                "Up to this distance the sound plays at full volume",
                ro,
                &mut t.min_dist,
                0.1,
                "m",
                &mut gesture,
            );
            t.min_dist = t.min_dist.clamp(0.01, 1000.0);
            real(
                g,
                "Max Dist",
                "Past this distance the sound is silent",
                ro,
                &mut t.max_dist,
                0.1,
                "m",
                &mut gesture,
            );
            t.max_dist = t.max_dist.clamp(0.1, 1000.0);
            if t.max_ai_dist > t.max_dist {
                // 'Max AI Dist' <= 'Max Dist': it is clamped
                t.max_ai_dist = t.max_dist;
                clamped = true;
            }
            let mut ai = t.max_ai_dist;
            real(
                g,
                "Max AI Dist",
                "How far NPCs hear the sound; at most the maximum distance",
                ro,
                &mut ai,
                0.1,
                "m",
                &mut gesture,
            );
            if ai < t.max_dist {
                t.max_ai_dist = ai.max(0.1);
            }
            real(
                g,
                "Base Volume",
                "The volume the sound plays at, 0 to 2",
                ro,
                &mut t.base_volume,
                0.01,
                "",
                &mut gesture,
            );
            t.base_volume = t.base_volume.clamp(0.0, 2.0);
            g.row("Game Type", "What NPCs make of the sound", ro, 0, |ui| {
                let unknown = format!("{:#010x}", t.game_type);
                let mut names: Vec<&str> = GAME_TYPES.iter().map(|(_, n)| *n).collect();
                let mut cur = GAME_TYPES.iter().position(|(v, _)| *v == t.game_type).unwrap_or_else(|| {
                    names.push(&unknown);
                    names.len() - 1
                });
                if ro.contains(PropFlags::READ_ONLY) {
                    read_only(ui, names.get(cur).copied().unwrap_or(""), Clip::End);
                } else if combo(ui, "game_type", &mut cur, &names)
                    && let Some((v, _)) = GAME_TYPES.get(cur)
                {
                    t.game_type = *v;
                    gesture = "game_type".to_owned();
                }
            });
            g.section("Attenuation", true, None, |g| {
                // one button a row: two side by side do not fit the pane's 220 pt
                let on = !ro.contains(PropFlags::READ_ONLY);
                g.row(
                    "Max Dist",
                    "The distance where the sound falls to 1%, from the minimum, with the game's rolloff",
                    ro,
                    1,
                    |ui| {
                        if ui
                            .add(
                                Button::new("Max from Min").enabled(on, "The sound folder cannot be written"),
                            )
                            .clicked()
                        {
                            t.max_dist = max_from_min(t.min_dist);
                            gesture = "auto".to_owned();
                        }
                    },
                );
                g.row(
                    "Min Dist",
                    "The minimum distance worked out from the maximum, with the game's rolloff",
                    ro,
                    1,
                    |ui| {
                        if ui
                            .add(
                                Button::new("Min from Max").enabled(on, "The sound folder cannot be written"),
                            )
                            .clicked()
                        {
                            t.min_dist = min_from_max(t.max_dist);
                            gesture = "auto".to_owned();
                        }
                    },
                );
            });
            if let Some(Some(gs)) = game.as_deref() {
                g.section("Game Sound", true, None, |g| {
                    g.row("File Size", "", PropFlags::READ_ONLY, 0, |ui| {
                        read_only(ui, &format!("{:.2} KB", gs.bytes as f32 / 1024.0), Clip::End)
                    });
                    g.row("Length", "", PropFlags::READ_ONLY, 0, |ui| {
                        read_only(ui, &format!("{:.2} s", gs.ms as f32 / 1000.0), Clip::End)
                    });
                });
            }
        });
        if clamped {
            gz_ui::state::toast(
                ui.ctx(),
                Tone::Info,
                "'Max AI Dist' <= 'Max Dist'. 'Max AI Dist' will be clamped.",
            );
        }
        if t != before && ro == PropFlags::NONE {
            let gesture = format!("{}:{gesture}", item.name);
            self.store
                .post(SoundCmd::Set { name: item.name.clone(), thm: t, gesture }, self.editor.ui_origin());
        }
    }

    fn state(&self, items: &[SoundItem]) -> Map<String, Value> {
        let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        let rows = shown(&keys, &|_| true, &self.list.search);
        let mut out = Map::new();
        out.insert("shown".into(), json!(rows.len()));
        out.insert("first".into(), json!(rows.first().and_then(|&i| keys.get(i)).copied().unwrap_or("")));
        out.insert("items".into(), json!(items.len()));
        out.insert("read_only".into(), json!(self.read_only));
        out.insert("saving".into(), json!(self.saving()));
        out.insert("dirty".into(), json!(self.store.is_dirty()));
        out.insert("auto_play".into(), json!(self.auto_play));
        out.insert("selected".into(), json!(self.selected));
        out.insert("folder".into(), json!(crate::paths::json_path(&self.dirs.sounds)));
        if let Some(item) = items.iter().find(|i| i.name == self.selected) {
            let t = self.thm_of(item);
            let game = self.game_sound();
            let g = game.as_deref().and_then(Option::as_ref);
            out.insert("min_dist".into(), json!(t.min_dist));
            out.insert("max_dist".into(), json!(t.max_dist));
            out.insert("game_sound".into(), json!(g.is_some()));
            out.insert("bytes".into(), json!(g.map_or(0, |g| g.bytes)));
            out.insert("ms".into(), json!(g.map_or(0, |g| g.ms)));
        }
        out
    }

    pub fn mcp_with(
        &mut self,
        action: &str,
        args: &Args,
        items: &[SoundItem],
    ) -> Result<Map<String, Value>, String> {
        if let Some(s) = args.str("search") {
            self.list.search = s.to_owned();
        }
        if action == "select" {
            let item = args.str("item").unwrap_or_default();
            let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
            let Some(at) = find(&keys, item) else {
                return Err(format!("no sound is named or matches '{item}'"));
            };
            let name = keys.get(at).copied().unwrap_or_default().to_owned();
            self.select(Some(&name));
        }
        Ok(self.state(items))
    }
}

/// Makes the game sound of `name` from its `.wav` (`MakeGameSound`): Vorbis at the description's
/// quality, its parameters as the first comment. A sound that cannot be made leaves no file.
pub fn make_game_sound(
    dirs: &EditorDirs,
    name: &str,
    thm: &SoundThm,
    age: Option<i64>,
) -> Result<(), String> {
    let wav_path = dirs.sounds.join(format!("{name}.wav"));
    let ogg_path = dirs.game_sounds.join(format!("{name}.ogg"));
    let failed = || {
        let _ = std::fs::remove_file(&ogg_path);
        format!("Can't make game sound '{}'.", ogg_path.display())
    };
    let made = std::fs::read(&wav_path)
        .map_err(|e| e.to_string())
        .and_then(|b| read_wav(&b).map_err(|e| e.to_string()))
        .and_then(|wav| {
            encode_game_sound(&wav, thm.quality, &OggParams::from(thm)).map_err(|e| e.to_string())
        });
    let ogg = match made {
        Ok(ogg) => ogg,
        Err(e) => return Err(format!("{} {e}", failed())),
    };
    if let Some(dir) = ogg_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = gz_editor::fsx::write_atomic(&ogg_path, &ogg) {
        return Err(format!("{} {e}", failed()));
    }
    if let Some(age) = age {
        let _ = set_file_age(&ogg_path, age);
    }
    Ok(())
}

/// Writes one description and makes its game sound again (`UpdateLib`: `SynchronizeSounds`
/// with the game sounds forced), every file of it at the same age.
fn save_sound(dirs: &EditorDirs, name: &str, thm: &SoundThm, age: i64) -> Result<(), String> {
    let thm_path = dirs.sounds.join(format!("{name}.thm"));
    gz_editor::fsx::write_atomic(&thm_path, &write_sound_thm(thm))
        .map_err(|e| format!("{} could not be written: {e}", thm_path.display()))?;
    let _ = set_file_age(&thm_path, age);
    make_game_sound(dirs, name, thm, Some(age))?;
    let wav = dirs.sounds.join(format!("{name}.wav"));
    let _ = set_file_age(&wav, age);
    Ok(())
}

/// What a sound synchronisation would take away (`OrphanSounds`): game sounds and descriptions
/// whose `.wav` is gone; up to ten named, and how many there are.
#[must_use]
pub fn orphans(dirs: &EditorDirs) -> (Vec<String>, usize) {
    let base: rustc_hash::FxHashSet<String> =
        list_files(&dirs.sounds, &["wav"], false, true).into_iter().map(|(n, _)| n).collect();
    let mut items = Vec::new();
    let mut total = 0;
    for (dir, ext) in [(&dirs.game_sounds, "ogg"), (&dirs.sounds, "thm")] {
        for (n, _) in list_files(dir, &[ext], false, true) {
            if base.contains(&n) {
                continue;
            }
            if items.len() < 10 {
                items.push(format!("{n}.{ext}"));
            }
            total += 1;
        }
    }
    (items, total)
}

/// `SynchronizeSounds` + `CleanupSounds`: a description for every `.wav` that has none or one of
/// another age, its game sound made again with it and wherever it is missing or of another age
/// than its `.wav`; the orphans renamed to `.~ogg` / `.~thm`.
pub fn synchronize_sounds(
    dirs: &EditorDirs,
    cancel: &gz_task::Cancel,
) -> Result<(usize, Vec<String>), String> {
    let base = list_files(&dirs.sounds, &["wav"], false, true);
    let age = super::texsync::now();
    let mut updated = 0;
    let mut notes = Vec::new();
    for (name, path) in &base {
        cancel.check().map_err(|_| "Stopped.".to_owned())?;
        let base_age = file_age(path);
        let thm_path = dirs.sounds.join(format!("{name}.thm"));
        let thm_age = file_age(&thm_path);
        if thm_age.is_none() || thm_age != base_age {
            let thm = std::fs::read(&thm_path).ok().and_then(|b| read_sound_thm(&b).ok()).unwrap_or_default();
            gz_editor::fsx::write_atomic(&thm_path, &write_sound_thm(&thm))
                .map_err(|e| format!("{} could not be written: {e}", thm_path.display()))?;
            let _ = set_file_age(&thm_path, age);
            let _ = set_file_age(path, age);
            updated += 1;
        }
        // the game sound is made again when the description was, or when it is missing or of
        // another age than its source
        let ogg_age = file_age(&dirs.game_sounds.join(format!("{name}.ogg")));
        let thm_new = thm_age.is_none() || thm_age != base_age;
        if thm_new || ogg_age.is_none() || ogg_age != base_age {
            let thm = std::fs::read(&thm_path).ok().and_then(|b| read_sound_thm(&b).ok()).unwrap_or_default();
            match make_game_sound(dirs, name, &thm, Some(age)) {
                Ok(()) => {
                    let _ = set_file_age(&thm_path, age);
                    let _ = set_file_age(path, age);
                    if !thm_new {
                        updated += 1;
                    }
                }
                Err(e) => notes.push(e),
            }
        }
    }
    let wavs: rustc_hash::FxHashSet<&str> = base.iter().map(|(n, _)| n.as_str()).collect();
    for (dir, ext) in [(&dirs.game_sounds, "ogg"), (&dirs.sounds, "thm")] {
        for (n, path) in list_files(dir, &[ext], false, true) {
            cancel.check().map_err(|_| "Stopped.".to_owned())?;
            if wavs.contains(n.as_str()) {
                continue;
            }
            // EFS.MarkFile(fn, true): renamed, never deleted
            let marked = path.with_extension(format!("~{ext}"));
            if let Err(e) = std::fs::rename(&path, &marked) {
                notes.push(format!("{} could not be renamed: {e}", path.display()));
            }
        }
    }
    Ok((updated, notes))
}

/// How loud the sound is with distance: the curve the old property canvas plotted.
fn draw_attenuation(ui: &mut Ui, thm: &SoundThm, height: f32) {
    let p = palette(ui.ctx());
    let w = ui.available_width().max(64.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), p.inset);
    let max_d = thm.max_dist.max(1e-3);
    let points: Vec<Pos2> = (0..=CURVE_POINTS)
        .map(|i| {
            let d = (max_d * i as f32 / CURVE_POINTS as f32).max(1e-3);
            pos2(
                rect.min.x + w * i as f32 / CURVE_POINTS as f32,
                rect.max.y - volume_at(thm.min_dist, d) * height,
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(2.0, p.accent_bright)));
    let min_x = rect.min.x + w * (thm.min_dist / max_d).clamp(0.0, 1.0);
    painter.line_segment([pos2(min_x, rect.min.y), pos2(min_x, rect.max.y)], Stroke::new(1.0, p.border));
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, p.border_subtle),
        egui::StrokeKind::Inside,
    );
    resp.on_hover_text("Volume over distance: full up to the minimum distance, silent past the maximum.");
    local_ends(ui, &format!("Min {:.1} m", thm.min_dist), &format!("Max {:.1} m", thm.max_dist));
}

#[allow(clippy::too_many_arguments)]
fn real(
    g: &mut Grid<'_>,
    label: &str,
    help: &str,
    flags: PropFlags,
    value: &mut f32,
    speed: f32,
    unit: &str,
    gesture: &mut String,
) {
    g.row(label, help, flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, &format!("{value:.2}"), Clip::End);
            return;
        }
        let before = *value;
        number(ui, value, speed, unit);
        if *value != before {
            *gesture = label.to_owned();
        }
    });
}

impl LibraryDoc for SoundLibDoc {
    fn mcp(&mut self, action: &str, args: &Args) -> Result<Map<String, Value>, String> {
        let items = self.items().unwrap_or_default();
        self.mcp_with(action, args, &items)
    }

    fn settle(&self) -> Option<Settle> {
        let (_, p) = self.game.as_ref()?;
        let p = p.clone();
        Some(Box::pin(async move {
            let _ = p.wait(std::time::Duration::from_secs(10)).await;
        }))
    }
}

impl Document for SoundLibDoc {
    fn id(&self) -> &str {
        KIND
    }
    fn kind(&self) -> &'static str {
        KIND
    }
    fn label(&self) -> String {
        "Sound Library".to_owned()
    }
    fn icon(&self) -> Icon {
        Icon::FileSound
    }
    fn tooltip(&self) -> Option<String> {
        Some("The sound descriptions of the editor's library".to_owned())
    }
    fn dirty(&self) -> bool {
        self.store.is_dirty()
    }
    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let dirty = self.store.is_dirty();
        let on = !self.read_only && !self.saving() && dirty;
        let why = if self.read_only {
            "The sound folder cannot be written"
        } else if self.saving() {
            "The library is saving"
        } else {
            "No unsaved changes"
        };
        tb.group(2);
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            if on { "Save the changed descriptions and make their game sounds again" } else { why },
            on,
            false,
            Kind::Ghost,
        ) && let Err(e) = self.queue_save()
        {
            let asker =
                if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
            if let Some(line) = gz_ui::dialog::notice(cx.ctx, asker, Tone::Error, "Save failed", &e) {
                self.editor.log.plain(line);
            }
        }
        tb.group(3);
        let game = self.game_sound();
        let pcm = game.as_deref().and_then(Option::as_ref).and_then(|g| g.pcm.clone());
        let one = !self.selected.is_empty();
        let no_sound =
            if one { "This description has no game sound (.ogg) yet" } else { "Pick a sound first" };
        let player = Player::of(&self.editor);
        let play_tip = player.error().unwrap_or_else(|| "Play the game sound".to_owned());
        if tb.action(
            "play",
            Icon::Play,
            "Play",
            if pcm.is_some() { &play_tip } else { no_sound },
            pcm.is_some(),
            false,
            Kind::Ghost,
        ) && let Some(pcm) = pcm
        {
            player.play(pcm);
        }
        // Stop is there to stop something: off while nothing plays
        let playing = player.is_playing();
        if tb.action(
            "stop",
            Icon::Stop,
            "Stop",
            if playing { "Stop the sound" } else { "Nothing is playing" },
            playing,
            false,
            Kind::Ghost,
        ) {
            player.stop();
        }
        if tb.action(
            "auto_play",
            Icon::SoundSource,
            "Auto Play",
            "Play every sound as it is picked in the list",
            true,
            self.auto_play,
            Kind::Ghost,
        ) {
            self.auto_play = !self.auto_play;
        }
        tb.group(1);
        let rebuild = !self.read_only && !self.saving() && one;
        let rebuild_tip = if rebuild {
            "Build the game sound (.ogg) of this description again"
        } else if self.read_only {
            "The sound folder cannot be written"
        } else if self.saving() {
            "The library is saving"
        } else {
            "Pick a sound first"
        };
        if tb.action("rebuild", Icon::Refresh, "Rebuild", rebuild_tip, rebuild, false, Kind::Ghost)
            && let Err(e) = self.queue_rebuild()
        {
            let asker =
                if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
            if let Some(line) = gz_ui::dialog::notice(cx.ctx, asker, Tone::Error, "Rebuild failed", &e) {
                self.editor.log.plain(line);
            }
        }
        if self.rebuilt.swap(false, std::sync::atomic::Ordering::AcqRel) && !self.selected.is_empty() {
            // the preview follows the new file
            let name = self.selected.clone();
            self.select(Some(&name));
        }
        tb.group(0);
        if self.read_only {
            tb.status("Read-only", Tone::Warning);
        } else if self.saving() {
            tb.status("Saving...", Tone::Info);
        }
        if playing {
            cx.ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let items = self.items();
        let all = items.clone().unwrap_or_default();
        let [list_r, preview_r, details_r] = panes(ui, KIND);
        let keys: Vec<&str> = all.iter().map(|i| i.name.as_str()).collect();
        let rows = shown(&keys, &|_| true, &self.list.search);
        // only an empty list says where the library is read from
        let empty_text = if items.is_some() && all.is_empty() {
            format!(
                "The library lists the .wav files in {}. A sound copied there shows up the next time the library opens.",
                folder_text(&self.editor, &self.dirs.sounds)
            )
        } else {
            String::new()
        };
        let empty = Empty {
            title: if items.is_some() { "No sounds" } else { "Reading the library..." },
            text: &empty_text,
            action: None,
        };
        let selected = self.selected.clone();
        let out = pane(ui, list_r, "sound_library.list.pane", false, false, |ui| {
            self.list.draw(ui, KIND, &keys, &rows, &selected, &empty)
        });
        if let Some(i) = out.picked
            && let Some(name) = keys.get(i)
        {
            let name = (*name).to_owned();
            self.select(Some(&name));
        }
        let current = all.iter().find(|i| i.name == self.selected).cloned();
        pane(ui, preview_r, "sound_library.preview", true, true, |ui| match &current {
            Some(item) => self.draw_preview(ui, item),
            None => {
                state::empty_state(
                    ui,
                    Icon::FileSound,
                    "No sound selected",
                    "Pick a sound in the list to hear it and see how far it carries.",
                    None,
                );
            }
        });
        pane(ui, details_r, "sound_library.details", false, true, |ui| match &current {
            Some(item) => self.draw_properties(ui, item),
            None => {
                state::empty_state(
                    ui,
                    Icon::Settings,
                    "No properties",
                    "Pick a sound to edit its description.",
                    None,
                );
            }
        });
        ui.allocate_rect(ui.max_rect(), Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let items = self.items().unwrap_or_default();
        let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        let rows = shown(&keys, &|_| true, &self.list.search);
        let changed = self.store.snapshot().edits.len();
        let mut line = format!("{} sounds", items.len());
        if rows.len() != items.len() {
            line.push_str(&format!(" \u{b7} {} shown", rows.len()));
        }
        if changed > 0 {
            line.push_str(&format!(" \u{b7} {changed} changed"));
        }
        let p = palette(ui.ctx());
        text::label(ui, &line, Font::Small, p.text_secondary, Clip::End);
    }

    fn edit(&mut self, _cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        let h = self.store.history();
        let there = match action {
            EditAction::Undo => !h.undo.is_empty(),
            EditAction::Redo => !h.redo.is_empty(),
            _ => return false,
        };
        if run && there {
            let store = self.store.clone();
            self.editor.exec.spawn(async move {
                match action {
                    EditAction::Undo => store.undo().await,
                    _ => store.redo().await,
                };
            });
        }
        there
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.queue_save()
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        self.store.post(SoundCmd::Reset, Origin::Ui);
    }

    fn on_deactivate(&mut self, _cx: &mut DocCx<'_>) {
        Player::of(&self.editor).stop();
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        Player::of(&self.editor).stop();
    }

    fn mcp_state(&self) -> Value {
        Value::Object(self.state(&self.items().unwrap_or_default()))
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `sound_library` factory.
pub fn factory(editor: &Arc<Editor>, _req: &OpenRequest) -> Result<Box<dyn Document>, String> {
    Ok(Box::new(SoundLibDoc::new(editor)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation() {
        assert_eq!(volume_at(2.0, 1.0), 1.0);
        assert!((volume_at(1.0, 10.0) - 1.0 / 7.5).abs() < 1e-6);
        let max = max_from_min(1.0);
        assert!((max - 1.0 / 0.0075 * 1.1).abs() < 1e-3);
        assert!(min_from_max(110.0) > 0.0);
        assert_eq!(GAME_TYPES.len(), 21);
    }
}
