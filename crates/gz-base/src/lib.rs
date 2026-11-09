//! The vocabulary every crate of the editor shares: revisions, units, paths, text and problems.
//!
//! Nothing here owns state or starts a thread.

pub mod origin;
pub mod path;
pub mod problem;
pub mod rev;
pub mod text;
pub mod units;

pub use origin::Origin;
pub use path::{GamePath, PathError, ProjectPath, RelPath};
pub use problem::{Problem, Severity};
pub use rev::{Rev, RevCounter};
pub use smol_str::SmolStr;
pub use units::{Pt, Px, Scale};

/// Immutable bytes shared between threads.
pub type Blob = std::sync::Arc<[u8]>;
