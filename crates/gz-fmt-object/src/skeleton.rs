//! Poses of a source skeleton: the rest pose and an animated one.
//!
//! Bones are expected in hierarchy order - a parent before its children - which is the order
//! `Object::read` leaves them in.

use glam::Vec3;

use crate::bone::Bone;
use crate::xmath::Mat4;

/// Per bone, the rest transform in its parent's space, in the model's
/// space, and the inverse of the latter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestPose {
    pub local: Vec<Mat4>,
    pub world: Vec<Mat4>,
    pub inverse: Vec<Mat4>,
}

fn parent_of(bones: &[Bone], i: usize) -> Option<usize> {
    bones.get(i).and_then(|b| b.parent_id).map(usize::from).filter(|p| *p < i)
}

/// The rest pose of a skeleton.
#[must_use]
pub fn rest_pose(bones: &[Bone]) -> RestPose {
    let mut pose = RestPose {
        local: Vec::with_capacity(bones.len()),
        world: Vec::with_capacity(bones.len()),
        inverse: Vec::with_capacity(bones.len()),
    };
    for (i, b) in bones.iter().enumerate() {
        let mut local = Mat4::xyz_i(b.rest_rotate);
        local.set_c(b.rest_offset);
        let parent = parent_of(bones, i).and_then(|p| pose.world.get(p)).copied().unwrap_or(Mat4::IDENTITY);
        let world = local.mul_a43(&parent);
        pose.local.push(local);
        pose.inverse.push(Mat4::invert_43(&world));
        pose.world.push(world);
    }
    pose
}

/// What a motion gives a bone at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneState {
    pub offset: Vec3,
    /// XYZ angles in the game's order.
    pub rotate: Vec3,
    /// The rotation is in the model's space, not the parent's.
    pub world_orient: bool,
}

impl BoneState {
    /// The rest pose of a bone.
    #[must_use]
    pub fn rest(bone: &Bone) -> Self {
        Self { offset: bone.rest_offset, rotate: bone.rest_rotate, world_orient: false }
    }
}

/// Per bone, the transform in its parent's space and in the model's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub local: Vec<Mat4>,
    pub world: Vec<Mat4>,
}

impl Pose {
    /// The skinning matrices: model space at rest to model space in this pose.
    #[must_use]
    pub fn render_transforms(&self, rest: &RestPose) -> Vec<Mat4> {
        self.world.iter().zip(&rest.inverse).map(|(l, ri)| Mat4::mul_43(l, ri)).collect()
    }
}

/// Poses a skeleton. A bone without a state stands in its rest pose.
#[must_use]
pub fn animate(bones: &[Bone], states: &[BoneState]) -> Pose {
    let mut pose = Pose { local: Vec::with_capacity(bones.len()), world: Vec::with_capacity(bones.len()) };
    for (i, b) in bones.iter().enumerate() {
        let state = states.get(i).copied().unwrap_or_else(|| BoneState::rest(b));
        let parent = parent_of(bones, i).and_then(|p| pose.world.get(p)).copied().unwrap_or(Mat4::IDENTITY);
        let mut local = Mat4::xyz_i(state.rotate);
        local.set_c(state.offset);
        let mut world = Mat4::mul(&parent, &local);
        if state.world_orient {
            for r in 0..3 {
                world.set_row(r, local.row(r));
            }
            local = local.mul_a43(&Mat4::invert_43(&parent));
        }
        pose.local.push(local);
        pose.world.push(world);
    }
    pose
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xmath::similar;

    fn chain() -> Vec<Bone> {
        vec![
            Bone { name: "root".into(), rest_offset: Vec3::new(0.0, 1.0, 0.0), ..Bone::default() },
            Bone {
                name: "arm".into(),
                parent: "root".into(),
                parent_id: Some(0),
                rest_offset: Vec3::new(1.0, 0.0, 0.0),
                rest_rotate: Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0),
                ..Bone::default()
            },
        ]
    }

    #[test]
    fn rest_pose_chains_parents() {
        let rest = rest_pose(&chain());
        assert_eq!(rest.world[0].c(), Vec3::new(0.0, 1.0, 0.0));
        assert!(similar(rest.world[1].c(), Vec3::new(1.0, 1.0, 0.0), 1e-6));
        // the inverse takes the bone's origin back to zero
        assert!(similar(rest.inverse[1].transform_tiny(Vec3::new(1.0, 1.0, 0.0)), Vec3::ZERO, 1e-6));
    }

    #[test]
    fn rest_states_give_identity_skinning() {
        let bones = chain();
        let rest = rest_pose(&bones);
        let pose = animate(&bones, &[]);
        for m in pose.render_transforms(&rest) {
            assert!(similar(m.transform_tiny(Vec3::new(0.3, 0.2, 0.1)), Vec3::new(0.3, 0.2, 0.1), 1e-5));
        }
        // a world-oriented bone keeps its own axes whatever the parent does
        let mut states: Vec<BoneState> = bones.iter().map(BoneState::rest).collect();
        states[0].rotate = Vec3::new(0.0, 0.0, 1.0);
        states[1].world_orient = true;
        let pose = animate(&bones, &states);
        let own = Mat4::xyz_i(states[1].rotate);
        for r in 0..3 {
            assert_eq!(pose.world[1].row(r), own.row(r));
        }
    }
}
