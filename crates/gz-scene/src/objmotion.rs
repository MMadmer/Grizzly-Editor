//! The motion of a dynamic scene object (`CustomObjectAnimation.cpp`): the engine's `COMotion`
//! (six envelopes) and the `SAnimParams` the Details rows edit - the frame range, the current
//! time, the key time - with the key commands, the playback controls and the files.
//!
//! The chunk an object was read with is kept and written back byte for byte until an edit makes
//! a new one ([`MotionRaw::from_model`]). Every function here is pure: it answers the motion
//! after the edit and, where the C++ calls `AnimationUpdate`, where the object stands.

use std::sync::Arc;

use gz_base::Blob;
use gz_fmt_object::{Envelope, Key, OMotion};

use crate::math::Vec3;
use crate::model::{MotionRaw, Object, co};

/// `flAutoKey`: a move of the selected object makes a key at the current time.
pub const AUTO_KEY: u32 = 1 << 30;
/// `flCameraView`: the camera looks from the object.
pub const CAMERA_VIEW: u32 = 1 << 31;
/// One frame of the controls (`1.f/30.f`).
pub const FRAME: f32 = 1.0 / 30.0;
/// `EPS_L` of the engine, `fsimilar`'s default.
const EPS_L: f32 = 0.001;

/// `COMotion::Load` of the chunk (the `.anm` body without its chunk header).
#[must_use]
pub fn decode(blob: &[u8]) -> Option<OMotion> {
    let mut w = gz_chunk::Writer::new();
    w.w_chunk(0x1100, blob);
    gz_fmt_object::read_anm(&w.into_bytes()).ok()
}

/// `COMotion::Save`: the chunk's body.
#[must_use]
pub fn encode(m: &OMotion) -> Vec<u8> {
    let file = gz_fmt_object::write_anm(m);
    file.get(8..).map(<[u8]>::to_vec).unwrap_or_default()
}

impl MotionRaw {
    /// The motion as the chunk holds it; `None` for a version the editor does not read.
    #[must_use]
    pub fn model(&self) -> Option<OMotion> {
        decode(&self.motion)
    }

    /// A new chunk from an edited motion and the current time.
    #[must_use]
    pub fn from_model(m: &OMotion, t_current: f32) -> Self {
        Self { motion: Blob::from(encode(m).as_slice()), t_current: Some(t_current) }
    }
}

/// `SAnimParams` as the rows show them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub min_t: f32,
    pub max_t: f32,
    pub t_current: f32,
}

/// `SAnimParams::Set(motion)`, with the time the object was saved at.
#[must_use]
pub fn params(m: &OMotion, raw: &MotionRaw) -> Params {
    let fps = if m.fps == 0.0 { 30.0 } else { m.fps };
    let (min_t, max_t) = (m.frame_start as f32 / fps, m.frame_end as f32 / fps);
    Params { min_t, max_t, t_current: raw.t_current.unwrap_or(min_t) }
}

/// `SetParam(min*30, max*30, 30)`.
pub fn set_range(m: &mut OMotion, min_t: f32, max_t: f32) {
    m.frame_start = (min_t * 30.0) as i32;
    m.frame_end = (max_t * 30.0) as i32;
    m.fps = 30.0;
}

/// `_Evaluate`, as `AnimationUpdate` stands the object: the position and the negated angles.
#[must_use]
pub fn pose_at(m: &OMotion, t: f32) -> (Vec3, Vec3) {
    let (p, r) = m.evaluate(t);
    (p, -r)
}

/// `AnimationCreateKey`: a key of every channel at `t` from where the object stands.
pub fn create_key(m: &mut OMotion, t: f32, position: Vec3, rotation: Vec3) {
    let r = -rotation;
    let v = [position.x, position.y, position.z, r.y, r.x, r.z];
    for (e, val) in m.envelopes.iter_mut().zip(v) {
        e.insert_key(t, val);
    }
}

/// `COMotion::DeleteKey`.
pub fn delete_key(m: &mut OMotion, t: f32) {
    for e in &mut m.envelopes {
        e.delete_key(t);
    }
}

/// `CEnvelope::FindNearestKey` on the first channel: the keys before and after `t` (`t` itself
/// where there is none).
#[must_use]
pub fn nearest_keys(m: &OMotion, t: f32) -> (f32, f32) {
    let keys = &m.envelopes[0].keys;
    for (i, k) in keys.iter().enumerate() {
        let before = if i == 0 { 0 } else { i - 1 };
        if (k.time - t).abs() < EPS_L {
            let mn = keys.get(before).map_or(t, |k| k.time);
            let mx = keys.get(i + 1).map_or(t, |k| k.time);
            return (mn, mx);
        }
        if k.time > t {
            return (keys.get(before).map_or(t, |k| k.time), k.time);
        }
    }
    (keys.last().map_or(t, |k| k.time), t)
}

/// `CEnvelope::FindKey`.
fn find_key(keys: &[Key], t: f32, eps: f32) -> Option<usize> {
    for (i, k) in keys.iter().enumerate() {
        if (k.time - t).abs() < eps {
            return Some(i);
        }
        if k.time > t {
            return None;
        }
    }
    None
}

/// `CEnvelope::FindNearestKey` as indices (`None` = `keys.end()`).
fn nearest_idx(keys: &[Key], t: f32, eps: f32) -> (Option<usize>, Option<usize>) {
    for (i, k) in keys.iter().enumerate() {
        let min = Some(if i == 0 { i } else { i - 1 });
        if (k.time - t).abs() < eps {
            return (min, (i + 1 < keys.len()).then_some(i + 1));
        }
        if k.time > t {
            return (min, Some(i));
        }
    }
    (if keys.is_empty() { None } else { Some(keys.len() - 1) }, None)
}

/// `CEnvelope::ScaleKeys`.
fn scale_envelope(e: &mut Envelope, from: f32, to: f32, scale: f32, eps: f32) -> bool {
    let keys = &mut e.keys;
    let min_k = find_key(keys, from, eps).or_else(|| nearest_idx(keys, from, eps).1);
    let mut max_k = find_key(keys, to, eps).or_else(|| nearest_idx(keys, to, eps).0);
    let Some(min_k) = min_k else { return false };
    if Some(min_k) == max_k {
        return false;
    }
    if let Some(m) = max_k {
        max_k = (m + 1 < keys.len()).then_some(m + 1);
    }
    let end = max_k.unwrap_or(keys.len());
    let mut t0 = keys[min_k].time;
    let mut offset = 0.0f32;
    let mut it = min_k + 1;
    while it < end {
        let new_time = offset + t0 + (keys[it].time - t0) * scale;
        offset += (new_time - keys[it - 1].time) - (keys[it].time - t0);
        t0 = keys[it].time;
        keys[it].time = new_time;
        it += 1;
    }
    while it < keys.len() {
        let new_time = offset + keys[it].time;
        offset += (new_time - keys[it - 1].time) - (keys[it].time - t0);
        keys[it].time = new_time;
        it += 1;
    }
    true
}

/// `COMotion::ScaleKeys`: every channel until one refuses.
pub fn scale_keys(m: &mut OMotion, from: f32, to: f32, scale: f32) -> bool {
    let eps = 1.0 / if m.fps == 0.0 { 30.0 } else { m.fps };
    for e in &mut m.envelopes {
        if !scale_envelope(e, from, to, scale, eps) {
            return false;
        }
    }
    true
}

/// `COMotion::NormalizeKeys`: the keys between the two times moved so the object travels at
/// `speed` metres a second, the rest kept their spacing.
pub fn normalize_keys(m: &mut OMotion, from: f32, to: f32, speed: f32) -> bool {
    if to < from || m.envelopes[0].keys.is_empty() {
        return false;
    }
    let fps = if m.fps == 0.0 { 30.0 } else { m.fps };
    let times: Vec<f32> = m.envelopes[0].keys.iter().map(|k| k.time).collect();
    let mut new_tm = 0.0f32;
    let mut t0 = times[0];
    let mut tms = vec![t0];
    for &t in times.iter().skip(1) {
        if t > from {
            if t < to + 0.0001 {
                let mut dist = 0.0f32;
                let (mut pt, _) = m.evaluate(t0);
                let mut tm = t0 + 1.0 / fps;
                while tm <= t {
                    let (p, _) = m.evaluate(tm);
                    dist += pt.distance(p);
                    pt = p;
                    tm += EPS_L;
                }
                new_tm += dist / speed;
                t0 = t;
                tms.push(new_tm);
            } else {
                new_tm += t - t0;
                t0 = t;
                tms.push(new_tm);
            }
        }
    }
    for e in &mut m.envelopes {
        for (k, t) in e.keys.iter_mut().zip(&tms) {
            k.time = *t;
        }
    }
    true
}

/// `COMotion::GetLength(&mn, &mx)`: the longest channel's span and the first and last times of
/// the last channel that has keys (each channel writes them in turn, as in the C++).
#[must_use]
pub fn length(m: &OMotion) -> (f32, f32, f32) {
    let (mut len, mut mn, mut mx) = (0.0f32, 0.0f32, 0.0f32);
    for e in &m.envelopes {
        let (a, b) = match (e.keys.first(), e.keys.last()) {
            (Some(a), Some(b)) => (a.time, b.time),
            _ => (0.0, 0.0),
        };
        mn = a;
        mx = b;
        len = len.max(b - a);
    }
    (len, mn, mx)
}

/// What a motion row or button asks.
#[derive(Clone, Debug, PartialEq)]
pub enum MotionEdit {
    /// Flags\Motionable: a new motion, or none.
    Motionable(bool),
    /// Motion\Files Import: the `.anm`'s motion, the time at its start.
    Import(Box<OMotion>),
    AddKey,
    DeleteKey,
    ScaleKeys {
        from: f32,
        to: f32,
        scale: f32,
    },
    NormalizeKeys {
        from: f32,
        to: f32,
        speed: f32,
    },
    Clamp,
    FirstFrame,
    PreviousKey,
    PreviousFrame,
    NextFrame,
    NextKey,
    LastFrame,
    /// Pause: where playback stopped.
    At(f32),
    AutoKey(bool),
    CameraView(bool),
    /// Start / End Frame (sec).
    Range {
        min_t: Option<f32>,
        max_t: Option<f32>,
    },
    /// Current Frame (sec): the range grows to hold it.
    Current(f32),
    /// Key Time (sec): the key at the current time moved there.
    KeyTime(f32),
}

/// The object after an edit: its motion, its flags, where it stands (`None`: it stays), and
/// whether the edit is no undo step of its own (the controls).
pub struct Edited {
    pub motion: Option<Arc<MotionRaw>>,
    pub flags: u32,
    pub pose: Option<(Vec3, Vec3)>,
    pub label: &'static str,
    pub merge: bool,
}

/// Applies an edit to an object; `Err` says why it does not apply.
pub fn apply(o: &Object, edit: &MotionEdit) -> Result<Edited, String> {
    let flags = o.co_flags();
    if let MotionEdit::Motionable(on) = edit {
        return Ok(if *on {
            let m = OMotion::default();
            Edited {
                motion: Some(Arc::new(MotionRaw::from_model(&m, 0.0))),
                flags: flags | co::MOTION,
                pose: None,
                label: "Motionable",
                merge: false,
            }
        } else {
            Edited { motion: None, flags: flags & !co::MOTION, pose: None, label: "Motionable", merge: false }
        });
    }
    let raw = o
        .motion
        .as_ref()
        .filter(|_| flags & co::MOTION != 0)
        .ok_or_else(|| format!("'{}' has no motion: set Flags\\Motionable first.", o.name))?;
    let mut m =
        raw.model().ok_or_else(|| format!("CustomObject: '{}' - motion has different version.", o.name))?;
    let p = params(&m, raw);
    let mut t = p.t_current;
    let mut flags_out = flags;
    let mut pose = true;
    let (label, merge) = match edit {
        MotionEdit::Motionable(_) => ("Motionable", false),
        MotionEdit::Import(new) => {
            m = (**new).clone();
            t = params(&m, &MotionRaw { motion: Blob::from(&[][..]), t_current: None }).min_t;
            ("Import Motion", false)
        }
        MotionEdit::AddKey => {
            create_key(&mut m, t, o.position(), o.rotation());
            ("Add Key", false)
        }
        MotionEdit::DeleteKey => {
            delete_key(&mut m, t);
            ("Delete Key", false)
        }
        MotionEdit::ScaleKeys { from, to, scale } => {
            scale_keys(&mut m, *from, *to, *scale);
            let (_, _, mx) = length(&m);
            if p.max_t < mx {
                set_range(&mut m, p.min_t, mx);
            }
            ("Scale Keys", false)
        }
        MotionEdit::NormalizeKeys { from, to, speed } => {
            normalize_keys(&mut m, *from, *to, *speed);
            let (_, _, mx) = length(&m);
            if p.max_t < mx {
                set_range(&mut m, p.min_t, mx);
            }
            ("Normalize Keys", false)
        }
        MotionEdit::Clamp => {
            let (_, mn, mx) = length(&m);
            set_range(&mut m, mn, mx);
            ("Clamp Motion", false)
        }
        MotionEdit::FirstFrame => {
            t = p.min_t;
            ("Motion Frame", true)
        }
        MotionEdit::PreviousKey => {
            t = nearest_keys(&m, t).0;
            ("Motion Frame", true)
        }
        MotionEdit::PreviousFrame => {
            t -= FRAME;
            ("Motion Frame", true)
        }
        MotionEdit::NextFrame => {
            t += FRAME;
            ("Motion Frame", true)
        }
        MotionEdit::NextKey => {
            t = nearest_keys(&m, t).1;
            ("Motion Frame", true)
        }
        MotionEdit::LastFrame => {
            t = p.max_t;
            ("Motion Frame", true)
        }
        MotionEdit::At(at) => {
            t = *at;
            ("Motion Frame", true)
        }
        MotionEdit::AutoKey(on) => {
            flags_out = if *on { flags | AUTO_KEY } else { flags & !AUTO_KEY };
            pose = false;
            ("Auto Key", false)
        }
        MotionEdit::CameraView(on) => {
            flags_out = if *on { flags | CAMERA_VIEW } else { flags & !CAMERA_VIEW };
            ("Camera View", false)
        }
        MotionEdit::Range { min_t, max_t } => {
            set_range(&mut m, min_t.unwrap_or(p.min_t), max_t.unwrap_or(p.max_t));
            pose = false;
            ("Motion Range", false)
        }
        MotionEdit::Current(at) => {
            t = *at;
            let (mut mn, mut mx) = (p.min_t, p.max_t);
            if t < mn {
                mn = t;
            } else if t > mx {
                mx = t;
            }
            set_range(&mut m, mn, mx);
            ("Motion Frame", true)
        }
        MotionEdit::KeyTime(to) => {
            // `OnMotionKeyTimeChange`: the pose at the current time keyed again at the new one
            let (pos, rot) = m.evaluate(t);
            delete_key(&mut m, t);
            let v = [pos.x, pos.y, pos.z, rot.y, rot.x, rot.z];
            for (e, val) in m.envelopes.iter_mut().zip(v) {
                e.insert_key(*to, val);
            }
            pose = false;
            ("Key Time", false)
        }
    };
    let at = if pose { Some(pose_at(&m, t)) } else { None };
    Ok(Edited {
        motion: Some(Arc::new(MotionRaw::from_model(&m, t))),
        flags: flags_out,
        pose: at,
        label,
        merge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_made_is_where_the_object_stands() {
        let mut m = OMotion::default();
        create_key(&mut m, 0.0, Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.1, 0.2, 0.3));
        create_key(&mut m, 1.0, Vec3::new(3.0, 2.0, 1.0), Vec3::new(0.1, 0.2, 0.3));
        let (p, r) = pose_at(&m, 0.0);
        assert!((p - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5);
        assert!((r - Vec3::new(0.1, 0.2, 0.3)).length() < 1e-5);
        assert_eq!(nearest_keys(&m, 0.5), (0.0, 1.0));
        // the chunk reads back
        let raw = MotionRaw::from_model(&m, 0.25);
        assert_eq!(raw.model().unwrap().envelopes[0].keys.len(), 2);
    }

    #[test]
    fn scaling_doubles_the_spacing() {
        let mut m = OMotion::default();
        for t in [0.0, 1.0, 2.0] {
            create_key(&mut m, t, Vec3::splat(t), Vec3::ZERO);
        }
        assert!(scale_keys(&mut m, 0.0, 2.0, 2.0));
        let times: Vec<f32> = m.envelopes[0].keys.iter().map(|k| k.time).collect();
        assert_eq!(times, vec![0.0, 2.0, 4.0]);
    }
}
