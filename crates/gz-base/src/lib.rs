//! The vocabulary every crate of the editor shares: revisions, units, paths, text and problems.
//!
//! Nothing here owns state or starts a thread.

pub mod origin;
pub mod path;
pub mod problem;
pub mod rev;
pub mod text;
pub mod units;
pub mod web;

pub use origin::Origin;
pub use path::{PathError, RelPath, file_name_problem};
pub use problem::{Finding, Findings, Problem, Severity};
pub use rev::{Rev, RevCounter};
pub use smol_str::SmolStr;
pub use text::{io_reason, os_reason};
pub use units::{Pt, Px, Scale};

/// Immutable bytes shared between threads.
pub type Blob = std::sync::Arc<[u8]>;
