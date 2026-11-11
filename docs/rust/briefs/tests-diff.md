# Brief: the differential run against the C++ editor

The C++ editor is the specification. This pass runs the same commands on both editors and
compares what they answer and what they write, then fixes the Rust editor where it differs.
You are alone in the tree. Read `docs/rust/CONVENTIONS.md`, `docs/rust/STATE.md` (all of it:
its "Findings" list the differences the packages already know of) and
`docs/RUST_MIGRATION.md` sections 9, 10 and 12 first.

You may edit `crates/`, `tools/qa/`, `tools/rust/`, `docs/rust/`. `Source/` and the game under
`D:\Games\Dead Air` are read-only. The C++ editor is used as built (`Bin\x64\Release\
LevelEditor.exe`, port 28016, fixed); never rebuild it.

## The tool: `tools/qa/diff_editors.py`

One Python script (stdlib only), written to stay: it is the parity gate of the cutover.

- **Two copies of one project** in a temp folder; the C++ editor opens one, the Rust editor the
  other (`tools/qa/qa_target.py` knows both binaries; the Rust one runs with `-mcp_port`, and
  `-headless` wherever the scenario needs no picture). Both get `-nodlg`, and `-qa_github` in
  every scenario that packages (with `GRIZZLY_CATALOG_HOME` in a temp folder: no author key,
  an unsigned release on both sides). `export_target` of both copies points into the temp
  folder, never at the game. The repository's own files the editors write (`qa_target.KEPT` of
  both targets) are put back byte for byte.
- **If something already answers on 28016, stop**: that is the author's own editor. Do not
  close it, do not wait for it; report it and run only what needs no oracle.
- An editor you started is closed with `window_control close` (the `*_discard` form), never
  killed unless it does not leave within its timeout.
- A **scenario** is a list of steps `{cmd, args}` run on both sides in the same order; after
  each step the two JSON answers are compared, after the scenario the two project copies and
  the two output folders are compared file by file.
- **Normalisation is explicit and small**: the copy's path becomes `<project>`, the temp
  folder `<temp>`; a table in the script names every field that is ignored, per command, each
  with its reason (timings, pixel rectangles and DPI, revision counters, the two deliberate
  differences of section 9 - signature bytes and packed bytes). Nothing is ignored by pattern
  or by default. A field not in the table that differs is a failure.
- Output: one line per step (`same` / `DIFF <path in the answer>: <c++> | <rust>`), a summary,
  exit code 1 on any difference; `--scenario <name>` runs one; `--write-parity` writes
  `docs/rust/PARITY.md` - every command of `crates/gz-mcp/tools.json` with its state: compared
  and equal / compared with the ignored fields named / not compared, and why.
## Time

The user's rule: **nothing that already passed is run again to check something new.**

- **The oracle is recorded once.** The C++ editor does not change on this branch, so what it
  answers to a scenario on a given project is a fact. Its side of every scenario - each step's
  answer and the files it wrote (path, size, SHA-256, and the bytes of those a diff will have
  to show) - is kept under `Intermediate/oracle/<scenario>-<key>/`; the key is a hash of the
  scenario's steps, `LevelEditor.exe` (size, write time), the source project (paths, sizes,
  write times) and the linked game's version. A run starts the C++ editor only for a scenario
  that has no record (`--refresh-oracle` forces it), so a busy 28016 stops only those.
- **Only what changed runs.** A scenario's last result is remembered with `Grizzly.exe`'s
  fingerprint and the scenario's key. The default run takes the scenarios that failed last
  time or whose key or editor changed; `--all` takes every one; `--scenario` one. While you
  fix: that scenario alone. The whole set once, at the end.
- **The Rust side runs scenarios side by side**: each on its own port, project copy and home
  (next point), headless wherever no picture is needed, about half the cores at once.
- **The editor's own files are isolated.** Add `-home <dir>` to the Rust editor: the folder
  for what it writes about itself (`_projects.ini`, `grizzly_editor.ltx`, `logs\`); the
  repository root when the flag is absent, as today. `qa_target` gives every Rust editor it
  starts a temp home (`GRIZZLY_QA_HOME` names one from outside), so no test touches the
  author's project list or settings and two editors never share a file. The C++ editor has no
  such flag: its files are still backed up and put back byte for byte.
- An editor starts once per scenario, never per step; polling at 50 ms.
- Budgets, all three named in the report: a first full run (the oracle recorded on the way)
  under 5 minutes; a full run with the oracle already recorded under 90 s; a run with nothing
  changed, a few seconds.

## Scenarios

Sources (copied, never opened in place): `D:\XFinedProjects\DARF-Overnight-Delivery` (a real
mod: quests, characters, a PDA layout, screens) and `D:\XFinedProjects\Test` (the Escape scene,
`rawdata\levels\l01_escape.level`). Add the graphs of `docs/nq/examples` to the first copy.

1. **Reads.** Every read command of the catalogue that needs no window: the manifest, the game
   link and modes, the quest catalogue, list, get, validate, check-all, layout, references and
   every `quest_lookup` type; characters list / get; `world_get`, `world_transitions`; the PDA
   list / get / validate; `screens_get`; assets and spawn sections; `ui_contract`,
   `list_commands`, `shortcuts`, `preferences`; the content listing.
2. **Manifest edits.** The same series of `mod_set_manifest`, `mod_exclude`, `world_set`
   (rules, player choices, the start kit, `preset=empty`), `screens_set` on both: `mod.ltx`
   equal byte for byte after every step.
3. **Assets as text.** `quest_new`, the edit commands (`quest_apply` / `quest_edit` /
   `quest_write` / `quest_rename_task` / `quest_layout`), undo and redo, `quest_save`, for a
   quest and for a behaviour graph; `characters_write` / `_delete`; `pda_write` (the template),
   `pda_use`, `pda_delete`; `content_create` of every kind: the files equal byte for byte.
4. **Content operations.** `content_mkdir` / `_move` / `_rename` / `_copy` / `_delete`,
   including the names of `test_unicode_names.py`: equal trees, equal answers.
5. **Builds.** `mod_export` into a folder (clean and again: the reuse), the flat export,
   `mod_package`: the same file set and the same bytes; the descriptor and the index equal
   except the fields that state bytes on disk; `left_out` and the warnings equal.
6. **The scene.** Open Escape; `scene_info`, `scene_stats`, `scene_tree`, `list_objects`,
   `object_info` of a sample of every class, `selection`; then `place_object`, `place_spawn`
   (a Player Start among them), `place_way`, the `object_set_*` commands, `rename_object`,
   `spawn_order`, `delete_selected`, undo and redo; `save_scene` - the saved scene equal (byte
   for byte, or chunk by chunk with the difference named); `mod_export_spawn`,
   `mod_export_xcform`, `mod_export_cut`, the bake: `.xspawn`, `spawn\custom_data\*.ltx`,
   `overlay_visuals.ltx` byte-equal, `overlay.xcform` and exported `.ogf` byte-equal or within
   the tolerance section 9 states.

## What to do with a difference

- The C++ side is right unless `docs/RUST_MIGRATION.md` section 12 says otherwise: fix the Rust
  crate that owns it, at the cause (the rules of sections 5-6 hold), and re-run that scenario.
- When the C++ side is plainly wrong (a bug the Rust side does not repeat) or the difference is
  one of the deliberate ones, the ignore table gets the field with the reason, and
  `docs/rust/STATE.md` gets a line under "Differences the differential left" - the lead decides
  which of them enter section 12. Keep that list short and exact; a long one means the Rust
  side was not fixed.
- A command that only makes sense with a window (`ui_*`, screenshots, the `*_view` commands) is
  the QA suites' business, not this script's: mark it "not compared: window" in the ledger.

## How to work

- Fully autonomous; never ask; never start agents of your own. No `cargo test` marathon here:
  after a fix run the tests of the crate you touched (`bash tools/rust/cg.sh test -p <crate>`),
  and `tools/rust/test.sh` once at the end - its default mode runs only the test binaries
  that changed since they were green.
- Never run a git command that changes the working tree or the index. Never push, publish,
  submit or scan anything; nothing leaves the machine.
- Nothing real is changed: projects are copied, the game is only read, `_projects.ini` and the
  settings come back byte for byte.

## Report

Under 400 words: scenarios and steps run, how many differed at first and how many are left,
the important fixes by name, the ignore table's size, the wall time, and what could not be
compared and why.
