//! A compiled model's skeleton posed as the game poses it: every bone's transform in model space
//! (what the game hangs an attached item on) and its skinning matrix (that times the inverse of
//! the bind pose).
//!
//! The bind pose is the one the IK data names (each bone's rest rotation and offset); a motion's
//! key of a bone is its rotation and translation, and a bone is its parent's transform times its
//! own. A bone the motion has no keys for stands at rest.

use glam::Vec3;
use gz_fmt_object::xmath::Mat4 as XMat4;
use smol_str::SmolStr;

use crate::motion::MotionSet;
use crate::visual::Visual;

/// The bones of a model: names, parents, rest pose.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub names: Vec<SmolStr>,
    pub parent: Vec<Option<usize>>,
    /// Every bone's own transform at rest.
    pub rest: Vec<XMat4>,
    /// Every bone's model-space transform at rest.
    pub bind: Vec<XMat4>,
}

impl Skeleton {
    /// The skeleton of a model; empty for one without bones.
    #[must_use]
    pub fn of(visual: &Visual) -> Self {
        let names: Vec<SmolStr> = visual.bones.iter().map(|b| b.name.clone()).collect();
        let parent: Vec<Option<usize>> = visual
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| {
                visual.bones.iter().position(|o| o.name.eq_ignore_ascii_case(&b.parent)).filter(|p| *p != i)
            })
            .collect();
        let rest: Vec<XMat4> = (0..names.len())
            .map(|i| {
                visual.ik.get(i).map_or(XMat4::IDENTITY, |ik| {
                    let mut m = XMat4::xyz_i(ik.rest_rotate);
                    m.set_c(ik.rest_offset);
                    m
                })
            })
            .collect();
        let bind = chain(&rest, &parent);
        Self { names, parent, rest, bind }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// A bone by its name, as the game finds one (any case).
    #[must_use]
    pub fn bone(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }

    /// The root: the first bone without a parent.
    #[must_use]
    pub fn root(&self) -> Option<usize> {
        self.parent.iter().position(Option::is_none)
    }

    /// For every bone the key slot of `set` that moves it: the set's parts name the bones, and
    /// the game matches them by name.
    #[must_use]
    pub fn slots_of(&self, set: &MotionSet) -> Vec<Option<usize>> {
        let mut out = vec![None; self.names.len()];
        for (slot, bone) in set.remap(&self.names).into_iter().enumerate() {
            if let Some(b) = bone
                && let Some(s) = out.get_mut(usize::from(b))
            {
                *s = Some(slot);
            }
        }
        out
    }

    /// Every bone's own transform playing `motion` of `set` at `t` seconds of the motion's own
    /// time (looped: the keys wrap); `slots` is [`Skeleton::slots_of`] of the set.
    #[must_use]
    pub fn locals(&self, set: &MotionSet, slots: &[Option<usize>], motion: usize, t: f32) -> Vec<XMat4> {
        let Some(m) = set.motions.get(motion) else { return self.rest.clone() };
        let len = m.seconds();
        let t = if len > 0.0 { t.rem_euclid(len) } else { 0.0 };
        (0..self.names.len())
            .map(|i| match slots.get(i).copied().flatten().and_then(|s| m.bones.get(s)) {
                Some(track) => {
                    let k = track.sample(t);
                    XMat4::from_quat(k.rotation, k.translation)
                }
                None => self.rest.get(i).copied().unwrap_or(XMat4::IDENTITY),
            })
            .collect()
    }

    /// Every bone's model-space transform from its own ones.
    #[must_use]
    pub fn model(&self, locals: &[XMat4]) -> Vec<XMat4> {
        chain(locals, &self.parent)
    }

    /// The skinning matrices of a pose in model space: each bone's times the inverse of its bind.
    #[must_use]
    pub fn skinning(&self, model: &[XMat4]) -> Vec<XMat4> {
        model.iter().zip(&self.bind).map(|(m, b)| XMat4::mul_43(m, &XMat4::invert_43(b))).collect()
    }
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
            let up = parent
                .get(i)
                .copied()
                .flatten()
                .and_then(|p| world.get(p).copied().flatten())
                .unwrap_or(XMat4::IDENTITY);
            let own = local.get(i).copied().unwrap_or(XMat4::IDENTITY);
            if let Some(slot) = world.get_mut(i) {
                *slot = Some(XMat4::mul_43(&up, &own));
            }
        }
    }
    world.into_iter().map(|m| m.unwrap_or(XMat4::IDENTITY)).collect()
}

/// A point of a bone in model space: where `model` puts the bone's origin.
#[must_use]
pub fn origin_of(model: &[XMat4], bone: usize) -> Option<Vec3> {
    model.get(bone).map(XMat4::c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::tests::sample_set;

    fn skeleton() -> Skeleton {
        // root at the origin, the arm a metre up it
        let rest = vec![XMat4::IDENTITY, XMat4::translation(Vec3::Y)];
        let parent = vec![None, Some(0)];
        let bind = chain(&rest, &parent);
        Skeleton { names: vec!["root".into(), "Arm".into()], parent, rest, bind }
    }

    #[test]
    fn a_pose_moves_the_bones_by_their_keys_and_the_rest_stand() {
        let s = skeleton();
        let set = sample_set();
        let slots = s.slots_of(&set);
        assert_eq!(slots, vec![Some(0), Some(1)]);
        assert_eq!(s.bone("arm"), Some(1));
        assert_eq!(s.root(), Some(0));
        // frame 0: the root's translation is 0, the arm's own constant key is +Y
        let model = s.model(&s.locals(&set, &slots, 0, 0.0));
        assert!((origin_of(&model, 1).unwrap() - Vec3::Y).length() < 1e-5);
        // half way to frame 1 the root is at x = 1, and the arm with it
        let model = s.model(&s.locals(&set, &slots, 0, 0.5 / 30.0));
        assert!((model[0].c().x - 1.0).abs() < 1e-4);
        assert!((model[1].c() - model[0].transform_tiny(Vec3::Y)).length() < 1e-4);
        // the length is two frames: 2/30 s wraps to the start
        let wrapped = s.model(&s.locals(&set, &slots, 0, 2.0 / 30.0));
        assert!(wrapped[0].c().x.abs() < 1e-5);
        // at rest the skinning is the identity
        let rest = s.skinning(&s.model(&s.rest));
        assert!(rest.iter().all(|m| (m.c()).length() < 1e-6 && (m.row(0) - Vec3::X).length() < 1e-6));
        // a bone without keys stands at rest
        let none = s.locals(&set, &[Some(0), None], 0, 0.0);
        assert_eq!(none[1], s.rest[1]);
    }

    #[test]
    fn a_cycle_is_found_by_its_definition_and_plays_at_the_engines_speed() {
        let mut set = sample_set();
        let (def, at) = set.cycle("idle").expect("a cycle");
        assert_eq!((def.name.as_str(), at), ("idle", 0));
        assert!(!def.stops_at_end());
        assert!((set.motions[0].seconds() - 2.0 / 30.0).abs() < 1e-6);
        set.defs[0].speed = 1.234_567;
        // stored in steps of 1/655.35
        assert!((set.defs[0].played_speed() - 809.0 / 655.35).abs() < 1e-6);
        set.defs[0].flags = crate::motion::ESM_FX;
        assert!(set.cycle("idle").is_none());
        assert!(set.cycle("walk").is_none());
    }
}
