//! The engine's two random sources: `CRandom` (`xrCore/_random.h`, the `::Random` that
//! `drand48()` reads) and the CRT's `rand()` that `NRand` and the noise table use. Both are the
//! same LCG; a system owns one of each so its draws do not depend on anything else running.

/// `CRandom` and MSVC's `rand()`: `holdrand = holdrand * 214013 + 2531011`, bits 16..31.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng {
    holdrand: i32,
}

impl Default for Rng {
    /// Both sources start from 1: `CRandom()` and a CRT nobody seeded.
    fn default() -> Self {
        Self::new(1)
    }
}

impl Rng {
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self { holdrand: seed }
    }

    /// `randI()`: 0..=32767.
    pub fn rand_i(&mut self) -> i32 {
        self.holdrand = self.holdrand.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.holdrand >> 16) & 0x7fff
    }

    /// `randI(max)`: `randI() % max`; the engine asserts `max != 0`, here it gives 0.
    pub fn rand_i_max(&mut self, max: i32) -> i32 {
        let r = self.rand_i();
        if max == 0 { 0 } else { r.wrapping_rem(max) }
    }

    /// `randI(min, max)`.
    pub fn rand_i_range(&mut self, min: i32, max: i32) -> i32 {
        min.wrapping_add(self.rand_i_max(max.wrapping_sub(min)))
    }

    /// `randF()`: 0..=1.
    pub fn rand_f(&mut self) -> f32 {
        self.rand_i() as f32 / 32767.0
    }

    /// `randF(min, max)`.
    pub fn rand_f_range(&mut self, min: f32, max: f32) -> f32 {
        min + self.rand_f() * (max - min)
    }

    /// `randFs(range)`: -range..=range.
    pub fn rand_fs(&mut self, range: f32) -> f32 {
        self.rand_f_range(-range, range)
    }
}

/// What an effect draws from: `::Random` (`drand48`) and the CRT's `rand()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rngs {
    pub random: Rng,
    pub crt: Rng,
}

impl Rngs {
    pub(crate) fn seeded(seed: i32) -> Self {
        Self { random: Rng::new(seed), crt: Rng::new(seed) }
    }

    /// `drand48()`.
    pub(crate) fn drand(&mut self) -> f32 {
        self.random.rand_f()
    }

    /// `PAPI::NRand`: a normal distribution of `sigma` by rejection, the sign from `rand()`.
    pub(crate) fn nrand(&mut self, sigma: f32) -> f32 {
        const ONE_OVER_SIGMA_EXP: f32 = 1.0 / 0.7975;
        if sigma == 0.0 {
            return 0.0;
        }
        let mut y;
        // the engine loops until it accepts; a bound keeps a broken source from hanging
        let mut tries = 0u32;
        loop {
            y = -self.drand().ln();
            tries += 1;
            if self.drand() <= (-(y - 1.0) * (y - 1.0) * 0.5).exp() || tries > 4096 {
                break;
            }
        }
        if self.crt.rand_i() & 1 != 0 {
            y * sigma * ONE_OVER_SIGMA_EXP
        } else {
            -y * sigma * ONE_OVER_SIGMA_EXP
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lcg_is_the_engines() {
        // the CRT's first values from the default seed: 41, 18467, 6334, 26500, 19169
        let mut r = Rng::default();
        let first: Vec<i32> = (0..5).map(|_| r.rand_i()).collect();
        assert_eq!(first, vec![41, 18467, 6334, 26500, 19169]);
        let mut r = Rng::default();
        assert!((r.rand_f() - 41.0 / 32767.0).abs() < 1e-9);
        let mut r = Rng::new(7);
        let a = r.rand_i_max(16);
        let mut s = Rng::new(7);
        assert_eq!(a, s.rand_i() % 16);
        assert_eq!(Rng::new(3).rand_i_max(0), 0);
    }

    #[test]
    fn nrand_is_signed_and_scaled() {
        let mut r = Rngs::default();
        assert_eq!(r.nrand(0.0), 0.0);
        // no draw for a zero sigma
        assert_eq!(r, Rngs::default());
        let v: Vec<f32> = (0..200).map(|_| r.nrand(1.0)).collect();
        assert!(v.iter().any(|x| *x > 0.0) && v.iter().any(|x| *x < 0.0));
        assert!(v.iter().all(|x| x.is_finite()));
    }
}
