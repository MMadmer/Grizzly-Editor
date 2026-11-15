//! The readers over fixtures the tests write as text (and one GLB they build) into a temp folder.

use std::path::{Path, PathBuf};

use base64::Engine;
use gz_fmt_object::glam::{Quat, Vec2, Vec3};
use gz_fmt_object::xmath;
use gz_fmt_object::{Object, ObjectFlags, VMapKind};
use gz_fmt_ogf::{ExportOptions, Visual, export_omf, export_skeleton, export_static, read_omf};
use serde_json::{Value, json};

use crate::{MeshSource, PictureData, read, read_gltf, read_obj};

/// A glTF document under construction: one binary buffer, its views and accessors.
#[derive(Default)]
struct Gltf {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Gltf {
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        self.views.push(json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() }));
        self.bin.extend_from_slice(bytes);
        self.views.len() - 1
    }

    fn accessor(&mut self, a: Value) -> usize {
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn f32s(&mut self, data: &[f32], kind: &str) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let n = match kind {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 16,
        };
        let v = self.view(&bytes);
        self.accessor(
            json!({ "bufferView": v, "componentType": 5126, "count": data.len() / n, "type": kind }),
        )
    }

    fn u16s(&mut self, data: &[u16], kind: &str, n: usize, normalized: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let v = self.view(&bytes);
        self.accessor(json!({ "bufferView": v, "componentType": 5123, "count": data.len() / n, "type": kind, "normalized": normalized }))
    }

    fn u8s(&mut self, data: &[u8], kind: &str, n: usize) -> usize {
        let v = self.view(data);
        self.accessor(
            json!({ "bufferView": v, "componentType": 5121, "count": data.len() / n, "type": kind }),
        )
    }

    /// The document with its buffer in a data URI; `rest` holds the scene's own keys.
    fn gltf(&self, rest: Value) -> String {
        let uri = format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&self.bin)
        );
        self.doc(rest, json!([{ "byteLength": self.bin.len(), "uri": uri }])).to_string()
    }

    fn doc(&self, rest: Value, buffers: Value) -> Value {
        let mut d = json!({
            "asset": { "version": "2.0" },
            "buffers": buffers,
            "bufferViews": self.views,
            "accessors": self.accessors,
        });
        if let (Some(d), Some(r)) = (d.as_object_mut(), rest.as_object()) {
            for (k, v) in r {
                d.insert(k.clone(), v.clone());
            }
        }
        d
    }

    /// The same document as a GLB: the buffer is the binary chunk.
    fn glb(&self, rest: Value) -> Vec<u8> {
        let mut json = self.doc(rest, json!([{ "byteLength": self.bin.len() }])).to_string().into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = self.bin.clone();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }
}

fn write(dir: &Path, name: &str, text: impl AsRef<[u8]>) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

/// A unit cube around the origin: four corners and one normal per side, as exporters write it.
fn cube(g: &mut Gltf) -> Value {
    let sides: [(Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::Z),
        (Vec3::NEG_Y, Vec3::Z),
        (Vec3::Z, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y),
    ];
    let (mut pos, mut nrm, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (s, (n, up)) in sides.iter().enumerate() {
        // right-handed: right = up x n, so corners (-r-u, r-u, r+u, -r+u) turn counter-clockwise seen along -n
        let r = up.cross(*n);
        for (k, (a, b)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().enumerate() {
            let p = (*n + r * *a + *up * *b) * 0.5;
            pos.extend_from_slice(&p.to_array());
            nrm.extend_from_slice(&n.to_array());
            uv.extend_from_slice(&[[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]][k]);
        }
        let o = (s * 4) as u16;
        idx.extend_from_slice(&[o, o + 1, o + 2, o, o + 2, o + 3]);
    }
    let p = g.f32s(&pos, "VEC3");
    let n = g.f32s(&nrm, "VEC3");
    let t = g.f32s(&uv, "VEC2");
    let i = g.u16s(&idx, "SCALAR", 1, false);
    json!({ "attributes": { "POSITION": p, "NORMAL": n, "TEXCOORD_0": t }, "indices": i, "material": 0 })
}

fn cube_doc(g: &mut Gltf) -> Value {
    let prim = cube(g);
    json!({
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "Crate", "mesh": 0 }],
        "meshes": [{ "primitives": [prim] }],
        "materials": [{ "name": "Wood", "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }],
        "textures": [{ "source": 0 }],
        "images": [{ "uri": "crate%20wood.png" }],
    })
}

/// The static object comes back through the `.object` file whole and exports as a static OGF.
fn round_trips(src: &MeshSource) {
    let o = &src.object;
    let back = Object::read(&o.write()).unwrap();
    assert_eq!(&back, o);
    let e = export_static(o, &ExportOptions::default()).unwrap();
    Visual::read(&e.bytes).unwrap();
}

fn outward(o: &Object) -> bool {
    o.meshes.iter().all(|m| {
        m.faces.iter().zip(m.face_normals()).all(|(f, n)| {
            let c = f.v.iter().map(|v| m.points[v.point as usize]).sum::<Vec3>() / 3.0;
            n.dot(c) > 0.0
        })
    })
}

#[test]
fn a_cube_gltf_is_welded_turned_and_keeps_its_hard_edges() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Gltf::default();
    let doc = cube_doc(&mut g);
    let path = write(dir.path(), "cube.gltf", g.gltf(doc));
    let src = read(&path).unwrap();
    let o = &src.object;
    assert!(!o.is_dynamic() && o.bones.is_empty());
    assert_eq!(o.meshes.len(), 1);
    let m = &o.meshes[0];
    assert_eq!(m.name, "Crate");
    assert_eq!(m.points.len(), 8);
    assert_eq!(m.faces.len(), 12);
    // every triangle borders two other sides (hard) and its twin over the diagonal (soft)
    assert!(m.smooth_groups.iter().all(|f| f.count_ones() == 2), "{:?}", m.smooth_groups);
    // so every corner keeps its side's normal
    let normals = m.vertex_normals();
    for (f, n) in m.face_normals().iter().enumerate() {
        for k in 0..3 {
            assert!((normals[f * 3 + k] - *n).length() < 1e-5);
        }
    }
    assert!(outward(o));
    for f in 0..12 {
        for uv in m.face_uv(f).unwrap() {
            assert!(uv.x == 0.0 || uv.x == 1.0);
            assert!(uv.y == 0.0 || uv.y == 1.0);
        }
    }
    // one surface, valid as it is, and the picture beside it
    assert_eq!(o.surfaces.len(), 1);
    let s = &o.surfaces[0];
    assert_eq!((s.name.as_str(), s.shader.as_str(), s.shader_xrlc.as_str()), ("Wood", "default", "default"));
    assert_eq!(
        (s.game_mtl.as_str(), s.vmap.as_str(), s.tex_count(), s.texture.as_str()),
        ("default", "Texture", 1, "")
    );
    assert_eq!(src.materials[0].picture, Some(0));
    assert_eq!(src.pictures[0].data, PictureData::File(dir.path().join("crate wood.png")));
    assert_eq!((src.pictures[0].name.as_str(), src.pictures[0].mime.as_str()), ("crate wood", "image/png"));
    round_trips(&src);

    // the same cube as a GLB gives the same object
    let mut g = Gltf::default();
    let doc = cube_doc(&mut g);
    let glb = write(dir.path(), "cube.glb", g.glb(doc));
    let from_glb = read_gltf(&glb).unwrap();
    assert_eq!(from_glb.object, src.object);
}

#[test]
fn a_triangle_facing_plus_z_faces_minus_z_with_its_winding_turned() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Gltf::default();
    let p = g.f32s(&[0., 0., 0., 1., 0., 0., 0., 1., 0.], "VEC3");
    let doc = json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": [{ "attributes": { "POSITION": p } }] }] });
    let src = read_gltf(&write(dir.path(), "tri.gltf", g.gltf(doc))).unwrap();
    let m = &src.object.meshes[0];
    assert!((m.face_normals()[0] - Vec3::NEG_Z).length() < 1e-6);
    let corner = |k: usize| m.points[m.faces[0].v[k].point as usize];
    assert_eq!([corner(0), corner(1), corner(2)], [Vec3::ZERO, Vec3::Y, Vec3::X]);
    // no material: the default surface; no texture coordinates: a note
    assert_eq!(src.object.surfaces[0].name, "default");
    assert!(src.notes.iter().any(|n| n.contains("texture coordinates")));
    round_trips(&src);
}

#[test]
fn node_transforms_apply_and_a_mirroring_node_keeps_faces_outward() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Gltf::default();
    let prim = cube(&mut g);
    let doc = json!({
        "scenes": [{ "nodes": [0] }],
        "nodes": [
            { "translation": [10.0, 0.0, 4.0], "children": [1, 2] },
            { "name": "big", "scale": [2.0, 2.0, 2.0], "mesh": 0 },
            { "name": "mirrored", "matrix": [-1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,5.0,0.0,1.0], "mesh": 0 },
        ],
        "meshes": [{ "primitives": [prim] }],
        "materials": [{ "name": "a", "doubleSided": true }],
    });
    let src = read_gltf(&write(dir.path(), "nodes.gltf", g.gltf(doc))).unwrap();
    let o = &src.object;
    assert_eq!(o.meshes.len(), 2);
    let big = &o.meshes[0].bbox;
    assert!(
        (big.min - Vec3::new(9.0, -1.0, -5.0)).length() < 1e-5
            && (big.max - Vec3::new(11.0, 1.0, -3.0)).length() < 1e-5
    );
    let mirrored = &o.meshes[1];
    for f in mirrored.faces.iter().zip(mirrored.face_normals()) {
        let c = f.0.v.iter().map(|v| mirrored.points[v.point as usize]).sum::<Vec3>() / 3.0;
        assert!(f.1.dot(c - Vec3::new(10.0, 5.0, -4.0)) > 0.0);
    }
    assert!(o.surfaces[0].two_sided() && src.materials[0].two_sided);
    round_trips(&src);
}

#[test]
fn strips_fans_quantised_and_sparse_accessors_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Gltf::default();
    // a strip of two triangles, positions as u16, texture coordinates normalised u16
    let quad = g.u16s(&[0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0], "VEC3", 3, false);
    let uv = g.u16s(&[0, 0, 65535, 0, 0, 65535, 65535, 65535], "VEC2", 2, true);
    // a fan of three triangles around a centre, u8 indices
    let fan_pos = g.f32s(&[0., 0., 5., 1., 0., 5., 1., 1., 5., 0., 1., 5., -1., 1., 5.], "VEC3");
    let fan_ix = g.u8s(&[0, 1, 2, 3, 4], "SCALAR", 1);
    // a triangle whose third corner a sparse accessor moves
    let base = g.f32s(&[0., 0., 9., 1., 0., 9., 0., 1., 9.], "VEC3");
    let sp_ix = g.view(&2u16.to_le_bytes());
    let sp_val = g.view(&[0.0f32, 2.0, 9.0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    let mut sparse = g.accessors[base].clone();
    sparse["sparse"] = json!({ "count": 1, "indices": { "bufferView": sp_ix, "componentType": 5123 }, "values": { "bufferView": sp_val } });
    let sparse = g.accessor(sparse);
    // no buffer view at all: zeros, then the sparse values
    let mut zeros = json!({ "componentType": 5126, "count": 3, "type": "VEC3" });
    let zx = g.view(&[1u8, 2]);
    let zv =
        g.view(&[3.0f32, 0.0, 0.0, 0.0, 3.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
    zeros["sparse"] = json!({ "count": 2, "indices": { "bufferView": zx, "componentType": 5121 }, "values": { "bufferView": zv } });
    let zeros = g.accessor(zeros);
    let line = g.f32s(&[0., 0., 0., 1., 1., 1.], "VEC3");
    let doc = json!({
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [
            { "attributes": { "POSITION": quad, "TEXCOORD_0": uv }, "mode": 5 },
            { "attributes": { "POSITION": fan_pos }, "indices": fan_ix, "mode": 6 },
            { "attributes": { "POSITION": sparse } },
            { "attributes": { "POSITION": zeros } },
            { "attributes": { "POSITION": line }, "mode": 1 },
        ] }],
        "extensionsUsed": ["KHR_mesh_quantization"],
        "extensionsRequired": ["KHR_mesh_quantization"],
    });
    let src = read_gltf(&write(dir.path(), "kinds.gltf", g.gltf(doc))).unwrap();
    let m = &src.object.meshes[0];
    assert_eq!(m.faces.len(), 2 + 3 + 1 + 1);
    // every strip and fan triangle faces -Z, as its source faced +Z
    assert!(m.face_normals().iter().all(|n| (*n - Vec3::NEG_Z).length() < 1e-5));
    assert!(m.points.contains(&Vec3::new(0.0, 2.0, -9.0)));
    assert!(m.points.contains(&Vec3::new(3.0, 0.0, 0.0)) && m.points.contains(&Vec3::new(0.0, 3.0, 0.0)));
    assert!(!m.points.contains(&Vec3::new(0.0, 1.0, -9.0)));
    let uvs: Vec<Vec2> = (0..2).flat_map(|f| m.face_uv(f).unwrap()).collect();
    assert!(uvs.contains(&Vec2::new(1.0, 1.0)));
    assert!(src.notes.iter().any(|n| n.contains("points or lines")));
    round_trips(&src);
}

/// A strip of three quads up +Y, two joints (root at the foot, tip 1.5 up), one clip.
fn skinned_doc(g: &mut Gltf) -> Value {
    let mut pos = Vec::new();
    for y in 0..4 {
        for x in 0..2 {
            pos.extend_from_slice(&[x as f32, y as f32, 0.2]);
        }
    }
    let p = g.f32s(&pos, "VEC3");
    let mut idx = Vec::new();
    for r in 0..3u16 {
        let o = r * 2;
        idx.extend_from_slice(&[o, o + 1, o + 3, o, o + 3, o + 2]);
    }
    let i = g.u16s(&idx, "SCALAR", 1, false);
    let joints: Vec<u8> = [[0, 0, 0, 0], [0, 1, 0, 0], [1, 0, 1, 0], [1, 0, 0, 0]]
        .iter()
        .flat_map(|j| [j, j])
        .flatten()
        .copied()
        .collect();
    let j = g.u8s(&joints, "VEC4", 4);
    let weights: Vec<f32> =
        [[1.0, 0.0, 0.0, 0.0], [0.75, 0.25, 0.0, 0.0], [0.4, 0.3, 0.3, 0.0], [0.5, 0.0, 0.0, 0.0]]
            .iter()
            .flat_map(|w| [w, w])
            .flatten()
            .copied()
            .collect();
    let w = g.f32s(&weights, "VEC4");
    let ibm = g.f32s(
        &[
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., //
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., -1.5, 0., 1.,
        ],
        "MAT4",
    );
    let times = g.f32s(&[0.0, 1.0], "SCALAR");
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let rot = g.f32s(&[0., 0., 0., 1., s, 0., 0., s], "VEC4");
    let step_t = g.f32s(&[0.0, 0.5], "SCALAR");
    let moves = g.f32s(&[0., 0., 0., 0., 0., 1.], "VEC3");
    json!({
        "scenes": [{ "nodes": [0, 1] }],
        "nodes": [
            { "name": "Strip", "mesh": 0, "skin": 0 },
            { "name": "Root", "children": [2] },
            { "name": "Tip", "translation": [0.0, 1.5, 0.0] },
        ],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": p, "JOINTS_0": j, "WEIGHTS_0": w }, "indices": i }] }],
        "skins": [{ "joints": [1, 2], "inverseBindMatrices": ibm }],
        "animations": [{
            "name": "Wave",
            "channels": [
                { "sampler": 0, "target": { "node": 2, "path": "rotation" } },
                { "sampler": 1, "target": { "node": 1, "path": "translation" } },
                { "sampler": 2, "target": { "node": 1, "path": "scale" } },
            ],
            "samplers": [
                { "input": times, "output": rot },
                { "input": step_t, "output": moves, "interpolation": "STEP" },
                { "input": times, "output": moves },
            ],
        }],
    })
}

#[test]
fn a_skinned_strip_has_bones_weights_and_a_motion_the_exporters_take() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Gltf::default();
    let doc = skinned_doc(&mut g);
    let src = read_gltf(&write(dir.path(), "strip.gltf", g.gltf(doc))).unwrap();
    let o = &src.object;
    assert!(o.flags.contains(ObjectFlags::DYNAMIC) && o.is_skeleton() && o.is_animated());
    let names: Vec<&str> = o.bones.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["root", "tip"]);
    assert_eq!((o.bones[1].parent.as_str(), o.bones[1].parent_id), ("root", Some(0)));
    assert!((o.bones[1].rest_offset - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-5);
    assert!(o.bones[1].rest_rotate.length() < 1e-5);
    assert_eq!(o.bone_parts.len(), 1);
    assert_eq!(o.bone_parts[0].bones, ["root", "tip"]);
    assert!(src.notes.iter().any(|n| n.contains("scale")));

    // weights: the heaviest four, brought to one; a vertex weighing one joint twice sums it
    let m = &o.meshes[0];
    assert!(m.vmaps.iter().filter(|v| v.kind == VMapKind::Weight).count() >= 2);
    let sv = m.skin_vertices(&o.bones, 4, &m.vertex_normals()).unwrap();
    let at = |y: f32| sv.iter().find(|v| (v.offs.y - y).abs() < 1e-6).unwrap();
    assert_eq!(at(0.0).bones.len(), 1);
    let mid = at(1.0);
    assert_eq!(mid.bones.len(), 2);
    assert!((mid.bones[0].w - 0.75).abs() < 1e-6 && mid.bones[0].id == 0);
    let upper = at(2.0);
    assert_eq!(upper.bones.len(), 2);
    assert!((upper.bones[0].w - 0.7).abs() < 1e-5 && upper.bones[0].id == 1);
    assert!((at(3.0).bones[0].w - 1.0).abs() < 1e-6);
    // skinned vertices stay where the mesh has them, mirrored
    assert!((at(0.0).offs.z + 0.2).abs() < 1e-6);

    // one clip, 30 keys a second over one second, the tip turned a quarter about X at its end
    assert_eq!(o.motions.len(), 1);
    let mo = &o.motions[0];
    assert_eq!((mo.name.as_str(), mo.length(), mo.fps), ("wave", 31, 30.0));
    assert_eq!(mo.bone_motions.len(), 2);
    let (t, r) = mo.evaluate(1, 1.0).unwrap();
    assert!((t - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-5);
    let mut want = crate::math::to_xray(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), Vec3::ZERO);
    want.set_c(Vec3::ZERO);
    let got = xmath::Mat4::xyz_i(r);
    assert!(got.0.iter().flatten().zip(want.0.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-4));
    // the root steps: still at 0.4 s, moved (mirrored) from 0.5 s on
    let (t, _) = mo.evaluate(0, 0.4).unwrap();
    assert!(t.length() < 1e-6);
    let (t, _) = mo.evaluate(0, 0.5).unwrap();
    assert!((t - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);

    // through the `.object` file: geometry and bones whole, the motion with its keys
    let back = Object::read(&o.write()).unwrap();
    assert_eq!(back.meshes, o.meshes);
    assert_eq!(back.bones, o.bones);
    assert_eq!(back.surfaces, o.surfaces);
    assert_eq!(back.bone_parts, o.bone_parts);
    assert_eq!(back.motions[0].length(), 31);
    assert!((back.motions[0].evaluate(0, 0.5).unwrap().0 - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);

    // the skeleton exporters take it, and the game's reader reads what they wrote
    let e = export_skeleton(o, &ExportOptions::default(), 4).unwrap();
    let v = Visual::read(&e.bytes).unwrap();
    assert_eq!(v.bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["root", "tip"]);
    let motions = v.motions.unwrap();
    assert_eq!(motions.motions.len(), 1);
    assert_eq!(motions.motions[0].length, 31);
    assert_eq!(motions.defs[0].name, "wave");
    let pose = motions.pose(0, 0.0).unwrap();
    assert!((pose[1].translation - Vec3::new(0.0, 1.5, 0.0)).length() < 1e-2);
    let omf = export_omf(o).unwrap();
    let set = read_omf(&omf.bytes).unwrap();
    assert_eq!(set.motions[0].bones.len(), 2);
    assert_eq!(set.parts[0].name, "default");
}

#[test]
fn an_animated_mesh_without_a_skin_is_a_rigid_skeleton() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Gltf::default();
    let prim = cube(&mut g);
    let times = g.f32s(&[0.0, 2.0], "SCALAR");
    let moves = g.f32s(&[0., 0., 0., 0., 3., 0.], "VEC3");
    let doc = json!({
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "Door", "children": [1] }, { "name": "Leaf", "mesh": 0, "translation": [1.0, 0.0, 0.0] }],
        "meshes": [{ "primitives": [prim] }],
        "animations": [{
            "channels": [{ "sampler": 0, "target": { "node": 1, "path": "translation" } }],
            "samplers": [{ "input": times, "output": moves, "interpolation": "CUBICSPLINE" }],
        }],
    });
    // a cubic spline holds in-tangent, value, out-tangent per key: this output is too short
    let bad = read_gltf(&write(dir.path(), "short.gltf", g.gltf(doc.clone())));
    assert!(bad.unwrap_err().to_string().contains("values for 2 keys"));

    let mut doc = doc;
    let cubic = g.f32s(&[0., 0., 0., 1., 0., 0., 0., 0., 0., 0., 0., 0., 1., 3., 0., 0., 0., 0.], "VEC3");
    doc["animations"][0]["samplers"][0]["output"] = json!(cubic);
    let src = read_gltf(&write(dir.path(), "door.gltf", g.gltf(doc))).unwrap();
    let o = &src.object;
    assert!(o.is_dynamic());
    assert_eq!(o.bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["door", "leaf"]);
    assert_eq!(o.motions[0].name, "motion_0");
    assert_eq!(o.motions[0].length(), 61);
    let (t, _) = o.motions[0].evaluate(1, 2.0).unwrap();
    assert!((t - Vec3::new(1.0, 3.0, 0.0)).length() < 1e-4);
    let w = o.meshes[0].skin_vertices(&o.bones, 4, &o.meshes[0].vertex_normals()).unwrap();
    assert!(w.iter().all(|v| v.bones.len() == 1 && v.bones[0].id == 1));
    export_skeleton(o, &ExportOptions::default(), 4).unwrap();
}

#[test]
fn an_obj_with_a_quad_an_ngon_negative_indices_and_its_mtl() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "walls.mtl",
        "newmtl brick\nKd 1 1 1\nmap_Kd -s 1 1 1 -clamp on textures/brick wall.png\nd 0.5\n\nnewmtl plain\nKd 0.5 0.5 0.5\n",
    );
    let obj = "\
# a quad and a pentagon
mtllib walls.mtl
o Wall
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 0
vt 1 0
vt 1 1
vt 0 0.25
vn 0 0 1
usemtl brick
f 1/1/1 2/2/1 3/3/1 4/4/1
g Roof
v 0 2 0
v 1 2 0
v 1.5 2.5 0
v 0.5 3 0
v -0.5 2.5 0
usemtl plain
f -5 -4 -3 -2 -1
l 1 2
";
    let src = read_obj(&write(dir.path(), "walls.obj", obj)).unwrap();
    let o = &src.object;
    assert_eq!(o.meshes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Wall", "Roof"]);
    assert_eq!(o.meshes[0].faces.len(), 2);
    assert_eq!(o.meshes[1].faces.len(), 3);
    assert_eq!(o.meshes[1].points.len(), 5);
    assert!(o.meshes.iter().all(|m| m.face_normals().iter().all(|n| (*n - Vec3::NEG_Z).length() < 1e-5)));
    // v runs up in OBJ: 0.25 becomes 0.75
    let uvs: Vec<Vec2> = (0..2).flat_map(|f| o.meshes[0].face_uv(f).unwrap()).collect();
    assert!(uvs.contains(&Vec2::new(0.0, 0.75)) && uvs.contains(&Vec2::new(1.0, 0.0)));
    assert_eq!(o.surfaces.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["brick", "plain"]);
    assert_eq!(src.materials[0].picture, Some(0));
    assert_eq!(src.materials[1].picture, None);
    assert_eq!(src.pictures[0].data, PictureData::File(dir.path().join("textures/brick wall.png")));
    assert!(src.notes.iter().any(|n| n.contains("lines")));
    round_trips(&src);
}

#[test]
fn an_l_seen_from_above_keeps_its_turn() {
    let dir = tempfile::tempdir().unwrap();
    // seen from above, x to the right and "away" up: a bar from (0, 0) to (1, 3) with a foot to
    // the right at the bottom, (1, 0) to (2, 1); counter-clockwise from the inner corner. OBJ's
    // away is -z.
    let outline = [(1.0, 1.0), (1.0, 3.0), (0.0, 3.0), (0.0, 0.0), (2.0, 0.0), (2.0, 1.0)];
    let mut text = String::new();
    for (x, away) in outline {
        text.push_str(&format!("v {x} 0 {}\n", -away));
    }
    text.push_str("f 1 2 3 4 5 6\n");
    let src = read_obj(&write(dir.path(), "l.obj", text)).unwrap();
    let m = &src.object.meshes[0];
    assert_eq!(m.faces.len(), 4);
    // it faces up, and the game's away (+z) is the source's: the foot is at the right and near
    assert!(m.face_normals().iter().all(|n| (*n - Vec3::Y).length() < 1e-5));
    assert_eq!((m.bbox.min, m.bbox.max), (Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 3.0)));
    assert!(m.points.iter().filter(|p| p.x == 2.0).all(|p| p.z <= 1.0));
    // the same outline turned the other way, from the same inner corner, faces down
    let mut text = String::new();
    for (x, away) in outline {
        text.push_str(&format!("v {x} 0 {}\n", -away));
    }
    text.push_str("s 1\nf 1 6 5 4 3 2\n");
    let down = read_obj(&write(dir.path(), "l_down.obj", text)).unwrap();
    assert!(down.object.meshes[0].face_normals().iter().all(|n| (*n - Vec3::NEG_Y).length() < 1e-5));
    // a smoothing group without normals: no hard edge inside the polygon
    assert!(down.object.meshes[0].smooth_groups.iter().all(|f| *f == 0));
}

fn gltf_err(dir: &Path, name: &str, text: &str) -> String {
    read_gltf(&write(dir, name, text)).unwrap_err().to_string()
}

#[test]
fn hostile_gltf_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let mut g = Gltf::default();
    let p = g.f32s(&[0., 0., 0., 1., 0., 0., 0., 1., 0.], "VEC3");
    let tri = |extra: Value| {
        let mut doc = json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": [{ "attributes": { "POSITION": p } }] }] });
        if let (Some(o), Some(e)) = (doc.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        doc
    };
    // a buffer shorter than the document says
    let mut short = g.doc(tri(json!({})), json!([{ "byteLength": 136, "uri": format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&g.bin)) }]));
    let e = gltf_err(d, "short.gltf", &short.to_string());
    assert!(e.contains("100 bytes short"), "{e}");
    // a buffer file that is not there
    short["buffers"] = json!([{ "byteLength": 36, "uri": "missing.bin" }]);
    assert!(gltf_err(d, "missing.gltf", &short.to_string()).contains("`missing.bin` could not be read"));
    // an external buffer that is there
    write(d, "tri.bin", &g.bin);
    short["buffers"] = json!([{ "byteLength": 36, "uri": "tri.bin" }]);
    read_gltf(&write(d, "external.gltf", short.to_string())).unwrap();
    // an accessor that reads past its view
    let mut over = g.gltf(tri(json!({})));
    over = over.replace("\"count\":3", "\"count\":4");
    assert!(gltf_err(d, "over.gltf", &over).contains("bytes short"));
    // an absurd count
    let huge = g.gltf(tri(json!({}))).replace("\"count\":3", "\"count\":1073741824");
    assert!(gltf_err(d, "huge.gltf", &huge).contains("the most this editor reads"));
    // indices out of range
    let mut g2 = Gltf::default();
    let p2 = g2.f32s(&[0., 0., 0., 1., 0., 0., 0., 1., 0.], "VEC3");
    let ix = g2.u16s(&[0, 1, 7], "SCALAR", 1, false);
    let n = g2.f32s(&[0., 0., 1., 0., 0., 1.], "VEC3");
    let doc = json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": [{ "attributes": { "POSITION": p2 }, "indices": ix }] }] });
    assert!(gltf_err(d, "ix.gltf", &g2.gltf(doc)).contains("points at vertex 7"));
    // accessors that disagree
    let doc = json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": [{ "attributes": { "POSITION": p2, "NORMAL": n } }] }] });
    assert!(gltf_err(d, "disagree.gltf", &g2.gltf(doc)).contains("2 elements of NORMAL for its 3 positions"));
    // node cycles, a node its own child, a node with two parents
    let cyc = g.gltf(tri(json!({ "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0, "children": [1] }, { "children": [0] }] })));
    assert!(gltf_err(d, "cycle.gltf", &cyc).contains("cycle"));
    let own = g.gltf(tri(json!({ "nodes": [{ "mesh": 0, "children": [0] }] })));
    assert!(gltf_err(d, "own.gltf", &own).contains("child"));
    let two = g.gltf(tri(json!({ "nodes": [{ "children": [2] }, { "children": [2] }, { "mesh": 0 }] })));
    assert!(gltf_err(d, "two.gltf", &two).contains("two nodes"));
    // a position that is not a number
    let mut g3 = Gltf::default();
    let p3 = g3.f32s(&[0., 0., 0., f32::NAN, 0., 0., 0., 1., 0.], "VEC3");
    let doc = json!({ "nodes": [{ "mesh": 0 }], "meshes": [{ "primitives": [{ "attributes": { "POSITION": p3 } }] }] });
    assert!(gltf_err(d, "nan.gltf", &g3.gltf(doc)).contains("not a number"));
    // not glTF, the wrong version, an extension that changes the geometry
    assert!(gltf_err(d, "junk.gltf", "{ not json").contains("not glTF JSON"));
    assert!(gltf_err(d, "v1.gltf", r#"{"asset":{"version":"1.0"}}"#).contains("glTF 2"));
    let draco = g.gltf(tri(json!({ "extensionsRequired": ["KHR_draco_mesh_compression"] })));
    assert!(gltf_err(d, "draco.gltf", &draco).contains("KHR_draco_mesh_compression"));
    // nothing to read
    assert!(gltf_err(d, "empty.gltf", r#"{"asset":{"version":"2.0"}}"#).contains("no triangles"));
    // a file that is not there
    assert!(matches!(read_gltf(&d.join("nothing.gltf")), Err(crate::MeshError::Io { .. })));
    // an unknown extension
    assert!(read(&write(d, "a.fbx", "")).unwrap_err().to_string().contains(".fbx"));
}

#[test]
fn hostile_glb_files_are_errors_never_panics() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let mut g = Gltf::default();
    let doc = cube_doc(&mut g);
    let glb = g.glb(doc.clone());
    let err = |bytes: &[u8]| read_gltf(&write(d, "x.glb", bytes)).unwrap_err().to_string();
    assert!(err(&glb[..8]).contains("header"));
    let mut long = glb.clone();
    long[8..12].copy_from_slice(&(glb.len() as u32 + 4).to_le_bytes());
    assert!(err(&long).contains("says it is"));
    let mut chunk = glb.clone();
    chunk[12..16].copy_from_slice(&(glb.len() as u32).to_le_bytes());
    assert!(err(&chunk).contains("only"));
    let mut version = glb.clone();
    version[4] = 1;
    assert!(err(&version).contains("version 1"));
    // every cut and many flipped bytes: an error or an object, never a panic
    for cut in (0..glb.len()).step_by(5) {
        let mut b = glb[..cut].to_vec();
        if cut >= 12 {
            b[8..12].copy_from_slice(&(cut as u32).to_le_bytes());
        }
        let _ = crate::gltf::from_bytes(&b, d);
    }
    for at in (0..glb.len()).step_by(3) {
        let mut b = glb.clone();
        b[at] ^= 0x5a;
        let _ = crate::gltf::from_bytes(&b, d);
    }
    let mut g = Gltf::default();
    let doc = skinned_doc(&mut g);
    let glb = g.glb(doc);
    crate::gltf::from_bytes(&glb, d).unwrap();
    for at in (0..glb.len()).step_by(2) {
        let mut b = glb.clone();
        b[at] = b[at].wrapping_add(1);
        let _ = crate::gltf::from_bytes(&b, d);
    }
}

#[test]
fn hostile_obj_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let err = |name: &str, text: &str| read_obj(&write(dir.path(), name, text)).unwrap_err().to_string();
    let three = "v 0 0 0\nv 1 0 0\nv 0 1 0\n";
    assert!(
        err("range.obj", &format!("{three}f 1 2 4\n"))
            .contains("line 4: the face points at vertex 4, but 3 are defined")
    );
    assert!(err("zero.obj", &format!("{three}f 0 1 2\n")).contains("vertex 0"));
    assert!(err("neg.obj", &format!("{three}f -1 -2 -4\n")).contains("vertex -4"));
    assert!(err("uv.obj", &format!("{three}f 1/1 2/1 3/1\n")).contains("texture coordinate 1, but 0"));
    assert!(err("nan.obj", "v nan 0 0\n").contains("finite"));
    assert!(err("word.obj", "v a 0 0\n").contains("`a` is not a number"));
    assert!(err("two.obj", &format!("{three}f 1 2\n")).contains("three corners"));
    assert!(err("none.obj", three).contains("no triangles"));
    // a library that is not there is a note, not an error
    let src =
        read_obj(&write(dir.path(), "lib.obj", format!("mtllib nowhere.mtl\nusemtl x\n{three}f 1 2 3\n")))
            .unwrap();
    assert!(src.notes.iter().any(|n| n.contains("nowhere.mtl")));
    assert!(src.notes.iter().any(|n| n.contains("`x`")));
    // line continuation
    let src = read_obj(&write(dir.path(), "cont.obj", format!("{three}f 1 \\\n 2 3\n"))).unwrap();
    assert_eq!(src.object.meshes[0].faces.len(), 1);
}

#[test]
fn data_uris_and_percent_names_decode() {
    assert_eq!(crate::gltf::percent_decode("a%20b%zz"), b"a b%zz");
    let (mime, bytes) = crate::gltf::data_uri("data:image/png;base64,AAEC").unwrap().unwrap();
    assert_eq!((mime.as_str(), bytes.as_slice()), ("image/png", &[0u8, 1, 2][..]));
    assert!(crate::gltf::data_uri("data:;base64,@@").unwrap().is_err());
    assert!(crate::gltf::data_uri("file.bin").is_none());
    assert!(crate::gltf::file_of(Path::new("d"), "https://x/y.bin").is_none());
    assert_eq!(
        crate::gltf::file_of(Path::new("d"), "sub/a%20b.bin"),
        Some(Path::new("d").join("sub").join("a b.bin"))
    );
}

/// A document names its files relative to itself: a name that points at a place of the machine
/// or the network is never turned into a path, so nothing there is opened.
#[test]
fn a_document_s_file_names_never_leave_for_a_drive_or_a_server() {
    let dir = Path::new("models").join("hut");
    assert_eq!(crate::beside(&dir, "tex\\a.png"), Some(dir.join("tex").join("a.png")));
    assert_eq!(crate::beside(&dir, "./a.bin"), Some(dir.join("a.bin")));
    assert_eq!(
        crate::beside(&dir, "../shared/a.png"),
        Some(Path::new("models").join("shared").join("a.png"))
    );
    for rooted in [
        "C:\\art\\a.png",
        "c:/art/a.png",
        "C:a.png",
        "/etc/a",
        "\\a",
        "\\\\host\\share\\a.bin",
        "//host/share/a.bin",
        "",
        "  ",
    ] {
        assert_eq!(crate::beside(&dir, rooted), None, "{rooted}");
        assert_eq!(crate::gltf::file_of(&dir, rooted), None, "{rooted}");
    }
    // an OBJ whose material library names a picture on the artist's disk: the surface is there,
    // its picture is not followed
    let tmp = std::env::temp_dir().join(format!("gz-fmt-mesh-rooted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write(&tmp, "a.mtl", "newmtl wall\nmap_Kd C:\\art\\wall.png\n");
    let obj = write(&tmp, "a.obj", "mtllib a.mtl\nv 0 0 0\nv 1 0 0\nv 0 0 1\nusemtl wall\nf 1 2 3\n");
    let src = read_obj(&obj).expect("the model reads");
    assert_eq!(src.object.surfaces.len(), 1);
    assert!(src.pictures.is_empty(), "{:?}", src.pictures.iter().map(|p| &p.name).collect::<Vec<_>>());
    // a glTF buffer on a server is refused in a sentence, with no file asked for
    let doc = json!({
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 36, "uri": "//host/share/a.bin" }],
    });
    let gltf = write(&tmp, "a.gltf", doc.to_string());
    assert!(read_gltf(&gltf).is_err());
    let _ = std::fs::remove_dir_all(&tmp);
}
