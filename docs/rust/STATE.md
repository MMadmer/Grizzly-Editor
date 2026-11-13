# Rust editor: where it stands

The hand-over note of the migration (`docs/RUST_MIGRATION.md` is the plan, `CONVENTIONS.md`
the rules). Whoever continues - a person, an agent, a later session - reads this first. It is
kept current: a finished package updates its row and adds what it learned.

## Open: nothing

The close sweep of 13 Nov (`docs/rust/briefs/close-sweep.md`; its work is "Closing: the old notes" at the end) went
through every statement of this record, `PARITY.md`, the plan, `UI_DESIGN.md`, `NQ_ARCHITECTURE.md`,
`PDA_FORMAT.md`, `CLAUDE.md`, `README.md` and `AI_SETUP.md` that something is not done, open, left or later, the
code's stubs and every command of the registry. Each statement below is also marked "(closed: ...)" where it stands.

| Statement | How it is closed |
|---|---|
| Findings, the object port: `ScaleKeys` / `NormalizeKeys` not done | ported in `gz-scene/src/objmotion.rs` (the level editor's motion keys); tests `scaling_doubles_the_spacing`, `normalizing_gives_each_metre_the_same_time` (new) |
| the same: `.bones` files, `GenerateBoneShape`, `OptimizeMesh` not done | dropped: only `ActorEditor` reaches them (`UI_ActorToolsProps.cpp`; `Optimize` called by a commented-out line); plan section 4, PARITY.md section 7 |
| Findings, the level port: the compiled `level` file, sectors, glows not written | phase 8: `gz-lc` `tree.rs` `save_sectors` / `save_glows`, held by `diff_compile.py` chunk by chunk |
| the same: `EDetailManager::Export`'s texture merge not written | `gz-levelbuild/src/details.rs` + `merge.rs`; the differential's `build_details.dds` rule (`s8_levelbuild2`) |
| Findings, the NQ port: the on-disk picker cache not ported | dropped: plan 5.4 (derived data is a query); PARITY.md section 8 |
| Integration: `LtxDoc` and `ManifestDoc` stay two documents | done in the sweep: one line model, `gz-ltx`'s (`Syntax::Ini` / `Syntax::Manifest`, `Placement`); `ManifestDoc` wraps it; both corpus round trips green |
| Integration: the Start page's status bar shows no centre | done before ("The shell's rest"); held now by `ui_list` `status_centre` in `test_ui_shell` A / F, written in UI_DESIGN.md |
| The crate table and two packages: "tests written, not run" (30) | run since "The first test run" (`tools/rust/test.sh`) |
| `gz-ui`: "not yet driven by a shell" | `gz-app` drives it |
| The renderer: details and wallmarks not seen in a picture | "What the QA pass left, closed", 2; `test_scene_tools` D / W |
| The object tools: deleting an object leaves its meshes in sectors | done before (`apply.rs` `forget_in_sectors`); its test is new: `deleting_a_scene_object_takes_it_out_of_its_sectors` |
| The shell's rest: the second look could not be run | "The scene's rest" (0 and 2 panels outside, no process left), "The look that was skipped" |
| The AI map tools: NOT LOOKED AT | "The look that was skipped", "What the QA pass left, closed" |
| The scene's rest: "still to look at" | the same two sections |
| The differential: `editor.spawn` in the game the next step | "Play ends in the game" |
| A root `grizzly_editor.ltx` a suite wrote | `run_all.py` fails "repository root written" |
| The 0.3 s budget under the run's parallel load | `test_ui_shell` runs alone; `run_all.py` waits for a quiet machine |
| Files kept for the lead (`rawdata\*_imgui_default.ini`, `ActorEditorLevel.cform`) | neither is in the tree |
| Phase 8b: the MU light not proven | phase 8c, 1; what is still unproven is the plan's section 9 list (the compiler engineer's) |
| Phase 8b: `recalculation_data_slots.details` not written | dropped with the network light it belongs to (D24) |
| The August compile records left for the lead | none is under `Intermediate/oracle` |
| Phase 8c's cleanup: corpus, suites, comparisons not run in that copy | run on the final tree |
| The final review, Play: `check.sh` red, D1 not naming the marker | green on the final tree; D1 names `play_root.txt` and both roots |
| The final review, the lead: the editor's license not decided | MIT (`77575f32`) |
| The final review, the lead: Play copies the engine (a change of the game) | closed by the Play engineer: "Closing: Play in place" (the installed engine on another fs config) |
| For the plan: the game root's depth "to be lifted" in the game | closed by the Play engineer: "Closing: Play in place" (`string_path`, `longPathAware`) |
| For the plan (both): the notes for the plan | folded - the last four after the last review; the layer table its own section |
| For the plan: drawing the viewport only when it can change "the shell's next step" | "After phase 8c: the window while a job runs", 1 |
| Next packages, in dependency order | history: every one done |
| PARITY.md: 24 rows `differs` / `unjudged` the list had closed or the plan explains | each says where (or its D); "Unjudged" judged |
| PARITY.md: Limited Depth / Depth Tolerance stored, never read | done in the sweep: the marquee (`draw.rs` `box_depth`, `frustum_select`), tested; Back Face Culling reads nothing in the C++ level editor either |
| Code: `gpu.rs` "on mutes for now" | wording: the C++ `MuteSounds` hold, renamed `mute_held` |
| Code: Preferences' rows "not kept by this build yet" | every row is live (`the_shells_keys_are_live`); the note is gone |
| Code: `todo!`, `TODO`, `FIXME`, "not yet", "later", stubs | none but words (PARITY.md section 9) |
| Commands no test named: `ui_toast`, `ui_split`, `spawn_animation`, `build_cform`, `build_aimap`, `build_all`, `reload_configs`, `level_rebuild_status`, `compile_details`, `play_game` | `test_ui_shell` E, `test_properties` C, `test_level_commands` (new); 0 left of 204 |
| UI_DESIGN.md section 8: copies on two surfaces | `section`, `raised_section`, `check`, `hint_field` moved into `gz-ui`'s `form` |
| UI_DESIGN.md section 8: "proposed and not needed yet" | not built until a surface needs one (said so) |
| NQ_ARCHITECTURE.md section 17, the rollout plan | every phase done (said so) |
| The tests' net sees only the temp folder | done in the sweep: `test.sh` watches the repository's tree too |
| The lead's: a command past the request timeout answered "editor busy (timeout)" while its job ran fine | done in the sweep: `gz-mcp` `run_request` runs the command in a job scope of its own (`gz_task::JobScope`); past the timeout a command whose job still runs answers `ok: false`, `running: true`, the jobs and where to follow them; unit test `a_command_whose_job_outlasts_the_timeout_names_the_job`, `test_build_job` T, AI_SETUP.md, the bridge's instructions |
| The last review (`docs/rust/briefs/critic-close.md`): `Grizzly.exe` is a console program | a window program, as the C++ editor was; `Grizzly-Cmd.exe` beside it for a shell - "The last review, the shell", "The last review: the lead's part and the run on the final tree" |
| The last review: three choosers (`skeletonanims`, `skeletonbones`, `gameanim`) cannot list | they list what the C++ listed - "The last review, the shell" |
| The last review: the root's `_projects.ini` a QA leftover, its snapshot in memory only | the author's list restored; `tools/qa/kept.py` keeps the root's files on disk while a tool has them aside (`b551b483`) |
| The last review: four notes not folded, the light bound met at equality, MPL-2.0 against "nothing copyleft", three ledger rows "in work", stale lines of `CONVENTIONS.md` | `a5331965` (bounds per kind of channel with a margin; `notices.py` `ALLOWED`) |
| The last review: texts that say "not part of this build"; the `COMMAND_LOAD` / Save As rows | "The last review, the shell"; two more of the kind in `gz-project` and `gz-ui-previews` (D16) |
| The last review: the ability read from `xrEngine.exe` alone; the path limit proven only to 368 characters; the early crash report's condition | "The last review, the game and Play" (game `232c5db9f`): four programs state it, 519 proven, a config inside the application folder reports as before |
| The last review: Play's copy path never run for real | there is no copy path: "Play in place only" - a game that cannot is refused before a build |
| The game engineer's: the renderers' `MAX_PATH` buffers on a path of the fs | "Play in place only" (game `ccf33e523`): `string_path`-sized, a dump proven at 364 characters |
| The shell engineer's: PowerShell's `>` takes nothing from a window program; a dump of the registry overwrote the author's log | `Grizzly-Cmd.exe`; a dump writes no log, a compile run its own |

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
| `gz-mcp-bridge` | done | bin `grizzly-mcp`: MCP over stdio (`initialize`, `tools/list`, `tools/call`) to the editor's TCP endpoint (`GRIZZLY_MCP_PORT`, 28017); tools, descriptions, schemas, annotations and instructions from `gz-mcp/tools.json` word for word; `tool_result` of the Python bridge ported (unknown arguments, fixed-argument tools, waits, images). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-editor`, `gz-mcp` | done | context, registry, `AutoSave`, `fsx`, the endpoint |
| `gz-app`, `grizzly` | done | the shell: caption (menus, fold under 1100 pt, tab strip, window buttons; drag, double-click, snap and the 6 pt resize edges are Windows' own through `gz-win`'s `WM_NCHITTEST` hook), toolbar, status bar, work area, drawers, Task Center card, palette, keys, session, settings file `grizzly_editor.ltx`; `ShellBuilder` + `Document` for areas (`install(b)` lines go into `grizzly/src/main.rs` `install_ui`); `UiQueue`/`on_ui`; 24 commands (`state`, `ui_*`, `window_control`, `titlebar_info`, `reset_layout`, `theme`, `preferences`, `shortcuts`, `command_palette`, `progress`, `screenshot_editor`, `list_commands`, `exec_command`); the tabs' own unsaved work as a `DirtySource` (`unsaved.rs`). `busy` = a job of a C++ blocking kind; the C++ command table and macros ("The shell's rest"). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
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
| `gz-ui` | done | theme, fonts, text, audit, layout, buttons; icons (all 189 glyphs, `icons/geometry.rs` + `icons/paint.rs`, `sheet`); widgets: `search`, `menu`, `toolbar`, `grid`, `list`, `splitter`, `state`, `card`, `dialog`, `units`. Not yet driven by a shell (closed: `gz-app` drives it); tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-project` | done | the list, `project.ltx`, path rules, asset kinds, delete checks, file ops, DARF listing, the file tree + watcher; 16 commands; hooks for other areas (open/close/scaffold/rename, templates, asset lists); holds the linked game's `GameFiles` (`game_snapshot`, `refresh_game`, `link_game`, `game_rev`) |
| `gz-vfs` | done | `GameFiles`: loose `gamedata` over `fsgame.ltx`'s `$arch_dir...$` archives (later over earlier), modules of `modules\` / `mods\` as roots of their own; `get` / `list` / `find` / `read` / `read_many` from the index; the saved index (`appdata\game_index\<hash>.bin`) checked by folder / archive stamps; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-game` | done | `GameLink` (project scope): `files`, `rev`, `refresh`, and as queries keyed by `rev`: `configs` (BFS include discovery, parallel parse, editor `CInifile` rules), `language`, `strings`, `texture_sheets`, `modes`, `spawn_sections`, `game_version`; `Bundled` + `read_game_or_bundled` / `spawn_roster`; commands `game_link`, `game_modes`, `list_spawn_sections`; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-mod` | done | `mod.ltx`: its own lossless `ManifestDoc` (the C++ comment rules, see findings), the typed `Manifest`, `Edit`s, metadata rules, `[commands]` (`E401`..`W408`), `[exclude]` / `outside_build`, `ManifestService` (store + `AutoSave`, outside changes reloaded); `mod_manifest`, `mod_set_manifest`, `mod_exclude`, `mod_commands`; the scaffold hook (`gz_mod::install`); slots `ManifestFields` (`world`) and `ManifestArgs` (`tags`, `thumbnail`) |
| `gz-release` | done | Package Release as the job `package` (group `build`) staged through `gz_build::run_build`; own ZIP writer (parallel level-9 deflate, ZIP64) and reader (index read back from the archive); index, descriptor, `[card]` as the C++ text; author key file (`xms-key 1`, PBKDF2 600000 + HMAC stream + MAC) and ECDSA P-256 signing/verify in pure Rust; MOD_CATALOG 8 / 10.3 content rules; tags and thumbnail hooks of `mod_set_manifest`; `mod_package`, interim `mod_catalog` (key actions only, `key_action` / `key_state` for `gz-catalog`). See findings |
| `gz-render` | done | wgpu 30 forward renderer, no window, no scene: `DrawList` -> `build_packet` (parallel cull or the caller's `Visibility`, instanced groups by mesh + material, blended back to front, helpers, labels, nearest 128 lights) -> the render thread (newest packet wins, passes in parallel chunks, frame clock `wait_frame`, `capture`, `Wake`), `render_offscreen`, `ThumbnailCache`; `Assets` (caller's `AssetLoader`, IO + CPU jobs, LRU by bytes, BC1-3 as they are), `MaterialTable` (data) -> 17 editor materials in WGSL (particle add / alpha-add / multiply, wallmark multiply 2x); `SpriteBatch` (camera / axes / along-velocity, world or screen size) in a dynamic buffer, instanced `Batch`es, cube textures and the `Sky`, decal depth bias, the editor's directional-light path (`ViewFlags::SCENE_LIGHTS`); Lit (studio) / Unlit / Wireframe / Game Lighting (`glit_light` ported); id-target outline; label occlusion queries; GPU skinning; `examples/offscreen.rs` rendered and looked at. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-particles` | done | pure port of XrParticles + `CParticleEffect` / `CPEDef` / `CParticleGroup` / `CPSLibrary`: the engine's `CRandom` and the CRT `rand` with MSVC's right-to-left draws, `pDomain`, Perlin `noise3` / `fractalsum3`, every action (`TargetRotateD` / `TargetVelocityD` folded), 33 ms steps clamped to 3, deferred stop, time limit, groups (related / free children by birth and death events, `time0` / `time1`), collision through a `Collider`; `Library` (sorted, effect then group), `System` (play / stop / follow parent / step / sprites / bounds); deterministic per seed. Pinned-step tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-build` | done | `BuildWalk` from the file tree, `BuildCheck` / `BuildChecks` (+ the bake slot), target refusals, the clean build in `modules\.new\<id>` with hard links and two renames, the flat export + report, `run_build` (for `gz-release`), `Builds` (job `build`, results, Task Center `BuildStatus`, `CleanQuestion`); `mod_export` |
| `gz-github` | done | the network side: `reqwest` on native-tls (Windows certificate store) + system proxy, three clients (API and uploads never follow a redirect, public files do), every request raced against the job's `Cancel`; `Secret` (zeroed, `Debug` redacted); `Endpoints` (real or `-qa_github` mock, `upload_url_trusted`, `vt_url_trusted`, the catalog's repo and mirrors); account (env -> Credential Manager -> `gh auth token`, a minute's cache), device flow; `publish` (repo, foreign latest, published tag / version, draft reuse, leftovers, uploads 3 at a time with the descriptor last, stored assets checked, latest, the served check); list / edit (`make_latest` where the mark is) / delete (release, then tag); `fetch_public`; catalog issues (`find_issue`, bot-only newest verdict, submit / withdraw / issue); VirusTotal (pace slots, policy, verdict, lookup, upload with the upload address check) and the scan (`lookup`, `scan`, `ORDER`, `describe`, `json`); ini helpers of the catalog files. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-publish` | done | `Publisher` (job `publish`: packaging through `gz_release::run_package` in group `build`, then the scan of a listed module + the upload in group `publish`; `ScanFirst` word for word; Task Center cards `key` `publish`), `FreshTag` / `deleted.tags`, device sign-in, `ReleaseList` (list, edit, delete, select, `show=edit` as a request the UI takes); `mod_publish` (answers when the packaging ended, as the C++ `FollowsJob`), `mod_releases`. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-catalog` | done | `Catalog` (one job at a time, group `catalog`: `check` = section 7 with the content rules over the local copy's zips, `vt`, `submit`, `submission`, `withdraw`), `Release` read as the game reads it + signature, `read_listing`, `vt_level` (never optional), the submission text / details, `website_is_module_page`; the real `mod_catalog` (key actions through `gz_release::key_action`). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-scene` | done | `SceneState` in one store (objects `OrdMap`, per-class order, names, selection, `ToolSettings`, `BulkData` with the AI map in 4096-node tiles), `SceneCmd` (place, delete, rename, transform batches with the group rules, select, show, tool eyes / read-only, group / ungroup, property replaces checked by `serial`, mod marks, order, snap list, bulk, saved, touch); all 16 classes read / written in LTX and stream (`objects`), the main file + parts (`io`: LTX, binary `.level`, undo / recovery stream, unknown sections / chunks / refused versions kept); `props` (rows, merge, text set, labels / units / help); library, pick tree (`Derived` by scene + library rev), drop / snap, recovery (`backup\recovery`, the C++ manifest); 34 commands, `list_assets` Objects / Groups |
| `gz-export-level` | done | The module's level layer of `EditorModScene.cpp`, pure over a scene snapshot + gathered disk state (`gather`): `.xspawn` (`op = add/start/remove`, `[path:]`, custom data with `[xms_order]`, fixed bones checked against the OGF, smart cover, lamp, zone shapes, every C++ warning, merge or mirror), `overlay.xcform`, cuts (+ `[cut_N]`), world-space `.ogf` overlay (parallel) + `[v_N]` registry; `%f`/`%g` exactly as UCRT (`cfmt`); `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe`; `install` fills `gz-build`'s bake (the mirror pass + its note), `gz-world`'s `SceneStarts` (E201 is `gz-world`'s) and `gz-nq`'s `SceneLookup` (restrictors, story ids, way paths, level). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-levelbuild` | done | The full level build: `build.prj` (static + MU objects, LODs merged into `level_lods*.dds`, hemi / sun / point lights, glows, portals, sectors, wallmark faces), `level.ltx`, `BuildGame` files, HOM/SOM, `build.aimap`, `build.details` + `build_details.dds`, `level.wallmarks`, scene stat; CForm; level graph, game graph + cross table (`CGameGraphBuilder`), multi-level merge (`CGraphMerger`); `editor.spawn` (spawn constructor, patrols, artefact points, graph embedded); `nav` = the engine's `ILevelGraph` lookups + A*; XrLC / xrDO / xrAI / the game as `tokio::process` jobs (`level_run`, group `game`, Stop = `taskkill /T`); 19 commands named as `gz-ui-level` expects + `level_rebuild_status` + `level_build action=`; the breakable / climable objects of `BuildGame` (`breakable`); the whole `Scene->Validate` (`validate`). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-lc-light` | done | phase 8a: `XrLCLight` + `xrDO_Light` - the compiler's model (mesh, deflectors, lightmaps, MU models, `xrLC_GlobalData` as `LcGlobalData`), the ray model, the light of lightmaps, vertices, implicit textures, MU models and detail slots; see "Phase 8a" below. 39 tests, run and green |
| `gz-lc` | done | phase 8b: `XrLC`'s `CBuild::Run` whole (the phases, the OGF family, QSlim, NvTriStrip, NVMeshMender, the bounding spheres, the savers) and `xrDO_Light`'s pipeline; `Grizzly.exe -headless -compile <level folder>` / `-compile_details`; proven against the C++ compilers by `tools/qa/diff_compile.py`; see "Phase 8b" below. 16 tests, run and green |
| `gz-lua` | done | LuaJIT 2.1 vendored (`mlua` 0.12, JIT off, count hook = instruction budget); `LuaHost` pool of states (one per caller, scripts kept per revision), `Script`, `Arg::{Value, Chunk}` (a text run in an empty env: the asset is the table LuaJIT builds), `call_omitting`; sandbox = base + string/table/math, the C++ reader's 18 globals; `vanilla_source` rewrites the fork's `//` `/* */` comments; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-pda` | done | `evaluate` (gz-luadata check, then the game's `normalize`), `Layout`/`Item`/... read as the C++ read the stack, `preview` block, `template`; `Preview` state machine (pages, vars, log, toasts, rows, scroll, keys, map zoom, boxes / hit test / `visible` in canvas units); `PdaService` (reader and layout `Query`s, W320 textures, list, `[pda] layout`); the build check; `content_create kind=pda` template; `pda_list`, `pda_get`, `pda_write`, `pda_delete`, `pda_use`, `pda_validate`; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-app` | done | the Start page (`ShellBuilder::start_page`: New Project form, Open Folder via `rfd` on a thread of its own, search, Last Opened / Name sort, 232x184 cards with twin parents, previews decoded on the pools through a `Query` keyed by path + write time, `ShowsNoScene`, card menu, `dlg.delete_project` counted off the UI thread), Link Game page card, the Output Log (one body for the drawer and the Level panel `output_log`, cursor over `gz_log::Log`, distinct counting per window from "Project opened:", the `log_state` hook), Preferences (`preferences` tab: 44 rows of `EditorPreferences`/`LevelPreferences`, nav, search, Keyboard page over `shortcuts` with an Assign dialog `dlg.assign_shortcut`); command `project_browser_menu`. Every row is live (`object_flags` and `recovery_*` are in `gz-app`'s `PREFS`, not on the `preferences` wire). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-nq` | done | quests and behaviour graphs: `asset` (typed `Graph` with `imbl` node/task vectors, read with the C++ shape messages, `write` byte-equal to `NqLua::Write`, outline, topo order, templates), `catalog` (hand-rolled reader, fold order game|bundled -> modules -> project, `Catalog::is_trigger`, exclusive forms, JSON), `layout` (`node_box`, the one rectangle; the dominator-tree auto layout on bitsets), `validate` (every code and message of `NqValidate`), `references` (typed walker, `find`, `rename_task`), `index` (snapshot, fingerprint, search), `pickers` (15 types from the game's ltx tree / strings / XML / `all.spawn`, the project's custom data, `spawn.part`, looks; `compose` with the scene), `caps` (`class_registrator.script` families), `doc` (`Store<GraphDoc>`: every `quest_apply` op, canvas ops, view state with bookmarks / history / find), `NqService` (queries keyed by game rev, file stamps, manifest, docs' edit serials; slots `CharacterLookup` / `SceneLookup` / `WorldLookup`), the build check (`E113` across graphs); 24 `quest_*` commands (all but `quest_view`), templates, rename hook, `DirtySource`. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-screens` | done | `Screens` model parsed off `ManifestDoc`, written back with `AppendScreens`' rows (sections in place while their order holds), `import` (+ `plan`: the decision by place and extension; parallel-safe copies), `validate` (E401..W411), the build check, `GameLevels` slot for W407; `screens_get`, `screens_set`; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-characters` | done | `model` (`Character` keeping unset unset, `parse` / `load_text` / `write` as `EditorCharacters::Parse` / `Write`, `{ rus = .. }` alone collapsed on write, `text_in` / `text_raw` / `text_set`, `template`, `game_id`), `resolve` (`E111` / `E112`, kits merged, a shop whole, defaults once), `validate` (every C++ code word for word over `Lookups` = `gz_nq::Pickers` and `Models`; `E123` from `OGF_S_BONE_NAMES` read off the chunk list, cached per model and game rev), `game` (trade presets, bone names, stalker damage bones); `Characters` service: `Roster` as a `Query` keyed by a stamp of the `.npcasset` files + `[exclude]`, read in parallel; fills `gz_nq`'s `CharacterLookup` on project open (briefs refreshed in the background, the file tree's until the first roster); `Checker` (linked game: pickers, presets, models); build check (first error, as `CheckForBuild`); `content_create kind=character` template; `characters_list`, `characters_get`, `characters_write`, `characters_delete`. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-world` | done | `model` (`World` / `StartKit` read off `ManifestDoc`'s raw lines as `EditorMod::Load`, written back as `Edit::SetLines` in the `Save` layout, JSON, `base_kit`), `game` (`read_all_spawn`: `gz-fmt-spawn` container + `SpawnIndex` in parallel, level changers by class `LVL_CHNG`, destination from `gz-entities` with the C++ byte walk as fallback; levels, transitions, patrols sorted), `starts` (`op = start` of `spawn\*.xspawn`), `validate` (`E201`..`W208`); `WorldService` (game world as a `Query` keyed by the game rev, captions from the Level picker, starts with the scene's through the `SceneStarts` slot, problems); fills `gz_nq`'s `WorldLookup` (kicked on first ask), `gz_screens::GameLevels`, `gz_mod::ManifestFields` `world`; build check; `world_get`, `world_set`, `world_transitions`. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-mod` | done | Mod Settings tab (`mod_settings`: nav of 7 pages with problem dots, 44 em form, the C++ `CheckSettings` sentences under fields, saved as edited through `mod_set_manifest` / `mod_commands` / `pda_use` once no field is being typed in; untouched fields follow outside saves); Build to Folder / Export Flat / Package Release dialogs, `dlg.clean_target`, `dlg.replace_release`, the kit `Result` (`build_result`, `export_result`, `package_result`; `ui_dialog open` puts the last one up again to look at), `KeyLine` (the author key row); Task Center cards of every `build` / `package` job (`cards`); Mod menu rows in `MOD_MENU` slots and the Build Mod split; `mod_settings_view`. Looked at on a temp copy. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-content` | done | the Content Browser: one `ContentBrowser` service, two hosts (`content_browser(ui, cx, Host)`: the Content Drawer via `install`, the Level panel `content_browser` - the Level document calls it with `Host::Panel`); sources Project (`FileTree` snapshot), Editor (each category a folder of the data root, walked with `FileTree::scan`), Game (`GameFiles`) as `Catalog`s built on the pools, folder views as a `Query` keyed by (source, folder, search) + revisions; pictures: `texture_tile` on the pools for the size drawn, models through a renderer of the crate's own + `ThumbnailCache`, textures released 3 frames late; `caption` (pure `DrawTileCaption`); every action an editor command; `DragPayload`; `content_browser_navigation` / `_open` / `_selection` / `_copy`. Looked at: tiles 48/96/160/192, list, textures, models, rename box, menus. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-world` | done | the Characters tab (`characters`: list 260 pt with search, dirty dots, Template badges, splitter, form up to 760 pt bound to the RAW values, base's values as hints / muted kit rows, Ctrl+S = `characters_write` with `gz_characters::write`, the file follows the id via `content_rename`, `dlg.delete_character` -> `characters_delete`, unsaved prompt before any switch, problems resolved + validated on the pools) and the World Rules tab (`world_rules`: every edit a `world_set`, sent in order; Original Game / Player Start / Starting Kit, More with Applies On / Playable Area / Transitions; problems bar + drawer; `dlg.empty_world`); `Lists` (pickers, trade categories, `meshes\actors` models, read on the pools, busy until read); portrait sheets decoded on the pools, kept per project; `item_picker` (both tabs); Mod menu rows; `characters_view`, `world_view`. Looked at on a temp copy. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-release` | done | Publish Release (`publish_release`, L: one Release line + Notes, the rest under More unless it stops the release; reads the releases as it opens, a version that is out is a problem with Open Mod Settings / Open Releases, a failed read a muted note; account rows, device code in Title Mono, token field; Publish closes it, the card follows, the kit `result` `dlg.publish_result`), `dlg.edit_release` (also from `take_edit_request`), the Releases tab (`releases`: sortable table, details with Edit... / Open Page / Delete... GhostDanger, row menu, Del / F5, the kit `confirm` `dlg.delete_release`, banners), Submit to Mod Browser (`submit`, fixed 600 pt body, five cards, VirusTotal card in the Required tone, footer busy + Stop <job> + Hide, label sets) with the sheet `dlg.catalog_withdraw`, follows `Catalog::window_open` / `wants_withdraw_question`; toolbar Publish Release / Releases, Mod rows at `gz_ui_mod::mod_menu_slot`; the key row is `gz_ui_mod::KeyLine`. Every button runs the domain path its command runs (`Publisher::start`, `ReleaseList::start`, `Catalog::start`, `AuthorKeys`, `Github` token / VT key). Locals: `local_secret_field`, `local_secret_row`, `local_code`, `local_card_title`, `local_fold`, `local_table_head`. Looked at under `-qa_github` with dead loopback endpoints; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-pda` | done | the `pda` tab (singleton view over a session `PdaTab` model behind one lock: file, Source text, the reader's answer via `PdaService::evaluate`, `gz_pda::Preview`, view, last frame's geometry; retargets through the shell's unsaved prompt, follows the file on the tree revision and before every `pda_preview`), the canvas (device of `pda.xml` / `pda_16.xml`, sheets / fonts / level maps from `PdaAssets` - decoded on the pools, uploaded once, dropped with the session, the link or Reload; bitmap glyph quads; Fit / 100% / wheel-at-mouse / pan in px semantics; screen edges; stock panel, toasts, map with actor and markers), Details tabs, Source pane (Mono, gutter, error band, Apply / Ctrl+Enter / Ctrl+S); `pda_preview` (canvas-unit state, `map.bound` / `spots`, waits for 2 frames when owed); the `screens` tab (lists, 16:9 preview with DDS / GIF thumbnails, layers with drag reorder, music combo, More, track form; edits as `Change`s applied locally and saved through `ManifestService::edit` on the freshest model); `local_file_drop_zone` (Explorer drops wait for the pointer's next move, `gz_ui_content::DragPayload` and std path payloads, inner zones first, `simulate_drop`); `screens_view` (waits for the drop's import and save); `COMMAND_MOD_SCREENS` / `COMMAND_MOD_PDA` at `gz_ui_mod::mod_menu_slot`. Looked at on a temp project: 4:3, 16:9, Source with an error, a two-layer screen. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-previews` | done | the `model` tab (own `gz_render::Renderer` sized to the canvas, a hand-built `DrawList`: the model posed by `idle` / its first cycle, a floor of round metre steps under it, bones as overlays; orbit / pan / zoom-at-cursor `Orbit`; packets built and submitted on the CPU pool only when the view changed or went stale; Details grid), the `texture` tab (read + uploaded on the pools: native DDS as the GPU takes it - BC1-7, 32-bit, R8, RGBA16F, every whole level - else decoded to B8G8R8A8; Fit / 1:1 / Zoom / channels / Mip / Details), both drawn through `image_pass` (a WGSL pass in egui's render pass: straight alpha, channel and alpha-as-grey views, opaque renders, nearest past 2:1); `loader::Sources` = project, editor, linked game (also the renderer's `AssetLoader`); Light Animations (`Store` with undo over `lanims.xr`, timeline, key bar, Play), Texture Library + New Textures (`.tga`/`.thm` walk as a `Query`, `Store` of edited descriptions, save/import as a job writing `.thm`, `.dds`, `.tga`; Types filter; texture picker), Sound Library (`.wav`/`.thm` walk, waveform from `Pcm::peaks`, `cpal` audio thread, save writes `.thm` and makes the `.ogg` again from the `.wav`, Rebuild, Synchronize encodes what is missing or of another age); Libraries menu incl. Check New Textures, Synchronize Textures... / Sounds... (confirm, then jobs); `preview_model`, `texture_preview`, `library`, `asset_preview`. Cube Map \ Smaller Copy, Reload Objects / Textures, Rewrite Texture Descriptions ("The shell's rest"). `gz-app/src/window.rs`: BC feature requested when the adapter has it, device errors logged. Looked at on a temp project. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-level` | done | the Level document (kind `level`): a dock of its own over the kit's splitters (`layout`: Wide / Compact builds, four slots, open / slot / front per panel and the sizes in `level.*` layout keys, custom = placement or 8 pt-rounded sizes differ from the build; `dock`: side slots keep pt, viewport min 360 x 240, splitters last; 26 pt tab strips with cross on hover and a Move To / Close menu; F11), the viewport pane in the centre; Level toolbar over registry commands (`toolbar::LEVEL_ACTIONS` - `build_cform`, `build_aimap`, `build_game_graph`, `build_all`, `play`, `play_game`, `play_clean`, `play_stop`, `reload_configs` - added only when registered), status centre + Rebuild chip over `level_rebuild_status` (polled, cached); World Outliner (`outliner_model::compute` as a `Query` by scene rev + filter, Unreal search grammar, natural sort, By Hierarchy, virtualised rows, Filters badge, More, Type / Owner columns, `dlg.rename_object`, `dlg.delete_object`); Tools (tiles, sticky header, Object / Spawn / Particles lists from `lists::Lists` read on the pools, Group, Detail Objects + `dlg.detail_objects`, Select by Reference, Random Settings `dlg.properties`, Snap Objects; picks go to `gz_ui_viewport::set_place_item`); Details / Level Settings (rows prepared as a `Query`, categories, kit grid, transform drags merged + sealed, other values sent on release, `dlg.choose` over the lists, `dlg.edit_text`, Quality segmented); Scene / Build / View menus and 11 scene `UiCommand`s; Recovery page card; `dlg.import_base_scene`; `preview.png` on save and a minute after a change; `outliner_show`, `outliner_filter`, `scene_visibility`, `property_inspector` (wraps gz-scene's, adds the panels). Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-viewport` | done | the Level tab's 3D view: `ViewportPane` (`new`/`ui`/`inactive`/`shortcut`/`mcp_state`, plus `set_maximized`/`take_maximize_request`), `Viewport` service (camera `Nav` = `CUI_Camera` + `ViewportNavigation`, view flags, tool and snaps, Game Lighting, render size), pure `draw::build` (scene snapshot -> `DrawList` with every class's helpers in the C++ colours) + `draw::visibility` from the scene's pick tree, packets built on the CPU pool only when scene/library/view/assets change, egui overlays (pills, 64 pt axis gizmo, transform gizmo, Mono stats pill, labels, safe rect, marquee), picking/select/place/move/rotate/scale as `SceneCmd`s (one merged undo step per drag), Game Lighting (`lighting`: weathers, user.ltx, lamps/campfires/level lights, roof map), the asset loader (project, editor, game), the camera per scene in `<project>\temp\viewport.ltx`, `capture_preview`, `set_place_item`, `view_menu`, `install` (Keyboard page commands); the extras built in the packet job (`draw_extra`: glows, `draw_particles`, `draw_aimap`, `draw_details`, `draw_wallmarks`, caches as services); 10 commands (`camera_get`, `camera_set`, `focus_object`, `viewport_navigation`, `viewport_controls`, `view_mode`, `game_lighting`, `frame_pacing`, `screenshot_viewport`, `particles_tool`). Looked at in the running editor; tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |
| `gz-ui-graph` | done | the graph tab (kinds `quest` / `beh`, ids `quest:<rel>` / `beh:<rel>`): `Scene` built per revision on the CPU pool from `gz_nq::layout` (texts, pins, beziers + back edges routed like `RouteBack`, 512 pt grid index, extents, sorted edges for `beyond`, the minimap's mesh) - a frame draws only the cells in view; `Canvas` (10 zoom levels, LOD by title px, pan / wheel / drag / link / marquee / action drag, edge pills, minimap, context menus, kind pickers by the document's vocabulary); a `ViewMirror` of the store's view, posted when a gesture ends and taken back when the document moved it; `Inspector` (Graph panel: header, Tasks or Roles, Variables, Global -> `mod.ltx [vars]`; Details: node, action under `graph.action_split`, help footer) staging a copy committed as `set_node` / `set_quest` (Replace only for roles and a variable rename); typed editors of `DrawTyped` in `params`; dialogs (Find in Project, Rename Task, Task References via their commands, Edit Text / Lua); `quest_view` (C++ wire, held for its frames), `quest_open` brings the tab up. The camera button of a position reads `gz_editor::ViewCamera`. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run") |

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
  `ScaleKeys` / `NormalizeKeys`. (closed: `ScaleKeys` / `NormalizeKeys` are the level editor's motion keys
  (`CustomObjectAnimation.cpp`), ported in `gz-scene/src/objmotion.rs` with their tests; the other three only
  `ActorEditor` reached - `UI_ActorToolsProps.cpp` loads / saves bone data and makes bone shapes, and
  `CEditableObject::Optimize`, the one caller of `OptimizeMesh`, is called by a commented-out line of
  `UI_ActorTools.cpp` - dropped with it, plan section 4, PARITY.md section 7.)

From the level port:
- Not written: the compiled `level` file (`fsL_*`, sectors, glows), `EDetailManager::Export`'s
  texture merge. (closed: the `level` file is `gz-lc` (phase 8, `tree.rs` `save_sectors` / `save_glows`), held chunk
  by chunk by `diff_compile.py`; the texture merge is `gz-levelbuild/src/details.rs` + `merge.rs`, held by the
  differential's `build_details.dds` rule on `s8_levelbuild2`.)
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
  picture: the test scene has neither. (closed: "What the QA pass left, closed", 2; `test_scene_tools` D / W.)
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
- Not ported: the on-disk picker cache (`nq_index_*.txt`) - the pickers are in-memory `Query`s. (closed: dropped
  by plan 5.4, PARITY.md section 8.)

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
(closed: one line model since "Closing: the old notes" - `ManifestDoc` is `gz_ltx::LtxDoc` read by `Syntax::Manifest`;
the Start page's status bar is held by `ui_list` `status_centre` in `test_ui_shell` A / F.)

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
  control of these panels. (closed: `gz-scene/src/apply.rs` `forget_in_sectors`, test
  `deleting_a_scene_object_takes_it_out_of_its_sectors`.) `gz_scene::math`'s `EPS_L` / `EPS_S` (0.0001 / 0.000001) are not the
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
  second panel out, the exit with the fix - could not be run. (closed: looked at in "The scene's rest" - 0 and 2
  panels outside the window, no process left - and "The look that was skipped".)
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
- NOT LOOKED AT (closed: "The look that was skipped", "What the QA pass left, closed"): no graphics device could be created while this package was built. Commands were
  driven headless; to be looked at: each panel, Generate Selected and its nodes, link edits,
  details reinitialised and drawn, a wallmark placed and drawn, `ui_audit`, `frame_pacing` during
  a generation. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run").

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
  `ui_audit` escaped nothing. Still to look at (closed: "The look that was skipped", "What the QA pass left, closed"): the Make question without `-nodlg`, motion playback
  and Camera View in motion, Random Append placing, Attach on Click, the Rename and Reset Surfaces
  questions, the Group tool's Pivot chooser. Tests written, not run (closed: they run in `tools/rust/test.sh` since "The first test run").

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
hand-run of the C++ editor's button on this scene is the next step (closed: "Play ends in the game"); then the actor's graph id against the
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
- Open (closed: `run_all.py` fails a run that changes or adds a file of the repository root, "repository root written"): a `grizzly_editor.ltx` with a `[window]` section (written 09:32:27, during `run_all.py`) appeared in the
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
  (12.1 s). Open: that budget is a timing check under the run's parallel load (closed: `test_ui_shell` runs alone, and `run_all.py` waits for a quiet machine before a suite that times something). No "repository root written".
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
  and the root files no section names are left for the lead. (closed: neither file is in the tree.)

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

## Phase 8b: the compiler and its proof

`crates/gz-lc` (on `gz-lc-light`, `gz-fmt-level`, `gz-fmt-ogf`, `gz-fmt-tex`, `gz-fmt-xr`, `gz-chunk`), 6.9k lines, 16
tests run green (`cg.sh test -p gz-lc`, 0.6 s). Brief: `docs/rust/briefs/lc-pipeline.md`. The proof:
`tools/qa/diff_compile.py` (the runs and the records) and `tools/qa/compare_compile.py` (the rules).

**What it holds (the C++ it ports):**

| Module | C++ |
|---|---|
| `lib` | `CBuild::Run` in its order, the `level` header, `Options` (the command line), `compile` / `compile_details`, the phase log of `cl_log.cpp` with each phase's seconds |
| `load` | `CBuild::Load` (build.prj, the shader library, the textures with their `.thm` on every core), the paths of `fsgame.ltx` |
| `geom` | `PreOptimize`, `CorrectTJunctions`, `xrPhase_AdaptiveHT` (tesselation, the hemisphere precalc, `u_SmoothVertColors`), `xrPhase_TangentBasis` |
| `mender` | NVMeshMender (`NVMeshMender.cpp`, the one the build links; `NvMender2002` is dead code) |
| `cform` | `CBuild::BuildCForm`, `SimplifyCFORM` (`xrOptimizeCFORM_qslim.cpp`) |
| `qslim` | `XrQSlim`: `MxStdModel`, `MxQSlim`, `MxEdgeQSlim`, `MxHeap`, `MxQuadric3` |
| `split` | `xrPhase_ResolveMaterials`, `Detach`, `xrPhase_UVmap`, `xrPhase_Subdivide`, `xrPhase_MergeGeometry` |
| `ogf` | `OGF_Face*.cpp` (`_BuildVertex`, `Optimize`, `MakeProgressive` over `gz-fmt-ogf`'s VIPM, `Stripify`, `CalculateTB`) |
| `stripify` | `nv_library` NvTriStrip, lists only, `xrStripify` |
| `vcache` | the face order of D3DX9's `D3DXOptimizeFaces` (closed code): Forsyth's linear-speed vertex cache order |
| `sphere` | `OGF_Face_Sphere.cpp` (`CalcBounds`), xrCore's miniball, `Mgc::MinSphere` with the UCRT `rand()` stream, MSVC's `std::random_shuffle` |
| `tree` | `Flex2OGF`, the MU `calc_ogf` / `export_geometry` / `export_ogf`, `BuildSectors`, `SaveTREE`, `SaveSectors`, `SaveLights`, glows, portals, shaders, `level.geom`/`level.geomx` |
| `details` | `xrCompileDO` over `gz-lc-light`'s detail light |

`Grizzly.exe -headless -nodlg -compile <level folder> [flags]` and `-compile_details <level folder>`: the log the C++
prints, phase by phase with seconds, on stdout; exit 0 or 1; on the caller's pool of every core
(`available_parallelism`). The flags of `xrLC.cpp`: `-f` is the folder; `-nosun`, `-noise`, `-nosmg`, `-no_optimize`,
`-no_invalidefaces`, `-att` are kept; `-silent`, `-o` (the options dialog) and `-keep_temp_files` (QSlim's SMF dumps)
are taken and change nothing; `-gi` and `-net` are refused with a sentence (D24); any other is named in the log and ignored, as the C++ ignores it. xrDO_Light's
`-net` the same. Wiring the compile into Play and the level build is the next pass.

**The scenes** (`diff_compile.py` `SCENES`; each C++ run from a stage under `%TEMP%`, `-silent -flushlog`, ten minutes):

- `mp_test` (the Play suite's): XrLC 18.1 / 18.0 s wall, 10.2 / 10.0 s CPU.
- `l12u_control_monolith`, the middle scene: 410k collision faces, 84,652 subdivisions merged to 641, 749 visuals, 40 lights,
  sectors and portals. XrLC 261.2 / 260.3 s wall, 992 / 1001 s CPU. The largest scene of `rawdata\levels` XrLC finishes
  within the limit; it has no MU model.
- The two smallest scenes with MU models do not finish within the limit: `l03u_agr_underground` (7 MU models, 15 references) passed it in
  "Building lightmap 0" (its LMaps took 513 s), `mp_close_combat` (MU, LODs, details) in "LIGHT: Implicit". The record
  refuses an unfinished run and starts no second one. l03u's `build.cform`, written at 25 s, was read from the stage
  before the end: its 5,661 MU triangles and their records equal the Rust ones byte for byte. The MU OGFs and the MU
  light are therefore not proven against the C++. (closed: "Phase 8c", 1, MU models against the C++ compiler; what
  stays unproven is the plan's section 9 list, the compiler engineer's.)
- `mp_close_combat` for the detail compiler alone (`DETAILS_ONLY`): both detail compilers read the same `build.prj`,
  `build.details` and the Rust compiler's `build.cform` / `build.lights` (equal to the C++ ones under the rules below
  on the other scenes). xrDO_Light 74.5 / 74.4 s wall.

**The rules** (`compare_compile.py`'s docstring; everything else byte for byte):

- `build.lights`: byte for byte.
- `build.cform`: each triangle's last word left out (the C++ writes the low half of a face pointer, the Rust the index);
  `b_rc_face`'s padding after the material and after `reserved` left out (never set in the C++).
- `level.cform`: else the triangles as a multiset; else - QSlim re-rates the edges around a contraction in the order
  of their heap addresses (`MxQSlim.cpp` `apply_contraction`), so ties fall another way and cpp1 and cpp2 differ too
  (l12u: 383,572 / 383,574 triangles) - the triangle count equal, the triangles one side alone has no more than between
  the C++ runs (at least 1e-4 of all), every material word's area within 1e-4. l12u: 16 one-sided of 383,572.
- `level`: chunks 1, 2, 4, 6, 7, 8 byte for byte; visuals sub-chunk by sub-chunk, a progressive visual's fast path with
  its sliding windows' four `reserved` words left out (never set: cpp1 and cpp2 differ there). l12u: 739 of 749 visuals
  byte for byte, 10 under this rule. The bounding spheres are byte for byte (the old 1e-4 bound is gone).
- `level.geom` / `level.geomx`: the header's quality left out (never set in the C++); vertex buffers byte for byte but
  the light bytes (under the light bounds) and the lightmap coordinates (a deflector the light finds flat shrinks to a
  texel or two, so a light difference moves the deflectors after it in the page); index buffers and sliding windows
  byte for byte, but a non-progressive visual's fast path in `level.geomx` as a multiset of triangles by corner
  positions with the same vertex positions (D3DX9's face order).
- The lightmaps: every texel inside each lightmapped triangle sampled through each side's own coordinates; the implicit
  textures texel by texel; `level.details` slot by slot.
- `recalculation_data_slots.details`: xrDO_Light's state for its own next partial relight; not written (the resumed
  light is not ported, phase 8a). (closed: dropped with the network light it belongs to, `recalculation.*` of D24.)

**The light bounds** (per channel in 0..255 steps, mean absolute error / 99th percentile): lightmaps 3.0 / 16, vertex
colours 2.0 / 16, details 0.5 / 2. The floor, cpp1 against cpp2, is zero on every channel of both scenes and on the
details. What the bounds allow is what the port may differ in: the DXT5 encoder (NVTT in the C++, texpresso here - a flat
128 decodes as 126 against 128, a third of a 5-bit step), another ray-triangle test than Opcode's on grazing and
coincident faces, the transparencies multiplied nearest first, Rust's `sin`/`cos`/`pow`. Measured:

| | 1.r | 1.g | 1.b | 1.a sun | 2.r | 2.g | 2.b | 2.a hemi | vertex |
|---|---|---|---|---|---|---|---|---|---|
| mp_test | 0 / 0 | 0 / 0 | 0 / 0 | 0.03 / 0 (max 96) | 1.99 / 2 | 0.02 / 0 | 1.99 / 2 | 0.01 / 0 | 0 / 0 |
| l12u | 0.73 / 8 | 0.63 / 6 | 0.65 / 8 | 0.003 / 0 | 0.01 / 0 | 0.01 / 0 | 0.01 / 0 | 0.24 / 4 | 0.005 / 0 |

mp_test's 2.r / 2.b is the encoder alone (page 2's colour is the sun, flat 128). Its sun's max 96 is the corner under
the small box: the C++ finds one of four samples of the floor's border texels lit through the seam of two coincident
walls, the Rust none, and so keeps two of the box's sides as one-texel deflectors where the C++ keeps them lit (24
lightmap coordinates of 136). `level.details` of mp_close_combat: byte for byte. The pictures
(`Intermediate/compile/<scene>/pictures`, each page of both sides, the Rust page sampled into the C++ layout and the
difference x8) were looked at: the same spots, gradients and shadows in the same places, no banding, seam, leak or dark
border; the difference is a speckle of the red and blue 5-bit steps and the triangles' outermost texels.

**Phase times** (seconds and CPU load, CPU time over wall time; the C++ log is read every 0.1 s, so its phases are
known to about 0.2 s; a phase only one side names counts with the one before it):

| phase | l12u C++ | load | l12u Rust | load | mp_test C++ | mp_test Rust |
|---|---|---|---|---|---|---|
| Optimizing | 0.88 | 0.9 | 0.78 | 3.0 | 0.22 | 0.02 |
| Adaptive HT | 21.28 | 6.6 | 1.39 | 7.1 | 0.98 | 0.00 |
| Building normals | 0.66 | 1.0 | 0.52 | 1.0 | 0.22 | 0.00 |
| Building collision database | 2.06 | 0.9 | 2.03 | 4.2 | 0.33 | 0.00 |
| Building tangent-basis | 1.88 | 0.9 | 1.39 | 0.9 | 0.22 | 0.00 |
| Building rcast-CFORM model | 0.99 | 0.7 | 0.70 | 2.2 | 0.22 | 0.00 |
| Resolving materials | 0.64 | 0.9 | 0.23 | 0.9 | 0.00 | 0.00 |
| Build UV mapping | 1.31 | 0.9 | 0.66 | 3.5 | 0.22 | 0.00 |
| Subdividing geometry | 0.98 | 0.9 | 0.17 | 9.9 | 0.22 | 0.00 |
| LIGHT: LMaps | 115.20 | 5.9 | 21.00 | 13.5 | 7.70 | 1.66 |
| LIGHT: Vertex | 4.25 | 1.8 | 0.59 | 4.6 | 3.05 | 0.00 |
| LIGHT: Merging lightmaps (+ Saving) | 30.51 | 1.0 | 17.95 | 12.8 | 0.86 | 0.11 |
| Merging geometry | 20.67 | 3.2 | 5.72 | 10.1 | 0.22 | 0.00 |
| Converting to OGFs | 53.61 | 1.0 | 3.95 | 12.3 | 0.88 | 0.02 |
| Building sectors (+ Saving) | 2.78 | 0.8 | 0.48 | 1.2 | 0.55 | 0.00 |
| total | 261.22 | 3.8 | 57.76 | 11.3 | 18.09 | 2.16 |

The detail compiler on mp_close_combat: xrDO_Light 74.51 s at load 1.5, the Rust 9.95 s at 13.1. Without a C++ end:
l03u 137.7 s whole in Rust (LMaps 84 s, merging 34 s) against more than 600 s; mp_close_combat 299 s (the implicit
light 152 s at 14 cores) against more than 600 s in the C++'s implicit light alone. `compare_compile.phase_table`
fails a Rust phase or run longer than the C++ one by more than 0.25 s. The C++ sets its own process to idle priority
in its log loop and lights on six threads, hence its loads.

**What the comparison found and was fixed** (in `gz-lc` unless named):

- The MU and level pools: the C++ sorts vertices, faces, adjacency and edges by address in a dozen places
  (tesselation, smoothing, `BuildCForm`, the tangent basis, `export_cform_game`, `calc_normals`); `gz-lc-light`'s
  `PoolSim` gives every vertex and face the address its `poolSS` (LIFO free list) gives, the MU models chained through
  one pool, `reload_vertex_pool` after the subdivision, the adjacency sorted by it.
- `random_shuffle` of `LMapsLocal`: the main thread's `rand()` never seeded before it, so the stream is fixed and the
  page layout follows it (`MsvcRand::shuffle`; this replaces 8a's "the deflector list is not shuffled").
- MSVC's `std::sort` (`gz-lc-light` `crt.rs`) for `calc_materials`, whose order of equal materials the MU OGFs follow.
- The tangent basis took two texture coordinates within `EPS_L` as one; `find_same_vertex` uses `fsimilar`'s `EPS`.
- `ValidateMergeLinearSize`'s `4*c_SS_maxsize/3` is integer arithmetic: 42 m, not 42.67.
- `MergeGeometry` takes only a volume below `flt_max`.
- gz-fmt-ogf's VIPM stops at `max_window` (`vipm_convert`), as `CalculateAllCollapses` does.
- Speed, each the same bytes: the VIPM collapse errors cached per point (the C++ caches the last point's quadric;
  `gz-fmt-ogf`); the stripify fast path's face order linear-time; `MergeGeometry` by `FaceEqual` key; the UV mapping's
  face areas taken once; `SimplifyCFORM`'s constraints and every edge rating in parallel, the face marks counted
  instead of written; `CollectorPacked`'s weld through a hash grid that answers the same first vertex
  (`gz-fmt-level`, tested against the cell scan); the multiple-edge search in parallel. Converting to OGFs went from
  97 s to 4 s on l12u, the whole compile from 167 s to 58 s.

**Decisions:**

- D14 as built: an order the C++ takes from its pools or from a fixed `rand` stream is reproduced (above); one it takes
  from the heap is stated as a rule: QSlim's edges (`xr_new`, here in the order made), and the pool blocks themselves -
  l03u's C++ face pool put its third block below the first two (the heap reused freed memory), so from there its face
  order sorts the newest faces first; PoolSim assumes blocks in the order they are made. l12u and mp_test are not
  touched by it.
- The MU light runs whole inside "LIGHT: Starting MU" on every core; the C++ starts a thread there and waits for it in
  "LIGHT: Waiting for MU-thread". On mp_close_combat that is 5.5 s.
- `build.err` (the invalid-face report) is written into the level folder, beside the files.
- `xrPhase_AdaptiveHT_qslim.cpp` is commented out in the C++ (the tesselation alone is the phase).
- The C++ log window is not read for times (its thread answers one message per 200 ms); the compilers run with
  `-flushlog` and their log file is followed instead.

**Untouched:** `find D:/XFinedProjects "D:/Games/Dead Air/modules" -newermt "2026-11-12 13:27:30"` found nothing
before the first run (13:37) and nothing after the last (15:48): 0 entries both times. No process of the
compilers is left, no `gzlc_*` stage under `%TEMP%`; the scratch copies are gone.

## Phase 8c: the compiler in the editor, no C++

The brief `docs/rust/briefs/lc-final.md`, 12 Nov, sections 1, 2 and 4; section 3 (the deletion, `Build.ps1`, the
documents, `qa_target.py`, the icons, the sweep of the other crates' tests) went to a second engineer on the branch
`phase8-cleanup` (the lead's split, 16:3x). Before the first run (15:53:50): `find D:\XFinedProjects "D:\Games\Dead
Air\modules" -newermt` ten minutes earlier - nothing; no editor, game or compiler of ours running.

**The oracle compilers.** `Bin\x64\CppCompilers`: a build of the tag `cpp-compilers-last` (now commit 0ba98df4,
the lead's: under `-silent` XrLC and xrDO_Light create no window at all - `cl_log.cpp`, `xrLC.cpp`, `xrDO_Light.cpp`;
window code only). `diff_compile.py` starts them from there and nowhere else (hidden startup info, `-silent`), and
checks while one runs that it owns no top-level window (EnumWindows by its process id, visible or not): mp_close_combat's
two xrDO_Light runs, "0 windows of its own". A run that made one fails its record. The XrLC records below were made
by the build of 16:2x, before that commit; the lead proved its outputs equal on mp_test (window code only). The key of
a record names the tag (`CPP_SOURCE`), not the binaries' stamps (`FORMAT` 3): a rebuild of the tag keeps the records,
and with no record and no oracle binary a scene fails with a sentence saying where the compiler is to be built.
Records are never deleted by the tool now; the August ones (FORMAT 2: `compile-mp_test-3798f398b6a05401`,
`compile-l12u_control_monolith-46e7b6d5bcfa14a3`, `compile-mp_close_combat-ccf94d1b52e339cf`) are left for the lead (closed: no such record is under `Intermediate/oracle` any more).

**The August build against today's** (the same input, `oldnew` of the records): mp_test - `build.lights`, `level`,
`level.geom(x)` equal, `level.cform` 4 of 24 triangles another triangulation of the same surface; l12u - light equal
texel for texel, `level.cform` 41 triangles one side alone has, the sliding windows' reserved words. What differs is
what the heap orders (QSlim's ties) and memory nobody sets: two builds of one source differ exactly there. Today's
is the oracle.

### 1. MU models against the C++ compiler

**The scene: `l03u_agr_underground`** (7 MU models, 15 references, LODs), its copy's `[build_params]` set before Make
All (`diff_compile.py` `OPTIONS`): `light_pixel_per_meter` 4 (was 10), `light_jitter_samples` 1 (was 9); the quality
stays High, because Draft takes the MU light to one sample (`xrMU_Model_Calc_lighting.cpp`) and skips progressive
meshes (`OGF_Face.cpp`). Both compilers also get `-noise` (`FLAGS`): no progressive meshes for the STATIC visuals
(`xrFlex2OGF.cpp`); the MU models keep theirs (`xrMU_Model_Calc_ogf.cpp` calls `MakeProgressive` unconditionally).
Without it XrLC passed 600 s in "Converting to OGFs" (one thread, the static VIPM) with the August binary, after
LMaps had shrunk from 513 s to about 100 s; the static progressive meshes are proven on l12u. `LIMITS` gives the
scene 1200 s a run (both together within the brief's forty minutes); it needed 186.8 s (688.6 s CPU) and 545.3 s
(890.5 s CPU) - XrLC sets its own process to the idle class, and a cargo release build of the other engineer had
every core during the second run, so its wall time says nothing (CPU seconds are given beside every C++ time).

**What differed, and what was done:**
- **The MU models' own light** (each model vertex's hemisphere, `level.geom`): mean 6.85, p99 36, max 86 per channel
  against the C++ - whose two runs are equal. The C++ base thread lights the models one after another from the one
  `::Random`, holdrand 1 at the start (x64's `FPU::initialize` seeds nothing), three draws per sample (`random_dir`
  with a cone), six samples per vertex, nothing else drawing meanwhile. Now `light_mu` gives every model the
  generator as its first draw finds it and every vertex jumps to its draws (`XrRandom::advanced`, the LCG composed in
  O(log n); `MuModel::light_draws`): the vertices still light in parallel. After: mean 0.002, max 1.
- **The static geometry from the tesselation on** (`build.cform`: 3082 triangles from face 512137 on, 16 split
  points elsewhere; then `level.cform`, 179 visuals' containers, the vertex buffers): the split order follows the
  face pool's addresses, and today's C++ put the face pool's block made after the load (block 63, the tesselation's)
  BELOW the 63 blocks of the load - `xrLC` reads `build.prj` into one buffer, loads the faces above it and frees it
  when `CBuild::Load` returns (`FS.r_close`, xrLC.cpp), and the later block takes that space. `PoolSim::mark_loaded`
  (called at the end of `load`) orders the face pool's later blocks below the load's; the vertex pool keeps rising
  order (tried: modelling it too breaks l03u and l12u - the faces take the space first). After: `build.cform` byte for
  byte, 1192 of 1219 visuals byte for byte, the rest under the rules; l12u and mp_test pass with it as before.
  Proven by experiment first: blocks 0..2 swapped changed nothing, block 63 moved below made `build.cform` equal.
- **The references' light** (a tree's `c_scale` / `c_bias`, a LOD's corner colours): the C++'s own two runs differ
  there (cpp1 / cpp2: 12 of 15 trees' `OGF_TREEDEF2`, all 15 `OGF_LODDEF2`). The C++ keeps a vertex's light in the
  MODEL its references share (`xrMU_Model::calc_lighting` writes `_vertex::C`, then reads it back for the group mix)
  and lights the references on four threads plus one for the rest (`CMULight`, `get_intervals(4, n)`): a reference
  whose model has another reference on another thread takes some of that one's light. Seen: the four LODs that differ
  most (3, 4, 7, 13) all belong to models of several references lit on several threads; the Rust light of a
  reference does not move with its seed (another seed: mean 0.54). Rust lights each reference by itself (the intent;
  plainly broken C++). The rule (`compare_compile.cmp_mu`): references whose model the C++ lights on one thread only
  are judged under the vertex bounds, the others are reported.
- **The LOD corners' three padding bytes** (`OGF_LOD::_vertex`, never set: cpp1 and cpp2 differ) are left out.
- **`level.cform`**: QSlim's tie order (heap) differs between cpp1 and cpp2 here too, in the count (7) and in up to
  8.2e-4 of a material's area; the rules now judge the count and the area that changed material against the C++
  runs' own (see "For the plan"), and a re-triangulation of the same surface (mp_test against today's build) passes.

**The proof** (`python tools/qa/diff_compile.py --scene l03u_agr_underground`, from the record): PASS. MU: 15
references of 7 models, 12 lit on several C++ threads; the models' geometry (vertices but their light, indices,
sliding windows, xforms, LOD quads) byte for byte; the models' own light mean 0.002 / p99 0 / max 1; the three
references lit alone: trees mean 0.0008 / max 1, LODs mean 0.046 / max 1; the twelve others (reported): trees mean
25.2 / p99 191 against the C++ runs' own 21.6 / 150, LODs 3.1 / 25 against 0.95 / 11. Lightmaps mean 0.62 / p99 8
at most (2.a hemi 0.51 / 8), static vertex light 0.018 / 0; `level.cform` 431 triangles one side alone has (the C++
runs: 547), 8 more on one side (theirs: 7), 0.0244 m2 changed material (theirs: 0.0228). The pictures
(`Intermediate/compile/l03u_agr_underground/pictures`) were looked at: the same light and shadows, the difference a
speckle of 5-bit steps. Rust 34.6 s (329.6 s CPU) against the C++'s 186.8 s (688.6 s CPU).

### 2. The editor compiles with it

- **In process, as a job** (`crates/gz-levelbuild/src/compile.rs`): `compile_geometry`, `compile_details`, the new
  `compile` (and the command `level_compile`) run `gz_lc::compile` / `compile_details` on the project's
  `gamedata\levels\<level>` as the job `level_run` of the group `game` (one at a time with the game, its card in the
  Task Center, `level_build stop` stops it), on the editor's CPU pool (`editor.exec.cpu`: every core), data from the
  editor's `fsgame.ltx` (`Sources::of_root`). The compiler's phases are the job's stage, its progress the card's
  fraction, its log lines the Output Log; all three are passed on at most four times a second (`Throttle`). The
  answer: `{started: "gz-lc", level, parts, job, report {level, dir, threads, seconds, parts [{part, seconds, phases,
  files, warnings, invalid_faces}]}}` when it ends, or at once with `wait=false`. `build_all` makes the build and
  compiles the level in the same job (`{build, compile}`). The Build menu's Compile rows show the report in the kit's
  Result dialog (`dlg.compile_result`: time, each part, files, threads, invalid faces, warnings). `gz_lc::Report` gained
  `warnings` (`Log::warn`: invalid faces kept, junctions, multiple edges, an oversized subdivision) and
  `invalid_faces`; a compile refused for invalid faces writes `build.err` first, as `err_save` does.
- **Gone**: the compilers' stage (`runs::stage_compilers`, `compiler_fsgame`, `temp\compilers`, the copies of
  `Bin\x64\Release`), `Program::XrLc` / `XrDo`. `rg -i "xrlc\.exe|xrdo_light\.exe" crates tools` finds only
  `diff_compile.py` / `compare_compile.py` (the oracle). xrAI (`compile_aimap`, `compile_spawn`) still looks for
  `xrAI.exe` beside the editor: no build makes one since the cutover, so those two rows answer "not there" as before.
- **MCP**: `crates/gz-mcp/tools.json` - `grizzly_level_build` describes the in-process compile and takes `compile`,
  `level`, `details`, `wait`; `grizzly_level_compile` is new. `check.sh`: every catalogue command is registered.
- **The window while it compiles** (`tools/qa/measure_compile.py`, the fast build, l12u_control_monolith, a copy of
  Test on a hidden desktop, the Level tab up, a frame asked for every quarter second; `Intermediate/compile/measure*`):
  idle - frame work mean 2.1 ms, p95 2.7, max 4.2, load 0.95 of 16 cores; compiling - frame work mean 3.2 ms, p95 4.0,
  max 12.2, no frame over 16.7 ms, the process at 8.3 - 10.3 of 16 cores. The compile in the editor took 80 - 109 s
  (runs at different moments of a busy machine) against 64 s headless right after; with the Preferences tab up (no
  viewport drawn) 70 s. The difference is the viewport: the window draws at its active rate (the frame pacing's idle
  cap; over 4096 frames in each window) and each frame of the Level tab draws the scene, part of it on the same CPU
  pool. Passing every progress report on (each wakes the window) cost 93 s; the throttle brought it to 80 s. Nothing
  was made smaller; the drawing rate is the shell's (For the plan).
- **QA**: `test_scene_tools` phase C (mp_test: Make All, a compile started with `wait=false` and stopped - the job ends
  `cancelled` -, then one over MCP ends `done`, its report names the phases and the files, the files are in the
  level folder): 9 / 9 (debug build, the compile 2.0 s on 16 threads).

### 3. Play in the game

`tools/qa/test_play_game.py` through `run_all.py test_play_game`: PASS, 88.5 s - the editor's level compiler compiled
mp_test in its process (0.2 s, 16 threads), the game created the new game from `editor.spawn`, made the player,
connected, built and read the level's collision and geometry, ended its synchronisation (the level in 41 s after
Play), no assert, no dump, still running when stopped.

**Play's root keeps copies of the executables** (the lead asked for the installed `xrEngine.exe` run in place with
`-fsltx`, unless the engine writes into its own folder whatever the fs config says - it does). A `-fsltx <root>\fsgame.ltx`
moves `$fs_root$` (the folder of that file: `CLocatorAPI::setup_fs_path`), and with it the app data, logs, saves, the
shader cache, `$game_data$` and the modules root (`xms_core.cpp`: `$fs_root$\modules`); the archives could be named
in place. But the game's content service works in the folder of `xrCore.dll` (`Core.ApplicationPath`,
`ContentService.cpp`): every start's verification (`StartVerify`, also from the play gate `PlayBlocked` of a level
start) ends in `ContentState::SaveCache`, a truncating rewrite of `<application>\.dead-air-x64\content-state.txt`,
and a level starts only while `$arch_dir$` is `<application>\database` (else "$arch_dir$ points at ..." latches the
install incomplete and blocks play). So the game root keeps copies of the executables and its junction to the
archives. Proven by the suite: the root's `content-state.txt` is rewritten during Play (a check), and a listing of
the installed game (path, size, write time; 107,624 entries, links listed, never followed) before and after the
run is equal (a check that fails on any difference).
- The test of `play.rs` writes no program file any more: `prepare_with` takes the game's programs
  (`GamePrograms`: engines and extensions; `LINKED_GAME` is Dead Air's) and the test uses `engine.bin` / `core.lib`.
  Nothing else in `crates/gz-levelbuild` writes a program; `rg -n "Command::new|spawn" crates`: what the editor starts
  is the linked game's engine (the copy in Play's root), xrAI, `cmd /c mklink /J`, `taskkill`, `explorer.exe`,
  `reg.exe`, `gh` - no copied program but Play's engine.

### 4. Proof

Once, on the final tree of this pass, 17:40 - 17:57, against the debug build (the release build and the icon are the
other engineer's and the lead's); the compile comparison drives `Intermediate\cargo\fast\Grizzly.exe` as it always did:
- `bash tools/rust/check.sh`: clippy clean, fmt clean, `unsafe` only in `gz-win` (28), one file-dialog function, every
  catalogue command registered (`level_compile` among them); 38 s.
- `bash tools/rust/test.sh --all`: 77 binaries run, 0 cached, 0 failed, 0 skipped; 1155 tests green; 131.6 s.
- `python tools/qa/diff_editors.py --all`: 10 scenarios, 755 steps, 0 differ; 746 written files, 0 differ; 62.5 s.
- `python tools/qa/diff_compile.py --all`: mp_test, l12u_control_monolith, mp_close_combat, l03u_agr_underground PASS
  from today's records (no C++ started); 245.3 s. Rust 2.41 / 64.81 / 9.34 / 28.19 s against the C++'s 20.09 / 504.78 /
  78.39 / 186.80 s wall (11.6 / 1390.7 / 115.4 / 688.6 s CPU; the l12u record was made on a loaded machine - the
  August record of the same input took 261 s).
- `python tools/qa/run_all.py --with-game`: 23 suites, 22 passed, wall 178.7 s; `test_play_game` 63.2 s. The one that
  failed, `test_mcp_bridge` ("listed 198, catalogue 199"), drove a `grizzly-mcp.exe` built at 11:18 that embeds the
  catalogue of then (`include_str!`); rebuilt (`cg.sh build -p gz-mcp-bridge`), the suite alone: PASS, 3.0 s.

At the end (17:57:04): `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 15:43:50'` - nothing.
No Grizzly, xrEngine, XrLC, xrDO_Light, LevelEditor or github_mock process left; no `gzlc_*`, `gzmc_*`, `gzpg_*`,
`gzpr_*`, `gzqa_*` or `grizzly_*` folder in the temp folder; no `.part` record; no `gamedata\levels` in the repository.
The repository root's `_projects.ini` (ignored by git) is an old C++ recording's (it names a `grizzly_diff_*\s6_scene\cpp`
copy), there before this pass; `run_all.py` puts it back byte for byte. Nothing under `Bin\`, `rawdata\` or `gamedata\`
was written; under `Intermediate\oracle` only the four new records.

### Decisions taken in this pass

- The MU scene's build options and `-noise` (above): the cheapest settings that leave the MU work as it is.
- The MU models' jitter reproduces the C++ stream; the references' light stays per reference (the C++ races).
- The face pool's later blocks sort below the load's (`PoolSim::mark_loaded`): a model of the oracle's heap, stated
  as `// oracle:` at `PoolSim`; the vertex pool is not modelled.
- The compile in the editor answers when it ends (like Make All), `wait=false` at once.
- Play keeps its own root with copies of the executables (the content service's write).
- `diff_compile.py` keys records by the tag and never deletes one.

### Phase 8c, the cleanup: the last C++ leaves, one build, the icon, tests create no program files

The brief `docs/rust/briefs/lc-cleanup.md`, 12 Nov, branch `phase8-cleanup` (a worktree of the repository at
`a2a0a9c5`). Nothing was committed: the lead commits and merges. The tags `cpp-editor-last` and
`cpp-compilers-last` keep every deleted file.

### 1. The last C++ leaves

**Deleted** (with the file system; the index still lists them until the lead's commit): 1,084 tracked files,
346,390 text lines and 4 binaries (`SDK/Lib/x64/d3dx9.lib`, `DxErr.lib`, `Source/External/RedImage/ISPC/win/ispc.exe`,
`.vs/slnx.sqlite`).
- `Source/` (1,050 files: `Editors/{XrLC, XrLCLight, xrDO_Light, XrDXT, XrETools, XrQSlim, FreeMagic, Public}`,
  `XrCore`, `XrCDB`, `XrAPI`, `External/{zlib, LZO, Ogg, Vorbis, RedImage, Public}`, the two `.rc`, the notices),
  `SDK/` (24), `cmake/` (5), `CMakeLists.txt`, `CMakePresets.json`.
- `.vs/` (3: `CMake Overview`, `ProjectSettings.json` naming the CMake configuration `x64-Debug`, `slnx.sqlite`):
  Visual Studio's state of the CMake project, read by nothing else.
- Of them C and C++: 1,059 files (608 `.h`, 364 `.cpp`, 52 `.c`, 25 `.inl`, 6 `.ch`, 4 `.hpp`), 338,835 lines.
- `.gitignore` (`Source/.vs/`, `Source/External/RedImage/libraries/`) and `.gitattributes` (the LZO rule) lost their
  lines for the deleted folders; `tools/lint/check_encoding.py` its two skips of them.

**No C or C++ is tracked**: of `git ls-files` minus the deleted paths, no `.c .cc .cpp .cxx .h .hpp .hxx .inl .ipp
.ch`, project (`.vcxproj`, `.props`, `.sln` of C++) or CMake file is left but the HLSL of `gamedata/shaders`
(37 `editor/*`, 128 `r5/*`: shader headers named `.h` and the HLSL shared-items project `Shaders.sln` /
`Shaders.vcxitems(.filters)`). They are shader data of the kept C++ editor binary (its renderer reads them from the
repository root while it records an oracle), not C or C++, and nothing under `gamedata\` is deleted by these passes.

**Notices.** The Rust editor ships three pieces of third-party C or C-derived code, found from the build graph
(`cargo tree -i cc -e normal,build`: `luajit-src` through `mlua-sys`, `aotuv_lancer_vorbis_sys` and `ogg_next_sys`
through `vorbis_rs`; `miniz_oxide` and `texpresso` are Rust) and from the sources: LuaJIT, libogg/libvorbis with
aoTuV, and the LZO1X port of `crates/gz-chunk/src/lzo.rs` (statement for statement from LZO 2.10, GPL v2+).
`licenses/THIRD_PARTY_NOTICES.txt` is the old notices rewritten for those three (zlib and Info-ZIP MiniZip went with
the compilers); `licenses/LZO_LICENSE.txt` is `Source/External/LZO/COPYING` byte for byte. The CMake build used to
put both beside the binaries; `Build.ps1` does now.

**`Build.ps1`** builds the Rust editor and nothing else: `cargo build -p grizzly -p gz-mcp-bridge [--release]` with
the settings of `cg.sh`, then copies `Grizzly.exe`, `Grizzly.pdb`, `grizzly-mcp.exe`, `grizzly_mcp.pdb` and the two
notices into `Bin\x64\Release` (`-Debug`: `Bin\x64\Debug`; `-Out <folder>`), nothing else written or removed there,
and prints what it copied and the seconds. Gone: `-Only`, `-Target`, `-Clean`, `-KeepGoing`, the developer shell,
`<hash_map>` toolset search, CMake, Ninja, the encoding gate (now a step of `check.sh`, below). Visual Studio's C++
workload is still needed: `cc` compiles LuaJIT and libvorbis with `cl`, and the link takes the Windows SDK's libraries.
Its run in this copy: 386.9 s for the first release build here, from nothing (cargo 6 min 26 s); 1.0 s again with
nothing changed. `Grizzly.exe` 73.0 MB, `grizzly-mcp.exe` 0.7 MB.

**References to deleted paths** (`Intermediate/cleanup/refs.py` over the tracked files that stay, the records -
briefs, `PARITY.md`, `STATE*.md` - aside): none left but
- Rust comments naming `Source/...` as the oracle of a function: kept, `docs/rust/CONVENTIONS.md` says once that they
  mean the tags;
- documents that already name the tag (`docs/NQ_ARCHITECTURE.md` 3.3, `docs/UI_DESIGN.md`);
- `docs/RUST_MIGRATION.md` (the lead's: see "For the plan"), `crates/gz-levelbuild/src/runs.rs` and
  `tools/qa/diff_compile.py` (the other engineer's), `tools/qa/compare_compile.py` (`XrLC.exe` / `xrDO_Light.exe`
  as the names of the C++ runs inside the records);
- unrelated strings that only look alike (`Source\WAVE name` of a sound source's properties).

**The comparison tools and the oracle binaries.** `tools/qa/qa_target.py`'s C++ target, `diff_editors.py`,
`tools/mcp/run_benchmark.py` and `tools/qa/bench_build.py` start the C++ editor from
`Bin\x64\CppEditor\LevelEditor.exe` (the kept self-contained copy) instead of `Bin\x64\Release`. `diff_editors.py`
keys its records with the fixed `CPP_STAMP`, so the move invalidates none; a scenario without a record and no binary
there is now said plainly and left out ("no record and no oracle binary to make one"), as a busy port already was;
the two benchmarks answer the same way.

**Documents.** `README.md` (Building), `CLAUDE.md` (the C++ bullet of "How the editor is built", `check.sh`'s list,
the Build paragraphs: one build, the icon, no program files in tests; the MCP safety paragraphs untouched),
`tools/mcp/AI_SETUP.md` (the bridge's build line), `docs/rust/CONVENTIONS.md` (the oracle is the tags, the two
`build.rs`, `gz-ico`, a Tests section), `tools/mcp/grizzly_mcp.py` (its hint named `-Only editor`).

### 2. The icon

- **`crates/gz-ico`** (new, layer 0, no dependency but `thiserror`, no `unsafe`): `images` reads an `.ico`,
  `icon_res` writes the compiled resource file of it - the empty 32-bit marker resource, every image as `RT_ICON`
  1..n, the `RT_GROUP_ICON` 1 over them, language 0x0409, the memory flags `rc.exe` sets - and `embed` is what a
  build script calls: `<OUT_DIR>/icon.res`, handed to the linker with `cargo:rustc-link-arg-bins` (`rust-lld` in
  its `lld-link` flavour takes a `.res` as an input and converts it itself). No resource compiler runs, no crate is
  downloaded, nothing generated is in the tree. Three unit tests (the directory, refusals, the `.res` layout).
- `crates/grizzly/build.rs` (the C runtime rule kept) and the new `crates/gz-mcp-bridge/build.rs` embed
  `assets/grizzly-editor-icon.ico`.
- **One source.** The window icon and the caption logo were the 512 px `.png` box-filtered to 64; they now read the
  same `.ico` the executables carry (`gz_app::chrome::logo_rgba`: `gz_ico::for_side`, the 64 px image as drawn for
  that size, filtered only when no image of the side exists). The `.png` stays as the generator's large render
  (`tools/branding/grizzly_editor_icon.py`); no code reads it.
- **The proof**: `tools/rust/check_icon.py <exe>...` reads each executable's first `RT_GROUP_ICON` from its resource
  section, wants as many images as the `.ico` holds and the largest one byte for byte equal to the `.ico`'s largest
  (256 px, 9,856 bytes). On the release build: "Grizzly.exe: the editor's icon (10 images, the 256 px one
equal to the .ico's)", the same for `grizzly-mcp.exe` (and on the debug build before it). A negative control: a build script's executable answers "carries no icon: no
  resource section", a missing file "cannot be read". `check.sh` runs it on `Bin/x64/Release` when both binaries
  are there and says so when they are not.

### 3. Tests create no program files

`tools/rust/check_test_files.py` (a step of `check.sh`): test code is every file under a crate's `tests/`, every item
under `#[cfg(test)]` (a `mod name;` takes its whole file) and every `#[test]`; in it, a string literal naming a file
with a program's extension (`.exe .dll .com .scr .bat .cmd .ps1 .msi`; `.com` not inside a URL), a bare extension
literal (`"exe"`, ... for `with_extension`), a literal starting with `MZ`, or the bytes `b'M'` .. `b'Z'` /
`0x4D, 0x5A` written in code is a finding, unless its line or the line above says `// program-files: in memory`
(the data never reaches the disk). A light Rust lexer (comments, raw and byte strings, char literals) keeps comments
out.

On the tree as found (`git archive a2a0a9c5 crates`) it reports 13 lines in 5 files:
- `gz-release/src/game_version.rs` 138, 183, 185 - **wrote** a fake PE (`MZ`, a `.rsrc` with a version resource) as
  `xrEngine.exe` into the temp folder, then `b"not a program"` under that name. Changed: the reader takes any
  `Read + Seek` (`version_of`; `exe_version(path)` opens the file and calls it), the test reads the fake PE from a
  `Cursor` in memory and asks `linked_game_version` of a folder that does not exist. Nothing is written.
- `gz-editor/src/fsx.rs` 192 - a path `<game>\bin\xrEngine.exe` given to `WriteGuard::check` (never written):
  now `<game>\bin\fsgame.ltx`, the same rule.
- `gz-game/src/version.rs` 38, 47; `gz-release/src/content.rs` 580, 581, 598 - in-memory buffers (`exe_version` of
  bytes, `path_allowed` of a string, `bytes_allowed` of a buffer): marked `// program-files: in memory`.
- `gz-levelbuild/src/play.rs` 256, 257, 265, 266 - **writes** `xrEngine.exe` and `xrCore.dll` into a fake game: the
  other engineer's crate (the brief leaves it to them); on this branch the step fails on exactly these four lines
  until their change is merged.

### Proof (this copy)

- `bash tools/rust/cg.sh check --workspace --all-targets`: clean.
- `cargo clippy --workspace --all-targets`: 0 warnings; `cargo fmt --all -- --check`: clean.
- `bash tools/rust/cg.sh test --workspace --lib`: 1,124 passed, 0 failed (57 test binaries).
- `pwsh -NoProfile -File .\Build.ps1`: 386.9 s (release, from nothing); `check_icon.py` on both release binaries: the
  editor's icon, 10 images, the 256 px one equal.
- `check.sh`'s new block: encoding clean; test files: only the four lines of `gz-levelbuild/src/play.rs`; icon: both.
- Not run here (no data, no editor runs in this copy): the corpus tests, the QA suites, `diff_editors.py`,
  `diff_compile.py` - the lead runs them after the merge. (closed: run since, "The run on the final tree".)

### Decisions

- `.vs/` counted as "what only the C++ build read" (the IDE's view of the CMake configuration) and deleted.
- Section 1 does not name them, so the lead decided, and deleted with the rest: `T-6638.7z` (20 MB, an archive of
  old X-Ray sources - 7,512 C/C++ files inside it), `ccc__temp__vertices` (a debug dump `XrLCLight` wrote), the empty
  `engine.log`, the empty `.gitmodules`, `rawdata\{actor,particle,shader}_imgui_default.ini` and
  `rawdata\ActorEditorLevel.cform` (of the Actor, Particle and Shader editors, which no kept binary is),
  `gamedata\shaders\r2.zip` and the HLSL project files `gamedata\shaders\r5\Shaders.{sln,vcxitems,vcxitems.filters}`
  (nothing reads them at run time). The shaders themselves stay: the kept C++ editor draws with them when it records.
- `gz-ico` is a workspace dependency like every other crate (`Cargo.toml`, `gz-ico.workspace = true`).
- `Build.ps1` writes the two notices besides the four binaries: they are the distribution of those binaries (the
  CMake build wrote the same two names there), not a third program.
- The icon resource is written by a crate of ours rather than `winresource` / `embed-resource`: neither is in the
  lock, both run `rc.exe` from a Windows SDK they have to find; the `.res` format is 30 lines, and `lld-link`
  converts it.
- `tools/lint/check_encoding.py` was the C++ build's gate; it moves to `check.sh` (the gamedata half - cp1251 for
  the game's parsers - still means something; the C++ half finds no file now).
- The docs name `Bin\x64\CppCompilers` as the compilers' oracle, as the brief says; `tools/qa/diff_compile.py` on
  this branch still reads `Bin\x64\CppEditor` (`CPP_BIN`) - the other engineer's file: the merge has to make the two
  agree.

### The merge

The lead merged the branch into `grizzly-rust` after the other half (no conflict; `Cargo.lock` merged by itself).
`crates/gz-levelbuild/src/play.rs` writes no program file since that half, so `check_test_files.py` is clean on the
merged tree. `diff_compile.py` reads `Bin\x64\CppCompilers`, as the documents say. The untracked leftovers of the
C++ build went with it: `Source\External\RedImage\intermediate` (129 object files), `Lib\` (the import
libraries) and `Intermediate\cmake-*` (the CMake build folders).

## After phase 8c: the window while a job runs

The brief `docs/rust/briefs/shell-idle.md`, 12 Nov, branch `grizzly-rust`; nothing committed (the lead commits). Before
the first run (18:04): `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt` ten minutes earlier - nothing; a
release build of the lead's ran at the start and had ended before the first measurement (no cargo, rustc, editor or
game process of anyone's during them; only the idle MCP servers of the desktop app).

### 1. A scene pass only when its picture can change

**What asked for frames.** Three things, found with the frame causes `frame_pacing` answers now (below):
- **The view state's revision moved on every `Viewport::with`**, whatever the closure changed - and
  `sync_scene_camera` runs one every frame for the camera pose's save timer. Every UI frame's `BuildKey` differed, so
  every UI frame built a scene pass; the render thread's finished frame pinged the wake, which drew the next UI frame:
  a loop at the window's full rate with nothing moving. Measured before (the 17:33 run of the last pass): 1690 frames
  in 10 idle seconds and 0.95 cores idle; over 4096 frames in every compile window.
- **The window drew a frame at once for every repaint egui asked for, also for a later one.** The repaint callback
  (`gz-app`'s `window.rs`) pinged the wake whatever the delay, so the status bar's job chip, which asks for its next
  frame in 100 ms (`chrome.rs:1476`), drew every frame the window could present while a job ran: 9249 - 11010 UI
  frames in a 59 - 68 s compile (157 - 161 a second), every one `wake gz-app/src/window.rs:473`, with the scene
  passes already at 0.
- **The kit's spinner asked for the very next frame** (`gz-ui`'s `state.rs`), so the Task Center's card (and every
  busy footer) drew at the full rate too: 11717 UI frames in a 72.9 s compile with the card up, against 54 - 57 s
  without it.

**What decides a scene pass now** (`gz-ui-viewport`'s `service.rs`): one function, `due(last, now, stale)`, over the
`BuildKey` of the pass's inputs - the scene's snapshot (objects, selection, the tools' data), the library, the view,
the assets a pass waited for, the animation tick (only while something in view plays: a particle system, a motion,
Real Time), a spawn's rig or playback - and the shown packet going stale (a texture reloaded). It answers the first
input that moved, or nothing: the pane shows the picture it has. The view's revision moves only when what a pass reads
of it changed: `ViewState::drawn` (the camera and navigation, the view options, the tool and its switches, the
preferences, the pane's pixel size - scale and size in one -, the theme's canvas, Game Lighting's inputs); `with`
compares it before and after, so the 93 call sites keep their form. The bookkeeping beside it (the cameras kept per
scene, the pose's save timer, the status line, Place's picks, the pane's points) and what a pass publishes (the
anchor, the game lights drawn, the roof map) move nothing. An input outside the view state asks with
`Viewport::touch()` (Real Time switched, Reload Textures, a spawn's playback) - the three `with(|_| {})` that were
used for it.

**A job's progress wakes no scene, and draws at a rate of its own.** The repaint callback pings the wake only for a
frame asked for now; a later one is a `UserEvent::Later` that only sets the loop's `WaitUntil` (the soonest asked
wins). The status chip so draws ten frames a second, the open card twenty (its own 50 ms), the spinner thirty
(`gz_ui::state::SPINNER_FRAME`, 33 ms - chosen from the card run above: a spinner at every frame cost the compile
30 %, one at thirty costs nothing measurable, below). The job's stage and fraction ping as before (the compile's
`Throttle`, four a second). Nothing was made smaller or slower: no pool lost a thread, no priority changed (the
drawing thread stays above normal), the compile is never paused.

**What keeps working.** `screenshot_viewport` builds a pass of its own (`Reason::Picture`) as before; `screenshot_editor`
holds through the picture gate (`Viewport::current`), which asks `request_build` until a pass of the caller's scene and
view revision was drawn - a change it needs moves an input, so `due` answers it; `preview.png` is a pass of its own
(`Reason::Preview`); a minimized or covered editor draws its UI frames as before; back on Level the pane shows its last
target and builds a pass only if something moved meanwhile (a size change is an input); a camera moved during a compile
is a view change: 6552 passes in the 74 s orbit run below, about 88 a second.

**MCP `frame_pacing`** answers, besides what it did, for the window since its last ask: `window_seconds`,
`ui_frames`, `ui_frame_causes` (the frames by what drew them, most first: `repaint <file>:<line>` - egui's request, by
the line that asked first -, `wake <file>:<line>` - the code that pinged the window last, `gz_task::Wake::ping` keeps its
caller -, `input`), `scene_passes`, `scene_pass_reasons {first, scene, library, view, assets, animation, spawn, stale,
picture, preview}` and `last_scene_pass {reason, ago_ms}`. Its catalogue entry says so. `gz_editor::FrameStamp` carries
the frame's `cause`; the frame clock counts them.

**Measurements** (`tools/qa/measure_compile.py`, the fast build, `l12u_control_monolith` on a throw-away copy of Test,
the editor on a hidden desktop with the Level tab up and a frame asked for every quarter second; 16 cores; the
numbers in `Intermediate/compile/measure*.json`, the runs' logs beside them):

| Run (editor build) | Editor compile s | Headless s | Ratio | UI frames | Scene passes | Load editor / headless |
|---|---|---|---|---|---|---|
| last pass (before) | 80 - 109 | 64 | 1.25 - 1.70 | > 4096 / window | one per frame | 8.3 - 10.3 / - |
| view fix only, back to back | 59.0, 67.6, 68.2 | 53.9, 54.0, 53.5 | 1.10, 1.25, 1.28 | 9410 - 11010 | 0 | 11.3 - 11.6 / 12.2 |
| + repaint fix, back to back | 54.0, 59.5, 61.2 | 52.0, 51.3, 51.7 | 1.04, 1.16, 1.18 | 1052 - 1191 | 0 | 11.3 - 11.5 / 12.2 - 12.4 |
| same, a fresh editor per pair | 50.6, 59.9, 58.0 | 49.6, 51.1, 53.7 | 1.02, 1.17, 1.08 | | 0 | |
| same, 45 s of nothing before every compile | 55.5, 52.9, 56.0 | 49.3, 47.7, 47.7 | 1.13, 1.11, 1.18 | 1030 - 1090 | 0 | 10.8 - 11.1 / 11.4 - 12.1 |
| same, the editor `-headless` (no window), cooled | 47.2, - | 46.0, - | 1.03, 0.98 | 0 | 0 | 12.1 / 12.2 |
| same, the process taken out of power throttling from outside, cooled | 47.9, - | 45.8, - | 1.05, 1.10 | | 0 | 12.1 / 12.3 |
| **final** (`keep_full_speed` in the editor), one pair | **47.7** | **49.7** | **0.96** | 933 | **0** | **12.3 / 12.7** |

- **Idle** (Level tab, a frame asked every quarter second, 10 s): 99 - 123 UI frames, 0 scene passes, 0.04 - 0.06 cores
  (before: 1690 frames, 0.95 cores).
- **Frame work** during the final compile: mean 2.5 ms, p95 3.1, max 6.1, no slow frame. **With the camera orbiting**
  during a compile (`viewport_gesture` Alt+drags, one step a frame, the whole 74 s compile, the build before the last
  fix): 11889 UI frames, 6552 scene passes (all `view`), frame work mean 3.1 ms, **p95 3.9**, max 13.3, 0 slow frames;
  the compile took 73.6 s then (the passes share the pool, as they should while the author moves the camera).
- **The card up** (Task Center card, its spinner): 72.9 s with the spinner at every frame (11717 frames) - 60.1 s and
  2241 frames with it at thirty a second (the back-to-back series' fourth compile).
- **Frame causes during a compile** (final build): `wake gz-editor/src/frames.rs:128` (the harness's `ui_list`, 4 a
  second) ~ 36 %, `repaint gz-app/src/chrome.rs:1476` (the job chip, 10 a second) ~ 33 %, `wake gz-app/src/queue.rs:56`
  (the MCP requests' UI queue) ~ 18 %, `wake gz-task/src/jobs.rs:105` (the job's fraction, 4 a second) ~ 12 %.

**What was left after the frames were fixed: Windows' power throttling.** With no scene pass and 20 UI frames a second
the editor's compile was still 11 - 18 % slower than headless at the same clock (45 s of nothing before every compile,
the processors' speed 148 - 152 % of base on both sides, `% Processor Performance` sampled through PDH), and the loss
was in the short serial phases (Optimizing 0.98 s against 0.68, Building normals 0.75 / 0.47, tangent basis 1.96 /
1.47, Merging geometry 4.8 / 3.9), not in the parallel light (LMaps 20.6 / 18.8). The editor started `-headless`
compiled like the headless one (1.03, 0.98; every phase equal), and the windowed editor taken out of execution-speed
throttling from outside (`SetProcessInformation(ProcessPowerThrottling)`, the measure's `--no-throttle`) did too (1.05;
the phases equal again, load 12.1). Windows had put the process under EcoQoS because its window was not the
foreground one - here on the measure's hidden desktop; for the author, an editor behind another window or minimised
while a compile runs. Its threads then run at an efficient, lower speed, and the serial phases on one core show it
most. **The fix: `gz_win::keep_full_speed()`** (in `gz-win`, the one crate with `unsafe`, with its `// SAFETY:` line:
the current process's pseudo-handle and a live local of the size passed) opts the process out of execution-speed
throttling once, from `gz-app`'s `window::run` beside the drawing thread's priority; a failure is a warning in the log.
No priority is changed and no pool is touched. The headless editor needs none (no window, never throttled in the
runs).

The measure (`tools/qa/measure_compile.py`) gained `--pairs`, `--fresh`, `--cool N`, `--card`, `--orbit`,
`--headless-editor`, `--no-throttle`, `--no-headless`, the headless run beside each editor compile (`Grizzly.exe
-compile <folder>`, its load and peak memory), the phases of both sides, the processors' speed (`clock_percent`), a
settle before each compile, and the frame causes.

### 2. The door to xrAI goes

`level_build`'s `compile_aimap` / `compile_spawn` and the Build > Compile rows AI Map High / Low / Verify and Spawn
This Level / All Levels are gone, with `Program::XrAi`, `runs::find_exe`, the executable folders (`exe_dirs`),
`not_there`, `start_run`, the `mode` and `all_levels` parameters (nothing else read them) and their words in
`crates/gz-mcp/tools.json` and `docs/UI_DESIGN.md` (`tools/mcp/AI_SETUP.md` named neither). `runs.rs` is now
`process.rs`, the game alone: `arguments`, `command_line` (the log line, now with the engine's real file name),
`Ended`, `run`. What holds the `level_run` job is `service::Running { Compile, Game }` (two variants; `level_build
status` answers `running` as before), and `run_program` became `start_game`. `COMMANDS` has 17 entries, the Build
menu's rows are 8 (`BUILD_ROWS` lost its argument column: every Compile row is the in-process compile). In lines:
`cmd.rs` 84 out and 21 in, `menus.rs` 33 / 15, `runs.rs` (141) became `process.rs` (103), `service.rs` 5 / 22 (the
`Running` enum moved there), `lib.rs` 13 / 12 (the docs), `tools.json` 16 out (the two actions, `mode`,
`all_levels`, the xrAI words of `params`).
`rg -i "xrai\.exe|XrAi\b|compile_aimap|compile_spawn" crates tools docs/UI_DESIGN.md` finds one line: the comment of
`gz-levelbuild`'s `lib.rs` naming xrAI's `CGraphMerger` as the oracle of the game graph merge (kept, as the brief says);
the `XRAI_VERSION_*` constants stay. `tools/qa/diff_editors.py`'s scenarios and the suites named neither action
(`rg` over `tools/qa`: nothing before the change).

### 3. A suite never runs against a stale bridge

Both binaries embed `crates/gz-mcp/tools.json` (`include_str!`) and now answer `-catalogue` with one line,
`catalogue fnv1a64:<16 hex> tools:<n>` - FNV-1a 64 of the embedded bytes and the tool count, worked out only when asked,
before the editor opens its log (`gz_mcp::catalogue_line`; the bridge's own copy of the 4 lines, `gz-mcp-bridge` has no
dependency). `tools/qa/catalogue.py <exe>` runs `<exe> -catalogue -dump_commands` with no input (an editor older than
the flag prints its commands and exits instead of opening; an old bridge reads its empty input and exits), computes
the same line of the file and answers in one sentence:
- `...\grizzly-mcp.exe is older than crates/gz-mcp/tools.json (it does not answer -catalogue): build it again - bash
  tools/rust/cg.sh build -p gz-mcp-bridge` (the debug bridge of 17:56, before this pass);
- `...\Intermediate\cargo\fast\grizzly-mcp.exe was built from another crates/gz-mcp/tools.json (199 tools, other text):
  build it again - bash tools/rust/cg.sh build --profile fast -p gz-mcp-bridge` (built before the `frame_pacing` text);
- the command follows the binary's place: `Bin\x64\*` is `pwsh -NoProfile -File .\Build.ps1` (`-Debug`), a cargo
  profile folder is `cg.sh build` with that profile.
`test_mcp_bridge.py` asks it before anything starts and stops with `MCP BRIDGE: STALE - <the sentence>` (exit 2).
`check.sh`'s registry step asks the same of the editor binary before `-dump_commands` (`registry: stale binary`), and
its "unchanged since the last pass" shortcut now also needs the catalogue not to be newer than the stamp - before, an
edited `tools.json` beside an unchanged `GRIZZLY_QA_EXE` passed on the old stamp.

### Proof

On the final tree, 19:3x - 19:47, the debug build (the measurements the fast build):
- `bash tools/rust/check.sh`: clippy clean, fmt clean (after `cargo fmt --all` on three of my files), `unsafe` only in
  `gz-win` (29 uses: `keep_full_speed` is the new one), every catalogue command registered, encoding clean, no test
  writes a program file, the icon in both release binaries; 45 s.
- `bash tools/rust/test.sh`: 60 binaries run, 18 cached, 0 failed; 1162 tests green; 82 s. New tests:
  `gz-ui-viewport` `pass_tests` (`due` answers only a moved input; `drawn` ignores the bookkeeping), `gz-mcp` and
  `gz-mcp-bridge` the catalogue line (FNV-1a 64 reference values, the tool count).
- `python tools/qa/run_all.py test_ui_shell test_stress test_scene_tools test_previews test_properties
  test_project_browser test_mcp_bridge`: 6 of 7 at once (36 s); `test_mcp_bridge` failed E there - a quest edited over
  MCP right after `ui_tab open`, then closed: the graph tab took its loaded document only when it drew, and with
  frames no longer running all the time its `dirty()` answered from nothing. Fixed in `gz-ui-graph`'s `tab.rs`
  (`dirty()` also asks a document still waiting in the load slot); then `run_all.py test_mcp_bridge test_ui_shell
  test_stress` twice: 3 / 3 each (25 s). A third repeat failed `test_stress` with WinError 10048 on the client side
  (the stress suite's 5470 connections a run, three runs in two minutes: the machine's ephemeral ports in TIME_WAIT),
  not an editor fault; it passed in the other three runs.
- The measurements above. **Not met as the brief asks**: three back-to-back pairs within 10 % - the three-pair series
  ran before the power-throttling fix (1.04 / 1.16 / 1.18); the final binary got one pair (0.96), as the lead decided.
- At the end (19:47): `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 17:54'` - nothing; no
  Grizzly, game or mock process left; no `gzmc_*` folder in the temp folder. Nothing under `Bin\` written.

## The lead's run on the release build, before the final review

12 Nov, 19:49 - 20:02, commit `d0ea0509`, one after another (`Intermediate/final/`, `summary.txt` and a log per
step), `GRIZZLY_QA_EXE` and `GRIZZLY_LC_EXE` naming `Bin\x64\Release\Grizzly.exe`:

| Step | Result | Time |
|---|---|---|
| `Build.ps1` | the two programs, their PDBs, the notices | 230 s |
| `check.sh` | ok (clippy, fmt, `unsafe`, the registry, encodings, test files, the icon) | 18 s |
| `test.sh --all` | 78 binaries, 1162 tests green | 69 s |
| `diff_editors.py --all` | 10 scenarios, 755 steps, 746 written files, 0 differ | 41 s |
| `diff_compile.py --all` | mp_test, l12u_control_monolith, mp_close_combat, l03u_agr_underground PASS (the release build compiles l12u in 38.4 s, l03u in 22.4 s) | 208 s |
| `run_all.py --with-game` | 23 suites, 22 passed (`test_play_game` 68.5 s) | 167 s |

Nothing changed under `D:\XFinedProjects` or the game's `modules`; no process of the run left.

**The one failure was a real fault: a dirty quest tab closed without a refusal** (`test_mcp_bridge` E: `ui_tab
action=close` answered `ok` for a quest edited over MCP, and the edit was gone). A graph tab opened for a document
that is not open yet loads it on the IO pool into a slot and takes the handle in its next frame; `dirty()`,
`save_now` and `discard` asked the tab's own copy of the handle. With frames drawn on demand ("After phase 8c") an
edit over MCP reaches the document before any frame of the tab; the last pass's fix read the slot, which is still
empty while the tab's own open task has not ended - under the load of 23 suites side by side it had not. The
service holds the one handle of a path, so the tab now asks it (`GraphTab::shared`: its own handle once a frame took
it, else `NqService::get_doc`): a close refuses, Save saves, Don't Save reloads, whatever the tab has drawn. On the
rebuilt release binary `test_mcp_bridge` beside six other suites: 3 of 3 (5 of 5 alone on the debug build).

**`test_stress` kept a connection per request** (5,477 a run): three runs within two minutes, beside the other
suites, left 15,185 sockets in TIME_WAIT of the machine's 16,384 client ports, and the third run failed with
WinError 10048 on its own `connect` - the harness's fault, not the editor's. Each of its threads now keeps one
connection (the server answers any number of requests on one; a connection the server closed is opened again
once): three runs back to back pass, TIME_WAIT falls during them, and the flood is 18,782 requests a run.

The seven suites after these two fixes ran on the rebuilt release binary; the other sixteen and the four other
steps ran on the binary before them (`d0ea0509`), which differs in `crates/gz-ui-graph/src/tab.rs` alone.

## For the plan

Nothing: everything this section held is in `docs/RUST_MIGRATION.md` - the four notes the close sweep left
here (the scene-made entity's script block in section 9, the records' keys in 10.1, `gz-ltx` in section 7, the
tests' net over the repository in section 6) were folded after the last review. The layer table is "The
crates' layers" below, where section 7 points.

## The crates' layers

A crate's layer = 1 + the highest of its workspace dependencies (`cargo metadata`, normal and build ones), as of the
close sweep (13 Nov; `Intermediate/close_sweep/layers.py` makes it):

  | Layer | Crates |
  |---|---|
  | 0 | gz-base, gz-ico, gz-luadata, gz-win |
  | 1 | gz-chunk, gz-entities, gz-fmt-xml, gz-log, gz-ltx, gz-lua, gz-mcp-bridge, gz-task, gz-ui |
  | 2 | gz-corpus, gz-crash, gz-fmt-level, gz-fmt-object, gz-fmt-sound, gz-fmt-spawn, gz-fmt-xr, gz-query, gz-store, gz-vfs |
  | 3 | gz-editor, gz-fmt-ogf, gz-fmt-tex, gz-particles |
  | 4 | gz-app, gz-github, gz-lc-light, gz-mcp, gz-project, gz-render |
  | 5 | gz-game, gz-lc, gz-mod, gz-ui-app |
  | 6 | gz-build, gz-scene, gz-ui-content |
  | 7 | gz-nq, gz-pda, gz-release, gz-screens, gz-ui-previews, gz-ui-viewport |
  | 8 | gz-catalog, gz-characters, gz-publish, gz-ui-graph, gz-ui-level, gz-ui-mod, gz-world |
  | 9 | gz-export-level, gz-ui-pda, gz-ui-release, gz-ui-world |
  | 10 | gz-levelbuild |
  | 11 | grizzly |

  `gz-game` depends on `gz-editor` (its services) and `gz-project` (the project holds the game's mount): it is a
  domain crate above the project, not beside `gz-vfs`. Neither edge is cheap to turn round (the mount and the
  services are its whole subject), so the table, not the code, changes.

## Next packages, in dependency order (history: every one is done)

1. `text` - done.
2. `project` (`gz-project`: the list, project files, rules, file tree, content operations).
3. `kit` (the UI kit's widgets, toolbar, dialogs) and `icons`.
4. `game-link` (`gz-vfs`, `gz-game`, `gz-lua`) after 1.
5. `mod-build`, `nq`, `assets` (characters, PDA, screens, world), `release` after 2 and 4.
6. `tex-xr`, `ogf`, `spawn-entities` - needed by the scene and the renderer (phases 4-5), and
   `spawn` by the world rules.
7. The shell and the tabs (phase 3), the scene (4), the renderer (5), the level tools (6).

## The final review, Play and the tests

The brief `docs/rust/briefs/final-play.md`, 12 Nov, branch `grizzly-rust` (HEAD `253a35de` plus the other engineer's
uncommitted compiler work); nothing committed. Before the first run (21:08:40): `find D:\XFinedProjects
"D:\Games\Dead Air\modules" -newermt` ten minutes earlier - nothing; no editor on 28016 or a QA port.

**1. Play never destroys what it did not make.** `gz-levelbuild`'s new `play_root.rs`: `check_root` stands in front
of every Play that names or makes a root - `play`, `play_game`, `play_clean` and the new `play_root` action (makes or
refreshes the root, starts nothing; what `test_play_game` uses before it writes the root's settings) - before the
scene is built or a byte written, and `play::prepare` asks it again. The questions are the deletion's: the same
`Guard`s from `gz_project::system_guards` (the exe's folder and the data root, the user's folders, Windows, Program
Files, Temp, the linked game, every listed project's game, editor and export target) and the same comparison - is,
holds, lies inside -, as written and where the path really is (a missing root's real path is its nearest existing
folder's, resolved); on top, the open project and every listed one as guards taken inside, any folder on the way
holding `project.ltx` / `mod.ltx` (a project nobody listed), a link, a drive or volume root, a relative path, a file.
Then the content: a missing or empty folder is made Play's - the marker `play_root.txt` written first -, a folder
with the marker is Play's, anything else is refused untouched: "<root> is not a Play root: choose an empty folder.
It holds N entries (...) and no play_root.txt, so Play leaves it as it is."
- **What Play clears in its root, and nothing else:** `gamedata\` (copied anew; a link in it unlinked), the staged
  level, the programs the marker lists that the game no longer has, the content latch; it overwrites `fsgame.ltx`
  and the programs it copies. `appdata\`, `modules\`, the archives' junction and anything an author put there stay.
  `stage_level` writes only into a marked root.
- **Every refusal before the disk changes:** the path length is now planned (the game's gamedata, the appdata
  caches, what a marked root keeps, the staged level, the archives' longest path, the programs) and checked before
  the root is made.
- **The default root moved** out of the editor's data root (`_appdata_\play\<project>` lies inside the editor, which
  the checks refuse): `%LOCALAPPDATA%\GrizzlyPlay\<FNV-1a of the project's path, 8 hex>` (49 characters here; the game
  takes roots up to about 66). No old root existed on this machine.
- **What is copied:** the engine, the libraries the game's own runtime list names (`.dead-air-x64\runtime-files.txt`,
  kept by its installer and updater; without it every `.dll` of the folder), and every library of the folder any of
  them imports, ordinary or delay-loaded (`pe.rs` reads the import tables, headers only) - never another executable.
  The import tables matter: `xrEngine.exe` imports `xrRender_R6.dll`, which the list lacks. Before: 76 files, 262.8
  MB (every `.exe` / `.dll`); now 57 files, 246.1 MB. Left out: the updater, JSGME, DarDelta, both uninstallers (2.9
  MB - the brief's "250 MB" was the libraries, which the engine needs) and 14 libraries nothing lists or imports
  (R1-R3, `xrD3D9-Null`, `xrXMLParser`, `lua51`, ...).
- **Proof:** 12 unit tests (`play_root.rs` 6 - each refusal, the folder listed equal before and after, a junction
  refused and met where it really is; `play.rs` 4 - the root, the marker, a dropped library removed while an author's
  file stays, the fallback list, the import closure, every refusal before a change; `pe.rs` 2), no program file
  written. `test_scene_tools` phase Y: 18 refusals (`play_clean`, `play_root`, `play` × a throw-away project folder,
  its gamedata, a folder holding a file, a folder inside the open project, the repository, a relative path), the
  folders listed equal, nothing started; 0.4 s. The reviewer's `play_root_demo.py` rerun on the debug build
  (`Intermediate/final-play/play_root_demo.out`): refused "it is a project folder", `precious.txt`, `my_level` and
  the folder's `fsgame.ltx` there, 2.5 s. `tools.json` says what `root=` takes and lists `play_root`.

**2. No test writes a program file - held at run time too.** `check_test_files.py` takes `"cmd"` and `"com"` as
extensions where they are one (the argument of `with_extension` / `set_extension`: elsewhere they are a JSON key,
the shell `Command::new` starts, a URL) and says in its header what it cannot see. `tools/rust/watch_programs.py` is
the net: `test.sh` gives the binaries `TEMP` / `TMP` = a `gztest_*` folder of the run, watched through
`ReadDirectoryChangesW` (ctypes, recursive, 1 MB buffer, armed - proven by a mark's own event - before the first
binary starts); a file created, renamed in or written there with a program's extension or an `MZ` head (two bytes,
read with every sharing flag) fails the run, naming the file and the binaries whose run spans its time; those
binaries lose their green stamp. An overflowed buffer walks the folder instead. The watcher removes the folder
(links unlinked, never followed) and reports last. Cost in the run below: 589 events, 0 overflows, 0.81 s of the
watcher's CPU beside 16 cores, about 0.15 s to arm. The control (`Intermediate/final-play/watch_control.py`): `x.exe`
written and deleted 1.35 ms later, and an `MZ` file under `y.dat`, both reported, `z.txt` not, the folder gone.
Where tests write: of the 20 files whose test code writes, every one takes its folder from `tempfile` or
`env::temp_dir()` (now the watched folder); the crates' `tests/` write nothing; `CARGO_MANIFEST_DIR` is read only.
After the run `git status` was unchanged and nothing in the repository was newer but the other engineer's files.

**3. A real level in the game.** `test_play_game.py --scene` (default `mp_test`; `SCENES` says what Play needs of
each): `l03u_agr_underground` - its 301 spawns go, an actor and a graph point stand at its actor graph point, its own
AI map, the level path `qa_l03u` (a level of the game's name keeps a one-level graph), Custom light at 4 px/m and
one jitter sample, `-noise`. One run (the first two stopped at the suite's own jitter label), on the debug build
made at 21:10 from the tree above: PASS 33 / 0, 143.9 s. The compile 32.1 s on 16 threads; the level in 50 s after
Play: `New game is successfully created!`, `# New player created.`, `connection accepted`, the object space and the
portals cache built for `qa_l03u`, `Loading HOM`, `level_lods composed`, `geometry level.geom` and `.geomx`, `[ground]
collision mesh: 332241 triangles, 189300 vertices`, `End of synchronization`, `[ALife]: objects[110]`; no assert, no
dump, still running when stopped, nothing of the installed game changed (107,624 entries). The log:
`Intermediate/final-play/l03u/engine.log`.

**Proof (21:13 - 21:25).**
- `bash tools/rust/check.sh`: FAILED on the other engineer's files only - clippy `manual_is_multiple_of`
  (`gz-fmt-level/src/build_prj.rs:468`), fmt (`gz-fmt-level/src/bvh.rs`, `gz-lc/src/{details,lib,tests}.rs`,
  `gz-lc-light/src/ray.rs`); unsafe only in `gz-win`, the registry against the catalogue, encoding, test files, the
  icon: ok; 36 s.
- `bash tools/rust/test.sh` (the changed binaries, under the net): 15 run, 63 cached, 1 failed - `gz-lc-light`
  (`tests.rs:517`, the other engineer's test of their change) -, 1138 tests green; 26.5 s; the net found nothing.
- `python tools/qa/run_all.py test_scene_tools test_play_game`: 2 / 2, wall 102.6 s (`test_scene_tools` 80 / 80 in
  15.3 s, `test_play_game` mp_test 36 / 0 in 87.3 s: 57 programs, 246.1 MB, the level in 31 s after Play).
- At the end (21:25:39) `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 21:08:40'`: nothing.
  No Grizzly or xrEngine process; no `gzpr_`, `gzpg_`, `gztest_`, `gzcr_` folder in the temp folder (an empty
  `grizzly_home_*` of the aborted l03u start removed); no `GrizzlyPlay` in the local application data.

Open: `check.sh` and `gz-lc-light` are red on the other engineer's work in progress; `test.sh --all` under the net is
the lead's run; `docs/RUST_MIGRATION.md` D1 does not name the marker or the default root's place (the lead's
document). (closed: both green and `test.sh --all` run under the net in "The run on the final tree"; D1 names
`play_root.txt` and both default roots.)

## The final review, the compiler

The brief `docs/rust/briefs/final-compiler.md`, 12 Nov, 20:53 - 22:1x. Files: `crates/gz-fmt-level` (`bvh.rs`,
`build_prj.rs`), `crates/gz-lc-light` (`ray.rs`, `lightmap.rs`, `tests.rs`), `crates/gz-lc` (`lib.rs`, `details.rs`,
`load.rs`, `tests.rs`), `tools/qa/diff_compile.py`, `tools/qa/compare_compile.py`. Before the first run (20:53):
`find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt` ten minutes earlier - nothing.

### 1. The same bytes on every run and on any number of cores

**What made runs differ.** `Probe::enter` (the BVH's slab test) gave an axis the ray does not move along a slope of
`1/1e-30`; with the origin exactly on the box's far face that is `0 * 1e30 = 0`, so the box ended at distance 0 and
was lost. The cached-triangle test of `RayModel::trace` tests no box, so it found the hit the walk lost, and the
light of a texel depended on which rays its worker had traced before - on how rayon split the work. The vertical
rays of the hemisphere (Adaptive HT's precalc, every hemisphere sample straight up) through a ceiling triangle's
corner or edge are exactly that case.

**Now.** An axis with `|d| < 1e-30` is *still*: the ray is inside its slab for every distance when
`min <= origin <= max` (faces included) and never otherwise, no reciprocal taken; the entry and exit distances are
widened by 4e-7 each way (the rounding of the difference, the reciprocal and the product). The test is monotonic in
the box, and `ray_triangle` (the cache's test) now tests the triangle's own box first: a cached hit is a hit of
every walk over a tree that holds the triangle (each box on its way holds the triangle's box), so the cache only
saves walks. The C++ (`xrCDB_ray.cpp` `isect_sse`) takes `1/0 = inf` and filters the NaN of `0 * inf` with
`minps`/`maxps` against infinities: on a still axis it answers inside iff `min <= origin <= max` too - the port now
answers as it does. Where they still differ the C++ is the less right one: its boxes are `center +- extents` rounded
(a border hit can be lost) and unwidened, its triangle test takes `0 < t <= range` against the port's
`0 <= t <= range`, and its cached triangle (`R_Light::tri`) is shared by all threads (below: its two runs differ on
shrunk deflectors).

**The proof.** Unit tests: `gz-fmt-level` `a_still_axis_ray_on_a_box_face_is_inside_the_slab` (the review's
`bvhedge`: the triangle test 5.0, the walk now 5.0 too; variants with `-0.0` and both directions; one ulp outside the
slab: no hit) and `axis_rays_through_corners_and_edges_agree_with_brute_force` (400 triangles on a half-metre grid,
the six axis directions through every corner and edge midpoint: `ray_nearest` / `ray_any` equal to the triangles
alone, every `ray_triangle` hit reported by `ray_all`); `gz-lc-light` `a_warm_cache_answers_as_a_cold_one_in_any_order`
(300 ceilings and walls, some transparent, axis and oblique rays through corners, skipped faces: a cold cache per
ray, one warm cache forward, one in reverse - the same bits). All three fail on the code before the fix (put back for
the check, then restored) and pass now. `diff_compile.py` compiles a scene of `TWICE` (mp_test, mp_close_combat,
l03u; `--all` every scene) a second time and wants every file byte for byte, and compiles mp_test once more with
`-threads=1` (a new option of `gz_lc::Options`: the compile on a pool of its own) against all 16 threads; a
difference names the first file and byte. Five runs of l03u on 16 threads with the final binary
(`Intermediate/final-compiler/repeat.py`): `aea8b2b8b9f3026f` five times, 39.5 - 45.1 s each (before the
placement fix below, at 21:03, also one hash in five, `716db78f85997ef2`).

**The BVH's other callers** (`rg` over `crates`); every one takes the same `Probe`, so the fault reached each that
casts an axis-parallel ray from a coordinate a box face can share:
- `gz-lc-light` `RayModel::trace` - every light ray of both compilers: reached (the review's runs).
- `gz-scene` `pick::ray_library` through `ray_scene_objects`: `probe_vertical` (+-Y), `place_way`'s snap to ground
  (-Y), `snap.rs`'s drop to the floor (-Y), `scan_walls` (horizontal rays, the first exactly +Z), the viewport's
  `lighting.rs` height probes (-Y), the particles' and `draw_extra`'s rays: reached for an object whose rotation keeps
  the axes (its object-space ray stays axis-parallel) when the probe's x or z equals a box face of its tree - a
  grid-snapped probe over a grid-snapped object could miss it.
- `gz-scene` `wallmark::ray_triangle` (wallmarks, and `ai_ground`: the AI tool's node height from the mouse ray,
  exactly vertical in a top orthographic view), `objtools::geom::pick_mesh` and `mesh_ray_hits` (the portals'
  sector rays): reachable the same way.
- `query_box` / `query_sphere` / `query_frustum` (`detail.rs`, `pick.rs`, `wallmark.rs`): no ray - not reached.
- The AI map's own node height cast (`gz-scene` `bulk/aimap.rs`) uses `collide.rs`'s grid and `xr::ray_tri`, not the
  BVH: not reached.

`a_detail_slot_is_lit_at_its_points_and_its_corners_decide_its_objects` (gz-lc-light) changed its numbers: the two
slots beside the test's roof have a row of 15 sample points on the roof's edge, which the old test lost; 210 of 225
points are lit there now (6 of 15, was 7).

### 2. The comparison

- **Flat triangles are sampled** (`compare_compile.flat_samples`): a lightmapped triangle whose coordinates collapse
  to a line or a point gives the texels along its longest side (four points a texel, each texel once), or the one
  under it, through each side's own coordinates, judged with the rest and reported on their own. Samples from flat
  triangles: mp_test 2009, l12u 43,964, l03u 269,504, mp_close_combat_implicit 42,321.
- **mp_test's texel (C++ 33, Rust 0): the Rust side is right.** It is `lmap#1` (309, 0), page 2's colour, which is
  the sun again (`Pack_hemi`: r = g = b = sun). The C++'s own page 1 alpha there - the same sun, in DXT5's eight-level
  alpha block, endpoints 126 and 0 - is 0, so the C++ wrote 0 before its encoder; page 2's colour block holds
  123/93/63/33 (four colours a block) and could not keep the 0 beside the block's 126, 90, 73, 55, 38. The rule
  (`sun_copy_rule`): on 2.r/g/b each side's own disagreement between its two copies of the sun is the encoder's and
  is taken off, the rest is judged; the raw numbers are reported (`lightmaps_raw`). mp_test's 2.r goes from 2.0 to
  0.016 (the flat-128 texpresso bias was the same thing), its flat samples' 2.r from 5.1 / 33 to 0.
- **The two new scenes.** `mp_close_combat_implicit` (the level of mp_close_combat, `light_jitter_samples` 1 at its
  own 10 px/m, no `-noise`) holds the implicit light and two lightmap pages. XrLC at one sample: 175.5 / 174.6 s wall,
  622.0 / 610.5 s CPU; xrDO_Light 34.0 / 33.6 s, 45.9 / 46.4 s CPU; the Rust 35.0 s and 4.7 s. Its MU models could not
  stay: XrLC of the tag crashes on them (exception 0xC0000005, twice at the same log line in "Converting MU-models to
  OGFs", after the tangent basis of a model part of 7706 vertices; 282.6 / 298.3 s wall, 1302.5 / 1334.1 s CPU), so
  `diff_compile.EMPTIED` empties build.prj's chunks 13-15 (LODs, models, references) in the input both compilers
  read. What the crashed run wrote first was compared by hand with the Rust compile of the input with the models
  (`Intermediate/final-compiler/partial.py`): build.lights byte for byte, build.cform under its rule (the MU
  references' ray triangles inside), the implicit textures mean 0.046 / 0.039, p99 1, max 20. A denser `mp_test` was
  tried for the pages and stays one page at 20 and at 40 px/m: every large deflector is clamped to 508 texels
  (`xrDeflector.cpp`).
- **What the two pages found.** First 282 visuals against 291. The deflectors' sizes read back from both outputs
  (`Intermediate/final-compiler/charts.py`): most equal; on every scene some shrunk on one side only (mostly the C++
  shrinks more: l12u's `crete_walls12` 20 x 16 against 41 x 33, a halving the Rust light did not pass), which moves
  what comes after them in the page; with one page only coordinates move, with two a deflector changes page, and a
  material on two pages is two visuals. One part was the port: `Place_Perpixel` tests an upright layer eight texels
  at a time while more than eight are left, and there a texel marked exactly `alpha_ref` counts as free; the port
  took it as taken everywhere (a stated difference). It now tests as the C++ does (`lightmap.rs` `fits`, test
  `a_texel_marked_alpha_ref_is_free_only_in_the_wide_columns`): 288 visuals. The rest follows the light, and its rule
  is `paged_compare` (compare_compile.py's header): the static triangles as one multiset, matched by key for their
  light. mp_close_combat_implicit: 132,544 triangles equal, 637 of them on the other page.
- **The C++'s floor is not zero** once the flat triangles count: its two runs differ on shrunk deflectors - l12u up
  to 8 on 1.r (mean 0.0001), l03u 12 on the hemisphere (0.0014), mp_close_combat_implicit 127 on the sun (0.0016) -
  most likely its shared cached triangle.
- **The bounds** (compare_compile.py, about twice the largest seen, flat triangles included): lightmaps and implicit
  textures mean 1.5 / p99 16 (seen 0.68 / 8: l12u, 1.r); vertex and MU light 0.2 / 2 (seen 0.085:
  mp_close_combat_implicit level.geom; p99 1: l03u's LODs lit alone); level.details byte for byte (the 0.5 / 2 bound
  is gone: seen 0 on both detail scenes). `VERSION` 7.

### 3. The smaller things

- `--refresh-oracle` over a record: the record is made whole in `.part`; the old one is moved to `.old`, the new one
  takes its place, the old one is removed; a recording that fails leaves the old record and no `.part`
  (`Intermediate/final-compiler/refresh_test.py`, `record_into` faked: replaced, then a failing one - the old one
  stays). A C++ run that fails keeps what it wrote and its log under `Intermediate/compile/<scene>/cpp_failed` and
  says how it ended ("crashed (exception 0x...)", "did not finish within N s", "exited with code N"); it said "did not
  finish within 1200 s" for a crash at 283 s.
- `build.prj` is checked whole before it is read (`check_whole`): its chunks end to end, each a chunk of a build
  project, each array a whole number of records, the seven chunks the C++ opens unchecked present. build.cform,
  build.lights and build.details are checked the same way (`whole_chunks`), their parse errors name the file, a
  missing file "is not there". The review's 23 inputs (`Intermediate/final-compiler/bad_inputs.py` on the fast
  build; `bad_before.out` / `bad_after.out`): none panics; cut in half "build.prj is cut short: its static lights
  chunk holds 26568 bytes and the file ends 12074 bytes into it.", the last byte missing "...it ends 7 bytes into the
  header of a chunk.", empty, garbage ("not a build project"), a chunk past the end, a record cut in two ("not a whole
  number of 80-byte records"), the detail inputs cut short - each says so. Two answers stay: one of the six
  random-byte files compiles (its 40 bytes fell into light values, vertex and texture coordinates and the slack after
  texture names - a valid file), and `-bogus` is named and ignored as XrLC ignores it.
- The level's files appear together: written into `<level>\.lc_new`, the files they replace moved to `.lc_old`, the
  new ones moved in, the old ones removed; a failing move puts every old file back; the lightmap pages of an earlier
  level compile that this one does not write go too (the detail compiler's one file leaves them). Test
  `the_level_files_appear_together_or_not_at_all` (a folder where a file must go makes the move fail half way: the
  folder is as before). A dot name: tooling to every walk of the editor.

### 4. Proof

Once, at the end, on the fast build of the final tree (`Intermediate\cargoast\Grizzly.exe`):
- `cg.sh clippy -p gz-fmt-level -p gz-lc-light -p gz-lc --all-targets`: clean; `fmt -- --check`: clean.
- `cg.sh test -p gz-fmt-level -p gz-lc-light -p gz-lc`: 54 + 1 + 41 + 18 tests green, 16 s with the build.
- `python tools/qa/diff_compile.py --all`, 21:51 - 22:01, 552 s: mp_test, l12u_control_monolith, mp_close_combat,
  mp_close_combat_implicit, l03u_agr_underground PASS, every second compile byte for byte, mp_test on one thread
  byte for byte (18.1 s against 2.3 s). The Rust 2.3 / 52.4 / 11.8 / 39.2 + 5.0 / 31.6 s wall (26 / 629 / 161 / 574 /
  349 s CPU) against the C++'s 20.1 / 504.8 / 78.4 / 175.5 + 34.0 / 186.8 s wall (11.6 / 1390.7 / 115.4 / 667.9 /
  688.6 s CPU).
- Five l03u runs: one hash (above).

### For the plan (closed: folded into the plan - section 9's light, "Compared this way", "Not proven"; phase 8; D27)

- Section 9, the light: "lightmaps within 1.5 / 16, vertex light within 0.2 / 2, the detail objects' light byte for
  byte. Seen: lightmaps mean 0.68 and p99 8 at most (l12u), flat triangles included; vertex light 0.085 / 1." The
  second picture's colour is judged less each side's own encoder disagreement with its first picture's alpha.
- Section 9, "compared this way": add `mp_close_combat_implicit` (the implicit light, two pages, its MU models
  emptied because XrLC crashes on them) and the rule for deflectors on other pages.
- Phase 8, "the same result on any number of them": proven by `diff_compile.py` (twice, and one thread against all).
- A deliberate difference: the BVH answers a still axis as the C++'s slab test does; the C++'s rounded boxes and its
  shared cached triangle are not ported. `Place_Perpixel`'s eight-at-a-time quirk is ported (no longer a difference).
- Unproven (compare_compile.py's header, phase by phase): the Draft quality, `-nosun`, `-nosmg`, `-no_optimize`,
  `-att`, `-no_invalidefaces` and the invalid-face abort, more than two pages, MU models other than l03u's (at 4 px/m,
  one sample, `-noise`), the implicit light at more than one sample, the nine-sample jitter only through the
  lightmaps of mp_test and l12u.

## The final review: the lead's part and the run on the final tree

The final review (Fable 5.2, 12 Nov; its report is in the conversation's record, its scripts and outputs under
`Intermediate/critic-final/`) said the move was not done: Play's `root=` could wipe a folder it did not make
(blocking), the compile differed from run to run, the comparison covered less than section 9 said, and the notices
named three pieces of 284. Two engineers closed the first three ("The final review, Play and the tests", "The final
review, the compiler", above); the lead's part:

- **The notices are made from what the programs link** (`tools/rust/notices.py`, `licenses/NOTICES_HEAD.txt` +
  `licenses/THIRD_PARTY_NOTICES.txt`, 684 KB): the 284 third-party crates of the two programs' dependency closure
  for Windows (normal dependencies, never through a proc macro), each with its license and repository, then the
  206 distinct license texts the crates ship, the fonts egui embeds included. `check.sh` fails a file the lock no
  longer gives.
- **No GPL code is linked.** `gz-chunk` carried a port of LZO 2.10 (GPL v2+), compressor and decompressor. The
  editor only READS LZO blocks (the files of the game's archives) and writes none - the compressor was called by
  one test. `gz_chunk::lzo::decompress` is now the `lzo` crate (Apache-2.0, `#![forbid(unsafe_code)]`, strict about
  the stream's end) behind the same contract: exactly the stated size; a size the stream cannot reach (more than
  256 times the block) refused before anything is reserved; a size the machine cannot hold an error
  (`try_reserve_exact`). The tests pack their fixtures with `lzokay-native` (MIT, a dev-dependency) and keep four
  blocks as the engine's compressor writes them. `licenses/LZO_LICENSE.txt` is gone. The whole corpus (every file
  of the game's archives) reads through it in `test.sh --all`.
- **Not decided here, the author's to decide before a release**: the editor's own license - the workspace says
  `LicenseRef-Grizzly` and no text of it exists in the repository. (closed: MIT, commit `77575f32`.)
- **Not done, a change of the GAME**: Play still copies the engine and its libraries into its root, because the
  game's content service works in the folder of its engine (D1). (closed: "Closing: Play in place".)

**The run on the final tree** - 12 Nov, 22:10 - 22:34, commit `7866dfd9`, the release build, one step after
another (`Intermediate/final2/`, `summary.txt` and a log per step):

| Step | Result | Time |
|---|---|---|
| `Build.ps1` | the two programs, their PDBs, the notices | 245 s |
| `check.sh` | ok: clippy, fmt, `unsafe` only in `gz-win` (29), the registry, encodings, test files, the icon, the notices | 50 s |
| `test.sh --all` | 78 binaries, 1178 tests green; the net over the tests' temp folder: 3287 events, no program file, 6.4 s of CPU | 322 s (228 s of it the build of the test binaries) |
| `diff_editors.py --all` | 10 scenarios, 755 steps, 746 written files, 0 differ | 53 s |
| `diff_compile.py --all` | mp_test, l12u_control_monolith, mp_close_combat, mp_close_combat_implicit, l03u_agr_underground PASS; every scene's second compile byte for byte | 502 s |
| `run_all.py --with-game` | 23 suites, 23 passed | 159 s |
| `test_play_game.py --scene l03u_agr_underground` | 33 checks pass: the level in the game 25 s after Play, no dump, the installed game's 107,624 entries unchanged | 83 s |

Nothing changed under `D:\XFinedProjects` or the game's `modules`; no process and no Play root left.

### The reviewer's three holes, closed

The reviewer's `play_root_verify.py` (Intermediate/critic-final) found three; the lead's brief of 12 Nov evening.
- **A root being made loses nothing.** `prepare` is now `check_root` + `prepare_checked(state)`: every entry of the
  root's top is claimed before it is written - its `program = ` / `entry = ` line appended to the marker - and a
  name that is there although the marker read at the check did not list it and this run has not written it is a
  refusal ("<name> is in the game root ... and Play did not write it ... Play leaves it as it is"). A missing or
  empty root's marker is made with `create_new`; the old `gamedata` is removed only when the marker listed it; the
  programs, `fsgame.ltx`, the junction, `appdata`, `modules` and the content folder go through the same claim;
  `stage_level` writes only into a `gamedata` the marker lists. At the end the marker is rewritten as the root is.
- **A project inside what Play clears refuses the root.** `gz-project`'s `delete.rs` gained `projects_inside` -
  `check_delete`'s "holds another project" question asked of any folder (the folder itself too, links counted,
  never entered, an unreadable folder an error) -; `check_root` asks it of a marked root's `gamedata`, and
  `prepare_checked` again right before the clear. On a real root (the copied loose gamedata): 681 entries, 3 ms,
  on the IO pool (`play_root` answers `walked`, `walk_ms`).
- **The default root follows the home.** An editor with a home of its own (`-home` / `GRIZZLY_HOME`: home not the
  data root) plays in `<home>\play\<id>`; the author's own in `%LOCALAPPDATA%\GrizzlyPlay\<id>`. A home too deep is
  the path-length refusal naming the path. `tools.json` and `test_play_game`'s docstring say so.

Proof (22:45 - 23:01, debug build of this tree; nobody else in the Rust tree): unit tests - `play.rs`
`a_root_that_gained_entries_after_its_check_loses_none_of_them` (an empty root gains `gamedata\precious.txt`, a
missing one a library or `fsgame.ltx`, or a marker, between the check and the clear: refused, nothing changed),
`a_marked_root_loses_only_what_its_marker_lists_and_never_a_project`; `play_root.rs`
`a_marked_root_whose_gamedata_holds_a_project_is_refused_and_left_as_it_is`,
`an_editor_with_a_home_of_its_own_plays_under_it`; `delete.rs`
`the_projects_inside_a_folder_about_to_be_cleared_are_found`. The reviewer's script rerun on this build
(`Intermediate/final-play/play_root_verify.out`): all 15 cases refused, nothing lost, the race (a file 0.12 s into a
0.4 s prepare) refused with the file there. `test_scene_tools` Y: a root `play_root` made gets
`gamedata\mods\my_mod\mod.ltx`; `play_root`, `play_clean`, `play` refuse it naming the project, its 3,095 entries
listed equal.
- `bash tools/rust/check.sh`: ok (clippy, fmt, unsafe only in `gz-win`, registry, encoding, test files, icon); 37 s.
- `bash tools/rust/test.sh`: 33 run, 45 cached, 0 failed, 1183 tests green; 47.3 s; the net: 1537 events, 0
  overflows, 1.89 s of CPU, no finding.
- `python tools/qa/run_all.py test_scene_tools test_play_game`: 2 / 2, wall 84.5 s (86 / 86 in 22.9 s; 36 / 0 in
  61.5 s, the level in 18 s after Play).
- `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-12 21:08:40'` at 23:01:27: nothing; no
  `gzpr_` / `gztest_` / `GrizzlyPlay` left (an empty `grizzly_home_*` of the verify script's import removed); the one
  Grizzly running is the lead's compile on its own home.

## The reviewer's verification of the fixes, closed

The reviewer went over its findings again at `3e643cb5` (scripts and outputs under `Intermediate/critic-final/`):
Play's root closed for what was reported (fifteen ways round it refused, nothing changed), the compile the same on
every run (five 16-thread runs of l03u one hash each file; one thread byte for byte equal to sixteen), the licences
closed (no copyleft-only code linked, the new decoder keeps the old one's contract), the smaller things closed. Its
verdict: done for the editor and the compiler, with one rule of the proof to replace and three small holes. All four
are closed:

- **The rule that could not fail** (`compare_compile.py`, `sun_copy_rule`, added in the pass before): it took each
  side's own difference between its two copies of the sun off the second lightmap picture's colour; by the triangle
  inequality what was left could never exceed the first picture's alpha difference, so a black, a white or a noisy
  second picture passed - and that picture's green is the sun the game reads (`get_sun`: `lmh.g`). Gone: every
  channel is judged as it is. Seen, raw: the second picture's green mean 0.85 / p99 12 at most
  (mp_close_combat_implicit), under 1.5 / 16; its red and blue, DXT5's 5-bit channels where the two encoders round a
  flat value two steps apart, mean 2.0 on mp_test and 1.3 on the implicit scene, held to 3 / 16
  (`LIGHT_MEAN_5BIT`). `python tools/qa/diff_compile.py --all` on the release binary: five scenes PASS, 776 s.
- **One thread against all on the scene that showed the fault**: with `--all`, l03u_agr_underground is compiled on
  one thread too (mp_test, the only scene checked that way before, was stable before the fix as well): every file
  byte for byte, 130 s.
- **What the paged rule leaves out** (the sliding windows, which sector a visual belongs to) and three more lines
  of "not proven" (the visual tree of a level whose deflectors moved between pages; MU models and the implicit
  light in one level; a level with implicit light loaded by the game) are in the tool's header and in the plan's
  section 9.
- **Play's root, three holes** ("The final review, Play and the tests", its last part): nothing is removed from a
  root Play is making (each entry's name goes into the marker before the entry; what the marker did not list and
  this run did not write is refused untouched); a project anywhere inside what Play would clear refuses the root
  (`gz_project::delete::projects_inside`, at the check and again before the clear); an editor with a home of its
  own keeps its default root under that home.
- **The watcher's limits** are in its header: an `MZ` head under a harmless name that is gone before it can be
  read, and anything written outside the watched folder, are seen only by the source check.

**The last run** - 12 Nov, 23:03 - 23:20, the release build of `6306a076` (`Intermediate/final3/`): `Build.ps1`
241 s; `check.sh` ok, 18 s; `test.sh` 1183 tests green (the changed binaries had run under the watcher in the pass
before; nothing to rerun); `diff_editors.py --all` 755 steps, 746 files, 0 differ, 46 s; the suites the last fixes
touch - `test_scene_tools`, `test_play_game`, `test_project_browser`, `test_mcp_bridge`, `test_build_job`.
`test_build_job` failed once there on its frame check (one frame of 95 ms, bound 64) when it started right after
`test_play_game`; alone it passed three times of three. `run_all.py` now waits for a quiet machine before a suite
that times something (`settle`: under a tenth of the cores busy for half a second, twenty seconds at most): the
same five suites in the same order, twice: 5 of 5 both times (the first waited 18 s). Nothing changed under
`D:\XFinedProjects` or the game's `modules`; no process, no Play root and no temp folder left.

The whole run before it (`Intermediate/final2/`, `7866dfd9`) stands for everything these last commits did not
touch: `test.sh --all`, the eighteen other suites, the level in the game.

## Closing: Play in place

The brief `docs/rust/briefs/close-play.md`, 13 Nov, both repositories; nothing committed. Play no longer copies the
game: the game was taught to run on another fs config without writing into its own folder, and Play starts the
installed `xrEngine.exe` where it is.

**1. The game** (`D:\Games\Dead Air\DeadAir-x64`, branch `modules-describe-the-game-does`). What it wrote beside
its programs on another fs config, found in the code and then by a listing: the content service's state cache and
latch (`ContentState::SaveCache` / `WriteLatch` into `<app>\.dead-air-x64`), the update check's sweep of
`<app>\.dead-air-x64\update-cache` (and its downloads), the early crash reports (`<working folder>\session_reports`
before the filesystem is up) - and everything a plain relative root of the config names (`$app_data_root$ =
appdata\` is the WORKING folder's, so a tool that starts the engine in its own folder got the logs and caches
there). Now, for a start with `-fsltx` naming a file outside the application folder
(`CLocatorAPI::fs_config_elsewhere`; without `-fsltx`, or with one in the application folder, nothing changes):
the content records go to `$app_data_root$\content\` (the first start copies the installation's state, so it
hashes nothing), the bundles verified are those of the config's `$arch_dir$`, the installation's latch is only
read, repair is refused, the update check is skipped, a relative root is the config's folder, early reports go
beside the config. `-fsltx` takes a quoted path. The path limit: `CLocatorAPI::Register`'s `string256` is a
`string_path` and the executable is `longPathAware`: a full path of 519 characters where Windows long paths are on,
259 where not (before: 255, a root of about 66). The engine says so in its version resource, `EngineAbilities =
fsltx-in-place long-paths` (`ProductVersion.h`). Documents: `CONTENT_BUNDLES.md` ("A game on another fs config"),
`AUTO_UPDATE.md`, `DIAGNOSTIC_REPORTS.md`, a `TEST_MATRIX.md` row.
- Build: `build_x64.ps1 -Configuration Release`, 2417 steps (the locator's header is in every precompiled
  header), then 20, then `no work to do`.
- Proof before the deploy (`Intermediate/close-play/prove_game.py`): the new runtime in a temp folder beside a
  junction to the installed `database`, started there with `-fsltx` naming a root 179 characters deep - a new
  game's level in 22 s, paths of 368 characters in the root, `verified 26 of 26 bundle(s), 0 hashed`, the
  application folder (746 entries) listed equal before and after; the same folder without `-fsltx` rewrote its own
  `.dead-air-x64\content-state.txt` (14 s). 8 / 8.
- Deploy: the game closed, 75 files backed up into `D:\Games\Dead Air\_predeploy_fsltx_20261113`, the whole
  `runtime-files.txt` list by `tools\package\deploy_live.py` (58 files, the PDBs, the overlay unchanged), SHA-256 of
  74 files against `bin\x64\Release`: 0 differ.
- The game's own check on its QA root after the deploy: `Run-NqQa.ps1 -Scenario Scout` (an existing save, a
  module, the default fs config) PASS 6 / 0.

**2. The editor.** `gz-game`'s `version::engine_abilities` reads the string; `gz-levelbuild`'s `play.rs` decides
by it before anything is written (`PlayMode::InPlace` / `Copied`, `ability_of`, never by trying). In place the root
holds the marker, `fsgame.ltx` (the game's, `$arch_dir$` rooted at the game's folder instead of `$fs_root$`),
`gamedata` with the level, `appdata`, an empty `modules` - no program, no link; a root an older engine used loses
its programs, its junction (as a link) and its `.dead-air-x64`, all listed by its marker. The engine is the game's
own, started with `-fsltx <root>\fsgame.ltx` in the root; the answer (`play`, `play_root`) and the log line say
`in_place` or `copied`. The copy path stays for an engine that says nothing (superseded: "Play in place only" -
the copy path left the editor, and the value is read in the process). The depth check is the engine's
(`path_limit`: 519 for `long-paths` with `LongPathsEnabled`, read through `reg.exe`; 259 without; 255 for an older
engine). `test_play_game.py`: the root's contents, no program and no link before and after, the content records
in the root's `appdata\content`, the game's own line; the game it started is told from the author's by its command
line, and it does not start while the installed engine runs for somebody else; the listing leaves out `.git`
(an outside `git status` in the game repository rewrote its submodules' folders during one run);
`mp_close_combat` added to `SCENES`. `test_scene_tools` Y: the root is for the engine in place.

**Proof** (the debug build). Unit tests: `play.rs` 3 new (a copied root turned in place, the path limits, the fs
config), `version.rs` 1; the copy path's tests stay, no program file written. The levels in the game, once each:
`l03u_agr_underground` PASS 36 / 0, 129.7 s (compile 38.3 s, the level in at 29 s, 332,241 collision triangles);
`mp_close_combat` (implicit light and MU models, Custom quality at 10 px/m, one jitter sample) PASS 39 / 0, 376.8 s
(compile 132.7 s; at nine samples it passed the three minutes an MCP command waits), the level in at 50 s, 74,027
triangles, `objects[113]`. Logs: `Intermediate/close-play/l03u`, `mpcc`.

**For the plan, D1 as it should read:** "Play in Editor starts the game. Play bakes the scene, builds the
collision, the AI map, the game graph and `editor.spawn`, compiles the level in the editor's process (`gz-lc`) and
runs the LINKED game's engine on it as a job, on a game root of its own outside the project: the root's
`fsgame.ltx` (the game's, with `$arch_dir$` naming the game's archives), a copy of the loose gamedata with the level
added, an appdata of its own, no module. An engine that says `fsltx-in-place` in its version resource
(`EngineAbilities`) is started where it is installed and writes nothing there; an older one runs from copies of
itself and its libraries in the root beside a junction to the archives. The root is `%LOCALAPPDATA%\GrizzlyPlay\<id>`
(`<home>\play\<id>` for an editor with a home of its own) or `root=`; only a missing or empty folder becomes one,
the marker `play_root.txt` first, and Play removes from a root only what its marker listed or this run wrote, behind
the checks of a project's deletion. A root deeper than the engine takes is refused naming the path." Why: "The
real game shows what a player gets, and its crash no longer takes the editor along; the game knows another fs
config, so nothing of the installation is copied or written."

## Closing: the old notes

The brief `docs/rust/briefs/close-sweep.md`, 13 Nov; nothing committed. Every statement found is a row of "Open:
nothing" at the top of this record and is marked "(closed: ...)" where it stands.

**1. One line model of LTX text.** `gz_ltx::LtxDoc` reads a line by a `Syntax`: `Ini` (`CInifile`: `;` and `//`,
`#` directives, `[s] : parents`, quoted values over lines, a section read at its first header) or `Manifest` (the
XMS reader and the C++ `EditorMod::Load`: `;` outside quotes with `\"` escapes, headers without inheritance, every
header of a name a body of it, entries without `=`); the raw-line edits (`items`, `section_lines`, `set_line_ci`,
`push_line`, `set_lines`, `remove_where`, `prune`) are its own, and a `Placement` (section rank, key rank, key
width) says where new lines go. `gz_mod::ManifestDoc` is `LtxDoc` read by `Syntax::Manifest` with the C++ `Save`
order and `AppendKey`'s widths - the manifest's meaning, nothing of the lines. Every earlier test of both kept;
`the_two_syntaxes_read_one_text_their_own_way`, `placement_orders_what_is_new` new; the corpus round trips of every
`.ltx` and `mod.ltx` byte for byte; `s2_manifest` of the differential equal.

**2. The marquee's depth.** Tools > Box Selection > Limited Depth and Depth Tolerance were stored and never read:
the marquee now ends at the plane through its corners' rays as far out as the farthest scene object under them, plus
the tolerance (`CLevelMain::SelectionFrustum`; `gz-ui-viewport` `draw.rs` `box_depth`, `frustum_select`). Back Face
Culling: only `CEditableMesh::FrustumPickFaces` reads it in the C++, which nothing of the level editor calls; its
help says so.

**3. Commands no test named.** `Grizzly.exe -dump_commands` (204) against the suites, the differential's scenarios
and the crates' tests (`Intermediate/close_sweep/untested.py`): 10 named by none, 0 now. `ui_toast` answers the
notes up and `ui_list` names them (`toasts`) and the status bar's centre (`status_centre`); `test_ui_shell` E holds
`ui_split` and `ui_toast`, A / F the centre (the Level tab's status, none on the Start page); `test_properties` C
`spawn_animation`; the new `test_level_commands` (in `run_all.py`, on mp_test) `level_rebuild_status`,
`reload_configs`, `build_cform`, `build_aimap` (the C++ "AIMap is empty!" on a scene without nodes), `build_all`,
`compile_details`, `play_game` (refused for a folder that holds a file, no game started).

**4. A request past its timeout** (the lead's): `gz-mcp` `run_request` runs the command in a `gz_task::JobScope` -
a tokio task-local that is a value only inside the request's future; `Jobs::start` records into it, and
`Executors::spawn` carries it into the tasks the command spawns. Past `GRIZZLY_MCP_REQUEST_TIMEOUT_MS` a command
whose job runs answers `{"ok": false, "running": true, "command", "error": "<cmd> is still running: its job <kind>
#<id> (...) goes on past this request's N s timeout, and this answer is not its result. Follow it with ...", "jobs":
[{id, kind, title, stage, group, started_ms}], "follow": ["jobs", "level_build action=status" | "mod_publish
action=status" | "mod_catalog action=status" | "mod_export answer_only=true ..." | ...]}`; only a command with no job
behind it says "editor busy (timeout)". Not seen: a job a command starts from the UI thread (`on_ui`). Tests:
`a_command_whose_job_outlasts_the_timeout_names_the_job` beside `a_command_runs_to_its_end_past_the_timeout`,
`a_request_scope_learns_the_jobs_started_for_it` (`gz-task`), `test_build_job` T (an editor with a 0.3 s timeout: a
build answered as running in 0.31 s, the job then done in `jobs`). AI_SETUP.md, the bridge's instructions, plan 5.7.

**5. The rest.** `NormalizeKeys` and the sectors' forget have tests; four widgets two surfaces carried copies of are
`gz-ui`'s `form` (`section`, `raised_section`, `check`, `hint_field`; the surfaces' `local_*` only name their ids);
Preferences' note of rows "not kept by this build yet" is gone (every row live, held by a test); `gpu.rs`'s
`mute_held`; PARITY.md's rows and "Unjudged" judged; plan section 4 names what only `ActorEditor` reached; the layer
table is a section of its own.

**6. The tests' net sees the repository.** `test.sh` runs `watch_programs.py` a second time, over the repository's
tree, while the binaries run: `--exclude Intermediate/cargo` (cargo's programs; it does not build then),
`--since-start` (only a file made after the watch began: the tree holds `Build.ps1` and release builds of its own),
`--marks Intermediate/test-logs` (its armed and stop marks, removed at the end). A finding fails the run and blames
the binaries running at its time, as in the temp folder. It cannot see what the header says: a harmless name gone
before its head is read, a file older than the watch rewritten in place or renamed, an excluded path. The control
(`Intermediate/close_sweep/watch_repo_control.py`): `x.exe` deleted 1 ms after it was written and an `MZ` head
under `y.dat` reported; `z.txt`, `Intermediate/cargo/negctl_q.exe` and an older `.ps1` rewritten not; armed in
0.53 s, 22 events, 0.25 s of CPU. Cost in the run below: 515 events, 0 overflows, 1.00 s of the watcher's CPU and
about half a second to arm, beside the temp folder's 3710 events and 4.62 s.

**Proof** (13 Nov, the release build of this tree, `Intermediate/cargo/release` - the debug and fast binaries were
another engineer's running editors, so `GRIZZLY_QA_EXE` named the release one):
- `bash tools/rust/check.sh`: ok, 158.6 s.
- `bash tools/rust/test.sh`: 46 binaries run, 32 cached, 0 failed, 0 skipped, 1193 tests green; 350.4 s (296.5 s
  the build); earlier `--all -p` of the crates of 1-2: 10 binaries, 49.3 s.
- `python tools/qa/diff_editors.py`: 10 scenarios, 755 steps, 0 differ, 746 written files, 0 differ; 231.8 s (every
  oracle recorded again on its own port).
- `python tools/qa/run_all.py`: `test_ui_shell`, `test_properties`, `test_level_commands`, `test_world`,
  `test_screens`, `test_previews`, `test_build_contents`, `test_ui_tour`, `test_characters` 9 / 9 (after three new
  checks were set right: a store key of the test's own for `ui_split`, a visual that resolves for
  `spawn_animation`, the empty AI map's refusal), then `test_build_job`, `test_mcp_bridge` 2 / 2 (48.4 s), and
  `test_scene_tools` 1 / 1 (19.9 s: M, the marquee, with Limited Depth on as the C++ default).
- `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-13 00:10'`: nothing; no Grizzly,
  LevelEditor or xrEngine process left.

Not run: `diff_compile.py` (nothing of the compiler changed), `test.sh --all` and the whole `run_all.py` (the
suites whose code did not change stand on their last runs).

## Closing: the compiler's proof

The brief `docs/rust/briefs/close-compiler.md`, 13 Nov, 00:14 - 04:40. Files: `tools/qa/diff_compile.py`,
`tools/qa/compare_compile.py`, the C++ working copy `D:\Games\XFined-Editor-cpp` (`cmake/CompilerTargets.cmake`); no
Rust file changed - nothing the new records found was the port's. Before the first run (00:14:51): `find
D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt` ten minutes earlier - nothing.

### The crash of mp_close_combat's MU models: the C++'s, fixed in the oracle

`diff_compile.py --diagnose <scene> --cpp-bin <build> [--set k=v] [--flag=-x]` (new) runs XrLC of a diagnostic build
(the working copy built with `/Zi /DEBUG` and nothing else, `Intermediate/close-compiler/build_cpp.ps1 -Diag`, into
`D:\Games\XFined-Editor-cpp\Bin\x64\Diag`) under a debugger written in the tool (kernel32 + dbghelp: first- and
second-chance faults, the faulting thread's stack walked and scanned, `_NO_DEBUG_HEAP`), on the scene at 1 px/m,
one sample, `-noise`, its models kept. Under the debugger the fault is **0xC00000FD, a stack overflow** (outside one
it surfaces as 0xC0000005 once the guard page is gone), on the main thread, `FindBestScore` on every frame of it (1,817 in the first 512 KB scanned)
(`XrETools/optimize_vertex_order.cpp:1456`): Forsyth's lookahead vertex-cache optimiser, which this tree uses in place
of D3DX since `5a3f5886` ("Cut the editor loose from d3dx9") and which VIPM's `CalculateSW` calls for every sliding
window, recurses once per face of the list; at ~300 bytes a frame the 1 MB default stack holds some 3,500 faces, and
a model part of 7,706 vertices has more. A fault of the C++ on a valid input. The port runs that recursion as a loop
(`gz-fmt-ogf`'s `optimize.rs`, said in its header).

**The fix** (`D:\Games\XFined-Editor-cpp\cmake\CompilerTargets.cmake`, uncommitted): `target_link_options(XrLC PRIVATE
/STACK:134217728)` - 128 MB reserve, committed only as deep as a run goes; VIPM takes up to 65,535 faces. No code
changes; the header of XrLC.exe says 134217728. (A conversion of `BuildGroups` in `NVMeshMenderTwo.cpp` to a loop was
tried first, before the debugger named the function, and reverted: the file is as at the tag.) The oracle
`Bin\x64\CppCompilers` is a build of the working copy (`Build.ps1 -Only compilers -Out ...`, 95 s; the old binaries
are kept in `Intermediate/close-compiler/oracle_before`). **Proven to change no result on mp_test**:
`diff_compile.py --rerecord mp_test` (new: the oracle again on the record's own input, the record untouched): XrLC
15.2 s wall, 10.8 s CPU; 4 files byte for byte equal to a run of the record, the other four (`build.cform`,
`level.cform`, both pages) the ones the record's own two runs differ in; under the rules light 0 on every channel,
vertex light 0, level.cform "the same surface triangulated another way". `mp_close_combat_whole` then ran through
"Converting MU-models to OGFs" twice.

### The list, each line

| | scene (options) | C++ XrLC wall / CPU s, two runs | result |
|---|---|---|---|
| Draft | `mp_close_combat_draft` (the editor's Draft: quality 0, 0.1 px/m, 1 sample) | 41.1 / 101.8, 36.2 / 106.6 (+ xrDO_Light 339 / 152, 213 / 170) | PASS after two rules (below) |
| -nosun | `mp_test_nosun` | 20.4 / 11.6, 101.5 / 15.5 | PASS; changes the sun: 1.a mean 125, 2.rgb 124 against mp_test |
| -nosmg | `mp_test_nosmg`; `mp_close_combat_draft_nosmg` (Draft, no detail objects) | 66.7 / 17.9, 143.9 / 19.1; 22.3 / 76.9, 356.5 / 104.8 | PASS; on mp_test it changes only level.cform (the normals are the same either way), on the Draft scene the normals: the visuals, level.geom, build.cform, build.err |
| -no_optimize | `mp_test_no_optimize` | 32.4 / 18.0, 24.1 / 15.0 | PASS; no weld: level.geom, its light mean 3.35 / max 17 against mp_test |
| -att | `mp_test_att` | 15.6 / 10.9, 16.6 / 11.9 | PASS; linear point lights: the page's colour, mean 0.66 / max 57 |
| -no_invalidefaces | `mp_test_invalid_kept` (one face's texture coordinates collapsed) | 15.8 / 11.0, 18.0 / 13.0 | PASS; the face kept: build.err 72 bytes (36 without), two more vertices |
| the abort | `mp_test_invalid` (the same face, `REFUSED`) | 1.4 / 0.6, 1.6 / 0.6 | PASS: XrLC exit 3 (Debug.fatal under `-nodlg`) twice, the Rust exit 1; "1 invalid faces" on all three; build.err 72 bytes byte for byte; the Rust writes nothing else, the C++ leaves build.lights and the 12-byte head of `level` |
| more than two pages | `mp_close_combat_pages` (20 px/m, 1 sample, its MU models) | 567.5 / 2402.9, 552.4 / 2370.1 | PASS: five pages both sides; 3,003 triangles on another page |
| implicit at nine samples, nine samples on a third scene, MU + implicit, MU at own settings | `mp_close_combat_whole` (own settings: 10 px/m, 9 samples, no `-noise`, 29 MU models / 102 references) | 1991.2 / 6543.5, 1826.4 / 6446.0 (+ xrDO_Light 89.7 / 132.8, 75.8 / 112.5) | PASS |
| MU at l03u's own settings | `l03u_agr_underground_own` (10 px/m, 9 samples, no `-noise`) | 1186.8 / 4295.7, 1154.1 / 4050.8 | PASS: two pages, 500 triangles on another page; level.cform under the rule of twice the C++ runs' (below) |
| the paged rule | `l03u_agr_underground_paged` (6 px/m, 1 sample, `-noise`: two pages over 31 sectors) | 220.6 / 869.4, 217.6 / 849.3 | PASS but the phase times (below) |

Measured on the new scenes (Rust against cpp1; lightmaps mean / p99 per channel at most): whole 1.a 0.44 / 7, 2.r/b
1.47 / 16 (5-bit, bound 3 / 16), 2.g 1.00 / 14, hemi 0.60 / 6, the terrain's implicit textures 0.04 / 1 (max 22),
vertex light 0.035, MU models' own light 0.002 / max 1, references lit alone trees 0.025 / 1, LODs 0.12 / 1,
level.details byte for byte, level.cform 103 one-sided (the C++ runs' 132); l03u_own 1.r 0.21 / 5, hemi 0.24 / 4,
vertex 0.013, MU references alone trees 0.002, LODs 0.04, level.cform 499 one-sided (theirs 497); pages 2.r/b 0.99 / 8, 1.a 0.23 / 3, vertex
0.035, level.cform 353 (theirs 313); l03u_paged 1.r 0.29 / 6, hemi 0.27 / 4, vertex 0.013, level.cform 452 (theirs
419); Draft: lightmaps 0, vertex 0.068, MU trees 0.245 / 5, LODs 0.35 / 3. Rust times against the C++'s (under the others' builds): whole 357 s
(4,898 s CPU) against 1,991 s; pages 149 s against 568 s; l03u_paged 48 s against 221 s; l03u_own 171 s against
1,187 s.

**9. No program is copied to be run.** XrCore of `cpp-compilers-last` takes no fs config on its command line: no
`-fsltx` anywhere, `Core._initialize("xrLC")` passes no `fs_fname`, and `CLocatorAPI::setup_fs_ltx` reads
`fsgame.ltx` three folders above the executable (`LocatorAPI.cpp`). Starting from `Bin\x64\CppCompilers` itself would
take XrLC and xrDO_Light parsing `-fsltx <path>` and passing it to `Core._initialize(name, 0, TRUE, path)` (xrCore
then takes `$fs_root$` from that file's folder) - a C++ change the brief allows only for the crash, so not made. What
is done instead: the stage is under `Intermediate/compile/stages` (the volume of `Bin`, never the temp folder) and its
bin folder holds **hard links** to the oracle's files (`link_binaries`): no byte of a program is written anywhere,
`fsutil hardlink list` shows the two names of one file. Every folder a recording makes under the temp folder (the
project copy, the Rust homes) is checked for a file with a program's name before it is removed (`no_programs`;
none in any run).

**10.** compare_compile.py's header lists what is recorded phase by phase of `CBuild::Run`; nothing is "never
recorded" but a level with implicit light loaded by the game (not on the brief's list; the files the game reads are
the ones compared) and -gi / -net (D24).

### What the new records found, and the rules

Nothing of the port. What failed on the first comparisons and why:
- **level.cform under Draft**: 92,001 of the C++'s triangles carry bit 14 or 15 of their word (suppress_shadows /
  suppress_wm). XrLC never sets them: `CollectorPacked::add_face` assigns the material and sector bit fields of a
  `TRI` on the stack and the two bits keep what the stack held (in High quality every face goes through
  SimplifyCFORM's `add_face` again; no High record shows a stray bit). The Rust writes 0. The rule leaves the two bits out (`STRAY_BITS`) and counts them; then
  the Draft level.cform is equal as a multiset. A Draft level of the C++ carried those bits to the game.
- **MU references in Draft**: at one sample the C++'s own two runs differ on references the C++ lights alone (trees
  0.086 / 3, LODs 0.26 / 2): its four CMULight threads draw sample directions from the one `::Random`, so no
  reference has a fixed sequence. New bound for a reference's light (`MU_REF_MEAN` / `MU_REF_P99`): 0.45 / 6 -
  scene mp_close_combat_draft, channels trees (seen 0.245 / 5) and LODs (0.35 / 3), under twice what is seen. A
  model's own light keeps 0.2 / 2.
- **Deflectors on other pages with the same shader list** (whole: every material on both pages on both sides, 463
  against 464 visuals): `paged` now also triggers when the visuals' shader sequence differs.
- **A progressive visual's coarser windows** follow the visual split; under the paged rule a progressive visual counts
  with its whole mesh (window 0). The C++'s own two runs still differ on mp_close_combat_pages in two wall_farm
  visuals (one has 132 against 216 faces in its whole window: its VIPM differs from run to run): 19 / 31 of 116,757
  triangles, the port 37 / 49. The paged rule's one-sided triangles (static, fast paths) are judged against twice
  the C++ runs' own (`PAGED_FLOOR`), at least 1e-4 of all.
- **level.cform against the C++ runs' own**: l03u_agr_underground_paged had 452 one-sided triangles against their
  419, l03u_agr_underground_own moved 0.0170 m2 of material word 1310754 against their 0.0089 (81.925 / 81.942 /
  81.933 m2: the port's value beyond cpp2, as far from it as cpp2 from cpp1) - three draws of QSlim's heap-ordered
  ties. Every level.cform count and area is now judged against twice what the C++ runs differ in (`CFORM_FLOOR`
  2.0; it was half again, and once only as much for the one-sided triangles) - under twice what was seen.
- **Phase times**: the C++ lights MU models on a thread beside the phases from "LIGHT: Starting MU" to "Converting
  MU-models"; the port inside "Starting MU". That span is judged as one row. And `settle()` waits up to 60 s for a
  quiet machine before the judged Rust compile.

The new checks of item 8: **the sector** whose visual tree holds a triangle's visual is part of the paged rule's key
(`visual_sectors`); **the sliding windows** are checked on every scene, each side by itself (`swi_problems`: inline
and shared - OGF_SWICONTAINER - windows of every progressive visual and fast path against its own indices and vertex
count). The check fails a corrupted window (tested: a window's vertex count set to 2, an offset past the end, a
window with more faces than the one before). Counts: whole 10,908 windows each side, pages 10,748 / 10,750,
l12u 9,401, l03u 1,013. build.err is now compared everywhere (byte for byte on every scene).

### Proof

Once, at the end, on the fast build of this tree (`Intermediate/cargo/fast/Grizzly.exe`, 00:24):
- `cg.sh clippy -p gz-fmt-level -p gz-lc-light -p gz-lc --all-targets -- -D warnings`: clean (9 s); `fmt -- --check`:
  clean; `cg.sh test` of the three: 54 + 1 + 41 + 18 green, 35 s with the build. No Rust file of mine changed.
- `python tools/qa/diff_compile.py --all`, 03:56 - 04:37, 41.5 min, no C++ run (every record there): 17 scenes PASS,
  every second compile byte for byte (16 scenes; the refused one compiles nothing), mp_test and l03u_agr_underground
  on one thread byte for byte against all cores.
- `find D:\XFinedProjects "D:\Games\Dead Air\modules" -newermt '2026-11-13 00:04:51'` at 04:37:51: nothing. No
  compiler process, no stage under `Intermediate/compile/stages`, no `gzlc_*` folder under the temp folder; the
  diagnosis stage (copies, made before the links) removed.

Not done, for the lead: the change in `D:\Games\XFined-Editor-cpp` is uncommitted, and the oracle in
`Bin\x64\CppCompilers` is built from it; `CPP_SOURCE` still names `cpp-compilers-last`, so the records made with the build before it stay
(the rebuild changes no result on mp_test). `oracle_before` under `Intermediate/close-compiler` holds the previous
oracle's binaries. Starting the oracle from `Bin` itself needs `-fsltx` in XrLC and xrDO_Light (item 9). A level with
implicit light loaded by the game was not run.

### For the plan

- Section 9, "Not proven against the C++": empty but the game loading a level with implicit light.
- Section 9, "compared this way": add the scenes above; the oracle is cpp-compilers-last with XrLC linked to a 128 MB
  stack (the crash was XrETools' recursive optimiser on the 1 MB main thread).
- Section 9, the rules: level.cform's suppress bits left out; an MU reference's light 0.45 / 6; the paged rule's
  sector key, whole meshes, one-sided triangles against twice the C++ runs'; the sliding windows held per side;
  build.err byte for byte; a refused input.

## Closing: the lead's part and the run on the final tree

13 Nov. The author's word after the final review: the editor is MIT, and nothing stays open. Three engineers closed
Play in place ("Closing: Play in place"), the compiler's proof ("Closing: the compiler's proof") and the old notes
("Closing: the old notes", "Open: nothing" at the top). The lead's part:

- **The license**: `LICENSE` (MIT), the workspace's `license = "MIT"`, the README's section; `Build.ps1` puts it
  beside the programs as `LICENSE.txt`.
- **The LZO decoder** is `lzokay-native` (MIT, safe Rust, a port of lzokay - an implementation of the format of its
  own, long in use), with its compressor a feature only the tests turn on: one crate instead of two, of the editor's
  own license. `gz_chunk::lzo::decompress` keeps its contract (exactly the stated size, the stream ends where the
  input ends, the output grows only as the stream fills it).
- **The game's commit** (`D:\Games\Dead Air\DeadAir-x64`, `3816558ff`, by path - the Play engineer's fourteen files,
  nothing else of that tree) and the check of its deploy: the installed `xrEngine.exe`, `xrCore.dll` and `xrGame.dll`
  equal `bin\x64\Release`, the backup `_predeploy_fsltx_20261113` holds 75 files.
- **The oracle's commit**: the C++ working copy `D:\Games\XFined-Editor-cpp` (branch `cpp-compilers`), `4b11cc22` -
  XrLC links with a 128 MB stack; the tag `cpp-compilers-last` points at it.
- **A job started on the UI thread is the request's** (`gz-task` `in_scope_sync`, `current_scope`; `gz-app`'s UI
  queue takes the caller's scope along): the sweep's "running" answer of a request past its timeout missed a job a
  command starts through the shell's queue. A test holds it.
- **The comparison's everyday run is its quick scenes** (`diff_compile.py` `QUICK`, ten scenes, eighty seconds);
  `--all` is all seventeen, each compiled twice - forty minutes, for a change of the compiler.
- **A fault of the lead's monitoring**: a `git status` in the game repository every 25 minutes, while the Play
  engineer's suite listed the installed game (the repository lies inside the installation) - one of its runs failed
  on `.git`. The suite's listing leaves that `.git` out; the lead runs no git command there while a suite runs.

**The run on the final tree** - 13 Nov, 04:41 - 05:06, commit `157bfacb`, the release build, one step after
another (`Intermediate/final4/`, `summary.txt` and a log per step):

| Step | Result | Time |
|---|---|---|
| `Build.ps1` | the two programs, their PDBs, the notices, `LICENSE.txt` | 266 s |
| `check.sh` | ok | 37 s |
| `test.sh --all` | 78 binaries, 1194 tests green; the net: 3769 events in the tests' temp folder and 685 in the repository, no program file | 209 s |
| `diff_editors.py --all` | 10 scenarios, 755 steps, 746 written files, 0 differ | 43 s |
| `diff_compile.py` | the ten quick scenes PASS | 80 s |
| `diff_compile.py --scene` l12u_control_monolith, mp_close_combat_implicit, l03u_agr_underground | PASS | 312 s |
| `run_all.py --with-game` | 24 suites, 24 passed | 246 s |
| `test_play_game.py --scene l03u_agr_underground` | 36 checks pass, the installed engine in place | 115 s |
| `test_play_game.py --scene mp_close_combat` | 39 checks pass: implicit light and MU models in the game, the level in 19 s after Play | 140 s |

All seventeen scenes of the compile comparison, each compiled twice, ran at the end of the compiler engineer's pass
on the same compiler sources (`Intermediate/close-compiler/final_all.out`: 17 PASS, 2492 s). The installed game's
listing before and after each game run is equal (107,495 entries); nothing changed under `D:\XFinedProjects` or the
game's `modules`; no process, no Play root and no temp folder left.

## The last review, the game and Play

The brief `docs/rust/briefs/fix-game.md`, 13 Nov, both repositories; nothing committed.

**The ability, stated by every program that implements it.** The game's `xrEngine.dll` (the quoted `-fsltx`),
`xrCore.dll` (the config elsewhere, the paths) and `xrGame.dll` (the content service, the update check) carry the
same `EngineAbilities` as `xrEngine.exe` in version resources of their own (`src/xrCore/ModuleVersion.rc2`, a
`version.rc` per library). Play reads all four (`play.rs` `ability_of`, `GamePrograms::stating`): a word counts only
when every one states it. A mixed installation - a library of an older build beside a newer engine - takes the copy
path, and the answer's `play_reason` (and the log line) names the library (superseded: "Play in place only" - it is
refused, naming the library). Before this deploy the installed game was
exactly that: a new `xrEngine.exe` beside libraries with no version resource.

**The path limit, what the game says.** UCRT `_splitpath` gives a folder over 256 characters back empty, and
`GetFullPathNameA` stops at `MAX_PATH` even in a long-path aware program. Replaced on the way a path takes:
`CLocatorAPI::Register`'s folder loop, `EFS_Utils::ExtractFilePath` (an ini file's `#include` folder), `xr_ini.cpp`'s
include split and the application folder (`xrCore.cpp`) by `xr_split_folder` (`FileSystem.h`, `_splitpath_s` with
`string_path` buffers), the protected mode's `real_path` by `_fullpath`. The limit stays 519 (`string_path`), now
proven: `Intermediate/fix-game/prove_long.py`, a root of 330 characters with a space in it, the config named quoted,
every folder indexed in it over 256, its longest path 519 - the new game's level in 19 s, the application folder
listed equal (746 entries); the installed build before this change stopped at once in `CInifile::Load` (`path &&
path[0]`). The editor adds what the game does not decide: the root is the engine's working folder, and
`CreateProcessW` refuses one over 258 characters (`ERROR_DIRECTORY`, measured), so `check_path_lengths` refuses such a
root (`WORKING_FOLDER_LIMIT`). The renderers' DirectStorage opens an archive by a path up to `MAX_PATH` and reads a
deeper one the ordinary way; the console's cloud and tree dumps stop at `MAX_PATH` - neither is on a path Play takes.

**The early crash report** moves beside the config only for a config outside the application folder (the folder of
`xrCore.dll`, the rule of `CLocatorAPI::setup_fs_path`); a config inside it reports into the working folder as
before. Proven by a start from a working folder of its own with the application folder's config named outright.

**The copy path, run for real** (superseded: "Play in place only" - there is no copy path, no `engine=` and one
run of the suite). Play takes `engine=` (`auto`, the default, or `copied`: copies whatever the engine
says, for a game whose in-place run the author does not want). `test_play_game.py` runs `mp_test` both ways on one
root (`--engine both`, mp_test's default; one way for the other scenes): in place, then copied - the copied engine and
its libraries all listed by the marker, the junction to the installed archives, the copy's content records in the
root's `.dead-air-x64`, the level loaded from the copy; the root removed with its junction unlinked as a link, the
installed `database` keeping its entries, the installed game's listing equal. Line 90's comment says what is true: the
compile comparison holds implicit light with MU models to the C++ record (`mp_close_combat_whole`), and the game loads
it. Every console program `gz-levelbuild` starts (`taskkill`, `reg.exe`, `cmd /c mklink`) goes through
`process::console`, which sets `CREATE_NO_WINDOW`; the engine is started as it is.

**Proof** (logs in `Intermediate/fix-game/`):
- The game: `build_x64.ps1 -Configuration Release -Clean` (CMake changed) 3102 steps, 8 min 46 s, then `no work to
  do`; `git diff --check` clean; `prove_long.py` 14 / 0 (elsewhere 22.5 s, inside 12.2 s, plain 9.7 s); the negative
  control on the installed build 3 fail as expected.
- Deploy: the game closed, 75 files backed up into `D:\Games\Dead Air\_predeploy_longpaths_20261113`, the 58 files of
  `runtime-files.txt` and the PDBs by `deploy_live.py`, SHA-256 of 74 files against `bin\x64\Release`: 0 differ; the
  four programs installed say `fsltx-in-place long-paths`. `Run-NqQa.ps1 -Scenario Scout` PASS 6 / 0.
- The editor: `check.sh` - clippy clean, registry, encoding, test files clean; fmt and the icon check fail in the
  shell engineer's files (`grizzly`, `gz-app`, `gz-ui-level`, `gz-project`; `Grizzly.exe` still subsystem 3).
  `test.sh` 53 binaries run, 1198 tests green. `run_all.py test_scene_tools test_play_game` 2 / 2 (23.6 s, 103.2 s:
  60 checks, in place the level in at 20 s, copied at 16 s, the installed game's 107,668 entries equal).
  `test_play_game --scene l03u_agr_underground` 37 / 0, 224.2 s (compile 141.5 s, the level in at 29 s).

## The last review, the shell

The brief `docs/rust/briefs/fix-shell.md`, 13 Nov; nothing committed.

**`Grizzly.exe` is a window program** (PE subsystem 2, was 3; `grizzly-mcp.exe` stays 3, a stdio server).
`tools/rust/check_icon.py` reads the subsystem beside the icon, `check.sh` runs it on the release build. Started from
Explorer the process has one window, the editor's, and no console host: nothing can close it behind the unsaved-work
question. A run that prints - `-dump_commands`, `-catalogue`, `-compile`, `-compile_details`, `-log_echo`, `-headless`
(`cli.rs` `speaks`) - calls `gz_win::attach_parent_console`: a stream the launcher redirected (a pipe, a file) stays
exactly what it was, a missing one is taken from the parent's console. A plain window run takes no console, so closing
the terminal it was typed in does not end it. cmd's `> file` and `| more`, PowerShell's `| more` / `| Out-File`, bash
and every Python pipe of the QA tools read what they read before; PowerShell's own `> file` and `$x = ...` capture
nothing from a window program (its rule for GUI programs: they are not redirected at the end of a pipeline) - pipe it.
The editor's console children already had `CREATE_NO_WINDOW` (`gz-app`'s `reg.exe`, `gz-github`'s `gh`; `explorer.exe`
is a window program); `gz-levelbuild`'s are its engineer's.

**The three choosers list** (`gz-ui-level`'s `lists.rs`, as `EditorChooseEvents.h` filled them): `skeletonbones` - the
bones of the row's model (`OGF_S_BONE_NAMES`), `skeletonanims` - every motion of the model, its own and those of each
`.omf` its motion references name, once each, `gameanim` - the game's `anims\*.anm` / `*.anms`. A model is looked for in
the project, the editor's data, then the game. The row carries its list's parameter now (`gz-scene`'s `props::Choice`:
the list, the model, `max_items`): Fixed bones takes several (a comma list, the ones it has picked). The typed fallback
and its sentence are gone; an empty list says what is missing ("The object has no model.", "The model ... has no
motions."). `property_choose` (`gz-ui-level` registers over `gz-scene`'s) answers the list with `open` / `status`;
`property_inspector` names a choose row's `list` and gives the object's own (`choices`) - `diff_editors.py` lists both
fields as added by the wire (D13). On Escape: a physic object with `stalker_neutral_1` 47 bones, `zone_burning_fuzz_weak`'s
Attack animation 2 motions, a helicopter's Motion 295 animations.

**What the shell called "not part of this build".** Every build installs those surfaces (`grizzly`'s test
`the_editor_installs_every_surface_the_contract_names`: every document kind, dialog, both drawers, the Start page, the
scene's file rows). Removed as dead: `-project`'s "cannot open projects yet", the Build chip's "level builders"
branch, `gz-app`'s own `COMMAND_LOAD` / `COMMAND_SAVE` rows (the Level tab's rows win in every build). Kept for a shell
nobody installed into, said truly: the Level placeholder, `ui_tab` / session restore of a kind, `ui_dialog`,
`ui_drawer`, the Content Browser popups, an empty menu ("... installed in this shell"), the Start page fallback
("game_link over MCP"). `COMMAND_LOAD` without a scene and `COMMAND_SAVE`'s Save As open the Open Scene / Save Scene As
dialogs for a person; an MCP caller gets no window and a refusal naming `p1s` (`test_level_commands.py` E).

**Proofs**: `Build.ps1 -Only editor` 296 s; `check.sh` clean but fmt (25 s: `gz-project/src/cmd.rs:835`, not this
brief's file); `test.sh` 12 binaries run, 1198 tests green (28 s); `test_properties` 47/47, `test_level_commands` 13/13,
`test_mcp_bridge` 20/20, `test_server_lifecycle` passed, `test_ui_shell` 79/79 (run directly, each on a port of its own:
`run_all.py` refused while another engineer's editor ran); `diff_compile.py --scene mp_test` PASS (43 s);
`diff_editors.py` `s1_shell` 0 differ, `s6_scene` 0 differ (after the two wire fields).

## Play in place only

The brief `docs/rust/briefs/play-in-place-only.md`, 13 Nov, both repositories; nothing committed.

**One way.** Play starts the linked game's installed `xrEngine.exe` where it is, `-fsltx <root>\fsgame.ltx`, or it does
not start it. The copy path left `gz-levelbuild`: `EngineChoice` (`engine=`), `PlayMode`, the copies of the engine and
its libraries, the junction to the archives, the content records copied into `.dead-air-x64`, the config named by its
bare name, `GamePrograms`' `libraries` / `runtime_list`, `PlayRoot`'s `mode` / `reason` / `programs` /
`program_bytes`, and `pe.rs` (155 lines, read by nothing else): `play.rs` 1257 to 979 lines, the suite 620 to 575.
`play::ready_engine` reads the version resources of the engine and of `xrEngine.dll`, `xrCore.dll`, `xrGame.dll` and
refuses in one sentence that names the program - the engine that does not state `fsltx-in-place`, or the libraries of
a mixed installation by name - "Play needs a Dead Air Refined build that states it, so update the game". `cmd.rs`'s
`play_place` asks it before the root check, so `play` / `play_game` refuse before a build starts; `prepare` asks again.
The answers keep `exe` / `exe_path` and lose `play`, `play_reason`, `programs`, `program_bytes`; the log line says the
installed engine was started in place in the root. A root an earlier editor made for a copied engine stays Play's: its
marker's `program =` files, its `database` junction (unlinked) and its `.dead-air-x64` go when Play prepares it; Play
writes no `program =` line (`play_root.rs` still reads them; its rules are unchanged). The limits: `long-paths` and
Windows long paths on 519, off 259, an engine without the word 255; `WORKING_FOLDER_LIMIT` 258.

**The registry read** is `gz_win::long_paths_enabled` (`crates/gz-win/src/registry.rs`, `RegGetValueW`,
`Win32_System_Registry`); `reg.exe` is no longer started. `process::console` stays for `taskkill` and the tests'
`mklink`. `test_scene_tools` phase Y read `play` / `programs` of `play_root`'s answer and now wants neither.

**The game** (`D:\Games\Dead Air\DeadAir-x64`, uncommitted): the three wide buffers on a path of the fs are
`string_path`-sized, a conversion that fails is the dump's log line and no save - `dump_cloud_map` (three
conversions; it also makes its folder now, `VerifyPath`: a fresh app data has no `screenshots`), the trees dump in
`da_trees_bake.cpp` (two), DirectStorage's `ArchiveFile` (the whole path; a refusal still reads the ordinary way). No
other site takes a path of the fs through a short buffer (`dx11HW.cpp`'s pipeline cache is `string_path`-sized
already; the rest are the application or system folder, names or messages). `CONTENT_BUNDLES.md` says so,
`TEST_MATRIX.md` has the row "A dump on a deep fs root".

**Proof** (logs in `Intermediate/play-only/`):
- The game: incremental Release 125 steps, again 8, then `no work to do`; `git diff --check` clean;
  `prove_dump.py` (prove_long.py's layout, the root 330 characters with a space, no `screenshots` folder, a script
  that runs `r__cloud_map_dump` once the level stands) 7 / 0, 26 s: `cloud_map.png` at 364 characters (1.2 MB) with
  both march PNGs, the application folder equal (746 entries). The first run, before `VerifyPath`, failed the save for
  want of the folder.
- Deploy: the game closed, 74 files backed up into `D:\Games\Dead Air\_predeploy_widepaths_20261113`, the 58 runtime
  files and the PDBs by `deploy_live.py`, SHA-256 of 74 files against `bin\x64\Release`: 0 differ; changed:
  `xrCore.dll`, `xrRender_R4/R5/R6.dll` and their PDBs.
- The editor: `test.sh` 52 binaries run, 1195 tests green (191.6 s); `run_all.py test_scene_tools test_play_game`
  2 / 2 (21.8 s, 62.2 s: 42 checks, the level in at 14 s, the answers without the old keys, the installed game's
  107,743 entries equal); `check.sh` clippy, fmt, unsafe, registry, encoding, notices clean; test files clean after the
  registry test's `reg.exe` got its in-memory mark (`check_test_files.py` again, exit 0); the icon step not run (no
  release build in `Bin\x64\Release`).

## The last review: the lead's part and the run on the final tree

13 Nov. The last review (`docs/rust/briefs/critic-close.md`) said "Open: nothing" was not yet true; its findings are
the rows at the end of that table. Three engineers closed the shell ("The last review, the shell"), the game's
ability and paths ("The last review, the game and Play") and Play's one way ("Play in place only"). The lead's part:

- **Bounds, licenses, notes, the ledger** (`a5331965`) and **the root's files of the author** (`b551b483`), as the
  table says.
- **Play runs the game in place and no other way** - the lead's decision, against the brief that had asked for the
  copy path's real run: the copy was what the author's antivirus answers to (a fresh unsigned program copying 57
  programs into a temp folder and running the copy), its only honest proof made a suite do exactly that on every run,
  and before the editor ships no fallback to an older game is owed. D1, I1 and `CLAUDE.md` say so.
- **`Grizzly-Cmd.exe`**, the editor for a command line (`crates/grizzly/src/cmd_twin.rs`, the package's second
  program; `gz-win` `job.rs` `end_with_this_process`): a shell does not wait for a window program, and PowerShell's
  `>` took 0 lines of `Grizzly.exe -dump_commands`. The twin runs the editor with the same arguments and its own
  streams, waits and ends with its exit code; the editor ends with it. Seen in a console of its own (the catalogue
  line, 204 commands, a refused compile's exit code 1), in PowerShell (`>` 204 lines, a variable, exit codes 0 and
  1); held by `check.sh` ("command line") and `test_server_lifecycle` (a headless editor prints through the twin and
  ends it with 0 when closed over MCP; an editor whose twin is ended is gone with it). `Build.ps1` copies it,
  `check_icon.py` wants it a console program with the icon.
- **The log of an open editor is nobody else's**: `-dump_commands` writes no log (`check.sh` ran it on every check
  and overwrote `logs\grizzly_<user>.log` of the repository root), a compile run writes
  `grizzly_compile_<user>.log`, and `check.sh`'s refused compile runs on a home under `Intermediate`.
- **Two refusals** said "in this build" of what no build has - `list_assets` for a category no area lists (the
  editor installs a lister for each of the ten), `asset_preview` for a category with no thumbnails (the C++
  sentence): both say what is so (D16).
- **The game's commits**, by path, nothing else of that tree: `232c5db9f` (the ability in every program, the folder
  split, the crash report's condition), `ccf33e523` (the renderers' buffers), `aaf8b4adc` (two paths of
  `SEASONS.md` that had lost a backslash and an `a` to a bell character - found by the same search that caught
  one in the new row of `TEST_MATRIX.md`). Both deploys checked: the backups `_predeploy_longpaths_20261113` and
  `_predeploy_widepaths_20261113`, SHA-256 equal to `bin\x64\Release`.

**The run on the final tree** - 13 Nov, 07:03 - 07:30, commit `e79f44ba`, the release build, one step after another
(`Intermediate/final6/`, `summary.txt` and a log per step):

| Step | Result | Time |
|---|---|---|
| `Build.ps1` | the editor, its twin, the bridge, their PDBs, the notices, `LICENSE.txt` | 292 s |
| `check.sh` | ok: the editor a window program, the twin and the bridge console ones, each with the icon; the twin answers as the editor | 25 s |
| `test.sh --all` | 79 binaries, 1195 tests green; the net: 3605 events in the tests' temp folder and 691 in the repository, no program file | 131 s |
| `diff_editors.py --all` | 10 scenarios, 755 steps, 746 written files, 0 differ - the C++ side recorded anew for each (the linked game's engine changed with the deploy, and a record is keyed by it) | 196 s |
| `diff_compile.py` | the ten quick scenes PASS | 85 s |
| `diff_compile.py --scene` l12u_control_monolith, mp_close_combat_implicit, l03u_agr_underground | PASS | 339 s |
| `run_all.py --with-game` | 24 suites, 24 passed | 282 s |
| `test_play_game.py --scene l03u_agr_underground` | 39 checks pass, the installed engine in place | 107 s |
| `test_play_game.py --scene mp_close_combat` | 42 checks pass: implicit light and MU models in the game | 131 s |

The installed game's listing before and after each game run is equal (107,743 entries); nothing changed under
`D:\XFinedProjects` or the game's `modules`; the root's project list is the author's; no process and no Play root
left.
