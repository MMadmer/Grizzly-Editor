//! A terrain of the scene (`docs/assets/SCENE.md`): a height
//! grid over a rectangle, made from its text and nothing else.
//!
//! - [`Terrain`] - the centre, the heading and the [`Spec`] the file says: size, cell, [`Base`],
//!   edge, [`Edit`]s or rows, [`Surface`], cut. [`Terrain::check`] says what is wrong with it in a
//!   sentence; [`Terrain::heights`] evaluates the grid from the base heights the caller samples
//!   ([`sample_ground`]); [`Terrain::mesh`] makes the model the editor draws, hits and compiles.
//! - [`clip`] - the game's exact cut of a triangle by a region, ported line by line, and
//!   [`clip::clip_mesh`] over a triangle soup.
//! - [`files`] - `overlay.cut` and `overlay.aiedit`, written and read as the game reads them.
//!
//! Everything is pure and deterministic: the same text and the same ground give the same bytes.

pub mod clip;
pub mod files;
mod grid;
mod ground;
mod mesh;
mod model;
#[cfg(test)]
mod tests;

pub use clip::{ClipResult, Corner, CutKind, Piece, Region, clip_mesh, clip_triangle};
pub use files::{AiEdit, read_ai_edits, read_cuts, write_ai_edits, write_cuts};
pub use glam;
pub use grid::Grid;
pub use ground::{GroundSample, GroundTri, Hit, sample_ground};
pub use mesh::{FaceLook, Looks};
pub use model::{
    Base, CELL_MAX, CELL_MIN, EDGE_MAX, Edit, Look, MAX_POINTS, SIZE_MAX, SKIRT_DROP, SKIRT_WIDTH, Spec,
    Surface, TOLERANCE, Terrain,
};
