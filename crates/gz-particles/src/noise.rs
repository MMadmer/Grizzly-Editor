//! Perlin's noise as `noise.cpp` has it: the table `noise3Init` builds from `srand(1)`, `noise3`
//! and `fractalsum3`. The table is data a [`crate::Library`] makes once and shares.

use glam::Vec3;

use crate::rand::Rng;

const B: usize = 256;
const N: usize = B + B + 2;

/// The gradient and permutation tables.
#[derive(Clone, Debug, PartialEq)]
pub struct Noise {
    p: Vec<usize>,
    g: Vec<[f32; 3]>,
}

impl Default for Noise {
    fn default() -> Self {
        Self::new()
    }
}

impl Noise {
    /// `noise3Init`: the tables from the CRT's `rand()` seeded with 1.
    #[must_use]
    pub fn new() -> Self {
        let mut rng = Rng::new(1);
        let mut p = vec![0i32; N];
        let mut g = vec![[0.0f32; 3]; N];
        for gi in g.iter_mut().take(B) {
            let mut v = [0.0f32; 3];
            loop {
                for slot in &mut v {
                    let rnd = rng.rand_i();
                    *slot = ((rnd % (B as i32 + B as i32)) - B as i32) as f32 / B as f32;
                }
                let s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                if s <= 1.0 {
                    let s = s.sqrt();
                    *gi = [v[0] / s, v[1] / s, v[2] / s];
                    break;
                }
            }
        }
        for (i, slot) in p.iter_mut().enumerate().take(B) {
            *slot = i as i32;
        }
        let mut i = B;
        while i > 0 {
            let rnd = rng.rand_i();
            let j = (rnd % B as i32) as usize;
            p.swap(i, j);
            i -= 2;
        }
        for i in 0..B + 2 {
            p[B + i] = p[i];
            g[B + i] = g[i];
        }
        Self { p: p.into_iter().map(|x| x.clamp(0, (N - 1) as i32) as usize).collect(), g }
    }

    fn pv(&self, i: usize) -> usize {
        self.p.get(i).copied().unwrap_or(0)
    }

    fn gv(&self, i: usize) -> [f32; 3] {
        self.g.get(i).copied().unwrap_or([0.0; 3])
    }

    /// `noise3`.
    #[must_use]
    pub fn noise3(&self, vec: Vec3) -> f32 {
        let setup = |x: f32| {
            let t = x + 10000.0;
            let f = t.floor();
            let b0 = (f as i32 & (B as i32 - 1)) as usize;
            let b1 = (b0 + 1) & (B - 1);
            let r0 = t - f;
            (b0, b1, r0, r0 - 1.0)
        };
        let (bx0, bx1, rx0, rx1) = setup(vec.x);
        let (by0, by1, ry0, ry1) = setup(vec.y);
        let (bz0, bz1, rz0, rz1) = setup(vec.z);
        let i = self.pv(bx0);
        let j = self.pv(bx1);
        let b00 = self.pv(i + by0);
        let b10 = self.pv(j + by0);
        let b01 = self.pv(i + by1);
        let b11 = self.pv(j + by1);
        let s_curve = |t: f32| t * t * (3.0 - 2.0 * t);
        let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
        let at = |q: [f32; 3], rx: f32, ry: f32, rz: f32| rx * q[0] + ry * q[1] + rz * q[2];
        let (sx, sy, sz) = (s_curve(rx0), s_curve(ry0), s_curve(rz0));

        let u = at(self.gv(b00 + bz0), rx0, ry0, rz0);
        let v = at(self.gv(b10 + bz0), rx1, ry0, rz0);
        let a = lerp(sx, u, v);
        let u = at(self.gv(b01 + bz0), rx0, ry1, rz0);
        let v = at(self.gv(b11 + bz0), rx1, ry1, rz0);
        let b = lerp(sx, u, v);
        let c = lerp(sy, a, b);

        let u = at(self.gv(b00 + bz1), rx0, ry0, rz1);
        let v = at(self.gv(b10 + bz1), rx1, ry0, rz1);
        let a = lerp(sx, u, v);
        let u = at(self.gv(b01 + bz1), rx0, ry1, rz1);
        let v = at(self.gv(b11 + bz1), rx1, ry1, rz1);
        let b = lerp(sx, u, v);
        let d = lerp(sy, a, b);

        1.5 * lerp(sz, c, d)
    }

    /// `fractalsum3`.
    #[must_use]
    pub fn fractalsum3(&self, v: Vec3, freq: f32, octaves: i32) -> f32 {
        let mut freq = freq;
        let boost = freq;
        let mut sum = 0.0f32;
        let mut at = v * freq;
        for _ in 0..octaves.max(0) {
            sum += self.noise3(at) / freq;
            freq *= 2.059;
            at = v * freq;
        }
        sum * boost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_a_permutation_and_unit_gradients() {
        let n = Noise::new();
        let mut seen = [false; B];
        for i in 0..B {
            seen[n.p[i]] = true;
            let g = n.g[i];
            let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "{len}");
            assert_eq!(n.p[B + i], n.p[i]);
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn noise_is_zero_on_the_lattice_and_smooth() {
        let n = Noise::new();
        // at an integer point every corner's offset is zero but one, and that one is zero too
        assert!(n.noise3(Vec3::new(3.0, 5.0, 7.0)).abs() < 1e-5);
        let a = n.noise3(Vec3::new(0.3, 0.4, 0.5));
        let b = n.noise3(Vec3::new(0.3001, 0.4, 0.5));
        assert!((a - b).abs() < 1e-2);
        assert!(a.abs() <= 1.5 * 3f32.sqrt());
        // one octave is noise3 at freq, times freq / freq
        let f = n.fractalsum3(Vec3::new(0.3, 0.4, 0.5), 2.0, 1);
        assert!((f - n.noise3(Vec3::new(0.6, 0.8, 1.0))).abs() < 1e-6);
        assert_eq!(n.fractalsum3(Vec3::ONE, 1.0, 0), 0.0);
    }
}
