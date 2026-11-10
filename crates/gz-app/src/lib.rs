//! The editor's window and the shell every tab lives in (docs/UI_DESIGN.md, sections 5 and 7).
//!
//! - [`window`] - the event loop and the frame; [`run`] opens the window
//! - [`ShellBuilder`] - what areas plug in before the window opens: document factories, menu
//!   rows, [`UiCommand`]s with their shortcuts, the project toolbar, the drawers' bodies, status
//!   chips, the Start page and the cards that take it over, dialogs by id, the Level panels
//! - [`Document`] - a major tab, drawn with a [`DocCx`]; it asks the shell for things through
//!   [`ShellRequests`]
//! - [`UiQueue`] / [`on_ui`] - how a command reaches the shell: a closure run on the UI thread at
//!   the start of the next frame
//! - [`TaskCardTable`] / [`TaskCard`] - what a job shows on the Task Center's card
//! - [`EditorSettings`] - the editor's own settings file: preferences, theme, window, layout,
//!   shortcuts
//! - [`register`] - the commands of the frame: `state`, `ui_*`, `window_control`,
//!   `titlebar_info`, `reset_layout`, `theme`, `command_palette`, `preferences`, `shortcuts`,
//!   `progress`, `screenshot_editor`
//!
//! Widgets the kit lacks, local to this crate: [`chrome::local_submenu`] (a menu row that opens a
//! submenu), `chrome::local_caption_button` (a caption menu's button) and
//! `chrome::local_window_button` (minimise, maximise, close).

pub mod builder;
pub mod chrome;
mod cmds;
pub mod contract;
pub mod doc;
pub mod keys;
pub mod palette;
pub mod project;
mod queue;
pub mod scroll;
pub mod settings;
mod shell;
mod shots;
pub mod tabs;
pub mod tasks;
mod window;

pub use builder::{DialogDesc, DrawerDesc, LogState, MenuContent, MenuEntry, PageCard, PanelDesc, ShellBuilder, UiCommand};
pub use cmds::register;
pub use doc::{DocCx, Document, Drawer, EditAction, OpenRequest, PanelInfo, PanelOp, ShellRequests, tab_id};
pub use keys::Chord;
pub use queue::{UiQueue, no_window, on_ui};
pub use settings::EditorSettings;
pub use scroll::{ScrollTo, report_scroll, take_scroll};
pub use shell::{GrizzlyShell, ShellOptions, ShellState, default_log_state, window_options};
pub use shots::encode_png;
pub use tasks::{TaskCard, TaskCardTable};
pub use window::{FrameReport, Shell, UserEvent, WindowLink, WindowOptions, run};
