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

/// The compiler's flags that decide what is built. Where the light is baked (`-light=`) is not one
/// of them: a level lit on the CPU is as fresh as the same level lit on the card.
#[must_use]
pub fn built_flags(flags: &str) -> String {
    flags
        .split_whitespace()
        .filter(|w| !w.to_ascii_lowercase().starts_with("-light="))
        .collect::<Vec<_>>()
        .join(" ")
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

/// The library objects the scene's objects are made from, as one number over their contents,
/// and the project's texture assets their surfaces name. An object read from a file has the
/// file's bytes as its stamp, one made in memory (a model asset) the bytes of its asset and of
/// everything its source is made of - and the library makes it again when one of those changes
/// (the media area's watcher), so the stamp follows. The textures: [`library_stamp_with`] over
/// the media assets of the project whose `rawdata\objects` is the library's first root (the
/// scene's library is made so), read now. Blocking for an object the library has not read yet:
/// off the UI thread.
#[must_use]
pub fn library_stamp(scene: &SceneState, lib: &gz_scene::library::Library) -> u64 {
    let project = project_of(lib);
    let index = project.as_deref().map(gz_media::scan);
    library_stamp_with(scene, lib, index.as_ref().zip(project.as_deref()))
}

/// The media assets as a project's media service last read them, with the project's folder.
pub type KnownMedia = (std::sync::Arc<gz_media::Index>, std::path::PathBuf);

/// The open project's media assets as its service holds them now, without waiting or walking.
#[must_use]
pub fn known_media(editor: &gz_editor::Editor) -> Option<KnownMedia> {
    let media = gz_media::Media::get(editor)?;
    media.latest().map(|index| (index, media.root().to_path_buf()))
}

/// [`library_stamp_with`] over what the media service holds; without it ([`known_media`] had
/// nothing yet) the project is read once, as [`library_stamp`] does.
#[must_use]
pub fn library_stamp_known(
    scene: &SceneState,
    lib: &gz_scene::library::Library,
    media: Option<&KnownMedia>,
) -> u64 {
    match media {
        Some((index, root)) => library_stamp_with(scene, lib, Some((index.as_ref(), root.as_path()))),
        None => library_stamp(scene, lib),
    }
}

/// The project a scene's library belongs to: the folder two above its first root,
/// `<project>\rawdata\objects`.
fn project_of(lib: &gz_scene::library::Library) -> Option<std::path::PathBuf> {
    let first = lib.roots().first()?;
    let named = |p: &std::path::Path, n: &str| p.file_name().is_some_and(|f| f.eq_ignore_ascii_case(n));
    let rawdata = first.parent()?;
    (named(first, "objects") && named(rawdata, "rawdata"))
        .then(|| rawdata.parent().map(std::path::Path::to_path_buf))?
}

/// [`library_stamp`] with the project's media assets given (`media`: the index and the
/// project's folder): every texture a surface of the scene's objects names that a texture asset
/// publishes adds its asset's text and picture, so a texture asset that changed makes the steps
/// that compile and light with it stale. A scene that names no texture asset keeps the number
/// it had before there were any.
#[must_use]
pub fn library_stamp_with(
    scene: &SceneState,
    lib: &gz_scene::library::Library,
    media: Option<(&gz_media::Index, &std::path::Path)>,
) -> u64 {
    let mut refs: Vec<&str> = Vec::new();
    let mut textures: Vec<String> = Vec::new();
    for o in scene.objects.values() {
        if let ObjData::SceneObject(d) = &o.data {
            refs.push(d.reference.as_str());
            textures.extend(d.surfaces.iter().map(|s| s.texture.to_lowercase()));
        }
    }
    refs.sort_unstable();
    refs.dedup();
    let mut h = Stable::new();
    for r in refs {
        h.str(r);
        match lib.load(r) {
            Ok(o) => {
                h.word(o.stamp);
                textures.extend(o.object.surfaces.iter().map(|s| s.texture.to_lowercase()));
            }
            Err(_) => h.word(0),
        }
    }
    if let Some((index, root)) = media {
        textures.sort_unstable();
        textures.dedup();
        for t in textures.iter().filter(|t| !t.is_empty()) {
            let Some(e) = index.by_name(gz_project::asset_kind::MediaKind::Texture, t) else { continue };
            h.str("texture asset");
            h.str(t);
            for f in std::iter::once(&e.path).chain(&e.source).chain(&e.uses) {
                h.str(f.as_str());
                h.bytes(&std::fs::read(f.to_fs(root)).unwrap_or_default());
            }
        }
    }
    h.finish()
}

/// The painted wallmarks as the build makes faces of them.
fn marks_hash(h: &mut Stable, scene: &SceneState) {
    let Some(w) = &scene.bulk.wallmarks else { return };
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

/// The layers and settings a step reads besides objects, hashed.
fn layers(scene: &SceneState, step: Step, opts: &Options) -> u64 {
    let mut h = Stable::new();
    let level = &scene.meta.level;
    // a step that reads the scene's objects reads the models they are made from
    if step.classes().contains(&ObjClass::SceneObject) {
        h.word(opts.library);
    }
    match step {
        Step::Geometry => {
            debug_hash(&mut h, &level.build);
            h.word(u64::from(level.light_hemi_quality) | u64::from(level.light_sun_quality) << 8);
            debug_hash(&mut h, &scene.tools.part(ObjClass::Light).light());
            marks_hash(&mut h, scene);
            h.word(opts.quality as u64);
        }
        Step::Game => {
            h.str(&level.map_version);
            h.word(u64::from(level.map_usage));
            h.str(&level.level_prefix);
        }
        Step::Light => {
            h.word(opts.quality as u64);
            h.str(&built_flags(&opts.flags));
            // only a level that streams says so: the fingerprint of one that does not stays as it was
            let (cells, _) = crate::service::cells_of(scene, opts);
            if cells > 0 {
                h.str("cells");
                h.word(u64::from(cells));
            }
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
        // the build settings, the light's, the quality, the flags, the linked game; the objects
        // (authored ones the piece, the rest its shadows) and the lights are the classes it reads
        Step::Piece => {
            // the marks a scene over a level of the game paints are faces of its piece
            if scene.meta.base.is_set() {
                marks_hash(&mut h, scene);
            }
            debug_hash(&mut h, &level.build);
            h.word(u64::from(level.light_hemi_quality) | u64::from(level.light_sun_quality) << 8);
            debug_hash(&mut h, &scene.tools.part(ObjClass::Light).light());
            h.word(opts.quality as u64);
            h.str(&built_flags(&opts.flags));
            h.str(&opts.game.to_lowercase());
        }
        Step::DetailLight | Step::GameGraph | Step::SpawnLayer => {}
    }
    if let Some(t) = terrain_layers(scene, step) {
        h.word(t);
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

/// How the module's piece of a level is made from the scene, as a number: it moves when the same
/// scene would be compiled into another piece (2: a static surface that names a shader for models
/// is compiled with the shader for level geometry), so a piece made the older way is stale.
const PIECE_MAKING: u64 = 2;

/// The fingerprint of every step for one scene version, `opts` the build's options.
#[must_use]
pub fn steps(scene: &SceneState, objects: &Objects, opts: &Options) -> FxHashMap<Step, u64> {
    let mut out: FxHashMap<Step, u64> = FxHashMap::default();
    for step in Step::ORDER {
        let mut h = Stable::new();
        h.str(step.name());
        if step == Step::Piece {
            h.word(PIECE_MAKING);
        }
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
    for c in step.classes().iter().filter(|c| **c != ObjClass::Terrain) {
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
    parts.extend(terrain_changes(step, was, now));
    parts.join(", ")
}

// ---------------------------------------------------------------- the terrains (C1b)

/// What a step reads of the scene for its terrains besides their objects: the piece writes the AI
/// map's nodes under a cutting terrain into `overlay.aiedit`. `None` for a scene without a cutting
/// terrain, so the fingerprint of every other scene stays as it was.
fn terrain_layers(scene: &SceneState, step: Step) -> Option<u64> {
    if step != Step::Piece || crate::service::cutting_terrain(scene).is_none() {
        return None;
    }
    let mut h = Stable::new();
    h.str("terrain cuts");
    h.word(ai_nodes_hash(scene));
    Some(h.finish())
}

/// What changed of a step's terrains, in words: "1 terrain changed". A terrain's hashes are its
/// text and its place; the ground it samples and the ground it cuts are scene objects of their own,
/// and their models are in the library's stamp.
fn terrain_changes(step: Step, was: &Objects, now: &Objects) -> Vec<String> {
    if !step.classes().contains(&ObjClass::Terrain) {
        return Vec::new();
    }
    let old: FxHashMap<&SmolStr, &ObjHash> =
        was.read_by(step, ObjClass::Terrain).map(|e| (&e.name, &e.hash)).collect();
    let new: FxHashMap<&SmolStr, &ObjHash> =
        now.read_by(step, ObjClass::Terrain).map(|e| (&e.name, &e.hash)).collect();
    let (mut moved, mut changed, mut added) = (0usize, 0usize, 0usize);
    for (name, h) in &new {
        match old.get(name) {
            None => added += 1,
            Some(o) if o.data != h.data => changed += 1,
            Some(o) if o.xform != h.xform => moved += 1,
            Some(_) => {}
        }
    }
    let removed = old.keys().filter(|n| !new.contains_key(**n)).count();
    [(moved, "moved"), (changed, "changed"), (added, "added"), (removed, "removed")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, word)| format!("{n} {} {word}", if n == 1 { "terrain" } else { "terrains" }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project with a model asset of `parts` whose surface wears the texture asset `od\wall`,
    /// and a scene that places it; the scene's library made as the scene service makes it.
    #[allow(clippy::unwrap_used)]
    fn project() -> (tempfile::TempDir, SceneState, gz_scene::library::Library) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let put = |rel: &str, bytes: &[u8]| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        };
        put("mod.ltx", b"[module]\nid = od\n");
        put("models\\crate.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
        put("models\\crate.meshasset", b"return { mesh = 1, source = \"crate.obj\" }\n");
        put(
            "models\\wall.meshasset",
            b"return {\n  mesh = 1,\n  parts = { { shape = \"box\", surface = \"brick\" } },\n  surfaces = { { name = \"brick\", texture = \"od\\\\wall\" } },\n}\n",
        );
        put("art\\wall.tga", &crate::compile::tests::tga(4, 4, [1, 2, 3, 255]));
        put("art\\wall.texasset", b"return { tex = 1 }\n");
        put("art\\other.tga", &crate::compile::tests::tga(4, 4, [1, 2, 3, 255]));
        put("art\\other.texasset", b"return { tex = 1 }\n");
        let lib = gz_scene::library::Library::new(vec![root.join("rawdata").join("objects")]);
        lib.add_provider(gz_media::provider(root.clone(), gz_media::resolve::no_game()));
        let mut scene = SceneState::default();
        for (name, r) in [("crate_1", "models/crate.meshasset"), ("wall_1", "models/wall.meshasset")] {
            let data = gz_scene::objects::SceneObjectData { reference: r.into(), ..Default::default() };
            scene.insert(Object::new(ObjClass::SceneObject, name, ObjData::SceneObject(data)));
        }
        (dir, scene, lib)
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn the_stamp_follows_a_model_assets_source() {
        let (dir, scene, lib) = project();
        let first = library_stamp(&scene, &lib);
        assert_eq!(library_stamp(&scene, &lib), first, "nothing changed");
        std::fs::write(dir.path().join("models\\crate.obj"), b"v 0 0 0\nv 2 0 0\nv 0 1 0\nf 1 2 3\n")
            .unwrap();
        // what the media area's watcher does when the source changes
        lib.forget("models/crate.meshasset");
        let moved = library_stamp(&scene, &lib);
        assert_ne!(moved, first);
        // and the asset's own text
        std::fs::write(
            dir.path().join("models\\crate.meshasset"),
            b"return { mesh = 1, source = \"crate.obj\", scale = 2 }\n",
        )
        .unwrap();
        lib.forget("models/crate.meshasset");
        assert_ne!(library_stamp(&scene, &lib), moved);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_texture_asset_a_surface_wears_makes_the_stamp_move_and_another_does_not() {
        let (dir, scene, lib) = project();
        let first = library_stamp(&scene, &lib);
        // a texture no surface wears changes: nothing the steps read changed
        std::fs::write(dir.path().join("art\\other.tga"), crate::compile::tests::tga(4, 4, [9, 9, 9, 255]))
            .unwrap();
        assert_eq!(library_stamp(&scene, &lib), first);
        // the picture of the worn one
        std::fs::write(dir.path().join("art\\wall.tga"), crate::compile::tests::tga(4, 4, [9, 9, 9, 255]))
            .unwrap();
        let picture = library_stamp(&scene, &lib);
        assert_ne!(picture, first);
        // its text
        std::fs::write(dir.path().join("art\\wall.texasset"), b"return { tex = 1, mips = false }\n").unwrap();
        assert_ne!(library_stamp(&scene, &lib), picture);
        // a library that belongs to no project reads no texture asset: the number it always had
        let bare = gz_scene::library::Library::new(Vec::new());
        assert_eq!(library_stamp(&scene, &bare), library_stamp_with(&scene, &bare, None));
    }

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

    fn terrain_scene(cut: bool) -> SceneState {
        let mut s = SceneState::default();
        let spec = gz_terrain::Spec { cut, ..gz_terrain::Spec::default() };
        s.insert(Object::new(ObjClass::Terrain, "Yard", ObjData::Terrain(Box::new(spec))));
        s
    }

    #[test]
    fn a_terrain_edit_makes_the_geometry_and_the_piece_stale_with_a_sentence() {
        let opts = Options::default();
        let cache = HashCache::default();
        let s = terrain_scene(true);
        let was = cache.objects(&s);
        let before = steps(&s, &was, &opts);
        // the terrain's text: one more edit
        let mut edited = s.clone();
        let mut t = (**edited.find_any("yard").unwrap()).clone();
        if let ObjData::Terrain(d) = &mut t.data {
            d.edits.push(gz_terrain::Edit::Raise {
                at: glam::Vec2::ZERO,
                radius: 2.0,
                falloff: 1.0,
                amount: 1.0,
            });
        }
        edited.replace(t);
        let now = cache.objects(&edited);
        let after = steps(&edited, &now, &opts);
        for step in [Step::Geometry, Step::Piece, Step::AiMap] {
            assert_ne!(before[&step], after[&step], "{step:?}");
            assert_eq!(changes(step, &was, &now), "1 terrain changed");
        }
        // what reads no terrain stays fresh
        assert_eq!(before[&Step::Game], after[&Step::Game]);
        assert_eq!(changes(Step::Game, &was, &now), "");
        // a scene without a terrain has the fingerprints it had before there were terrains
        let empty = SceneState::default();
        let none = cache.objects(&empty);
        assert_eq!(terrain_layers(&empty, Step::Piece), None);
        assert!(terrain_changes(Step::Piece, &none, &none).is_empty());
        // the AI map under a cutting terrain is part of the piece; under one that does not cut, not
        assert!(terrain_layers(&s, Step::Piece).is_some());
        assert!(terrain_layers(&terrain_scene(false), Step::Piece).is_none());
        assert!(terrain_layers(&s, Step::Geometry).is_none());
    }
}
