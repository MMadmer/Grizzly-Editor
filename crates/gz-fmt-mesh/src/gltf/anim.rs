//! Skeletons (skins, or a bone per node of an animated mesh) and animation clips as motions.

use gz_base::SmolStr;
use gz_fmt_object::glam::{Mat4, Quat, Vec3};
use gz_fmt_object::{BoneMotion, Key, SMotion, behavior, shape};

use super::Tree;
use super::data::Data;
use super::doc::Doc;
use crate::assemble::unique;
use crate::math::{angles, mirror_mat, relative, rigid};
use crate::{Bad, MAX_FRAMES, SAMPLE_FPS};

/// A bone as the source gives it: its rest transform in the model's space, without scale.
#[derive(Clone, Debug)]
pub(crate) struct BoneSrc {
    /// Lower case, unique.
    pub name: SmolStr,
    pub parent: Option<usize>,
    /// The node that moves it; `None` for a root the reader added.
    pub node: Option<usize>,
    pub rest: (Quat, Vec3),
}

#[derive(Clone, Debug)]
pub(crate) struct Skeleton {
    pub bones: Vec<BoneSrc>,
    /// The bone of every node that is one.
    pub node_bone: Vec<Option<u16>>,
    pub root: u16,
}

impl Skeleton {
    /// The bone that carries a node: its own, the nearest ancestor's, or the root.
    pub fn bone_of(&self, tree: &Tree, node: usize) -> u16 {
        tree.up(node).find_map(|n| self.node_bone.get(n).copied().flatten()).unwrap_or(self.root)
    }
}

/// The bones of a set of nodes (in tree order): the parent of each the nearest ancestor among
/// them; several roots get a root of their own above them.
fn skeleton(doc: &Doc, tree: &Tree, nodes: &[(usize, (Quat, Vec3))]) -> Result<Skeleton, Bad> {
    let mut node_bone = vec![None; doc.nodes.len()];
    let mut names = Vec::new();
    let mut bones = Vec::with_capacity(nodes.len() + 1);
    for (i, (n, rest)) in nodes.iter().enumerate() {
        let i =
            u16::try_from(i).map_err(|_| "the skeleton has more bones than a model can hold".to_owned())?;
        node_bone[*n] = Some(i);
        let raw = doc.nodes[*n].name.clone().unwrap_or_default().to_ascii_lowercase();
        let name = unique(&raw, &format!("bone_{n}"), &mut names);
        bones.push(BoneSrc { name: SmolStr::new(name), parent: None, node: Some(*n), rest: *rest });
    }
    for b in &mut bones {
        let Some(n) = b.node else { continue };
        b.parent = tree.up(n).skip(1).find_map(|a| node_bone[a]).map(usize::from);
    }
    let roots: Vec<usize> = (0..bones.len()).filter(|b| bones[*b].parent.is_none()).collect();
    let root = if let [one] = roots.as_slice() {
        *one
    } else {
        let at = bones.len();
        let name = unique("root", "root", &mut names);
        bones.push(BoneSrc {
            name: SmolStr::new(name),
            parent: None,
            node: None,
            rest: (Quat::IDENTITY, Vec3::ZERO),
        });
        for r in roots {
            bones[r].parent = Some(at);
        }
        at
    };
    let root =
        u16::try_from(root).map_err(|_| "the skeleton has more bones than a model can hold".to_owned())?;
    Ok(Skeleton { bones, node_bone, root })
}

/// One bone per node that holds an animated mesh or is above one.
pub(crate) fn rigid_skeleton(doc: &Doc, tree: &Tree, nodes: &[usize]) -> Result<Skeleton, Bad> {
    let list: Vec<(usize, (Quat, Vec3))> = nodes.iter().map(|n| (*n, rigid(&tree.world[*n]))).collect();
    skeleton(doc, tree, &list)
}

/// One bone per joint of the skins; the bind pose from the inverse bind matrices.
pub(crate) fn skin_skeleton(
    doc: &Doc,
    data: &Data<'_>,
    tree: &Tree,
    skins: &[usize],
    notes: &mut Vec<String>,
) -> Result<Skeleton, Bad> {
    let mut rest: Vec<Option<(Quat, Vec3)>> = vec![None; doc.nodes.len()];
    for &s in skins {
        let skin = &doc.skins[s];
        let ibm = match skin.inverse_bind_matrices {
            Some(a) => {
                let (m, _) = data.floats(a, &format!("skin {s} inverse bind matrices"), &["MAT4"])?;
                if m.len() / 16 < skin.joints.len() {
                    return Err(format!(
                        "skin {s} has {} joints but {} inverse bind matrices",
                        skin.joints.len(),
                        m.len() / 16
                    ));
                }
                m
            }
            None => Vec::new(),
        };
        for (k, &j) in skin.joints.iter().enumerate() {
            if !tree.visited.get(j).copied().unwrap_or(false) {
                return Err(format!("joint node {j} of skin {s} is not in the scene"));
            }
            if rest[j].is_some() {
                continue;
            }
            let bind = match ibm.get(k * 16..k * 16 + 16) {
                Some(cols) => {
                    let m = Mat4::from_cols_slice(cols);
                    if m.determinant().abs() < 1e-12 || !m.is_finite() {
                        notes.push(format!(
                            "joint node {j} has an inverse bind matrix that cannot be inverted"
                        ));
                        Mat4::IDENTITY
                    } else {
                        mirror_mat(m.inverse())
                    }
                }
                None => Mat4::IDENTITY,
            };
            rest[j] = Some(rigid(&bind));
        }
    }
    let list: Vec<(usize, (Quat, Vec3))> =
        tree.order.iter().filter_map(|n| rest[*n].map(|r| (*n, r))).collect();
    if list.is_empty() {
        return Err("the skins have no joints".to_owned());
    }
    skeleton(doc, tree, &list)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Interp {
    Linear,
    Step,
    Cubic,
}

/// One channel of a clip, read and checked.
struct Track {
    input: Vec<f32>,
    output: Vec<f32>,
    /// 3 for a translation, 4 for a rotation.
    n: usize,
    interp: Interp,
}

impl Track {
    fn value(&self, key: usize) -> &[f32] {
        let at = if self.interp == Interp::Cubic { (key * 3 + 1) * self.n } else { key * self.n };
        &self.output[at..at + self.n]
    }

    /// The value at `t`, held before the first key and after the last.
    fn sample(&self, t: f32) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        let last = self.input.len() - 1;
        let k = self.input.partition_point(|x| *x <= t);
        let put = |out: &mut [f32; 4], v: &[f32]| out[..v.len()].copy_from_slice(v);
        if k == 0 {
            put(&mut out, self.value(0));
            return out;
        }
        if k > last {
            put(&mut out, self.value(last));
            return out;
        }
        let (k0, k1) = (k - 1, k);
        let (t0, t1) = (self.input[k0], self.input[k1]);
        let s = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 1.0 };
        let (v0, v1) = (self.value(k0), self.value(k1));
        match self.interp {
            Interp::Step => put(&mut out, v0),
            Interp::Linear if self.n == 4 => {
                let q = Quat::from_slice(v0).slerp(Quat::from_slice(v1), s);
                out = q.to_array();
            }
            Interp::Linear => {
                for c in 0..self.n {
                    out[c] = v0[c] + (v1[c] - v0[c]) * s;
                }
            }
            Interp::Cubic => {
                let dt = t1 - t0;
                let b0 = &self.output[(k0 * 3 + 2) * self.n..(k0 * 3 + 3) * self.n];
                let a1 = &self.output[(k1 * 3) * self.n..(k1 * 3 + 1) * self.n];
                let (s2, s3) = (s * s, s * s * s);
                let (h00, h10, h01, h11) =
                    (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
                for c in 0..self.n {
                    out[c] = h00 * v0[c] + h10 * dt * b0[c] + h01 * v1[c] + h11 * dt * a1[c];
                }
            }
        }
        out
    }
}

/// The work a clip may take: keys times nodes.
const MAX_SAMPLES: usize = 1 << 27;

/// Every clip a motion, sampled at [`SAMPLE_FPS`].
pub(crate) fn motions(
    doc: &Doc,
    data: &Data<'_>,
    tree: &Tree,
    sk: &Skeleton,
    notes: &mut Vec<String>,
) -> Result<Vec<SMotion>, Bad> {
    let mut out = Vec::new();
    let mut names = Vec::new();
    let mut scale_note = false;
    for (ai, anim) in doc.animations.iter().enumerate() {
        // per node: the translation track and the rotation track
        let mut tracks: Vec<[Option<usize>; 2]> = vec![[None, None]; doc.nodes.len()];
        let mut list: Vec<Track> = Vec::new();
        let mut duration = 0.0f32;
        for (ci, ch) in anim.channels.iter().enumerate() {
            let what = |s: &str| format!("animation {ai} channel {ci} {s}");
            let Some(node) = ch.target.node else { continue };
            if node >= doc.nodes.len() {
                return Err(what(&format!("moves node {node}, which the document lacks")));
            }
            let (slot, n) = match ch.target.path.as_str() {
                "translation" => (0, 3),
                "rotation" => (1, 4),
                "scale" => {
                    scale_note = true;
                    continue;
                }
                _ => continue,
            };
            let sampler =
                anim.samplers.get(ch.sampler).ok_or_else(|| what("names a sampler the animation lacks"))?;
            let interp = match sampler.interpolation.as_deref().unwrap_or("LINEAR") {
                "LINEAR" => Interp::Linear,
                "STEP" => Interp::Step,
                "CUBICSPLINE" => Interp::Cubic,
                other => return Err(what(&format!("has the unknown interpolation `{other}`"))),
            };
            let (input, _) = data.floats(sampler.input, &what("times"), &["SCALAR"])?;
            let (output, _) =
                data.floats(sampler.output, &what("values"), &[if n == 3 { "VEC3" } else { "VEC4" }])?;
            if input.is_empty() {
                return Err(what("has no keys"));
            }
            if input.iter().any(|t| !t.is_finite() || *t < 0.0) || input.windows(2).any(|w| w[1] < w[0]) {
                return Err(what("has key times that are not ascending numbers of zero or more"));
            }
            let per_key = if interp == Interp::Cubic { 3 * n } else { n };
            if output.len() != input.len() * per_key {
                return Err(what(&format!("holds {} values for {} keys", output.len() / n, input.len())));
            }
            if output.iter().any(|v| !v.is_finite()) {
                return Err(what("has a value that is not a number"));
            }
            duration = duration.max(input[input.len() - 1]);
            tracks[node][slot] = Some(list.len());
            list.push(Track { input, output, n, interp });
        }
        if list.is_empty() {
            notes.push(format!("animation {ai} moves nothing this editor keeps; it was left out"));
            continue;
        }
        let frames = ((f64::from(duration) * f64::from(SAMPLE_FPS)).round() as usize).saturating_add(1);
        if frames > MAX_FRAMES {
            return Err(format!(
                "animation {ai} is {duration} seconds long; the most this editor samples is {} seconds",
                MAX_FRAMES / 30
            ));
        }
        if frames.saturating_mul(tree.order.len().max(1)) > MAX_SAMPLES {
            return Err(format!("animation {ai} is too long for a scene of {} nodes", tree.order.len()));
        }

        let mut bone_motions: Vec<BoneMotion> =
            sk.bones.iter().map(|b| BoneMotion { name: b.name.clone(), ..BoneMotion::default() }).collect();
        let mut prev: Vec<Vec3> = sk
            .bones
            .iter()
            .map(|b| angles(b.parent.map_or(b.rest, |p| relative(sk.bones[p].rest, b.rest)).0, None))
            .collect();
        let mut world = tree.world.clone();
        let mut bone_world = vec![(Quat::IDENTITY, Vec3::ZERO); sk.bones.len()];
        for f in 0..frames {
            let time = f as f32 / SAMPLE_FPS;
            for &n in &tree.order {
                let local = match tracks[n] {
                    [None, None] => tree.local[n],
                    [t, r] => {
                        let mut trs = tree.trs[n];
                        if let Some(t) = t {
                            let v = list[t].sample(time);
                            trs.t = Vec3::new(v[0], v[1], v[2]);
                        }
                        if let Some(r) = r {
                            let q = Quat::from_array(list[r].sample(time));
                            trs.r = if q.length() > 1e-6 { q.normalize() } else { Quat::IDENTITY };
                        }
                        trs.game()
                    }
                };
                world[n] = tree.parent[n].map_or(Mat4::IDENTITY, |p| world[p]) * local;
            }
            for (b, bone) in sk.bones.iter().enumerate() {
                bone_world[b] = bone.node.map_or((Quat::IDENTITY, Vec3::ZERO), |n| rigid(&world[n]));
            }
            for (b, bone) in sk.bones.iter().enumerate() {
                let (q, t) = match bone.parent {
                    Some(p) => relative(bone_world[p], bone_world[b]),
                    None => bone_world[b],
                };
                let a = angles(q, Some(prev[b]));
                prev[b] = a;
                let env = &mut bone_motions[b].envelopes;
                // channels: position x, y, z, then heading (the angles' y), pitch (x), bank (z)
                for (ch, value) in [t.x, t.y, t.z, a.y, a.x, a.z].into_iter().enumerate() {
                    env[ch].keys.push(Key { value, time, shape: shape::LINEAR, ..Key::default() });
                }
            }
        }
        for bm in &mut bone_motions {
            for e in &mut bm.envelopes {
                e.behavior = [behavior::CONSTANT, behavior::CONSTANT];
            }
        }
        let raw = anim.name.clone().unwrap_or_default().to_ascii_lowercase();
        let name = unique(&raw, &format!("motion_{ai}"), &mut names);
        let mut m = SMotion {
            name: SmolStr::new(name),
            frame_start: 0,
            frame_end: frames as i32 - 1,
            fps: SAMPLE_FPS,
            bone_motions,
            ..SMotion::default()
        };
        m.optimize();
        out.push(m);
    }
    if scale_note {
        notes.push("animated scale was left out: a bone carries no scale".to_owned());
    }
    Ok(out)
}
