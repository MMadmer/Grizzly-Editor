# Brief E8 - the manifest's check, what is left

You are an engineer of the Grizzly Editor 0.7 work (repository `D:\Games\XFined-Editor`, branch `grizzly-rust`, a
Rust workspace under `crates/`). The critic's fifth round is `docs/plan/reports/CRITIC.md`; you close three bullets
of its finding **6** (`minor`): "The manifest's check, what is left", "The areas' problems are not at their line",
"`screens.set`'s card against the page". Read them there first, then `docs/plan/reports/E7.md` (the engineer before
you made the reference's rows judge every value: `crates/gz-mod/src/reference.rs`, `rules.rs` - `accepts`,
`refusal`, `validate`) and `CLAUDE.md` ("Mod releases": the manifest metadata rules; "Game modes"; "Screens and
music").

## What to build

1. **The check reads every value the page lists** - the page says so ("The check reads every value this page
   lists"). Written by hand and passed in silence today:
   - `[command.<name>] cheat = maybe` (the page: `true | false`), a layer's `opacity = 7` (the page: 0 to 1),
     `api = banana`, `version =` (empty: `mod.set` refuses `''`), a section `[screen.Bad Id]` (the page: the id
     takes a-z, 0-9 and _; `screens.set` turns it into `bad_id`) and the same for `[track.<id>]` /
     `[command.<name>]` where the page gives the name a rule. Walk the page's rows: every key whose "takes" is not
     free text has a judge, every section whose name carries an id has its id judged. Errors where the editor
     refuses to save the value, warnings where a reader ignores it (E7's rule; `docs/plan/reports/E7.md`).
   - **A misspelled key of `[module]` / `[provides_mode]`** (`versoin = 2.0`, `Tags = story`): an unknown key is
     kept by contract and stays unreported - but a key that is a known key of that section in another case, or
     one edit away from one, is a warning (`mod.key`) naming the key it is near, with the fix. Nothing else of
     those sections is reported.
   - **The target and the mode go together** as the page pairs them: `target = default` goes with no `mode`,
     `existing` with a mode id, `new` with a `[provides_mode]`, `any` with `mode = *`. `target = default` with
     `mode = night`, and `target = any` with `mode = night`, pass the check and `mod.set` alike today; `target =
     any` alone answers the new project's sentence ("no target game mode ...", fix `target = default`), which is not
     this file's. Give each mismatch its own code, sentence (the two lines as written, what goes with what) and
     fix, at the line of the key that is wrong; `mod.set` refuses the same pairs with the same sentence (it already
     sets the target from the mode where the target is not given - keep that). Read the game's rule before you
     write the sentence (`D:\Games\Dead Air\DeadAir-x64\docs\dead-air\MODDING.md`, read only: what the game does
     with `mode`; the game ignores `target`) - the check must not refuse a manifest the game runs as the author
     meant it. The manifests of the corpus (the author's projects, the game's modules) stay clean: a rule that
     fails one is a finding to report, not to hide.
2. **The areas' problems are at their line.** `E401` (a layer's file) lands on the `[screens]` header or on
   `mod.ltx:1:1`; `E409` / `W410` of `[track.t1]` on 1:1; `E207` of `[start_kit]` on `[world]`. The area checks
   (`gz-screens`, `gz-world`, and whatever else contributes to the manifest's check - look at how
   `gz-asset`'s `check_manifest` gathers them) say which section and key a problem is about, and the manifest's
   check places it at that key's line (the way E7's `validate` places its own; a problem about a whole section at
   the section's header). No area re-parses the file for it: the document (`ManifestDoc`) knows the lines.
3. **`screens.set`'s card says what the page says.** `rect`: the card says "in shares of the screen", the command
   writes `rect=[0,0,0.5,0.5]` as `0 0 0.5 0.5` and the page says per cent with the default `0 0 100 100` - one
   unit, said the same in the card, the page and the reader (the reader decides: read `gz-screens` and the
   game's `MODDING.md`, "Screens and music"). `fit`: the card lists "... or none", the page does not have it -
   whichever the reader takes. `track_add file=<a picture>` converts the picture and writes a track the check
   then refuses (`E409`): a track takes a sound, and a picture is refused there in a sentence before anything is
   written or converted. Look at the other `screens.*` and `world.*` cards for the same kind of gap against the
   page.

## Tests

- Unit tests in `gz-mod` (each new rule: wrong value, its code, its line; the pairs of target and mode, both
  ways; the near-miss key and a far one that stays silent) and in `gz-screens` / `gz-world` (a problem carries
  its section and key).
- `tools/qa/test_cli.py` **phase J only** (the architect adds another phase to this file after you finish): the
  new wrong values, each named by `check` with its line; a layer's missing file at the layer's line.
- `tools/qa/test_screens.py`: `screens.set` with `rect` in the unit the card names shows what the page says;
  `track_add` of a picture is refused, nothing written.

## Rules

- Files that are yours: `crates/gz-mod/**`, `crates/gz-screens/**`, `crates/gz-world/**`, `crates/gz-asset/**`
  (only the manifest's check and how it places problems), `crates/gz-characters/**` and `crates/gz-pda/**` only
  where they hand a problem to the manifest's check, `crates/grizzly/tests/manifest_reference.rs`,
  `tools/qa/test_cli.py` (phase J only), `tools/qa/test_screens.py`, `tools/qa/test_world.py`. The architect works
  at the same time on `crates/gz-app`, `crates/gz-project`, `crates/gz-ui-previews`, `crates/gz-ui-mod`,
  `crates/gz-ui-app`, `crates/gz-base`, `crates/grizzly/src`, `crates/gz-editor`, `tools/rust`,
  `tools/qa/test_previews.py`, `tools/qa/test_project_browser.py`, `tools/qa/test_ui_shell.py` and the documents.
  Do not edit a file that is not yours; if you need a change there, say so in your report.
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
  your suites, one at a time. Never open or change anything under `D:\XFinedProjects` (the corpus test reads the
  manifests there through its environment variables - `tools/rust/test.sh` sets them; reading is all it does),
  never talk to port 28016, never weaken the QA client's network isolation, never write under the linked game or
  the game repository. Work in a scratch folder of your own under the system temp folder.
- Product text is English, official, short: a problem is a sentence for the author, never a bare code, never a
  raw system error number. Never name a function or a class of another program.
- The documents are the architect's: list in your report every sentence of `docs/*.md` / `CLAUDE.md` to change
  (the reference page's own words are yours: make them true).

## Done

- `python tools/rust/limited.py 900 bash tools/rust/cg.sh clippy -p gz-mod -p gz-screens -p gz-world -p gz-asset
  -p gz-build --all-targets` without a warning, `python tools/rust/limited.py 120 cargo fmt --all -- --check -l`
  prints none of your files, `python tools/rust/limited.py 120 python tools/rust/check_words.py` prints no line of
  yours.
- `bash tools/rust/test.sh` for your crates green with the corpus read (`python tools/rust/limited.py 1500 bash
  tools/rust/test.sh gz-mod gz-screens gz-world gz-asset grizzly` if the script takes package names - read its
  header; else `cg.sh test -p <crate>` with the corpus variables the script would set); `test_cli`, `test_screens`,
  `test_world` green.
- The critic's values (bullet one of its finding 6) through `Grizzly-Cmd.exe check` (with `-home` and a project
  under the system temp folder): each named, with its line.
- Write `docs/plan/reports/E8.md`: what you built, the codes and sentences as they read, the document sentences to
  change, what you did NOT prove. Answer with a summary of it.
