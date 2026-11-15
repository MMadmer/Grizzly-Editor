//! glTF 2.0: the container (`.gltf` JSON or `.glb`), the node tree and the whole read.

mod anim;
mod data;
mod doc;
mod mesh;

use std::path::Path;

use gz_base::SmolStr;
use gz_fmt_object::glam::{Mat4, Quat, Vec3};
use gz_fmt_object::{Bone, BonePart, Object, ObjectFlags};

use crate::assemble::{self, Surfaces};
use crate::math::{angles, mirror, mirror_mat, mirror_quat, relative};
use crate::{Bad, MAX_NODES, MeshError, MeshSource, Result};
use doc::Doc;

#[cfg(test)]
pub(crate) use data::{data_uri, file_of, percent_decode};

const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

/// Reads a `.gltf` (its buffers and pictures beside it or in data URIs) or a `.glb`.
pub fn read_gltf(path: &Path) -> Result<MeshSource> {
    let bytes = crate::read_file(path)?;
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    from_bytes(&bytes, dir).map_err(|message| MeshError::Invalid { file: crate::file_name(path), message })
}

/// The JSON of a document and the binary chunk of a GLB.
fn container(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), Bad> {
    if !bytes.starts_with(GLB_MAGIC) {
        return Ok((bytes, None));
    }
    let u32_at =
        |at: usize| -> Option<u32> { Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)) };
    let (Some(version), Some(length)) = (u32_at(4), u32_at(8)) else {
        return Err(format!("the GLB file is {} bytes long; its header alone takes 12", bytes.len()));
    };
    if version != 2 {
        return Err(format!("the GLB file has version {version}; this editor reads version 2"));
    }
    let length = length as usize;
    if length > bytes.len() || length < 12 {
        return Err(format!("the GLB file says it is {length} bytes long, but it is {}", bytes.len()));
    }
    let mut at = 12;
    let mut json = None;
    let mut bin = None;
    while at < length {
        let (Some(len), Some(kind)) = (u32_at(at).filter(|_| at + 8 <= length), u32_at(at + 4)) else {
            return Err(format!("the GLB file ends inside the header of a chunk at byte {at}"));
        };
        let start = at + 8;
        let end = start.checked_add(len as usize).filter(|e| *e <= length).ok_or_else(|| {
            format!(
                "a chunk of the GLB file says it is {len} bytes long, but only {} are left",
                length.saturating_sub(start)
            )
        })?;
        match kind {
            CHUNK_JSON if json.is_none() => json = Some(&bytes[start..end]),
            CHUNK_BIN if bin.is_none() && json.is_some() => bin = Some(&bytes[start..end]),
            _ => {}
        }
        if json.is_none() {
            return Err("the first chunk of the GLB file is not its JSON".to_owned());
        }
        // chunks are four-byte aligned
        at = end.div_ceil(4) * 4;
    }
    json.map(|j| (j, bin)).ok_or_else(|| "the GLB file holds no JSON chunk".to_owned())
}

/// A node's rest transform in the source's own space, for an animation to change.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Trs {
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

impl Trs {
    /// The transform in the game's space.
    pub fn game(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, mirror_quat(self.r), mirror(self.t))
    }
}

/// The nodes of the scene: which, in what order, and where they stand.
pub(crate) struct Tree {
    /// The scene's nodes, every parent before its children.
    pub order: Vec<usize>,
    pub parent: Vec<Option<usize>>,
    pub visited: Vec<bool>,
    /// The rest transform of every node, in the game's space.
    pub local: Vec<Mat4>,
    pub trs: Vec<Trs>,
    /// The rest transform in the scene, in the game's space; identity for a node outside it.
    pub world: Vec<Mat4>,
}

impl Tree {
    fn build(doc: &Doc, notes: &mut Vec<String>) -> Result<Tree, Bad> {
        let n = doc.nodes.len();
        if n > MAX_NODES {
            return Err(format!("the document holds {n} nodes; the most this editor reads is {MAX_NODES}"));
        }
        let mut parent = vec![None; n];
        for (i, node) in doc.nodes.iter().enumerate() {
            for c in &node.children {
                let slot = parent
                    .get_mut(*c)
                    .ok_or_else(|| format!("node {i} names child {c}, which the document lacks"))?;
                if slot.is_some() || *c == i {
                    return Err(format!("node {c} is the child of two nodes, or of itself"));
                }
                *slot = Some(i);
            }
        }
        let roots: Vec<usize> = if doc.scenes.is_empty() {
            (0..n).filter(|i| parent[*i].is_none()).collect()
        } else {
            let s = doc.scene.unwrap_or(0);
            let scene =
                doc.scenes.get(s).ok_or_else(|| format!("the document names scene {s}, which it lacks"))?;
            for r in &scene.nodes {
                if *r >= n {
                    return Err(format!("the scene names node {r}, which the document lacks"));
                }
            }
            scene.nodes.clone()
        };

        let mut local = Vec::with_capacity(n);
        let mut trs = Vec::with_capacity(n);
        let mut bad_rotation = false;
        for (i, node) in doc.nodes.iter().enumerate() {
            let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
            if let Some(m) = &node.matrix {
                if !finite(m) {
                    return Err(format!("node {i} has a matrix that is not a number"));
                }
                let m = Mat4::from_cols_array(m);
                let (s, r, t) = m.to_scale_rotation_translation();
                let r = if r.is_finite() { r.normalize() } else { Quat::IDENTITY };
                local.push(mirror_mat(m));
                trs.push(Trs { t, r, s });
                continue;
            }
            let t = node.translation.unwrap_or([0.0; 3]);
            let r = node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let s = node.scale.unwrap_or([1.0; 3]);
            if !finite(&t) || !finite(&r) || !finite(&s) {
                return Err(format!("node {i} has a transform that is not a number"));
            }
            let mut q = Quat::from_array(r);
            if q.length() < 1e-6 {
                bad_rotation = true;
                q = Quat::IDENTITY;
            }
            let x = Trs { t: Vec3::from_array(t), r: q.normalize(), s: Vec3::from_array(s) };
            local.push(x.game());
            trs.push(x);
        }
        if bad_rotation {
            notes.push("a node's rotation has no length; it was read as no rotation".to_owned());
        }

        // depth first from the roots; a node met twice means the children form a cycle
        let mut visited = vec![false; n];
        let mut tree_parent = vec![None; n];
        let mut order = Vec::new();
        let mut world = vec![Mat4::IDENTITY; n];
        for r in roots {
            if visited[r] {
                continue;
            }
            let mut stack = vec![(r, None::<usize>)];
            while let Some((i, p)) = stack.pop() {
                if visited[i] {
                    return Err(format!("the nodes form a cycle: node {i} is reached twice"));
                }
                visited[i] = true;
                tree_parent[i] = p;
                world[i] = p.map_or(Mat4::IDENTITY, |p| world[p]) * local[i];
                order.push(i);
                for c in doc.nodes[i].children.iter().rev() {
                    stack.push((*c, Some(i)));
                }
            }
        }
        Ok(Tree { order, parent: tree_parent, visited, local, trs, world })
    }

    /// The node and its ancestors, the node first.
    pub fn up(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        let mut next = Some(node);
        std::iter::from_fn(move || {
            let n = next?;
            next = self.parent[n];
            Some(n)
        })
    }
}

/// How the meshes come out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Level geometry: every node's transform applied.
    Static,
    /// A mesh moved by an animation, no skin: a bone per node.
    Rigid,
    /// Skins: a bone per joint.
    Skinned,
}

/// Extensions a file may require that change nothing this reader reads.
fn harmless(ext: &str) -> bool {
    ext == "KHR_mesh_quantization"
        || ext == "KHR_texture_transform"
        || ext == "KHR_lights_punctual"
        || ext == "KHR_texture_basisu"
        || ext == "EXT_texture_webp"
        || ext.starts_with("KHR_materials_")
}

pub(crate) fn from_bytes(bytes: &[u8], dir: &Path) -> Result<MeshSource, Bad> {
    let (json, bin) = container(bytes)?;
    let doc: Doc =
        serde_json::from_slice(json).map_err(|e| format!("the document is not glTF JSON ({e})"))?;
    let Some(asset) = &doc.asset else {
        return Err("the document has no `asset`, which every glTF file has".to_owned());
    };
    let major = |v: &str| v.split('.').next().unwrap_or("").to_owned();
    if major(&asset.version) != "2" {
        return Err(format!("the document is glTF `{}`; this editor reads glTF 2", asset.version));
    }
    if let Some(min) = &asset.min_version
        && major(min) != "2"
    {
        return Err(format!("the document needs glTF `{min}`; this editor reads glTF 2"));
    }
    if let Some(ext) = doc.extensions_required.iter().find(|e| !harmless(e)) {
        return Err(format!("the document requires the extension `{ext}`, which this editor does not read"));
    }
    let mut notes = Vec::new();
    if doc.extensions_used.iter().any(|e| e == "KHR_texture_transform") {
        notes.push("texture transforms (KHR_texture_transform) were left out".to_owned());
    }

    let data = data::Data::load(&doc, dir, bin)?;
    let tree = Tree::build(&doc, &mut notes)?;

    let mut mesh_nodes = Vec::new();
    for &n in &tree.order {
        let node = &doc.nodes[n];
        if let Some(m) = node.mesh {
            if m >= doc.meshes.len() {
                return Err(format!("node {n} names mesh {m}, which the document lacks"));
            }
            if let Some(s) = node.skin
                && s >= doc.skins.len()
            {
                return Err(format!("node {n} names skin {s}, which the document lacks"));
            }
            mesh_nodes.push(n);
        }
    }
    let mut carries_mesh = vec![false; doc.nodes.len()];
    for &n in &mesh_nodes {
        for a in tree.up(n) {
            carries_mesh[a] = true;
        }
    }
    let mut skins: Vec<usize> = mesh_nodes.iter().filter_map(|n| doc.nodes[*n].skin).collect();
    skins.sort_unstable();
    skins.dedup();
    let moves_a_mesh = doc.animations.iter().any(|a| {
        a.channels
            .iter()
            .any(|c| c.target.node.is_some_and(|n| carries_mesh.get(n).copied().unwrap_or(false)))
    });
    let mode = if !skins.is_empty() {
        Mode::Skinned
    } else if moves_a_mesh {
        Mode::Rigid
    } else {
        if !doc.animations.is_empty() {
            notes.push("the animations move nothing that holds a mesh; they were left out".to_owned());
        }
        Mode::Static
    };

    let skeleton = match mode {
        Mode::Static => None,
        Mode::Rigid => {
            let nodes: Vec<usize> = tree.order.iter().copied().filter(|n| carries_mesh[*n]).collect();
            Some(anim::rigid_skeleton(&doc, &tree, &nodes)?)
        }
        Mode::Skinned => Some(anim::skin_skeleton(&doc, &data, &tree, &skins, &mut notes)?),
    };

    let mut surfaces: Surfaces<Option<usize>> = Surfaces::default();
    let mut pictures = mesh::Pictures::default();
    let soups = mesh::soups(
        &doc,
        &data,
        dir,
        &tree,
        &mesh_nodes,
        mode,
        skeleton.as_ref(),
        &mut surfaces,
        &mut pictures,
        &mut notes,
    )?;
    let wmaps: Vec<SmolStr> =
        skeleton.as_ref().map(|s| s.bones.iter().map(|b| b.name.clone()).collect()).unwrap_or_default();
    let meshes = assemble::build_meshes(&soups, &wmaps, &mut notes)?;

    let mut object = Object { surfaces: surfaces.surfaces, meshes, ..Object::default() };
    if let Some(sk) = &skeleton {
        let mut motions = if doc.animations.is_empty() {
            Vec::new()
        } else {
            anim::motions(&doc, &data, &tree, sk, &mut notes)?
        };
        object.flags = ObjectFlags::DYNAMIC;
        object.bones = sk
            .bones
            .iter()
            .map(|b| {
                let (q, t) = match b.parent {
                    Some(p) => relative(sk.bones[p].rest, b.rest),
                    None => b.rest,
                };
                Bone {
                    name: b.name.clone(),
                    parent: b.parent.map(|p| sk.bones[p].name.clone()).unwrap_or_default(),
                    wmap: b.name.clone(),
                    rest_offset: t,
                    rest_rotate: angles(q, None),
                    ..Bone::default()
                }
            })
            .collect();
        object.prepare_bones().map_err(|e| e.to_string())?;
        for m in &mut motions {
            m.sort_bones_by_skeleton(&object.bones);
        }
        object.motions = motions;
        object.bone_parts = vec![BonePart {
            alias: SmolStr::new_static("default"),
            bones: object.bones.iter().map(|b| b.name.clone()).collect(),
        }];
        if object.bones.len() > 223 {
            notes.push(format!("the skeleton has {} bones; the game takes 223 at most", object.bones.len()));
        }
    }
    Ok(MeshSource { object, materials: surfaces.materials, pictures: pictures.list, notes })
}
