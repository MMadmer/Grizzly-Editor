# Brief: the project area (`gz-project`)

The first area crate: projects, their files and what may be done to them. Read
`docs/rust/CONVENTIONS.md` (the section "An area crate" is binding) and `docs/rust/STATE.md`
first; then `crates/gz-editor/src/*.rs` and `crates/gz-ltx/src/doc.rs`. The behaviour is
described in the repository's `CLAUDE.md`, sections "The Start page (project browser)", "What a
build ships" (tooling entries, `.gitignore`), "Dangerous operations" and "Icons for the editor's
own assets" (rename rules) - read those four.

You own: `crates/gz-project/**` (new; write `src/lib.rs` before `Cargo.toml`).

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorProject.h/.cpp` (3.2k lines): the list, project files,
  open / close / create / delete, the checks, `.gitignore`, previews (skip: see below).
- `Source/Editors/XrECore/Editor/EditorFileOps.h/.cpp` (1k lines): file operations and their rules.
- The MCP handlers (the wire is frozen by them):
  - `Source/Editors/XrECore/Editor/GrizzlyMCP.cpp`: `list_projects`, `open_project` (around
    lines 950-1000), and what `state` answers about the project;
  - `Source/Editors/LevelEditor/Editor/Mcp/LevelMcp.cpp`: `project_close`, `project_create`,
    `project_delete`, `project_forget`, `project_reveal`, `game_link`;
  - `Source/Editors/LevelEditor/Editor/Mcp/LevelMcpContent.cpp`: `content_mkdir`,
    `content_create`, `content_copy`, `content_move`, `content_rename`, `content_delete`,
    `list_assets`, `darf_list`, `darf_copy` (find each handler by its name string).
  Use `rg -n '"<command>"'` to land on a handler; read handlers, not whole files.
- Tool descriptions and argument schemas of these commands: `crates/gz-mcp/tools.json`.

## Design

Layout (domain pure, service, commands - as CONVENTIONS says):

- `list.rs` - the project list, `_projects.ini` in `editor.paths.root`, through
  `gz_ltx::LtxDoc` so that an untouched list is written back byte for byte (QA restores and
  compares this file). Entries, last-opened order, `[browser] sort`, and the state of each entry:
  ok, folder missing, folder holds no `project.ltx` / `mod.ltx` (`list_projects` names every
  entry with its `state`). A missing folder stays on the list (a drive that is not plugged in).
- `project.rs` - `project.ltx` (and what the editor reads of it: `[project] last_scene`,
  the linked game root, `[xms] export_target`, ...) through `LtxDoc`. `last_scene` is
  project-relative, written at close only, never added to a project that did not have it, and
  an absolute path counts only inside the project. A folder with `mod.ltx` and no `project.ltx`
  is a project (a fresh clone): opening it writes `project.ltx`.
- `rules.rs` - the one place for what a path IS:
  `is_tooling_entry` (every dot-name, `Thumbs.db`, `ehthumbs*.db`, `desktop.ini`, Office `~$`
  locks - at any depth), `is_editor_only_entry`, `is_source_only_entry`, and `ensure_gitignore`
  (a git working copy without `.gitignore` gets one, once; an author's own is never touched).
  Port the lists and the generated text exactly.
- `asset_kind.rs` - the editor's own asset kinds: `.nqasset` quest, `.behasset` behaviour,
  `.npcasset` character (a template when its first kilobyte says `abstract = true` - a pure
  function over the bytes), `.pdaasset` PDA interface. `own_extension(name)`. The UI maps a kind
  to an icon; this crate knows no UI. Spec: `Source/Editors/XrECore/Editor/EditorAssetIcons.cpp`
  (`KindOf`, `OwnExtension`, `Name`) - the pure parts only.
- `delete.rs` - `check_delete`, the ONE set of checks in front of the confirmation, the deletion
  and MCP alike, each refusal with the C++ message: only a listed folder that holds
  `project.ltx` or `mod.ltx`; never the open project; never a link (name where it points);
  never a drive root; never a folder that is or holds the profile, Documents, Desktop,
  Downloads, AppData, Temp, Windows or Program Files; never one that is, holds or lies inside
  the editor or a linked game; never one that holds another project. Paths compare as written
  AND as resolved (`std::fs::canonicalize`). A folder that cannot be listed refuses the whole
  deletion. The deletion itself: final (no Recycle Bin), links removed as links and never
  followed (`symlink_metadata().file_type().is_symlink()` is true for junctions too; a OneDrive
  placeholder is not a link), the read-only mark cleared first, **the manifests last** so a
  deletion that stops at a file in use leaves a project that can be deleted again; the answer
  names every entry that stayed. The confirmation's numbers (files, folders, size, `.git`,
  `releases\`) come from one walk - return them as data for the dialog.
- `ops.rs` - `EditorFileOps`: copy, move, rename, delete, mkdir, create with their rules: the
  project root is refused; an own asset is renamed by its NAME and keeps its extension (dots
  and all: `route.v2` is a name) and another extension of the family is refused; name
  collisions; what a delete counts. Deletion is final.
- `tree.rs` - **the file tree of the open project**, the index every later area reads instead of
  touching the disk: an immutable `Arc<FileTree>` (folders, entries with name, size, write time,
  `AssetKind`) built by a parallel walk (`jwalk` or rayon over directories on the IO pool) that
  skips tooling entries and never follows links, plus a watcher (`notify`) that only marks
  folders dirty and bumps a `Rev`; the changed folders are re-listed on the IO pool and a new
  snapshot is published (unchanged folders are shared by `Arc`). `.git` changing must not cause
  a refresh. API: `snapshot()`, `rev()`, `refresh()` (await a re-list), lookups by `RelPath`,
  iteration of a subtree, "all files with extension".
- `service.rs` - `Projects` (app scope): the list in a `Store` with `AutoSave`. `Project`
  (project scope, in `session.services`): `project.ltx` in a `Store` with `AutoSave`, the
  `FileTree`, the watcher. Register a `DirtySource` only if this area has unsaved state (it
  should not: both files save as edited).
- `cmd.rs` - the commands above. Opening: validate the folder, write `project.ltx` for a fresh
  clone, `ensure_gitignore`, refuse unsaved work of the current project (`editor.refuse_unsaved`)
  unless `discard`, drop the old session, `editor.set_project(Some(session))`, move the entry to
  the front of the list, `editor.log.reset_counts()`. `open_project` on the open project answers
  `{"note":"already open"}`. Closing: the same refusal, write `last_scene` if the session knows
  one (keep a `last_scene: Option<RelPath>` slot in the `Project` service that the scene area
  will set later), `set_project(None)`.
- `hooks.rs` - other areas need to act when a project opens or closes (restore tabs, open the
  last scene). Offer `ProjectHooks` in `editor.services`: `on_open(fn(&Arc<Editor>,
  &Arc<ProjectSession>))`, `on_close(..)`, called by this crate's open and close.

Out of scope here (later packages will add them on top): `preview.png` capture and anything
that draws; Recovery; Link Game's UI; `project_browser_menu` (it is a look at the Start page);
`content_browser_*` (the browser's UI state). `project_reveal` only launches Explorer
(`explorer.exe /select,<path>`; `dry_run` launches nothing and answers what it would open).

## Performance and safety notes

- A project that is a git working copy has thousands of entries under `.git`: the walk must not
  enter it at all, and the watcher must ignore it.
- Every path that arrives over MCP is hostile: resolve it against the project root with
  `RelPath` / `canonicalize` and refuse what leaves it.
- Nothing here blocks the runtime: walks, reads and deletions run through `editor.exec.io`.

## Done means

- `bash tools/rust/cg.sh check -p gz-project --tests` clean, zero warnings; unit tests written
  (not run) for the rules, the delete checks on temp folders, rename rules, list round trips.
- `pub fn register(reg: &mut gz_editor::Registry)` exists and registers every command named
  above; the rustdoc at the top of `lib.rs` lists the public API.
- Add `gz-project` to `crates/grizzly/Cargo.toml` and call `gz_project::register(&mut registry)`
  in `crates/grizzly/src/main.rs` (the one edit you make outside your crate).

## Report

Under 300 words: commands registered, public API, every place where you could not reproduce the
C++ wire or behaviour exactly, and what is left. Then add a row for `gz-project` to
`docs/rust/STATE.md`.
