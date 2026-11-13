//! The two system calls the update needs beyond `std`, from `gz-win`.

use std::io;
use std::path::Path;

/// A rename on one volume that never copies and never replaces.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        gz_win::rename_in_place(from, to)
    }
    #[cfg(not(windows))]
    {
        if to.exists() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        std::fs::rename(from, to)
    }
}

/// The event a restarted editor signals once it is up: named after its process id.
#[must_use]
pub fn started_event_name(pid: u32) -> String {
    format!("Local\\GrizzlyEditor-updated-{pid}")
}

#[cfg(windows)]
pub use gz_win::NamedEvent as Event;

/// Without Windows there is no restart handshake.
#[cfg(not(windows))]
#[derive(Debug)]
pub struct Event;

#[cfg(not(windows))]
impl Event {
    pub fn open(_name: &str) -> io::Result<Self> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    pub fn set(&self) -> io::Result<()> {
        Ok(())
    }
    #[must_use]
    pub fn is_set(&self) -> bool {
        false
    }
}
