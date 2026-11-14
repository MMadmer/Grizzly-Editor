//! Gather, light, scatter: what every consumer of the baker shares. A consumer gathers the sample
//! points of some of its items (texels, vertices, slots) with where each item's samples are, has
//! them lit in one call, and scatters the lights back. A batch holds about [`BATCH`] samples, so
//! the memory of a level of 10^9 samples stays bounded.

use crate::bake::{LightJob, LitScene, Sample};
use crate::color::ColorC;
use crate::{Ctl, Result};

/// The samples a consumer gathers before it has them lit.
pub const BATCH: usize = 1 << 20;

/// The samples of one item: `count` of them from `first`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Run {
    /// What the item is to its consumer: a texel's index, a vertex's.
    pub item: u32,
    pub first: u32,
    pub count: u32,
}

/// Samples and the runs of the items they belong to.
#[derive(Clone, Debug, Default)]
pub struct Gathered {
    pub samples: Vec<Sample>,
    pub runs: Vec<Run>,
}

impl Gathered {
    /// Starts an item's run; [`Self::end`] closes it.
    pub fn begin(&self) -> u32 {
        self.samples.len() as u32
    }

    /// Closes the run started at `first`.
    pub fn end(&mut self, item: u32, first: u32) {
        let count = self.samples.len() as u32 - first;
        self.runs.push(Run { item, first, count });
    }

    /// Another consumer's gathering appended: its runs moved by the samples before it.
    pub fn append(&mut self, other: Gathered) {
        let base = self.samples.len() as u32;
        self.samples.extend(other.samples);
        self.runs.extend(other.runs.into_iter().map(|r| Run { first: r.first + base, ..r }));
    }
}

/// The samples lit in `scene`, one light per sample.
pub fn light_samples(
    scene: &dyn LitScene,
    job: &LightJob<'_>,
    samples: &[Sample],
    ctl: &Ctl<'_>,
) -> Result<Vec<ColorC>> {
    let mut out = vec![ColorC::default(); samples.len()];
    if !samples.is_empty() {
        scene.light(job, samples, &mut out, ctl)?;
    }
    ctl.check()?;
    Ok(out)
}

/// The sum of a run's lights in the order of its samples: what `light_point` would have added
/// into one colour sample after sample, up to the rounding of the sums.
#[must_use]
pub fn sum(lit: &[ColorC], run: &Run) -> ColorC {
    let mut c = ColorC::default();
    let (a, b) = (run.first as usize, run.first as usize + run.count as usize);
    for l in lit.get(a..b).unwrap_or_default() {
        c.add(l);
    }
    c
}
