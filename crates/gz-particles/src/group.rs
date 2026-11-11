//! `CParticleGroup`: effects that start and stop at their times, and the child effects a
//! particle of one starts - for its life (`OnPlayChild`), at its birth (`OnBirthChild`) or at its
//! death (`OnDeadChild`).

use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::actions::Ctx;
use crate::effect::{Effect, Event, Particle, SpriteBatch};
use crate::system::{Collider, GroupDef, GroupEffect, Library, group_flags};
use crate::STEP;

/// xrCore's `EPS_L`.
const EPS_L: f32 = 0.001_000_0;

/// `SItem`: one effect of the group and its children.
#[derive(Debug)]
struct Item {
    effect: Effect,
    /// A child per particle of the effect, in the particles' order.
    related: Vec<Effect>,
    /// Children living out their lives.
    free: Vec<Effect>,
}

/// The emitter's transform as a child sees it (`emitter->m_XFORM` when it draws in its space).
#[derive(Clone, Copy)]
struct Emitter {
    xform: Option<Mat4>,
}

/// The matrix and velocity a child starts with (`StartRelatedChild`, `StartFreeChild`).
fn child_parent(em: Emitter, m: &Particle) -> (Mat4, Vec3) {
    let vel = (m.pos - m.pos_b) / STEP;
    match em.xform {
        Some(x) => {
            let vel = x.transform_vector3(vel);
            let p = x.transform_point3(m.pos);
            let mut out = x;
            out.w_axis = p.extend(x.w_axis.w);
            (out, vel)
        }
        None => (Mat4::from_translation(m.pos), vel),
    }
}

impl Item {
    fn is_playing(&self) -> bool {
        self.effect.is_playing()
    }

    fn play(&mut self) {
        self.effect.play();
    }

    /// `SItem::Stop`: the effect and every child; without a deferred stop the children go.
    fn stop(&mut self, deferred: bool) {
        self.effect.stop(deferred);
        for c in self.related.iter_mut().chain(self.free.iter_mut()) {
            c.stop(deferred);
        }
        if !deferred {
            self.related.clear();
            self.free.clear();
        }
    }

    fn start_related(&mut self, lib: &Library, name: &str, em: Emitter, m: &Particle) {
        let mut c = Effect::new(lib.effect(name).cloned());
        let (mat, vel) = child_parent(em, m);
        c.play();
        c.update_parent(&mat, vel, false);
        self.related.push(c);
    }

    fn stop_related(&mut self, idx: usize) {
        if idx < self.related.len() {
            let mut c = self.related.swap_remove(idx);
            c.stop(true);
            self.free.push(c);
        }
    }

    fn start_free(&mut self, lib: &Library, name: &str, em: Emitter, m: &Particle, warnings: &mut Vec<String>) {
        let mut c = Effect::new(lib.effect(name).cloned());
        if c.is_looped() {
            let w = format!("Can't use looped effect '{name}' as 'On Birth' child for group.");
            if !warnings.contains(&w) {
                warnings.push(w);
            }
            return;
        }
        let (mat, vel) = child_parent(em, m);
        c.play();
        c.update_parent(&mat, vel, false);
        self.free.push(c);
    }

    /// The group's callbacks for what the effect's particles did (`OnGroupParticleBirth`,
    /// `OnGroupParticleDead`), in the order they happened.
    fn handle_events(&mut self, def: &GroupEffect, lib: &Library, warnings: &mut Vec<String>) {
        let events = std::mem::take(&mut self.effect.store.events);
        let em = Emitter { xform: self.effect.xform() };
        for e in events {
            match e {
                Event::Birth(p) => {
                    if def.flags & group_flags::ON_BIRTH_CHILD != 0 {
                        self.start_free(lib, &def.on_birth_child, em, &p, warnings);
                    }
                    if def.flags & group_flags::ON_PLAY_CHILD != 0 {
                        self.start_related(lib, &def.on_play_child, em, &p);
                    }
                }
                Event::Death(idx, p) => {
                    if def.flags & group_flags::ON_PLAY_CHILD != 0 {
                        self.stop_related(idx);
                    }
                    if def.flags & group_flags::ON_DEAD_CHILD != 0 {
                        self.start_free(lib, &def.on_dead_child, em, &p, warnings);
                    }
                }
            }
        }
    }

    /// `SItem::OnFrame`.
    #[allow(clippy::too_many_arguments)]
    fn on_frame(
        &mut self,
        dt_ms: u32,
        def: &GroupEffect,
        cx: &mut Ctx<'_>,
        lib: &Library,
        collider: Option<&dyn Collider>,
        bounds: &mut Option<(Vec3, Vec3)>,
        playing: &mut bool,
        warnings: &mut Vec<String>,
    ) {
        let merge = |b: &mut Option<(Vec3, Vec3)>, e: Option<(Vec3, Vec3)>| {
            if let Some((lo, hi)) = e {
                *b = Some(match *b {
                    Some((a, c)) => (a.min(lo), c.max(hi)),
                    None => (lo, hi),
                });
            }
        };
        self.effect.on_frame(dt_ms, cx, collider);
        self.handle_events(def, lib, warnings);
        if self.effect.is_playing() {
            *playing = true;
            merge(bounds, self.effect.bounds);
            if def.flags & group_flags::ON_PLAY_CHILD != 0 && !def.on_play_child.is_empty() {
                // the engine asserts a child per particle; a short list moves what it has
                for (m, c) in self.effect.store.list.iter().zip(self.related.iter_mut()) {
                    let vel = (m.pos - m.pos_b) / STEP;
                    c.update_parent(&Mat4::from_translation(m.pos), vel, false);
                }
            }
        }
        for c in &mut self.related {
            c.on_frame(dt_ms, cx, collider);
            if c.is_playing() {
                *playing = true;
                merge(bounds, c.bounds);
            } else if def.flags & group_flags::ON_PLAY_CHILD_REWIND != 0 {
                c.play();
            }
        }
        for c in &mut self.free {
            c.on_frame(dt_ms, cx, collider);
            if c.is_playing() {
                *playing = true;
                merge(bounds, c.bounds);
            }
        }
        self.free.retain(Effect::is_playing);
    }

    fn effects(&self) -> impl Iterator<Item = &Effect> {
        std::iter::once(&self.effect).chain(self.related.iter()).chain(self.free.iter())
    }
}

/// `CParticleGroup`.
#[derive(Debug)]
pub(crate) struct Group {
    def: Arc<GroupDef>,
    items: Vec<Item>,
    playing: bool,
    deferred: bool,
    current_time: f32,
    initial: Vec3,
    pub bounds: Option<(Vec3, Vec3)>,
    pub warnings: Vec<String>,
}

impl Group {
    /// `Compile`: an effect per entry, its births and deaths reported to the group.
    pub(crate) fn new(def: Arc<GroupDef>, lib: &Library) -> Self {
        let items = def
            .effects
            .iter()
            .map(|e| {
                let mut effect = Effect::new(lib.effect(&e.effect).cloned());
                effect.store.record = true;
                Item { effect, related: Vec::new(), free: Vec::new() }
            })
            .collect();
        Self {
            def,
            items,
            playing: false,
            deferred: false,
            current_time: 0.0,
            initial: Vec3::ZERO,
            bounds: None,
            warnings: Vec::new(),
        }
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.playing
    }

    /// `Play`: the clock starts; the items start when it reaches their `time0`.
    pub(crate) fn play(&mut self) {
        self.current_time = 0.0;
        self.deferred = false;
        self.playing = true;
    }

    pub(crate) fn stop(&mut self, deferred: bool) {
        if deferred {
            self.deferred = true;
        } else {
            self.playing = false;
        }
        for i in &mut self.items {
            i.stop(deferred);
        }
    }

    pub(crate) fn update_parent(&mut self, m: &Mat4, vel: Vec3, xform: bool) {
        self.initial = m.w_axis.truncate();
        for i in &mut self.items {
            i.effect.update_parent(m, vel, xform);
        }
    }

    /// `OnFrame`.
    pub(crate) fn on_frame(&mut self, dt_ms: u32, cx: &mut Ctx<'_>, lib: &Library, collider: Option<&dyn Collider>) {
        if !self.playing {
            let e = Vec3::splat(EPS_L);
            self.bounds = Some((self.initial - e, self.initial + e));
            return;
        }
        let ct = self.current_time;
        let f_dt = dt_ms as f32 / 1000.0;
        let def = self.def.clone();
        for (e, item) in def.effects.iter().zip(self.items.iter_mut()) {
            if e.flags & group_flags::ENABLED == 0 {
                continue;
            }
            if item.is_playing() {
                if ct <= e.time1 && ct + f_dt >= e.time1 {
                    item.stop(e.flags & group_flags::DEFERRED_STOP != 0);
                }
            } else if !self.deferred && ct <= e.time0 && ct + f_dt >= e.time0 {
                item.play();
            }
        }
        self.current_time += f_dt;
        if self.current_time > def.time_limit && def.time_limit > 0.0 && !self.deferred {
            self.stop(true);
        }
        let mut playing = false;
        let mut bounds = None;
        for (e, item) in def.effects.iter().zip(self.items.iter_mut()) {
            item.on_frame(dt_ms, e, cx, lib, collider, &mut bounds, &mut playing, &mut self.warnings);
        }
        if self.deferred && !playing {
            self.playing = false;
            self.deferred = false;
        }
        if bounds.is_some() {
            self.bounds = bounds;
        }
    }

    pub(crate) fn sprites(&self, out: &mut Vec<SpriteBatch>) {
        for i in &self.items {
            out.extend(i.effects().filter_map(Effect::sprites));
        }
    }

    pub(crate) fn particles(&self) -> Vec<Particle> {
        self.items.iter().flat_map(Item::effects).flat_map(|e| e.store.list.iter().copied()).collect()
    }

    pub(crate) fn particle_count(&self) -> usize {
        self.items.iter().flat_map(Item::effects).map(|e| e.store.list.len()).sum()
    }
}
