# Rust editor: where it stands

The hand-over note of the migration (`docs/RUST_MIGRATION.md` is the plan, `CONVENTIONS.md`
the rules). Whoever continues - a person, an agent, a later session - reads this first. It is
kept current: a finished package updates its row and adds what it learned.

## Decisions taken while building (they refine the plan)

- **No hybrid.** `Source/` is never edited. The C++ editor is the oracle the Rust one is
  compared with over MCP; both open the same project folder.
- **`gz-editor` is the context and the registry** (the plan's `gz-cmd` folded into it). It names
  no area. An area crate hangs its state on `editor.services` / `session.services` by type and
  registers its commands; so an edit to an area rebuilds that crate and the link only.
- **The wire is the old one.** One JSON line in (`{"cmd": name, ...args}`), one out
  (`{"ok":true,...}` / `{"ok":false,"error":text}`), on 127.0.0.1. The Rust editor listens on
  **28017** until the cutover (`-mcp_port`). `crates/gz-mcp/tools.json` is the bridge's tool
  catalogue exported from `tools/mcp/grizzly_mcp.py`: 165 tools over 157 commands. It is the
  parity list - every command in it must end up registered - and the source of the tool
  descriptions for the Rust bridge.
- **The window is egui 0.37 on wgpu 30 with a frame loop of our own** (`gz-app/src/window.rs`),
  not eframe: frames run straight from the event loop on demand (system redraw, egui repaint,
  `Wake` ping), so a minimised editor still runs its UI pass and is pictured through
  `egui_wgpu::winit::Painter`'s capture path. Measured: start 0.14 s headless, a screenshot of a
  minimised 2160x1350 window 70 ms. Leaving the loop must be asked from a message handler
  (`user_event`), not from `about_to_wait`, or winit sleeps first.
- **egui units are points.** `Pt` is egui's own unit; DPI enters through pixels per point. Font
  sizes in egui are em sizes: Body is 12 (a 16 pt line), see `gz-ui/src/font.rs`.
- **MCP reaches the UI through a queue**, not through shared state: a handler that needs the
  shell posts a closure that runs on the UI thread at the start of the next frame and answers
  through a channel (to be built with the shell). Reads of data go to stores and never visit
  the UI thread.
- **The viewport will render on its own thread into a texture** the UI shows (wgpu's device and
  queue are shared between threads); the UI thread only presents.
- **Compile time:** many small crates; `rust-lld` links; `cargo build --profile fast` is the
  everyday optimised build (no LTO, incremental), `--release` (thin LTO) is for shipping.
  `tools/rust/cg.sh <cargo args>` runs cargo with a summary of errors and heals a crate folder
  that is still being written.
- **Who does what:** the architecture and every decision are the lead's; implementation is done
  by agents on Opus 5.5 at high effort (never below), each from a brief file under
  `docs/rust/briefs/`. One agent per main task - related crates go to one agent, a task is not
  cut into small agents - and an agent never starts agents of its own. Briefs carry decisions
  and pointers to the C++; a stopped agent's raw reasoning is not handed on, its code is.
- **The end of the task:** when everything is implemented, ONE critic on Fable 5.2 reviews the
  whole work (architecture rules kept, the wire and the formats against the C++, what the fast
  tests show), before anything is called done. Tests run then and only then, and fast.
- **Briefs written:** `text`, `project`, `kit`, `icons`, `spawn-entities`, `ogf`, `tex-xr`,
  `game-link`, `mod-build`, `nq`, `characters-world`, `pda-screens`, `release`, `publish`,
  `shell`, `scene-core`, `render`.

## Crates

| Crate | State | Notes |
|---|---|---|
| `gz-base`, `gz-task`, `gz-store`, `gz-query`, `gz-log` | done | layer 0 |
| `gz-win`, `gz-crash` | done | `gz-win` is the only crate with `unsafe` |
| `gz-editor`, `gz-mcp` | done | context, registry, `AutoSave`, `fsx`, the endpoint |
| `gz-app`, `grizzly` | done | the shell: caption (menus, fold under 1100 pt, tab strip, drag band, window buttons, 6 pt resize edges), toolbar, status bar, work area, drawers, Task Center card, palette, keys, session, settings file `grizzly_editor.ltx`; `ShellBuilder` + `Document` for areas (`install(b)` lines go into `grizzly/src/main.rs` `install_ui`); `UiQueue`/`on_ui`; 22 commands (`state`, `ui_*`, `window_control`, `titlebar_info`, `reset_layout`, `theme`, `preferences`, `shortcuts`, `command_palette`, `progress`, `screenshot_editor`). Not matched: WM_NCHITTEST (no subclassing without `gz-win`), busy frame (no blocking jobs). Tests written, not run |
| `gz-chunk` | done | reader, writer, `lzhuf` (byte-equal compress), `lzo` (LZO1X + archive index helpers) |
| `gz-ltx` | done | `doc.rs` (lossless); `tree.rs`: `parse_file` (parallel per file) + `Ltx::assemble` / `Builder` (the sequential C++ load), `Dialect::Editor` / `Dialect::Game` (`#include_optional`, `*.ltx` includes, multi-line recovery, XMS layer merge, `.ltxp` patches) |
| `gz-luadata` | done | `parse` = LuaJIT 2.1 lexer/parser port (the editor's fork) + `DeclarativeProblem` + a run of the data subset with the interpreter's semantics; writer byte-equal to `NqLua.cpp` (`write_value`, `write_node`, `write_asset`); `syntax_check`; JSON |
| `gz-fmt-xml` | done | tolerant tree with `CXml` navigation, `preprocess` (`#include`), `strings` (editor scan / game XML, parallel build), `sheets` (`textures_descr`) |
| `gz-fmt-object` | done | `.object`, `.skl/.skls`, `.anm`, envelopes, normals, skinning; see findings |
| `gz-fmt-ogf` | done | every visual type read (FVF and skinned 1-4L decoded to one vertex), bones, IK, motions and `.omf` with key sampling; static and skeleton export, the face optimiser ported; bone OBBs and VIPM are not byte-exact (see findings) |
| `gz-fmt-level` | done | cform, xcform + overlay visuals, BVH, AI map, details, `build.prj`; see findings |
| `gz-fmt-spawn` | done | `all.spawn` (header, spawn graph with both packets as ranges of the file, artefact points, patrols, embedded game graph, unknown and compressed chunks kept), `game.graph`, `level.gct`, `peek_packet`, `SpawnIndex`; byte-equal round trips |
| `gz-entities` | done | every class of `xrServerEntities` in the game's layout, Dead Air's script fields, `FillProps` as typed properties with set by path; `Opaque` keeps what does not decode or does not write back to its own bytes |
| `gz-fmt-tex` | done | DDS read/write, TGA, THM (texture/object/group; the sound one is `gz-fmt-sound`'s, which tex depends on and re-exports), params, game texture, bump, thumbnail, `texture_tile`; reviewed by spot checks against `ETextureParams.cpp` |
| `gz-fmt-sound` | done | OGG comment params (versions 1-3) rewritten without re-encoding, sound THM, PCM via lewton, peaks; spot-checked against `SoundRender_Source_loader.cpp` |
| `gz-fmt-xr` | done | `lanims.xr` read/write + colour at a frame, `gamemtl.xr` read, `shaders.xr` read (constants, matrices, blender descriptions + raw property streams), `shaders_xrlc.xr` read/write, `particles.xr` read with the action stream typed, `senvironment.xr` read/write |
| `gz-ui` | done | theme, fonts, text, audit, layout, buttons; icons (all 189 glyphs, `icons/geometry.rs` + `icons/paint.rs`, `sheet`); widgets: `search`, `menu`, `toolbar`, `grid`, `list`, `splitter`, `state`, `card`, `dialog`, `units`. Not yet driven by a shell; tests written, not run |
| `gz-project` | done | the list, `project.ltx`, path rules, asset kinds, delete checks, file ops, DARF listing, the file tree + watcher; 16 commands; hooks for other areas (open/close/scaffold/rename, templates, asset lists); holds the linked game's `GameFiles` (`game_snapshot`, `refresh_game`, `link_game`, `game_rev`) |
| `gz-vfs` | done | `GameFiles`: loose `gamedata` over `fsgame.ltx`'s `$arch_dir...$` archives (later over earlier), modules of `modules\` / `mods\` as roots of their own; `get` / `list` / `find` / `read` / `read_many` from the index; the saved index (`appdata\game_index\<hash>.bin`) checked by folder / archive stamps; tests written, not run |
| `gz-game` | done | `GameLink` (project scope): `files`, `rev`, `refresh`, and as queries keyed by `rev`: `configs` (BFS include discovery, parallel parse, editor `CInifile` rules), `language`, `strings`, `texture_sheets`, `modes`, `spawn_sections`, `game_version`; `Bundled` + `read_game_or_bundled` / `spawn_roster`; commands `game_link`, `game_modes`, `list_spawn_sections`; tests written, not run |
| `gz-mod` | done | `mod.ltx`: its own lossless `ManifestDoc` (the C++ comment rules, see findings), the typed `Manifest`, `Edit`s, metadata rules, `[commands]` (`E401`..`W408`), `[exclude]` / `outside_build`, `ManifestService` (store + `AutoSave`, outside changes reloaded); `mod_manifest`, `mod_set_manifest`, `mod_exclude`, `mod_commands`; the scaffold hook (`gz_mod::install`); slots `ManifestFields` (`world`) and `ManifestArgs` (`tags`, `thumbnail`) |
| `gz-release` | done | Package Release as the job `package` (group `build`) staged through `gz_build::run_build`; own ZIP writer (parallel level-9 deflate, ZIP64) and reader (index read back from the archive); index, descriptor, `[card]` as the C++ text; author key file (`xms-key 1`, PBKDF2 600000 + HMAC stream + MAC) and ECDSA P-256 signing/verify in pure Rust; MOD_CATALOG 8 / 10.3 content rules; tags and thumbnail hooks of `mod_set_manifest`; `mod_package`, interim `mod_catalog` (key actions only, `key_action` / `key_state` for `gz-catalog`). See findings |
| `gz-render` | done | wgpu 30 forward renderer, no window, no scene: `DrawList` -> `build_packet` (parallel cull or the caller's `Visibility`, instanced groups by mesh + material, blended back to front, helpers, labels, nearest 128 lights) -> the render thread (newest packet wins, passes in parallel chunks, frame clock `wait_frame`, `capture`, `Wake`), `render_offscreen`, `ThumbnailCache`; `Assets` (caller's `AssetLoader`, IO + CPU jobs, LRU by bytes, BC1-3 as they are), `MaterialTable` (data, 31 rules) -> 13 editor materials in WGSL; Lit (studio) / Unlit / Wireframe / Game Lighting (`glit_light` ported); id-target outline; label occlusion queries; GPU skinning; `examples/offscreen.rs` rendered and looked at. Tests written, not run |
| `gz-build` | done | `BuildWalk` from the file tree, `BuildCheck` / `BuildChecks` (+ the bake slot), target refusals, the clean build in `modules\.new\<id>` with hard links and two renames, the flat export + report, `run_build` (for `gz-release`), `Builds` (job `build`, results, Task Center `BuildStatus`, `CleanQuestion`); `mod_export` |
| `gz-github` | done | the network side: `reqwest` on native-tls (Windows certificate store) + system proxy, three clients (API and uploads never follow a redirect, public files do), every request raced against the job's `Cancel`; `Secret` (zeroed, `Debug` redacted); `Endpoints` (real or `-qa_github` mock, `upload_url_trusted`, `vt_url_trusted`, the catalog's repo and mirrors); account (env -> Credential Manager -> `gh auth token`, a minute's cache), device flow; `publish` (repo, foreign latest, published tag / version, draft reuse, leftovers, uploads 3 at a time with the descriptor last, stored assets checked, latest, the served check); list / edit (`make_latest` where the mark is) / delete (release, then tag); `fetch_public`; catalog issues (`find_issue`, bot-only newest verdict, submit / withdraw / issue); VirusTotal (pace slots, policy, verdict, lookup, upload with the upload address check) and the scan (`lookup`, `scan`, `ORDER`, `describe`, `json`); ini helpers of the catalog files. Tests written, not run |
| `gz-publish` | done | `Publisher` (job `publish`: packaging through `gz_release::run_package` in group `build`, then the scan of a listed module + the upload in group `publish`; `ScanFirst` word for word; Task Center cards `key` `publish`), `FreshTag` / `deleted.tags`, device sign-in, `ReleaseList` (list, edit, delete, select, `show=edit` as a request the UI takes); `mod_publish` (answers when the packaging ended, as the C++ `FollowsJob`), `mod_releases`. Tests written, not run |
| `gz-catalog` | done | `Catalog` (one job at a time, group `catalog`: `check` = section 7 with the content rules over the local copy's zips, `vt`, `submit`, `submission`, `withdraw`), `Release` read as the game reads it + signature, `read_listing`, `vt_level` (never optional), the submission text / details, `website_is_module_page`; the real `mod_catalog` (key actions through `gz_release::key_action`). Tests written, not run |
| `gz-scene` | done | `SceneState` in one store (objects `OrdMap`, per-class order, names, selection, `ToolSettings`, `BulkData` with the AI map in 4096-node tiles), `SceneCmd` (place, delete, rename, transform batches with the group rules, select, show, tool eyes / read-only, group / ungroup, property replaces checked by `serial`, mod marks, order, snap list, bulk, saved, touch); all 16 classes read / written in LTX and stream (`objects`), the main file + parts (`io`: LTX, binary `.level`, undo / recovery stream, unknown sections / chunks / refused versions kept); `props` (rows, merge, text set, labels / units / help); library, pick tree (`Derived` by scene + library rev), drop / snap, recovery (`backup
| `gz-export-level` | done | The module's level layer of `EditorModScene.cpp`, pure over a scene snapshot + gathered disk state (`gather`): `.xspawn` (`op = add/start/remove`, `[path:]`, custom data with `[xms_order]`, fixed bones checked against the OGF, smart cover, lamp, zone shapes, every C++ warning, merge or mirror), `overlay.xcform`, cuts (+ `[cut_N]`), world-space `.ogf` overlay (parallel) + `[v_N]` registry; `%f`/`%g` exactly as UCRT (`cfmt`); `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe`; `install` fills `gz-build`'s bake (the mirror pass + its note), `gz-world`'s `SceneStarts` (E201 is `gz-world`'s) and `gz-nq`'s `SceneLookup` (restrictors, story ids, way paths, level). Tests written, not run |
| `gz-levelbuild` | done | The full level build: `build.prj` (static + MU objects, LODs merged into `level_lods*.dds`, hemi / sun / point lights, glows, portals, sectors, wallmark faces), `level.ltx`, `BuildGame` files, HOM/SOM, `build.aimap`, `build.details` + `build_details.dds`, `level.wallmarks`, scene stat; CForm; level graph, game graph + cross table (`CGameGraphBuilder`), multi-level merge (`CGraphMerger`); `editor.spawn` (spawn constructor, patrols, artefact points, graph embedded); `nav` = the engine's `ILevelGraph` lookups + A*; XrLC / xrDO / xrAI / the game as `tokio::process` jobs (`level_run`, group `game`, Stop = `taskkill /T`); 19 commands named as `gz-ui-level` expects + `level_rebuild_status` + `level_build action=`. Tests written, not run |
ecovery`, the C++ manifest); 34 commands, `list_assets` Objects / Groups |
| `gz-lua` | done | LuaJIT 2.1 vendored (`mlua` 0.12, JIT off, count hook = instruction budget); `LuaHost` pool of states (one per caller, scripts kept per revision), `Script`, `Arg::{Value, Chunk}` (a text run in an empty env: the asset is the table LuaJIT builds), `call_omitting`; sandbox = base + string/table/math, the C++ reader's 18 globals; `vanilla_source` rewrites the fork's `//` `/* */` comments; tests written, not run |
| `gz-pda` | done | `evaluate` (gz-luadata check, then the game's `normalize`), `Layout`/`Item`/... read as the C++ read the stack, `preview` block, `template`; `Preview` state machine (pages, vars, log, toasts, rows, scroll, keys, map zoom, boxes / hit test / `visible` in canvas units); `PdaService` (reader and layout `Query`s, W320 textures, list, `[pda] layout`); the build check; `content_create kind=pda` template; `pda_list`, `pda_get`, `pda_write`, `pda_delete`, `pda_use`, `pda_validate`; tests written, not run |
| `gz-ui-app` | done | the Start page (`ShellBuilder::start_page`: New Project form, Open Folder via `rfd` on a thread of its own, search, Last Opened / Name sort, 232x184 cards with twin parents, previews decoded on the pools through a `Query` keyed by path + write time, `ShowsNoScene`, card menu, `dlg.delete_project` counted off the UI thread), Link Game page card, the Output Log (one body for the drawer and the Level panel `output_log`, cursor over `gz_log::Log`, distinct counting per window from "Project opened:", the `log_state` hook), Preferences (`preferences` tab: 44 rows of `EditorPreferences`/`LevelPreferences`, nav, search, Keyboard page over `shortcuts` with an Assign dialog `dlg.assign_shortcut`); command `project_browser_menu`. Rows whose key `gz-app`'s `PREFS` lacks (`object_flags`, `recovery_*`) show their default read-only until those keys are added there. Tests written, not run |
| `gz-nq` | done | quests and behaviour graphs: `asset` (typed `Graph` with `imbl` node/task vectors, read with the C++ shape messages, `write` byte-equal to `NqLua::Write`, outline, topo order, templates), `catalog` (hand-rolled reader, fold order game|bundled -> modules -> project, `Catalog::is_trigger`, exclusive forms, JSON), `layout` (`node_box`, the one rectangle; the dominator-tree auto layout on bitsets), `validate` (every code and message of `NqValidate`), `references` (typed walker, `find`, `rename_task`), `index` (snapshot, fingerprint, search), `pickers` (15 types from the game's ltx tree / strings / XML / `all.spawn`, the project's custom data, `spawn.part`, looks; `compose` with the scene), `caps` (`class_registrator.script` families), `doc` (`Store<GraphDoc>`: every `quest_apply` op, canvas ops, view state with bookmarks / history / find), `NqService` (queries keyed by game rev, file stamps, manifest, docs' edit serials; slots `CharacterLookup` / `SceneLookup` / `WorldLookup`), the build check (`E113` across graphs); 24 `quest_*` commands (all but `quest_view`), templates, rename hook, `DirtySource`. Tests written, not run |
| `gz-screens` | done | `Screens` model parsed off `ManifestDoc`, written back with `AppendScreens`' rows (sections in place while their order holds), `import` (+ `plan`: the decision by place and extension; parallel-safe copies), `validate` (E401..W411), the build check, `GameLevels` slot for W407; `screens_get`, `screens_set`; tests written, not run |
| `gz-characters` | done | `model` (`Character` keeping unset unset, `parse` / `load_text` / `write` as `EditorCharacters::Parse` / `Write`, `{ rus = .. }` alone collapsed on write, `text_in` / `text_raw` / `text_set`, `template`, `game_id`), `resolve` (`E111` / `E112`, kits merged, a shop whole, defaults once), `validate` (every C++ code word for word over `Lookups` = `gz_nq::Pickers` and `Models`; `E123` from `OGF_S_BONE_NAMES` read off the chunk list, cached per model and game rev), `game` (trade presets, bone names, stalker damage bones); `Characters` service: `Roster` as a `Query` keyed by a stamp of the `.npcasset` files + `[exclude]`, read in parallel; fills `gz_nq`'s `CharacterLookup` on project open (briefs refreshed in the background, the file tree's until the first roster); `Checker` (linked game: pickers, presets, models); build check (first error, as `CheckForBuild`); `content_create kind=character` template; `characters_list`, `characters_get`, `characters_write`, `characters_delete`. Tests written, not run |
| `gz-world` | done | `model` (`World` / `StartKit` read off `ManifestDoc`'s raw lines as `EditorMod::Load`, written back as `Edit::SetLines` in the `Save` layout, JSON, `base_kit`), `game` (`read_all_spawn`: `gz-fmt-spawn` container + `SpawnIndex` in parallel, level changers by class `LVL_CHNG`, destination from `gz-entities` with the C++ byte walk as fallback; levels, transitions, patrols sorted), `starts` (`op = start` of `spawn\*.xspawn`), `validate` (`E201`..`W208`); `WorldService` (game world as a `Query` keyed by the game rev, captions from the Level picker, starts with the scene's through the `SceneStarts` slot, problems); fills `gz_nq`'s `WorldLookup` (kicked on first ask), `gz_screens::GameLevels`, `gz_mod::ManifestFields` `world`; build check; `world_get`, `world_set`, `world_transitions`. Tests written, not run |
| `gz-ui-mod` | done | Mod Settings tab (`mod_settings`: nav of 7 pages with problem dots, 44 em form, the C++ `CheckSettings` sentences under fields, saved as edited through `mod_set_manifest` / `mod_commands` / `pda_use` once no field is being typed in; untouched fields follow outside saves); Build to Folder / Export Flat / Package Release dialogs, `dlg.clean_target`, `dlg.replace_release`, the kit `Result` (`build_result`, `export_result`, `package_result`; `ui_dialog open` puts the last one up again to look at), `KeyLine` (the author key row); Task Center cards of every `build` / `package` job (`cards`); Mod menu rows in `MOD_MENU` slots and the Build Mod split; `mod_settings_view`. Looked at on a temp copy. Tests written, not run |
| `gz-ui-content` | done | the Content Browser: one `ContentBrowser` service, two hosts (`content_browser(ui, cx, Host)`: the Content Drawer via `install`, the Level panel `content_browser` - the Level document calls it with `Host::Panel`); sources Project (`FileTree` snapshot), Editor (each category a folder of the data root, walked with `FileTree::scan`), Game (`GameFiles`) as `Catalog`s built on the pools, folder views as a `Query` keyed by (source, folder, search) + revisions; pictures: `texture_tile` on the pools for the size drawn, models through a renderer of the crate's own + `ThumbnailCache`, textures released 3 frames late; `caption` (pure `DrawTileCaption`); every action an editor command; `DragPayload`; `content_browser_navigation` / `_open` / `_selection` / `_copy`. Looked at: tiles 48/96/160/192, list, textures, models, rename box, menus. Tests written, not run |
| `gz-ui-world` | done | the Characters tab (`characters`: list 260 pt with search, dirty dots, Template badges, splitter, form up to 760 pt bound to the RAW values, base's values as hints / muted kit rows, Ctrl+S = `characters_write` with `gz_characters::write`, the file follows the id via `content_rename`, `dlg.delete_character` -> `characters_delete`, unsaved prompt before any switch, problems resolved + validated on the pools) and the World Rules tab (`world_rules`: every edit a `world_set`, sent in order; Original Game / Player Start / Starting Kit, More with Applies On / Playable Area / Transitions; problems bar + drawer; `dlg.empty_world`); `Lists` (pickers, trade categories, `meshes\actors` models, read on the pools, busy until read); portrait sheets decoded on the pools, kept per project; `item_picker` (both tabs); Mod menu rows; `characters_view`, `world_view`. Looked at on a temp copy. Tests written, not run |
| `gz-ui-release` | done | Publish Release (`publish_release`, L: one Release line + Notes, the rest under More unless it stops the release; reads the releases as it opens, a version that is out is a problem with Open Mod Settings / Open Releases, a failed read a muted note; account rows, device code in Title Mono, token field; Publish closes it, the card follows, the kit `result` `dlg.publish_result`), `dlg.edit_release` (also from `take_edit_request`), the Releases tab (`releases`: sortable table, details with Edit... / Open Page / Delete... GhostDanger, row menu, Del / F5, the kit `confirm` `dlg.delete_release`, banners), Submit to Mod Browser (`submit`, fixed 600 pt body, five cards, VirusTotal card in the Required tone, footer busy + Stop <job> + Hide, label sets) with the sheet `dlg.catalog_withdraw`, follows `Catalog::window_open` / `wants_withdraw_question`; toolbar Publish Release / Releases, Mod rows at `gz_ui_mod::mod_menu_slot`; the key row is `gz_ui_mod::KeyLine`. Every button runs the domain path its command runs (`Publisher::start`, `ReleaseList::start`, `Catalog::start`, `AuthorKeys`, `Github` token / VT key). Locals: `local_secret_field`, `local_secret_row`, `local_code`, `local_card_title`, `local_fold`, `local_table_head`. Looked at under `-qa_github` with dead loopback endpoints; tests written, not run |
| `gz-ui-pda` | done | the `pda` tab (singleton view over a session `PdaTab` model behind one lock: file, Source text, the reader's answer via `PdaService::evaluate`, `gz_pda::Preview`, view, last frame's geometry; retargets through the shell's unsaved prompt, follows the file on the tree revision and before every `pda_preview`), the canvas (device of `pda.xml` / `pda_16.xml`, sheets / fonts / level maps from `PdaAssets` - decoded on the pools, uploaded once, dropped with the session, the link or Reload; bitmap glyph quads; Fit / 100% / wheel-at-mouse / pan in px semantics; screen edges; stock panel, toasts, map with actor and markers), Details tabs, Source pane (Mono, gutter, error band, Apply / Ctrl+Enter / Ctrl+S); `pda_preview` (canvas-unit state, `map.bound` / `spots`, waits for 2 frames when owed); the `screens` tab (lists, 16:9 preview with DDS / GIF thumbnails, layers with drag reorder, music combo, More, track form; edits as `Change`s applied locally and saved through `ManifestService::edit` on the freshest model); `local_file_drop_zone` (Explorer drops wait for the pointer's next move, `gz_ui_content::DragPayload` and std path payloads, inner zones first, `simulate_drop`); `screens_view` (waits for the drop's import and save); `COMMAND_MOD_SCREENS` / `COMMAND_MOD_PDA` at `gz_ui_mod::mod_menu_slot`. Looked at on a temp project: 4:3, 16:9, Source with an error, a two-layer screen. Tests written, not run |
| `gz-ui-previews` | done | the `model` tab (own `gz_render::Renderer` sized to the canvas, a hand-built `DrawList`: the model posed by `idle` / its first cycle, a floor of round metre steps under it, bones as overlays; orbit / pan / zoom-at-cursor `Orbit`; packets built and submitted on the CPU pool only when the view changed or went stale; Details grid), the `texture` tab (read + uploaded on the pools: native DDS as the GPU takes it - BC1-7, 32-bit, R8, RGBA16F, every whole level - else decoded to B8G8R8A8; Fit / 1:1 / Zoom / channels / Mip / Details), both drawn through `image_pass` (a WGSL pass in egui's render pass: straight alpha, channel and alpha-as-grey views, opaque renders, nearest past 2:1); `loader::Sources` = project, editor, linked game (also the renderer's `AssetLoader`); Light Animations (`Store` with undo over `lanims.xr`, timeline, key bar, Play), Texture Library + New Textures (`.tga`/`.thm` walk as a `Query`, `Store` of edited descriptions, save/import as a job writing `.thm`, `.dds`, `.tga`; Types filter; texture picker), Sound Library (`.wav`/`.thm` walk, waveform from `Pcm::peaks`, `cpal` audio thread, save rewrites `.thm` and the `.ogg` params in place); Libraries menu incl. Check New Textures, Synchronize Textures... / Sounds... (confirm, then jobs); `preview_model`, `texture_preview`, `library`, `asset_preview`. Not done: Vorbis encoding (Rebuild / changed .wav), the cube map Smaller Copy button. `gz-app/src/window.rs`: BC feature requested when the adapter has it, device errors logged. Looked at on a temp project. Tests written, not run |
| `gz-ui-level` | done | the Level document (kind `level`): a dock of its own over the kit's splitters (`layout`: Wide / Compact builds, four slots, open / slot / front per panel and the sizes in `level.*` layout keys, custom = placement or 8 pt-rounded sizes differ from the build; `dock`: side slots keep pt, viewport min 360 x 240, splitters last; 26 pt tab strips with cross on hover and a Move To / Close menu; F11), the viewport pane in the centre; Level toolbar over registry commands (`toolbar::LEVEL_ACTIONS` - `build_cform`, `build_aimap`, `build_game_graph`, `build_all`, `play`, `play_game`, `play_clean`, `play_stop`, `reload_configs` - added only when registered), status centre + Rebuild chip over `level_rebuild_status` (polled, cached); World Outliner (`outliner_model::compute` as a `Query` by scene rev + filter, Unreal search grammar, natural sort, By Hierarchy, virtualised rows, Filters badge, More, Type / Owner columns, `dlg.rename_object`, `dlg.delete_object`); Tools (tiles, sticky header, Object / Spawn / Particles lists from `lists::Lists` read on the pools, Group, Detail Objects + `dlg.detail_objects`, Select by Reference, Random Settings `dlg.properties`, Snap Objects; picks go to `gz_ui_viewport::set_place_item`); Details / Level Settings (rows prepared as a `Query`, categories, kit grid, transform drags merged + sealed, other values sent on release, `dlg.choose` over the lists, `dlg.edit_text`, Quality segmented); Scene / Build / View menus and 11 scene `UiCommand`s; Recovery page card; `dlg.import_base_scene`; `preview.png` on save and a minute after a change; `outliner_show`, `outliner_filter`, `scene_visibility`, `property_inspector` (wraps gz-scene's, adds the panels). Tests written, not run |
| `gz-ui-viewport` | done | the Level tab's 3D view: `ViewportPane` (`new`/`ui`/`inactive`/`shortcut`/`mcp_state`, plus `set_maximized`/`take_maximize_request`), `Viewport` service (camera `Nav` = `CUI_Camera` + `ViewportNavigation`, view flags, tool and snaps, Game Lighting, render size), pure `draw::build` (scene snapshot -> `DrawList` with every class's helpers in the C++ colours) + `draw::visibility` from the scene's pick tree, packets built on the CPU pool only when scene/library/view/assets change, egui overlays (pills, 64 pt axis gizmo, transform gizmo, Mono stats pill, labels, safe rect, marquee), picking/select/place/move/rotate/scale as `SceneCmd`s (one merged undo step per drag), Game Lighting (`lighting`: weathers, user.ltx, lamps/campfires/level lights, roof map), the asset loader (project, editor, game), the camera per scene in `<project>	empiewport.ltx`, `capture_preview`, `set_place_item`, `view_menu`, `install` (Keyboard page commands); 9 commands (`camera_get`, `camera_set`, `focus_object`, `viewport_navigation`, `viewport_controls`, `view_mode`, `game_lighting`, `frame_pacing`, `screenshot_viewport`). Looked at in the running editor; tests written, not run |
| `gz-ui-graph` | done | the graph tab (kinds `quest` / `beh`, ids `quest:<rel>` / `beh:<rel>`): `Scene` built per revision on the CPU pool from `gz_nq::layout` (texts, pins, beziers + back edges routed like `RouteBack`, 512 pt grid index, extents, sorted edges for `beyond`, the minimap's mesh) - a frame draws only the cells in view; `Canvas` (10 zoom levels, LOD by title px, pan / wheel / drag / link / marquee / action drag, edge pills, minimap, context menus, kind pickers by the document's vocabulary); a `ViewMirror` of the store's view, posted when a gesture ends and taken back when the document moved it; `Inspector` (Graph panel: header, Tasks or Roles, Variables, Global -> `mod.ltx [vars]`; Details: node, action under `graph.action_split`, help footer) staging a copy committed as `set_node` / `set_quest` (Replace only for roles and a variable rename); typed editors of `DrawTyped` in `params`; dialogs (Find in Project, Rename Task, Task References via their commands, Edit Text / Lua); `quest_view` (C++ wire, held for its frames), `quest_open` brings the tab up. Not ported: the camera button of a position. Tests written, not run |

## Findings worth keeping

From the object/mesh port:
- Both `CalculateTB` bodies of the OGF exporters are commented out in the C++: no NVMeshMender
  port is needed.
- The skeleton exporter never welds vertices (every split is a triangle soup), which by
  analysis reduces VIPM and the face optimiser to fixed reorderings - verify before relying on it.
- `.object` details that cannot be byte-equal because the C++ writes arbitrary data: surface
  face-list order (address order of surfaces; Rust keeps file order), unused vertex-map
  reference slots (uninitialised in C++, `0xFFFFFFFF` in Rust).
- Not done in `gz-fmt-object`: `.bones` files, `GenerateBoneShape`, `OptimizeMesh`, envelope
  `ScaleKeys` / `NormalizeKeys`.

From the level port:
- Not written: the compiled `level` file (`fsL_*`, sectors, glows), `EDetailManager::Export`'s
  texture merge.
- Byte differences against C++ output that are padding or uninitialised memory there:
  `build.prj` struct padding, the four spare bits of an AI node's link block.

From the text port:
- `NqLua::ParseValue` is LuaJIT itself, so the port is LuaJIT's compile step (every syntax message,
  token for token) followed by a run of what `DeclarativeProblem` lets through. A constructor's
  constant fields go into a template and the others are stored at run time afterwards, so
  `{ a = -"5", a = 1 }` reads `a = -5`: a computed value wins over a constant one whatever the
  order. `-0` and `1e999 - 1e999` are never folded (LuaJIT avoids -0 and NaN constants).
- The editor's LuaJIT is a fork with `//` and `/* */` comments; `/* */` counts no lines and a `*`
  that does not close swallows the next character. `DeclarativeProblem` refuses `/` anyway.
- A chunk name over 59 bytes (a long file name) is cut in LuaJIT's messages; the C++ then fails to
  strip it, so such an error keeps the `name:line:` prefix and reports line 0. Ported as is.
- The editor's `CInifile` skips a missing include (warning); the game's stops on it unless it is
  `#include_optional`. The game's `*.ltx` include adds the mask's folder twice
  (`configs\x\*.ltx` lists `configs\x\x\`) - ported as is, marked `// oracle:`.
- The editor's string-table reader is a text scan (first definition wins, text as written);
  the game's is the XML tree (condensed, later wins). `gz_fmt_xml::strings::Mode` offers both.
- Not in `gz-luadata`: the typed quest model and its outline (`QuestToValue`, `Outline`, the
  header of `Write`) - that is `gz-nq`; `write_asset(head, top)` takes the header ready made.

From the text port (interrupted pass):
- `rtc` adds no framing: a bare LZO1X block; sizes live beside it. A game archive's directory
  is chunk 1 (LZHUF when flagged), one record per file: `u16 n`, `u32 size_real`,
  `u32 size_compressed`, `u32 crc`, name, `u32 offset`; equal sizes mean stored as is.
  `gz_chunk::lzo::{read_archive_index, unpack_entry}` implement it.

From the engine's own headers (applied to `gz-chunk`):
- `w_float_q16` / `w_float_q8` round (`floor(q * max + 0.5)`); `r_float_q8` divides by
  `255.0001`.

From the project port:
- `_projects.ini` and `project.ltx` are not `AutoSave` documents: the C++ edited them in place
  (`WritePrivateProfileString`) and QA restores them behind the editor's back, so every edit
  re-reads the file, applies itself to it and writes only a changed text (under a lock).
- Areas plug in through `gz_project::{ProjectHooks, AssetTemplates, AssetLists}`: the module
  area must register the `mod.ltx` scaffold (`on_scaffold`) - until it does, a created project
  has only `project.ltx`; quests/characters/PDA register their `content_create` templates; the
  object library / game link register the `list_assets` categories other than `Levels`.
- The shell's `state` command takes its project fields from `gz_project::project_state`.
- `darf.rs` was a private archive reader; it is gone - `darf_list`, `darf_copy` and the file
  ops read `gz-vfs`'s `GameFiles`, which `Project` mounts on first need. `game_link` moved to
  `gz-game` (the setter is `Project::link_game`).

From the game link:
- The index keeps the C++ editor's order: keys have `/`, but they sort as their backslash
  spellings did (`a1.ogg` before `a/x.ogg`), so `darf_list` and the mode captions' "first file
  wins" come out as before.
- Archives are those of `fsgame.ltx`'s `$arch_dir...$` aliases in its order (`database\` without
  one); the engine meets them while scanning any alias. An archive's `entry_point` and its
  `auto_load = false` are ignored, as DARF ignored them. Marked `// oracle:` in `gz-vfs`.
- The configs are read with the C++ editor's `CInifile` rules (`Dialect::Editor`: a missing
  include is skipped), as `EditorGameConfigs` handed them to it; cp1251. Without a link (or when
  the linked configs fail) `list_spawn_sections` reads the editor's own `gamedata\configs`.
- Whatever writes into the game (Build Into Game) must call `GameLink::refresh()` afterwards:
  the C++ called `EditorGameModes::Invalidate`; here the modes, configs and strings follow the
  new index revision by themselves.
- `game_link` / `game_modes` refusals carry `error` before their fields (the shared error
  reply); the C++ `game_link` refusal had no `error` key at all.

From the OGF port:
- Verified: both `CalculateTB` bodies are commented out (skinned vertices carry the
  constructor's tangent (1,0,0) and binormal (0,0,1)); neither exporter welds; the static
  exporter's `MakeProgressive` returns at once. VIPM runs only for a skeleton flagged
  progressive - not ported, such an export is ordered as a plain one, with a warning.
- The skeleton's face order comes from XrETools' lookahead optimiser (`D3DX_OptimizeFaces`
  over `_OptimiseVertexCoherencyTriList`, mode 2): ported float for float in
  `gz-fmt-ogf/src/optimize.rs`.
- Bone OBBs: the C++ fits them with Wild Magic's `MinBox3`; the port fits a covariance box,
  so `OGF_S_BONE_NAMES` differs from the C++ in those 60 bytes per bone.
- Oracle quirks kept: a two-sided static surface is walked twice (four faces per face); the
  default bone part of `OGF_S_SMPARAMS` is written without bone names; the build time is 0.

From the entity port:
- The game's layouts differ from this repository's `Source/xrServerEntities` in: the weapon's
  `m_condition_type` (state and update), `CSE_ALifeCustomZone` keeping `m_maxPower`, one 64-bit
  bone mask in `SPHBonesData`, the quantised item update below version 122, the smart cover's
  `can_fire` default, the squad roster limit, the shotgun's cartridge cap.
- Dead Air's scripts are packed; an unpacked copy is in `D:\Games\Dead Air\_work\da_unpack`
  (`xtra` overrides `configs`). The repository's `gamedata\scripts` are Call of Pripyat's and
  differ (smart terrain, squads, zones): never take entity layouts from them.
- A decoded entity keeps its packets and writes them back unchanged until edited; a packet of
  today's version that does not re-encode to its own bytes is `Opaque` (`Lossy`).

From the kit:
- `units::em(n)` is n x 16 pt (the C++ `Em`, the Body line), not the egui em of 12: widths in the
  design (18 em stacking, 36 em prose) are in it. `text::prose` still wraps at 36 x 12.
- egui strokes mitred with butt caps: icons above 2 px of an opaque stroke are cut at their
  corners with a disc on each (round joins and caps); at 20 px and below points snap to pixel
  centres (odd widths) or edges (even) and a backtrack pass drops the spikes. egui fills only
  convex paths, so the two concave fills (`star_filled`, `bookmark_filled`) are ear-clipped meshes
  at build time; rounded rects are paths built from the snapped corners (egui's corner radius is
  a `u8`).
- Dialogs are `egui::Modal`s positioned by hand; egui hides an area on its first frame, which
  stands in for the C++ "a measuring frame is not shown". `dialog::draw_overlays` must run last
  every frame: it draws the queued questions, runs the answers and rotates what `open_dialogs`
  reports.
- MCP's unsaved prompt answers Cancel (the command decides with `save` / `discard` first).

From the shell:
- An undecorated window reports its taskbar button's size while iconic: `window.rs` ignores
  `Resized` then and feeds egui the last restored size, so a minimised editor is pictured whole.
- `gz-crash` makes every main-thread panic fatal; the shell wraps that hook so a panic inside a
  document's `body` only logs (unless `-nodlg`) and the tab shows its error page.
- The shell knows the project only through `gz_app::project::set_project_probe` (main installs
  `gz-project`'s answer); jobs publish their card JSON into `gz_app::TaskCardTable`, an `Editor`
  service keyed by a std type so area crates below the UI need no `gz-app` dependency.
- A kit question's `then` is `Send` and cannot touch the shell: it posts to `UiQueue::post`.
- egui 0.37 deprecates `Id::new` and `Ui::id`: use `Id::unique` / `Ui::scope_id`.

From the manifest and build port:
- `mod.ltx` is not read with `gz_ltx::LtxDoc`: the manifest reader (C++ `Load`, the game's XMS)
  takes only `;` as a comment, outside quotes with `\"` escapes. `LtxDoc` cuts `//`, so
  `website = https://...` would read `https:`. `gz_mod::ManifestDoc` is the lossless document
  with the manifest's rules.
- The file tree never holds tooling entries, but `left_out` must name them (QA asserts `.git`,
  `Thumbs.db`, `gamedata/.git`): the walk lists each folder it enters once more for tooling
  names only. A field of skipped names in `gz_project::Dir` would remove that listing.
- Not ported: the C++ free-space check before a build (`GetDiskFreeSpaceEx` needs a `gz-win`
  function).
- Areas plug in: `gz_build::BuildChecks::of(editor).add(..)` (quests, characters, world, PDA,
  screens), `.set_bake(..)` (the scene), `gz_mod::ManifestFields::of(editor).provide("world", ..)`
  (the `world` field of `mod_manifest`, `null` until then), `gz_mod::ManifestArgs::of(editor)
  .provide("tags" | "thumbnail", ..)` (the catalog area; until then those arguments are refused).

From the release port:
- Byte-equal to the C++ by construction: the file index, the descriptor head, `[card]`, the key
  file, the signed text framing. NOT byte-equal: the packages (zlib-rs deflate and our own
  headers instead of minizip/zlib - the index is read back, so the contract holds) and the
  signatures (RFC 6979 instead of CNG's random nonce - they verify the same, and match the
  catalog's `xms_catalog.py`, which is RFC 6979 too).
- Dead stages are found by a lock file (`owner.lock`, held unshared) instead of the C++ pid
  check; a C++ editor's stage (no lock file) is left to it.
- Documents is read from `HKCU\...\User Shell Folders\Personal` through `reg export` (UTF-16,
  no `unsafe`), `%USERPROFILE%\Documents` as the fallback.
- While the module stages, `gz_build::Builds::status` shows "Building Module"; the Task Center
  should show `gz_release::Releases::status`, which overlays the build's row.

From the publish / catalog port:
- The Credential Manager is used through `windows-native-keyring-store` (+ `keyring-core`) with
  an explicit `target`, not the `keyring` facade: the targets stay the C++ editor's own
  (`GrizzlyEditor:GitHub`, `GrizzlyEditor:VirusTotal`, `...:qa`) and the secret its raw bytes,
  so a token either editor saved is the other's too.
- `mod_publish start` answers in one call once the packaging job has ended (the C++ answered
  `job_pending` and the pump asked again with `answer_only`; `answer_only` still answers the same).
  The upload is a second job of the same card (group `publish`), so builds are free meanwhile.
- Uploads go 3 at a time (the C++ uploaded one after another); the descriptor still goes last.
  The card and `sent` add up the assets in flight.
- The UI half is not here: the Releases tab and the Publish / Submit windows set
  `ReleaseList::set_tab_open`, `set_publish_dialog_open`, `Catalog::set_window` and read
  `take_edit_request` / `wants_withdraw_question`; until they exist `tab` and `window` answer false
  unless MCP `show` set them.
- `reqwest` copies the bearer and the `x-apikey` header into its own buffers (marked sensitive,
  but not zeroed when freed); every copy of ours is zeroed.

From the renderer:
- glam 0.34 has no `Mat4::look_*` / `perspective_*` (they moved to `glam::camera`); `gz-render`'s
  camera builds its own left-handed, reverse-Z matrices. Clockwise faces are front faces.
- The window's device is egui-wgpu's default and asks for no features: `gz-app` must request
  `TEXTURE_COMPRESSION_BC` when the adapter has it, or every DDS is decoded on the CPU. wgpu's
  default uncaptured-error handler panics; the owner of the device should install its own.
- `R32Uint` has no MSAA: the id target and its depth are 1x, a pass of their own before the scene;
  a see-through surface writes it only when selected, so glass never cuts an outline.
- The UI shows `TargetHandle::view` with `TargetHandle::uv()` (the texture is bucketed) and
  registers the view again when `serial` changes.
- A packet that drew a placeholder says `waiting_on_assets()`; the adapter rebuilds it when
  `is_stale(assets)`. Textures that arrive later are picked up by the render thread itself.
- Not drawn yet (materials exist): particle systems, detail objects, glows, wallmarks, level
  geometry from `gcontainer` OGFs, cube-map skies (a sky texture is sampled as 2D).

From the PDA and screens port:
- LuaJIT's vendored build (`msvcbuild.bat`) runs `minilua` from its own folder; a shell with
  `NoDefaultCurrentDirectoryInExePath=1` (Claude Code's) breaks it ("minilua is not recognized").
  Build with the variable unset: `env -u NoDefaultCurrentDirectoryInExePath bash tools/rust/cg.sh ...`.
  The first `cargo check` of `mlua-sys` took about 17 s.
- The asset is compiled by LuaJIT itself (after `gz_luadata` accepted it), so `pairs` walks the
  game's order; the reader's answer crosses as `Value` with `state` and `lists` left out (the
  run-time rows bookkeeping; the preview never called `list_rows`).
- `list_assets` has no PDA or screens category (C++ `CATEGORIES` is closed): nothing registered.
- W407 (a screen's level the game lacks) waits for the world area: `gz_screens::GameLevels::of
  (editor).provide(..)`; unchecked until then, as the C++ while `EditorWorld::Available()` was false.
- `screens_set action=drop` makes screens and tracks for the loading surface; the C++ used the
  tab's selected surface (loading unless the author switched it).

From the Content Browser:
- `content_create` lands in the browser's folder only when `folder=` is passed (`gz-project`'s
  handler; its templates are `pub(crate)`, so the browser cannot take the command over). The C++
  made it in the folder the browser showed: QA's `test_asset_icons.py` A needs a hook in
  `gz-project` (a default-folder provider) to pass again.
- The Editor source is files, not the C++ library references: each category is a folder of the
  data root (Objects = `rawdata\objects`, Visuals = `gamedata\meshes`, ...); Particles,
  Entities and Light Anims live in one shared file each and list nothing. `content_browser_open`
  and `_copy` still find a reference without its extension.
- Bash heredocs eat `\` (MSYS): MCP drivers with Windows paths are written with the Write tool.

From the scene port:
- The C++ `CInifile` writer sorts sections and keys (`strcmp`), lower-cases section names and
  strips every blank outside quotes from names and values (`_parse`), then pads `%-32s` and adds
  `, ` after commas (`_decorate`): `gz_scene::ltx::IniWriter` does the same, so parts write back
  byte for byte. `w_u32` prints `%d` (negative above 2^31); `w_bool` writes `on` / `off`.
- Oracle quirks kept: an LTX sound environment loses `env_inner` / `env_outer` on load; a fog
  volume never saves its shape (its rows are the shape's); way links come back in key order
  (`link_wp_0_10` before `link_wp_0_2`); `object_set_*` on a group moves the group alone
  (the Details rows carry the members).
- Deliberately more forgiving than the C++: a body of a refused version, a scene object whose
  library file is missing, a portal without its sectors or an empty group no longer fail the
  whole load - they are kept (`KeptObject`) or loaded as they are and written back.
- Spawn entities decode with the linked game's configs (`gz_scene::game::GameSettings` over
  `gz-game`); without them a spawn keeps its numbered values / packet raw and writes them back.

From the graphs port (`gz-nq`):
- What a node IS depends on the catalog: the reader orders pins by it, the writer's topological
  order and the outline's `[T]` ask it which kinds are triggers, the box counts its lines. Every
  pure function takes the `Catalog`, and a store command carries the `Arc<Catalog>` it applies with.
  `Catalog::empty()` behaves like the C++ with no catalog loaded (a trigger is `trigger.*`).
- The node box is sized by counts (lines, actions, pins), not by measured text: no text callback
  is needed, the C++ never measured either.
- The catalog reader keeps `rows` names across sections (the C++ never cleared them) - marked
  `// oracle:`.
- The catalog is keyed by the stamps of the project's `configs\nq\kinds` files and the module id,
  not by the tree revision: a new catalog bumps its generation, which re-parses every graph.
- `quest_edit` answers `path` as the absolute file (forward slashes), as the C++ did; the other
  tab commands answer project-relative. Positions and bounds are JSON floats (`-112.0`).
- `quest_history` keeps the C++ view history and adds `undo_steps` / `redo_steps`, the store's
  labels. Undo back to the saved state is clean again (the C++ stayed dirty).
- Not ported: the on-disk picker cache (`nq_index_*.txt`) - the pickers are in-memory `Query`s; the
  rescue save of dirty graphs on project close (`NqDocs::SaveDirty`).

From the Mod Settings and build dialogs port:
- UI builds call `gz_build::Builds::start` / `gz_release::Releases::start` themselves: `mod_export`
  and `mod_package` are the MCP path (always clean, never remembering the target, never asking).
- Nobody else fed the Task Center's card from `Builds::status` / `Releases::status`:
  `gz_ui_mod::cards` does, for MCP jobs too (`progress` shows their steps).
- The Mod menu rows of other packages (Characters, World Rules, Screens & Music, PDA Interface,
  Bake Scene Layer, Publish Release, Releases, Submit) are theirs to add at
  `gz_ui_mod::mod_menu_slot(id)` with a `UiCommand` of that id; until then they are not shown.
- `mod_set_manifest` without `target` reads it off the mode (the C++ too): Mod Settings always
  states `target` when it is default/existing/new; a classic `target = any` module turns into
  `existing` (mode `*` kept) on its first save from the tab.

From the characters and world port:
- Deviations from the C++ wire: `characters_write` refuses a path outside the project (the C++
  wrote wherever an absolute path pointed); a written text without `id` is validated under its
  file's id (the C++ validated an empty id, `E102`); `world_set` writes only the section it
  changed (the C++ `Save` rewrote the whole file); a declared but empty `[start_kit]` is written
  as `perks = true` - the manifest store drops a section that holds nothing, and the game reads
  the same kit.
- `gz_nq`'s slots are sync: the briefs and the game world are cached and refreshed in the
  background when their revision moves; commands await the fresh value themselves. Until the
  first roster the characters lookup answers what the file tree says (as `gz-nq`'s own fallback).
- The scene area fills `gz_world::SceneStarts::of(editor).provide(..)` (its level and Player
  Starts, `SetSceneStarts`); until then `E201` counts the baked `spawn\*.xspawn` files alone.
- No `list_assets` category for characters: `gz_project::CATEGORIES` is closed (as for the PDA).
- The UI half (Characters tab, World Rules tab, the item picker) is not here; the tab binds to
  `gz_characters::text_raw`, edits with `text_set`, and reads `Roster::raw` for what a base gives.

From the level export and build port:
- `{:.6}` of the `f64` a `float` promotes to is UCRT's `%f` (both round exact ties to even);
  `cfmt::g` is `%g` built on `{:e}`.
- Not byte-equal to the C++ by construction: the xcform material order and the build project's
  face order follow the surfaces in file order (the C++ walks a map keyed by surface pointers);
  detail textures and patrol paths are sorted by name (the C++ by `shared_str` address); the
  game graph's equal-weight edge removals, the A* ties and the artefact / death points (C++
  `rand` shuffles) are deterministic here; the editor's cross-table size prefix (`+1`) is written
  as xrAI writes it (`+4`).
- `gz-fmt-xr`'s `GameMaterial::DYNAMIC` is `1 << 3`; the engine's `flDynamic` is `1 << 8`
  (`XrGameMaterialLibraryInterface.h`). `gz-levelbuild` uses its own flags.
- `gz-entities` has no setters for a zone's `m_artefact_position_offset` or a level changer's
  destination: the spawn build leaves both as the scene holds them. The space restrictor
  separator check (Play's option), the breakable / climable objects of `BuildGame` and
  `Scene->Validate` before a build are not ported.

## Next packages, in dependency order

1. `text` - done.
2. `project` (`gz-project`: the list, project files, rules, file tree, content operations).
3. `kit` (the UI kit's widgets, toolbar, dialogs) and `icons`.
4. `game-link` (`gz-vfs`, `gz-game`, `gz-lua`) after 1.
5. `mod-build`, `nq`, `assets` (characters, PDA, screens, world), `release` after 2 and 4.
6. `tex-xr`, `ogf`, `spawn-entities` - needed by the scene and the renderer (phases 4-5), and
   `spawn` by the world rules.
7. The shell and the tabs (phase 3), the scene (4), the renderer (5), the level tools (6).
