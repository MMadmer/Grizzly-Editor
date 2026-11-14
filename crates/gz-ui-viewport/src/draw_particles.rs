//! The scene's particle systems playing in the viewport (`viewport.play what=particles`): a
//! system per particle object made from
//! the linked game's `particles.xr` the first time it is played, stepped on the CPU pool as part
//! of a scene pass - one task per system, only while it plays and its emitter is in view - and
//! drawn as sprite batches. `viewport.play what=particles` plays and stops them.

use std::sync::Arc;
use std::time::Instant;

use glam::{Mat4, Vec3};
use gz_base::Rev;
use gz_editor::{CmdError, Editor, Reply, SmolStr, Value, json};
use gz_particles::{Align, Collider, Cull, Library, System};
use gz_render::{
    Aabb, Assets, Color, DrawList, Frustum, InstanceFlags, MaterialDesc, Sprite, SpriteAlign, SpriteBatch,
    SpriteCull, SpriteSize,
};
use gz_scene::objects::PSOBJECT_SIZE;
use gz_scene::pick::{ScenePick, ray_scene_objects};
use gz_scene::{ObjClass, ObjData, ObjId, SceneService, SceneState};
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::draw::gm;

/// A particle object's system.
struct Entry {
    reference: SmolStr,
    /// None when the library has no effect or group of that name (`Compile` failed), and while a
    /// pass steps it outside the lock (`out`).
    system: Option<System>,
    /// What the system was last told it stands at.
    transform: Mat4,
    /// A pass took the system out to step it: what `status` answers meanwhile (playing,
    /// particles, group).
    out: Option<(bool, usize, bool)>,
}

/// The library of the linked game, per link revision.
#[derive(Default)]
struct LibSlot {
    /// The link revision read last; a read that failed is not tried again for it.
    read: Option<Option<Rev>>,
    library: Option<Arc<Library>>,
    error: Option<String>,
    /// The last note logged: a refresh of the same game reads the same file again and must not
    /// say it once more.
    noted: Option<String>,
}

#[derive(Default)]
pub(crate) struct State {
    /// The library the entries were made from.
    library: Option<Arc<Library>>,
    entries: FxHashMap<ObjId, Entry>,
    /// When the last pass stepped the systems.
    last: Option<Instant>,
}

/// The viewport's particle systems: an `Editor` service.
#[derive(Default)]
pub struct Particles {
    lib: Mutex<LibSlot>,
    /// Held while the library is read: callers that ask meanwhile wait for that read.
    reading: tokio::sync::Mutex<()>,
    pub(crate) state: Mutex<State>,
}

impl Particles {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Particles> {
        editor.services.get_or_init(Particles::default)
    }

    /// `particles.xr` of the linked game (or the editor's own), read and parsed off the UI
    /// thread once per link revision.
    pub async fn library(&self, editor: &Editor) -> Option<Arc<Library>> {
        let rev = gz_game::GameLink::of(editor).map(|g| g.rev()).ok();
        let current = || {
            let slot = self.lib.lock();
            (slot.read == Some(rev)).then(|| slot.library.clone())
        };
        if let Some(l) = current() {
            return l;
        }
        let _reading = self.reading.lock().await;
        if let Some(l) = current() {
            return l;
        }
        let parse = |bytes: Vec<u8>| async move {
            editor
                .exec
                .cpu(move || {
                    gz_fmt_xr::Particles::read(&bytes)
                        .map(|p| Arc::new(Library::new(&p)))
                        .map_err(|e| format!("particles.xr could not be read: {e}"))
                })
                .await
        };
        let mut read: Result<Arc<Library>, String> =
            match gz_game::read_game_or_bundled(editor, "particles.xr").await {
                Ok((bytes, _)) => parse(bytes).await,
                Err(e) => Err(e.to_string()),
            };
        // a file the reader refuses falls back to the editor's own copy, said in the log
        let mut note = None;
        if let Err(e) = &read
            && let Some(bytes) = gz_game::Bundled::of(editor).read("particles.xr").await
            && let Ok(l) = parse(bytes).await
        {
            note = Some(format!(
                "the linked game's {e}; the particle systems play from the editor's own particles.xr"
            ));
            read = Ok(l);
        }
        let mut slot = self.lib.lock();
        slot.read = Some(rev);
        let said = match read {
            Ok(l) => {
                slot.library = Some(l);
                slot.error.clone_from(&note);
                note
            }
            Err(e) => {
                slot.library = None;
                slot.error = Some(e.clone());
                Some(format!("the particle systems cannot play: {e}"))
            }
        };
        let say = said.filter(|n| slot.noted.as_ref() != Some(n));
        if say.is_some() {
            slot.noted.clone_from(&say);
        }
        let library = slot.library.clone();
        drop(slot);
        if let Some(n) = say {
            editor.log.warn(n);
        }
        library
    }

    /// Why there is no library, when there is none.
    #[must_use]
    pub fn library_error(&self) -> Option<String> {
        self.lib.lock().error.clone()
    }
}

/// Plays or stops particle objects (`Play` / `Stop`; a stop lets the particles live out their
/// lives). `names` empty: the selected visible ones, as the tool's buttons do. Answers what each
/// named object did; never blocks the UI thread (call it from a task).
pub async fn play(editor: &Arc<Editor>, names: &[String], on: bool) -> Result<Vec<Value>, CmdError> {
    let svc = SceneService::of(editor)?;
    let parts = Particles::of(editor);
    let library = parts.library(editor).await;
    let snap = svc.store.snapshot();
    let state = &snap.state;
    let targets: Vec<(String, Option<Arc<gz_scene::Object>>)> = if names.is_empty() {
        state
            .of_class(ObjClass::Particles)
            .filter(|o| o.flags.visible && state.is_selected(o.id))
            .map(|o| (o.name.to_string(), Some(o.clone())))
            .collect()
    } else {
        names
            .iter()
            .map(|n| (n.clone(), state.find(ObjClass::Particles, &n.to_lowercase()).cloned()))
            .collect()
    };
    let mut out = Vec::with_capacity(targets.len());
    // under the lock: the entries of the named objects taken out; the systems are made and told
    // to play or stop without it (calls into the particle crate), then put back under it
    let mut taken: Vec<(ObjId, Option<Entry>)> = {
        let mut st = parts.state.lock();
        if st.library.as_ref().map(Arc::as_ptr) != library.as_ref().map(Arc::as_ptr) {
            st.entries.clear();
            st.library.clone_from(&library);
        }
        targets.iter().filter_map(|(_, o)| o.as_ref().map(|o| (o.id, st.entries.remove(&o.id)))).collect()
    };
    let mut slot = taken.iter_mut();
    for (name, obj) in targets {
        let Some(o) = obj else {
            out.push(json!({ "name": name, "ok": false, "error": "no particle system of that name" }));
            continue;
        };
        let Some((_, held)) = slot.next() else { continue };
        let ObjData::Particles(d) = &o.data else { continue };
        let entry = held.get_or_insert_with(|| Entry {
            reference: d.reference.clone(),
            system: None,
            transform: Mat4::IDENTITY,
            out: None,
        });
        if entry.system.is_none() || entry.reference != d.reference {
            entry.reference = d.reference.clone();
            entry.system = library.as_ref().and_then(|l| l.create(&d.reference));
            entry.transform = Mat4::NAN;
        }
        let world = gm(&o.world());
        match &mut entry.system {
            Some(s) => {
                if entry.transform != world {
                    s.update_parent(&world, Vec3::ZERO, false);
                    entry.transform = world;
                }
                if on {
                    s.play()
                } else {
                    s.stop(true)
                }
                out.push(json!({ "name": name, "ok": true, "playing": s.is_playing() }));
            }
            None => out.push(json!({
                "name": name,
                "ok": false,
                "error": format!("EParticlesObject: '{}' not found in library", d.reference),
            })),
        }
    }
    {
        let mut st = parts.state.lock();
        for (id, e) in taken {
            if let Some(e) = e {
                st.entries.insert(id, e);
            }
        }
    }
    editor.exec.wake().ping();
    Ok(out)
}

/// What a ray meets: the nearest visible scene object, else the ground
/// plane y = 0 the editor falls back to.
struct SceneCollider<'a> {
    state: &'a SceneState,
    lib: &'a gz_scene::library::Library,
    pick: Option<&'a ScenePick>,
}

impl Collider for SceneCollider<'_> {
    fn ray_pick(&self, start: Vec3, dir: Vec3, dist: f32) -> Option<(Vec3, Vec3)> {
        if let Some(pick) = self.pick
            && let Some(hit) = ray_scene_objects(pick, self.state, self.lib, start, dir, dist, None)
        {
            // the engine's normal is the triangle's in the object's own space; this one is the
            // world's, the one a bounce off a turned object needs
            return Some((hit.point, hit.normal.unwrap_or(Vec3::Y)));
        }
        // the plane through the origin with the normal (0, -1, 0)
        let denom = -dir.y;
        if denom.abs() < 1e-7 {
            return None;
        }
        let t = -(-start.y / denom);
        (t >= -1e-6 && t <= dist).then(|| (start + dir * t, Vec3::new(0.0, -1.0, 0.0)))
    }
}

/// The emitter's box.
fn emitter_box(p: Vec3) -> Aabb {
    Aabb::new(p - Vec3::splat(PSOBJECT_SIZE), p + Vec3::splat(PSOBJECT_SIZE))
}

/// What a scene pass needs to step and draw the systems.
pub(crate) struct Pass<'a> {
    pub state: &'a SceneState,
    pub lib: &'a gz_scene::library::Library,
    pub pick: Option<&'a ScenePick>,
    pub library: Option<Arc<Library>>,
    pub assets: &'a Assets,
    pub frustum: &'a Frustum,
    /// The class is shown.
    pub shown: bool,
}

/// Follows the scene (a reference changed: a new system, not playing; the object moved: its
/// actions follow; gone: dropped), steps the systems that play and whose emitter is in view by
/// the time since the last pass, and adds their sprites. `true` while a system in view plays.
pub(crate) fn step_and_draw(parts: &Particles, p: &Pass<'_>, list: &mut DrawList) -> bool {
    // under the lock: follow the scene and take out the systems that step - no lock is held
    // across the parallel stepping or the calls into the particle crate and the collider
    let (dt_ms, mut active) = {
        let mut st = parts.state.lock();
        if st.library.as_ref().map(Arc::as_ptr) != p.library.as_ref().map(Arc::as_ptr) {
            st.entries.clear();
            st.library.clone_from(&p.library);
        }
        let now = Instant::now();
        let dt_ms = st.last.map_or(0, |t| now.duration_since(t).as_millis().min(1000) as u32);
        st.last = Some(now);
        let State { entries, library, .. } = &mut *st;
        entries.retain(|id, _| p.state.get(*id).is_some_and(|o| matches!(o.data, ObjData::Particles(_))));
        let mut active: Vec<(ObjId, System)> = Vec::new();
        for (id, e) in entries.iter_mut() {
            let Some(o) = p.state.get(*id) else { continue };
            let ObjData::Particles(d) = &o.data else { continue };
            if e.reference != d.reference {
                e.reference = d.reference.clone();
                e.system = library.as_ref().and_then(|l| l.create(&d.reference));
                e.transform = Mat4::NAN;
            }
            let Some(s) = e.system.as_mut() else { continue };
            let world = gm(&o.world());
            if e.transform != world {
                s.update_parent(&world, Vec3::ZERO, false);
                e.transform = world;
            }
            // a system steps and draws only while its emitter's box is in view
            let seen = p.shown && o.flags.visible && p.frustum.intersects_aabb(&emitter_box(o.position()));
            if seen && s.is_playing() {
                e.out = Some((true, s.particle_count(), s.is_group()));
                if let Some(taken) = e.system.take() {
                    active.push((*id, taken));
                }
            }
        }
        (dt_ms, active)
    };
    let collider = SceneCollider { state: p.state, lib: p.lib, pick: p.pick };
    active.par_iter_mut().for_each(|(_, s)| s.on_frame(dt_ms, Some(&collider)));
    let table = p.assets.table();
    let mut batches = Vec::new();
    let mut playing = false;
    for (id, s) in &active {
        playing |= s.is_playing();
        batches.clear();
        s.sprites(&mut batches);
        for b in &batches {
            let material = p.assets.material(MaterialDesc::textured(table.classify(&b.shader), &b.texture));
            list.sprites.push(SpriteBatch {
                material,
                sprites: b
                    .sprites
                    .iter()
                    .map(|s| Sprite {
                        position: s.pos,
                        half: s.half,
                        angle: s.angle,
                        color: Color::from_argb(s.color),
                        uv: s.uv,
                        align: match s.align {
                            Align::Camera => SpriteAlign::Camera,
                            Align::Axes { t, r } => SpriteAlign::Axes { top: t, right: r },
                            Align::Dir(d) => SpriteAlign::Along(d),
                        },
                    })
                    .collect(),
                cull: match b.cull {
                    Cull::None => SpriteCull::None,
                    Cull::Clockwise => SpriteCull::Clockwise,
                    Cull::CounterClockwise => SpriteCull::CounterClockwise,
                },
                size: SpriteSize::World,
                bounds: Some(Aabb::new(b.bounds.0, b.bounds.1)),
                id: id.0,
                flags: InstanceFlags::NO_ID,
            });
        }
    }
    // under the lock again: each system back into its entry, unless the entry went or got
    // another system meanwhile
    let mut st = parts.state.lock();
    for (id, s) in active {
        if let Some(e) = st.entries.get_mut(&id) {
            e.out = None;
            if e.system.is_none() {
                e.system = Some(s);
            }
        }
    }
    playing
}

/// What `status` says of each system.
fn status(parts: &Particles, state: &SceneState) -> Vec<Value> {
    let st = parts.state.lock();
    state
        .of_class(ObjClass::Particles)
        .map(|o| {
            let reference = match &o.data {
                ObjData::Particles(d) => d.reference.as_str(),
                _ => "",
            };
            let entry = st.entries.get(&o.id);
            let e = entry.and_then(|e| e.system.as_ref());
            // a system a pass is stepping right now answers what it was when taken out
            let (playing, particles, group) = match (e, entry.and_then(|x| x.out)) {
                (Some(s), _) => (s.is_playing(), s.particle_count(), s.is_group()),
                (None, Some(out)) => out,
                (None, None) => (false, 0, false),
            };
            json!({
                "name": o.name.as_str(),
                "reference": reference,
                "selected": state.is_selected(o.id),
                "visible": o.flags.visible,
                "playing": playing,
                "particles": particles,
                "group": group,
            })
        })
        .collect()
}

/// `viewport.play what=particles`: plays (`Some(true)`) or stops the named systems (every one
/// without names), and says what plays.
pub(crate) async fn play_command(ed: &Arc<Editor>, names: &[String], on: Option<bool>) -> Reply {
    let changed = match on {
        Some(on) => Some(play(ed, names, on).await?),
        None => None,
    };
    let action = match on {
        Some(true) => "play",
        Some(false) => "stop",
        None => "state",
    };
    let svc = SceneService::of(ed)?;
    let snap = svc.store.snapshot();
    let parts = Particles::of(ed);
    let library = parts.library(ed).await;
    let systems = status(&parts, &snap.state);
    let playing = systems.iter().filter(|s| s["playing"].as_bool() == Some(true)).count();
    let mut out = json!({
        "action": action,
        "library": library.as_ref().map_or_else(
            || json!({ "error": parts.library_error().unwrap_or_default() }),
            |l| {
                json!({
                    "effects": l.effects().len(),
                    "groups": l.groups().len(),
                    "note": parts.library_error().unwrap_or_default(),
                })
            },
        ),
        "playing": playing,
        "systems": systems,
    });
    if let (Some(c), Some(o)) = (changed, out.as_object_mut()) {
        o.insert("changed".into(), Value::Array(c));
    }
    Ok(out)
}

/// Whether a system plays somewhere: the viewport keeps drawing while one does.
#[must_use]
pub fn any_playing(editor: &Editor) -> bool {
    Particles::of(editor)
        .state
        .lock()
        .entries
        .values()
        .any(|e| e.system.as_ref().is_some_and(System::is_playing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_falls_back_to_the_ground_plane() {
        let state = SceneState::default();
        let lib = gz_scene::library::Library::new(Vec::new());
        let c = SceneCollider { state: &state, lib: &lib, pick: None };
        let hit = c.ray_pick(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -1.0, 0.0), 5.0);
        assert_eq!(hit, Some((Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0))));
        // too short, or going up
        assert!(c.ray_pick(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -1.0, 0.0), 1.0).is_none());
        assert!(c.ray_pick(Vec3::new(0.0, 2.0, 0.0), Vec3::Y, 5.0).is_none());
        assert_eq!(emitter_box(Vec3::ONE).size(), Vec3::ONE);
    }
}
