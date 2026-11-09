//! LightWave envelopes: keys, their two file forms and the evaluation (`Envelope.cpp`,
//! `interp.cpp`).

use gz_chunk::{Reader, Writer};

use crate::Result;
use crate::io::{checked_count, w_float_q16};
use crate::xmath::EPS;

/// Key shapes: how the span that ends at a key is interpolated.
pub mod shape {
    pub const TCB: u8 = 0;
    pub const HERMITE: u8 = 1;
    pub const BEZIER: u8 = 2;
    pub const LINEAR: u8 = 3;
    pub const STEPPED: u8 = 4;
    pub const BEZIER_2D: u8 = 5;
}

/// What an envelope does before its first key and after its last.
pub mod behavior {
    pub const RESET: i32 = 0;
    pub const CONSTANT: i32 = 1;
    pub const REPEAT: i32 = 2;
    pub const OSCILLATE: i32 = 3;
    pub const OFFSET: i32 = 4;
    pub const LINEAR: i32 = 5;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Key {
    pub value: f32,
    pub time: f32,
    pub shape: u8,
    pub tension: f32,
    pub continuity: f32,
    pub bias: f32,
    pub param: [f32; 4],
}

impl Key {
    fn read_v1(r: &mut Reader<'_>) -> Result<Key> {
        let value = r.r_f32()?;
        let time = r.r_f32()?;
        let shape = (r.r_u32()? & 0xff) as u8;
        let tension = r.r_f32()?;
        let continuity = r.r_f32()?;
        let bias = r.r_f32()?;
        let param = [r.r_f32()?, r.r_f32()?, r.r_f32()?, r.r_f32()?];
        Ok(Key { value, time, shape, tension, continuity, bias, param })
    }

    fn read_v2(r: &mut Reader<'_>) -> Result<Key> {
        let mut k = Key { value: r.r_f32()?, time: r.r_f32()?, shape: r.r_u8()?, ..Key::default() };
        if k.shape != shape::STEPPED {
            k.tension = r.r_float_q16(-32.0, 32.0)?;
            k.continuity = r.r_float_q16(-32.0, 32.0)?;
            k.bias = r.r_float_q16(-32.0, 32.0)?;
            for p in &mut k.param {
                *p = r.r_float_q16(-32.0, 32.0)?;
            }
        }
        Ok(k)
    }

    fn write(&self, w: &mut Writer) {
        w.w_f32(self.value);
        w.w_f32(self.time);
        w.w_u8(self.shape);
        if self.shape != shape::STEPPED {
            w_float_q16(w, self.tension, -32.0, 32.0);
            w_float_q16(w, self.continuity, -32.0, 32.0);
            w_float_q16(w, self.bias, -32.0, 32.0);
            for p in self.param {
                w_float_q16(w, p, -32.0, 32.0);
            }
        }
    }

    /// `st_Key::equal`: everything but the time, within the engine's epsilon.
    fn equal(&self, o: &Key) -> bool {
        let sim = |a: f32, b: f32| (a - b).abs() < EPS;
        sim(self.value, o.value)
            && sim(f32::from(self.shape), f32::from(o.shape))
            && sim(self.tension, o.tension)
            && sim(self.continuity, o.continuity)
            && sim(self.bias, o.bias)
            && (0..4).all(|i| sim(self.param[i], o.param[i]))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    /// Pre and post behaviour, see [`behavior`].
    pub behavior: [i32; 2],
    pub keys: Vec<Key>,
}

impl Default for Envelope {
    fn default() -> Self {
        Self { behavior: [behavior::CONSTANT, behavior::CONSTANT], keys: Vec::new() }
    }
}

impl Envelope {
    /// The first file form: 32-bit fields, floats as they are.
    pub(crate) fn read_v1(r: &mut Reader<'_>) -> Result<Envelope> {
        let behavior = [r.r_i32()?, r.r_i32()?];
        let count = checked_count(r, r.clone().r_u32()? as usize, 40)?;
        r.advance(4)?;
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            keys.push(Key::read_v1(r)?);
        }
        Ok(Envelope { behavior, keys })
    }

    /// The current file form: bytes for the behaviours, 16-bit steps for the key parameters.
    pub(crate) fn read_v2(r: &mut Reader<'_>) -> Result<Envelope> {
        let behavior = [i32::from(r.r_u8()?), i32::from(r.r_u8()?)];
        let count = checked_count(r, usize::from(r.clone().r_u16()?), 9)?;
        r.advance(2)?;
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            keys.push(Key::read_v2(r)?);
        }
        Ok(Envelope { behavior, keys })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.w_u8(self.behavior[0] as u8);
        w.w_u8(self.behavior[1] as u8);
        w.w_u16(self.keys.len() as u16);
        for k in &self.keys {
            k.write(w);
        }
    }

    /// `CEnvelope::Evaluate`: the value at `time`.
    #[must_use]
    pub fn evaluate(&self, mut time: f32) -> f32 {
        let keys = &self.keys;
        let (Some(skey), Some(ekey)) = (keys.first(), keys.last()) else {
            return 0.0;
        };
        let sz = keys.len();
        if sz == 1 {
            return skey.value;
        }
        let (skey_n, ekey_p) = (&keys[1], &keys[sz - 2]);
        let mut offset = 0.0f32;

        if time < skey.time {
            match self.behavior[0] {
                behavior::RESET => return 0.0,
                behavior::CONSTANT => return skey.value,
                behavior::REPEAT => time = range(time, skey.time, ekey.time).0,
                behavior::OSCILLATE => {
                    let (t, noff) = range(time, skey.time, ekey.time);
                    time = t;
                    if noff % 2 != 0 {
                        time = ekey.time - skey.time - time;
                    }
                }
                behavior::OFFSET => {
                    let (t, noff) = range(time, skey.time, ekey.time);
                    time = t;
                    offset = noff as f32 * (ekey.value - skey.value);
                }
                behavior::LINEAR => {
                    let out = outgoing(None, skey, skey_n) / (skey_n.time - skey.time);
                    return out * (time - skey.time) + skey.value;
                }
                _ => {}
            }
        } else if time > ekey.time {
            match self.behavior[1] {
                behavior::RESET => return 0.0,
                behavior::CONSTANT => return ekey.value,
                behavior::REPEAT => time = range(time, skey.time, ekey.time).0,
                behavior::OSCILLATE => {
                    let (t, noff) = range(time, skey.time, ekey.time);
                    time = t;
                    if noff % 2 != 0 {
                        time = ekey.time - skey.time - time;
                    }
                }
                behavior::OFFSET => {
                    let (t, noff) = range(time, skey.time, ekey.time);
                    time = t;
                    offset = noff as f32 * (ekey.value - skey.value);
                }
                behavior::LINEAR => {
                    let inc = incoming(ekey_p, ekey, None) / (ekey.time - ekey_p.time);
                    return inc * (time - ekey.time) + ekey.value;
                }
                _ => {}
            }
        }

        // the span that holds the time; the C++ walks off the end when an oscillation leaves
        // the key range, this stops at the last span
        let mut k = 0usize;
        while k + 2 < sz && time > keys[k + 1].time {
            k += 1;
        }
        let key0 = &keys[k];
        let key1 = &keys[k + 1];
        let key0_p = if k > 0 { Some(&keys[k - 1]) } else { None };
        let key1_n = keys.get(k + 2);

        if time == key0.time {
            return key0.value + offset;
        } else if time == key1.time {
            return key1.value + offset;
        }

        let t = (time - key0.time) / (key1.time - key0.time);
        match key1.shape {
            shape::TCB | shape::BEZIER | shape::HERMITE => {
                let out = outgoing(key0_p, key0, key1);
                let inc = incoming(key0, key1, key1_n);
                let (h1, h2, h3, h4) = hermite(t);
                h1 * key0.value + h2 * key1.value + h3 * out + h4 * inc + offset
            }
            shape::BEZIER_2D => bez2(key0, key1, time) + offset,
            shape::LINEAR => key0.value + t * (key1.value - key0.value) + offset,
            shape::STEPPED => key0.value + offset,
            _ => offset,
        }
    }

    /// `InsertKey`: sets the value of the key at `t`, or adds a TCB key there.
    pub fn insert_key(&mut self, t: f32, value: f32) {
        let mut at = self.keys.len();
        for (i, k) in self.keys.iter_mut().enumerate() {
            if (k.time - t).abs() < crate::xmath::EPS_L {
                k.value = value;
                return;
            }
            if k.time > t {
                at = i;
                break;
            }
        }
        self.behavior = [behavior::CONSTANT, behavior::CONSTANT];
        self.keys.insert(at, Key { time: t, value, shape: shape::TCB, ..Key::default() });
    }

    /// `DeleteKey`: removes the key at `t`.
    pub fn delete_key(&mut self, t: f32) {
        if let Some(i) = self.keys.iter().position(|k| (k.time - t).abs() < crate::xmath::EPS_L) {
            self.keys.remove(i);
        }
    }

    /// `RotateKeys`: adds an angle to every value.
    pub fn rotate_keys(&mut self, angle: f32) {
        for k in &mut self.keys {
            k.value += angle;
        }
    }

    /// `GetLength`: the time between the first and the last key.
    #[must_use]
    pub fn length(&self) -> f32 {
        match (self.keys.first(), self.keys.last()) {
            (Some(a), Some(b)) => b.time - a.time,
            _ => 0.0,
        }
    }

    /// `Optimize`: an envelope whose keys all hold the same thing keeps the first and the last.
    pub fn optimize(&mut self) {
        let Some(first) = self.keys.first().copied() else { return };
        let equal = self.keys.iter().skip(1).all(|k| k.equal(&first));
        if equal && self.keys.len() > 2 {
            let last = self.keys[self.keys.len() - 1];
            self.keys = vec![first, last];
        }
    }
}

/// The equivalent of `v` in `[lo, hi]` and the number of wavelengths between the two.
fn range(v: f32, lo: f32, hi: f32) -> (f32, i32) {
    let r = hi - lo;
    if r == 0.0 {
        return (lo, 0);
    }
    let v2 = lo + v - r * (v / r).floor();
    let i = -((f64::from((v2 - v) / r) + if v2 > v { 0.5 } else { -0.5 }) as i32);
    (v2, i)
}

fn hermite(t: f32) -> (f32, f32, f32, f32) {
    let t2 = t * t;
    let t3 = t * t2;
    let h2 = 3.0 * t2 - t3 - t3;
    let h1 = 1.0 - h2;
    let h4 = t3 - t2;
    let h3 = h4 - t2 + t;
    (h1, h2, h3, h4)
}

fn bezier(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let c = 3.0 * (x1 - x0);
    let b = 3.0 * (x2 - x1) - c;
    let a = x3 - x0 - c - b;
    a * t3 + b * t2 + c * t + x0
}

/// The parameter at which the time curve of a 2D Bezier span passes `time`, by bisection.
fn bez2_time(x0: f32, x1: f32, x2: f32, x3: f32, time: f32) -> f32 {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    // the C++ recurses until it is within tolerance; a float interval is exhausted long
    // before this many halvings
    for _ in 0..64 {
        let t = t0 + (t1 - t0) * 0.5;
        let v = bezier(x0, x1, x2, x3, t);
        if (time - v).abs() > 0.0001 {
            if v > time {
                t1 = t;
            } else {
                t0 = t;
            }
        } else {
            return t;
        }
    }
    t0 + (t1 - t0) * 0.5
}

fn bez2(key0: &Key, key1: &Key, time: f32) -> f32 {
    let x = if key0.shape == shape::BEZIER_2D {
        key0.time + key0.param[2]
    } else {
        key0.time + (key1.time - key0.time) / 3.0
    };
    let t = bez2_time(key0.time, x, key1.time + key1.param[0], key1.time, time);
    let y = if key0.shape == shape::BEZIER_2D { key0.value + key0.param[3] } else { key0.value + key0.param[1] / 3.0 };
    bezier(key0.value, y, key1.param[1] + key1.value, key1.value, t)
}

/// The outgoing tangent at `key0`.
fn outgoing(key0p: Option<&Key>, key0: &Key, key1: &Key) -> f32 {
    match key0.shape {
        shape::TCB => {
            let a = (1.0 - key0.tension) * (1.0 + key0.continuity) * (1.0 + key0.bias);
            let b = (1.0 - key0.tension) * (1.0 - key0.continuity) * (1.0 - key0.bias);
            let d = key1.value - key0.value;
            match key0p {
                Some(p) => {
                    let t = (key1.time - key0.time) / (key1.time - p.time);
                    t * (a * (key0.value - p.value) + b * d)
                }
                None => b * d,
            }
        }
        shape::LINEAR => {
            let d = key1.value - key0.value;
            match key0p {
                Some(p) => {
                    let t = (key1.time - key0.time) / (key1.time - p.time);
                    t * (key0.value - p.value + d)
                }
                None => d,
            }
        }
        shape::BEZIER | shape::HERMITE => {
            let mut out = key0.param[1];
            if let Some(p) = key0p {
                out *= (key1.time - key0.time) / (key1.time - p.time);
            }
            out
        }
        shape::BEZIER_2D => {
            let mut out = key0.param[3] * (key1.time - key0.time);
            if key0.param[2].abs() > 1e-5 {
                out /= key0.param[2];
            } else {
                out *= 1e5;
            }
            out
        }
        _ => 0.0,
    }
}

/// The incoming tangent at `key1`.
fn incoming(key0: &Key, key1: &Key, key1n: Option<&Key>) -> f32 {
    match key1.shape {
        shape::LINEAR => {
            let d = key1.value - key0.value;
            match key1n {
                Some(n) => {
                    let t = (key1.time - key0.time) / (n.time - key0.time);
                    t * (n.value - key1.value + d)
                }
                None => d,
            }
        }
        shape::TCB => {
            let a = (1.0 - key1.tension) * (1.0 - key1.continuity) * (1.0 + key1.bias);
            let b = (1.0 - key1.tension) * (1.0 + key1.continuity) * (1.0 - key1.bias);
            let d = key1.value - key0.value;
            match key1n {
                Some(n) => {
                    let t = (key1.time - key0.time) / (n.time - key0.time);
                    t * (b * (n.value - key1.value) + a * d)
                }
                None => a * d,
            }
        }
        shape::BEZIER | shape::HERMITE => {
            let mut inc = key1.param[0];
            if let Some(n) = key1n {
                inc *= (key1.time - key0.time) / (n.time - key0.time);
            }
            inc
        }
        shape::BEZIER_2D => {
            let mut inc = key1.param[1] * (key1.time - key0.time);
            if key1.param[0].abs() > 1e-5 {
                inc /= key1.param[0];
            } else {
                inc *= 1e5;
            }
            inc
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, shape: u8) -> Key {
        Key { time, value, shape, ..Key::default() }
    }

    fn env(keys: &[Key]) -> Envelope {
        Envelope { keys: keys.to_vec(), ..Envelope::default() }
    }

    #[test]
    fn empty_and_single_key() {
        assert_eq!(Envelope::default().evaluate(3.0), 0.0);
        assert_eq!(env(&[key(1.0, 7.5, shape::TCB)]).evaluate(-4.0), 7.5);
    }

    #[test]
    fn linear_and_stepped_spans() {
        let e = env(&[key(0.0, 0.0, shape::LINEAR), key(2.0, 10.0, shape::LINEAR), key(4.0, 20.0, shape::STEPPED)]);
        // t = 0.25 of the first span
        assert_eq!(e.evaluate(0.5), 2.5);
        // a stepped key holds the previous value until it is reached
        assert_eq!(e.evaluate(3.0), 10.0);
        // the keys themselves
        assert_eq!(e.evaluate(2.0), 10.0);
        assert_eq!(e.evaluate(4.0), 20.0);
        // constant before and after
        assert_eq!(e.evaluate(-1.0), 0.0);
        assert_eq!(e.evaluate(9.0), 20.0);
    }

    #[test]
    fn tcb_span_of_two_keys_is_a_hermite_with_equal_tangents() {
        // tension = continuity = bias = 0 and no neighbours: out = in = d = 10, so
        // f(t) = h2*10 + (h3 + h4)*10 with t = 0.5: h2 = 0.5, h3 = 0.125, h4 = -0.125
        let e = env(&[key(0.0, 0.0, shape::TCB), key(1.0, 10.0, shape::TCB)]);
        assert_eq!(e.evaluate(0.5), 5.0);
        // t = 0.25: h2 = 3/16 - 2/64 = 0.15625, h3 = 0.140625, h4 = -0.046875
        // f = 1.5625 + 1.40625 - 0.46875 = 2.5
        assert!((e.evaluate(0.25) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn hermite_keys_use_their_stored_tangents() {
        let mut k0 = key(0.0, 0.0, shape::HERMITE);
        let mut k1 = key(1.0, 0.0, shape::HERMITE);
        k0.param[1] = 4.0; // outgoing
        k1.param[0] = 0.0; // incoming
        // f(0.5) = h3 * 4 = 0.125 * 4
        assert_eq!(env(&[k0, k1]).evaluate(0.5), 0.5);
    }

    #[test]
    fn pre_and_post_behaviours() {
        let keys = [key(0.0, 0.0, shape::LINEAR), key(2.0, 4.0, shape::LINEAR)];
        let mut e = env(&keys);
        e.behavior = [behavior::RESET, behavior::RESET];
        assert_eq!(e.evaluate(-1.0), 0.0);
        assert_eq!(e.evaluate(5.0), 0.0);
        e.behavior = [behavior::REPEAT, behavior::REPEAT];
        // 2.5 wraps to 0.5 -> 1.0
        assert_eq!(e.evaluate(2.5), 1.0);
        assert_eq!(e.evaluate(-1.5), 1.0);
        e.behavior = [behavior::OFFSET, behavior::OFFSET];
        // one wavelength further: the wrapped value plus the rise of the envelope
        assert_eq!(e.evaluate(2.5), 5.0);
        e.behavior = [behavior::LINEAR, behavior::LINEAR];
        // the slope of the last span continues: 4 / 2 per unit of time
        assert_eq!(e.evaluate(3.0), 6.0);
        assert_eq!(e.evaluate(-1.0), -2.0);
        e.behavior = [behavior::OSCILLATE, behavior::OSCILLATE];
        // 2.5 lies one wavelength out: mirrored to 2 - 0.5 = 1.5 -> 3.0
        assert_eq!(e.evaluate(2.5), 3.0);
    }

    #[test]
    fn bezier_2d_span_is_reached_by_bisection() {
        // straight handles: the curve is the line between the keys
        let mut k0 = key(0.0, 0.0, shape::BEZIER_2D);
        let mut k1 = key(3.0, 3.0, shape::BEZIER_2D);
        k0.param[2] = 1.0;
        k0.param[3] = 1.0;
        k1.param[0] = -1.0;
        k1.param[1] = -1.0;
        let v = env(&[k0, k1]).evaluate(1.5);
        assert!((v - 1.5).abs() < 1e-3);
    }

    #[test]
    fn both_file_forms_read_and_the_current_one_round_trips() {
        let mut e = env(&[key(0.0, 1.0, shape::TCB), key(1.0, 2.0, shape::STEPPED), key(2.0, 3.0, shape::LINEAR)]);
        e.keys[0].tension = 0.5;
        e.keys[0].param = [1.0, -1.0, 0.25, 31.0];
        let mut w = Writer::new();
        e.write(&mut w);
        let bytes = w.into_bytes();
        // 4 + (9 + 14) + 9 + (9 + 14)
        assert_eq!(bytes.len(), 59);
        let back = Envelope::read_v2(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(back.keys.len(), 3);
        assert!((back.keys[0].tension - 0.5).abs() < 1e-3);
        let mut w2 = Writer::new();
        back.write(&mut w2);
        assert_eq!(w2.into_bytes(), bytes);

        let mut v1 = Writer::new();
        v1.w_i32(2);
        v1.w_i32(1);
        v1.w_u32(1);
        v1.w_f32(5.0);
        v1.w_f32(0.5);
        v1.w_u32(3);
        for _ in 0..7 {
            v1.w_f32(0.0);
        }
        let bytes = v1.into_bytes();
        let old = Envelope::read_v1(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(old.behavior, [2, 1]);
        assert_eq!(old.keys[0].shape, shape::LINEAR);
        assert_eq!(old.keys[0].value, 5.0);
    }

    #[test]
    fn a_huge_key_count_is_refused() {
        let bytes = [1u8, 1, 0xff, 0xff, 0, 0, 0, 0];
        assert!(Envelope::read_v2(&mut Reader::new(&bytes)).is_err());
    }

    #[test]
    fn editing_keeps_keys_in_time_order() {
        let mut e = Envelope::default();
        e.insert_key(1.0, 1.0);
        e.insert_key(0.0, 0.0);
        e.insert_key(1.0, 5.0);
        assert_eq!(e.keys.len(), 2);
        assert_eq!(e.keys[1].value, 5.0);
        e.insert_key(2.0, 5.0);
        e.insert_key(3.0, 5.0);
        e.delete_key(0.0);
        e.optimize();
        assert_eq!(e.keys.len(), 2);
        assert_eq!(e.length(), 2.0);
    }
}
