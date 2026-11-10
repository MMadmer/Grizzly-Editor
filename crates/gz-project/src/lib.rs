//! The project area: projects, their files and what may be done to them.
//!
//! The C++ specification is `EditorProject`, `EditorFileOps`, `EditorAssetIcons` (the pure
//! parts) and `EditorGameContent` (the linked game's content as the DARF browser lists it; the
//! index itself is `gz-vfs`'s).
//!
//! The domain (pure, or IO where that is the stated job):
//!
//! - [`rules`] - what a path IS: [`is_tooling_entry`], [`is_editor_only_entry`],
//!   [`is_source_only_entry`], [`ensure_gitignore`] and its [`GITIGNORE_TEXT`];
//! - [`asset_kind`] - the editor's own asset kinds: [`AssetKind`], [`own_extension`],
//!   [`kind_of`];
//! - [`list`] - `_projects.ini`: [`ProjectList`], [`EntryState`], [`Sort`];
//! - [`project`] - `project.ltx`: [`ProjectFile`], [`scene_in_project`];
//! - [`delete`] - [`check_delete`] / [`delete_project`], the one set of checks in front of a
//!   project's deletion, with [`DeleteInfo`] for the confirmation;
//! - [`ops`] - copy, move, rename, delete, mkdir inside the project: [`FileOps`], [`Report`];
//! - [`paths`] - path text as the C++ compared it;
//! - [`tree`] - the file tree of the open project: [`FileTree`] snapshots kept current by a
//!   [`TreeWatch`].
//!
//! The service and the commands:
//!
//! - [`Projects`] (editor scope: the list) and [`Project`] (project scope: `project.ltx`, the
//!   tree, the last scene, the linked game's index: [`Project::game_snapshot`],
//!   [`Project::refresh_game`], [`Project::link_game`]), [`project_state`] for the `state`
//!   command;
//! - [`ProjectHooks`] - what other areas run when a project opens, closes, is scaffolded or a
//!   file is renamed; [`AssetTemplates`] and [`AssetLists`] - what other areas provide for
//!   `content_create` and `list_assets`;
//! - [`register`] - `list_projects`, `open_project`, `project_close`, `project_create`,
//!   `project_delete`, `project_forget`, `project_reveal`, `content_mkdir`,
//!   `content_create`, `content_copy`, `content_move`, `content_rename`, `content_delete`,
//!   `list_assets`, `darf_list`, `darf_copy`.

pub mod asset_kind;
mod cmd;
mod fsys;
pub mod delete;
mod hooks;
mod ini;
pub mod list;
pub mod ops;
pub mod paths;
pub mod project;
pub mod rules;
mod service;
pub mod tree;

pub use asset_kind::{AssetKind, kind_of, own_extension};
pub use cmd::register;
pub use gz_vfs::looks_like_game;
pub use delete::{DeleteContext, DeleteFailed, DeleteFailure, DeleteInfo, Guard, check_delete, delete_project, system_guards};
pub use hooks::{AssetLists, AssetTemplates, CATEGORIES, ProjectHooks, TemplateCx, TemplateKind};
pub use list::{EntryState, MAX_RECENT, ProjectList, Sort};
pub use ops::{Failure, FileOps, Report};
pub use project::{ProjectFile, scene_in_project};
pub use rules::{GITIGNORE_TEXT, ensure_gitignore, is_editor_only_entry, is_source_only_entry, is_tooling_entry};
pub use service::{NOT_LINKED, Project, Projects, project_state};
pub use tree::{Dir, Entry, EntryKind, FileTree, TreeWatch};
