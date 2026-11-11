//! How the editor holds state.
//!
//! A [`Store`] owns one value. Commands reach it through a queue and are applied one at a time
//! to a copy; the copy is published as an immutable snapshot only when the command succeeded.
//! Readers take the current snapshot with one atomic load, from any thread, and never wait.
//!
//! What follows from that, by construction:
//!
//! - no reader sees a half-applied edit, and a command that fails or panics changes nothing;
//! - undo is the previous snapshot - it cannot disagree with the model;
//! - a data race on editor state does not compile: there is no shared mutable state to race on.
//!
//! A state keeps its collections in persistent structures ([`imbl`]), so the copy a command
//! works on shares everything it does not touch.

mod error;
mod history;
mod store;

pub use error::CmdError;
pub use history::HistoryInfo;
pub use imbl;
pub use store::{Apply, Snap, State, Store, StoreOptions};

#[cfg(test)]
mod tests;
