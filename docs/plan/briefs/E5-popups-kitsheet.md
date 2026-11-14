# Brief E5 - what no command could bring into view, and the kit's proofs

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's second round is `docs/plan/reports/CRITIC.md`; you close the
`minor` findings named below (its section 5). Read that section first - it gives the files and lines - then
`CLAUDE.md` ("Editor UI", "Build and verification": "A picture is ONE call and always current ... anything that
must be looked at from outside needs a command into view") and `docs/UI_DESIGN.md` sections 4 and 7.

## What to build

1. **A count that is not known yet does not read 0** (`crates/gz-ui-level/src/place.rs` around 502-518). A Place
   category whose list is still being read shows 0 in its header, and its busy note only when the fold is open,
   so `ui.audit` reports no `loading` and the tour pictures "Objects 0" seconds before the dialogs say 8844. A
   category that is being read shows no count and the kit's busy mark in its header (a spinner where the count
   stands - `gz_ui::state::spinner_at` / `busy_line`, whichever the header's row takes), reported to an audited
   frame as `loading` whether the fold is open or not; the count appears when the list is read.
2. **`loading` has a control** (`crates/gz-ui/src/tests.rs`, with `testkit.rs`): a kit test in which an audited
   frame that shows a `state::busy_line`, a dialog `Footer::busy` and a `card::card_busy` gives one
   `AuditEntry::Loading` each, and a frame without them gives none. And in `tools/qa/test_ui_shell.py` (or the
   suite that already drives a slow job): one check that `ui.audit` answers a non-empty `loading` while something
   is busy and an empty one after - so `test_tour`'s wait is proven to wait for something.
3. **The popups a command can bring into view.** World Rules' "Add Level..." list
   (`crates/gz-ui-world/src/world.rs` around 917) and Mod Settings' "Add Module..." and Function lists
   (`crates/gz-ui-mod/src/settings.rs` around 1091 and 1330) are in no picture and no suite: nothing reaches them.
   `characters.view show=<field>` is the model (`gz_ui::menu::take_popup_request` / the kit's popup request: see
   `crates/gz-ui-world/src/picker.rs` `opened` and how `characters.view` asks). `world.view` and `mod.view` gain
   the same `show=` for these lists (documented parameter values, refused with the real ones otherwise), and
   `ui.contract`'s popups name them (`crates/gz-app/src/contract.rs` is the ONE place for a constant QA pins).
   `tools/qa/test_world.py` and the suite that drives Mod Settings (find it: `mod.view` in `tools/qa`) each open
   their lists with `show=`, check `ui.list` shows the popup with rows, and close it.
4. **No frame makes a thread** (`crates/gz-ui/src/file_dialog.rs` around line 108): the file dialog's thread is
   made on the UI thread in the frame of the click. It is started with `Executors::thread` (`gz-task`; a pool
   thread makes the thread - see `crates/gz-ui-previews/src/audio.rs` and `image_pass.rs` for the two uses). The
   kit's file dialog function must reach an executor: take it the way its callers can give it (they all have the
   editor), without a `static`.
5. **The kit sheet shows what the kit gained** (`crates/gz-ui/src/kitsheet.rs`; `docs/UI_DESIGN.md` section 4:
   "a sheet of every widget below in every state"): `form::secret_field` (empty, typed), `form::row_field` and
   `hint_field` (empty with its hint, typed), `list::table_headers_ruled`, `state::error_state`,
   `state::busy_line`, `card::card_busy`. Then compare the sheet with the table of section 4 row by row and add
   whatever else the table names and the sheet lacks; say in your report what you added.
6. **Crate docs that name widgets which moved into the kit**: `crates/gz-ui-content/src/lib.rs:35`
   (`local_checker`), `crates/gz-ui-level/src/lib.rs:38` (`local_tab`) - and read every surface crate's `lib.rs`
   doc against its `local.rs` as it is now (NOT `gz-ui-previews`: engineer E4 has it).
7. **What section 8 must list** - you report, the architect writes: for `gz-ui-viewport` (`local_axis_gizmo`,
   `local_stats_pill`), `gz-ui-level` (`local_number_row`) and any other crate, every `local_*` widget that lives
   OUTSIDE `local.rs` (`grep -rn "fn local_" crates/gz-ui-*/src`), with the one reason it is that surface's alone.
   If two surfaces draw the same thing, say so - do not merge it yourself unless it is a one-line call of the kit.

## Rules

- Files that are yours: `crates/gz-ui/src/{tests,testkit,kitsheet,file_dialog}.rs` and the kit files a kit sheet
  entry needs touched (NOT `pick.rs`, `backdrop.rs`), `crates/gz-ui-level/**`, `crates/gz-ui-world/**`,
  `crates/gz-ui-mod/**`, `crates/gz-app/src/contract.rs`, the callers of the kit's file dialog function in any
  surface crate EXCEPT `gz-ui-previews` (list E4's callers in your report if the signature changes - and then
  keep the old signature working for them: an added function, not a changed one), `tools/qa/test_world.py`,
  `test_ui_shell.py` and the suite that drives Mod Settings. Engineer E4 works at the same time on
  `crates/gz-ui-previews/**`, `tools/qa/test_previews.py`, `test_tour.py`, `test_cli.py`; the architect on
  comments in `gz-scene`, `gz-lc*`, `gz-fmt-ogf`, `gz-entities`, `gz-particles`, `gz-levelbuild`, on `tools/rust`
  and the documents. Do not edit a file that is not yours.
- Keep the workspace compiling: finish one change before you start the next. If a build fails in a file that is
  not yours, wait a minute and try once more (at most three times), then go on with what needs no build.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm, mv): the architect commits.
  Do not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>`. Format only your own files (`rustfmt --edition 2024 <file>`), never
  `cargo fmt --all` without `--check`. A run over two minutes is watched: say every two minutes how far it is.
  No unbounded `find` / `grep -r` over `Intermediate/` or `Bin/`.
- QA only through `python tools/rust/limited.py 300 python tools/qa/run_all.py <suite> [--only <phases>]`, your
  suites, one at a time, on copies of projects. Never open or change anything under `D:\XFinedProjects`, never
  talk to port 28016, never start an editor outside `qa.Editor`, never weaken the QA client's network isolation.
- Product text is English, official, short; UI through the kit only: theme tokens, points. No `unsafe`, no
  `static` state; the UI thread never waits and makes no thread.
- The documents are the architect's: list in your report every sentence of `docs/*.md` / `CLAUDE.md` to change
  (`world.view` / `mod.view`'s new parameter, the popups of `ui.contract`, the kit sheet).

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy --workspace --all-targets` without a warning in
  your crates, `python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints none of your files,
  `python tools/rust/limited.py 120 python tools/rust/check_words.py` prints no line of yours.
- `bash tools/rust/cg.sh test -p gz-ui -p gz-ui-level -p gz-ui-world -p gz-ui-mod -p gz-app` green; your suites
  green, each run alone.
- A picture of each popup you made reachable, taken with `shot.window` through its suite's `--shots` (or
  `qa.shot`) into `Intermediate/plan/e5/`, and looked at by you with the Read tool.
- Write `docs/plan/reports/E5.md`: what you built, the list for section 8 (item 7), the document sentences to
  change, what you did NOT prove. Answer with a summary of it.
