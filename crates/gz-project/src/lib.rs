//! The project area: projects, their files and what may be done to them (`docs/assets/PROJECT.md`).
//!
//! A project is a folder with its project file (`<Name>.grizzly`) or a `mod.ltx`; nothing in it
//! names a place outside it. What is this machine's - the game, the export target, the tabs -
//! lives in the editor's home by the project's id. The game is the editor's default one unless
//! the project names its own, and the default is found by itself.
//!
//! The domain (pure, or IO where that is the stated job):
//!
//! - [`rules`] - what a path IS: [`is_tooling_entry`], [`is_editor_only_entry`],
//!   [`is_source_only_entry`], [`ensure_gitignore`] and its [`GITIGNORE_TEXT`];
//! - [`asset_kind`] - the editor's own asset kinds: [`AssetKind`], [`own_extension`],
//!   [`kind_of`];
//! - [`list`] - `_projects.ini`: [`ProjectList`], [`EntryState`], [`Sort`];
//! - [`project`] - the project file: [`ProjectFile`], [`scene_in_project`]; [`legacy`] - the old
//!   `project.ltx`, read once and deleted;
//! - [`machine`] - this machine's state of a project ([`machine::ProjectState`]) and the
//!   editor's own places ([`machine::MachineFile`]); [`games`] - the default game and its finder
//!   ([`Machine`], [`library_root`]);
//! - [`library`] - what a scene takes from the editor's library: [`library::bring_in`],
//!   [`library::note_scene_refs`], [`library::collect`];
//! - [`guide`] - `AGENTS.md`, written once; [`association`] - `.grizzly` opens in this editor;
//! - [`delete`] - [`check_delete`] / [`delete_project`], the one set of checks in front of a
//!   project's deletion, with [`DeleteInfo`] for the confirmation;
//! - [`ops`] - copy, move, rename, delete, mkdir inside the project: [`FileOps`], [`Report`];
//! - [`paths`] - path text: backslashes, ASCII case ignored;
//! - [`tree`] - the file tree of the open project: [`FileTree`] snapshots kept current by a
//!   [`TreeWatch`]; [`watch`] - the project's one source of "a file changed" ([`ProjectWatch`],
//!   `project.tree.changes()`): debounced, coalesced, the editor's own writes left out.
//!
//! The service and the commands:
//!
//! - [`Projects`] (editor scope: the list) and [`Project`] (project scope: the project file,
//!   this machine's state of it, the tree, the scene to reopen, the game's index:
//!   [`Project::game_snapshot`], [`Project::refresh_game`], [`Project::link_game`]),
//!   [`project_state`] for the `state` command;
//! - [`ProjectHooks`] - what other areas run when a project opens, closes, is scaffolded or a
//!   file is renamed; [`AssetTemplates`] and [`AssetLists`] - what other areas provide for
//!   `file.create` and `file.list source=library`;
//! - [`register`] - `project.list`, `project.open`, `project.close`, `project.create`,
//!   `project.delete`, `project.forget`, `project.reveal`, `project.info`, `project.guide`,
//!   `project.collect`, `project.associate`, `file.list`, `file.copy`, `file.move`, `file.delete`, `file.mkdir`,
//!   `file.rename`, `file.create`, `file.import`.

pub mod asset_kind;
pub mod association;
mod cmd;
pub mod delete;
mod diskdoc;
mod fsys;
pub mod games;
pub mod guide;
mod hooks;
mod ini;
pub mod legacy;
pub mod library;
pub mod list;
pub mod machine;
pub mod ops;
pub mod paths;
pub mod project;
pub mod rules;
mod service;
pub mod tree;
pub mod watch;

pub use asset_kind::{AssetKind, kind_of, own_extension};
pub use cmd::{close_project, register};
pub use delete::{
    DeleteContext, DeleteFailed, DeleteFailure, DeleteInfo, Guard, Inside, check_delete, delete_project,
    projects_inside, system_guards,
};
pub use games::{Candidate, Machine, clean_root, library_root};
pub use gz_vfs::looks_like_game;
pub use hooks::{
    AssetLists, AssetTemplates, CATEGORIES, ProjectHooks, TemplateCx, TemplateKind, editor_file_list,
};
pub use list::{EntryState, MAX_RECENT, ProjectList, Sort};
pub use machine::Found;
pub use ops::{Failure, FileOps, Report};
pub use project::{ProjectFile, scene_in_project};
pub use rules::{
    GITIGNORE_TEXT, ensure_gitignore, is_editor_only_entry, is_source_only_entry, is_tooling_entry,
};
pub use service::{GameSource, LinkScope, NOT_LINKED, Project, Projects, project_state};
pub use tree::{Dir, Entry, EntryKind, FileTree, TreeWatch};
pub use watch::{Change, ChangeKind, Filter, ProjectWatch};
