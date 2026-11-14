# Report S - the Start page, the Content Browser, previews and libraries

## 1. Built

**The Content Browser lists assets (`crates/gz-ui-content`)**
- `model.rs`: two sources, `Source::{Project, Library}` (wire `project`, `library`); the Library's kinds `KINDS`
  (Objects, Groups, Textures, Sounds, Particles, Spawn Sections, Base Scenes: caption, wire word, icon, payload kind,
  placed, extension, library folder, game folder), `HOME` for the Library's top; `Location` keeps the kind in
  `category`; `shown_tile` fits by the rows a folder needs (`entries`, `width`), `LIST_BELOW` = 64 pt.
- `catalog.rs`: `FileItem` gains `origin` (Disk / Game / None), `label`, `note`, `companions`. `Catalog::from_project`
  hides `.thm`, backups (`.~x`, `x~`, `.bak`, `.old`), an object's `.skl`/`.skls`/`.bones`, a scene's folder (of a
  `.scene` and of an SDK `.level`), `mod.ltx` and the project file; what is hidden becomes the asset's companions.
  `Catalog::from_library_dir` lists only a kind's own extension. `home_view`, `ViewInput` (exclude rules, the project
  file's `library` list -> `ViewEntry::wanted`). Tests: an object and its thumbnail are one tile, a thumbnails-only
  folder is empty, a scene is one tile, the project file is no tile, a library kind lists its own files only.
- `library.rs` (new): a Library kind's catalog built on the pools - library folders walked; textures and sounds merged
  game-first with what only the library has; particles from `file.list source=library kind=particles`; spawn sections
  from `game.lookup type=spawn_section` grouped by their `$spawn` path, named by `game.lookup type=item_section` (a
  shared name adds its section). A kind that cannot be read keeps a `note`.
- `service.rs`: the catalogs per (Project) and (Library, kind), revisions mixing the kind's Refresh and the game's
  revision for the kinds that read it; `inputs`/`make_view` (the project's library list read inside the view query);
  `Dragging`, `drag_payload`; `HostView::{grid_w, entries, shows_list}`; Up from a kind's root goes to the Library's
  top, Home and a source switch go to the source's top.
- `pictures.rs`: uploads within `UPLOAD_BUDGET` (the kit's `FRAME_BUDGET`, a megapixel) a frame, the rest kept for
  the next frames (audit D4); an object's picture is its `.thm` (rendered when none), a group's its `.thm`
  (`Picture::Thumb`); a render asked before the renderer is up is asked again.
- `lib.rs`: `payload_of` - the kit's `drop::Asset` of an entry; the crate's own `DragPayload` is gone.
- `actions.rs`: `what_opens(cat, path)` (a Library base scene comes into the project and opens; Library objects and
  textures open their previews; a project object opens by its reference); `placeable`; Place in Level through the
  viewport's own drop at its centre (`ui.list` -> the viewport's rect -> `viewport.gesture kind=drop assets=`), else
  `scene.place on_surface=true` at ten metres along `viewport.camera`'s direction; `bring_into_project`
  (`file.import`, a base scene via `import_base_scene`); `import_files` (Explorer drops: `file.copy`, a picture into a
  `gamedata\textures` folder made a `.dds`); delete, rename, copy, move take an asset's companions; `link_game` opens
  Preferences > Game.
- `local.rs` `local_menu_heading` as wide as its words (with the size row, what made the View popup ~600 pt).
- `draw.rs`: the Sources pane's `[Project | Library]` with kit `Section` headers; the project tree's root row named
  after the project, the Library's tree its kinds; breadcrumbs Library > kind > folders; the View popup's size row
  placed by hand (its width was the popup's); tiles under 64 pt drawn as the list; the kit's `drop::start` on a
  drag and `drop::Zone` on project folders (tiles, tree rows, the grid as catch-all); the Library's note as a banner
  (with Link Game... when unlinked); Place in Level and Bring into Project in the entry menu; the wanted dot; tooltips
  for an SDK `.level` and spawn sections.
- `cmd.rs`: the typed `content.view` (section 3). `local.rs`: `local_section` removed (the kit's `Section`).
  `icons.rs`: `tile_icon(path, folder, own, kind_icon)`, `.scene` glyph. `Cargo.toml`: serde, schemars, tempfile.

**The Start page (`crates/gz-ui-app`)**
- `start.rs`: New Project is the grid's first slot - a card with a + that turns into the form in its place (Name,
  one status line, Location under More, Cancel | Create, Create the one Primary); the head row Open Folder... | Open
  Project File... (`folder.rs` `pick_file`, filtered to `*.grizzly`) | the SearchField capped at 24 em | the sort at
  the right; the cards straight on the page (no inset frame); a catch-all `drop::Zone` over the page opens a dropped
  project file or folder (`open_folder` now takes either); the version in the bottom right corner.
- `cards.rs`: a card is named by its project file (`project_name`), the folder's name without one.
- `browser_cmd.rs`: `project_browser_menu` became the typed `project.page`; answers `new_project`.
- `log.rs`: whole rows (the area holds whole rows, a scroll at rest settles on a row); the marks `! ~ *` set the level
  and are stripped from the shown text and the status line; the level's word is searchable (`LineLevel::word`).
- `local.rs`: `local_section` removed (`prefs.rs` uses the kit's `Section`).

**Previews and libraries (`crates/gz-ui-previews`)**
- `cmd.rs`: typed `preview.model`, `preview.texture`, `library.view`, `shot.asset` (section 3); no `pending`.
- `image_pass.rs`: `ImagePass::get` makes the pipeline on a thread of its own and answers `None` until it is there;
  the model and texture tabs ask as they open (audit D5); `ImageDraw::new` answers `Option`.
- `model.rs`: Place in Level and Show in Content Browser on the toolbar; `opened_from`. `model_load.rs`: Title Case
  kinds (Static Mesh, Rigid Skeleton, Library Object), source words `library`/`project`/`game`. `texture.rs`: Show in
  Content Browser; `editor` field. `texture_load.rs`: `project`/`library`/`game`.
- `library/textures.rs`, `sounds.rs`: whether the folder can be written is a `Pending` from the IO pool (audit D6);
  More menus with the manual work; the Texture Library counts the import folder's new pictures as it opens and offers
  Import over the list; one whole-tab empty state with Import New Textures as the Primary; blank details with nothing
  picked. `lanim.rs` likewise. `lib.rs`/`libtools.rs`: the Libraries menu is the three tabs and Reload Objects.
  `local.rs`: `local_section` removed.

**The PDA tab's two leftovers (`crates/gz-ui-pda`)**
- The stall: measured on the author's `shmyandex.pdaasset` (57 KB) - 243 ms and 128 ms on the first two frames. The
  first was the synchronous layout of the whole text (cold galley cache) plus a line number laid out for every row;
  the second the colouring thread holding the fonts lock for its whole layout. Now (`local.rs`) a text over 16 KB is
  laid out 8 KB a frame before the field shows it (`warm_up`, `prefix_job`: each frame lays out the part so far, so
  egui's per-line galley cache keeps every line and the whole is then a cache hit); the thread is gone; only visible
  line numbers are laid out; lines do not wrap. After: no stall (section 5).
- The sheets: `assets.rs` `Sheet` holds a kit `upload::Tiled` (cut on the CPU pool), `PdaAssets::pump` hands the
  tiles over a frame's budget at a time (called from `doc.rs`), `Sheet::paint` draws a piece tile by tile;
  `canvas.rs` draws textures, glyphs, spots and the map through it.
- `drop.rs`: reads the kit's payload instead of `gz_ui_content::DragPayload`; the crate's dependency on
  gz-ui-content is gone.

**Messages at the source**: the `! ` / `~ ` / `* ` marks dropped from 34 messages logged with a level of their own in
gz-build, gz-export-level, gz-levelbuild, gz-nq, gz-release, gz-scene; nine `BuildError` texts of gz-levelbuild
rewritten as sentences ("The AI map is empty.", "The level has no actor spawn point.", ...).

**Call sites of my changes**: `gz-app` `cmds.rs` (`ui.tab` texture sources `project|library|game`),
`gz-ui-world` `characters.rs` (`preview.model`, sources), `gz-ui-pda` `drop.rs` + `Cargo.toml`.

**QA**: `test_previews.py` (new phase C: one tile per asset in the project and the Library, a folder of thumbnails
empty, the Library's kinds, an object's and a spawn section's payloads with `place` / `import`, Bring into Project,
no stall while 587 objects are scrolled through; the new command names; no `pending`; ends on `perf.stalls`),
`test_asset_icons.py` (`content.view`, `ui.reveal`, tiles under 64 are the list, ends on `perf.stalls`),
`test_project_browser.py` (`project.page`, `query=`, New Project a card, Open Project File..., the form opens in its
slot with no card moving), renames in `test_behaviours.py`, `test_build_contents.py`, `test_ui_tour.py` (#24, #25:
no `pending`, #25 opens a library object), `test_unicode_names.py`.

## 2. Decisions

- **The Library's kind is the location's `category`**, its top `HOME`; one catalog per kind, built when the kind is
  first shown, so opening the Library costs nothing and Refresh rebuilds one kind. A search searches the kind shown.
- **Sidecars and scene folders are companions of their asset**, not merely hidden: a delete, rename, copy or move of
  the tile takes them along. Skeleton files count only beside an `.object` of their stem; `.thm` always.
- **The drag payload follows the viewport's reader** (L2a's `place::of_asset`, written meanwhile): objects and groups
  by their file with its extension, sounds and particles by their reference, a spawn by its section (kind `spawn`;
  L2a also reads `spawn_section`). `scene.place`'s arguments are derived from it (`placeable`).
- **Place in Level uses the viewport's own drop** at the middle of the viewport's rect (`ui.list`), so the ghost's
  surface pick and snapping decide; with the Level tab not up it falls back to `scene.place` ten metres along the
  live camera (`viewport.camera`), dropped to the surface. The model preview's Place in Level posts `scene.place
  on_surface=true` (it has no browser to borrow from).
- **`content.view` changes no file**: it answers, per entry, what places it and what brings it in; the window's Bring
  into Project and an agent both run `file.import` / `scene.place`. `ui.reveal` replaces the old reveal action.
- **Explorer drops** use `file.copy`, and the picture conversion is done in gz-ui-content (load + `make_game_texture`
  with default parameters) - `file.import` takes no file from the disk (Requests).
- **Tiles under 64 pt are the list** (48 and 56 stay valid sizes, drawn as the list): the cheapest way to never cut a
  folder's name mid-word; the fit no longer shrinks for a second row the folder does not need.
- **Spawn sections by their in-game names** come from `item_section` (an `inv_name`); NPC sections keep their ids.
- **The Texture Library's "Check New Textures becomes what import does by itself"**: it looks at the import folder as
  it opens and offers Import over the list; the manual rows moved to each tab's More, the Libraries menu keeps the
  three tabs and Reload Objects.
- **The PDA Source pane lays a big text out over frames** instead of in a thread: egui's galley cache splits a job
  into lines and keeps those a frame used, so growing prefixes leave the whole text in the cache; no lock is held long.
- `project_browser_menu` is mine (gz-ui-app) and was untyped: it is now `project.page`, `search=` became `query=`.
- `Source` numbers (0/1/2) and the words `editor`/`darf` are gone (no aliases).

## 3. Contract changes

- Commands added: `content.view`, `project.page`, `preview.model`, `preview.texture`, `library.view`, `shot.asset`.
  Removed: `content_browser_navigation`, `content_browser_selection`, `content_browser_open`,
  `content_browser_copy`, `project_browser_menu`, `preview_model`, `texture_preview`, `library`, `asset_preview`.
  Source words: `ui.tab kind=texture source=project|library|game`; `preview.model source=library|project|game`.
- `docs/UI_DESIGN.md`: Content Browser (rewritten: sources, one tile per asset, payload convention, menu, MCP),
  Output Log, PDA Interface (tiles, the layout over frames), Model Preview and Texture Preview, Start page, Libraries,
  section 7 (held commands, the surfaces' commands), section 8 (local widgets).
- `docs/COMMANDS.md` section 5: `content`, `preview`, `library` areas; `project.page` under `project`.

## 4. Requests

- **K / architect (`gz-app` `contract.rs`)**: `CATEGORIES` (answered by `ui.contract categories`) still lists the
  ten Editor categories; it should be the Library's kinds (`gz_ui_content::model::KINDS` captions). `chrome.rs:2146`
  names `content_browser_selection` in an error text (now `content.view`).
- **A2 / architect (`gz-project`)**: `file.import` should take a file from the disk (`from=disk`), with the picture
  conversion where the target is a texture folder - then the browser's Explorer drop and Screens & Music's import are
  one routine (today the browser converts with `gz_fmt_tex` itself).
- **L2a**: `viewport.gesture kind=drop` could take no `at` (the centre); the browser's Place in Level reads the
  viewport's rect from `ui.list` to compute it. The doc's "Dropping into the viewport" names kind `spawn_section`; the
  browser sends `spawn` (both are read).
- **M / Q (fields that take a payload)**: the Characters tab's Model field and a foliage list should take the kit's
  `Payload::Assets` (`kind` `model` / `object`) - their drop zones are in gz-ui-world / gz-ui-level.
- **Owners of gz-ui-viewport, gz-ui-level, gz-render** (active while I worked, not touched): messages with `! ` / `~ `
  / `* ` at the start remain (`gz-ui-level` `detail_objects.rs`, `gz-ui-viewport` `service.rs`); and the logs
  collected as `Vec<String>` with a mark as their level (gz-export-level, gz-levelbuild `files.rs`, `graph.rs`,
  `spawn.rs`) keep it (the Output Log hides it). The kit's `~ [dlg suppressed]` lines are its contract (UI_DESIGN 4).
- **Q**: `docs/rust/*`, `RUST_MIGRATION.md`, `tools/mcp/AI_SETUP.md` and CLAUDE.md name the removed commands.
- The status line keeps the last report of a closed project on the Start page ("the scene is open: ...") - gz-app.

## 5. Proof

- `bash tools/rust/cg.sh clippy -p gz-ui-content -p gz-ui-app -p gz-ui-previews -p gz-ui-pda --all-targets`: no
  warning. `cg.sh fmt` on the four crates.
- `bash tools/rust/cg.sh test -p gz-ui-app -p gz-ui-content -p gz-ui-previews -p gz-ui-pda`: gz-ui-app 23,
  gz-ui-content 20, gz-ui-previews 19, gz-ui-pda (with `a_prefix_keeps_the_lines_of_the_whole`) green.
- `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge`, then with `GRIZZLY_QA_PORT=28974`, each under
  `tools/rust/limited.py`: `run_all.py test_previews` 78/78 (15 s), `test_asset_icons` 32/32,
  `test_project_browser` 79/79, `test_unicode_names`, `test_behaviours`, `test_build_contents` pass,
  `test_ui_tour` pass (after the #24/#25 fixes), `test_pda` pass.
- The PDA stall, on a copy of DARF-Overnight-Delivery: `pda.preview action=open pane=both` on `shmyandex.pdaasset`
  then `perf.stalls`: before `[{243 ms}, {128 ms}]`, after `[]`; again after a zoom and a texture preview: `[]`.
- Pictures in `Intermediate/plan/S/` (each looked at; 1600 x 900 and 1280 x 720 at scale 1.5):
  - `cb_drawer_library_objects_1600.png`, `_1280.png` - Library > Objects > corps: one tile per object, the `.thm`
    thumbnails as pictures, the Library tree with its kinds.
  - `cb_view_popup_1600.png` - the View popup as wide as its rows (its headings and its size field had made it
    ~600 pt wide).
  - `cb_drawer_library_spawns_1600.png` - Spawn Sections > weapons > ak-74 by in-game names, a shared name with its
    section.
  - `cb_drawer_library_home_1600.png` - the Library's top: its seven kinds.
  - `cb_drawer_project_fit_1600.png` - the project's root fitted; `level_panel_1600.png` - the docked panel.
  - `output_log_drawer_1600.png` - the Output Log: whole first row, the profile lines.
  - `model_preview_1280.png` - corp_dog: Library Object, Place in Level, Show in Content Browser.
  - `texture_preview_1280.png` - a library texture with its toolbar.
  - `texture_library_1600.png` - one empty state, Import New Textures the Primary; `sound_library_1600.png`.
  - `pda_source_1600.png` - the author's layout with the Source pane, the sheets drawn from tiles.
  - `start_page_1600.png`, `start_page_form_1600.png`, `start_page_form_1280.png` - New Project in the first slot
    and its form, Open Project File..., the search capped, the version in the corner.
- Not proven: a real drag from Explorer onto the Start page or a project folder (QA drives no mouse; the kit's
  `drop::simulate_files` has no MCP door), and the drag of a tile into the viewport end to end (L2a's suite).
