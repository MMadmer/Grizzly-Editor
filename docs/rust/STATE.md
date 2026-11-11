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
- `property_inspector`: a spawn's `Model\Animation` is the C++ playback buttons (`SSpawnData::
  FillProp`) - the Rust editor plays no spawn animation; the six Detail Objects rows of Level Settings
  are on the Detail Objects panel and `details_tool`; `Scene\Targets` is in Level Settings (the C++
  Preferences).
- A level bound shape counts as visible in the C++ until a viewport frame runs `CEditShape::OnFrame`;
  the minimised oracle never ran one (`object_info`, `scene_tree`, `scene_visibility`).
- Not in the tables, kept out of the scenarios: the C++ ray pick skips objects its camera does not see
  (`CSceneObject::RayPick`, `occ_visible`) - `place_way` snapping, `probe_vertical`, `scan_walls`,
  `drop_objects` follow a minimised window's camera; the C++ `scene_visibility` refuses a JSON boolean
  `visible`; the C++ undo stack after a Details edit is not the Rust one (D8).
- Fields of their nature: the index fingerprint hashes absolute paths; `fps`, `dpi`, `ui_scale`;
  packages, the index's byte columns and the descriptor's sizes (D4); `preview.png` (D2); the
  shell's tabs (`temp\session.ltx`).

## Next packages, in dependency order

1. `text` - done.
2. `project` (`gz-project`: the list, project files, rules, file tree, content operations).
3. `kit` (the UI kit's widgets, toolbar, dialogs) and `icons`.
4. `game-link` (`gz-vfs`, `gz-game`, `gz-lua`) after 1.
5. `mod-build`, `nq`, `assets` (characters, PDA, screens, world), `release` after 2 and 4.
6. `tex-xr`, `ogf`, `spawn-entities` - needed by the scene and the renderer (phases 4-5), and
   `spawn` by the world rules.
7. The shell and the tabs (phase 3), the scene (4), the renderer (5), the level tools (6).
