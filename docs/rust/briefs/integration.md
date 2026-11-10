# Brief: integration

Every package is written. This pass closes what they left between them, so the editor is whole
before it is tested. You are alone in the tree: no other agent is working. Read
`docs/rust/CONVENTIONS.md`, then **all of `docs/rust/STATE.md`** - its "Findings" sections are
the packages' own lists of what they left, and this brief is built from them; where STATE.md
names a leftover this brief does not, close it too if it is small, and list it in your report
if it is not.

You may edit any crate under `crates/`, `tools/rust/` and `docs/rust/STATE.md`. `Source/` (the
C++ specification) and the game under `D:\Games\Dead Air` are never edited. The architecture
rules of `docs/RUST_MIGRATION.md` sections 5-6 hold: no `unsafe` outside `gz-win`, no global
state, the UI thread never waits, state changes through stores.

## 1. `gz-win`: the Win32 the packages asked for

`gz-win` is the one crate with `unsafe` (read it first: every block has a SAFETY line and the
API is safe - keep that standard exactly). Add:

- **The caption hit test.** The window has no system frame, and Windows' own hit test always
  answers "client": the system's drag, double-click maximise, snap layouts and the QA check of
  `HTCAPTION` do not work. Subclass the window procedure (`SetWindowSubclass` or
  `SetWindowLongPtrW` with a chained procedure) and answer `WM_NCHITTEST` from a small shared
  description the shell updates every frame: the caption band, the interactive rectangles
  inside it (menus, tabs, buttons - `HTCLIENT`), the maximise button (`HTMAXBUTTON`), the
  resize borders. The description lives in an `Arc` the procedure reads with an atomic load
  (`arc-swap`), never a lock the UI thread could hold. `gz-app` then drops its own
  `StartDrag` / `BeginResize` emulation. The C++ behaviour to match:
  `Source/Editors/LevelEditor/UI/UITitleBar.cpp` and `titlebar_info` in `GrizzlyMCP.cpp`.
- **Free space** on a volume (`GetDiskFreeSpaceExW`): the check before a build and before a
  release is written (`gz-build`, `gz-release` left it out; port the C++ check and message).
- **Known folders** (`SHGetKnownFolderPath`: Documents, Desktop, Downloads, the profile,
  AppData): replace the environment-variable guesses of `gz-project`'s delete guards and the
  `reg export` call of `gz-release`'s catalog home. And whether a folder is a volume mount
  point, for the "never a drive root" guard.

## 2. Slots and hooks nobody filled

Read each owner crate's rustdoc for the exact names.

- `gz_world::SceneStarts` and `gz-nq`'s `SceneLookup` from the open scene (unless
  `gz-levelbuild` already fills them - check).
- `GameLink::refresh()` after anything that writes into the game (Build Into Game): the modes
  list and the installed modules must follow.
- The Task Center shows `gz_release::Releases::status` during packaging (the build's title
  shows now).
- `gz_ui_viewport::install` is called by nobody: call it, so the viewport's commands reach the
  Keyboard page.
- `content_create` over MCP without `folder=` lands in the project root: it must land in the
  folder the Content Browser shows (`gz-project` needs a "default folder" hook the browser
  fills; `tools/qa/test_asset_icons.py` phase A relies on it).
- **Saving on request**: `project_close save=true`, `window_control close save=true` and
  `ui_tab close save=true` must save every dirty document. Add a `SaveAll`-style registry to
  `gz-editor` beside `DirtySource` (each area that reports dirty documents also offers to save
  them), implement it for quests, the scene, the PDA source, the libraries, characters, and use
  it in those three commands. Also the "rescue save of unsaved graphs when a project closes"
  the quest package left out.
- `game_link` refuses while a scene is open, and the Start page's Close Project is off then,
  as the C++ does (the scene area exists now).
- The preferences `recovery_*` and `object_flags` are missing from `gz-app`'s table, so their
  rows in the Preferences tab are read-only: add them and wire recovery to `gz-scene`.
- The Content Browser registers as a key owner with the shell, so the scene's bindings do not
  fire while it has focus.
- `preview.png` is taken after saves and a minute after changes because the close hook runs
  too late: add a pre-close hook to `gz-project` (called BEFORE the session is dropped) and
  take the picture there, as the C++ does.
- The two commands still missing from the registry: `exec_command` and `list_commands` (the
  editor's command table: list the shell's `UiCommand`s with their C++ ids and descriptions,
  execute one by id). Spec: the handlers in `Source/Editors/XrECore/Editor/GrizzlyMCP.cpp` /
  `LevelMcp.cpp`, the table in `UI_MainCommand.cpp`; schemas in `crates/gz-mcp/tools.json`.
  Then `Intermediate\cargo\debug\Grizzly.exe -dump_commands` must list every `cmd` of
  `tools.json`.

## 3. The kit's leftovers

- `ui_audit` returns no controls for kit dialogs (`dlg.publish_release`, `dlg.catalog_submit`,
  ...): find why (the surface name, or widgets inside a dialog not reporting) and fix it - QA
  reads a dialog's buttons through the audit.
- A dialog fixes its height on its first two frames, so content that arrives later (a note
  from a finished read) pushes a folded block below the fold. Let a dialog measure again when
  its content's height changes while nobody is interacting with it (never while a job runs:
  "a running job never resizes it" stays).
- `menu_item` takes a tooltip, so a disabled row says why.
- A standalone `segmented` becomes a combo when it does not fit.
- Lists get keyboard navigation (arrows, Home / End, type-ahead) where the C++ lists had it.

## 4. Behaviour to settle against the C++

- **Classic modules.** `CLAUDE.md` ("Classic mods"): a classic module's `mod.ltx` says
  `mode = *` and `target = any`, and it mounts in every game. The Mod Settings tab's first save
  rewrites `target = any` to `existing`. Read what the C++ really does with `any`
  (`EditorModManifest.cpp`: the manifest model and `mod_set_manifest`) and make the Rust side
  keep a classic module classic: opening and saving its settings must not change `target`.
- `LtxDoc` treats `//` as a comment, which is why `gz-mod` grew its own `ManifestDoc`. Give
  `gz_ltx::LtxDoc` a comment-rule option (engine rules / manifest rules) if that lets `gz-mod`,
  `gz-project` and `gz-github` drop their private ini code without changing a byte they write;
  if it does not come out clean, leave them and say so.

## 5. Sound encoding

The Sound Library cannot make a game sound from a WAV: there is no Vorbis encoder. Add WAV ->
OGG Vorbis encoding to `gz-fmt-sound` with the `vorbis_rs` crate (a safe API over vendored
libvorbis - the second C dependency after LuaJIT; note it in `docs/RUST_MIGRATION.md` section 6
and 8), at the quality the C++ uses (`SoundManager.cpp`: `MakeGameSound`), write the game
parameters into the comment as today, and bring back the library's Rebuild.

## 6. The MCP bridge in Rust

`tools/mcp/grizzly_mcp.py` becomes a small binary, crate `crates/gz-mcp-bridge` (bin
`grizzly-mcp`): an MCP server over stdio (JSON-RPC lines: `initialize`,
`notifications/initialized`, `tools/list`, `tools/call`) that forwards each tool call to the
editor's TCP endpoint. Dependencies: `serde_json` only (blocking std IO; it must start in
milliseconds).
- The tools, their descriptions, schemas, annotations and the server instructions come from
  `crates/gz-mcp/tools.json` (`include_str!`), word for word - the `PLAYER_SAFETY` and
  `WITHDRAW_SAFETY` paragraphs are in those descriptions and must reach the client unchanged.
- Port `tool_result` of the Python bridge exactly: unknown arguments are refused with its
  message; the tools that are a command with a fixed argument (`grizzly_mod_release_delete`,
  `_edit`, `grizzly_mod_catalog_submit`, `_withdraw`, the three `*_discard` tools - the ONLY
  way `discard` is ever sent); `png_base64` becomes an image content with the Python's text;
  `isError` from `ok`. The waiting loops of the Python bridge stay as a fallback for an answer
  that says `pending`, though the Rust editor answers when the work has ended.
- The port: `GRIZZLY_MCP_PORT`, default 28017 until the cutover.

## 6a. What the level build left

- **A bug**: `gz-fmt-xr` defines the game material's dynamic flag as `1 << 3`; the engine's is
  `1 << 8` (check `Source/XrEngine/GameMtlLib.h`, fix the crate, and let `gz-levelbuild` and
  `gz-fmt-ogf` - which skipped its "dynamic" check - use it).
- `gz-entities` has no setters for a zone's artefact offset and a level changer's destination,
  so the spawn build leaves both as the scene holds them: add the setters and use them where
  the C++ spawn constructors set them.
- Not ported in `gz-levelbuild`: the breakable and climable objects `BuildGame` adds to
  `level.spawn`, `Scene->Validate` before a build, the space-restrictor separator check for
  Play. Port them.

## 7. Lints

- `cargo clippy --workspace --all-targets` clean (the workspace denies `unwrap_used`,
  `expect_used`, `dbg_macro`, `todo`; a known error sits in `gz-fmt-tex`). Fix causes, do not
  silence: an `#[allow]` needs a comment that says why it is right.
- `cargo fmt --all`.
- `grep -rn "unsafe" crates --include=*.rs` shows `unsafe` in `gz-win` only.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Tests are written, not run - the test pass comes after you. Compile with
  `bash tools/rust/cg.sh check --workspace --all-targets` and `bash tools/rust/cg.sh build -p
  grizzly`.
- After the caption work and the dialog work, LOOK once through `tools\rust\run-editor.ps1`
  (the only way to run the editor): the Start page, a dialog, the Level tab on a temp COPY of
  a project; `titlebar_info` answers the band; nothing real is touched.
- Update `docs/rust/STATE.md` as you close items: move each from its "Findings" list to a
  short "Closed in integration" list, so the critic sees what is open.

## Report

Under 500 words: every item above as done / partly / not done with the reason, anything else
you closed from STATE.md, and what is still open. Be exact: the critic reads this.
