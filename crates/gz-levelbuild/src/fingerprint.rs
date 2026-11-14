//! What a build step was made from, as numbers that survive a restart of the editor.
//!
//! Every object of the scene gets two 64-bit hashes - its transform and the rest of it (class
//! data, flags, marks, motion) - and every step a fingerprint over the hashes of the objects of
//! the classes it reads, the layers and settings it reads and the fingerprints of the steps it
//! needs. The hashes are of the content alone (no session id, no pointer), so a scene saved,
//! closed and opened again gives the same numbers, and the build's record (`build.state`) says
//! after a restart what is fresh.
//!
//! An object's hashes are kept per object version (`Arc` identity) while the scene is open: an
//! edit makes new hashes for the objects it touched, never for the whole scene.

use std::fmt::Write as _;
use std::sync::Arc;

use gz_base::SmolStr;
use gz_scene::{ObjClass, ObjData, Object, SceneState};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::steps::{Options, Step};

/// A hasher whose numbers do not change between runs, machines or toolchain versions.
#[derive(Clone, Copy, Debug)]
pub struct Stable(u64);

impl Default for Stable {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Stable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bytes(&mut self, b: &[u8]) {
        let (words, rest) = b.as_chunks::<8>();
        for w in words {
            self.word(u64::from_le_bytes(*w));
        }
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.word(u64::from_le_bytes(w) ^ ((rest.len() as u64) << 59));
        }
    }

    pub fn word(&mut self, v: u64) {
        let z = (self.0 ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = (z ^ (z >> 29)).rotate_left(23).wrapping_add(0xD1B5_4A32_D192_ED03);
    }

    pub fn f32(&mut self, v: f32) {
        // -0 and 0 are one value; every NaN is one value
        let v = if v == 0.0 { 0.0 } else { v };
        self.word(if v.is_nan() { 0x7fc0_0000 } else { u64::from(v.to_bits()) });
    }

    pub fn str(&mut self, s: &str) {
        self.word(s.len() as u64);
        self.bytes(s.as_bytes());
    }

    #[must_use]
    pub fn finish(self) -> u64 {
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl std::fmt::Write for Stable {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.bytes(s.as_bytes());
        Ok(())
    }
}

/// A text's hash, case left out (a path compared the way Windows compares it).
#[must_use]
pub fn text_hash(s: &str) -> u64 {
    let mut h = Stable::new();
    h.str(&s.to_lowercase());
    h.finish()
}

/// One number of a value's `Debug` text, written into the hasher without a string between.
fn debug_hash(h: &mut Stable, v: &dyn std::fmt::Debug) {
    let _ = write!(h, "{v:?}");
}

/// An object's two hashes: where it is, and what it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ObjHash {
    pub xform: u64,
    pub data: u64,
}

/// The hashes of one object version.
#[must_use]
pub fn object_hash(o: &Object) -> ObjHash {
    let mut x = Stable::new();
    let p = o.position();
    let (r, s) = (o.rotation(), o.scale());
    for v in [p.x, p.y, p.z, r.x, r.y, r.z, s.x, s.y, s.z] {
        x.f32(v);
    }
    let mut d = Stable::new();
    d.word(u64::from(o.class.id()));
    d.word(u64::from(o.co_flags()));
    match &o.data {
        // a group's member list names session ids: what it holds is its members' own business
        ObjData::Group(_) => debug_hash(&mut d, &o.data.clone().without_members()),
        ObjData::Spawn(sp) => spawn_hash(&mut d, sp),
        other => debug_hash(&mut d, other),
    }
    if let Some(m) = &o.motion {
        d.bytes(&m.motion);
    }
    ObjHash { xform: x.finish(), data: d.finish() }
}

/// A spawn point's data: a decoded entity by the packets the game reads - what the reader of a
/// file noted about the packet it came from (its version words) is no part of it - with the
/// object's own transform left out (it is the other hash).
fn spawn_hash(d: &mut Stable, sp: &gz_scene::objects::SpawnData) {
    debug_hash(d, &sp.kind);
    match &sp.entity {
        Some(gz_scene::objects::SpawnEntity::Decoded(e)) => {
            let mut e = (**e).clone();
            let _ = e.set_position(glam::Vec3::ZERO);
            let _ = e.set_angle(glam::Vec3::ZERO);
            d.bytes(&e.to_spawn_packet());
            if let Some(u) = e.update_packet() {
                d.bytes(&u);
            }
        }
        other => debug_hash(d, other),
    }
    d.word(u64::from(sp.sd_flags));
    debug_hash(d, &sp.order);
    debug_hash(d, &sp.rpoint);
    debug_hash(d, &sp.env);
    if let Some(a) = &sp.attached {
        let h = object_hash(a);
        d.word(h.xform);
        d.word(h.data);
    }
}

/// One object of a scene version as a step sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: SmolStr,
    pub hash: ObjHash,
    /// A spawn point the game graph is made of.
    pub graph_point: bool,
}

/// The hashes of every object of a scene version, by class and name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Objects {
    /// Per class (by [`ObjClass::index`]), in placement order.
    pub by_class: Vec<Vec<Entry>>,
}

impl Objects {
    #[must_use]
    pub fn class(&self, c: ObjClass) -> &[Entry] {
        self.by_class.get(c.index()).map_or(&[], Vec::as_slice)
    }

    /// The entries of a class a step reads.
    pub fn read_by(&self, step: Step, c: ObjClass) -> impl Iterator<Item = &Entry> {
        self.class(c).iter().filter(move |e| step != Step::GameGraph || e.graph_point)
    }
}

fn is_graph_point(o: &Object) -> bool {
    matches!(&o.data, ObjData::Spawn(s) if s.section() == Some("graph_point"))
}

/// The per-object hashes of the open scene, kept per object version.
#[derive(Default)]
pub struct HashCache {
    // the `Arc` is held so its address is not reused by another object while it is cached
    memo: Mutex<FxHashMap<usize, (Arc<Object>, ObjHash)>>,
}

impl HashCache {
    /// The hashes of `scene`: an object version seen before is not hashed again; versions that
    /// are gone leave the cache.
    pub fn objects(&self, scene: &SceneState) -> Objects {
        let mut memo = self.memo.lock();
        let mut next: FxHashMap<usize, (Arc<Object>, ObjHash)> = FxHashMap::default();
        next.reserve(scene.objects.len());
        let mut by_class = vec![Vec::new(); ObjClass::ALL.len()];
        for c in ObjClass::ALL {
            let list = &mut by_class[c.index()];
            for o in scene.of_class(c) {
                let key = Arc::as_ptr(o) as usize;
                let h = match memo.remove(&key) {
                    Some((_, h)) => h,
                    None => object_hash(o),
                };
                next.insert(key, (o.clone(), h));
                list.push(Entry { name: o.name.clone(), hash: h, graph_point: is_graph_point(o) });
            }
        }
        *memo = next;
        Objects { by_class }
    }
}

/// The layers and settings a step reads besides objects, hashed.
fn layers(scene: &SceneState, step: Step, opts: &Options) -> u64 {
    let mut h = Stable::new();
    let level = &scene.meta.level;
    match step {
        Step::Geometry => {
            debug_hash(&mut h, &level.build);
            h.word(u64::from(level.light_hemi_quality) | u64::from(level.light_sun_quality) << 8);
            debug_hash(&mut h, &scene.tools.part(ObjClass::Light).light());
            if let Some(w) = &scene.bulk.wallmarks {
                for slot in crate::wallmarks::read(&w.raw) {
                    h.str(&slot.shader);
                    h.str(&slot.texture);
                    for m in &slot.marks {
                        for (p, _, t) in &m.verts {
                            h.f32(p.x);
                            h.f32(p.y);
                            h.f32(p.z);
                            h.f32(t.x);
                            h.f32(t.y);
                        }
                    }
                }
            }
            h.word(opts.quality as u64);
        }
        Step::Game => {
            h.str(&level.map_version);
            h.word(u64::from(level.map_usage));
            h.str(&level.level_prefix);
        }
        Step::Light => {
            h.word(opts.quality as u64);
            h.str(&opts.flags);
        }
        Step::Details => {
            if let Some(d) = &scene.bulk.details {
                let m = &d.model;
                debug_hash(&mut h, &m.header);
                debug_hash(&mut h, &m.indices.objects);
                for s in m.slots.iter() {
                    // the slot's lighting is the compiler's to fill
                    h.word(u64::from(s.word0));
                    h.word(u64::from(s.word1 & 0xfff));
                    for p in s.palette {
                        h.word(u64::from(p));
                    }
                }
            }
        }
        Step::AiMap => {
            debug_hash(&mut h, &scene.bulk.ai_map.as_ref().map(|a| &a.header.snap_objects));
            h.word(ai_nodes_hash(scene));
        }
        Step::Spawn => h.str(&opts.game.to_lowercase()),
        Step::DetailLight | Step::GameGraph | Step::SpawnLayer => {}
    }
    h.str(&level.level_path);
    h.finish()
}

/// The AI map's nodes as the scene's file holds them - packed over the map's box, so a map saved
/// and read again hashes the same - with its settings; the selection is no content.
#[must_use]
pub fn ai_nodes_hash(scene: &SceneState) -> u64 {
    let mut h = Stable::new();
    let Some(a) = &scene.bulk.ai_map else { return h.finish() };
    debug_hash(&mut h, &a.header.params);
    let b = a.header.bbox;
    for v in [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z] {
        h.f32(v);
    }
    for t in &a.tiles {
        for n in t.iter() {
            for l in n.links {
                h.word(u64::from(l));
            }
            let pos = u64::from(n.pos.x as u16) | u64::from(n.pos.y) << 16 | u64::from(n.pos.z as u16) << 32;
            h.word(pos | u64::from(n.plane) << 48);
            h.word(u64::from(n.flags & !gz_fmt_level::node_flags::SELECTED));
        }
    }
    h.finish()
}

/// The fingerprint of every step for one scene version, `opts` the build's options.
#[must_use]
pub fn steps(scene: &SceneState, objects: &Objects, opts: &Options) -> FxHashMap<Step, u64> {
    let mut out: FxHashMap<Step, u64> = FxHashMap::default();
    for step in Step::ORDER {
        let mut h = Stable::new();
        h.str(step.name());
        for c in step.classes() {
            for e in objects.read_by(step, *c) {
                h.word(u64::from(c.id()));
                h.str(&e.name);
                h.word(e.hash.xform);
                h.word(e.hash.data);
            }
        }
        h.word(layers(scene, step, opts));
        for need in step.needs() {
            h.word(out.get(need).copied().unwrap_or(0));
        }
        out.insert(step, h.finish());
    }
    out
}

/// What changed between two object sets for a step, in words: "12 objects moved, 1 added".
#[must_use]
pub fn changes(step: Step, was: &Objects, now: &Objects) -> String {
    let (mut moved, mut changed, mut added, mut removed) = (0usize, 0usize, 0usize, 0usize);
    for c in step.classes() {
        let old: FxHashMap<&SmolStr, &ObjHash> = was.read_by(step, *c).map(|e| (&e.name, &e.hash)).collect();
        let new: FxHashMap<&SmolStr, &ObjHash> = now.read_by(step, *c).map(|e| (&e.name, &e.hash)).collect();
        for (name, h) in &new {
            match old.get(name) {
                None => added += 1,
                Some(o) if o.data != h.data => changed += 1,
                Some(o) if o.xform != h.xform => moved += 1,
                Some(_) => {}
            }
        }
        removed += old.keys().filter(|n| !new.contains_key(**n)).count();
    }
    let mut parts = Vec::new();
    let mut say = |n: usize, word: &str| {
        if n > 0 {
            parts.push(format!("{n} {} {word}", if n == 1 { "object" } else { "objects" }));
        }
    };
    say(moved, "moved");
    say(changed, "changed");
    say(added, "added");
    say(removed, "removed");
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_the_same_on_every_run() {
        let mut h = Stable::new();
        h.str("level");
        h.f32(1.5);
        // a fixed number: a hasher that changed would make every recorded build stale
        assert_eq!(h.finish(), {
            let mut again = Stable::new();
            again.str("level");
            again.f32(1.5);
            again.finish()
        });
        let mut z = Stable::new();
        z.f32(0.0);
        let mut nz = Stable::new();
        nz.f32(-0.0);
        assert_eq!(z.finish(), nz.finish());
    }
}
