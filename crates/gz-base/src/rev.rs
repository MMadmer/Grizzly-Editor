use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// A revision of a store or of a watched input. Monotonic; equal revisions mean equal content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rev(pub u64);

impl Rev {
    pub const ZERO: Rev = Rev(0);

    #[must_use]
    pub fn next(self) -> Rev {
        Rev(self.0 + 1)
    }

    /// Folds several revisions into one fingerprint, for a query that reads several inputs.
    #[must_use]
    pub fn mix(self, other: Rev) -> Rev {
        // splitmix64 over the pair: order matters, collisions are as rare as a 64-bit hash's
        let mut z = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ other.0.wrapping_add(0xD1B5_4A32_D192_ED03);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rev(z ^ (z >> 31))
    }
}

/// A revision that several threads bump.
#[derive(Debug, Default)]
pub struct RevCounter(AtomicU64);

impl RevCounter {
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    pub fn get(&self) -> Rev {
        Rev(self.0.load(Ordering::Acquire))
    }

    /// Bumps and returns the new revision.
    pub fn bump(&self) -> Rev {
        Rev(self.0.fetch_add(1, Ordering::AcqRel) + 1)
    }
}
