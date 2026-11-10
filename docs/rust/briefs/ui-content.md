# Brief: the Content Browser (`gz-ui-content`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/UI_DESIGN.md`
sections 1, 1a, 2, 4 and, in section 6, "Content Browser" - **the contract** -; `CLAUDE.md`
sections "Icons for the editor's own assets", "What a build ships" (tooling entries, Exclude
from Build), "Dangerous operations", "Code and text" (file names); the rustdoc at the top of
`gz-ui`, `gz-app` (`ShellBuilder`: drawers, panels, `reveal_in_browser`, `on_ui`),
`gz-project` (the file tree, the file operations, `AssetKind`), `gz-game` / `gz-vfs` (the Game
source), `gz-mod` (exclusion), `gz-fmt-tex` (`texture_tile`), `gz-render` (`ThumbnailCache`,
`render_offscreen`), `gz-editor`.

You own: `crates/gz-ui-content/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line in `crates/grizzly/src/main.rs`, and the
dependency in its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/LevelEditor/UI/ContentBrowser/UIContentBrowser.h/.cpp` (5.1k lines): the
  browser - sources, tree, breadcrumb, tiles and list, selection, the inline rename, drag and
  drop, context menus, favorites, Add, captions (`DrawTileCaption`), the watcher.
- `Source/Editors/XrECore/Editor/EThumbnailTexture.cpp`, `EThumbnailVisual.cpp`: what a tile's
  picture is for a texture and for a model.
- `Source/Editors/XrECore/Editor/EditorAssetIcons.cpp`: which own asset draws which icon
  (`gz-project` has the kinds; map kind -> `gz_ui::Icon` here; `Name()` strings are what
  `content_browser_selection` answers in `icons`).
- Handlers (frozen wire) in `Source/Editors/LevelEditor/Editor/Mcp/LevelMcpContent.cpp`:
  `content_browser_open`, `content_browser_navigation`, `content_browser_selection`,
  `content_browser_copy`. Schemas: `crates/gz-mcp/tools.json`. The data commands
  (`content_copy`, `content_move`, ...) already exist in `gz-project`: the browser's actions go
  through them.
- What QA asserts: `tools/qa/test_asset_icons.py`, `tools/qa/test_unicode_names.py`.

## Architecture (decided)

- **One browser object, two hosts.** The same state draws as the Level tab's docked panel
  (`ShellBuilder::panel`) and as the Content Drawer over any tab (`drawer(Drawer::Content, ..)`),
  one host per frame; each host keeps its own tile size and view (tiles or list). View state
  lives on the UI thread in a struct the shell owns for you; MCP reaches it through `on_ui`.
- **Listing never touches the disk in a frame.** The Project source reads `gz-project`'s
  `FileTree` snapshot (it already skips tooling entries and follows the watcher); the Game
  source reads `gz-vfs`'s index; the Editor source the editor's own data root through a tree
  of the same kind (ask `gz-project` for one over `editor.paths.root`, or build it with its
  walker). A folder's entries, sorted and filtered, are a `Query` keyed by (source revision,
  folder, search): typing in the search does not re-list per frame.
- **Pictures arrive later.** A texture's tile picture (`gz_fmt_tex::texture_tile`, made for the
  size it is drawn at; a picture with transparency on a checkerboard) and a model's
  (`gz_render::ThumbnailCache`) are requested by key and drawn when ready; until then the
  vector glyph. Decoding runs on the pools, bounded and cancellable when the folder changes;
  textures are released on a later frame than the one that stops drawing them. Own assets,
  folders and other files draw vector icons, never above 48 pt, under the selection wash.
- **Captions** (port `DrawTileCaption` as a pure function of the name and measured widths): two
  lines, breaking at spaces, `_`, `-`, `.` and CamelCase words, mid-word only when a word is
  wider than the tile, never right before the extension; a last line that does not fit keeps
  the extension and loses the middle - reported to the audit as truncated. Own assets show
  their NAME without the extension; the rename box edits the name alone and can never change
  the extension (`gz-project` enforces it; the box selects accordingly).
- **Everything a person can do is a command.** Rename, move (drag and drop), copy, delete
  (through the kit's `confirm`, naming what disappears; deletion is final), new folder, Add
  (Quest, Behaviour Graph, Character, PDA Interface - `content_create`), Exclude from Build /
  Include in Build (`mod_exclude`; an excluded tile is veiled and says why on hover), open
  (a double click asks the shell to open the document of that kind; a scene opens through the
  scene's command), reveal. The browser emits `editor.post(..)`; it changes no file itself.
- Drag sources: a tile drags its path (the Screens tab and the viewport will accept it): use
  egui's drag-and-drop payload with a small public type of this crate.

## Rules

- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-content -p grizzly`, zero warnings. Then build,
  run once and LOOK (start `Intermediate\cargo\debug\Grizzly.exe -nodlg -mcp_port 28917` with
  `Start-Process -WindowStyle Minimized`; open a COPY of a real project made in a temp folder,
  never the original; `ui_drawer action=open id=content`; `screenshot_editor`): tiles at 48
  and 192, the list view, a folder with textures, the rename box, the context menu through
  `ui_popup`. Back up `_projects.ini` and the editor's settings file before and restore them
  byte for byte after; delete the temp copy; close with `window_control action=close`. If
  `grizzly` does not build because of someone else's crate, wait and retry a few times, then
  say so. Do not run the QA suites.

## Report

Under 300 words: what is built, `local_*` widgets, what the pictures showed, deviations from
the C++ wire and from UI_DESIGN. Then add a row for `gz-ui-content` to `docs/rust/STATE.md`.
