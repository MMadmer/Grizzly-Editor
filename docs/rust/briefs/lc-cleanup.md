# Brief: phase 8c, second engineer - the last C++ leaves, one build, the icon

Phase 8c is split between two engineers. The other one works in the main working copy
(`D:\Games\XFined-Editor`): the MU proof of the Rust compiler, the editor compiling in process,
Play, the in-game proof. **You work in a second working copy of the same repository,
`D:\Games\XFined-Editor-cleanup`, on the branch `phase8-cleanup`**, and take what needs no
editor run: the removal of the last C++, the one build, the documents, the executable's icon,
and the rule that tests create no program files. The lead merges your branch after both are
done.

Your working copy has the tracked files only - no `rawdata\` objects, no `Bin\`, no game data:
you build and check, you do NOT run the editor, the QA suites, the corpus tests or the
compilers. Never touch `D:\Games\XFined-Editor` (the other engineer's tree), not even to read
its build output.

Read `docs/RUST_MIGRATION.md`, "Phase 8 - The compilers" and "Phase 7"; `docs/rust/briefs/
lc-final.md` (the other engineer's brief: its section 3 is now yours); `Build.ps1`;
`tools/rust/cg.sh`, `tools/rust/check.sh`.

## 1. The last C++ leaves

- Delete `Source/`, `SDK/`, `cmake/`, `CMakeLists.txt`, the CMake presets and whatever else only
  the C++ build read (`THIRD_PARTY` entries of removed code - keep the notices of what the Rust
  editor ships; find what the Rust build really links: `cargo tree`, the vendored C of
  `mlua` and `vorbis_rs`). `git ls-files` then holds no C or C++ source, header, project or
  CMake file.
- The Rust code refers to C++ places in comments (`Source/...` as the oracle of a function):
  those stay as they are - they name the tags `cpp-editor-last` / `cpp-compilers-last`, where
  the files live. Say so once in `docs/rust/CONVENTIONS.md`.
- `Build.ps1` builds the Rust editor and nothing else: the options that named the compilers
  go; it still runs under pwsh 7, puts `Grizzly.exe` and `grizzly-mcp.exe` into
  `Bin\x64\Release` (never touching anything else there), and says what it built and how long
  it took.
- Documents: `README.md`, `tools/mcp/AI_SETUP.md` and the BUILD paragraphs of `CLAUDE.md` say
  what is true now: one build, no C++ in the tree; the comparison tools
  (`tools/qa/diff_editors.py`, `tools/qa/diff_compile.py`) work from their records under
  `Intermediate/oracle`, and record new ones from the kept C++ binaries on disk -
  `Bin\x64\CppEditor\LevelEditor.exe` for the editor, `Bin\x64\CppCompilers` for the compilers
  - while those exist. `tools/qa/qa_target.py`'s C++ target points at
  `Bin\x64\CppEditor\LevelEditor.exe`. References to a deleted path in what stays: none
  (`rg`), the migration's own records (briefs, `PARITY.md`, STATE.md) and the oracle comments
  aside. The other engineer edits `tools/qa/diff_compile.py`, `tools/qa/test_play_game.py`,
  `crates/gz-lc*`, `crates/gz-levelbuild`, `crates/gz-ui-level`, `crates/gz-mcp/tools.json`:
  do not edit those.

## 2. The icon

The editor has ONE icon, `assets/grizzly-editor-icon.ico` (`.png` beside it). `Grizzly.exe`
and `grizzly-mcp.exe` carry none today: Explorer shows the system's generic one.
- Embed the `.ico` as the executable's icon resource in both (a build script in each binary
  crate; the resource compiler of the Windows SDK is on this machine with the toolchain the
  build already needs - a small build dependency such as `winresource` or `embed-resource` is
  fine; no generated binary blob is committed). `crates/grizzly/build.rs` exists already (it
  drops the static C runtime from the link): extend it.
- The editor's own window icon (`crates/gz-app/src/window.rs`, set at run time) is that same
  picture - check where it comes from and make it one source.
- Proof without a window: after `Build.ps1`, a check reads each exe's icon group resource and
  wants its largest image equal to the `.ico`'s (a small script the check calls;
  `tools/rust/check.sh` gets it as a step that runs when the release binaries exist). Put
  your additions to `check.sh` in one block of their own at the end - the other engineer
  edits that file too.

## 3. Tests create no program files

The author's antivirus flags our test binaries: a freshly built unsigned program that writes an
`.exe` into a temp folder looks like a dropper. No test in the workspace writes, copies or
renames a file with a program's extension (`.exe`, `.dll`, `.com`, `.scr`, `.bat`, `.cmd`,
`.ps1`, `.msi`), nor a file that starts with `MZ`.
- Go through every crate's tests EXCEPT `crates/gz-levelbuild` (the other engineer fixes its
  `play.rs`): `rg -n "\.exe|\.dll|\.bat|\.cmd|\.ps1|\.msi|b\"MZ" crates -g '*.rs'`, read each
  hit, change what a test creates to a harmless name with the code under test taking the name
  as a parameter where it must.
- A step of `tools/rust/check.sh` (in your block) fails on such a write inside test code; show
  in your report what it catches on the tree as you found it.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index (no add, commit, rm,
  checkout, reset, restore, clean, stash, merge): delete with the file system, read history
  with `git show <commit>:<path>`. The lead commits and merges.
- Delete only inside YOUR working copy and only what section 1 names; a recursive delete never
  goes through a link.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>` (its target folder is your
  copy's own `Intermediate\cargo`: the first build is a full one, start it early and to a
  file). Proof at the end, once: `bash tools/rust/cg.sh check --workspace --all-targets`,
  clippy and fmt as `check.sh` runs them, `pwsh -NoProfile -File .\Build.ps1` (release; its
  time), the icon check, the unit tests that need no data (`bash tools/rust/cg.sh test
  --workspace --lib`). The corpus tests, the suites and the comparisons are run by the lead in
  the main tree after the merge.
- Write your notes into a NEW file, `docs/rust/STATE-8c-cleanup.md` (not into STATE.md: the
  other engineer edits it): what was deleted (files, lines), the build, the icon's proof, the
  test sweep, and "For the plan" - what `docs/RUST_MIGRATION.md` should say now.

## Report

Under 350 words: what was deleted (files, lines) and that no C or C++ file is tracked; the
new `Build.ps1` and its time; the icon in both binaries and its proof; the test sweep (what
was found and changed, what the check catches); the documents changed; anything still open
and exactly why.
