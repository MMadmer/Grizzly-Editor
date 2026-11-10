//! Poses for GPU skinning: one matrix per bone, model space at rest to model space in the pose.

use std::sync::Arc;

use glam::Mat4;
use gz_fmt_object::xmath::Mat4 as XMat4;
use gz_fmt_object::{Object, Pose, rest_pose};
use gz_fmt_ogf::{MotionSet, Visual};
use rustc_hash::FxHashMap;

fn to_glam(m: &XMat4) -> Mat4 {
    // the engine's rows i, j, k, c are glam's columns
    Mat4::from_cols_array_2d(&m.0)
}

/// The skinning matrices of a library object in a pose made by `gz_fmt_object::animate`.
#[must_use]
pub fn object_pose(object: &Object, pose: &Pose) -> Arc<[Mat4]> {
    let rest = rest_pose(&object.bones);
    pose.render_transforms(&rest).iter().map(to_glam).collect()
}

/// A library object at rest: every bone's matrix is the identity.
#[must_use]
pub fn object_rest(object: &Object) -> Arc<[Mat4]> {
    vec![Mat4::IDENTITY; object.bones.len().max(1)].into()
}

/// The skinning matrices of a compiled model playing `motion` of `motions` at `t` seconds. The
/// rest pose is the one its IK data names; a bone the motion has no keys for stands at rest.
/// `None` when the model has no skeleton or the motion does not exist.
#[must_use]
pub fn visual_pose(visual: &Visual, motions: &MotionSet, motion: usize, t: f32) -> Option<Arc<[Mat4]>> {
    if visual.bones.is_empty() {
        return None;
    }
    let keys = motions.pose(motion, t)?;
    let names: Vec<_> = visual.bones.iter().map(|b| b.name.clone()).collect();
    // key slot -> bone, turned round
    let mut key_of: Vec<Option<usize>> = vec![None; names.len()];
    for (slot, bone) in motions.remap(&names).into_iter().enumerate() {
        if let Some(b) = bone
            && let Some(k) = key_of.get_mut(usize::from(b))
        {
            *k = Some(slot);
        }
    }
    let by_name: FxHashMap<String, usize> = names.iter().enumerate().map(|(i, n)| (n.to_ascii_lowercase(), i)).collect();
    let parent: Vec<Option<usize>> = visual
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| by_name.get(&b.parent.to_ascii_lowercase()).copied().filter(|p| *p != i))
        .collect();

    let rest_local: Vec<XMat4> = (0..names.len())
        .map(|i| {
            visual.ik.get(i).map_or(XMat4::IDENTITY, |ik| {
                let mut m = XMat4::xyz_i(ik.rest_rotate);
                m.set_c(ik.rest_offset);
                m
            })
        })
        .collect();
    let pose_local: Vec<XMat4> = (0..names.len())
        .map(|i| match key_of.get(i).copied().flatten().and_then(|s| keys.get(s)) {
            Some(k) => XMat4::from_quat(k.rotation, k.translation),
            None => rest_local.get(i).copied().unwrap_or(XMat4::IDENTITY),
        })
        .collect();

    let bind = chain(&rest_local, &parent);
    let world = chain(&pose_local, &parent);
    Some(world.iter().zip(&bind).map(|(w, b)| to_glam(&XMat4::mul_43(w, &XMat4::invert_43(b)))).collect())
}

/// Every bone's model-space transform: its parent's times its own, parents worked out first
/// whatever the order of the list (a loop is cut at the bone that closes it).
fn chain(local: &[XMat4], parent: &[Option<usize>]) -> Vec<XMat4> {
    let n = local.len();
    let mut world: Vec<Option<XMat4>> = vec![None; n];
    let mut stack = Vec::new();
    for start in 0..n {
        stack.clear();
        let mut at = Some(start);
        while let Some(i) = at {
            if world.get(i).copied().flatten().is_some() || stack.contains(&i) || stack.len() > n {
                break;
            }
            stack.push(i);
            at = parent.get(i).copied().flatten();
        }
        while let Some(i) = stack.pop() {
            let up = parent.get(i).copied().flatten().and_then(|p| world.get(p).copied().flatten()).unwrap_or(XMat4::IDENTITY);
            let own = local.get(i).copied().unwrap_or(XMat4::IDENTITY);
            if let Some(slot) = world.get_mut(i) {
                *slot = Some(XMat4::mul(&up, &own));
            }
        }
    }
    world.into_iter().map(|m| m.unwrap_or(XMat4::IDENTITY)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn chains_take_parents_first_in_any_order() {
        let t = |x: f32| XMat4::translation(Vec3::new(x, 0.0, 0.0));
        // bone 0's parent is bone 2, bone 2's is bone 1
        let local = [t(1.0), t(10.0), t(100.0)];
        let parent = [Some(2), None, Some(1)];
        let w = chain(&local, &parent);
        assert_eq!(w[1].c(), Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(w[2].c(), Vec3::new(110.0, 0.0, 0.0));
        assert_eq!(w[0].c(), Vec3::new(111.0, 0.0, 0.0));
        // a loop does not hang
        let w = chain(&local, &[Some(1), Some(0), None]);
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn engine_matrices_become_glam_columns() {
        let m = XMat4::translation(Vec3::new(1.0, 2.0, 3.0));
        let g = to_glam(&m);
        assert_eq!(g.transform_point3(Vec3::ZERO), Vec3::new(1.0, 2.0, 3.0));
        let r = XMat4::hpb(0.3, 0.2, 0.1);
        let p = Vec3::new(0.5, -1.0, 2.0);
        assert!((to_glam(&r).transform_point3(p) - r.transform_tiny(p)).abs().max_element() < 1e-6);
    }
}
