//! The editor's window: the event loop, the frame, and what other threads need of it.

mod shots;
mod window;

pub use shots::{encode_png, register};
pub use window::{FrameReport, Shell, UserEvent, WindowLink, WindowOptions, run};
