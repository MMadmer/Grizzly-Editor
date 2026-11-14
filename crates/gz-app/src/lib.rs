//! The editor's window and the shell every tab lives in (docs/UI_DESIGN.md, sections 5 and 7).
//!
//! - [`window`] - the event loop and the frame; [`run`] opens the window
//! - [`ShellBuilder`] - what areas plug in before the window opens: document factories,
//!   [`Action`]s (what menus, toolbars, keys and the palette run), menu rows, the project toolbar,
//!   the drawers' bodies, status chips, the Start page and the cards that take it over, dialogs by
//!   id, the Level panels, groups of Preferences
//! - [`actions`] - [`Action`] (a registry command, its arguments from [`ActionCx`], a title, an
//!   icon, keys, a group, a [`Scope`]) and the table the Keyboard page rebinds
//! - [`Document`] - a major tab, drawn with a [`DocCx`]; it asks the shell for things through
//!   [`ShellRequests`]
//! - [`UiQueue`] / [`on_ui`] - how a command reaches the shell: a closure run on the UI thread at
//!   the start of the next frame
//! - [`TaskCardTable`] / [`TaskCard`] - what a job shows on the Task Center's card
//! - [`EditorSettings`] - the editor's own settings file: preferences, theme, window, layout,
//!   shortcuts
//! - [`register`] - the commands of the window: `state`, `ui.*`, `app.*`, `doc.*`, `shot.window`,
//!   `perf.stalls`
//! - [`stalls`] - the stall monitor: every UI pass and event over 100 ms, for `perf.stalls`
//! - [`gpu`] - the adapter the device is made on and the driver's hint (View > Render > GPU);
//!   the sounds' mute switch
//! - Panels outside the window: the window runs egui's immediate viewports as native windows of
//!   their own while `[ui] floating_panels` is on (read at start); `shot.window` names them
//!   in `outside`
//!
//! Widgets the kit lacks, local to this crate: [`chrome::local_submenu`] (a menu row that opens a
//! submenu), `chrome::local_caption_button` (a caption menu's button) and
//! `chrome::local_window_button` (minimise, maximise, close).

pub mod actions;
mod app_cmds;
pub mod builder;
pub mod chrome;
mod cmds;
pub mod contract;
pub mod doc;
pub mod gpu;
pub mod keys;
pub mod palette;
pub mod project;
mod queue;
pub mod scroll;
pub mod settings;
mod shell;
mod shell_actions;
mod shots;
pub mod stalls;
pub mod tabs;
pub mod tasks;
mod unsaved;
mod window;

pub use actions::{Action, ActionCx, Actions, Enabled, ReplyCx, Scope, args, menu_row, toolbar_button};
pub use builder::{
    DialogDesc, DrawerDesc, LogState, MenuContent, MenuEntry, PageCard, PanelDesc, PrefGroup, PrefKind,
    PrefRow, ShellBuilder,
};
pub use cmds::register;
pub use doc::{DocCx, Document, Drawer, EditAction, OpenRequest, PanelInfo, PanelOp, ShellRequests, tab_id};
pub use keys::Chord;
pub use queue::{UiQueue, no_window, on_ui};
pub use scroll::{ScrollTo, report_scroll, take_scroll};
pub use settings::EditorSettings;
pub use shell::{
    GrizzlyShell, PrefGroups, ShellOptions, ShellState, default_log_state, floating_panels, project_relative,
    window_options,
};
pub use shots::{PictureGate, encode_png};
pub use tasks::{TaskCard, TaskCardTable};
pub use unsaved::save_tabs;
pub use window::{
    FrameReport, GpuOwners, Shell, StartProfile, UserEvent, WindowLink, WindowOptions, gpu_gate, run,
};
