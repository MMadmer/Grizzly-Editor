# Brief E7 - the manifest's check reads the values

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's fourth round is `docs/plan/reports/CRITIC.md`; you close its finding
**2** and two of its minors. Read that finding there first, then `CLAUDE.md` ("Assets and projects", "Game modes",
"Mod releases": the manifest metadata rules) and `docs/plan/reports/E6.md` (the engineer before you built the
manifest's reference, `crates/gz-mod/src/reference.rs`, and the first problem's sentence).

## The finding

`mod.ltx` written by hand with `version = v1.2-beta`, `website = http://evil.example/x`, `tags = banana, Story`,
`target = sideways`, `mode = Not A Mode!`, `[update] github = not a repo`, `[vars] bad-name = 1`, `[world]
bogus_rule = true`, `mutants = maybe`, `[screens] loading_screens = sometimes` passes `Grizzly-Cmd.exe check` (exit
0, no problem) and `mod.build target=folder` builds it without a warning. `gz_mod::validate`
(`crates/gz-mod/src/rules.rs`, about line 155) checks the id, the missing target and the declared modes, nothing
else; `validate_version`, `validate_website`, `validate_github_repo`, `validate_mode_id`, `valid_global_name` and
the tag vocabulary are called by `mod.set` (`cmd.rs`), the Mod Settings form and the packaging only. The reference
page tells an agent "edit the file with your own tools ... check it with asset.check path=mod.ltx" and "any other
word is refused" - on the file's path nothing is.

## What to build

1. **The manifest's check runs the value rules the commands already hold**, each problem positioned (file, line,
   column of the key's line; the check places `mod.target` and the `[pda]` problem already - follow that), with a
   code of its own, a sentence in the file's words (the key, what it takes, what was written) and a `fix` where it
   is mechanical. Errors, because the editor refuses to save them: the version, the website (the allow-list), a
   tag outside the vocabulary, a `target` that is none of the four values (also when a mode is written - today an
   unknown target passes as soon as `mode` is set), a mode id that breaks the id rule, `[update] github` that is no
   `owner/repo`, a global's name (`[vars]`) that breaks its rule, and whatever other value rule `mod.set` or the
   form enforces that `validate` skips (look for every `validate_*` / `valid_*` of the crate and of the callers
   named above). Warnings: a key of `[world]`, `[start_kit]`, `[screens]`, `[screen.<id>]`, `[track.<id>]` that no
   reader takes, and a value none of them reads as written (`mutants = maybe`) - the readers ignore those, so the
   module still builds, but the author is told. Take the known keys and values from where the code already has
   them: the readers (`gz-world`, `gz-screens`) if `gz-mod` can see them, else the table of `reference.rs`, which
   `grizzly`'s `manifest_reference` test already holds to the readers - one list, never a second one typed by hand.
   An unknown key of a section the editor keeps as written (`[budget]`, `[redirects]`, an unknown section) is NOT
   a problem: unknown keys are kept, by contract.
2. **One list of rules.** `mod.set`, the form and the check call the same functions: no rule lives in two places.
   If a rule today is a sentence inside `mod.set`'s handler, move it into `rules.rs` and call it from both.
3. **The build follows**: `mod.build` / `mod.package` refuse on the errors (they go through the same `validate`,
   check that they do) and report the warnings; `mod.info` lists them.
4. **Two minors of the critic**:
   - `crates/gz-mod/src/model.rs` about line 102 and `rules.rs` about line 19: the comments say an empty `mode` is
     "all modes"; the game and the reference say the ordinary game only (the code follows the game). Make the
     comments say what the code does.
   - `crates/gz-asset/src/check.rs`, `walk_disk` (about lines 51-65): `check` walks the editor's own folders. A
     quest copied to `temp\release_stage_4242\modules\<id>\quests\` (what a crashed packaging leaves) makes `check`
     exit 1 ("E002 ... used by another quest of this project") while `mod.build` builds. The walk skips what the
     project's rules call tooling and editor-only (`gz-project`'s `rules.rs`: `is_tooling`, `is_editor_only` -
     `temp\`, `releases\`, the scratch folders) and what a build leaves out is still checked (an excluded quest is
     the author's file). Use the project's rules, do not copy the list; if the layering forbids the call
     (`docs/ARCHITECTURE.md` section 8), say so in your report and take the smallest honest way.

## Tests

- Unit tests in `gz-mod`: one wrong value per rule gives its problem with its line and code; a correct manifest of
  every form (`target = default`; `existing` with a mode; `new` with `[provides_mode]`; `any` with `mode = *`)
  gives none; the real manifests of the corpus stay clean (if a corpus test over the author's projects exists in
  the crate or in `grizzly`, run it: a rule that fails a shipped project is a finding to report, not to hide).
- `tools/qa/test_cli.py` phase J: one wrong value per rule, written into the project's `mod.ltx` by the test,
  `check` exits 1 naming each with its line; put right, exit 0. `[world] bogus_rule` is a warning: exit 0 with the
  warning listed.
- `tools/qa/test_assets.py` (or `test_cli.py`): a quest under `temp\release_stage_1\...` does not fail `check`.

## Rules

- Files that are yours: `crates/gz-mod/**`, `crates/gz-asset/**`, `crates/gz-build/**` only if `mod.info` /
  `mod.build` must follow, `crates/gz-ui-mod/src/form.rs` (or wherever the Mod Settings form calls a rule) only to
  call the moved rule, `crates/grizzly/tests/manifest_reference.rs`, `tools/qa/test_cli.py`,
  `tools/qa/test_assets.py`. The architect works at the same time on `crates/gz-app`, `crates/gz-project`,
  `crates/gz-ui-level`, `crates/gz-ui-previews`, `crates/gz-ui-world`, `crates/gz-ui-graph`,
  `crates/gz-ui-mod/src/settings.rs`, `tools/rust`, `tools/qa/test_scene_tools.py`, `tools/qa/test_previews.py`,
  `tools/qa/test_asset_icons.py`, `tools/qa/test_ui_shell.py` and the documents. Do not edit a file that is not
  yours; if you need a change there, say so in your report.
- Keep the workspace compiling: finish one change before you start the next. If a build fails in a file that is
  not yours, wait a minute and try once more (at most three times), then go on with what needs no build.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm, mv): the architect commits.
  Do not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>`. Format only your own files (`rustfmt --edition 2024 <file>`), never
  `cargo fmt --all` without `--check`. A run over two minutes is watched: say every two minutes how far it is.
  No unbounded `find` / `grep -r` over `Intermediate/` or `Bin/`.
- Unit tests of a crate: `python tools/rust/limited.py 900 bash tools/rust/cg.sh test -p <crate>`. NEVER run
  `Intermediate/cargo/debug/Grizzly.exe` or `Grizzly-Cmd.exe` yourself without `-home <a new folder under the
  system temp folder>` and a project under the system temp folder: with no arguments it is the author's own
  editor. QA only through `python tools/rust/limited.py 300 python tools/qa/run_all.py <suite> [--only <phases>]`,
  your two suites, one at a time. Never open or change anything under `D:\XFinedProjects`, never talk to port
  28016, never weaken the QA client's network isolation, never write under the linked game.
- Product text is English, official, short: a problem is a sentence for the author, never a bare code. Never name
  a function or a class of another program.
- The documents are the architect's: list in your report every sentence of `docs/*.md` / `CLAUDE.md` to change
  (the reference page's own words - "any other word is refused" - are yours: make them true or exact).

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy -p gz-mod -p gz-asset -p gz-build -p gz-ui-mod
  --all-targets` without a warning, `python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints none of
  your files, `python tools/rust/limited.py 120 python tools/rust/check_words.py` prints no line of yours.
- `bash tools/rust/cg.sh test -p gz-mod -p gz-asset -p grizzly` green; `test_assets`, `test_cli` green.
- The critic's manifest (the ten wrong values above) through `Grizzly-Cmd.exe check` (with `-home` and a project
  under the system temp folder): every value named, with its line.
- Write `docs/plan/reports/E7.md`: what you built, the codes and sentences as they read, the document sentences to
  change, what you did NOT prove. Answer with a summary of it.
