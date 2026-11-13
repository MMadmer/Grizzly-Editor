# Brief L1b - building a level: one pipeline, no hand-made steps

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, the audit
`Intermediate/plan2/ux_level.md` sections **B4, B5, B6, B7, B8** and the blocking audit **D** (the Rebuild chip, the
render thread's waits, Play's copies), `Intermediate/plan2/commands.md` 3a ("level builds: 20 names, one
handler") and 1.6 (four ways long work answers today), and CLAUDE.md's sections on the level compiler, Play and
"Where a mod exports to" (the write guard, Play's root rules - all of that stays).

You own: `crates/gz-levelbuild`, `crates/gz-export-level`, the entry points of `crates/gz-lc` and
`crates/gz-lc-light` (not their algorithms), `crates/gz-scene/src/bulk/` (AI map, detail objects, wallmarks) and
`crates/gz-scene/src/objtools/{sector.rs,portal.rs}`. Stream L1a works beside you in the rest of `gz-scene` and
in `gz-entities`: it removes the edit target, changes the scene's file format and owns the snapshot types - when
it changes one you use it fixes your call sites; when you need a change in its files, ask through your report
and work with what is there. The UI (`gz-ui-level`, `gz-ui-viewport`) is stream L2's, after you: you keep it
compiling with the smallest edits.

## 1. Build Level - one action, the right steps, in order

Today there are about twenty entry points in three places, and the order of the classic pipeline is the author's
to know (audit B6). The model:

- **A build is a pipeline of steps with inputs and outputs**: collision, AI map, game graph, spawn, geometry and
  light (the compiler), details, detail light, the mod's level layers (spawn layer, collision overlay, visual
  overlay, level cut) - each step declares what it reads (which parts of the scene's snapshot, which settings,
  which files) as a fingerprint and what it writes.
- **`level.build`** (`steps`: left out = everything stale, in dependency order; `only`: one step and what it
  needs; `force`; `quality: draft | standard | final` for the compiler; `wait`): one job of the group
  `level_build`, a card in the Task Center naming the step and its progress, Stop between steps and inside the
  long ones as today. The compiler stays the editor's own code on every core.
- **Fingerprints are saved beside the level's build output** (the project's `gamedata\levels\<level>\` holds
  what was built; add `build.state` there: Lua data, the fingerprint and the time of every step), so "what is
  stale" is right after the editor restarts. Today it lives in memory and the Rebuild chip is orange on every
  open (audit B6), re-fingerprinting thousands of objects every second (`gz-levelbuild/src/service.rs:98-133`,
  `gz-ui-level/src/status.rs:41-66`): make it a `Query` keyed on the scene's revision - no polling.
- **`level.status`**: per step - fresh, stale (and what changed: "12 objects moved"), never built, failed - plus
  the last build's problems.
- **`level.problems`** (the compiler's and the build's findings, positioned on objects: `{object, text, code}`;
  `clear`): replaces `compiler_errors` and the Import / Export / Clear Error List menu rows.
- **`level.play`** (`action: start | stop | status`, `clean`, `in_game`): builds what is stale, then starts the
  installed game in place exactly as now (the ability check, the root rules, `-fsltx`). Play's gamedata copy
  becomes hard links where the volume allows, as the module build does (`gz-build`'s `install.rs`), with the copy
  as the fallback (audit D).
- **`level.export`** (`kind: spawn | collision | visuals | cut | obj`, `objects`): the mod's level layers and the
  OBJ export - `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe`,
  `scene_export_obj`.
- **`level.validate`**: `scene_validate`.
- **`level.compile`** stays the folder compiler for a `build.prj` without a project, as a command
  (`folder`, the compiler's flags as named parameters) so the commandlet `Grizzly-Cmd.exe run level.compile
  folder=...` replaces the `-compile` flag; remove the flag and `crates/grizzly/src/compile.rs` with it (tell the
  architect in your report: `cli.rs` is the architect's file - you may make that edit, it is yours for this).
- The 17 alias commands (`build_cform`, `make_all`, `compile_geometry`, `play_clean`, ...),
  `level_rebuild_status`, `reload_configs` (becomes `level.play action=reload_configs` or goes if nothing but a
  running Play needs it) are deleted, with their callers fixed.
- A `level.*` command that needs the scene when none is up loads the project's scene first (stream L1a gives
  `scene.*` the same): `Grizzly-Cmd.exe run level.build` builds from a cold start.
- One way to answer long work (`docs/COMMANDS.md` 3): the command waits for its job, `wait=false` answers the
  job id, `job.*` follows it. No `action=status` polling contract, no `answer_only`.

## 2. Visibility cells are the build's work

Sectors and portals are made by hand today: create the default sector, compute portals, validate, and a button
that does nothing (audit B5).

- The build's geometry step makes the default sector when the scene has none and computes portals itself
  (`gz-levelbuild`'s `prj.rs`, the code behind Create Default / Compute All); problems are build problems on the
  objects involved.
- Hand-made sectors and portals remain possible - that is the mode `zones` (stream L2 gives it a face): the
  model keeps "authored" zones, and the build uses them and fills what they do not cover. A scene that has hand-
  made zones keeps them through the format change (L1a writes `zones.lua`; agree on its shape with what your code
  reads - you own the two tools, L1a the file: tell L1a's report reader what you need by writing it into
  `docs/assets/SCENE.md`'s zones section yourself).
- Commands: `scene.zones` (`action: list | create | assign | compute | validate | remove`) replaces
  `sector_tool` and `portal_tool`; "Distribute Objects" goes.

## 3. The ground is the level, not a hidden list

Half the tools read a "Snap Objects" list the author fills by hand (audit B4): the AI map generates over it, the
detail slots lie on it, wallmarks need it. Replace it:

- **The ground** of every tool is every static object the collision build takes, narrowed only by an optional
  volume or an explicit `objects` list. The Snap Objects list leaves the model and the scene file (an old scene's
  list is read and, when it is not "everything", kept as the explicit list of the tools that used it).
- **AI map** (audit B7): generation happens inside an **AI Map Bounds** volume (a shape of the scene with that
  role; a scene without one generates over the whole ground) with the parameters on the tool (`can_up`,
  `can_down`, smoothing ...; they leave Level Settings); it is a build step, so the status says when it is
  stale. Node editing (brush, links) stays as the mode `ai_map`. `scene.ai_map` (`action: generate | clear |
  paint | link | info`, `wait`) replaces `aimap_tool`.
- **Detail objects** (audit B8) become painted foliage: a list of detail meshes with density, scale range and
  random rotation; a brush paints and erases them on the ground; the data stays the game's detail slots
  (`level.details`), so nothing changes for the game. The colour-index base texture becomes an import
  (`scene.details action=import_texture`) and an export, not the way to work. `scene.details`
  (`action: list | add | remove | set | paint | erase | fill | clear | import_texture | info`) replaces
  `details_tool`; the detail light compile is a step of Build Level.
- **Wallmarks**: `scene.wallmarks` replaces `wallmark_tool`, on the same ground.
- `fog_tool`, `light_tool`, `shape_tool`: what they did beyond setting properties (L1a's `scene.set`) and placing
  (`scene.place`) - if anything is yours (in `bulk/`), it gets one command named for what it does.

## 4. Nothing waits where the author works

Audit D, your rows: the Rebuild status (above); the render thread's blocking GPU waits per pass
(`gz-render/src/renderer.rs:394-397`, `frame.rs:310-340` - not your crate: describe the fix in your report for
stream L2, do not edit it); Play's copy (above). Every job you start has a stage text a person can read and a
fraction when it can know one; a build that cannot start says why in a sentence (`code: busy` names the job in
the way).

## 5. A regression net for the compiler

The differential tool against the C++ compilers is gone with its records. Give the compiler a net of its own:
`tools/qa/test_level_compile.py` compiles two small scenes of the editor's library at draft quality and compares
the outputs' structure with records in `tools/qa/golden/compile/<scene>.json` (counts of visuals, sectors,
portals, light maps and their sizes, collision triangles, a hash of the deterministic files) - records the suite
writes itself with `--record`, small enough to commit, compared with a tolerance where a value depends on thread
timing. Under two minutes; in `run_all.py`.

## Proof

- Rust tests of the pipeline: dependency order, staleness by fingerprint (change one object - the steps that
  read it are stale, the others fresh), state saved and read back, Stop between steps leaves the finished ones
  fresh; default-zone generation on a small scene; ground selection; the detail paint model to slots and back.
- QA: `tools/qa/test_build_job.py` stays the module's; yours are `test_level_commands.py` (the build side,
  rewritten for `level.*`), `test_level_compile.py` (new), `test_play_game.py` (the names; it runs with
  `--with-game` only). A phase proves the status after an editor restart: build, close the editor, start it,
  `level.status` says fresh; move an object, it says which steps are stale.

## Done means

PLAN section 5 "L1" for the build half, every sentence proven; one build command and one status; no alias left;
no step an author must run by hand to get a level that loads; `docs/UI_DESIGN.md` untouched by you (L2 writes
the level's surfaces) but your report lists what L2 must show: the steps, the states, the modes.
