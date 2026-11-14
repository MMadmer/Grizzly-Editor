# Brief E6 - the module manifest can be learned from the editor

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's third round is `docs/plan/reports/CRITIC.md`; you close its finding
**3** and two of its minors. Read that finding there first, then `CLAUDE.md` ("Assets and projects", "Game modes",
"Commands") and `docs/ASSETS.md`.

## The finding

`mod.ltx` is the one file every module must have, and it is the one an agent cannot learn from the editor:

- A new project's first check (`project.create`, then `Grizzly-Cmd.exe check`) answers one error: "no target game
  mode: open Mod > Mod Settings, Game Mode, and pick one ..." (`crates/gz-mod/src/rules.rs` around line 164). It
  names no key and no command: the fix it offers is a menu.
- `mod.set`'s card lists `mode, target` without their values; the values (`default`, `existing`, `new`, `any`) are
  said only by the refusal of a wrong one (`crates/gz-mod/src/cmd.rs` around line 339).
- `asset.kinds` lists eight kinds, `reference` knows seven: `Grizzly-Cmd.exe reference "module manifest"` answers
  "no kind of asset". That kind's reference is "the game's docs/dead-air/MODDING.md", a file of another
  repository. The project's `AGENTS.md` (`crates/gz-project/src/guide.rs`) promises `reference <kind>`.

## What to build

1. **The reference of the manifest.** `asset.reference kind=manifest` (and the commandlet's `reference manifest`;
   accept `mod.ltx` and `module manifest` as the same kind's names if the kind table makes that cheap) answers the
   manifest's format as text, the way the other seven kinds answer theirs (see how `gz-nq`, `gz-pda`,
   `gz-characters` and `gz-game`'s `project_kind.rs` give a `reference` to `gz-asset`'s kind table): every section
   and key this editor reads or writes - `[module]` (id, name, author, description, version, target, mode,
   thumbnail, tags, disable_game_updates, ...), `[provides_mode]`, `[requires]` / load order / conflicts,
   `[vfs]`, `[exclude]`, `[vars]`, `[world]`, `[start_kit]`, `[pda]`, `[screens]` / `[screen.<id>]` /
   `[track.<id>]`, `[update]`, the commands - with each key's values, default and one line of what it means, and a
   short complete example. It is generated from what `gz-mod` already holds (`model.rs`, `rules.rs`, the
   validators) wherever the code has the list; where the meaning is the game's, say what the game does in one line.
   With a game linked it adds what only the game knows (the modes that exist, by the names a player reads) the way
   the other references do; without one it still answers. A unit test holds the reference to the model: every
   section and key `gz-mod` reads is in the text (so a key added later without its words fails the test).
   `asset.kinds` names that reference for the manifest (it may still point at the game's document as the
   normative text).
2. **The first problem names its fix in the file's words.** The "no target game mode" problem (and any other
   problem of `gz-mod`'s rules whose sentence sends the author to a menu) says what to write: the key and its
   values in `mod.ltx`, and the command (`mod.set set={...}`), then - last - where it is in the window. Problems are
   positioned: file, line, column where a line exists; a missing key points at its section.
3. **`mod.set`'s card says the values.** The doc comment of every field (and of the keys `set` takes) names its
   values and what each means; `help mod.set` must be enough to set a target and a mode without a refusal first.
   Look at the other `mod.*` cards for the same gap.
4. **`AGENTS.md`** (the guide a new project gets, `crates/gz-project/src/guide.rs`): its three commands are true
   for the manifest too - it names `reference manifest` beside the other kinds, and the one line an agent needs
   first (a module targets one mode: set it).
5. **Two minors of the critic** in files nobody else has now:
   - `crates/gz-ui-previews/src/library/sounds.rs` around line 506: the toast "'Max AI Dist' <= 'Max Dist'. 'Max AI
     Dist' will be clamped." speaks in labels that no longer exist; it is a sentence in the rows' words (Max AI
     Distance, Max Distance) - or it goes, if the row's own tip says it already.
   - Three comments cite another program's calls: `crates/gz-ui-previews/src/library/lanim.rs` around lines 241
     and 247 (`LALib.Reload`, `LALib.Save`) and `crates/gz-scene/src/library.rs` around line 238
     (`FS.file_list($groups$)`): each says what THIS code does, or goes.

## Tests

- Unit tests in `gz-mod` (the reference against the model; the problem's sentence names the key and the command)
  and in `grizzly` if `every_card_is_worth_reading` needs a word.
- `tools/qa/test_assets.py`: `asset.reference kind=manifest` answers the format (a few keys and the example are in
  it); `asset.kinds` lists eight kinds and every one of them has a reference `asset.reference` answers.
- `tools/qa/test_cli.py`: one phase - a project made from nothing (`project.create` in a temp folder, the way the
  critic did: "`project.create`, `file.create`, `mod.set`, `check`, `reference`, `mod.build target=folder`"), its
  first `check` names the key to set; `mod.set` sets the target from what `help mod.set` says; the second `check`
  is clean (exit 0); `reference manifest` prints the document. No game is needed for any of it; if `check` needs
  one for a mode, use `target = default`.

## Rules

- Files that are yours: `crates/gz-mod/**`, `crates/gz-asset/**`, `crates/gz-project/src/guide.rs`,
  `crates/gz-build/**` only if `mod.info` / `mod.build` must follow, the two files of `gz-ui-previews` and the one
  of `gz-scene` named in item 5 (those lines only), `tools/qa/test_assets.py`, `tools/qa/test_cli.py`. The
  architect works at the same time on `crates/gz-app`, `crates/gz-ui-level`, `crates/gz-ui-pda`,
  `crates/gz-ui-previews/src/library/textures.rs`, `crates/gz-project/src/{cmd,games}.rs`, `crates/gz-vfs`,
  `tools/rust`, `tools/qa/test_scene_tools.py`, `tools/qa/test_previews.py` and the documents. Do not edit a file
  that is not yours; if you need a change there, say so in your report.
- Keep the workspace compiling: finish one change before you start the next. If a build fails in a file that is
  not yours, wait a minute and try once more (at most three times), then go on with what needs no build.
- No git command that changes anything (no add, commit, checkout, stash, reset, rm, mv): the architect commits.
  Do not spawn sub-agents.
- Every program runs under `python tools/rust/limited.py <seconds> ...`. Every cargo command goes through
  `bash tools/rust/cg.sh <cargo args>`. Format only your own files (`rustfmt --edition 2024 <file>`), never
  `cargo fmt --all` without `--check`. A run over two minutes is watched: say every two minutes how far it is.
  No unbounded `find` / `grep -r` over `Intermediate/` or `Bin/`.
- QA only through `python tools/rust/limited.py 300 python tools/qa/run_all.py <suite> [--only <phases>]`, your
  two suites, one at a time, on copies and temp folders. Never open or change anything under `D:\XFinedProjects`,
  never talk to port 28016, never weaken the QA client's network isolation.
- Product text is English, official, short. The game's own names (mode ids, section names of `mod.ltx`) are the
  format and stay; never name a function or a class of another program. The document `mod.ltx`'s meaning lives in
  is the game repository's `docs/dead-air/MODDING.md`: do not copy it, describe the keys in your own words, and do
  not invent a key the code does not read.
- The documents are the architect's: list in your report every sentence of `docs/*.md` / `CLAUDE.md` to change.

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy --workspace --all-targets` without a warning in
  your crates, `python tools/rust/limited.py 120 cargo fmt --all -- --check -l` prints none of your files,
  `python tools/rust/limited.py 120 python tools/rust/check_words.py` prints no line of yours.
- `bash tools/rust/cg.sh test -p gz-mod -p gz-asset -p gz-project -p grizzly` green; `test_assets`, `test_cli`
  green.
- `Intermediate/cargo/debug/Grizzly-Cmd.exe reference manifest` prints the document; `help mod.set` names the
  values.
- Write `docs/plan/reports/E6.md`: what you built, the reference's outline, the sentences as they read now, the
  document sentences to change, what you did NOT prove. Answer with a summary of it.
