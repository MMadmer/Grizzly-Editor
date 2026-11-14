# Brief Q2 - the code speaks on its own terms

Read first: `docs/plan/RULES.md`, `CLAUDE.md` ("Code and text", "Editor UI"), `docs/ARCHITECTURE.md` section 7.

The editor is its own program. Its code still explains itself by another program in about fifteen hundred
places: comments that name a C++ class or function, messages in the X-Ray SDK's voice, levels written as marks
in a message's text, the names of commands that no longer exist. This work removes all of it and leaves a check
that keeps it out. **It changes no behaviour** except where part 2 says so.

You own: comments, doc comments, messages, port-shaped names and test modules in every crate under `crates/`,
EXCEPT these files, which engineer Q1 owns and sweeps itself: `crates/gz-editor/src/registry.rs`, `help.rs`,
`tests.rs`, `lib.rs`, `crates/gz-mcp/src/lib.rs`, `crates/gz-mcp-bridge/src/main.rs`. You also own
`tools/rust/check_words.py` with its lists `tools/rust/words_allowed.txt` and `tools/rust/removed_commands.txt`.
Nothing else under `tools/` is yours (Q1 rewrites the suites at the same time), and no document
(`docs/*.md`, `README.md`, `CLAUDE.md`, `tools/mcp/AI_SETUP.md`: the architect's - list in your report what the
code makes untrue in them).

The architect is working in `crates/gz-win/src/lib.rs`, `crates/gz-app/src/stalls.rs` and
`crates/gz-app/src/window.rs` while you start: sweep those three files last, and read each again right before
you touch it. Your QA port, if you run an editor at all, is 28993 (`GRIZZLY_QA_PORT`).

Order of work, because engineer Q1 needs a tree that compiles: do part 2 (the typed findings) FIRST and
finish it crate by crate - `bash tools/rust/cg.sh check --workspace` clean before you go on - then the sweep,
which cannot break a build. Never leave the tree not compiling for more than a few minutes.

## 1. The sweep

Sweep every crate (but Q1's six files):

- **Comments and doc comments** that explain something by the C++ editor, a `Source/...` path, a C++ class or
  function name (`EScene::`, `TUI_`, `CUI...`, `NqUtil::`, `EditorCharacters::` ...), "as the C++ did", "byte
  identical to the C++", "the oracle", "parity", a tag name: rewrite each to say what the code does and why, or
  delete it when it says nothing else. A fact about a FILE FORMAT that is X-Ray's (`.ogf`, `.object`, LTX,
  `level.geom`, the SDK scene the importer reads) stays, stated plainly.
- **Names** in code that only make sense as a port (`cpp_*`, `Cpp*`, `*_cpp`, `COMMAND_*` ids if any survived):
  rename, with their users.
- **Messages** an author can see that carry the SDK's voice (`"TUI_ControlObjectAdd:: Can't load reference
  object."`, `"*ERROR: Can't find any Spawn Object."`, `"!Level doesn't contain HOM objects!"`): sentences in the
  editor's tone ("The level has no spawn point.").

## 2. A message's level is a value (do this first)

- **A message's level is a value, never a mark in its text.** Today the level build's checks and exports collect
  `Vec<String>` whose lines start with `!`, `~`, `*` or `*ERROR:` and their readers take the level from the first
  character (`gz-levelbuild`'s `pipeline.rs` `line_problems` and the two loops that log lines, `gz-export-level`'s
  `cmd.rs`). Make them typed: one small type in `gz-base` beside `Problem` - `Finding { severity, text }` with
  `Finding::error / warning / info` - produced by `gz-levelbuild` (`validate.rs`, `files.rs`, `spawn.rs`, `graph.rs`,
  `prj.rs`'s `BuildError` texts), `gz-scene` (`objtools/sector.rs`, `summary.rs`) and `gz-export-level` (`gather.rs`,
  `overlay.rs`, `spawn.rs`), and read by level, not by character, where they become log lines and `level.problems`
  rows. The level compiler's own lines (`gz-lc`, `gz-lc-light`) keep their text; where they enter the editor (the
  compile step's log and problems) a mark is read ONCE into a level and dropped from the text, so no row of
  `level.problems`, no line of the status bar and no line of the Output Log shows `!`, `~`, `*` or `***`. Also
  `gz-ui-level`'s `detail_objects.rs` (two `"* Colour indices ..."` lines) and `gz-ui`'s `file_dialog.rs` (`"~ the
  file dialog did not open"`). The Output Log's reader of marks (`gz-ui-app`'s `log.rs`) then goes: nothing
  writes one. `gz_log::marked` and `Level::mark` go with it unless the log FILE's format needs them (then only
  the file writer uses them). A unit test per crate holds one finding of each level, and one in `gz-levelbuild`
  that no problem's text starts with a mark (the suites are Q1's: say in your report what a suite should assert).

## 3. Tests, names of removed commands, the check

- **Tests** whose point was parity with the C++ editor, its writer's bytes or its answers: delete. Tests that
  duplicate another test or assert a constant: delete. Count the tests before and after and say what went, by
  crate, in your report - the aim is fewer tests that each catch something real, not a number.
- **Removed command names** (`docs/plan/old_command_names.txt`, one per line: the commands 0.7 replaced). A
  mention of one in a string literal, a message, a comment or a doc comment anywhere in `crates/` is stale (Q1
  does the same in `tools/`) - say the command that does it now (`help` knows), or what the code does. A Rust identifier
  that merely has the same spelling (`fn ui_tab`, a field `view_mode`) is no mention. The word check below fails
  on one of these names inside a string literal or a comment; keep the list it reads beside it
  (`tools/rust/removed_commands.txt` - `docs/plan` does not outlive this work).
- The forbidden-word check (`tools/rust/check_words.py`, exit 1 on a hit): `C++`, `cpp`, `oracle`, `parity`, `Source/`, `SDK 0.7`, `LevelEditor`,
  `xrLC` outside a short allow-list file of places where a format's or a game program's real name is the fact
  (`tools/rust/words_allowed.txt`, each entry with its reason).

How to find the comments: `grep -rnE "C\+\+|\bcpp\b|oracle|parity|Source/|LevelEditor|xrLC|TUI_|EScene|CUI[A-Z]" crates`
(about 550 lines) and the identifiers of the old code quoted in comments - `(\`CamelCase\`)` and `/// \`CamelCase\`:`
where the name is no item of this workspace (`grep -rnE "\(\`[A-Z][A-Za-z0-9]+(::[A-Za-z0-9_~]+)*\`\)" crates`,
about a thousand lines, many of them real Rust names that stay). A comment that only named the old function is
deleted; one that explains WHY keeps the why in plain words. Go crate by crate, the largest first (`gz-scene`,
`gz-nq`, `gz-lc-light`, `gz-ui-graph`, `gz-levelbuild`, `gz-fmt-ogf`, `gz-lc`), and keep a line of each crate's
count before and after for the report.

## Proof

`bash tools/rust/cg.sh check --workspace --all-targets` and `clippy` clean, `cargo fmt --all` applied,
`bash tools/rust/test.sh` green (each under `python tools/rust/limited.py`); `python tools/rust/check_words.py`
passing, with its allow-list in the report, each entry with its reason; the counts per crate before and after;
the tests by crate before and after with what went and why. `git diff --stat` of the sweep shows comments,
strings an author sees, names and deleted tests - and the typed findings of part 2.

## Done means

No mention of the C++ editor, an oracle or parity, no identifier of the old code and no removed command's name in
the code outside the allow-list; no message whose level is a character of its text; a check that fails on any
of it.
