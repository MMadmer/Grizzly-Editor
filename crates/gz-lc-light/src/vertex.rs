//! The light of the vertex-lit faces: every vertex of a
//! face whose shader asks for vertex light gets the static light and the sun (its hemisphere
//! was found before and is kept), then the vertices at one point share their light by their
//! translucency.
//!
//! X-Ray's compiler registers a vertex into its point's group as its thread finishes it; here the
//! vertices are lit in parallel and registered in list order, which is the order it has
//! on one thread.

use std::collections::BTreeMap;

use glam::Vec3;
use rayon::prelude::*;

use crate::bake::{LitScene, Sample};
use crate::batch::{BATCH, light_samples};
use crate::color::{BaseColor, ColorC};
use crate::data::{ShaderXrlc, Shaders, shader_of};
use crate::deflector::LightEnv;
use crate::math::EPS_L;
use crate::mesh::VertId;
use crate::point::LP_DONT_HEMI;
use crate::{Ctl, Result};

/// A key of the map of points by x: equal keys stay in the order they came.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key(f32, u32);

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Key {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0).then(self.1.cmp(&o.1))
    }
}

/// The groups of vertices at one point: a vertex joins the first group
/// whose first vertex is within `EPS_L` of it, groups ordered by their first vertex's x.
#[derive(Debug, Default)]
pub struct TransGroups {
    map: BTreeMap<Key, usize>,
    groups: Vec<(Vec3, Vec<u32>)>,
    seq: u32,
}

impl TransGroups {
    fn key(x: f32) -> f32 {
        // -0 and +0 are one key, as X-Ray's compiler compares them
        if x == 0.0 { 0.0 } else { x }
    }

    /// Adds item `id` at point `p`. With `first_only` it joins the first group it fits
    /// (the translucent vertices), otherwise every group it fits.
    pub fn register(&mut self, p: Vec3, id: u32, first_only: bool) {
        let eps2 = 2.0 * EPS_L;
        let k = Self::key(p.x);
        let lo = Key(k - eps2, 0);
        let hi = Key(k + eps2, u32::MAX);
        let mut found = false;
        for (_, &g) in self.map.range(lo..=hi) {
            let Some((front, list)) = self.groups.get_mut(g) else { continue };
            if similar(*front, p, EPS_L) {
                list.push(id);
                found = true;
                if first_only {
                    return;
                }
            }
        }
        if !found {
            let g = self.groups.len();
            self.groups.push((p, vec![id]));
            self.map.insert(Key(k, self.seq), g);
            self.seq += 1;
        }
    }

    /// The groups in X-Ray's map order, each a list of items.
    #[must_use]
    pub fn in_order(&self) -> Vec<&[u32]> {
        self.map.values().filter_map(|&g| self.groups.get(g).map(|(_, l)| l.as_slice())).collect()
    }
}

fn similar(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps && (a.z - b.z).abs() < eps
}

/// The mixed light of a group (`Process all groups`): each vertex goes towards the brightest of
/// the group by its translucency (`tmp`), and is never darker than it was; `scale` is applied
/// after (0.5 for an MU model, 1 for the level).
#[must_use]
pub fn mix_group(colors: &[ColorC], scale: f32) -> Vec<ColorC> {
    let mut c = ColorC::default();
    for v in colors {
        c.max(v);
    }
    colors
        .iter()
        .map(|v| {
            let mut r = ColorC::default();
            r.lerp(v, &c, v.tmp);
            r.max(v);
            if scale != 1.0 {
                r.mul(scale);
            }
            r
        })
        .collect()
}

/// Whether a face of the vertex asks for vertex light, and the mean
/// translucency of its faces.
fn translucency(
    env: &LightEnv<'_>,
    shaders: &Shaders,
    materials: &[crate::data::BuildMaterial],
    v: VertId,
) -> (bool, f32) {
    let Some(vx) = env.mesh.vertex(v) else { return (false, 0.0) };
    let mut vertex_light = false;
    let mut t = 0.0f32;
    for f in &vx.adjacents {
        let Some(face) = env.mesh.face(*f) else { continue };
        let s = shader_of(face.material, shaders, materials);
        t += s.vert_translucency;
        if s.flags & ShaderXrlc::LIGHT_VERTEX != 0 {
            vertex_light = true;
        }
    }
    (vertex_light, t / vx.adjacents.len() as f32)
}

/// Answers the new light of the listed vertices in list order (`None` for one
/// that keeps its own). The caller writes them back ([`apply_vertex_light`]).
pub fn light_vertices(
    env: &LightEnv<'_>,
    shaders: &Shaders,
    materials: &[crate::data::BuildMaterial],
    scene: &dyn LitScene,
    ctl: &Ctl<'_>,
) -> Result<Vec<Option<BaseColor>>> {
    let verts = env.mesh.vertices();
    let total = verts.len();
    let flags = env.sun_flag() | LP_DONT_HEMI;
    let job = env.job();
    let mut lit: Vec<Option<BaseColor>> = Vec::with_capacity(total);
    // a batch of vertices: one sample each for those of a vertex-lit face
    for chunk in verts.chunks(BATCH) {
        ctl.check()?;
        let wants: Vec<Option<(Sample, f32, f32)>> = chunk
            .par_iter()
            .with_min_len(64)
            .map(|&v| {
                let (vertex_light, trans) = translucency(env, shaders, materials, v);
                let vx = env.mesh.vertex(v)?;
                vertex_light.then(|| (Sample::new(vx.p, vx.n, flags, None), trans, vx.c.get().hemi))
            })
            .collect();
        let samples: Vec<Sample> = wants.iter().flatten().map(|w| w.0).collect();
        let light = light_samples(scene, &job, &samples, ctl)?;
        let mut at = light.iter();
        for w in &wants {
            lit.push(w.and_then(|(_, trans, hemi)| {
                let mut c = ColorC::default();
                c.add(at.next()?);
                c.tmp = trans;
                c.mul(0.5);
                // the hemisphere found before is kept
                c.hemi = hemi;
                Some(BaseColor::from_c(&c))
            }));
        }
        ctl.progress(lit.len(), total);
    }
    ctl.check()?;

    ctl.stage("the translucent vertices");
    let mut groups = TransGroups::default();
    for (i, (v, c)) in verts.iter().zip(&lit).enumerate() {
        if c.is_some()
            && let Some(vx) = env.mesh.vertex(*v)
        {
            groups.register(vx.p, i as u32, true);
        }
    }
    let mut out = lit;
    let mixed: Vec<Vec<(u32, BaseColor)>> = groups
        .in_order()
        .par_iter()
        .map(|list| {
            let colors: Vec<ColorC> = list
                .iter()
                .filter_map(|&i| out.get(i as usize).copied().flatten())
                .map(|c| c.get())
                .collect();
            list.iter().copied().zip(mix_group(&colors, 1.0).iter().map(BaseColor::from_c)).collect()
        })
        .collect();
    for (i, c) in mixed.into_iter().flatten() {
        if let Some(slot) = out.get_mut(i as usize) {
            *slot = Some(c);
        }
    }
    Ok(out)
}

/// Writes what [`light_vertices`] answered into the listed vertices.
pub fn apply_vertex_light(mesh: &mut crate::mesh::Mesh, light: &[Option<BaseColor>]) {
    let verts = mesh.vertices().to_vec();
    for (v, c) in verts.iter().zip(light) {
        if let (Some(c), Some(vx)) = (c, mesh.vertex_mut(*v)) {
            vx.c = *c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vertex_joins_the_first_group_at_its_point() {
        let mut g = TransGroups::default();
        g.register(Vec3::new(1.0, 0.0, 0.0), 0, true);
        g.register(Vec3::new(0.0, 0.0, 0.0), 1, true);
        g.register(Vec3::new(1.0005, 0.0, 0.0), 2, true);
        g.register(Vec3::new(1.0, 1.0, 0.0), 3, true);
        let groups = g.in_order();
        // by x: the group at 0, then the two at x = 1 in the order they were made
        assert_eq!(groups, vec![&[1][..], &[0, 2][..], &[3][..]]);
    }

    #[test]
    fn a_translucent_vertex_takes_the_light_of_its_neighbours() {
        let dark = ColorC { rgb: Vec3::ZERO, hemi: 0.0, sun: 0.0, tmp: 0.5 };
        let bright = ColorC { rgb: Vec3::ONE, hemi: 1.0, sun: 1.0, tmp: 0.0 };
        let mixed = mix_group(&[dark, bright], 1.0);
        assert_eq!(mixed[0].rgb, Vec3::splat(0.5));
        assert_eq!(mixed[1].rgb, Vec3::ONE);
    }
}
