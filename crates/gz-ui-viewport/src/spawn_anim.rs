//! A spawn's model posed by its startup animation: the first frame,
//! paused, until the Details row `Model\Animation` (First Frame, Play, Pause, Stop, Last Frame)
//! or `viewport.play what=animation` says otherwise. Nothing of it is saved; the scene never changes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use glam::Mat4;
use gz_editor::{CmdError, Editor, Reply, json};
use gz_fmt_ogf::{MotionSet, Visual, read_omf};
use gz_render::{AssetLoader, AssetRef};
use gz_scene::model::ObjData;
use gz_scene::{ObjId, SceneService, SceneState};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::loader::SceneLoader;

/// Last Frame stops this far before a cycle's end.
const LAST_FRAME_BACK: f32 = 0.4444;
/// Paused poses kept before the cache starts again.
const POSE_CACHE: usize = 4096;

/// What a spawn's model does now. No entry is the first frame, paused.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Playing {
        since: Instant,
        from: f32,
    },
    Paused(f32),
    /// Every cycle closed: the model at rest.
    Stopped,
}

#[derive(Clone, Debug)]
struct Play {
    /// The visual and animation it was started for: another one starts at its first frame.
    of: (SmolStr, SmolStr),
    mode: Mode,
}

/// A skeleton and every motion it can play.
pub(crate) struct Rig {
    visual: Visual,
    sets: Vec<MotionSet>,
}

/// A cycle found by name: its set, its motion, its length (s) and speed.
#[derive(Clone, Copy)]
struct Cycle {
    set: usize,
    motion: usize,
    length: f32,
    speed: f32,
}

impl Rig {
    /// The cycle of that name that is no effect, in the order the sets load.
    fn cycle(&self, name: &str) -> Option<Cycle> {
        self.sets.iter().enumerate().find_map(|(set, s)| {
            let d = s.defs.iter().find(|d| d.flags & 1 == 0 && d.name.eq_ignore_ascii_case(name))?;
            let motion = usize::from(d.motion);
            let m = s.motions.get(motion)?;
            let length = m.length as f32 / gz_fmt_ogf::SAMPLE_FPS;
            Some(Cycle { set, motion, length, speed: if d.speed > 0.0 { d.speed } else { 1.0 } })
        })
    }

    fn pose(&self, c: Cycle, t: f32) -> Option<Arc<[Mat4]>> {
        gz_render::visual_pose(&self.visual, self.sets.get(c.set)?, c.motion, t)
    }
}

enum Slot {
    Loading,
    Ready(Option<Arc<Rig>>),
}

/// A paused pose: the visual, the animation and the time's bits.
type PausedKey = (SmolStr, SmolStr, u32);

/// The spawns' playback: an `Editor` service.
#[derive(Default)]
pub struct SpawnAnims {
    loader: Mutex<Option<Arc<SceneLoader>>>,
    rigs: Mutex<FxHashMap<SmolStr, Slot>>,
    plays: Mutex<FxHashMap<ObjId, Play>>,
    paused: Mutex<FxHashMap<PausedKey, Arc<[Mat4]>>>,
    /// Bumped when a rig arrives or a playback changes: the viewport draws again.
    rev: AtomicU64,
}

/// The poses of one pass, by spawn, and whether one of them plays.
#[derive(Default)]
pub(crate) struct Poses {
    pub by_object: FxHashMap<ObjId, Arc<[Mat4]>>,
    pub playing: bool,
}

/// The visual and startup animation of a spawn that has a model.
fn model_of(o: &gz_scene::Object) -> Option<(SmolStr, SmolStr)> {
    let ObjData::Spawn(d) = &o.data else { return None };
    let e = d.entity.as_ref()?.entity()?;
    let visual = e.visual_name().filter(|v| !v.is_empty())?;
    Some((SmolStr::new(visual.to_ascii_lowercase()), SmolStr::new(e.startup_animation().unwrap_or(""))))
}

impl SpawnAnims {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<SpawnAnims> {
        editor.services.get_or_init(SpawnAnims::default)
    }

    /// The loader of the open project and linked game: every rig is read again through it.
    pub(crate) fn set_loader(&self, loader: Arc<SceneLoader>) {
        *self.loader.lock() = Some(loader);
        self.rigs.lock().clear();
        self.paused.lock().clear();
        self.rev.fetch_add(1, Ordering::AcqRel);
    }

    #[must_use]
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::Acquire)
    }

    /// The rig of `visual`, `None` while it loads (the load is started then) or when the model
    /// has no skeleton.
    fn rig(self: &Arc<Self>, editor: &Arc<Editor>, visual: &SmolStr) -> Option<Arc<Rig>> {
        let mut rigs = self.rigs.lock();
        match rigs.get(visual) {
            Some(Slot::Ready(r)) => return r.clone(),
            Some(Slot::Loading) => return None,
            None => {}
        }
        let loader = self.loader.lock().clone()?;
        rigs.insert(visual.clone(), Slot::Loading);
        drop(rigs);
        let (me, ed, name) = (self.clone(), editor.clone(), visual.clone());
        editor.exec.spawn_io(move || {
            let rig = load_rig(&loader, &name).map(Arc::new);
            me.rigs.lock().insert(name, Slot::Ready(rig));
            me.rev.fetch_add(1, Ordering::AcqRel);
            ed.exec.wake().ping();
        });
        None
    }

    /// Every visible spawn's pose for one pass. Runs on the CPU pool.
    pub(crate) fn poses(self: &Arc<Self>, editor: &Arc<Editor>, state: &SceneState) -> Poses {
        let mut out = Poses::default();
        let now = Instant::now();
        let plays = self.plays.lock().clone();
        for o in state.objects.values() {
            if !(o.flags.visible && state.tools.part(o.class).visible) {
                continue;
            }
            let Some(of) = model_of(o) else { continue };
            let Some(rig) = self.rig(editor, &of.0) else { continue };
            let mode = plays.get(&o.id).filter(|p| p.of == of).map_or(Mode::Paused(0.0), |p| p.mode);
            let Some(c) = rig.cycle(&of.1) else { continue };
            let pose = match mode {
                Mode::Stopped => continue,
                Mode::Playing { since, from } => {
                    out.playing = true;
                    rig.pose(c, wrap(from + now.duration_since(since).as_secs_f32() * c.speed, c.length))
                }
                Mode::Paused(t) => {
                    let key = (of.0.clone(), of.1.clone(), t.to_bits());
                    if let Some(p) = self.paused.lock().get(&key) {
                        out.by_object.insert(o.id, p.clone());
                        continue;
                    }
                    let p = rig.pose(c, t);
                    if let Some(p) = &p {
                        let mut cache = self.paused.lock();
                        if cache.len() >= POSE_CACHE {
                            cache.clear();
                        }
                        cache.insert(key, p.clone());
                    }
                    p
                }
            };
            if let Some(p) = pose {
                out.by_object.insert(o.id, p);
            }
        }
        out
    }

    /// A press of one of the row's buttons, by its caption or action.
    fn press(
        &self,
        id: ObjId,
        of: (SmolStr, SmolStr),
        rig: Option<&Rig>,
        action: &str,
    ) -> Result<(), CmdError> {
        let mut plays = self.plays.lock();
        let now = Instant::now();
        let current = plays.get(&id).filter(|p| p.of == of).map_or(Mode::Paused(0.0), |p| p.mode);
        let cycle = rig.and_then(|r| r.cycle(&of.1));
        let time_of = |m: Mode| match (m, cycle) {
            (Mode::Playing { since, from }, Some(c)) => {
                wrap(from + now.duration_since(since).as_secs_f32() * c.speed, c.length)
            }
            (Mode::Paused(t), _) => t,
            _ => 0.0,
        };
        let mode = match action {
            "first" | "first frame" => Mode::Paused(0.0),
            "play" => Mode::Playing { since: now, from: 0.0 },
            // Pause pauses a playing cycle and goes on with a paused one
            "pause" => match current {
                Mode::Playing { .. } => Mode::Paused(time_of(current)),
                Mode::Paused(t) => Mode::Playing { since: now, from: t },
                Mode::Stopped => Mode::Stopped,
            },
            "stop" => Mode::Stopped,
            "last" | "last frame" => {
                Mode::Paused(cycle.map_or(0.0, |c| (c.length - LAST_FRAME_BACK).max(0.0)))
            }
            other => {
                return Err(CmdError::invalid(format!(
                    "unknown action '{other}' (first, play, pause, stop, last or state)"
                )));
            }
        };
        plays.insert(id, Play { of, mode });
        drop(plays);
        self.rev.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn describe(
        &self,
        id: ObjId,
        of: &(SmolStr, SmolStr),
        rig: Option<&Rig>,
        loaded: bool,
    ) -> serde_json::Value {
        let now = Instant::now();
        let mode = self.plays.lock().get(&id).filter(|p| &p.of == of).map_or(Mode::Paused(0.0), |p| p.mode);
        let cycle = rig.and_then(|r| r.cycle(&of.1));
        let (state, time) = match (mode, cycle) {
            (Mode::Playing { since, from }, Some(c)) => {
                ("playing", wrap(from + now.duration_since(since).as_secs_f32() * c.speed, c.length))
            }
            (Mode::Playing { .. }, None) => ("playing", 0.0),
            (Mode::Paused(t), _) => ("paused", t),
            (Mode::Stopped, _) => ("stopped", 0.0),
        };
        json!({
            "visual": of.0,
            "animation": of.1,
            "loaded": loaded,
            "skeleton": rig.is_some(),
            "found": cycle.is_some(),
            "state": state,
            "time": (f64::from(time) * 1000.0).round() / 1000.0,
            "length": cycle.map_or(0.0, |c| (f64::from(c.length) * 1000.0).round() / 1000.0),
        })
    }
}

/// A looping cycle's time.
fn wrap(t: f32, length: f32) -> f32 {
    if length > 0.0 { t.rem_euclid(length) } else { 0.0 }
}

fn load_rig(loader: &SceneLoader, name: &str) -> Option<Rig> {
    let bytes = loader.load(AssetRef::Visual(name)).ok()?.bytes;
    let mut visual = Visual::read(&bytes).ok()?;
    if visual.bones.is_empty() {
        return None;
    }
    let mut sets = Vec::new();
    if let Some(m) = visual.motions.take()
        && !m.motions.is_empty()
    {
        sets.push(m);
    }
    for r in &visual.motion_refs {
        for part in r.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if let Some(b) = loader.motions(part)
                && let Ok(set) = read_omf(&b)
                && !set.motions.is_empty()
            {
                sets.push(set);
            }
        }
    }
    // only the skeleton is posed: the meshes are the renderer's
    visual.vertices = None;
    visual.indices = None;
    visual.children.clear();
    visual.collision_vertices = None;
    visual.collision_indices = None;
    Some(Rig { visual, sets })
}

/// `viewport.play what=animation`: a spawn's startup animation played, paused, stopped or put at
/// a frame (`action`), and its state.
pub(crate) async fn animation_command(editor: &Arc<Editor>, name: &str, action: &str) -> Reply {
    let editor = editor.clone();
    let svc = SceneService::of(&editor)?;
    let snap = svc.store.snapshot();
    let o = snap
        .find_relaxed(name)
        .ok_or_else(|| CmdError::not_found(format!("there is no object '{name}'")))?
        .clone();
    let of =
        model_of(&o).ok_or_else(|| CmdError::invalid(format!("'{name}' is no spawn point with a model")))?;
    let anims = SpawnAnims::of(&editor);
    // the rig as the viewport has it, read now when no pass asked for it yet
    let rig = {
        let ready = match anims.rigs.lock().get(&of.0) {
            Some(Slot::Ready(r)) => Some(r.clone()),
            _ => None,
        };
        match ready {
            Some(r) => r,
            None => {
                let loader = anims.loader.lock().clone();
                let name = of.0.clone();
                let rig = match loader {
                    Some(l) => editor.exec.io(move || load_rig(&l, &name).map(Arc::new)).await,
                    None => None,
                };
                if anims.loader.lock().is_some() {
                    anims.rigs.lock().insert(of.0.clone(), Slot::Ready(rig.clone()));
                }
                rig
            }
        }
    };
    let loaded = anims.loader.lock().is_some();
    if action != "state" {
        anims.press(o.id, of.clone(), rig.as_deref(), action)?;
        crate::service::Viewport::of(&editor).touch();
        editor.exec.wake().ping();
    }
    let mut out = anims.describe(o.id, &of, rig.as_deref(), loaded);
    if let Some(m) = out.as_object_mut() {
        m.insert("object".into(), json!(o.name.as_str()));
    }
    Ok(out)
}
