# Brief E2 - the viewport's own camera, the kit's one pick list, the last comments that cite another program

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's first round is `docs/plan/reports/CRITIC.md`; you close its findings
**2 and 9** and the comment list of finding **6**. Read those findings there first - they name the files and
lines - then `CLAUDE.md` (the rules; sections "Editor UI", "The Level tab") and the parts of `docs/UI_DESIGN.md`
the findings point at (the viewport's input around lines 395-410, section 8).

## What to build

### A. Shift belongs to the tools, not to an old camera (finding 2)

`crates/gz-ui-viewport/src/camera.rs` still has a second camera scheme: every press made with Shift held starts a
camera move (Shift+LMB pans, Shift+RMB lifts, both turn). No contract states it, and it swallows what the design
gives Shift: Foliage "Shift erases" (`pane.rs`, `erase: b.shift`) and Decals "Shift + click moves the selected one
there" (`pane.rs`, `mark_place(.., b.shift)`), which cannot be reached with the mouse today.

- Remove that scheme whole: the claim of a Shift press in `move_start`, its flag, its end, its part of the camera's
  update, and what only it used. The camera is what `docs/UI_DESIGN.md` describes and nothing else. Check every
  other gesture the design gives Shift, Ctrl and Alt in the viewport (marquee add / remove, Alt copy, snapping)
  still reaches its code.
- `tools/qa/test_scene_tools.py`: a `viewport.gesture` with `mods="shift"` proves the foliage erase (a stroke
  with Shift removes what a stroke laid) and the decal move (Shift + click moves the selected decal there). If
  `viewport.gesture` cannot carry Shift for these, that is a bug of the command: fix it in `gz-ui-viewport`.

### B. One pick list, no dead kit functions, a paragraph that is true (finding 9)

- `crates/gz-ui-mod/src/local.rs` has `local_pick_list`, a second pick list, used by `settings.rs`. Mod Settings
  uses the kit's `PickList` (`crates/gz-ui/src/pick.rs`); the local one goes. If the kit's lacks something Mod
  Settings needs, add it to the kit's (in `pick.rs` - it is yours for this), never beside it.
- `gz_ui::form::section` and `raised_section` (`crates/gz-ui/src/form.rs`) have no caller: remove them (and
  anything else in `form.rs` nothing calls).
- `crates/gz-ui-world/src/local.rs` says the kit has no hint field while `gz_ui::form::hint_field` exists: use
  the kit's, remove the local one and the comment.
- Then look through every surface crate's `local.rs` (`crates/gz-ui-*/src/local.rs`): what is a one-line call of
  the kit is inlined at its callers and removed; what is a real widget of that surface alone stays. Report the
  list of what is left and why each is local - the architect rewrites `docs/UI_DESIGN.md` section 8 from it.

### C. Comments that only cite another program's functions (finding 6, the comment list)

The finding lists the lines: `gz-scene` (`bulk/aimap.rs`, `bulk/details.rs`, `bulk/wallmark.rs`,
`objtools/mod.rs`, `objtools/sector.rs`, `objtools/geom.rs`, `summary.rs`), `gz-ui-viewport` (`camera.rs`,
`draw.rs`, `draw_aimap.rs`), `gz-levelbuild` (`validate.rs`, `prj.rs`, `libs.rs`, `spawn.rs`), `gz-ui-level`
(`outliner_model.rs`), and `crates/gz-ui-level/src/place.rs:16` ("stream L2a's"). They name members and
functions of a program that is not this one (`m_HASH`, `BuildNodes(bFromSelectedOnly)`, `Clear(bOnlyNodes)`,
`GenObjectName(cls, buf, pref)`, `m_SM`, `SetSensitivity`, `csPlaneMove`, `Validate(bNeedOkMsg, ...)`, ...).

- Rewrite each so it says what THIS code does and why (the why is usually a file format or what the game reads);
  a comment that only cites is deleted. Where a format or an algorithm follows X-Ray's own tools and that is the
  reason, say "X-Ray's compiler" / "the X-Ray SDK" - never a function, class or member name of it, never "the
  original", never the editor this one replaced.
- Then search your crates for more of the same kind that the finding did not list (backticked `m_Name`,
  Hungarian-prefixed arguments like `bFlag` / `pPtr` / `dwCount` in a comment, `CSomething::Method`): fix them
  too. The game's own real names stay where they are the format: server entity classes (`CSE_ALifeObject`,
  `STATE_Read`), chunk ids, `.ltx` keys, script names.

## Rules

- Files that are yours: `crates/gz-ui-viewport/**`, `crates/gz-ui-mod/**`, `crates/gz-ui/src/form.rs` and
  `pick.rs`, every `crates/gz-ui-*/src/local.rs` and its callers in the same crate (NOT `gz-ui-previews` and NOT
  `gz-ui-world/src/characters.rs` / `char_form.rs` - engineer E1 has those; for `gz-ui-world`'s hint field touch
  `local.rs` and report the callers in E1's files instead of editing them), comments only in `crates/gz-scene/src/bulk/**`,
  `objtools/**`, `summary.rs`, `crates/gz-levelbuild/src/**`, `crates/gz-ui-level/src/outliner_model.rs` and
  `place.rs`, and `tools/qa/test_scene_tools.py`. Engineer E1 works at the same time on `crates/gz-ui-previews`,
  `crates/gz-ui-world/src/characters.rs`, `crates/gz-ui/src/{audit,state,dialog,card}.rs`, `crates/gz-app/src/cmds.rs`
  and three other suites; the architect on `crates/gz-scene/src/cmd*.rs` and `service.rs`,
  `crates/gz-export-level`, `tools/rust`, `tools/qa/qa.py` and the documents. Do not edit a file that is not
  yours; if you need a change there, say so in your report.
- Keep the workspace compiling: finish one change before you start the next. If a build fails in a file that is
  not yours, wait a minute and try once more (at most three times), then go on with what needs no build; never
  loop.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm): the architect commits. Do
  not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>` (e.g. `python tools/rust/limited.py 900 bash tools/rust/cg.sh build -p grizzly
  -p gz-mcp-bridge`). A run over two minutes is watched: say every two minutes how far it is. No unbounded `find` /
  `grep -r` over `Intermediate/` or `Bin/`.
- QA: `python tools/rust/limited.py 300 python tools/qa/run_all.py test_scene_tools [--only <phases>]` - only that
  suite (the others belong to the people above; two runs of one suite at once collide). A suite opens only a COPY
  of a project; never open or change anything under `D:\XFinedProjects`, never talk to port 28016, never start an
  editor outside `qa.Editor`. Every program a suite starts is a QA run that reaches no real service; do not weaken
  that.
- Product text is English, official, short; UI through the kit (`gz-ui`) only. No `unsafe`, no `static` state,
  the UI thread never waits and makes no thread. Comments only where the why is not obvious.
- The documents are the architect's: do not edit `docs/*.md` or `CLAUDE.md`. List in your report every sentence
  they must gain or change.

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy --workspace --all-targets` without a warning,
  `cargo fmt` clean (`python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints nothing),
  `python tools/rust/limited.py 60 python tools/rust/check_words.py` ok.
- The unit tests of the crates you changed green (`bash tools/rust/cg.sh test -p <crate>`).
- `test_scene_tools` green with the two new Shift checks (run it alone: a stall "beside other suites" is not
  yours, a stall when it runs alone is).
- Write `docs/plan/reports/E2.md`: what you removed and rewrote (counts per crate), what is left in each
  `local.rs` and why, the document sentences to change, what you did NOT prove, and anything you found that is not
  yours. Answer with a summary of it.
