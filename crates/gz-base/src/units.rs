use std::ops::{Add, Div, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

/// A length in points: what `docs/UI_DESIGN.md` is written in. 1 pt is 1 px at 100%.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pt(pub f32);

/// A length in physical pixels: what a screenshot and a hit test are measured in.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Px(pub f32);

/// Pixels per point. The one place DPI enters.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scale(pub f32);

impl Default for Scale {
    fn default() -> Self {
        Scale(1.0)
    }
}

impl Pt {
    pub const ZERO: Pt = Pt(0.0);

    #[must_use]
    pub fn to_px(self, scale: Scale) -> Px {
        Px(self.0 * scale.0)
    }

    #[must_use]
    pub fn max(self, other: Pt) -> Pt {
        Pt(self.0.max(other.0))
    }

    #[must_use]
    pub fn min(self, other: Pt) -> Pt {
        Pt(self.0.min(other.0))
    }
}

impl Px {
    #[must_use]
    pub fn to_pt(self, scale: Scale) -> Pt {
        Pt(self.0 / scale.0)
    }

    /// The pixel grid position, for an answer that names whole pixels.
    #[must_use]
    pub fn round(self) -> i32 {
        self.0.round() as i32
    }
}

macro_rules! arith {
    ($t:ident) => {
        impl Add for $t {
            type Output = $t;
            fn add(self, o: $t) -> $t {
                $t(self.0 + o.0)
            }
        }
        impl Sub for $t {
            type Output = $t;
            fn sub(self, o: $t) -> $t {
                $t(self.0 - o.0)
            }
        }
        impl Mul<f32> for $t {
            type Output = $t;
            fn mul(self, k: f32) -> $t {
                $t(self.0 * k)
            }
        }
        impl Div<f32> for $t {
            type Output = $t;
            fn div(self, k: f32) -> $t {
                $t(self.0 / k)
            }
        }
        impl Neg for $t {
            type Output = $t;
            fn neg(self) -> $t {
                $t(-self.0)
            }
        }
    };
}
arith!(Pt);
arith!(Px);
