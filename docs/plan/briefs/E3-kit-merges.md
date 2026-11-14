# Brief E3 - one widget per job: the last widgets two surfaces draw each for themselves

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The rule you close (`CLAUDE.md`, "Editor UI"): **the kit (`gz-ui`) is the only way
to draw UI and has ONE widget per job**; a widget the kit lacks starts in its surface crate's `local.rs` and moves
into `gz-ui` when a second surface wants it. Engineer E2 went through every `crates/gz-ui-*/src/local.rs` and
listed what is left (`docs/plan/reports/E2.md`, the table "What is left in each `local.rs`, and why", and the two
notes under it). Most of it is a real widget of one surface. What follows is not: it exists twice, or beside a kit
widget of the same job.

Read first: `CLAUDE.md` ("Editor UI"), `docs/UI_DESIGN.md` sections 3, 4 and 8, `crates/gz-ui/src/lib.rs` (the kit's
list of widgets), and `crates/gz-ui/src/kitsheet.rs` (`-kitsheet` shows every kit widget in every state).

## What to build

1. **The panel tab strip** - `gz-ui-pda`'s `local_tab_strip` and `gz-ui-level`'s `local_tab` (with `TabResponse`,
   `local_tab_width`) draw the same thing: a 26 pt panel tab with an accent overline. Make ONE kit widget of it
   (in `gz-ui`, a new file or `toolbar.rs` / `layout.rs` where it fits), with what both need (a close cross where a
   tab can close, a drag where tabs can be dragged, the overflow each has today), use it in both, remove the two
   local ones. The pictures must not change: same sizes, same colours, same hit rectangles.
2. **The world's level picker** - `gz-ui-world`'s `local_level_picker` / `open_level_picker` is a searchable popup
   list beside the kit's `PickList` (`crates/gz-ui/src/pick.rs`). World Rules uses the kit's `PickList` (the way Mod
   Settings does since E2: `crates/gz-ui-mod/src/settings.rs`); the local one goes. If the kit's lacks something the
   level picker needs, add it to the kit's.
3. **The release windows** (`gz-ui-release`): `local_fold` becomes the kit's `Section`; `local_table_head` the kit's
   `Table` (or `list::table_headers` if `Table` is not what a static head needs - one of the kit's, not a third);
   `local_secret_row` uses the kit's `form::secret_field` (E2 added it) and what is left of the row is layout at its
   one caller. `local_code`, `local_card_title` and `cell` stay if they are the release windows' alone - say so.
4. **The world's wrappers** (`gz-ui-world/src/local.rs`): `local_section` (the kit's `Section` plus a 4 pt gap),
   `local_hint_field` (two lines over `gz_ui::form::hint_field`), `local_check` and `local_cell_check` (over the
   kit's `Check`), `local_spinner_at`, `set_section_open`: inline each at its callers (`char_form.rs`,
   `characters.rs`, `picker.rs`, `world.rs`, `portraits.rs`, ...) and remove it. What is a real widget of the world
   tabs alone (`local_number_text`, `local_dirty_dot`, `local_problems_drawer`, `local_cell`, `drawer_height`)
   stays.
5. **The two text-field wrappers** - `gz-ui-app`'s and `gz-ui-mod`'s `local_text_field` are both the kit's
   `hint_field` with one thing added (an audit name; the row's width). If the addition is one argument, the kit's
   `hint_field` takes it and both wrappers go; if they truly differ, they stay and you say why.
6. **One drop board** - `gz-ui-pda`'s `drop.rs` (`begin_drops`, `local_file_drop_zone`) is a second copy of the
   kit's drop board (`crates/gz-ui/src/drop.rs`, `Zone`): with two boards a drop can be taken by one and not the
   other. Screens & Music takes its drops through the kit's `Zone` (the Texture Library does since engineer E1:
   `crates/gz-ui-previews/src/library/textures.rs`); `gz-ui-pda`'s copy goes. Inner zones are still asked first.
7. **Busy marks drawn by hand** - `crates/gz-ui-release/src/submit.rs` (`wide_status(ui, Tone::Muted,
   "Checking...")`, around line 994) and `publish.rs` ("Contacting GitHub...", around line 814) are busy lines of
   cards drawn by hand: they call the kit's `card::card_busy` / `state::busy_line`, so `ui.audit` reports them as
   `loading`. Look through the surface crates for any other hand-drawn spinner-and-text and do the same.
8. `-kitsheet` shows every widget you add to the kit, in every state it has (`crates/gz-ui/src/kitsheet.rs`), and
   the kit's own tests (`crates/gz-ui/src/tests.rs`, `testkit.rs`) hold what can be held without a window (the tab
   strip's widths and overflow, the pick list's new behaviour).

Do NOT redesign anything: the surfaces look and behave as they do. This is the same picture drawn by one widget.

## Rules

- Files that are yours: `crates/gz-ui/src/**` (the kit), every `crates/gz-ui-*/src/local.rs` and their callers in
  the same crates (`gz-ui-pda`'s `drop.rs` and its users, `gz-ui-release`'s windows), `tools/qa/test_world.py`,
  `test_pda.py`, `test_screens.py`, `test_mod_publish.py` only where a check must follow a widget's id. Nobody else edits code while you work; the architect edits documents only.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm): the architect commits. Do
  not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>` (e.g. `python tools/rust/limited.py 900 bash tools/rust/cg.sh build -p grizzly
  -p gz-mcp-bridge`). A run over two minutes is watched: say every two minutes how far it is. No unbounded `find` /
  `grep -r` over `Intermediate/` or `Bin/`.
- QA: `python tools/rust/limited.py 400 python tools/qa/run_all.py <suite ...>` on COPIES of projects only (the
  suites do that); never open or change anything under `D:\XFinedProjects`, never talk to port 28016, never start
  an editor outside `qa.Editor`. Every program a suite starts is a QA run that reaches no real service; do not
  weaken that.
- The proof that nothing moved is pictures: before you change anything, take the tour's pictures
  (`python tools/rust/limited.py 300 python tools/qa/run_all.py test_tour --extra --shots
  "D:/Games/XFined-Editor/Intermediate/plan/e3_before"`), and after your work the same into
  `Intermediate/plan/e3_after`; compare the tabs you touched (Level, PDA, World Rules, Characters, Mod Settings,
  Releases, the Package / Publish / Submit dialogs) by eye with the Read tool, and say in your report what differs.
- Product text is English, official, short; theme tokens and points only. No `unsafe`, no `static` state, the UI
  thread never waits and makes no thread. Comments only where the why is not obvious; never cite another program.
- The documents are the architect's: do not edit `docs/*.md` or `CLAUDE.md`. Your report gives the architect the
  final list for `docs/UI_DESIGN.md` section 8: every widget still in a `local.rs`, by crate, with the one reason
  it is local, and the kit's new widgets for section 4.

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy --workspace --all-targets` without a warning,
  `python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints nothing,
  `python tools/rust/limited.py 120 python tools/rust/check_words.py` prints no line of yours.
- `bash tools/rust/cg.sh test -p gz-ui` and the tests of every crate you changed green.
- `python tools/rust/limited.py 500 python tools/qa/run_all.py test_tour test_ui_shell test_world test_pda
  test_screens test_characters test_mod_publish test_mod_catalog test_level_commands test_scene_tools` green (a suite that fails
  on a stall alone beside the others is run again alone by the runner; that is not yours).
- Write `docs/plan/reports/E3.md`: what moved into the kit, what was removed, what stays local and why (the final
  table), what the before / after pictures show, what you did NOT prove. Answer with a summary of it.
