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
| `gz-corpus` | done | test support only (dev-dependency of the corpus tests): `GRIZZLY_TEST_GAME` / `GRIZZLY_TEST_PROJECTS`, the game's files read independently of `gz-vfs` (archive directories + loose `gamedata`), `walk`, `stride`, `check_all`, `finish` |
| `gz-win`, `gz-crash` | done | `gz-win` is the only crate with `unsafe`: process counters, minimised start, dumps, foreground, the caption hit test (`install_caption_hit_test`, `CaptionShape` in an `ArcSwap`), known folders, free space, volume roots |
| `gz-mcp-bridge` | done | bin `grizzly-mcp`: MCP over stdio (`initialize`, `tools/list`, `tools/call`) to the editor's TCP endpoint (`GRIZZLY_MCP_PORT`, 28017); tools, descriptions, schemas, annotations and instructions from `gz-mcp/tools.json` word for word; `tool_result` of the Python bridge ported (unknown arguments, fixed-argument tools, waits, images). Tests written, not run |
| `gz-editor`, `gz-mcp` | done | context, registry, `AutoSave`, `fsx`, the endpoint |
| `gz-app`, `grizzly` | done | the shell: caption (menus, fold under 1100 pt, tab strip, window buttons; drag, double-click, snap and the 6 pt resize edges are Windows' own through `gz-win`'s `WM_NCHITTEST` hook), toolbar, status bar, work area, drawers, Task Center card, palette, keys, session, settings file `grizzly_editor.ltx`; `ShellBuilder` + `Document` for areas (`install(b)` lines go into `grizzly/src/main.rs` `install_ui`); `UiQueue`/`on_ui`; 24 commands (`state`, `ui_*`, `window_control`, `titlebar_info`, `reset_layout`, `theme`, `preferences`, `shortcuts`, `command_palette`, `progress`, `screenshot_editor`, `list_commands`, `exec_command`); the tabs' own unsaved work as a `DirtySource` (`unsaved.rs`). `busy` = a job of a C++ blocking kind; the C++ command table and macros ("The shell's rest"). Tests written, not run |
| `gz-chunk` | done | reader, writer, `lzhuf` (byte-equal compress), `lzo` (LZO1X + archive index helpers) |
| `gz-ltx` | done | `doc.rs` (lossless); `tree.rs`: `parse_file` (parallel per file) + `Ltx::assemble` / `Builder` (the sequential C++ load), `Dialect::Editor` / `Dialect::Game` (`#include_optional`, `*.ltx` includes, multi-line recovery, XMS layer merge, `.ltxp` patches) |
| `gz-luadata` | done | `parse` = LuaJIT 2.1 lexer/parser port (the editor's fork) + `DeclarativeProblem` + a run of the data subset with the interpreter's semantics; writer byte-equal to `NqLua.cpp` (`write_value`, `write_node`, `write_asset`); `syntax_check`; JSON |
| `gz-fmt-xml` | done | tolerant tree with `CXml` navigation, `preprocess` (`#include`), `strings` (editor scan / game XML, parallel build), `sheets` (`textures_descr`) |
| `gz-fmt-object` | done | `.object`, `.skl/.skls`, `.anm`, envelopes, normals, skinning; see findings |
| `gz-fmt-ogf` | done | every visual type read (FVF and skinned 1-4L decoded to one vertex), bones, IK, motions and `.omf` with key sampling; static and skeleton export, the face optimiser ported (both modes); the VIPM pass of a progressive skeleton (`vipm`: collapses, 5D quadric, sliding windows, `OGF_SWIDATA`); the box fitters (`obb`: Wild Magic's `MinBox3` for bones, RAPID's for parts); the breakable part exporter (`export_breakable`); see findings |
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
| `gz-render` | done | wgpu 30 forward renderer, no window, no scene: `DrawList` -> `build_packet` (parallel cull or the caller's `Visibility`, instanced groups by mesh + material, blended back to front, helpers, labels, nearest 128 lights) -> the render thread (newest packet wins, passes in parallel chunks, frame clock `wait_frame`, `capture`, `Wake`), `render_offscreen`, `ThumbnailCache`; `Assets` (caller's `AssetLoader`, IO + CPU jobs, LRU by bytes, BC1-3 as they are), `MaterialTable` (data) -> 17 editor materials in WGSL (particle add / alpha-add / multiply, wallmark multiply 2x); `SpriteBatch` (camera / axes / along-velocity, world or screen size) in a dynamic buffer, instanced `Batch`es, cube textures and the `Sky`, decal depth bias, the editor's directional-light path (`ViewFlags::SCENE_LIGHTS`); Lit (studio) / Unlit / Wireframe / Game Lighting (`glit_light` ported); id-target outline; label occlusion queries; GPU skinning; `examples/offscreen.rs` rendered and looked at. Tests written, not run |
| `gz-particles` | done | pure port of XrParticles + `CParticleEffect` / `CPEDef` / `CParticleGroup` / `CPSLibrary`: the engine's `CRandom` and the CRT `rand` with MSVC's right-to-left draws, `pDomain`, Perlin `noise3` / `fractalsum3`, every action (`TargetRotateD` / `TargetVelocityD` folded), 33 ms steps clamped to 3, deferred stop, time limit, groups (related / free children by birth and death events, `time0` / `time1`), collision through a `Collider`; `Library` (sorted, effect then group), `System` (play / stop / follow parent / step / sprites / bounds); deterministic per seed. Pinned-step tests written, not run |
| `gz-build` | done | `BuildWalk` from the file tree, `BuildCheck` / `BuildChecks` (+ the bake slot), target refusals, the clean build in `modules\.new\<id>` with hard links and two renames, the flat export + report, `run_build` (for `gz-release`), `Builds` (job `build`, results, Task Center `BuildStatus`, `CleanQuestion`); `mod_export` |
| `gz-github` | done | the network side: `reqwest` on native-tls (Windows certificate store) + system proxy, three clients (API and uploads never follow a redirect, public files do), every request raced against the job's `Cancel`; `Secret` (zeroed, `Debug` redacted); `Endpoints` (real or `-qa_github` mock, `upload_url_trusted`, `vt_url_trusted`, the catalog's repo and mirrors); account (env -> Credential Manager -> `gh auth token`, a minute's cache), device flow; `publish` (repo, foreign latest, published tag / version, draft reuse, leftovers, uploads 3 at a time with the descriptor last, stored assets checked, latest, the served check); list / edit (`make_latest` where the mark is) / delete (release, then tag); `fetch_public`; catalog issues (`find_issue`, bot-only newest verdict, submit / withdraw / issue); VirusTotal (pace slots, policy, verdict, lookup, upload with the upload address check) and the scan (`lookup`, `scan`, `ORDER`, `describe`, `json`); ini helpers of the catalog files. Tests written, not run |
| `gz-publish` | done | `Publisher` (job `publish`: packaging through `gz_release::run_package` in group `build`, then the scan of a listed module + the upload in group `publish`; `ScanFirst` word for word; Task Center cards `key` `publish`), `FreshTag` / `deleted.tags`, device sign-in, `ReleaseList` (list, edit, delete, select, `show=edit` as a request the UI takes); `mod_publish` (answers when the packaging ended, as the C++ `FollowsJob`), `mod_releases`. Tests written, not run |
| `gz-catalog` | done | `Catalog` (one job at a time, group `catalog`: `check` = section 7 with the content rules over the local copy's zips, `vt`, `submit`, `submission`, `withdraw`), `Release` read as the game reads it + signature, `read_listing`, `vt_level` (never optional), the submission text / details, `website_is_module_page`; the real `mod_catalog` (key actions through `gz_release::key_action`). Tests written, not run |
| `gz-scene` | done | `SceneState` in one store (objects `OrdMap`, per-class order, names, selection, `ToolSettings`, `BulkData` with the AI map in 4096-node tiles), `SceneCmd` (place, delete, rename, transform batches with the group rules, select, show, tool eyes / read-only, group / ungroup, property replaces checked by `serial`, mod marks, order, snap list, bulk, saved, touch); all 16 classes read / written in LTX and stream (`objects`), the main file + parts (`io`: LTX, binary `.level`, undo / recovery stream, unknown sections / chunks / refused versions kept); `props` (rows, merge, text set, labels / units / help); library, pick tree (`Derived` by scene + library rev), drop / snap, recovery (`backup\recovery`, the C++ manifest); 34 commands, `list_assets` Objects / Groups |
| `gz-export-level` | done | The module's level layer of `EditorModScene.cpp`, pure over a scene snapshot + gathered disk state (`gather`): `.xspawn` (`op = add/start/remove`, `[path:]`, custom data with `[xms_order]`, fixed bones checked against the OGF, smart cover, lamp, zone shapes, every C++ warning, merge or mirror), `overlay.xcform`, cuts (+ `[cut_N]`), world-space `.ogf` overlay (parallel) + `[v_N]` registry; `%f`/`%g` exactly as UCRT (`cfmt`); `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe`; `install` fills `gz-build`'s bake (the mirror pass + its note), `gz-world`'s `SceneStarts` (E201 is `gz-world`'s) and `gz-nq`'s `SceneLookup` (restrictors, story ids, way paths, level). Tests written, not run |
| `gz-levelbuild` | done | The full level build: `build.prj` (static + MU objects, LODs merged into `level_lods*.dds`, hemi / sun / point lights, glows, portals, sectors, wallmark faces), `level.ltx`, `BuildGame` files, HOM/SOM, `build.aimap`, `build.details` + `build_details.dds`, `level.wallmarks`, scene stat; CForm; level graph, game graph + cross table (`CGameGraphBuilder`), multi-level merge (`CGraphMerger`); `editor.spawn` (spawn constructor, patrols, artefact points, graph embedded); `nav` = the engine's `ILevelGraph` lookups + A*; XrLC / xrDO / xrAI / the game as `tokio::process` jobs (`level_run`, group `game`, Stop = `taskkill /T`); 19 commands named as `gz-ui-level` expects + `level_rebuild_status` + `level_build action=`; the breakable / climable objects of `BuildGame` (`breakable`); the whole `Scene->Validate` (`validate`). Tests written, not run |
| `gz-lc-light` | done | phase 8a: `XrLCLight` + `xrDO_Light` - the compiler's model (mesh, deflectors, lightmaps, MU models, `xrLC_GlobalData` as `LcGlobalData`), the ray model, the light of lightmaps, vertices, implicit textures, MU models and detail slots; see "Phase 8a" below. 38 tests, run and green |
| `gz-lua` | done | LuaJIT 2.1 vendored (`mlua` 0.12, JIT off, count hook = instruction budget); `LuaHost` pool of states (one per caller, scripts kept per revision), `Script`, `Arg::{Value, Chunk}` (a text run in an empty env: the asset is the table LuaJIT builds), `call_omitting`; sandbox = base + string/table/math, the C++ reader's 18 globals; `vanilla_source` rewrites the fork's `//` `/* */` comments; tests written, not run |
| `gz-pda` | done | `evaluate` (gz-luadata check, then the game's `normalize`), `Layout`/`Item`/... read as the C++ read the stack, `preview` block, `template`; `Preview` state machine (pages, vars, log, toasts, rows, scroll, keys, map zoom, boxes / hit test / `visible` in canvas units); `PdaService` (reader and layout `Query`s, W320 textures, list, `[pda] layout`); the build check; `content_create kind=pda` template; `pda_list`, `pda_get`, `pda_write`, `pda_delete`, `pda_use`, `pda_validate`; tests written, not run |
| `gz-ui-app` | done | the Start page (`ShellBuilder::start_page`: New Project form, Open Folder via `rfd` on a thread of its own, search, Last Opened / Name sort, 232x184 cards with twin parents, previews decoded on the pools through a `Query` keyed by path + write time, `ShowsNoScene`, card menu, `dlg.delete_project` counted off the UI thread), Link Game page card, the Output Log (one body for the drawer and the Level panel `output_log`, cursor over `gz_log::Log`, distinct counting per window from "Project opened:", the `log_state` hook), Preferences (`preferences` tab: 44 rows of `EditorPreferences`/`LevelPreferences`, nav, search, Keyboard page over `shortcuts` with an Assign dialog `dlg.assign_shortcut`); command `project_browser_menu`. Every row is live (`object_flags` and `recovery_*` are in `gz-app`'s `PREFS`, not on the `preferences` wire). Tests written, not run |
| `gz-nq` | done | quests and behaviour graphs: `asset` (typed `Graph` with `imbl` node/task vectors, read with the C++ shape messages, `write` byte-equal to `NqLua::Write`, outline, topo order, templates), `catalog` (hand-rolled reader, fold order game|bundled -> modules -> project, `Catalog::is_trigger`, exclusive forms, JSON), `layout` (`node_box`, the one rectangle; the dominator-tree auto layout on bitsets), `validate` (every code and message of `NqValidate`), `references` (typed walker, `find`, `rename_task`), `index` (snapshot, fingerprint, search), `pickers` (15 types from the game's ltx tree / strings / XML / `all.spawn`, the project's custom data, `spawn.part`, looks; `compose` with the scene), `caps` (`class_registrator.script` families), `doc` (`Store<GraphDoc>`: every `quest_apply` op, canvas ops, view state with bookmarks / history / find), `NqService` (queries keyed by game rev, file stamps, manifest, docs' edit serials; slots `CharacterLookup` / `SceneLookup` / `WorldLookup`), the build check (`E113` across graphs); 24 `quest_*` commands (all but `quest_view`), templates, rename hook, `DirtySource`. Tests written, not run |
| `gz-screens` | done | `Screens` model parsed off `ManifestDoc`, written back with `AppendScreens`' rows (sections in place while their order holds), `import` (+ `plan`: the decision by place and extension; parallel-safe copies), `validate` (E401..W411), the build check, `GameLevels` slot for W407; `screens_get`, `screens_set`; tests written, not run |
| `gz-characters` | done | `model` (`Character` keeping unset unset, `parse` / `load_text` / `write` as `EditorCharacters::Parse` / `Write`, `{ rus = .. }` alone collapsed on write, `text_in` / `text_raw` / `text_set`, `template`, `game_id`), `resolve` (`E111` / `E112`, kits merged, a shop whole, defaults once), `validate` (every C++ code word for word over `Lookups` = `gz_nq::Pickers` and `Models`; `E123` from `OGF_S_BONE_NAMES` read off the chunk list, cached per model and game rev), `game` (trade presets, bone names, stalker damage bones); `Characters` service: `Roster` as a `Query` keyed by a stamp of the `.npcasset` files + `[exclude]`, read in parallel; fills `gz_nq`'s `CharacterLookup` on project open (briefs refreshed in the background, the file tree's until the first roster); `Checker` (linked game: pickers, presets, models); build check (first error, as `CheckForBuild`); `content_create kind=character` template; `characters_list`, `characters_get`, `characters_write`, `characters_delete`. Tests written, not run |
| `gz-world` | done | `model` (`World` / `StartKit` read off `ManifestDoc`'s raw lines as `EditorMod::Load`, written back as `Edit::SetLines` in the `Save` layout, JSON, `base_kit`), `game` (`read_all_spawn`: `gz-fmt-spawn` container + `SpawnIndex` in parallel, level changers by class `LVL_CHNG`, destination from `gz-entities` with the C++ byte walk as fallback; levels, transitions, patrols sorted), `starts` (`op = start` of `spawn\*.xspawn`), `validate` (`E201`..`W208`); `WorldService` (game world as a `Query` keyed by the game rev, captions from the Level picker, starts with the scene's through the `SceneStarts` slot, problems); fills `gz_nq`'s `WorldLookup` (kicked on first ask), `gz_screens::GameLevels`, `gz_mod::ManifestFields` `world`; build check; `world_get`, `world_set`, `world_transitions`. Tests written, not run |
| `gz-ui-mod` | done | Mod Settings tab (`mod_settings`: nav of 7 pages with problem dots, 44 em form, the C++ `CheckSettings` sentences under fields, saved as edited through `mod_set_manifest` / `mod_commands` / `pda_use` once no field is being typed in; untouched fields follow outside saves); Build to Folder / Export Flat / Package Release dialogs, `dlg.clean_target`, `dlg.replace_release`, the kit `Result` (`build_result`, `export_result`, `package_result`; `ui_dialog open` puts the last one up again to look at), `KeyLine` (the author key row); Task Center cards of every `build` / `package` job (`cards`); Mod menu rows in `MOD_MENU` slots and the Build Mod split; `mod_settings_view`. Looked at on a temp copy. Tests written, not run |
| `gz-ui-content` | done | the Content Browser: one `ContentBrowser` service, two hosts (`content_browser(ui, cx, Host)`: the Content Drawer via `install`, the Level panel `content_browser` - the Level document calls it with `Host::Panel`); sources Project (`FileTree` snapshot), Editor (each category a folder of the data root, walked with `FileTree::scan`), Game (`GameFiles`) as `Catalog`s built on the pools, folder views as a `Query` keyed by (source, folder, search) + revisions; pictures: `texture_tile` on the pools for the size drawn, models through a renderer of the crate's own + `ThumbnailCache`, textures released 3 frames late; `caption` (pure `DrawTileCaption`); every action an editor command; `DragPayload`; `content_browser_navigation` / `_open` / `_selection` / `_copy`. Looked at: tiles 48/96/160/192, list, textures, models, rename box, menus. Tests written, not run |
| `gz-ui-world` | done | the Characters tab (`characters`: list 260 pt with search, dirty dots, Template badges, splitter, form up to 760 pt bound to the RAW values, base's values as hints / muted kit rows, Ctrl+S = `characters_write` with `gz_characters::write`, the file follows the id via `content_rename`, `dlg.delete_character` -> `characters_delete`, unsaved prompt before any switch, problems resolved + validated on the pools) and the World Rules tab (`world_rules`: every edit a `world_set`, sent in order; Original Game / Player Start / Starting Kit, More with Applies On / Playable Area / Transitions; problems bar + drawer; `dlg.empty_world`); `Lists` (pickers, trade categories, `meshes\actors` models, read on the pools, busy until read); portrait sheets decoded on the pools, kept per project; `item_picker` (both tabs); Mod menu rows; `characters_view`, `world_view`. Looked at on a temp copy. Tests written, not run |
| `gz-ui-release` | done | Publish Release (`publish_release`, L: one Release line + Notes, the rest under More unless it stops the release; reads the releases as it opens, a version that is out is a problem with Open Mod Settings / Open Releases, a failed read a muted note; account rows, device code in Title Mono, token field; Publish closes it, the card follows, the kit `result` `dlg.publish_result`), `dlg.edit_release` (also from `take_edit_request`), the Releases tab (`releases`: sortable table, details with Edit... / Open Page / Delete... GhostDanger, row menu, Del / F5, the kit `confirm` `dlg.delete_release`, banners), Submit to Mod Browser (`submit`, fixed 600 pt body, five cards, VirusTotal card in the Required tone, footer busy + Stop <job> + Hide, label sets) with the sheet `dlg.catalog_withdraw`, follows `Catalog::window_open` / `wants_withdraw_question`; toolbar Publish Release / Releases, Mod rows at `gz_ui_mod::mod_menu_slot`; the key row is `gz_ui_mod::KeyLine`. Every button runs the domain path its command runs (`Publisher::start`, `ReleaseList::start`, `Catalog::start`, `AuthorKeys`, `Github` token / VT key). Locals: `local_secret_field`, `local_secret_row`, `local_code`, `local_card_title`, `local_fold`, `local_table_head`. Looked at under `-qa_github` with dead loopback endpoints; tests written, not run |
| `gz-ui-pda` | done | the `pda` tab (singleton view over a session `PdaTab` model behind one lock: file, Source text, the reader's answer via `PdaService::evaluate`, `gz_pda::Preview`, view, last frame's geometry; retargets through the shell's unsaved prompt, follows the file on the tree revision and before every `pda_preview`), the canvas (device of `pda.xml` / `pda_16.xml`, sheets / fonts / level maps from `PdaAssets` - decoded on the pools, uploaded once, dropped with the session, the link or Reload; bitmap glyph quads; Fit / 100% / wheel-at-mouse / pan in px semantics; screen edges; stock panel, toasts, map with actor and markers), Details tabs, Source pane (Mono, gutter, error band, Apply / Ctrl+Enter / Ctrl+S); `pda_preview` (canvas-unit state, `map.bound` / `spots`, waits for 2 frames when owed); the `screens` tab (lists, 16:9 preview with DDS / GIF thumbnails, layers with drag reorder, music combo, More, track form; edits as `Change`s applied locally and saved through `ManifestService::edit` on the freshest model); `local_file_drop_zone` (Explorer drops wait for the pointer's next move, `gz_ui_content::DragPayload` and std path payloads, inner zones first, `simulate_drop`); `screens_view` (waits for the drop's import and save); `COMMAND_MOD_SCREENS` / `COMMAND_MOD_PDA` at `gz_ui_mod::mod_menu_slot`. Looked at on a temp project: 4:3, 16:9, Source with an error, a two-layer screen. Tests written, not run |
| `gz-ui-previews` | done | the `model` tab (own `gz_render::Renderer` sized to the canvas, a hand-built `DrawList`: the model posed by `idle` / its first cycle, a floor of round metre steps under it, bones as overlays; orbit / pan / zoom-at-cursor `Orbit`; packets built and submitted on the CPU pool only when the view changed or went stale; Details grid), the `texture` tab (read + uploaded on the pools: native DDS as the GPU takes it - BC1-7, 32-bit, R8, RGBA16F, every whole level - else decoded to B8G8R8A8; Fit / 1:1 / Zoom / channels / Mip / Details), both drawn through `image_pass` (a WGSL pass in egui's render pass: straight alpha, channel and alpha-as-grey views, opaque renders, nearest past 2:1); `loader::Sources` = project, editor, linked game (also the renderer's `AssetLoader`); Light Animations (`Store` with undo over `lanims.xr`, timeline, key bar, Play), Texture Library + New Textures (`.tga`/`.thm` walk as a `Query`, `Store` of edited descriptions, save/import as a job writing `.thm`, `.dds`, `.tga`; Types filter; texture picker), Sound Library (`.wav`/`.thm` walk, waveform from `Pcm::peaks`, `cpal` audio thread, save writes `.thm` and makes the `.ogg` again from the `.wav`, Rebuild, Synchronize encodes what is missing or of another age); Libraries menu incl. Check New Textures, Synchronize Textures... / Sounds... (confirm, then jobs); `preview_model`, `texture_preview`, `library`, `asset_preview`. Cube Map \ Smaller Copy, Reload Objects / Textures, Rewrite Texture Descriptions ("The shell's rest"). `gz-app/src/window.rs`: BC feature requested when the adapter has it, device errors logged. Looked at on a temp project. Tests written, not run |
| `gz-ui-level` | done | the Level document (kind `level`): a dock of its own over the kit's splitters (`layout`: Wide / Compact builds, four slots, open / slot / front per panel and the sizes in `level.*` layout keys, custom = placement or 8 pt-rounded sizes differ from the build; `dock`: side slots keep pt, viewport min 360 x 240, splitters last; 26 pt tab strips with cross on hover and a Move To / Close menu; F11), the viewport pane in the centre; Level toolbar over registry commands (`toolbar::LEVEL_ACTIONS` - `build_cform`, `build_aimap`, `build_game_graph`, `build_all`, `play`, `play_game`, `play_clean`, `play_stop`, `reload_configs` - added only when registered), status centre + Rebuild chip over `level_rebuild_status` (polled, cached); World Outliner (`outliner_model::compute` as a `Query` by scene rev + filter, Unreal search grammar, natural sort, By Hierarchy, virtualised rows, Filters badge, More, Type / Owner columns, `dlg.rename_object`, `dlg.delete_object`); Tools (tiles, sticky header, Object / Spawn / Particles lists from `lists::Lists` read on the pools, Group, Detail Objects + `dlg.detail_objects`, Select by Reference, Random Settings `dlg.properties`, Snap Objects; picks go to `gz_ui_viewport::set_place_item`); Details / Level Settings (rows prepared as a `Query`, categories, kit grid, transform drags merged + sealed, other values sent on release, `dlg.choose` over the lists, `dlg.edit_text`, Quality segmented); Scene / Build / View menus and 11 scene `UiCommand`s; Recovery page card; `dlg.import_base_scene`; `preview.png` on save and a minute after a change; `outliner_show`, `outliner_filter`, `scene_visibility`, `property_inspector` (wraps gz-scene's, adds the panels). Tests written, not run |
| `gz-ui-viewport` | done | the Level tab's 3D view: `ViewportPane` (`new`/`ui`/`inactive`/`shortcut`/`mcp_state`, plus `set_maximized`/`take_maximize_request`), `Viewport` service (camera `Nav` = `CUI_Camera` + `ViewportNavigation`, view flags, tool and snaps, Game Lighting, render size), pure `draw::build` (scene snapshot -> `DrawList` with every class's helpers in the C++ colours) + `draw::visibility` from the scene's pick tree, packets built on the CPU pool only when scene/library/view/assets change, egui overlays (pills, 64 pt axis gizmo, transform gizmo, Mono stats pill, labels, safe rect, marquee), picking/select/place/move/rotate/scale as `SceneCmd`s (one merged undo step per drag), Game Lighting (`lighting`: weathers, user.ltx, lamps/campfires/level lights, roof map), the asset loader (project, editor, game), the camera per scene in `<project>\temp\viewport.ltx`, `capture_preview`, `set_place_item`, `view_menu`, `install` (Keyboard page commands); the extras built in the packet job (`draw_extra`: glows, `draw_particles`, `draw_aimap`, `draw_details`, `draw_wallmarks`, caches as services); 10 commands (`camera_get`, `camera_set`, `focus_object`, `viewport_navigation`, `viewport_controls`, `view_mode`, `game_lighting`, `frame_pacing`, `screenshot_viewport`, `particles_tool`). Looked at in the running editor; tests written, not run |
| `gz-ui-graph` | done | the graph tab (kinds `quest` / `beh`, ids `quest:<rel>` / `beh:<rel>`): `Scene` built per revision on the CPU pool from `gz_nq::layout` (texts, pins, beziers + back edges routed like `RouteBack`, 512 pt grid index, extents, sorted edges for `beyond`, the minimap's mesh) - a frame draws only the cells in view; `Canvas` (10 zoom levels, LOD by title px, pan / wheel / drag / link / marquee / action drag, edge pills, minimap, context menus, kind pickers by the document's vocabulary); a `ViewMirror` of the store's view, posted when a gesture ends and taken back when the document moved it; `Inspector` (Graph panel: header, Tasks or Roles, Variables, Global -> `mod.ltx [vars]`; Details: node, action under `graph.action_split`, help footer) staging a copy committed as `set_node` / `set_quest` (Replace only for roles and a variable rename); typed editors of `DrawTyped` in `params`; dialogs (Find in Project, Rename Task, Task References via their commands, Edit Text / Lua); `quest_view` (C++ wire, held for its frames), `quest_open` brings the tab up. The camera button of a position reads `gz_editor::ViewCamera`. Tests written, not run |

## Findings worth keeping

From the object/mesh port:
- Both `CalculateTB` bodies of the OGF exporters are commented out in the C++: no NVMeshMender
  port is needed.
- The skeleton exporter never welds vertices (every split is a triangle soup): VIPM, ported
  whole, collapses nothing on such a split (no edge has two triangles) and comes out as a fixed
  reordering - vertices last first, faces newest first through the cheap optimiser, one window.
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
- A build into the linked game refreshes it (`gz-build`, after the module is in place).
- `game_link` / `game_modes` refusals carry `error` before their fields (the shared error
  reply); the C++ `game_link` refusal had no `error` key at all.

From the OGF port:
- Verified: both `CalculateTB` bodies are commented out (skinned vertices carry the
  constructor's tangent (1,0,0) and binormal (0,0,1)); neither exporter welds; the static
  exporter's `MakeProgressive` returns at once. VIPM runs only for a skeleton flagged
  progressive, instead of the face optimiser (`gz-fmt-ogf/src/vipm.rs`: `PropSlimTools.cpp`,
  `object.cpp`, `object_sliding.cpp`, `mesh.h`; the error is QSlim's 5D quadric in double).
  Kept as the C++ has them: `ArbitraryList::resize` keeps what lies past the size,
  `insert` overwrites, lists empty by "copy the last over it", the triangle list takes new ones
  at its head, points are walked last first. An assertion of the C++ is a "Can't make
  progressive." warning here (the split stays unoptimised, as when the C++ gets NULL).
- The skeleton's face order comes from XrETools' lookahead optimiser (`D3DX_OptimizeFaces`
  over `_OptimiseVertexCoherencyTriList`, mode 2; the VIPM pass asks for mode 1, lookahead 1
  and cutoff 4): ported float for float in `gz-fmt-ogf/src/optimize.rs`.
- Bone OBBs: `ComputeOBB_WML` = Wild Magic's `MinBox3` (`gz-fmt-ogf/src/obb.rs`: the 4x4x4
  angle grid, `MinimizeN` over `Minimize1` with the C++'s direction-set bug - the cycling never
  refreshes the last pointer, so from the second pass one direction is used twice -, `Cos` /
  `Sin` / `Sqrt` as the double functions rounded to float). `ComputeOBB_RAPID` (moments,
  `Fmatrix33::Meigen`) boxes a breakable part. Not equal where the C++ reads memory it never
  wrote: a box whose volume is not below `FLT_MAX` (the C++ keeps an uninitialised `Fobb`),
  Jacobi without convergence after 50 sweeps (the C++ leaves the axes unwritten).
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
- Documents is the shell's known folder (`gz_win::known_folder`), `%USERPROFILE%\Documents` as the fallback.
- While the module stages, the cards (package and publish) show `gz_release::Releases::status`, which overlays the build's row.

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
- Drawn now: particle systems (`gz-particles` stepped on the CPU pool, `SpriteBatch`es built into
  one dynamic buffer per packet; the viewport asks for frames while one in view plays), glows
  (screen or fixed size, the C++ visibility test), the AI map (tile meshes per 16 m cell near the
  camera, rebuilt per tile revision), detail objects (the game's decompression from
  `build.details`, instanced `Batch`es, slot boxes), wallmarks (`level.wallmarks`, decal depth
  bias), the cube-map sky under Game Lighting (both weather skies blended, `sky_color` and
  `sky_rotation`); the studio light path takes the scene's directional lights with `use_in_d3d`
  on (at most 4, the sun with Show Sun) as the C++ DX11 editor's fixed-function emulation does -
  none enabled draws unlit, as there.
- Not drawn: the `Distort` particle material (the editor has no distortion pass either);
  particles with a mesh form (Dead Air has none). Details and wallmarks were not seen in a
  picture: the test scene has neither.
- `gz-fmt-xr` refuses Dead Air's `particles.xr` (50 effects carry the SoC 20-byte
  `TargetColor`; first: `a_ufp_particles\group_ufp\atom_flame`, "the data ends 4 bytes early at
  offset 516"); the viewport then plays the editor's bundled `particles.xr` and logs a note.
  The reader needs the short form.
- The C++ editor never draws `gcontainer` OGFs: `FVisual.cpp` loads `OGF_GCONTAINER` only under
  `#ifndef REDITOR` (the editor's `CRender` has no shared vertex / index pools), the thumbnail
  path refuses tree and LOD visuals, and the editor never opens `level.geom`.

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
- `screens_set action=drop` makes screens and tracks for the surface the open Screens tab shows
  (`surface=` wins; loading without the tab), as the C++.

From the Content Browser:
- `content_create` without `folder=` lands in the folder the browser shows (`ProjectHooks::provide_content_folder`, filled by `gz-ui-content`).
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
- Not ported: the on-disk picker cache (`nq_index_*.txt`) - the pickers are in-memory `Query`s.

From the Mod Settings and build dialogs port:
- UI builds call `gz_build::Builds::start` / `gz_release::Releases::start` themselves: `mod_export`
  and `mod_package` are the MCP path (always clean, never remembering the target, never asking).
- Nobody else fed the Task Center's card from `Builds::status` / `Releases::status`:
  `gz_ui_mod::cards` does, for MCP jobs too (`progress` shows their steps).
- The Mod menu rows of other packages (Characters, World Rules, Screens & Music, PDA Interface,
  Bake Scene Layer, Publish Release, Releases, Submit) are theirs to add at
  `gz_ui_mod::mod_menu_slot(id)` with a `UiCommand` of that id; until then they are not shown.
- `mod_set_manifest` without `target` reads it off the mode (the C++ too), except `mode = *`: a classic module keeps `target = any`.

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
- `gz-export-level` fills `gz_world::SceneStarts` (the scene's level and Player Starts).
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
- The breakable and climable objects of `BuildGame` (`gz-levelbuild/src/breakable.rs`, over
  `GeometryPartExtractor.cpp`): ported whole - gathering, parts, RAPID box, neighbours, the
  fragments, the bone offsets, `export_breakable`, the `breakable_object` / `climable_object`
  entities (their chunks open `level.spawn`, before the spawn points, as the Scene Object tool
  exports first). But this editor's `VCPacked::add_vert` no longer welds, so no two faces share a
  vertex: every face is a part of its own without a neighbour and is refused - a C++ build writes
  no such object, only "Error face found at pos: [...]" and "Can't export invalid part #N" per
  face, and so does the port (`// oracle:`). Past that point not byte-equal by construction: the
  fragment sizes (`Random.randI`: the C++ global generator, seeded from the CPU clock; here one
  of its own from 1), the IK record's friction and a split's starting count of degenerate faces
  (uninitialised in the C++, 0 here), the part numbering where a mesh has several surfaces (file
  order, the C++ pointer order). The climable's material goes into the packet after
  `gz-entities` wrote it (`set_additional_info` has no setter there): the default string at the
  state's end is replaced and the state's size fixed.
- `Scene->Validate` is whole (`validate.rs`): the tools in class order (scene objects with
  their surfaces, duplicates and full-test LOD textures; lights; sectors, the first failing one
  ending the tool; wallmark shaders), the portal test when there are sectors or portals, HOM,
  counts, names, shaders, the sound library, particles. Left as said there: a visual-mode object
  passes, a particle system's reference is not looked up in a library, a LOD texture is there
  when the linked game or the editor's `gamedata` has it. The duplicate-position test is now the
  engine's `similar(EPS_L)` (it was `abs_diff_eq(0.0001)`). A failing breakable export puts its
  lines into the job's error before "ERROR: Failed to build game.".

## Closed in integration

- `gz-win`: the caption hit test (`WM_NCHITTEST` from a `CaptionShape` the shell publishes each
  frame; `HTMAXBUTTON` with the clicks and the hover handled; resize edges), known folders, free
  space, volume roots. `gz-app` dropped `StartDrag` / `BeginResize`. Looked at: `titlebar_info`
  and the hit test answer client / caption / max button / left edge as they should.
- The build's free-space check (C++ message), the delete guards and the catalog home on known
  folders, "the root of a drive" also for a volume mounted into a folder.
- `gz_ui_viewport::install` called; a build into the linked game refreshes `GameLink`; the publish
  card shows `Releases::status` (step, numbers, file) while packaging.
- `content_create` default folder hook; `ProjectHooks::on_before_close` (awaited, first) - the
  Level tab takes `preview.png` there; the editor closes its project on quit (last scene,
  preview, unsaved graphs).
- Saving on request: `DirtySource::save` / `discard`, `Editor::save_all` / `discard_all`;
  quests, the scene, the PDA source, and every tab-only document (characters, libraries) through
  the shell's own source, waited for until clean; used by `project_close save=`,
  `window_control close save=`, `ui_tab close save=`. The rescue save of graphs on close
  (`NqDocs::SaveDirty`, `.recovered` beside a file changed on disk).
- `game_link` refuses a different game under a live scene; Link Game's Close Project is off then.
- `object_flags`, `recovery_*` in `PREFS` (not on the wire); recovery follows the Preferences and
  the author's input.
- Key owners: the Content Browser and the Level's owning panels keep the scene's bindings away.
- `list_commands`, `exec_command`; `-dump_commands` lists every `cmd` of `tools.json`.
- Kit: dialogs audit under `Title###dlg.<name>`; a dialog measures again when what it holds
  changes by itself; `MenuItem::tip`; a segmented that does not fit is a combo; lists take the
  arrows, Home / End, Page Up / Down and type-ahead.
- A classic module (`mode = *`, `target = any`) stays classic: `mod_set_manifest` infers no
  target from `*` (`// oracle:`), Mod Settings shows Every Mode for it.
- `quest_edit` / `_find` / `_bookmarks` / `_history` / `_minimap` bring the tab up first.
- `tools.json`: `grizzly_jobs`, `grizzly_level_build` (+ `verify_restrictors`), `content_create
  folder`, `characters_view show`, `world_view more`, `project_browser_menu new_project`.
- `UI_DESIGN.md`: the Characters sentence. Sound encoding (`vorbis_rs`, `gz_fmt_sound::
  encode_game_sound`), noted in `RUST_MIGRATION.md` sections 6 and 8.
- Level build: `GameMaterial::DYNAMIC` is bit 8 (and `gz-fmt-ogf` checks bone / surface
  materials when given them); zone artefact offsets and level changer destinations set by the
  spawn build; `Scene->Validate` whole; the breakable / climable objects; Play's Verify Space
  Restrictors.
- Clippy clean over the workspace, `cargo fmt --all`, `unsafe` in `gz-win` only.

Still open from integration: `gz_ltx::LtxDoc` and `ManifestDoc` stay two documents (entries without `=`, raw line edits and
headers without inheritance would all have to enter `LtxDoc`); the Start page's status bar shows no centre now ("The shell's rest").

## The object tools

The Sector, Portal, Way, Shape, Light and Fog Volume tools (`gz_scene::objtools`, the forms in
`gz-ui-level/src/tools_objects.rs`, the gestures in `gz-ui-viewport/src/objtools.rs`).

- Commands (each `action=`, `show=true` makes the tool current): `sector_tool` (state, set,
  validate, create_default, remove_default, capture_volume, distribute, add_mesh, remove_mesh,
  add_meshes, remove_meshes, new_from_mesh, new_from_object), `portal_tool` (state, compute_all,
  compute_selected, remove_duplicates, invert), `way_tool` (state, mode, auto_link, the six link
  edits, add_point, select_points, move_point, remove_points), `shape_tool` (state, set, add,
  attach, detach, edit_level_bound, recalculate), `light_tool` (state, enable/disable
  selected/all), `fog_tool` (state, group, ungroup). Tools `grizzly_<name>` in `tools.json`.
- Every change is `SceneCmd::ObjTool`: an `objtools::Batch` computed from a snapshot (refused when
  the serial moved, computed again by `commit`, four times at most) or a way's point edit; one undo
  step. Portals, Capture Volume, Validate, Create Default and the level bound run as jobs of the
  Task Center (`commit_job`, group `scene_tool`). The panels' switches are `ToolSettings::obj`
  (`ObjToolState`), kept across Undo, not saved.
- Deviations from the C++: what the C++ could not undo (Compute portals, Remove Duplicates,
  Remove Default, Attach, Detach, the light and fog edits) is one undo step and its confirmation
  says so. Portal edges are sorted by sector order and vertices (the x64 C++ `memcmp` sorted by
  the sectors' pointers only, so duplicates could survive); the orientation vote draws from a
  `CRandom` of its own per portal (the C++ shares the global `Random`); `computed` counts the
  edge loops as the C++ does, `created` what was kept. Distribute Objects changes nothing (its C++
  body is commented out): the command answers so, the button is off. In Points mode a marquee
  selects the points in the box (`CWayObject::FrustumSelect`); the C++ tool called `Select`, which
  selected every point of a picked way. `m_SphereMode` was never initialised: a click adds a box
  until the switch says Sphere. Viewport Lighting edits each light's `use_in_d3d`; the picture
  does not follow it, since the viewport lights with the studio light or Game Lighting.
- Not ported: removing a deleted scene object's meshes from sectors (`OnObjectRemove`) - not a
  control of these panels. `gz_scene::math`'s `EPS_L` / `EPS_S` (0.0001 / 0.000001) are not the
  engine's (0.001 / 0.0000001); the object tools use their own (`objtools::geom`).

## The shell's rest

Brief `briefs/shell-rest.md`. What the shell, the menus and the libraries lacked.

- **The C++ command table** (`gz-app/src/cpp.rs`): `list_commands` / `exec_command` answer the
  C++ rows under their C++ names and enum ids (`id` = the C++ value: `COMMAND_UNDO` is 47,
  `COMMAND_GIZMO_MODE` 109), `menu` = the C++ `Desc()` (left out when empty), `presets` with
  `desc`, `p1`, `p2` and (added) `keys`, and `keys` per row. `exec_command` reads `p1s`/`p1i`/
  `p2s`/`p2i`, picks the sub-command by its preset (a text also by its name), answers `result`
  as the C++ (`1`/`0` or a text) and adds `refused` with the reason. The slot is
  `ShellBuilder::cpp_command(CppCommand)`: C++ id, name, description, `editable`, sub-commands
  with default keys, and an action - `CppAction::Ui` (the `UiCommand`s already registered under
  `NAME` / `NAME."desc"`), `Editor` (a registry command with arguments of p1/p2), `Shell` (a UI
  function with p1/p2) or `Task` (off the UI thread). Non-`Ui` rows get faces (`UiCommand`s;
  bindable and in the palette when `editable`, as `REGISTER_CMD_SE`). A `Ui` row whose commands
  are not registered is left out. Rust-only commands are listed from id 123
  (`COMMAND_UNHIDE_ALL` 123, `COMMAND_MOD_SCREENS` 124, `COMMAND_MOD_BUILD_TO_FOLDER` 125,
  `COMMAND_MOD_EXPORT_FLAT` 126, `COMMAND_REWRITE_THUMBNAILS` 127, `COMMAND_MOD_BAKE_SPAWN` /
  `_XCFORM` / `_OGF` / `_CUT` 128..131); any other unnamed `UiCommand` gets an id from 200.
- **Left out of the table for the scene package to register** (their functions do not exist
  yet): `COMMAND_CLEAR` (39), `COMMAND_LOAD` without a path (the dialog; with `p1s` it runs
  `open_scene`), `COMMAND_SAVE."Save As"` (41, p2 = 1), `COMMAND_TOGGLE_AIMAP_VISIBILITY` (44),
  `COMMAND_CHANGE_TARGET` (57, 18 subs), `ENABLE_TARGET` (58), `READONLY_TARGET` (59),
  `SHOW_TARGET` (60), `MULTI_RENAME_OBJECTS` (62), `CLEAR_DEBUG_DRAW` (69),
  `IMPORT_COMPILER_ERROR` (70), `EXPORT_COMPILER_ERROR` (71), `VALIDATE_SCENE` (72), `CUT` /
  `COPY` / `PASTE` (74-76), `LOAD_SELECTION` / `SAVE_SELECTION` (77, 78), `LOAD_LEVEL_PART` /
  `UNLOAD_LEVEL_PART` (79, 80), `CLEAR/COLLECT/SHOW/EXPORT_SCENE_SUMMARY` (81-84), `BUILD`,
  `MAKE_GAME`, `MAKE_DETAILS`, `MAKE_HOM`, `MAKE_SOM`, `MAKE_AIMAP` (87-92: they need
  `ConfirmLevelBuild` first - `make_*` exist but run unasked), `SET/ADD_SEL/DEL_SEL/CLEAR/
  SELECT_SNAP_OBJECTS` (100-104), `LOAD_FIRSTRECENT` (106), `CB_PLACE_ASSET` (110).
- **Macros** (`gz-app/src/macros.rs`): `COMMAND_EXECUTE_COMMAND_LIST` (50), `LOG_COMMANDS` (51,
  `!p1` as the C++; the log line is the C++ form whose `Msg` was commented out), `RUN_MACRO`
  (52, Slot #1..#8, Alt+1..Alt+8), `ASSIGN_MACRO` (53, p1 slot, p2 file or the open dialog). The
  C++ file format; the slot's file in `[macros] COMMAND_RUN_MACRO."Slot #N"`, relative to
  `<root>\import` when it lies there. Each line runs through `exec_command`'s path off the UI
  thread; the first refusal stops the list ("Can't find command: '...'", "Can't execute
  command: '...' - why"); nesting stops at 8.
- **The window**: `-uiscale <f>` (0.5..4, applied again when the monitor's scale changes); the
  Start page's status bar has no centre; `screenshot_editor` `busy` = a job of the kinds the C++
  ran blocking (`level_build`, `scene_load`, `scene_tool`, `sync_textures`, `sync_sounds`,
  `texture_library`, `sound_library`), `outside` = the titles of the panels' own windows.
- **Panels outside the window**: with `[preferences] floating_panels` on at start the window
  runs egui's immediate viewports as native windows (`gz-app/src/window.rs`: one painter, a
  surface per window, input routed by window id, a window made after the pass that first asked
  for it - that pass paints nothing, its textures are kept - and dropped when no pass asks).
  A Level panel floats from its tab's Move To > Own Window, from a tab let go outside the window,
  or `ui_panel floating=true` (`false` docks it); its window's close docks it; its place is
  `level.float.<panel>[.x/.y/.w/.h]`. Floating panels draw while Level is behind another tab too.
  With the preference off the layout loads every panel docked. Kit dialogs stay in the main
  window (they are drawn by the root pass). Touched `gz-ui-level/src/doc.rs` (the brief named
  dock.rs and layout.rs only: drawing a panel and `PanelOp::Float` / `Dock` live in the
  document).
- **View > Render**: GPU (adapters by name from `RenderState::available_adapters`, System
  Default, `[render] gpu_adapter` used by `native_adapter_selector` when the device is made; the
  driver hint as the C++ writes it, `HKCU\...\DirectX\UserGpuPreferences`, through `reg.exe` on
  the IO pool); Quality 25/50/100/200 % (a session's choice as in the C++; the render size
  scaled, the picture stretched over the pane - `pane.rs` touched for that and for the labels);
  Fill Mode, Shade Mode, Edged Faces, Linear Filtering, Textures; Fog, Environment (a switch,
  as `view_mode` has it), Mute Sounds (`[render] mute_sounds`, the Sound Library's player obeys
  it, `COMMAND_MUTE_SOUND` (38) mutes for a while), Real Time (`[render] real_time`: a pass and a
  frame every frame at the frame rate limit). `view_mode` takes and answers `quality`,
  `mute_sounds`, `real_time`, `gpu`, `driver_hint` (`cmd.rs` touched for that). The renderer
  does not apply shade, edged faces, linear filtering, textures, fog or environment yet - the
  flags are kept, as before.
- **Libraries**: Reload Objects (73, `Library::clear` + the scene's objects read again), Reload
  Textures (10, `gz_app::gpu::TextureReload`: the viewport's renderer gets a new loader, the
  model and texture previews read again; the Content Browser's tiles do not follow it),
  Rewrite Texture Descriptions... (`rewrite_thumbnails`, a job; a file that does not read is
  left as it is and named), Cube Map \ Smaller Copy (`gz_fmt_tex::cubemap::simplify`, a port of
  `ETools_cm.cpp`, parallel per pixel). Refresh Sound Environment Geometry is no row: its C++
  work is commented out.
- **Mod**: Bake Scene Layer > Spawn Layer..., Collision Overlay..., Visual Overlay..., Level
  Cut... run their command at once, as the C++, and answer in `dlg.bake_result` (also through
  `ui_dialog open id=bake_result`). `mod_export_spawn` does not answer `missing` and `removed`:
  the C++ dialog's "Base objects removed" row and the missing-model warning are not shown until
  `gz-export-level` adds them to its answer. The Mod Settings picture waits in
  `temp\thumbnail_pending.dds`; Remove is a mark (Keep Picture undoes it); both go with the
  tab's explicit save (Ctrl+S or the footer's Save, shown only then), the tab is dirty meanwhile.
  `screens_set action=drop`: `surface=` wins, else the open Screens tab's surface (read through
  `screens_view`), else loading; the menu makes menu + pause.
- **Quests**: the position's camera button reads `gz_editor::ViewCamera` (a new slot in
  `gz-editor`), which `gz-ui-viewport`'s service fills with the camera's position and the open
  scene's level; the button shows only once it is filled.
- Seen in the editor on a copy of `Test`: `list_commands` (60 rows), the macros, `view_mode`
  quality 50 stretched over the pane, the View menu with Render folded, a Details panel moved
  out (`outside: ["Details"]`, `host: window`) and back, `dlg.bake_result`, the Start page's
  status bar; `ui_audit` escaped nothing.
- **Found the hard way**: egui keeps the immediate viewports' renderer in a thread-local; the
  closure holds the painter, so the wgpu device was dropped while the process ended and the
  first run with panels outside hung there and, killed, stayed in the GPU driver (pid 11576).
  `App::exiting` now replaces the renderer before the window state goes. While that process
  stands no process of this machine makes a device (`gz-render`'s `offscreen` example hangs
  too): its exe was renamed `Intermediate\cargo\debug\Grizzly.stuck-11576.exe` so builds go
  on; it needs a reboot (or a driver reset). The second look - the stretched 100 % picture, a
  second panel out, the exit with the fix - could not be run.
- Existing, not this package's: `COMMAND_TOGGLE_SAFE_RECT` and `COMMAND_SHOW_OBJECTLIST` both
  defaulted to Ctrl+F; closed in "The scene's rest" (Ctrl+W, as the C++).

## The AI map, details and wallmark tools

`gz_scene::bulk` (`aimap`, `motion`, `collide`, `details`, `wallmark`, `xr`, `cmd`), the forms in
`gz-ui-level/src/tools_bulk.rs`, the gestures in `gz-ui-viewport/src/bulktools.rs`.

- Commands `aimap_tool` (state, set, add_nodes, generate, generate_selected, clear, reset_selected,
  smooth_selected, remove_selected, move_selected, rotate_selected, select_links, select_all,
  deselect_all, invert_selection, select_nodes, pick, select_box, links, remove_links, invert_links,
  ignore_material, unignore_material, clear_ignored), `details_tool` (state, set, first_initialize,
  reinitialize, reinitialize_objects, reinitialize_selected, update_renderer, the selection, pick,
  select_box, add_objects, remove_objects, set_color, remove_color, clear_slots, clear),
  `wallmark_tool` (state, set, add, move, pick, select_box, the selection, select_marks,
  remove_selected); `grizzly_<name>` in `tools.json`. `show=true` makes the tool current.
- Every change is `SceneCmd::Bulk(BulkEdit)`: new parts computed from a snapshot (refused when the
  serial moved; `bulk::commit` computes again, four times). `Kind::Step` is an undo step (a brush
  stroke or a gizmo drag one merged step), `Kind::Selection` moves the serial but is no undo step
  and leaves nothing unsaved, `Kind::Switches` only the panel's switches (`ToolSettings::bulk`:
  Ignore Constraints, Auto Link, the link mode, the ignored materials, the detail slots'
  selection - kept across Undo, not saved).
- `AiMapPart` now holds the tool's nodes exact (`nodes`, `AiMapNode` tiles of 4096) beside the packed
  `tiles`; an edit works on a flat `AiWork` and `AiMapPart::from_nodes` shares every tile equal to the
  old one (packed tiles too while the box and parameters stay). The viewport draws the exact nodes.
  `gz-levelbuild`'s `files::ai_nodes` takes `AiMapPart::all_nodes()` (exact, as the C++ exports
  them) since "The scene's rest".
- `WallmarkPart` gained `model: Option<Arc<Wallmarks>>` (the chunk read and written as
  `LoadStream` / `SaveStream`); every edit writes `raw` again, so the build and the drawing follow.
- The AI Map and Detail Objects tools keep their own snap lists (names in their parts, the C++
  `GetSnapList`): `snap::tool_list` / `apply_snap` work on the current tool's list; the Tools
  panel's Snap Objects section shows it. The wallmarks are laid on the scene's list, used.
- The generation (`AiWork::generate` / `brush`): the C++ queue order kept; the pure part of each
  side (the body's slide, the 81 rays) is worked out ahead for 512 queued nodes on every core, then
  taken in order (`find_node` / registration). A job `aimap_generate` / `details_generate` in group
  `scene`; Stop keeps what was made (as the C++); a scene that moved meanwhile drops the result.
  The collision model (`collide::CollideModel`: collision shaders of `shaders_xrlc.xr`, two-sided
  faces twice, a 2 m XZ grid, queries in model order, the `xrCDB_box` triangle test) is cached per
  session by the snap objects' transforms and the library revision.
- Measured headless on a copy of Escape (625k nodes, 3848 objects): Generate Selected from one seed
  in a 20 x 20 m hole: 11194 nodes in 0.18 s; Generate Full: 1.85M nodes added in 10.7 s (the
  scene's map had been trimmed; its snap list is 3000+ statics); `progress` answered in 16 ms or less
  meanwhile. Details over the terrain (801 x 801 slots): First Initialize 1.0 s.
- Deviations from the C++: Ignored Materials work (the C++ check is commented out; here an ignored
  material's faces are no ground for a node); Invert of the link pad turns a one-way link round
  (the C++ swaps the two link slots and links a node to itself); a ray or box select with Ctrl
  selects (the C++ toggles then sets the flag - kept, `// oracle:`); `MoveSelectedWallmarkTo`
  (disabled in the C++, and looping) is Ctrl+Place and `wallmark_tool move`; Slow Calculate Mode is
  kept with the scene but changes nothing (one collision model path); the detail shuffle takes
  `seed=` or the clock's second once per run (the C++ re-seeds every slot with `time(NULL)`); a
  detail slot whose faces all fall outside is empty (the C++ leaves allocation garbage); tri order
  of box queries is the model's / the library's, not OPCODE's, so ties and the stuck push's float
  sums may differ from the C++ in rare cases; the AI hash follows the box at every edit (the C++
  refreshed it on snap list changes and frames); Update Renderer is the Show Details switch.
- The C++ has no random ranges for wallmarks (the brief named them): none here.
- NOT LOOKED AT: no graphics device could be created while this package was built. Commands were
  driven headless; to be looked at: each panel, Generate Selected and its nodes, link edits,
  details reinitialised and drawn, a wallmark placed and drawn, `ui_audit`, `frame_pacing` during
  a generation. Tests written, not run.

## The scene's rest

Brief `briefs/scene-rest.md`. New in `gz-scene`: `selection.rs` (the selection stream and the
`CF_XRAY_CLASS_LIST` clipboard record), `edit.rs` (cut / copy / paste / duplicate, selection files,
level parts, New Scene), `cmd_edit.rs` (the commands below), `summary.rs`, `debug.rs` (the compiler's
error list), `objmotion.rs` (object motions); in `gz-ui-level`: `level_cmds.rs` (the C++ command
rows, the File menu's rows, the dialogs) and `motion_ui.rs`.

- Commands: `scene_edit`, `selection_file`, `rename_objects`, `level_parts`, `scene_new`,
  `scene_summary`, `compiler_errors`, `scene_validate`, `scene_export_obj`, `object_motion`,
  `object_tool`, `spawn_attach`, `group_tool` (center_pivot, align_pivot, save_as, reload, place),
  `group_thumbnail` (gz-ui-viewport: it needs the viewport's pass); `grizzly_level_build` takes
  `all_levels` and `compile_aimap mode=verify`. All in `tools.json` (191 tools).
- The C++ command table now holds CHANGE_TARGET (the level.ini keys 1-9, 0, Ctrl+1..5, Ctrl+8,
  Ctrl+Q), CLEAR / LOAD / SAVE (p2 = Save As) / LOAD_FIRSTRECENT, CUT / COPY / PASTE, LOAD_SEL /
  SAVE_SEL, MULTI_RENAME_OBJECTS, ENABLE / READONLY / SHOW_TARGET, LOAD / UNLOAD_LEVEL_PART, the
  four SUMMARY rows, CLEAR_DEBUG_DRAW, IMPORT / EXPORT_COMPILER_ERROR, VALIDATE_SCENE,
  TOGGLE_AIMAP_VISIBILITY (`\`), BUILD (F9) and the MAKE_* rows (F5 Make Game) behind
  `ConfirmLevelBuild` (p2 = confirmed). SHOW_OBJECTLIST is Ctrl+W again (it shared Ctrl+F with the
  safe rect); a test wants no two rows of the Level tables on one default key.
- Copy writes the selection to `temp\clipXXXX.tmp` and puts the C++ record on the clipboard, so
  either editor pastes the other's copy. Paste follows the C++: free names, nothing selected, the
  copied flags kept (so Duplicate leaves the copies unselected).
- Level parts (Level Settings > Scene\Targets): Enable unloads a class (not drawn, its file not
  written) and reads it again from the scene's file; Read Only refuses every edit of the class
  ("The object '...' is read only.") and keeps its file. An unload is not an undo step; Undo after it
  keeps the class's current load state (`restored()` swaps classes whose `enabled` changed).
- The recent scenes are `[editor_prefs] recent_files_N` of the editor's settings (`EditorSettings::
  recent_scenes` / `add_recent_scene`, fed by `SceneFileHooks` on every load and save).
- Deleting a scene object takes its meshes out of the sectors (a sector left empty goes, as
  `CSector::DelMesh`). Delete and the Hide / Deselect rows of the menus work on the current tool's
  class and ask nothing, as the C++; Edit > Delete in the Outliner keeps its question.
- Motions: Flags\Motionable and the Motion rows of dynamic scene objects; the envelopes are edited as
  `COMotion` (`objmotion.rs`: keys, scale and normalize, range, the pose at a time), each edit one
  undo step, the frame controls of one run merged. Playback poses the object in the viewport
  without changing the scene until Pause; Camera View follows the object during playback only.
- Breakables: the Rust build welds them on the SDK grid (`VCPacked`, 24 x 16 x 24 cells) and writes
  one part per connected piece. The C++ fork writes NONE (its extractor is commented out) - a
  deliberate difference, the breakables used to be broken in this fork.
- `Validate` on request runs the level build's checks as a job; a summary is collected per class as
  the C++ does it (Highlight Texture rows are left out: their C++ command does nothing).
- The Group tool places library groups now (`TUI_ControlGroupAdd`, `PlaceGroup`) - nothing placed
  a group before. Reload References gives a group its library objects again where it stands (a
  unique member stays, the others keep their names; the sectors are not touched). Make Thumbnail
  writes `<project>\rawdata\groups\<reference>.thm` - the C++ writes `$objects$\<scene name>.thm`,
  where nothing reads it. A library `.group` has an empty reference chunk; it is given the name it
  was opened by before it is read.
- The exit path (`gz-app/src/window.rs`, "Leaving"): `GpuOwners` hooks stop the render threads of the
  viewport, the content browser's thumbnails and the previews' thumbnails before the device goes;
  every step is caught, and a panic while leaving is logged, not raised.
- `gz-render`: `ViewStyle` (flat, textures, linear, edged faces, fog, environment) for View >
  Render; the edged faces draw the wire over the solid pass.
- Outside the brief's files: `gz-app/src/settings.rs` (the recent list), `gz-app/src/lib.rs` (the
  `GpuOwners` export), `gz-ui-previews/src/cmd.rs` (its thumbnail renderer's exit hook),
  `gz-entities` (`set_climable_material`), `gz-ui-viewport/Cargo.toml` (gz-fmt-tex for the group
  thumbnail). `gz-levelbuild`'s `ai_nodes` reads `AiMapPart::all_nodes()` (the note above).
- The tool switches (Normal Align, Object / Vertex Snap, the move and angle snaps, Pick Any Class)
  are kept in `[editor_prefs] tools_settings` as the C++ `ETFlags` (other bits written back as
  read); before, they lived for the session only.
- **A table row run over MCP never opens a question** (the lead's fix after the package): `exec`
  carries the caller's `Origin` (`exec_command` -> the row, a macro's lines, the editor command of
  a `Call`, a `Task`), and for an MCP caller the row runs inside `gz_ui::dialog::answer_as(ctx,
  Asker::Mcp)` - a scope in which `confirm`, `unsaved_prompt`, `notice` and `result` answer for the
  caller whatever asker the code that raises them names (the C++ `FromMcp()`; it wins over
  `-nodlg`). Seen live: `exec_command COMMAND_MULTI_RENAME_OBJECTS` logs `* [mcp] ... -> Yes`,
  renames, no modal. The Make question names `<editor>\gamedata\levels\<level>`: that IS
  `$game_levels$` of the editor's `fsgame.ltx` (`$fs_root$\gamedata\levels`), where the C++
  builds too - a level build never writes into the linked game.
- Looked at (windowed, a copy of the Test project): leaving with 0 and 2 panels outside the window,
  no process left; the File, Edit, Scene and Build menus; Level Settings > Targets; the Summary
  sheet; edged and flat faces; copy / paste / undo, selection files, part unload and reload, error
  lists, Validate, OBJ export, motion keys, a library group placed, reloaded and its thumbnail made.
  `ui_audit` escaped nothing. Still to look at: the Make question without `-nodlg`, motion playback
  and Camera View in motion, Random Append placing, Attach on Click, the Rename and Reset Surfaces
  questions, the Group tool's Pivot chooser. Tests written, not run.

## The first test run

Brief `briefs/tests-unit.md`. Every test is run by **`tools/rust/test.sh`** from now on; the rules
the code is held to by **`tools/rust/check.sh`** (clippy without a warning, `cargo fmt --check`,
`unsafe` only in `gz-win` - 24 uses -, the editor built and `-dump_commands` holding every `cmd` of
`gz-mcp/tools.json`; the registry is compared again only when `Grizzly.exe` is newer than its last
pass).

- **test.sh's mode.** Default: changed only. `cargo test --no-run --message-format=json` builds
  every test binary once and names it; a binary runs only when its fingerprint differs from its last
  green run - the executable's size and write time, and for a corpus binary the corpus variables
  plus one stamp of the folders behind them (each tree's folder count and newest folder time, the
  game's archives one by one; walking every file cost 5 s, the folders 1.2 s; an edit in place that
  leaves its folder's time alone needs `--all`). Green runs: `Intermediate/test-stamps.txt`; a failed
  binary is never cached and prints its failing tests. `--all` runs everything, `-p <crate>` narrows.
  Binaries run 8 at a time (half the cores) from their package folder; `gz-render` (it makes a
  graphics device) runs alone at the end. A binary hit by "os error 32" (the scanner holding a fresh
  executable) is started once more. Without a value of their own the corpus variables are set to
  this machine's game and projects. Rustdoc is not run: no crate has a runnable doc test.
- **Timings** (warm, this machine): `test.sh --all` **27.5 s** (cargo's no-op build check 0.7 s,
  corpus stamp 1.2 s, the 74 binaries 24.5 s - the longest `gz-fmt-object/corpus` 6.6 s under load;
  run alone every unit binary finishes in under 0.1 s); `test.sh` right after with nothing changed
  **2.9 s** (74 cached). Building: the first build of every test binary after `cargo fmt` touched
  127 files took 68.7 s; touching `gz-fmt-xr` rebuilds 15 crates and relinks in 8.0 s. Running, not
  linking, is the larger part of a warm run, and the test profile was left as it is: a separate test
  profile would build every dependency a second time beside the editor's `dev` artifacts.
- **Counts:** 1056 tests green - the 866 unit tests that existed, the new unit and stress tests, and
  20 corpus tests in 17 crates.
- **Concurrency** (stress tests, each under 2 s): `gz-store` (readers see growing revisions and whole
  states under 8 writers; undo / redo interleaved with edits lose and double nothing; merged steps,
  seal and the saved mark under load; a panicking command leaves the last good snapshot), `gz-task`
  (48 jobs started / cancelled from 8 threads stop at their next `check`, every `wait` returns, the
  snapshot never shows an ended job running; a group admits one of 16 at once; a panicking job fails
  and its waiters return), `gz-editor` (`FrameClock::after` from 32 tasks answered never before
  their frame; `AutoSave` under a burst ends on disk as the last edit, nothing written after the
  flush), `gz-query` (16 threads asking one key compute once per revision, `get` and `fresh` share
  a computation, a panicking computation leaves the key usable).
- **No device needed:** `gz_render::testing::test_device(limit)` makes a device on a thread of its
  own and gives up after `TEST_DEVICE_LIMIT` (5 s), printing `skipped: no graphics device`; the one
  test that draws (`a_red_box_renders_offscreen`) uses it. Everything else is tested without one.
- Removed from the workspace manifest, used by no crate: `parry3d`, `rapier3d`, `loom`, `egui_dock`,
  `jwalk`, `walkdir`, `schemars`, `zip`, `proptest`, `memchr`, `gif`, `quick-xml`, `keyring`,
  `backtrace`, `tokio-util`, and the paths of crates that do not exist (`gz-fmt-release`,
  `gz-virustotal`, `gz-cmd`). Added: `tempfile` (dev), `gz-corpus`.

The corpus (`crates/<crate>/tests/corpus.rs`): `gz-ltx` every `.ltx` of the repository's `gamedata`,
the fixtures and the projects (2863) and of the game (4098) written back byte for byte, and
`system.ltx` plus six other roots in both dialects; `gz-mod` every `mod.ltx` (6); `gz-luadata` 128
assets; `gz-nq` 39 graphs (the 36 the editor wrote byte for byte, the hand-written examples at a
fixed point); `gz-fmt-xml` 481 XML files and the rus / eng string tables in both modes; `gz-fmt-xr`
the six libraries of the editor and of the game; `gz-particles` all 1428 + 2131 effects and groups
for a second; `gz-fmt-spawn` the 100 MB `all.spawn`; `gz-entities` its 22958 entities, decoded and
written anew; `gz-fmt-object` **every 16th** of 8844 `.object` and all 1353 motions; `gz-fmt-ogf`
all 5486 `.ogf` / `.omf`; `gz-fmt-tex` **every 8th** of the repository's 22104 `.thm`, all 946 of the
projects, all 20469 DDS headers; `gz-fmt-sound` **every 4th** of 29172 `.ogg`; `gz-fmt-level` 231
built level files; `gz-scene` 37 scenes; `gz-vfs` the mount against the second reader (62042 files,
every 8th archived file read, 100 compared); `gz-game` modes, items, communities, ranks, trade
presets, 111 level changers. Rules a test states where byte equality is not the C++'s either: the
2015 SDK's scenes and older THM / `.skl` versions reach a fixed point instead; the physics update
block and pre-128 packets are written as `UPDATE_Write` / `Spawn_Write` write them.

Real bugs the tests found, fixed:
- `gz-fmt-xr`: Dead Air's 50 short `TargetColor`s (also in the middle of a list, `atom_flame`
  first) - each colour action takes the form that makes the list read to its end, a bounded search;
  `times_stored` keeps the form. The viewport now plays the game's own library (1520 effects, seen
  in the editor on a copy of Test, no note in the log).
- `gz-ui-viewport`: the particles note was logged once per concurrent first read - one read at a
  time, and one note per text.
- `gz-ltx`: `r_u64` saturated at `i64::MAX`; a scene GUID half above it no longer matched its binary
  parts - 20 of the repository's 35 levels and Overnight Delivery's `l03_agroprom` were refused
  ("Level part GUID does not match the scene"). `_strtoui64` now.
- `gz-scene`: a cp1251 scene file was written back as UTF-8; each file keeps its code page.
- `gz-scene`: the compiler error list read a caption's size and capacity 8 bytes too early.
- `gz-entities`: bytes a class does not read at the end of a state (Call of Pripyat's zone time
  marker the spawn compiler left, smart terrains) made 7027 of the 22958 entities opaque; they are
  kept and written back.
- `gz-query`: `Query::fresh` computed once per concurrent caller; it waits for a running
  computation now, and a caller that stops waiting or panics leaves no key marked running.
- `gz-editor`: `AutoSave` subscribed inside its task: an edit published before the task ran was not
  written and its `flush` waited for ever.
- `gz-fmt-ogf`: legacy `.omf` motions with fewer tracks than bones were refused (the engine keeps the
  rest at bind pose); models whose user data has no terminator were refused (the engine reads the
  chunk as an ini); chunks the engine reads with `find_chunk` were unpacked when marked compressed, and a
  damaged description failed the model (`dev_pda_hud.ogf`).
- `gz-ui`: a nested `answer_as` scope ended the outer one too.
- ~20 test expectations were wrong and were corrected from the C++ (named in their comments): the
  engine's normal quantisation, `D3DX_OptimizeVertices`, `+N more`, the default bone part, the
  unnormalised box filter, include order, a touching pair of boxes, the stalker's `[game_info]`,
  the empty Lua environment, `color_argb_f`'s order, `noise3Init`'s shuffle, the placeholder
  material, the light's six decimals.

Defects of the game's data the corpus shows (read-only, not changed): Dead Air's engine takes the
short `TargetColor` only at a list's end, so 50 effects read out of step in the game; 27 `all.spawn`
packets are shorter than their sections' classes read now (repair kits, containers, generators) and
stay opaque; `internal_light_torch_r2.dds` states a 128-byte header (DirectXTex refuses it, so do
we); two `.omf` hold the exporter's nameless default part; 7 THMs of Weapons Evolution are damaged and
220 have chunk sizes that disagree with their content.

## The differential run

Brief `briefs/tests-diff.md`. **`tools/qa/diff_editors.py`** runs the same commands on the C++ editor
(the oracle) and the Rust one and compares the answers step by step and the two project copies and
output folders file by file. Seven scenarios (`s1_reads`, `s1_shell` - windowed, minimised: the shell's
`list_commands`, `shortcuts`, `state` -, `s2_manifest`, `s3_assets`, `s4_content`, `s5_builds`,
`s6_scene`), 474 steps; copies of Overnight Delivery (plus `docs/nq/examples`) and Test in the system
temp folder, `export_target` into it.

- **The oracle is recorded once** (`Intermediate/oracle/<scenario>-<key>/`: the steps as expanded,
  the normalised answers, the watched files after each step, every file's size and SHA-256, the bytes
  of the changed ones up to 16 MB, a ruled file in its rule's form). The key hashes the scenario's
  steps and the code that expands them, `LevelEditor.exe`, the source project, the examples and the
  linked game (engine, `fsgame.ltx`, archives). A scenario keeps one record. The C++ editor starts with
  no `_projects.ini`, `level.ini`, `level_imgui.ini` (all three put back byte for byte after), as the
  Rust one does in its fresh home; nothing starts it while 28016 answers.
- **Only what changed runs**: `Intermediate/diffrun/state.json` keeps each scenario's result with
  `Grizzly.exe`'s size and write time and the scenario's key. The Rust side runs scenarios side by
  side (half the cores), each on its own port (28120+), copy and `-home`, headless but `s1_shell`. A
  failed scenario leaves the Rust copy in `Intermediate/diffrun/<scenario>.rust` and its answers in
  `<scenario>.rust.json` until its next run.
- **`-home <dir>`** (`grizzly/src/cli.rs`, `AppPaths::home`): `_projects.ini`, `grizzly_editor.ltx`
  and `logs\` go there; the data root without it. `qa_target` gives every Rust editor a temp home
  (`GRIZZLY_QA_HOME` names one), `qa_target.projects_ini()` is the list the target reads.
- **Times** (this machine): the first full run, oracle recorded on the way, 136.7 s (the C++ side is
  serial: 7 records, 119 s); a full run with the oracle recorded 27.3 s; nothing changed 0.4 s.
- `--write-parity` writes PARITY.md's section "Commands compared by the differential run": 183
  commands, 82 equal, 12 equal with fields ignored, 89 not compared (window, the network, Rust only,
  the C++ camera, the project list).

Fixed at the cause (the run found them):
- `gz-project` / `gz-scene` / `gz-game`: `list_assets` Visuals, Textures, Textures (raw), Sounds (the
  editor's folders, `editor_file_list`), Particles, Light Anims, Entities (async providers:
  `AssetLists::provide_async`) answered "no enumerator"; Groups lower case in byte order, as `FS`.
- `gz-vfs`: the game's loose `gamedata` keeps dot-names (`.overlay-manifest.txt`, as the C++ DARF
  content); a module still leaves tooling out. The saved index is version 2.
- `gz-mod`: a new key or section goes where the C++ `Save` writes it (`section_rank`, `key_rank`):
  `[requires]` after `[update]`, `disable_game_updates` after `website`, ...
- `gz-store` / `gz-editor` / `gz-project`: a file operation's failure report is `{"ok":false, ...}` with
  no `error` (`CmdError::report`).
- `gz-entities`: a scene section's numbered values are written back as read until the entity changes
  (`raw_ini`), and values past the class's fields are kept (`ini_tail`: the repository's Call of Pripyat
  zone scripts write `last_spawn_time`); `spawn.part` was 9 KB short. The Game Type row is gone
  (commented out in the C++), `ALife\Story ID (script)` added (`ScriptStoryOf` / `WithScriptStory`).
- `gz-scene` / `gz-ui-level`: Details and Level Settings rows in the property tree's order
  (`tree_order`), story ids by name, the Profile by the Spawn tool's name (`PropContext`
  `story_names` / `profiles`, kept in `SceneService::prop_cx` for the panel), Level Settings' help
  (`kWorldHelp`) and the smooth angle's unit; `spawn_set_visual`'s error had a doubled backslash.
- `gz-nq` / `gz-world`: `quest_lookup level_changer` and `way_path` answered empty until `all.spawn`
  was read: `WorldLookup::ready`.
- `gz-app`: `shortcuts` lists the C++ table's editable rows in its order under the Keyboard page's
  names (`COMMAND_NAMES`, `command_name`); a Run Macro preset's `p1` is the slot's file; `progress`
  `status_line` is the line as the C++ log held it (`gz_log::marked`); `window_control close` with
  `discard` (or `-nodlg`) drops the work first (`discard_all`; the scene's `DirtySource::discard`,
  `SceneCmd::Forget`), so recovery ends clean.
- `gz-store`'s reader stress test looks once before it stops (it failed on a busy machine).

### Differences the differential left

Each is a line of the script's IGNORE / IGNORE_ROWS / NORMALISE / FILE_RULES with its reason; the lead
decides which enter section 12.
- `list_commands`: the 35 C++ rows of plumbing or with bodies that do nothing are not listed (PARITY
  section 1); rows 123-131 and `keys` are additions (D13).
- `shortcuts`: the Rust editor ships default keys (the repository's `level.ini [shortcuts]`); the C++
  binds none without a `level.ini` (D3). Its four do-nothing rows are not listed.
- `list_assets Particles` `total`: the C++ `CPSLibrary::Load` stops at Dead Air's first short
  `TargetColor` and keeps one group (1035 of 1428).
- `mod_package` `warnings`: the C++ glues the unsigned-release sentence to the last build warning.
- `mod_export_spawn` adds `missing` and `removed` (D13).
- `property_inspector`: the six Detail Objects rows of Level Settings are on the Detail Objects panel
  and `details_tool`; `Scene\Targets` is in Level Settings (the C++ Preferences); a new smart cover's
  `hold position time` is uninitialised memory in the C++ (`CSE_SmartCover` never sets it; drawn
  clamped to 60), 0 in Rust. A spawn's `Model\Animation` is compared since "The QA suites" (the Rust
  editor plays it).
- A level bound shape counts as visible in the C++ until a viewport frame runs `CEditShape::OnFrame`;
  the minimised oracle never ran one (`object_info`, `scene_tree`, `scene_visibility`).
- Not in the tables, kept out of the scenarios: the C++ ray pick skips objects its camera does not see
  (`CSceneObject::RayPick`, `occ_visible`) - `place_way` snapping, `probe_vertical`, `scan_walls`,
  `drop_objects` follow a minimised window's camera; the C++ `scene_visibility` refuses a JSON boolean
  `visible`; the C++ undo stack after a Details edit is not the Rust one (D8).
- Fields of their nature: the index fingerprint hashes absolute paths; `fps`, `dpi`, `ui_scale`;
  packages, the index's byte columns and the descriptor's sizes (D4); `preview.png` (D2); the
  shell's tabs (`temp\session.ltx`).

## The QA suites

Brief `briefs/tests-qa.md`. Every suite of `tools/qa` (17), the two `tools/mcp` scripts (through
`tools/mcp/run_lifecycle.py` and `tools/mcp/run_benchmark.py`, which give them an editor of their own)
and the new `test_mcp_bridge.py` and `test_stress.py` run against the Rust editor through
**`tools/qa/run_all.py`** - side by side (`-j`, a quarter of the cores), each suite with its own port,
`-home`, project copy and mock port, a hard timeout, `PASS/FAIL <suite> <seconds> <failed checks>`,
every output line stamped with its second in `Intermediate/qa/<suite>.log`, results in
`Intermediate/qa/state.json` (`--failed` reruns what failed, a name runs one suite, `--extra` passes
arguments to it). `test_stress`, the lifecycle and the benchmark run alone after the others.

`qa_target` gained `copy_project` (no `.git`, `releases`, `temp`, `logs`, `backup`; `project.ltx`
naming the copy; `export_target` pointed into the suite's temp folder - no suite can build into the
game), `mark()` and a `WALL <s> (copy, editor start, checks)` line every suite prints when it ends;
the start loops poll at 50 ms. The suites restore `qa_target.KEPT` and read `qa_target.log_glob()`.

Fixed in the editor (the suites found them):
- `gz-ui`: **`file_dialog`** - the one way the system's file dialog opens (`pick(ctx, asker, FileRequest,
  then)` on a thread of its own); for an MCP caller, a held caller scope or `-nodlg` nothing opens, the line
  `~ [dlg suppressed] <title>` is logged and a table row answers `refused: name the file`. Twelve callers
  moved; `rfd` is a dependency of `gz-ui` alone; `check.sh` fails on `rfd::` anywhere else.
- `gz-ui`: dialogs are centred in the shell's work area (`dialog::set_work_area`, set by the chrome every
  frame), not in the whole window; a sheet is listed after the dialog it belongs to, so `modal_on_top`
  names the sheet (`dlg.catalog_withdraw`).
- `gz-ui-viewport`: **spawn animation** (`spawn_anim.rs`): a spawn's model is posed by its startup
  animation - the first frame, paused, as `PlayAnimationFirstFrame`; First Frame / Play / Pause (a toggle,
  `TogglelendCB`) / Stop / Last Frame (0.4444 s before the end, `SetBlendLastFrameCB`) from the Details row
  `<section>\Model\Animation` (the C++ buttons, which replace the startup-animation chooser) and over MCP
  `spawn_animation` (tools.json `grizzly_spawn_animation`); rigs (bones, IK, embedded and `.omf` motions)
  read once per visual on the IO pool, paused poses cached, the viewport draws every frame while one plays.
- `gz-ui-viewport`: **a deadlock** the stress test found - two scene passes built at once on the CPU pool:
  a worker inside the first pass's rayon work under the extras' lock (details, AI map, wallmarks) stole
  the second pass and waited for that lock for ever; the CPU pool stalled and a scene load hung at
  "Parsing". One pass is built at a time now (`packet_gate`).
- `gz-ui-viewport`: `viewport_controls` reads the Preferences' snap steps before it answers.
- `gz-entities` / `gz-scene`: a graph point's Location rows are tokens over `game.ltx`'s `[location_N]`
  (`PropContext::locations`, `GameSettings::prop_context` reads `game.ltx` and its includes); a flag shown
  with captions (a lamp's Light\Type Point / Spot) answers the caption in force.
- `gz-scene`: `open_scene` waits for a scene job that runs (the project's last scene opening) instead of
  refusing it - the C++ load was synchronous.
- `gz-project`: the file tree never lists a file being written (`<name>.gz~` of `fsx::write_atomic`) - a
  build that walked the tree while the bake wrote `spawn\*.xspawn` failed on it; `project_delete`'s
  refusal spells paths as the wire does (`/`); `Project::game_linked_now` - the window drew a `stat` of
  the game folder every frame, 70 ms frames while a build kept the disk busy.
- `gz-ui-content`: `content_create` without `folder=` makes the asset in the folder the browser shows
  (`install_hooks`: the hook was never filled).
- `gz-ui-mod`: the Task Center's build and packaging rows are pumped from the start (a build started over
  MCP had none) and every 25 ms, so `progress` sees a copy of a few hundred ms move.
- `gz-app`: `progress` answers without the UI thread (`JobMarks`); `window_control resize` answers once
  frames are drawn at the size asked for (or the size Windows clamped it to); the palette's query is the
  C++ 256-byte field (a longer one is not taken, never cut).
- `gz-mcp`: a request line over 1 MiB is refused with the C++ text (`request buffer exceeds 1 MiB`) and
  the connection closed (the limit was 64 MiB and missed a line whose end came in the last read);
  `GRIZZLY_MCP_REQUEST_TIMEOUT_MS` is taken down to 100 ms.
- `gz-characters`: `characters_delete` rebuilds the roster before it answers: a quest validated next says
  the character is gone.
- `gz-catalog`: a job started right after the last one ended waits for its worker to leave the job table
  (it was refused as busy now and then).
- `gz-ui-graph`: `quest_view roles_more=show|hide|toggle` (a behaviour graph's roles keep Title, Binding
  and Default under a folded More); the selected action is scrolled into the node pane at once, also when
  the split moved in the same call.
- `gz-ui-level`: the questions only a click raises can be put up to be looked at:
  `ui_dialog open id=make_question | rename_question | reset_surfaces_question`.
- `gz-github`: under `-qa_github`, `GRIZZLY_GITHUB_PAUSE_MS` shortens the pauses before the served check
  and an upload's retry (3 s / 2 s for the author). `tools/qa/github_mock`: `--vt-rate-window MS`.

Suite setups moved (what they check unchanged): `test_characters` copies the reference mod without its
quests (it deletes the author's characters, which those quests name - E030); `test_properties` presses
Quality > Custom before it sets an out-of-range Hemisphere quality (the Escape scene is built at High,
where the row is read-only in the C++ too); `test_behaviours` opens the roles' More before it looks for
the Binding row (the C++ moved it under More after the check was written); `test_ui_shell` takes the
window of its own editor (others run beside it); `test_mod_publish` throttles the cancelled upload to
three seconds and cancels after one; `test_mod_catalog` gives the mock a 3 s VirusTotal window instead
of sleeping out a real minute twice. `diff_editors.py` fails a run at once when an editor died, and its
key hashes the data its generators read.

The differential run (`tools/qa/diff_editors.py`) now places one spawn of every entity class the linked
game's sections give (81 of 88 classes; scenario s6, 354 steps) and compares `property_inspector` on
each, `Model\Animation` included: 0 differences over 722 steps. Left out, with the reason in the script:
`AI_SPGRP`, `II_BTTCH`, `NW_ATTCH`, `W_MOUNTD` (no server class in the game's factory: the Rust editor
refuses them, the C++ places a spawn point whose type is never set) and the sections the C++ factory
cannot make (`m_lesnik`, `m_trader`, `holder_stul`: "Can't create entity", then THROW in
`CSpawnPoint::FillProp` / `SaveStream`); the class steps run with `scene_undo_level = 0`.

### Results (the whole set through run_all.py)

All 21 PASS, wall **122.7 s** for the whole set (four at a time, the last three alone):

| Suite | s | Suite | s | Suite | s |
|---|---|---|---|---|---|
| test_project_browser | 8.5 | test_characters | 14.7 | test_previews | 11.2 |
| test_ui_shell | 9.6 | test_behaviours | 8.3 | test_mod_publish | 64.0 |
| test_ui_tour | 31.0 | test_graph_ui | 11.0 | test_mod_catalog | 47.7 |
| test_asset_icons | 8.0 | test_pda | 21.6 | test_mcp_bridge | 5.6 |
| test_unicode_names | 9.0 | test_screens | 7.1 | test_stress | 11.7 |
| test_build_contents | 10.5 | test_world | 5.3 | test_server_lifecycle | 3.5 |
| test_build_job | 26.3 | test_properties | 5.9 | benchmark_scene_load | 2.3 |

`test_mod_publish` is over the 60 s budget under load (56 s beside the catalog suite alone): its time is
some thirty packagings of the reference mod (a 6 MB level-9 zip each, about a second) and the waits it
plays against the mock; the editor's own pauses there are now QA knobs. The stress run: ~5000-6600
requests in 8 s from ten connections (reads, scene edits, UI commands, screenshots) beside back-to-back
builds (61-68, every one ended) and scene loads (22-34, every one loaded), the worst ping 16-32 ms, the
editor gone 0.6 s after `window_control close`, no panic in its log. The benchmark: Escape loads in a
median 187 ms (ready 212 ms) against the C++ editor's 731 ms (750 ms), 6006 objects both.

### The look that was skipped

Looked at on a copy of Test: the exit path with a panel in a window of its own and the viewport (the
process gone in 0.65-0.9 s, every run since); the AI Map, Detail Objects and Wallmark panels;
Generate Selected (262 nodes from one) and the AI map's nodes drawn; `ui_audit` escaping nothing;
`frame_pacing` while generating; a spawn's animation played, paused, stopped and at its last frame (the
pictures differ as they should); the Make Game, Rename Objects and Reset Surfaces questions (centred,
named, closed by Cancel). Not seen, for want of data or of a way to click: details drawn and a wallmark
drawn (Test has no detail slots or wallmark textures, and First Initialize needs a snap list - "Snap list
empty!", the C++ text), Random Append placing and Attach on Click (a click in the viewport; no MCP path
and no synthetic input), the Group tool's Pivot chooser, motion playback with Camera View. Generate Full
on Escape against the C++: the C++ editor has no MCP command for it - not compared. (All of these were looked at or
settled since: "What the QA pass left, closed".)

### Budgets (plan 10.8, measured)

- **The window within 0.3 s of the process start (warm)**: the 0.12 s once written here was measured to the
  wrong point (the profile ended before the window and the device existed; the first frame was 1.0-1.2 s).
  Measured again since: "The reviewer's findings, closed", 7 - the window shown 0.07 s, the first frame
  0.96-0.98 s.
- **File operations of a start**: 383 (30 reads) for a start that opens Test; the C++ editor's 82k.
- **No UI frame over 16 ms caused by the editor's own work**: measured with `frame_pacing` on Overnight
  Delivery's Agroprom, the window restored to 2160 x 1350 and View > Real Time on: idle mean 6.6 ms, max
  20 ms (frames bound by the GPU's acquire / present); during a build mean 12-18 ms, max 52-73 ms; during
  a scene load max 15 ms. A stack sampler on the UI thread found one piece of editor work there - a
  `stat` of the game folder every frame (`game_linked`), fixed (`game_linked_now`); what is left in the
  samples during a build is drawing and wgpu's acquire / present while the build's parallel copy holds
  the cores and the disk. Not met in the strict sense: frames over 16 ms do occur, idle as well. (The
  cause was found since - tessellation on rayon's global pool and the viewport's renderer made on the UI
  thread: "The reviewer's findings, closed", 9.)
- **A rebuild no slower than the C++ job**: the Rust module build of Overnight Delivery with
  `test_build_job`'s 1.1 GB payload takes 1.1 s; the C++ job was not timed on the same data in this pass,
  so the comparison is not made. (Made since: "What the QA pass left, closed" 5 - C++ 4.89 s clean / 4.36 s
  rebuild, Rust 0.9-1.2 s / 0.2-0.3 s.) The scene load (the benchmark) is 3.9 times faster than the C++ one.

### The game as the judge

- `tools/nq/run_lua.py test_examples.lua` with `NQ_EXAMPLES` = the three examples as the Rust editor
  writes them (`quest_write`, canonical text): **874 passed, 0 failed**.
- `tools/nq/test_pda_layout.lua` with the Rust editor's PDA template (`pda_write`) and Overnight
  Delivery's `shmyandex.pdaasset` written back by it: **100 passed, 0 failed**, both assets read without
  a warning.
- In game, once: `tools/qa/nq/Run-NqQa.ps1 -Scenario PdaModule -TimeoutSeconds 180` with the game suite's
  own QA module `pda_qa` (its `mod.ltx` and the courier layout fixture, opened as a project and BUILT by
  the Rust editor into a temp folder): **PASS, 8 / 0**, the layout's three pages shown and captured in the
  isolated QA root. The QA root (`<game>\_qa
q`, its database junction unlinked first) and the run's
  results folder were removed after; no installed module was touched.

## What the QA pass left, closed

Brief `briefs/qa-rest.md`. Numbers on this machine (16 logical cores, 64 GB).

**1. Every gesture of the viewport has an MCP path.** `viewport_gesture` (`gz-ui-viewport/src/gesture.rs`;
`grizzly_viewport_gesture` in `tools.json`): `kind` click | double_click | drag | hover, `at` / `to` in viewport points,
`button`, `mods`, `steps`. A gesture is a queue of frame steps (the pointer arrives, presses, moves, lets go, rests) the
pane takes one a frame in `ViewportPane::input` in place of the window's pointer - the same code every mouse event runs;
nothing goes to the operating system. The command answers once the steps were taken, the tasks they started ended (the
pane's spawns go through `gesture::spawn`, counted in `Viewport::tracked`), the scene store applied what was posted
(`Store::flush`, new in `gz-store`) and two frames were drawn: `picked`, `deselected`, `selection`, `placed` (name, class,
reference, transform), `moved` (from / to), `changed` (with `attached`), `removed`, `ai_nodes` / `wallmarks` when they
changed, `tool`, `camera_moved`, `gizmo` (the pivot and a point on each arrow, in points), `viewport` (its size in
points), `log`. Refused while a dialog or the palette is up and while another tab than Level is active.
- Random Append has its MCP path too: `object_tool action=random` (on, objects, scale, proportional, min/max_scale,
  rotate, min/max_rotation in degrees) - the sheet's settings had none. The Group tool's Group and Ungroup buttons:
  `group_tool action=group | ungroup`. The Pivot chooser: `ui_dialog open id=group_pivot` (`Select Object###dlg.choose`,
  through `InspectorView::tool_chooser`, the same `tools::pivot_chooser` the panel's button opens); its pick is
  `group_tool action=align_pivot object=`. Motion playback and Camera View had their commands (`object_motion play |
  pause | camera_view`).
- Looked at on a copy of Test (Escape): a Random Append click (one box of the set, turned and scaled, at the click), Attach
  on Click (the point takes the shape's place), a marquee, a move drag on the gizmo's X arrow (X alone, one undo step),
  the Pivot chooser (the group's objects as a tree), Camera View during playback (the camera at the object's pivot).
- Found and fixed: a point's own sphere that Attach on Click detaches went back into the scene nameless (`EnsureShape`
  makes it with no name, `gen_name("")` kept it so) - it gets a free `shape` name now (`// oracle:` in `apply.rs`; the C++
  appends it nameless). `screenshot_viewport` waited up to 2 s whenever passes followed each other (`settle` waited for a
  moment with no pass in flight): it waits for the pass under way only - every viewport picture costs ~0.2 s instead of
  2.3 s, in every suite.
- Not a bug, learned: a frame change of a motion with no key poses the object by the empty motion, at the origin (the
  C++ does the same); key first, then move.

**2. Details and wallmarks, drawn.** Made over MCP in a copy of Test: the Detail Objects' snap list (the terrain), two
detail objects (`detail\trava_green`, `detail\trava_black`), a base texture written into the copy (one opaque colour,
`qa\qa_detail_base`), its colour mapped to both objects, First Initialize (641601 slots), Reinitialize (every slot
filled); a wallmark of the game's `wm\wm_anarh1` laid on the terrain. Both looked at: grass over the ground, the base
texture over the terrain in its colour, the poster lying on the ground under the camera. The repository's `det\*`
textures are the terrain's detail maps (mostly alpha 0): as a base texture they give slots of density 0 and draw black -
the data, not the editor. Proven by `tools/qa/test_scene_tools.py` (D, W: the picture against the same view without them,
waiting for the models and textures a pass asked for).

**3. Generate Full: the whole ground flooded, no duplicates.** `aimap_tool action=stats` (`aimap::stats`) tells it apart.
Escape (snap list 3847 objects, 1808170 collision faces, patch 0.7 m): the model has ground under 2659647 patches
(1303227 m², facing up within 60 degrees). The scene's own map: 624967 nodes on 624211 patches (305863 m², 23 % of the
ground), 756 patches with two floors, 3 duplicates and 1 node off the ground - already in the scene's data. Generate
Full: 2481528 nodes (1856561 added in 16.3 s) on 2473526 patches (93 % of the ground), 7861 patches with floors over each
other (at most 4), still 3 duplicates and 1 off the ground, none off the grid. The count is the whole terrain flooded from
the existing nodes - the C++ `BuildNodes(false)` floods everything reachable; the scene's own map was trimmed to the
playable area. Unit test `generation_counts_follow_the_cpp_algorithm`: a 7.2 m floor 81 nodes / 288 links (histogram 4 /
28 / 49), a 2 x 2 m hole 72 / 240, a wall half way between two columns 36 / 118, a 2 m step 36 / 118 from below and from
above - numbers worked out from `CreateNode` (9 x 9 rays, `rc_lim` 0.7), `CanTravel` (the body stops 0.145 short of the
next patch) and `BuildNodes`; the Rust generation gives exactly them.

**4. The UI frame while a build runs.** On the user's word the pools keep normal priority and every core (CPU pool =
16, IO pool 32, async 8); the thread that draws and presents (the UI thread, `gz_app::window::run`) and the viewport's
render thread run ABOVE normal (`gz_win::raise_this_thread_priority`, one `// SAFETY:` call; `this_thread_priority` for
the test). `frame_pacing` answers, over the frames since its last ask, `sampled_frames`, `frame_work_ms` {mean, p95, max}
(the frame's own time: input, the UI pass, tessellation, recording), `frame_wait_ms` {mean, max} (the swap chain's image,
the submission, the present, a capture's poll - `paint_and_update_textures`' own vsync time) and `slow_frames`
(`FrameClock` keeps 4096 frames' times; `FrameStamp` carries `work_ms` / `wait_ms`).
- Before (this file, "Budgets"): a whole frame's time only, idle mean 6.6 ms / max 20, during a build mean 12-18 / max
  52-73. After, same data (Overnight Delivery's Agroprom, 2160 x 1350, Real Time, `test_build_job`'s 1.1 GB payload,
  `tools/qa/bench_build.py`): idle work mean 3.1-3.7 ms, p95 3.9-6.8, wait mean 8.2-8.7 (vsync); during the clean build
  work mean 5.0-7.8 ms, p95 13-73, max 36-218, slow frames 2-5 of ~76. The long frames are stalls inside egui's
  tessellation (20k vertices; 20-90 ms) during which the UI thread was charged no CPU time at all (GetThreadTimes): it is
  blocked, not working, and only at moments of heavy disk IO (a scene's assets loading, the build's copy). Raising the
  UI thread changed nothing measurable (A/B, two runs each); free memory 45 GB, so no paging. NOT CLOSED: the cause of
  those stalls was not found. `test_build_job` A holds the mean: frame work under 16 ms during a build (window shown, not
  activated, Real Time on; measured 9 ms there).
- CPU use of a clean build: 5.2-6.7 s of process CPU over 1.03-1.16 s of wall, 0.24-0.38 of 16 cores. The stages: bake of
  the scene 0.33 s, the module's checks 0.38 s, copy and install 0.3 s; the copy and the reuse check run on the IO pool
  in parallel, the build is bound by the disk there. Whether the bake and the checks keep all cores busy was not profiled.
- Found under load and fixed: `Project::game_snapshot` mounted the game under a mutex while the mount's parallel walk
  ran on the IO pool; a worker waiting inside that walk ran another queued `game_snapshot` and blocked on the lock its
  own stack held - the editor hung (two editors of a `run_all` at once: a publish stuck in "Checking the module", a quest
  edit of `test_mcp_bridge`; seen with a stack sampler over dbghelp). No lock is held across a mount now
  (`put_game_first`: two first callers may mount twice, the first index stays); `refresh_game` and `link_game` likewise.

**5. A rebuild against the C++ job** (`tools/qa/bench_build.py`, reference mod + 24 x 48 MB payload, 228 files, a copy
in temp each): C++ (`LevelEditor.exe` over 28016, minimised) clean 4.89 s, rebuild with nothing changed 4.36 s; Rust clean
0.91-1.22 s, rebuild 0.20-0.33 s. The C++ editor's `_projects.ini`, `level.ini`, `level_imgui.ini` came back byte for
byte.

**6. The suites' time.**
- `--only <phases>` in `test_mod_catalog` (A-J, W, I), `test_mod_publish` (A-J, R, U, V, K, L, N, M), `test_pda` (A-H),
  `test_ui_shell` (A-H), `test_build_job` (A-E), `test_previews` (T, M, L, S), `test_scene_tools` (R A M G P V D W),
  `test_ui_tour` (its issue numbers, as before); `run_all.py <suite> --only ...` passes it on (and notes it in
  state.json). A phase stands on its own setup: one that needs another's work runs that phase first without its checks
  (`NEEDS`, shown as `TIME phase X (setup)`). The other suites keep their checks in one run (each under 16 s).
- `test_mod_publish` and `test_mod_catalog` work on a trimmed copy (`qa_target.TRIMMED`: no rawdata, design, docs,
  textures - no phase is about a package's size; I gets 6 MB that do not pack, so the throttled upload still meets the
  cancel); one mock process for the publish run, its switches a `POST /__restart` (new in `tools/qa/github_mock`, the
  state as a new process has it) instead of ~30 process starts of 0.4 s each; the mock is built once per version of its
  sources (`Intermediate/qa/mock`, keyed by their hash) instead of on every run; polls at 50 ms; `GRIZZLY_VT_SCAN_WAIT_MS`
  1.5 s; the device sign-in approved on the first poll. The catalog's pictures are decoded with Pillow when it is there.
- `test_pda`: the sleeps before pictures only with `--shots`; the waits for the tab to read a file written behind it
  were sleeps - `pda_preview` reads it before it answers. `test_build_job`: the packaging phase does not pack 96 MB of
  random payload; start poll 50 ms. `test_ui_tour`: polls at 50 ms. D of `test_build_job` (6.6 s) is the install's own
  patience for a file held by a scanner (30 x 200 ms) before it refuses a module in use - product behaviour, kept.
- Alone: `test_mod_publish` 64.0 -> 28.9 s, `test_mod_catalog` 47.7 -> 23.2 s, `test_pda` 21.6 -> 6.5 s,
  `test_build_job` 26.3 -> 15.0 s, `test_ui_tour` 31.0 -> 20.9 s, `test_ui_shell` 9.6 -> 6.1 s, `test_scene_tools` 10.5 s.
  The whole set through `run_all.py`: see "The final runs" below.
- Safety (the lead's order): a suite only ever opens a copy under the system temp folder. `qa_target.copy_project` is the
  one copy (whole or `trimmed`, export target into temp; it refuses a destination outside temp) and every suite and
  `tools/mcp/run_*.py`, `diff_editors.py`, `bench_build.py` calls `qa_target.guard_project(<path>)` before `-project` /
  `open_project`: a path outside temp stops the suite with exit code 2 before an editor starts. `test_mod_publish` opened
  the source in place until this pass (it restored `mod.ltx` in `finally`); it copies now. `run_all.py` ends a timed-out
  suite's whole process tree (`taskkill /T`) and names the suite.
- `tools/rust/cg.sh` prints a test run's failures and counts (it printed only the last line, often blank).
- Found by the full set under load and fixed: `quest_view` could answer for a tab that came up a moment ago and was not
  drawn yet (its graph still being read: every pane null, `test_graph_ui` failed one run in three) - it holds until the
  tab was drawn once (`view_pending`, up to 5 s); `viewport_gesture` answers after a pass of the scene as the gesture
  left it (`drawn_scene`), so `gizmo` is that pass's anchor.

**The final runs** (2026-11-12, about 03:40-04:05):
- `tools/rust/check.sh` ok; `tools/rust/test.sh` 1068 tests, all green; `tools/qa/diff_editors.py` 722 steps, 0 differ.
- `run_all.py`: 22 suites, 21 passed in one go, wall 102.1 s. `test_build_job` failed there: A measured the frames while
  three other suites ran beside it (work mean 23.6 ms, p95 132, max 572). It times frames, so it is an `alone` suite now,
  like the others that time something (run after the rest, by itself); alone it passes in 16.0 s (work mean 8.0 ms, p95
  22.4, max 75.7). The limit (16 ms) stays. Seconds per suite in the parallel run: asset_icons 9.6, project_browser 9.7,
  ui_shell 11.2, unicode_names 8.7, build_contents 9.8, behaviours 11.0, ui_tour 33.6, characters 17.7, graph_ui 13.4,
  screens 8.7, world 5.1, pda 12.0, properties 7.2, previews 11.0, mcp_bridge 6.6, scene_tools 19.0, mod_catalog 30.5,
  mod_publish 38.7, stress 11.7, server_lifecycle 3.5, benchmark 2.6.
- NOT REACHED: the set under 75 s. Under four suites at once the longest ones stretch (ui_tour 20.9 s alone -> 33.6,
  publish 28.9 -> 38.7, catalog 23.2 -> 30.5) and the `alone` ones add their own time at the end; the set is bound by
  those three plus the alone tail, not by the runner. Nothing was cut from their checks to get there.
- Found by the final runs and fixed: the copies of `test_graph_ui` and `test_behaviours` stayed in temp (`ignore_errors`
  rmtree cannot remove a copy's read-only git objects). `qa_target.remove_tree` (read-only entries too, a few retries,
  a warning naming what stayed) is the one removal every suite and `tools/mcp/run_*.py` use now; both suites rerun, their
  copies gone.
- The sources, checked with `find -newermt '2026-11-12 03:15'` before and after the final runs: nothing in
  `D:\XFinedProjects\Test`, `D:\XFinedProjects\WeaponsEvolution` or `D:\Games\Dead Air\modules`. In
  `D:\XFinedProjects\DARF-Overnight-Delivery` only the `.git` folder's own time (03:37:12) is newer: no entry inside it
  and no file of the project changed. Since `test_mod_publish` was the one suite that opened that project and it copies
  now (copies leave `.git` out), the folder's time is not from a suite; the cause was not found.
- After the runs: no `Grizzly.exe`, `LevelEditor.exe` or `github_mock` of these runs left; the repository's
  `_projects.ini`, `level.ini`, `level_imgui.ini`, `grizzly_editor.ltx` unchanged (`git status`); the temp copies removed.

## A QA fault: the author's project was opened in place

From the QA pass until 12 Nov 03:15 local, `tools/qa/run_all.py` handed `test_mod_publish.py` the
author's own project (`D:\XFinedProjects\DARF-Overnight-Delivery`), and the suite opened it in place:
it rewrote `mod.ltx` for the run (version 0.9.6, a QA repository), packaged into its `releases\`,
restored the manifest and removed its release at the end. Three runs did so (the QA pass's, the
lead's acceptance run, one run of the pass after). Found by the reviewer. Checked by the lead:
`mod.ltx`, `preview.png` and `spawn\l03_agroprom.xspawn` equal, by SHA-256, the copy the differential
run took before those runs (`mod.ltx` and the `.xspawn` also the module installed in the game);
`project.ltx` reads whole; `releases\` holds 1.0.0 and 1.0.1 only; the write times of those four
files and of `temp\session.ltx` are 03:09-03:10. Nothing was lost; a run killed midway would have
left the author's manifest changed. Since then a suite opens only a copy under the system temp folder
(`qa_target.copy_project`, `guard_project`: exit code 2 before an editor starts), and `run_all.py`
ends a timed-out suite's whole process tree. The source checks further down start at 03:15 for that
reason.

## What the suites could not hold the Rust editor to

ImGui artefacts with no meaning in the new shell:
- the files the C++ editor rewrote (`level.ini`, `level_imgui.ini`): the suites restore `qa_target.KEPT`
  instead (`_projects.ini`, `grizzly_editor.ltx` of the repository root, which the Rust editor never
  writes - it writes into its `-home`);
- `test_mod_catalog` F: the stale `level_imgui.ini` entry that put the Submit window below the editor is
  not written for the Rust target (the shell keeps no window placements); the checks behind it stay and
  pass - the window inside the picture, centred, more than a flat fill, also of a minimised editor.

Deliberate differences of section 12:
- D15: `test_server_lifecycle.py --shutdown` runs `exec_command COMMAND_QUIT`, a plumbing row the Rust
  editor does not list; `run_lifecycle.py` proves the same shutdown with idle clients through the palette
  (`COMMAND_EXIT`).
- D6: `lint_ui_units.py` is not run.

## The reviewer's findings, closed

The brief `docs/rust/briefs/critic-fixes.md`, 12 Nov. One line per finding: what was done, where its test is.

1. **Level exports and the level build compared for real.** Every step of `tools/qa/diff_editors.py` says what it
   expects (`ok` unless `expect="refused"`); a step that answers otherwise fails the scenario, while recording the
   oracle as well. The scenarios do the work now: S2/S4 with a quest the reader takes (named tasks), files that
   exist for rename / move / delete; S5 opens Agroprom first, so a build bakes ("Scene baked: 346 object(s)");
   S6 places a golden static (`statics\army_base\prop_map_02`), selects before every export, one Player Start, and
   exports `overlay.xcform`, the cut and the `.ogf` of the selection and of 40 objects of the scene; S7 runs
   `level_build make_all` on Escape of Test against the C++ `COMMAND_BUILD` (test_barrel, a visual-mode object the
   C++ build crashes on, deleted first). Binary outputs are watched as `sha256:<hex>:<size>` and compared by the
   rules of "For the plan"; the summary counts the files really compared. The oracle's key hashes LevelEditor.exe
   and every DLL beside it (`ORACLE_FORMAT` 3); a C++ level build that wrote into `<repo>\gamedata\levels` fails the
   run (`REPO_LEVELS`). Golden tests from the oracle: `crates/gz-export-level/tests/golden.rs` (+ `tests/data`).
   Rust faults the repaired scenarios found and fixed: the level build wrote into the editor's folder (the C++
   writes into the project's `gamedata\levels` through its `$game_levels$` mount); `Fbox::xform` was a different
   algorithm (`gz-scene/src/math.rs` `xform_box`, an exact port); a zone or a smart terrain made in the scene had
   no editor-script block in its packet (`gz-entities` `write_editor_script`, test
   `a_new_packet_carries_the_editor_scripts_defaults`).
   Game suites on Rust output: `Run-ScreensQa.ps1 -Part Descriptions` with the 87 `.npcasset` files the Rust editor
   shipped in a build of a copy of Overnight Delivery: 31 of 31 checks pass in 39 s, the extra module logs
   "85 character(s) read" (the other 2 are templates) and no file left out (driver
   `Intermediate/fix/screens_on_rust.py`; QA root, results folder and the compat-flag value it set removed after).
   `Run-ModsQa.ps1` cannot take Rust input without a change to the game repository: Updates and Catalog generate
   every module, descriptor, index and package themselves (`catalog_qa.py`, "an independent second reading of the
   contract"), and Adopt's `-RealRelease` takes only darf-overnight-delivery's release signed with the key the
   public catalog lists - a QA run does not sign with the author's private key. `Run-NqQa.ps1 -Scenario Scout`
   neither: it installs the suite's own `tools\qa\nq\module` (`Set-QaModule -Variant Full`) and has no parameter
   for another module (only PdaModule takes `-PdaModuleDir`, run by the QA pass).
2. **The level path guard.** `gz_base::file_name_problem` (one component: no separator, no `.`/`..`, no drive, no
   reserved name, no trailing dot or space); refused when typed (`gz-scene` props, test
   `the_level_path_is_one_folder_name`) and by the build (`gz-levelbuild/src/folder.rs` `level_folder`); before a
   wipe the real path must lie strictly below the levels folder and not be a link (`check_wipe`; tests in
   `folder.rs`, a junction made with `mklink /J`). Tools: the plain / `_discard` pairs for `open_scene`,
   `quest_close`, `characters_view`, `level_parts`, `scene_new` (8 discard tools), `destructiveHint` on
   `level_build`, `content_delete`, `quest_reload`, `content_copy`, `content_move`, `darf_copy`,
   `content_browser_copy`; bridge tests `only_a_discard_tool_drops_work_and_every_one_is_marked`,
   `a_discard_tool_sends_discard`.
3. **No QA run opens a real project.** The guard read first (`qa_target.guard_project` / `copy_project`), every
   suite that starts an editor goes through it; `diff_editors.py` copies through `copy_project`; the scripts of
   `tools/mcp` that drive an editor already running (`benchmark_scene_load.py`; `test_server_lifecycle.py` with
   `--timeout-scene` or a shutdown) refuse unless the editor's open project (`state.project`) and an absolute scene
   path lie under the system temp folder. `find` before the first run (12 Nov 04:23:21) and after the last:
   see the end of this section.
4. **The UI thread waits on no GPU readback.** The capture is polled on a blocking worker (`window.rs` `polls`,
   joined at exit); the device is made on a thread of its own and the surface on the UI thread with one
   `block_on` before the first frame; the extras' `set_window` takes `now_or_never`. Test: `test_ui_shell` and
   the screenshot checks of every suite (`run_all.py`).
5. **No lock across parallel work in the viewport.** Particles: the systems are taken out of the lock, stepped
   with `par_iter_mut` and put back (`draw_particles.rs`); AI map, details, wallmarks: the caches are copied out
   (`draw_extra.rs` `unlocked`). Tests `a_cache_is_unlocked_while_it_draws`,
   `passes_and_a_job_on_the_same_caches_never_stall_the_pool`.
6. **The smaller findings.** Frames: a request is answered by a frame that began after it (`frames.rs` `begin`,
   test `a_frame_under_way_does_not_answer_a_request`). Corpus: skips are printed by `test.sh`
   (`skipped <binary>: <why>`), `--all` fails on one where the script set the variables and reads every file
   (`GRIZZLY_TEST_FULL`); the corpus binaries start first. Reading every file found three things the strides
   had passed by, all settled: an entity read from `all.spawn` got the editor scripts' block when written anew
   (only one made here or read from a scene does now: `Known::made`); the game's `hk416_shoot.ogg` keeps its
   first audio packet on the setup header's page, which the parameter writer refused - such packets now get a
   page of their own (test `audio_on_the_last_header_page_gets_a_page_of_its_own`); two library objects are what
   the C++ makes of them - `rostok_outside_mp_21.object` has an empty body chunk (`open_chunk` answers null),
   `artixvakt_simple1.object` a version-1 motion whose envelope parameters every write quantises to 16 bits -
   and the object corpus test states both rules (its module comment) instead of failing on them; it compares by
   value first, which took it from 84 s to 14 s. Dialogs: a project's dialog goes down with the project, `ui_dialog close` without `id`
   closes the one on top (`test_ui_shell` F). `test_unicode_names` G: a name with a character outside the BMP is
   made on disk, found through the listing, renamed and found on disk again (the constant log check stays for the
   C++ target only). Publish: one start at a time (`StartClaim`, tests `one_start_at_a_time`,
   `racing_starts_take_one_claim`). MCP: a command runs to its end past the timeout (test
   `a_command_runs_to_its_end_past_the_timeout`). Query: a failed revision is not computed again (test
   `a_read_does_not_compute_a_failed_revision_again`); `fresh` does not cancel ("For the plan"). Crash filter:
   `TerminateProcess` after the report. Layering: the table in "For the plan". PARITY.md: no row `missing`.
7. **Start-up measured to the first presented frame.** The profile's milestones `window shown` and `first frame`
   and the stages between (graphics instance, adapter, device, surface, first frame) are in the log; the window is
   shown in the theme's background before the device exists; DX12 is asked first, Vulkan only when it fails.
   Four starts (`tools/qa/probe_start.py`): window shown 0.389 (the first, colder) / 0.070 / 0.071 / 0.077 s,
   first frame 1.558 / 0.964 / 0.969 / 0.984 s; adapter 0.39-0.58 s, device 0.15-0.18 s, surface 0.28-0.31 s.
   Test: `test_ui_shell` A fails a warm start whose window was shown later than 0.3 s.
8. **A screenshot after a scene load is current.** `screenshot_editor` holds for the viewport's pass of the
   caller's scene revision (`PictureGate`) and for its assets, up to 15 s, then answers `pending`;
   `screenshot_viewport` alike. The wait takes a finished pass of the caller's revision or a newer one, whatever
   pass runs meanwhile: while a particle system plays a new pass follows each one, and a wait for a moment with
   none under way held every picture of Escape for its whole 15 s (seen in the first `run_all` of this pass).
   Test: `test_scene_tools` S (one shot after the load, the viewport not flat, no `pending`).
9. **Frames during a build.** egui's `rayon` feature is out (`Cargo.toml`); nothing else on the drawing thread
   enters a pool (the particles' `par_iter_mut` runs in the pass build on the CPU pool). The worst frame left was
   not the build's: the Level tab's first frame made the viewport's renderer on the UI thread (412.9 ms); it is
   made on a thread of its own now (`Viewport::start_renderer`; a command waits for it in `ensure_renderer`, a
   picture in `current`). Frame work (`frame_pacing`, Real Time on, OD): before, during a build p95 13-73 ms,
   max 36-218 ms; with `rayon` out, max 18.0-46.5 ms; with the renderer off the UI thread, the Level tab's first
   1.5 s max 19.2 ms, two builds max 4.9 / 4.6 ms (p95 4.4 / 3.6). `test_build_job` A now also holds the worst
   frame (`FRAME_WORK_MAX_MS` 64): mean 4.8, p95 10.6, max 31.6 ms.
10. **Core load of a build** - closed early on the user's word (a build of about a second is fast enough). The bake
    of the open scene runs beside the module's checks (`gz-build/src/job.rs` `check_module`, `futures::join!`),
    the files listed again after the bake; the log names every stage (`* [XMS] build stages: ...`). Stages of a
    clean build of Overnight Delivery (wall s / cores busy):

    | Stage | Before | After |
    |---|---|---|
    | bake | 0.301 / 0.38 | 0.658 / 0.37 (bake and checks together) |
    | checks | 0.381 / 0.35 | (with the bake) |
    | listing | - | 0.013 |
    | copy | 0.380 / 0.32 | 0.357 / 0.32 |
    | whole build | 1.073 s, 5.95 CPU s, 0.35 | 1.030 s, 5.78 CPU s, 0.35 |
    | rebuild (hard links) | 0.305 | 0.308 |

    Parallel now: the bake beside the checks, the checks by area, the copy by file. Serial and left so: the
    first use of the linked game's data, which both the bake and the checks wait on, and the copy, bound by the
    disk (about 3 GB/s).

The final runs of this pass (12 Nov):
- `bash tools/rust/check.sh`: ok, 19 s (clippy and fmt clean, `unsafe` only in gz-win, every command registered).
- `bash tools/rust/test.sh --all`: 75 binaries, 1086 tests green, 0 skipped; run 59.6 s, total 62.7 s with every
  corpus file read (the strided default run stays as it was). The renderer start, the picture wait and the entity
  flag changed after it; those crates' tests were run again: green.
- `python tools/qa/diff_editors.py --all`: 8 scenarios, 744 steps, 0 differ; 713 written files compared, 0 differ
  (s6 the level exports 115, s7 the level build 17, s5 the module builds 497); 32.3 s.
- `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py`: 22 of 22 suites pass, wall 122.0 s. The first run of this
  pass failed 13 (the picture wait and the minimised start above); the two folders its timed-out suites left in
  the temp folder were removed by hand - a suite ended by force does not clean up after itself.
- `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 04:23:21'`: nothing, before the
  first run (04:23:21) and after the last (06:55:27). No Grizzly, LevelEditor, xrEngine or github_mock process
  left; `D:\Games\Dead Air\_qa\screens` and the screens suite's results folder removed; `<repo>\gamedata\levels`
  does not exist.

Decisions taken in this pass:
- A C++ level build writes into the project's `gamedata\levels\<level>` (its `$game_levels$` mount), not
  `<repo>\gamedata\levels`; the Rust build does the same, and the differential moves each side's output into its
  record. The `-level_out` flag tried for QA was taken out again: it was not needed.
- S7 deletes `test_barrel` before Make All: the C++ editor's `BuildHOMModel` asserts on a visual-mode object.
- The golden `.ogf`'s build-machine name is scrubbed to a fixed one; the test compares by the `.ogf` rule.
- The frame limit of `test_build_job` A is 64 ms worst, 16 ms mean.
- A minimised start (the QA harness's `SW_SHOWMINNOACTIVE`) keeps the window out of sight until it is
  restored, as before this pass: shown iconic, it reported its taskbar button's size and the previews laid out
  at zero (the first `run_all` of this pass).
- A game suite runs on Rust output only where it takes outside input (Screens Descriptions); the other two are
  named with the reason above, not forced.

## The reviewer's verification, closed

The brief `docs/rust/briefs/play-proof.md`, 12 Nov. Sources checked before the first run: `find D:\XFinedProjects
"D:\Games\Dead Air\modules" -newermt '2026-11-12 07:07:18'` (ten minutes before the start) - nothing.

**1. The small list.**
- The start-up profile says what happened: `window shown` or `window minimised` (`gz-app/src/window.rs`).
  `test_ui_shell` A checks the suite's own minimised start says `window minimised`, and makes one start of its
  own (no project, a port and home of its own) with `SW_SHOWNOACTIVATE`, shown and never activated: window shown
  0.119 s, first frame 1.262 s. `tools/qa/probe_start.py` shows its starts the same way.
- The first picture after a scene load. What it waited for, measured with timings inside the wait: (1) every
  mesh of the scene, the ones outside the view too - the packet asked the assets for the meshes of all 4252
  instances, and the wait took the global loading count; (2) the AI map's meshes of every tile the camera's
  square touched - Escape's 4096-node tiles are strips, so the first pass meshed most of the map (1.44 s), and
  two passes at once did it twice; (3) on Overnight Delivery's Agroprom, 2666 cell meshes made one by one, three
  GPU buffers each (0.7 s); (4) the project's preview pass at the moment a scene came up, beside the viewport's
  first pass. Now: the packet asks for the meshes of what is in view (`build_packet_with` with the caller's
  visibility); the picture waits for the frame of the caller's pass with nothing it drew still loading
  (`FrameInfo::pending`, `FramePacket::textures_loading`); the AI map sorts a tile's nodes into 16 m cells once
  and meshes only the cells the view radius reaches; `Assets::add_meshes` uploads many at once and announces
  once; a mesh is ONE buffer (indices, edges, vertices); a scene's first preview waits its minute (a close still
  takes one before it clears); the screenshot's PNG is encoded with one row filter (`Compression::Fastest`) and
  one frame after the gate instead of two. Numbers: the first `screenshot_editor` after an Escape load 0.92 s
  (0.47 s of it the renderer's own start in a fresh editor; 0.31 s on a later load; was 3.7 s), `test_ui_tour`
  29.6 s (was 40-56 s), `test_properties` 4.9 s (was 16 s), `test_scene_tools` S passes (the picture current).
- `gz-export-level/tests/golden.rs` writes no debug file.
- `draw_extra.rs`: the caches are `Shared<T>` - the finished cache with a version; a pass draws with its own
  copy (shallow: the AI map's chunks, the wallmarks and their meshes are `Arc`s) and swaps it in when no other
  pass finished one meanwhile. A pass never starts from nothing; two passes that meet the same change at the same
  moment both make it (waiting for the other could stall the pool, the deadlock of the stress run). Tests
  `a_cache_is_unlocked_while_it_draws`, `a_pass_beside_another_starts_from_the_finished_cache`.
- The write guard: `gz_editor::fsx::WriteGuard` (made by `Project::write_guard(module)`), the path as written and
  where it really is (its deepest existing part resolved), refuses anything under the linked game's root but the
  module's `modules\<id>`, its stage `modules\.new\<id>` / `<id>.old` and the flat `gamedata_<id>`. Passed by the
  project's file operations (`FileOps::writable`), the module build and flat export (`plan_target`), the level
  exports (`apply_spawn`, `write_level_file`, the visuals), the level build (`write_files`, the wipe, the OBJ),
  the release stage and folder (`package.rs`), the compilers' stage and Play's root. `ProjectPath` / `GamePath`
  are gone (nothing used them). Tests `a_write_under_the_games_gamedata_is_refused`,
  `a_link_does_not_carry_a_write_into_the_game` (gz-editor), `nothing_is_written_under_the_linked_game`
  (gz-project), `refusals_before_anything_is_made` (gz-build).
- `run_all.py` gives every suite a temp folder of its own (`TEMP`/`TMP`) and removes it after, also after a
  timeout (checked with a suite that sleeps past a 3 s limit: its copy and folder gone).

**2. A level that has everything** - `s8_levelbuild2`: l10_limansk, the smallest of the 35 scenes whose parts
hold all four (10.2 MB of parts; 79 sectors, 274 portals, 251001 detail slots of 18 objects, 127 wallmarks; the
only smaller one with sectors, portals and details, l08_yantar, has no wallmark), copied from `rawdata\levels`
into a copy of Test (`"scenes"` of a scenario: copied by `Side.prepare`, part of the key and the source).
Make All: 19 written files compared, 0 differ (C++ 10.7 s). Rust faults it found, fixed: the detail objects'
library objects were never loaded (`prefetch_library` names them now), their textures were read from the
linked game, which is not what `$game_textures$` names (the editor's own `gamedata\textures`) and did not fit
the atlas; a portal was written with its raw corners instead of `m_SimplifyVertices` (`prj.rs` runs the load's
`Update(true)`). Rules added: `build.details`, `build_details.dds`, the portals of `build.prj` ("For the plan").

**3. The Play half** - `s9_play`: the rows of the C++ Build menu's command table on the scene that gives each
work: Make Game, Make AI Map, Make Details, Make HOM on limansk, Make SOM on Escape (limansk has no sound
occluder: both refuse it - a C++ row's `result: 0` now counts as a refusal in `expectation`). 9 steps, 14 written
files compared, 0 differ. The collision, the AI map and the game graph built in memory, `editor.spawn` and the
start of the game are the C++ top bar's buttons (`ClickCForm`, `ClickAIMap`, `ClickGGraph`, `ClickPlayPC`) with
no command row and no MCP path: Rust only, proven by the game (below). Play as built could not run: it looked for
IXRay's `Xr3DA.exe` (not in this repository, and not the linked game's engine), it would have written into the
project only, and XrLC - run from `Bin\x64\Release` - reads the editor's `fsgame.ltx`, whose levels folder is not
the project's, so no project's level could be compiled. Now: `compile_*` run the compilers from a stage of the
project (`temp\compilers`: copies of the executables, an `fsgame.ltx` naming the editor's data and the project's
levels and spawns; `runs::stage_compilers`, `compiler_fsgame`), `params=` appends to their command line; Play
(`play`, `play_game`, `play_clean`) runs the linked game's engine in a game root of its own (`root=`, default
`<appdata>\play\<project>`; `gz-levelbuild/src/play.rs`): copies of the game's executables, a junction to its
archives, a copy of its loose gamedata with the level folder, `level.ai` (Dead Air's engine reads that name, not
IXRay's `level.ai.temp`) and `editor.spawn` staged into it, an appdata of its own (the player's `user.ltx` copied
the first time, the caches once, no save), an empty `modules`, the content manifest copied; the command line is
`-fsltx fsgame.ltx` + the C++ one + `params=`. Test `a_play_root_mirrors_the_game_and_changes_nothing_of_it`.
The default root is `<appdata>\play\<project>`, NOT in the project: the game started in a root that lies in a
project folder stopped in xrCore at its start (breakpoint, 1.4 s, no log line); the same root moved out ran.
Found on the way: `runs::arguments` dropped every parameter of XrLC / xrDO, so no `-silent` reached XrLC and its
"Congratulation!" box held the job for ever (XrLC compiles mp_test in 11 s now).

**The in-game load: NOT reached** (reached since - "Play ends in the game"). `tools/qa/test_play_game.py` (not in `run_all.py`: it starts the game; it
fails with the reason below) on a copy of Test with mp_test (its MP spawns replaced by an actor and a graph
point, an AI map of 1840 nodes generated, Make All, XrLC): the editor, on a hidden desktop, builds the collision
(16 vertices, 24 triangles), the AI map, the game graph (1 vertex), `editor.spawn`, stages the root and starts
the linked game (`xrEngine.exe`, build 10137) in it. The game reads its configs, runs every script's
`on_game_start` ("phase time: 3007 ms"), then `* Creating new game...`, `* Loading spawn registry...`, and stops
22 s after Play in `CSE_ALifeDynamicObject::on_register()` (xrGame `alife_dynamic_object.cpp`, called for the
actor from `game_sv_Single::Create` <- `xrServer::Connect`): `alife().graph().level()` - the ALife graph
registry has no current level for the actor, so the actor's graph vertex (`m_tGraphID`) or the level table of
the spawn's game graph header in the Rust `editor.spawn` is not what the game expects. Not "New game is
successfully created!". The log is `Intermediate/qa/play_game/engine-crash.log`. Ruled out: the root (the game
starts in it and loads its scripts), the settings, the caches, the staged level files as a cause at start.
What it needs: `editor.spawn` compared with one the C++ `CGameSpawnConstructor` writes for the same scene - which
the C++ editor makes only from its Play on PC button (no command row, no MCP path), so a small C++ harness or a
hand-run of the C++ editor's button on this scene is the next step; then the actor's graph id against the
graph's vertices. The equal files of section 3 stand: of the two proofs, the comparison exists, the in-game load
does not.

The final runs of this pass (12 Nov, 09:20-09:33):
- `bash tools/rust/check.sh`: ok, 44 s.
- `bash tools/rust/test.sh --all`: 75 binaries, 1092 tests green, 0 skipped; 147.9 s (build 57.6 s, run 87.8 s).
- `python tools/qa/diff_editors.py --all`: 10 scenarios, 755 steps, 0 differ; 746 written files compared, 0 differ;
  164.4 s - every oracle recorded again (the key now names the scenario's scenes; 28016 was free).
- `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py`: 22 of 22 pass, wall 107.3 s (ui_tour 38.0 s and
  properties 7.0 s beside three others; alone 29.6 s and 4.9 s).
- `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 07:17:18'` after the last run
  (09:33:34): nothing. No Grizzly, LevelEditor, xrEngine, XrLC or github_mock process left; the temp copies, the
  game roots (their archive junctions unlinked first, the game's 43 archives checked there), the probe roots and
  the compilers' stages removed; `<repo>\gamedata\levels` does not exist; `D:\Games\Dead Air\_qa` as it was.
- Open: a `grizzly_editor.ltx` with a `[window]` section (written 09:32:27, during `run_all.py`) appeared in the
  repository root; removed. The suites put `qa_target.KEPT` back only when the file was there before, so a file
  that was not is left; the suite whose editor wrote it was not identified in this pass.

Decisions taken in this pass:
- Play's game root copies the game's executables instead of hard-linking them: a link would share a file of the
  install with whatever the root's removal does (the read-only mark) - 250 MB once per root.
- The compilers' stage copies 71 MB of `Bin\x64\Release` once per project.
- The in-game run drives the editor on a hidden desktop (the game repository's
  `Start-DetachedHiddenDesktopProcess.ps1`, used as it is): the game Play starts inherits it.
- A test that wrote files named like the editor's compilers was refused execution by the machine's on-access
  scanner ("Access denied", os error 5, at every relink); the stage's test checks the pure `compiler_fsgame`.

## Play ends in the game

The brief `docs/rust/briefs/play-fix.md`, 12 Nov. Before the first run (09:46:17): `find D:\XFinedProjects
"D:\Games\Dead Air\modules" -newermt` ten minutes earlier - nothing; no editor or game running, 28016 free.

**1. The game loads the level Play made.** Three faults, each found in a file read back or a crash report, in
this order:
- **The actor was not the player.** `M_SPAWN_OBJECT_ASPLAYER` (bit 3 of `s_flags`) was missing from the actor's
  packet in `editor.spawn`. The game takes the player from that mark alone (`CALifeGraphRegistry::update`, xrGame
  `alife_graph_registry.cpp`): without it `m_actor` stays null, `setup_current_level` never runs, and the actor's
  `on_register` meets no current level - the crash of the last pass. The C++ sets the mark when a spawn point is
  made, from the section's `$player` (`CSpawnPoint::SSpawnData::Create`, `SpawnPoint.cpp`); `Spawn_Write(.., TRUE)`
  keeps it. The Rust `place_spawn` made the entity with the factory alone (`Entity::create`). Now
  `Entity::create_spawn_point` does what `SSpawnData::Create` does, and `build_spawn` marks the one actor whatever
  the scene saved (a scene placed before this fix keeps the factory's entity). The game's `all.spawn` read with
  the repository's reader is the model: its one actor carries the mark and `[actor] $player = on`
  (`gz-entities/tests/corpus.rs`, `the_actor_of_all_spawn_is_the_player`).
- **Dead Air's start scripts name vertices of the game's graph.** With the player in, the new game was made
  ("New game is successfully created!"), the level loaded, and the actor's first update stopped in
  `CALifeGraphRegistry::add` from `alife():create`: `xr_actor.actor_on_first_update` spawns repair kits and a kit
  on fixed game vertices of the base world (643, 2772, 3616, 4147, 451, 698, 1892) unless the level is
  `fake_start`. A one-level graph has one vertex. Decision: Play's `editor.spawn` holds the level's own graph
  (as the C++ builds it, unchanged) appended to the linked game's (`graph::join_base`), the game's own rule for a
  module's level (`XmsGame::ComposeGameGraph`): base vertices, edges, death points, cross tables and guid as the
  game has them, the level under id 128 (the first module id, `XMS::LevelId`) at offset (10000, 0, 10000), its
  vertices, edges and cross table numbered after the game's. `build_spawn` finds its level by name in the graph
  (`game_graph().header().level(name)`, as `CLevelSpawnConstructor` does) instead of taking level 0. A scene
  named like a level of the game keeps the one-level graph and logs a warning (the composer adds no base level
  either). The `level_count` of the header stays 1: the levels whose objects the file holds.
- **The root's depth.** Under `run_all.py` (a suite's temp folder is deeper) the game stopped 1.5 s after its
  start with no log line; its crash report (`session_reports\*.zip`, symbolised with the game's own pdb through
  dbghelp): a breakpoint in xrCore from the CRT, `CLocatorAPI::Register` (`LocatorAPI.cpp:330`) copying a path
  into a `string256`. The game indexes its own shader cache, whose longest name is 188 characters, so a root over
  66 characters cannot start. `play::check_path_lengths` refuses such a root naming the path; `test_play_game`
  puts its root in a temp folder of its own (`gzpr_*`), `run_all.py`'s suite folders are `gzqa_*`.

The invariants the game's loader relies on are a unit test of the builder (`spawn.rs`,
`check_spawn_file`, read back with `gz-fmt-spawn` and `gz-entities`): header version, spawn counts, graph guid,
level count; the graph's counts and records; the level in the table with the AI map's guid; one cross table per
level, its guids, its node count equal to `level.ai`'s (read back from the bytes written); every vertex's level in
the table, its edges, its level vertex given back to it by the cross table; every spawn's id, its game vertex on
the level, its level vertex leading to it; one actor, the player, under direct control. Three tests: a level alone,
a level joined onto a two-level world, an actor without the mark. `graph.rs`
`a_level_is_appended_to_the_linked_games_graph_with_its_ids_unchanged`; `play.rs` the path limit.

**The proof** - `tools/qa/test_play_game.py` (opt-in: `run_all.py --with-game`, or named): PASS 32 / 0 through
`run_all.py test_play_game` (50.7 s). The game's log, 26 s after Play: `* New game is successfully created!`,
`# New player created.`, `* client : connection accepted - <All Ok>`, `* ObjectSpace cache for
'appdata\cdb_cache\mp_test\objspace.bin' ... Building the model from scratch`, `* [texload] geometry level.geom`,
`* [ground] collision mesh: 24 triangles, 15 vertices`, `* End of synchronization A[1] R[1]`, then
`[ALife]: objects[110], online[4]` (the start kit spawned); no assert, no fatal error, no dump; the game still
running 31 s after Play, ended by Play's stop (the suite checks no process of the root is left), the root's archive
link unlinked before the root went. The log: `Intermediate/qa/play_game/engine5.log`.

**2. No editor writes into the repository root.** `grizzly`: without `-home` the home is `GRIZZLY_HOME`, then the
data root (`Cli::home`, test `the_home_comes_from_the_command_line_then_the_environment_then_the_root`).
`qa_target` sets `GRIZZLY_HOME` for every process a suite starts, `run_all.py` for every suite, `diff_editors.py`
for every Rust editor. `run_all.py` reads the root's `_projects.ini` and `grizzly_editor.ltx` before the run and
fails it when either changed or appeared ("repository root written"), putting it back. Both `remove_tree`s of the
QA (`qa_target`, `run_all`) unlink every junction and symbolic link inside first (a name-surrogate reparse tag),
so a suite ended by its timeout cannot take the game's archives with its root (checked on a scratch junction).

The final runs of this pass (12 Nov, 10:02-10:15):
- `bash tools/rust/check.sh`: ok.
- `bash tools/rust/test.sh` (default mode): 17 binaries run, 58 cached, 0 failed, 0 skipped; 1099 tests green.
- `python tools/qa/diff_editors.py` (default mode): 10 scenarios, 755 steps, 0 differ; 746 written files, 0 differ
  (`s9_play` 9 / 14, `s7_levelbuild` 4 / 17, `s8_levelbuild2` 2 / 19); 42.3 s.
- `GRIZZLY_QA_TARGET=rust python tools/qa/run_all.py`: 22 suites, 21 passed in the run (108.2 s); `test_ui_shell` A
  failed its 0.3 s budget for a shown window at 0.357 s with four suites beside it, and passed alone right after
  (12.1 s). Open: that budget is a timing check under the run's parallel load. No "repository root written".
- `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 09:36:17'` at 10:15:24: nothing. No
  Grizzly, xrEngine, XrLC or github_mock process left; no `gzpg_*`, `gzpr_*`, `gzqa_*` or `grizzly_*` folder in the
  temp folder; the game's 43 archives in place; no `grizzly_editor.ltx` and no `gamedata\levels` in the repository.

Decisions taken in this pass:
- Play's graph is the linked game's with the level appended (above); the C++ one-level graph stays for a level
  of the game's own name.
- `build_spawn` marks the actor `ASPLAYER`; `place_spawn` marks a `$player` section's new entity as the C++ does.
- The root depth is refused, not worked around in the game (the game repository is read-only here).

## The cutover

The brief `docs/rust/briefs/cutover.md`, 12 Nov. The last commit with the C++ editor is the tag `cpp-editor-last`.
Before the first run (10:28:36): `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 10:18:35'` -
nothing; no editor, game or compiler running.

**1. What left the repository.** The closure of the three compilers was taken from the build itself, not from
names: the compilers built from a fresh CMake folder of the full tree (`Intermediate/cutover/closure`, 445 steps,
72 s), then ninja's graph (`ninja -t inputs XrLC xrDO_Light XrLCLight`) and its dependency records (`ninja -t
deps`: every header each object read) gave the files, the link lines of `build.ninja` and the `#pragma
comment(lib)` of those files the libraries (from the SDK: `d3dx9.lib`, `DxErr.lib`; the rest is the Windows SDK's).
Two kinds of input no record holds were added by hand and proven by the second build: `ISPC/win/ispc.exe` (the
command of a custom rule) and what a `.rc` includes (`Editors/Public/grizzly_resource.h` - the first proof build
stopped there and it was put back from the tag; `assets/grizzly-editor-icon.ico` stays at the root). The license
texts of the third-party code that stays stay with it. `Intermediate/cutover/closure.py`, `plan_delete.py`,
`kept.txt`, `dropped.txt`.
- Deleted (16,047 tracked files): under `Source/` everything but the closure - `Editors/LevelEditor`, `XrECore`,
  `XrEUI`, `XrEProps`, `ActorEditor`, `ParticleEditor`, `ShaderEditor`, `XrAI`, `XrFontGenerate`, the three
  `XrSE_Factory*`, `XrGame`, `XrGameCS`, `XrGameSOC`, `XrGameSpy`, `XrNetServer`, `XrPhysics`, `XrParticles`,
  `XrXMLParser`, `XrCPU_Pipe` (but the header the compilers read), `XrSound` (but `Sound.h`), `XrRender` (but
  `Public` and one header), `XrEngine` (but 40 headers and `xrLoadSurface.cpp`), `xrServerEntities*`,
  `XrWeatherEditor*` (but one header), `Xr3DA*`, `TestApp`, `packages`, `Engine.sln`, every `.vcxproj` /
  `.filters` / `.props` (the pipeline is CMake), `External/{BugTrap, Crypto, FreeType, LuaJIT, Luabind, Ode,
  OpenALSoft, OpenAutomate, OpenSSL, Theora}` and what `External/Public`, `RedImage` (its MSBuild junk, the
  generated ISPC headers - now written into the build folder), `LZO` held beyond the build; under `SDK/` all but
  12 DirectX headers, 8 hxgrid headers, `stb_image.h`, `library_linkage.h` and the two libraries (`boost`, `loki`,
  `freetype`, `DPlay`, `Lib/Win32`, `Bin`, `LuaJit.7z`); at the root `fs_cs.ltx`, `fs_soc.ltx`, `fsgame_cs.ltx`,
  `fsgame_soc.ltx`, `actor.ini`, `particle.ini`, `shader.ini` (the dropped flavours and editors; no Rust code
  reads them), the four `tool_*xrAI*` / `tool_create_spawn` / `tool_verify_ai_map` launchers of the dropped
  standalone XrAI; `cmake/{EditorTargets, GameTargets, GameScriptTUs, LuaJITTarget}.cmake`, `cmake/LuaJITHost`;
  `tools/qa/lint_ui_units.py` (D6).
- Stays for the compilers (1,075 files under `Source/` and `SDK/`): `Editors/{XrLC, XrLCLight, xrDO_Light,
  XrDXT, XrETools, XrQSlim, FreeMagic}` and seven files of `Editors/Public`, `XrCore`, `XrCDB`, `XrAPI`,
  `External/{zlib, LZO, Ogg, Vorbis, RedImage, Public (30)}`, `xr_msvc_stl_compat.h`, `THIRD_PARTY_NOTICES.txt`
  (rewritten for what is built now: LZO, Ogg/Vorbis, zlib/MiniZip, and LuaJIT/libvorbis inside the editor).
  `XrETools` stays: `XrLC` links it.
- Left, as the rules of this pass require: `rawdata\{actor,particle,shader}_imgui_default.ini` and
  `rawdata\ActorEditorLevel.cform` (only the C++ editors read them; nothing under `rawdata\` is deleted in this
  pass), and at the root `T-6638.7z` (an archive of old X-Ray sources), `ccc__temp__vertices`, an empty
  `engine.log`, `tool_compile_xrLC.cmd` / `tool_compile_xrDO.cmd` (naming a `Bin\Win32` that no build makes) - none
  is named by the brief's section 1. The untracked MSBuild output under `Source/External/RedImage/{intermediate,
  libraries}` was left; `intermediate` was ignored already, `libraries` is ignored since the reviewer's check.
- The repository: 18,201 files tracked at the tag, 2,157 now (2,155 kept, `cmake/CompilerTargets.cmake`, `crates/grizzly/build.rs`); C++ 3,270,206 lines at the tag, 338,302 now (21,607 of them SDK headers); Rust 246,592 (`Intermediate/cutover/size.py`).

**The compilers build from what is left**: `pwsh -NoProfile -File .\Build.ps1 -Only compilers -Out
Intermediate\cutover\proof-bin` - a fresh CMake folder, LTO on as shipped: 80 s (the first run stopped at the
`.rc`'s header after 441 of 445 steps, the second finished the 5 left). `XrLC.exe`, `xrDO_Light.exe`,
`XrLCLight.dll`, `XrCore.dll`, `XrCDB.dll`, `XrAPI.dll`, `XrDXT.dll`, `XrETools.dll`. The proof did not write into
`Bin\` (`-Out`): there the compilers of the last C++ build stay, and Play runs them.

**2. The editor under its own name.**
- `Build.ps1` builds the editor first (cargo with the settings of `cg.sh`, outside the Visual Studio shell: inside
  it the C dependencies' build scripts see another environment and rebuild, 3.5 min each time) and then the
  compilers (CMake in the developer shell, the newest MSVC with `<hash_map>`, the newest ninja found), into
  `Bin\x64\Release` (`-Out` elsewhere, `-Only editor|compilers`, `-Debug`), and prints each part's seconds. It
  copies `Grizzly.exe`, `grizzly-mcp.exe` and their PDBs; `LevelEditor.exe` and its DLLs were not touched (no
  DLL was added, so the oracle's stamp of the folder is what it was).
- **The release profile** stays thin LTO, 16 codegen units, `debug = "line-tables-only"` (the PDB beside the
  executable is what `std`'s backtrace in `gz-crash` reads). It had never been linked: the link failed on
  `__imp_realloc` - the vendored LuaJIT (`luajit-src` runs `msvcbuild.bat static`, /MT) pulls the static C runtime,
  the rest of the program is built for the DLL one, and under thin LTO the static one is met first.
  `crates/grizzly/build.rs` leaves the static runtime out of the executable's link (`/NODEFAULTLIB:libcmt.lib`,
  `libucrt.lib`, `libvcruntime.lib`): one C runtime, the DLL one, as the debug build already used
  (`vcruntime140.dll`, the UCRT). A clean release build of the editor and the bridge: 347 s; after a change of the
  `grizzly` crate alone 164 s; Build.ps1's run 220 s. `Grizzly.exe` 71.6 MB, its PDB 250 MB.
- The default port is 28016 (`gz_mcp::DEFAULT_PORT`, the editor and `grizzly-mcp`); QA keeps 28917 and up.
- `tools/mcp/grizzly_mcp.py` is a launcher of `grizzly-mcp.exe` (`GRIZZLY_MCP_EXE`, `Bin\x64\Release`,
  `Intermediate\cargo\release`, `Intermediate\cargo\debug`; same stdio, the child's exit code, a sentence and exit
  code 1 when none is built). `test_mcp_bridge.py` holds the safety paragraphs as the Python bridge wrote them, word
  for word (checked against the tag), and takes the bridge beside the editor it drives.
- QA: `qa_target.py` drives the Rust editor with nothing set (the debug build; `GRIZZLY_QA_EXE` another binary,
  which `run_all.py`, `diff_editors.py`, `check.sh`'s registry and `run-editor.ps1` take too); `GRIZZLY_QA_TARGET=cpp`
  is accepted from `diff_editors.py` alone, any suite refuses it with a sentence, an unknown value too.
  `diff_editors.py`'s key of the C++ editor is the stamp the folder had at the cutover (`CPP_STAMP`): the sources
  are gone, so it cannot change, and Build.ps1 now rebuilds the compilers' `XrCore.dll` / `XrCDB.dll` beside
  `LevelEditor.exe` from the same sources - that must not throw the records away.
  `tools/mcp/run_benchmark.py` and `tools/qa/bench_build.py` still measure the C++ editor against the Rust one
  (records and an opt-out, `--no-cpp`); the other `tools/mcp` scripts were never the C++ editor's alone.

**3. The documents.** `CLAUDE.md` rewritten rule by rule ("The move to Rust" became "How the editor is built");
`tools/mcp/AI_SETUP.md` (the binary, the port, the catalogue, how a project and its last scene open, the raw
protocol as the Rust server runs it; the safety section untouched); `README.md`; `docs/UI_DESIGN.md` (the kit is
`gz-ui`, the contract `gz-app`'s `contract.rs`, section 8 is where it lives now; the C++ plan is at the tag);
`docs/NQ_ARCHITECTURE.md` (its C++ sections say they are the tag's tree). A Rust comment that cited a deleted
`Source/...` path cites `cpp-editor-last:Source/...`. Two tool descriptions named C++ files (`ui_split`,
`shortcuts`: `level_imgui.ini`, `level.ini`; `ui_contract`: `EditorUiContract.h`) and now name the editor's own.
References to deleted paths left in what stays (`Intermediate/cutover/deleted_refs.py`): the game engine's own
sources the documents cite (`PhraseDialog.cpp`, `CustomOutfit.cpp`, ... - all in the game repository's
`src\xrGame`), and the C++ file names inside the two sections of `NQ_ARCHITECTURE.md` marked as the tag's. The
migration's records (`docs/rust/briefs`, `PARITY.md`, this file, `RUST_MIGRATION.md`) keep the paths they were
written with: they name the tree at the tag.

CLAUDE.md rules dropped, and why (every other rule carried over in the new editor's terms):
- "`Source/` is the oracle and is not edited on this branch", `GRIZZLY_QA_TARGET=rust` - the migration rule is over;
  `Source/` holds the compilers.
- The modal pin of the patched `imgui.cpp` (`BeginPopupModal`), ImGui's platform windows for a window that leaves
  the viewport, the stale `level_imgui.ini` placing a modal, `NoSavedSettings` - ImGui; the behaviour (a dialog in
  the main window, centred, clamped, scrolling) carried over.
- `XrUIManager::NewPlatformFrame` / `SetRenderSize` / `CEditorRenderDevice::Initialize`, and "unless another
  window of the editor is in front" - the C++ minimized-picture code; the Rust window pictures a minimized editor at
  its restored size (`window.rs`), the in-front exception has no counterpart.
- `test_mod_catalog.py` F's stale layout entry and "the layout and preferences files come back byte for byte" - the
  Rust shell keeps no ImGui layout; the suite skips that precondition.
- The splash thread of `EditorStartup` (D7), its `Stage()` / `Finish()`, the C++ start numbers (19k reads, 62k
  file operations), the `CPU::Detect` and `LoadTHM` lessons, the Per-Monitor V2 opt-in of `DpiScale()`, "a frame
  drawn before it loads them itself" - C++ startup code; the profile and its rules carried over.
- `XrEUI/XrUIKit.h`, `GrizzlyTheme::U32`, `XUI::Pt` / `Em` / `SetDpi`, `lint_ui_units.py` and `// ui-units: ok`
  (D6), xrCore's `FLT_MIN`, `ImGui::MenuItem` vs `XUI::MenuItem`, `XUI_Local<Name>`, "the 20 pt logo is the only
  bitmap left" - the C++ kit; the kit rules carried over as `gz-ui`'s.
- `XrUIShell`, `XUI::Shell::Open`, `DocDesc`, `KeepAliveOnly`, `[ui] level_layout_custom`, `[ui] layout_version`
  and `level_imgui.ini` - the C++ shell and its ImGui files (D3: nothing migrated).
- "`BlockShortCuts` is gone" - C++ history.
- `FromMcp()` / `Unattended()` / `ELog.DlgMsg` - now `gz-ui`'s `Asker` (`Ui`, `Mcp`, `Unattended`).
- Gone-for-good items that were C++ files (`ed\bar`, `spectrum.*`, `rawdata/level_imgui_default.ini`) and the
  render target's 64 px buckets - C++ renderer details.
- Start page: `XUI::BeginPage` / `BeginPageCard` / `CardFooter`, `EditorRecovery::OfferPending`, `SetPreviewScope`,
  "never inside the UI pass", the deferral of Switch Project out of the UI pass, `s_DeadPreviews` (an ImGui texture
  kept for the frame being drawn), `Close()` writing `last_scene` - C++ mechanics; the behaviour carried over.
- `GetFinalPathNameByHandle`, `IsReparseTagNameSurrogate`, `EditorFileOps::Delete` / `Rename` - names; the checks
  carried over (`gz-project`'s `delete.rs`, `ops.rs`; `gz-win`).
- Builds: `TUI::Idle` running the checks a slice per frame and "the descriptor on the main thread" - the Rust job
  runs the checks off the UI thread; `FollowsJob`, `SBuildWalk`, `EditorMod::*` names.
- Releases, catalog, characters, quests, behaviour graphs, the world, PDA, screens: the C++ class and function
  names (`EditorMod::PackageRelease`, `kPartInputLimit`, minizip's `ReadPackageLayout`, `EditorCatalog::DrawKeyLine`,
  `kClientId` in `EditorGithub.cpp`, `CreateKey`, `CheckStagedContent`, `TagVocabulary`, `VtPace`, `kOrder`,
  `lvRequired`, `SCatalog*`, `XUI::BeginCard` / `FooterBusy` / `BeginSheet`, `EditorCharacters::*`, `SNqValue`,
  `TextRaw`, `NqInspector::*`, `NqPickers::*`, `SetModelPreview`, `[GrizzlyLayout]` and the old `level.ini
  quest_*_split` keys, `NqLayout::Box`, `kZoom * XUI::Dpi()`, `NqLook::*`, `EditorMod::SManifest::vars`,
  `ProjectGlobals`, `NqCatalog::kTypes`, `NqValidate::CheckParamValue`, `EditorWorld`, `ptPlayerStart`,
  `ManifestGeneration`, `SXmsOrder`, `EditorPda`, `DrawItem`, `Toolbar::Segmented`, `ValidateForBuild`,
  `EditorScreens`) - replaced by the crate and file that hold it now; "Until the implementation starts, the
  document IS the contract" - the implementation started long ago.
- Screens: `XUI::FileDropZone` (`XrUIFileDrop.cpp`), `WM_DROPFILES`, the multithreaded apartment where
  `RegisterDragDrop` fails, `IsEditorProcessForeground` while `FileDropActive` - C++ / COM details.
- Dangerous operations: "deleting scene objects, clearing the scene" as confirmed operations - D8 makes them one
  undo step that asks nothing (the Outliner's Delete still names what goes); the rule says so now.
- Code and text: "From any ImGui panel's `Draw()` never call anything that redraws the frame" (`XrUIManager::
  InUIPass`) - ImGui; the twins of `Source/XrRender/Private` and `Source/Editors/XrECore/Editor` - gone; "the
  editor process speaks UTF-8" (`LevelEditor.manifest` `activeCodePage`, the profile's `file names: UTF-8` line) -
  the Rust editor talks UTF-16 to the file system, no code page is involved (the rule became "a file name is one
  string"); the pointer rule now applies to the compilers' C++.
- Icons: `EditorAssetIcons::KindOf` / `IconOf` (joining the project root first), the generation `Invalidate()`
  bumps - C++ implementation details; `XUI::Icon::Quest` etc. - names.

**4. Proof.** Once, on the final tree, 11:19-11:27, nothing else running:
- `GRIZZLY_QA_EXE=Bin/x64/Release/Grizzly.exe bash tools/rust/check.sh`: clippy clean, fmt clean, `unsafe` only in
  `gz-win` (28), one file-dialog function, the release binary registers every catalogue command; 20 s.
- `bash tools/rust/test.sh --all`: 75 binaries run, 0 cached, 0 failed; 1099 tests green; 81 s.
- `GRIZZLY_QA_EXE=<release> python tools/qa/diff_editors.py --all`: 10 scenarios, 755 steps, 0 differ; 746 written
  files, 0 differ; 43.0 s, all from the records (no C++ editor started; the fingerprint is the release binary's).
- `GRIZZLY_QA_EXE=<release> python tools/qa/run_all.py --with-game`: 23 suites, 23 passed (`test_ui_shell`'s 0.3 s
  window budget among them, `test_play_game` 57.6 s: the game loaded the level Play built with the compilers of
  `Bin\x64\Release`); wall 168.7 s.
- Release against debug, from the same source (`Intermediate/cutover/bench_rel.py`: each minimised, a copy of Test
  and of Overnight Delivery): the endpoint answers 0.58 s / 0.58 s after the process starts, the first held
  screenshot 1.61 s / 1.98 s; Escape's load (6006 objects, median of 3) 201 ms / 258 ms, ready 215 ms / 259 ms;
  Overnight Delivery's clean module build (204 files) 1.05 s / 1.38 s, the rebuild 0.31 s / 0.31 s.

At the end: `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 10:18:35'` at 11:28:11: nothing. Under `Bin\` only the four new files (`Grizzly.exe`, `Grizzly.pdb`, `grizzly-mcp.exe`, `grizzly_mcp.pdb`); nothing under `rawdata\`, `gamedata\` or `Intermediate\oracle` written. No Grizzly, LevelEditor, xrEngine, XrLC, xrDO_Light or github_mock process left; no `gzpg_*`, `gzpr_*`, `gzqa_*`, `grizzly_*` or `gzbench_*` folder in the temp folder.

Decisions taken in this pass:
- The closure is file by file (ninja's records), not folder by folder; dead sources inside the kept folders (the
  `EXCLUDE` lists of the CMake files, the `.vcxproj` files) went, and so did the `EXCLUDE` lists.
- One build pipeline for the C++ (CMake via Build.ps1); `Engine.sln` and every project file went.
- Build.ps1 builds the editor outside the developer shell (the same environment as `cg.sh`).
- The release executable links the DLL C runtime alone (`crates/grizzly/build.rs`).
- `diff_editors.py`'s C++ stamp is frozen at the cutover's value.
- `check.sh` / `test.sh` are code-level: they ran once on the final tree; the release binary is what `check.sh`'s
  registry check, `diff_editors.py` and `run_all.py` drove (`GRIZZLY_QA_EXE`). `test.sh` in the release profile
  would rebuild all 75 test binaries with thin LTO.
- What the rules of this pass kept from deletion (`rawdata\*_imgui_default.ini`, `rawdata\ActorEditorLevel.cform`)
  and the root files no section names are left for the lead.

## Phase 8a: the compiler's model and light

`crates/gz-lc-light` (layer 4: `gz-task`, `gz-chunk`, `gz-fmt-level`, `gz-fmt-tex`, `gz-fmt-xr`), 6.9k lines, 38
tests run green (`cg.sh test -p gz-lc-light`, 0.6 s). Brief: `docs/rust/briefs/lc-light.md`.

**What it holds, module by module (the C++ it ports):**

| Module | C++ |
|---|---|
| `math` | `xrCore`'s `iFloor`/`iCeil` (from `gz-fmt-level`), `exact_normalize`, `normalize(_safe)`, `from_bary`, `Fmatrix` (`build_camera`, `transform_tiny/_dir`, 4x3 `invert`, `transpose`), `Fbox2`, `getsphere`, `CRandom` + `random_dir`, `u8_clr`, colours |
| `color` | `base_color_c`, `base_color` (`fixed16<16>`) |
| `data` | `Shader_xrLC_LIB`, `post_process_materials`, `shader()`, `b_BuildTexture`, the texture part of `CBuild::Load` / `global_claculation_data::xrLoad` (`prepare_texture`), `HasAlphaChannel` |
| `light` | `R_Light`, `base_lighting` (`select`), the lights of `CBuild::Load`, `build.lights` read/write |
| `mesh` | `MeshStructure.h` (`Tvertex`/`Tface`), `DataVertex`/`DataFace`, `base_Vertex`/`base_Face`, `base_basis`, `_TCF`, `xrFace.cpp`, `xrFaceInline.h`, `MeshStaic.cpp`, `calculate_normals.h`, `itterate_adjacents*.h`, `face_smoth_flags.h`, the error writers of `err_save` |
| `hash2d` | `hash2D` |
| `layer` | `lm_layer`, the blits of `xrDeflector.cpp`, `ApplyBorders` |
| `ray` | the product of `CBuild::BuildRapid` (`RapidCollector`, `RcastForm` = `build.cform`), `rayTrace` + `getLastRP_Scale` of both compilers |
| `point` | `LightPoint` of `xrDeflectorLight.cpp` and of `detail_slot_calculate.cpp` |
| `deflector` | `xrDeflector.cpp`, `xrDeflectorLight.cpp`, `xrDeflectoL_Direct.cpp`, `Jitter_Select`, `rms_*`, `compress_*`, `CBuild::LMapsLocal` / `CLMThread` |
| `lightmap` | `Lightmap.cpp` (`Capture`, `Save` less the DDS), `xrPhase_MergeLM.cpp` + `xrPhase_MergeLM_Surface.cpp` + `xrPhase_MergeLM_Rect.h` (XrLC's, ported here as "the packing") |
| `vertex` | `xrLightVertex.cpp` (and the `g_trans` groups the MU light shares) |
| `implicit` | `xrLight_Implicit.cpp`, `xrLight_ImplicitDeflector.*`, `xrLight_ImplicitCalcGlobs.*`, `xrLight_ImplicitThread.cpp` |
| `mu` | `xrMU_Model*.cpp`, `xrMU_Model_Reference*.cpp`, `mu_model_face.*`, `mu_model_light*.cpp`, `fitter.*` |
| `detail` | `xrDO_Light.cpp`, `compiler.cpp`, `detail_slot_calculate.cpp`, `global_calculation_data.cpp`, `global_slots_data.*` (less the file work), `LightThread.cpp`, `DetailFormat.h`'s slot rules (the slot itself is `gz-fmt-level`'s) |
| `filter` | `xrImage_Filter.cpp` (no caller in the compilers) |
| `global` | `xrLC_GlobalData` (`LcGlobalData`) |

The resampler of `xrImage_Resampler.cpp` is `gz_fmt_tex::imf_resample` (already ported, reused); the DDS writer is
`gz_fmt_tex::encode_dds`; the BVH is `gz_fmt_level::Bvh`, made shareable by two additions that change nothing
else: `Bvh::ray_visit` (a walk over every hit that can stop) and `ray_triangle` (the walks' own triangle test).

**Not ported:** the network light (D24): `net_*` (38 files), `lcnet_*` (4), `mu_light_net.*`, `xrLightDoNet.*`,
`xrLightVertexNet.cpp`, `xrLight_ImlicitNet.cpp`, `recalculation.*` (the resumed / partial detail light over files;
its partial square survives as `light_details(only)`), `lc_net_global_data.*`, `lm_net_global_data.*`,
`implicit_net_global_data.*`, `detail_net_global_data.*`, `ref_model_net_global_data.*`, `gl_base_cl_data.*`,
`net_cl_data_prepare.*`, `net_global_data_cleanup.*`, `serialize.*`, `net_stream.*`, `file_compress.*`,
`net_task*.*`, `light_execute.*`, `lightstab_interface.h`, and the serialisation halves (`read`/`write`) of every
class. `-gi` radiosity is XrLC's `xrPhase_GI.cpp`; its `LT_SECONDARY` lights are still lit by `light_point`. Also
gone as infrastructure: `xrThread.*` / `CThreadManager` (rayon), `cl_log.*` and the `clMsg`/`Status`/`Progress`
log of `xrLC_Light.cpp` (`Ctl`), `tga.*` (`gz_fmt_tex::write_tga`), `xrDXTC.h`, `execute_statistics.*`,
`vector_clear.h`, `xrIsect.h` (included, never called), `_d3d_extensions.h` (`gz_fmt_level::Light`),
`ETextureParams.*` (`gz_fmt_tex::TextureParams`), the DLL / exe entries and resources.

**The public API the pipeline (`gz-lc`) calls, against the C++ call it replaces:**

| Rust | C++ |
|---|---|
| `LcGlobalData::new(params, shaders, materials, textures, lights)`, `nosun`, `linear` | `create_global_data`, `b_nosun_set`, `initialize` (`-att`) |
| `LightSet::from_project(controls, statics)`, `RLight::from_flight` | `CBuild::Load`, "Loading lights" |
| `prepare_texture(raw, thm, TextureUse::Level{..}, load)`, `post_process_materials` | `CBuild::Load`, "Processing textures", "Post-process materials" |
| `LcGlobalData::load_mesh(&prj)`, `load_mu(&prj)` | `CBuild::Load` vertices, faces, smoothing groups, MU models and references |
| `Mesh::create_vertex/create_face/set_vertex/destroy_face/destroy_vertex/v_replace/...`, `Mesh::faces_mut()` | `create_vertex`, `create_face`, `SetVertex`, `destroy_*`, `VReplace`, `g_faces()` / `g_vertices()` |
| `Mesh::verify_face`, `MeshErrors::write_err` | `Face::Verify`, `InvalideFaces`, `CBuild::err_save` |
| `Mesh::isolate_vertices` | `CBuild::IsolateVertices`, `vertices_isolate_and_pool_reload` |
| `LcGlobalData::calc_normals(Smoothing)`, `mu_calc_normals` | `CBuild::CalcNormals`, `MU_ModelsCalculateNormals` |
| `LcGlobalData::light_prepare` | `CBuild::Light_prepare` (`CacheOpacity`, `calc_faceopacity`) |
| `LcGlobalData::build_rapid(&bounds)` -> `RcastForm` (`.write()` is `build.cform`) | `CBuild::BuildRapid`, `create_rcmodel` |
| `LightSet::write_build_lights` | `CBuild::Run`, "Saving lights" |
| `Deflectors::create`, `Deflector::oa_set_normal`, `oa_unwarp`, `oa_place`, `oa_place_all`, `oa_export`, `remap_uv`, `base_material` | `xrPhase_UVmap` / `xrPhase_Subdivide`'s `new CDeflector`, `OA_SetNormal`, `OA_Unwarp`, `OA_Place`, `OA_Export`, `RemapUV`, `GetBaseMaterial` |
| `LcGlobalData::light_mu(ctl)` | `run_mu_light`, `wait_mu_base`, `wait_mu_secondary` |
| `LcGlobalData::implicit_lighting(ctl)` -> `ImplicitMap` (`.dds()`) | `ImplicitLighting` (with its two DXT writes) |
| `LcGlobalData::light_deflectors(ctl)` | `CBuild::LMaps` / `LMapsLocal` |
| `LcGlobalData::light_vertices(ctl)` | `LightVertex` |
| `LcGlobalData::merge_lightmaps(ctl)` -> `LightmapPage` (`.dds()`, `.debug_tga()`) | `CBuild::xrPhase_MergeLM` (with `CLightmap::Save`'s DXT writes) |
| `LcGlobalData::light(ctl)` | `CBuild::Light` |
| `MuReference::export_cform_game` | `xrMU_Reference::export_cform_game` (`CBuild::BuildCForm`) |
| `LcGlobalData::has_implicit_lighting`, `shader` | `DataFace::hasImplicitLighting`, `base_Face::Shader` |
| `RcastForm::read`, `LightSet::read_build_lights`, `DetailScene::new`, `light_details(scene, slots, only, ctl)` | `xrCompileDO`: `gl_data.xrLoad`, `xrLight` / `LightThread` |

`Ctl` is the cancel token and the progress of a job (`Ctl::new(&cancel, &report)`); every long call checks it and
answers `LcError::Cancelled` when stopped.

**Decisions (each also a line in the code):**

- **Same light on any number of threads.** The C++ keeps, in each `R_Light`, the triangle that last shadowed it -
  shared state that makes a texel depend on which texel came first, and a data race in vertex, implicit and MU
  light, which use the global set. Here the cache is the worker's (`LightCache`), tested with the BVH's own
  triangle test and never for the skipped face, so it only saves time. The transparent hits of a ray are multiplied
  nearest first (the C++: its tree's order). Proven by `the_same_light_on_one_thread_and_on_eight`: a scene with
  every light kind, an alpha occluder, MU, implicit, vertex light and details, compiled whole on 1 and 8 threads,
  equal bytes.
- **Random numbers from the sample**: the MU model's six jittered normals per vertex come from a `CRandom` seeded by
  the vertex's world position (the C++: the global `::Random`, shared by its threads). `LT_SECONDARY`'s jitter is
  dropped: the C++ moves the light and then traces along the direction taken before, so it changes nothing.
- **Orders the C++ took from pointers or `random_shuffle`** (D14): the deflector list is not shuffled; merge-sort
  ties stay in list order; the vertex groups (`g_trans`) are filled in list order (the C++: as its threads finish);
  a vertex's adjacency is sorted by face id; `calc_materials` sorts stably; `export_cform_game` takes faces in model
  order; the detail ground search takes the box's triangles by index.
- **Plainly broken C++, the intent ported**: `Shaders::get` of a material without a compiler shader gives the default
  shader (the C++ indexes the library at 65535); the merge's SIMD placement test took a texel marked exactly 253 as
  free, the scalar part as taken - taken everywhere; `RunImplicitMultithread` left the last `height % 8` rows unlit -
  all rows are lit; the detail compiler kept a description's size beside a surface of another size - both compilers
  take the surface's; `g_trans_register`'s walk back from `end()` read the end node - the window of keys is searched.
- **Kept as the C++ does it (`// oracle:`)**: `o_test(4, ...)` fits each channel against the model's sun (0 in a
  model's own light) after the regression took its hemisphere; `Face::Verify` tests the area's validity twice; the
  `-att` point light ignores its shadow; `U %= dwWidth` takes a negative texel coordinate as unsigned.
- `build.cform`'s triangle word is the triangle's index (the C++ writes the low half of a face pointer; nobody reads
  it).

**Proven by the crate's tests** (in `src/tests.rs` unless named): a lit and a shadowed texel of a deflector with the
values `L_Direct` gives (sun 0.5 -> 1023, sky 0.25 -> 511; 0 and 0 under a roof; 511 on the shadow's edge);
`LightPoint` through an alpha-tested occluder and an opaque one with the formula worked out from the C++ lines; the
hemisphere's lights counted by their side; `compress_Zero` of a free floor, its capture into page `lmap#1`, the
faces' second channel, the 16-ring border of the page, the two DDS; the packing of six deflectors without overlap
and a turned one (`lightmap.rs`); `compress_RMS` shrinking a gradient and not noise (`deflector.rs`); vertex light on
a two-triangle floor (attenuation per corner, hemisphere kept, translucency mixed away); the implicit light's alpha;
an MU model's own light, its reference's and the fit; a 2 x 2 detail slot grid, one slot roofed, and the four
corner densities of a part deciding its object; the border growth, the rotated blit, the hash grid, the matrix,
`exact_normalize`, `CRandom`, the regression, the formats `build.lights` and `build.cform`; a stopped compile.

**What waits for the comparison with the C++ compiler's output** (the next pass, on a real `build.prj`): the
lightmaps and vertex colours within a stated error (the BVH's triangle test is not Opcode's, `sin`/`cos`/`tan`/`pow`
are Rust's, the transparencies multiply in another order); the page layout (the D14 orders above move deflectors);
the MU colours (other random directions).

## For the plan

What `docs/RUST_MIGRATION.md` (and CLAUDE.md where named) should say after the reviewer's pass; the lead edits
them.

- **Section 9, the comparison rules of the level exports and the level build** (`diff_editors.py` `FILE_RULES`,
  the `cmp_*` functions). Byte for byte unless named here; no broad ignore:
  - `overlay.xcform`, the cut: byte for byte except the material table's order (D14: the C++ fills it in the order
    a mesh's surface map, keyed by `CSurface*`, meets the surfaces); each triangle's material compared by its name.
    The golden files of a single static are equal byte for byte (`gz-export-level/tests/golden.rs`).
  - the exported `.ogf`: the children (one per surface) as a multiset of their bytes and the collision mesh as a
    multiset of triangles, by the same pointer order; every other chunk byte for byte.
  - `build.prj`: chunks 0, 1, 2, 8, 9, 10 and 15 byte for byte; the material, shader and texture tables as sets of
    what they name (the faces meet them in pointer order); faces as a multiset with the material resolved to what
    it names and the smoothing group, `b_face`'s pad left out; the light set by its fields per light type, a point
    light's colour within 4 ULP (the two compilers round its conversion differently); LOD models in order, corners and UVs within
    2 micrometres (a different float order in the C++ quad code: 8 ULP seen), the LOD material's `reserved` and
    `area` left out (never cleared in the C++); an unset `Flight`'s fields left out (uninitialised); MU models
    per model with their faces as a multiset.
  - `level.spawn`: the Rust build's breakable and climable entries left out (D10); a smart cover's loopholes as a
    set (Lua `pairs` order); every other entry byte for byte. `meshes\brkbl#*.ogf`: Rust only (D10).
  - `level_stat.txt`: every line equal, the maxima within 0.5 % (at least 2): a vertex a rounding away from a 1 m
    cell's border lands beside it; `level_stat.tga` at most one pixel in 10000. Neither file is read by a compiler
    or the game.
  - `build.prj`'s portals: sectors and corner counts equal, corners within 1e-4 m (the corners go onto the
    portal's plane and back in float - `Mgc::OrthogonalPlaneFit`, `build_camera` - with the operations in another
    order; seen 6.1e-5); a portal simplified from more than six corners drops the edge of the smallest triangle
    each round, and an arch's near-equal triangles fall to that rounding (limansk's portal_0047, eleven corners) -
    it passes when the two polygons' areas agree within 2 % (seen 0.55 %).
  - `build.details`: every model's UVs equal up to one shift of its own, everything else byte for byte (D14: the
    textures go into the atlas in the order of their shared-string addresses, sorted by a comparator that is no
    strict order).
  - `build_details.dds`: the header byte for byte; the first level decoded, its 128 px tiles matched one to one,
    each pair within DXT5's error (alpha mean 1, opaque colour mean 8 per channel; seen 0.25 / 5.3); the smaller
    levels are filtered across the pictures' borders and follow the packing (D14, and the DXT5 encoder is
    another: the C++ xrDXT, here gz-fmt-tex).
  With every file equal under these rules the external compilers get the same input, so phase 6's gate holds by
  that; no level was compiled from both outputs.
- **What the differential run compares of the level build** (section 9, 11): Make All on Escape (`s7`) and on
  l10_limansk (`s8`: sectors, portals, details, wallmarks), the Build menu's Make Game / AI Map / Details / HOM /
  SOM one at a time (`s9`). Collision, AI map, game graph, `editor.spawn` and Play are the C++ top bar's buttons
  with no command row: proven by the game (`test_play_game`, "Play ends in the game").
- **D1, Play's game graph**: `editor.spawn` carries the level's own game graph (the C++ `CGameGraphBuilder`'s)
  appended to the linked game's (`spawns\all.spawn`, chunk 4, read only) the way the game appends a module's level
  (`XmsGame::ComposeGameGraph`): the game's vertex numbers and graph guid unchanged, the level under id 128 or the
  first above the game's, offset (10000, 0, 10000), its cross table renumbered. A level the game already has keeps
  the C++ single-level graph (with a warning). Reason: Dead Air's own start scripts (`xr_actor.script`, the actor's
  first update of a new game) spawn items on fixed vertices of the game's graph (643, 2772, 3616, ...); on a
  one-level graph that is an index past the registry (`CALifeGraphRegistry::add`).
- **D1, the game root's depth**: Dead Air's `CLocatorAPI::Register` copies every path it indexes into a
  `string256`; the game's own shader cache (`appdata\shaders_cache_grizzly\r5_dxil\...`) has names of 188
  characters, so a game root longer than 66 characters stops the game at its start (a breakpoint in xrCore,
  before the log opens; the same symptom as the root inside a project folder of the last pass). Play refuses such a root
  with the path that does not fit (`play::check_path_lengths`). It is a limit of the game, to be lifted there (the
  game repository was not to be edited in this pass).
- **D1 as built**: Play runs the LINKED game's engine (`xrEngine.exe`) in a game root of its own outside the
  project (`<appdata>\play\<project>` or `root=`): copies of the executables, a junction to the archives, a copy
  of the loose gamedata with the level, `level.ai` and `editor.spawn` staged, an appdata of its own, no module.
  The compilers run from `<project>\temp\compilers` with an `fsgame.ltx` naming the project's levels.
- **Section 5.9's sentence about `ProjectPath` / `GamePath`** is to be replaced by: "Every writer of the editor -
  the project's file operations, the module build and the flat export, the level exports, the level build, the
  release stage, the compilers' stage and Play's game root - passes `gz_editor::fsx::WriteGuard` before it
  writes: nothing under the linked game's root, judged as written and where the path really is, but the module's
  `modules\<id>` (with its stage in `modules\.new`) and the flat `gamedata_<id>`."
- **Where a level build writes**: into the open project's `gamedata\levels\<level>` (and `gamedata\spawns`), the
  folder the C++ editor's `$game_levels$` / `$game_spawn$` mount names - not `<editor>\gamedata\levels` as the plan
  and the brief said; never into the game.
- **The level path** is one file-name component (`gz_base::file_name_problem`); a wipe of a level folder needs its
  real path strictly below the levels folder and no link in its place.
- **The C++ Make All asserts** (`BuildHOMModel`) on an object in visual mode; the Rust build does not. Recorded,
  not reproduced.
- **The UI thread draws and presents.** It waits on no file, no network, no lock a job may hold, no GPU readback
  and no pipeline compile: the one `block_on` it runs is the surface and the painter before the first frame;
  the device is made on a thread of its own (a winit window's handle is reachable only from the thread that
  made it, so the surface is made on the UI thread); the viewport's renderer is made on a thread of its own.
  No `rayon` feature of egui: tessellation runs on the drawing thread.
- **Start-up (D7)**: the window is shown at 0.07 s warm; the first frame follows at 0.96-0.98 s, most of it the
  adapter (0.39-0.58 s) and the surface (0.28-0.31 s) - the driver's. The 0.3 s budget holds for the window,
  not for the first frame.
- **Plan 5.4, `fresh`**: a fresh read does not cancel a computation already running; it waits for it and reads
  again if its revision moved. A `compute` that panics is remembered as failed for its revision.
- **A new scene-made entity's packet** carries the editor script's block the C++ editor writes (it runs the
  repository's CoP scripts, where `STATE_Read` returns early in the editor): a `se_zone_*` writes `u8 0`, a smart
  terrain six `0` bytes and `u16 6`.
- **The layer table, as `cargo metadata` gives it** (a crate's layer = 1 + the highest of its workspace
  dependencies):

  | Layer | Crates |
  |---|---|
  | 0 | gz-base, gz-luadata, gz-mcp-bridge, gz-win |
  | 1 | gz-chunk, gz-entities, gz-fmt-xml, gz-log, gz-ltx, gz-lua, gz-task, gz-ui |
  | 2 | gz-corpus, gz-crash, gz-fmt-level, gz-fmt-object, gz-fmt-sound, gz-fmt-spawn, gz-fmt-xr, gz-query, gz-store, gz-vfs |
  | 3 | gz-editor, gz-fmt-ogf, gz-fmt-tex, gz-particles |
  | 4 | gz-app, gz-github, gz-mcp, gz-project, gz-render |
  | 5 | gz-game, gz-mod, gz-ui-app |
  | 6 | gz-build, gz-scene, gz-ui-content |
  | 7 | gz-nq, gz-pda, gz-release, gz-screens, gz-ui-previews, gz-ui-viewport |
  | 8 | gz-catalog, gz-characters, gz-publish, gz-ui-graph, gz-ui-level, gz-ui-mod, gz-world |
  | 9 | gz-export-level, gz-ui-pda, gz-ui-release, gz-ui-world |
  | 10 | gz-levelbuild |
  | 11 | grizzly |

  `gz-game` depends on `gz-editor` (its services) and `gz-project` (the project holds the game's mount): it is a
  domain crate above the project, not beside `gz-vfs`. Neither edge is cheap to turn round (the mount and the
  services are its whole subject), so the table, not the code, changes.
- **Section 10's rule** stands for long operations: no pool is made smaller or slower for smoothness. A build of
  about a second is not chased for core load (the user's word, 12 Nov).
- **The game's suites as proof of section 9**: only `Run-ScreensQa.ps1 -Part Descriptions` takes outside input
  (`-ExtraCharacters`); `Run-ModsQa.ps1` and `Run-NqQa.ps1 -Scenario Scout` make their own fixtures, so they prove
  the game's side of the contract, not the editor's output.
- **After the cutover** (the pass "The cutover" above), the plan should say:
  - the status line: "Status: the Rust editor is the editor of the repository (phase 7 done; the C++ editor's last
    commit is the tag `cpp-editor-last`). Phase 8, the compilers, is left."
  - I4 and I5: the project folder WAS the interface of the two editors until the cutover; the oracle is now the
    C++ editor of the tag, through its records under `Intermediate/oracle` (the built `Bin\x64\Release\LevelEditor.exe`
    records a new one); `Source/` holds the compilers' closure alone and is edited only for them.
  - section 3 and 4: the scope is done - every row of "Rewritten" is in `crates/`, "Dropped" is gone from the tree;
    "Rewritten last" (`XrLC`, `XrLCLight`, `xrDO_Light`) is what `Source/` holds: 1,075 files under `Source/` and
    `SDK/` (the compilers, `XrETools`, `XrDXT`, `XrQSlim`, `FreeMagic`, `XrCore`, `XrCDB`, `XrAPI`, zlib, LZO,
    Ogg, Vorbis, RedImage, 30 headers of `External/Public`, 40 of `XrEngine` and `xrLoadSurface.cpp`, 21 of the SDK and
    `d3dx9.lib` / `DxErr.lib`), found from ninja's records of a build of them, built by CMake (`cmake/`) through
    `Build.ps1`.
  - section 6: the release profile (thin LTO, 16 codegen units, the line tables in a PDB beside the executable)
    and `crates/grizzly/build.rs`: the executable links the DLL C runtime alone (the vendored LuaJIT is /MT; with
    both runtimes the release link fails on `__imp_realloc`).
  - section 8: `tools/mcp/grizzly_mcp.py` is a launcher of `grizzly-mcp.exe`; `tools/qa/lint_ui_units.py` is gone.
  - section 10.1: "The oracle. The C++ editor of the tag `cpp-editor-last`: its records under `Intermediate/oracle`,
    keyed with the stamp it had at the cutover (`diff_editors.py` `CPP_STAMP`), so a rebuild of the compilers'
    libraries beside it does not throw them away."
  - section 13, the paragraph "Side by side" becomes: "**The editor.** `Bin\x64\Release\Grizzly.exe` (Build.ps1, the
    release profile) or `Intermediate\cargo\debug\Grizzly.exe` (`cg.sh build`), listening on 28016 (`-mcp_port`
    names another; QA uses 28917 and up, each editor in a home of its own). The QA suites drive it with nothing set
    (`GRIZZLY_QA_EXE` names another binary); `GRIZZLY_QA_TARGET=cpp` is for `diff_editors.py`'s records only, and a
    suite refuses it." "The branch", "Areas flip one at a time" and "What changed in C++ since then is the port
    list" end with the merge into `main`.
  - phase 7: done as written, with `Build.ps1` building the editor first (outside the developer shell, as `cg.sh`)
    and the compilers second, `-Only`, `-Out`.

## Next packages, in dependency order

1. `text` - done.
2. `project` (`gz-project`: the list, project files, rules, file tree, content operations).
3. `kit` (the UI kit's widgets, toolbar, dialogs) and `icons`.
4. `game-link` (`gz-vfs`, `gz-game`, `gz-lua`) after 1.
5. `mod-build`, `nq`, `assets` (characters, PDA, screens, world), `release` after 2 and 4.
6. `tex-xr`, `ogf`, `spawn-entities` - needed by the scene and the renderer (phases 4-5), and
   `spawn` by the world rules.
7. The shell and the tabs (phase 3), the scene (4), the renderer (5), the level tools (6).
