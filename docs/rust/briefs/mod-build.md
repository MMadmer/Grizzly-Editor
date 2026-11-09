# Brief: the manifest and the build (`gz-mod`, `gz-build`)

`mod.ltx` and what turns a project into a module of the game. Read `docs/rust/CONVENTIONS.md`
("An area crate") and `docs/rust/STATE.md` first; then the rustdoc at the top of
`crates/gz-project/src/lib.rs`, `crates/gz-ltx/src/doc.rs`, `crates/gz-editor/src/lib.rs`; and
`CLAUDE.md` sections "Compatibility", "Mod philosophy", "Game modes", "Where a mod exports to",
"What a build ships", "Code and text" (file names the game cannot open).

You own: `crates/gz-mod/**`, `crates/gz-build/**` (new; write `src/lib.rs` before `Cargo.toml`),
and the registration lines in `crates/grizzly`.

## The C++ that is the spec

`Source/Editors/XrECore/Editor/EditorModManifest.cpp` (6.1k lines) + `.h`. It holds several
things; read it by region (grep the function names):

- the manifest model: `SManifest`, `LoadManifest`, `SaveManifest`, `ValidateVersion`,
  `ValidateWebsite`, `ValidateGithubRepo`, the quoting of `name` / `author` / `description`,
  `ManifestGeneration`, `ProjectGlobals` (`[vars]`), `[commands]`, `[exclude]`
  (`ExcludeMatch`, `OutsideBuild`);
- the build: `SBuildWalk`, `Export`, `ExportFlat`, `ValidateForBuild`, `StartJob` /
  `JobThread` / `WarmThread`, the clean build into `modules\.new\<id>`, the hard links, the
  refusals, `left_out`;
- `PackageRelease` and everything after it: NOT yours (the release package).

The game's side of the contract, to check a key's meaning (READ-ONLY):
`D:\Games\Dead Air\DeadAir-x64\docs\dead-air\MODDING.md` and `XMS_ARCHITECTURE.md`.

Handlers (frozen wire): `mod_manifest`, `mod_set_manifest`, `mod_exclude`, `mod_export`,
`mod_commands` in `Source/Editors/LevelEditor/Editor/Mcp/LevelMcpMod.cpp` (and where they
delegate). Tool schemas: `crates/gz-mcp/tools.json`. QA that pins behaviour:
`tools/qa/test_build_contents.py`, `tools/qa/test_build_job.py` (read them for what is asserted).

## Design

**`gz-mod` - the manifest.**

- The manifest is ONE document that every editor of it writes through: Mod Settings, World
  Rules, Screens & Music, the graph tab's globals, Exclude from Build. In C++ each re-read the
  file first and tabs polled a generation counter. Here it is a `Store<ManifestState>` in
  `session.services` whose state holds the `gz_ltx::LtxDoc` (lossless: an edit touches its
  lines only; an author's comments and order survive) and the parsed `Manifest`; `AutoSave`
  writes it; the store's `Rev` IS the manifest generation.
- `Manifest`: typed, read-only view of every section the C++ model knows, with unknown keys
  and sections preserved in the doc. Sections owned by later areas (`[world]`, `[start_kit]`,
  `[screens]`, `[screen.*]`, `[track.*]`, `[pda]`) get raw accessors here
  (`section_entries(name)`, `set_section(..)`) so those areas can read and write them through
  the same store without this crate knowing their meaning.
- Commands of the store are edits of the document (`Set { section, key, value }`,
  `Remove`, `ReplaceSection`, `SetExclude`, ...); validation is pure (`validate(&Manifest) ->
  Vec<Problem>`), with the C++ messages.
- An external change of `mod.ltx` (git checkout, an agent editing the file): the project's file
  tree revision moves; the service compares the file's bytes with what it last wrote and, when
  they differ, loads the file as a new document (`cx.new_document()`).
- `exclude_match`, `outside_build(rel)` - the one question every walk that feeds a build asks.
- `globals()` - `[vars]` as declared (defaults kept as written), a `Query` on the store's rev.

**`gz-build` - the build.**

- `BuildWalk`: ONE list of what will be written, made before anything is written, from the
  project's `FileTree` snapshot (no second walk of the disk): what ships, and `left_out` with
  the reason for each (tooling, editor-only, source-only, `[exclude]`, a link).
- Checks: this crate does not know quests, characters, PDA or screens. Each area registers a
  `BuildCheck` (a trait object: `fn check(&self, editor, session, walk) -> BoxFuture<Vec<Problem>>`)
  in `editor.services` (`BuildChecks` registry, defined here); the build runs all of them in
  parallel and an `Error` problem stops it. The manifest's own `ValidateForBuild` is the first.
- The export, as a job (`editor.jobs.start("build", Some("build"), ..)`, one at a time):
  written beside the last build in `<target>\modules\.new\<id>`, taking its place in two renames
  at the very end; a file the last build holds unchanged (size and write time) is a hard link
  (`std::fs::hard_link`), anything else a copy; copies and links run in parallel on the IO pool
  with `ctx.progress`; `ctx.check()?` between files so Stop works and costs the module nothing.
  Refusals BEFORE a folder is created: the target is a link, or is / holds / sits in the
  project (compare `canonicalize`d paths); a module the game holds open is refused and stays
  whole. `wipe_tree` removes a link without following it.
- Flat export (`gamedata_<id>`), Build to Folder, the clean-target question as data (up to ten
  of the files it would delete) - the dialog is drawn elsewhere; over MCP the answer is JSON.
- Warnings the C++ gives: file names the game cannot open (`gz_base::text::fits_cp1251`).
- `mod_export` answers when the job has ended (await `editor.jobs.wait(id)`); every other
  command goes on meanwhile.

## Plugging into `gz-project`

`gz-project` (read the rustdoc at the top of its `lib.rs`) offers `ProjectHooks`, `AssetTemplates`
and `AssetLists`; an area registers there what only it knows:
- the **scaffold** hook: a new project gets its `mod.ltx` (seed `[module]` as the C++
  `project_create` does: the id and the name from the folder, the mode target) - until this
  crate registers it, a created project has `project.ltx` alone.

## Why

- "The editor describes, the game does": a build generates nothing - it ships the author's
  files. If you find the C++ generating a file for the game, check `CLAUDE.md`: it is probably
  dead code from before that rule.
- The build must agree with the browser and the asset indexes on what the module IS; all of
  them ask `gz-project`'s rules and `outside_build`, nothing has a list of its own.

## Done means

- `bash tools/rust/cg.sh check -p gz-mod -p gz-build --tests` clean, zero warnings. Tests
  written (not run): manifest round trips that keep comments and order, validators, exclude
  patterns, the walk on a temp tree, the `.new` swap and hard links on temp folders, refusals.
- APIs at the top of each `lib.rs`; commands registered.

## Report

Under 300 words: API, commands, deviations from the C++ wire, what is left. Then add rows for
both crates to `docs/rust/STATE.md`.
