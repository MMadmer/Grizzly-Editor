# Brief: the cutover (phase 7)

The Rust editor has passed its gate: the four entry points are green (`tools/rust/check.sh`;
`tools/rust/test.sh --all`, 1099 tests on the whole corpus; `tools/qa/diff_editors.py --all`, 755
steps and 746 written files equal on both editors; `tools/qa/run_all.py --with-game`, 23 suites,
the game loading a level Play built), the parity ledger has no `missing` row, an independent
reviewer's findings are closed. This pass makes the Rust editor THE editor of the repository
(`docs/RUST_MIGRATION.md`, phase 7). You are alone in the tree. The last commit that holds the
C++ editor is tagged `cpp-editor-last`.

Read `docs/RUST_MIGRATION.md` sections 4 (scope), 11 (phase 7 and 8), 12 and 13; `CLAUDE.md`
whole (you rewrite it); `tools/mcp/AI_SETUP.md`; `README.md`; `Build.ps1`; `docs/rust/STATE.md`
sections "For the plan" and "Play ends in the game".

## 1. What leaves the repository

- **The C++ editor and every engine library only it used** are deleted from `Source/`: by
  section 4 that is `LevelEditor`, `XrECore`, `XrEUI`, `XrEProps`, `XrETools`, the entity
  factories, the engine libraries the editor linked, and everything section 4 lists as dropped
  (the Clear Sky and Shadow of Chernobyl flavours, `ActorEditor`, `ParticleEditor`,
  `ShaderEditor`, the standalone `XrAI`, GameSpy, `XrNetServer`).
- **What the level compilers need stays until phase 8**: `XrLC`, `XrLCLight`, `xrDO_Light`
  and exactly the libraries, headers, property sheets and third-party code they build from.
  Find that closure from the build files (`Source/Engine.sln`, the `.vcxproj` / CMake files,
  the include paths) - do not guess from names. After the deletion the three compilers still
  BUILD from the repository (`Build.ps1`, or the solution's compiler projects alone); a build
  that fails means something they need was deleted: put it back from the tag
  (`git show cpp-editor-last:<path> > <path>` - reading from history is allowed).
- The editor's own data (`rawdata\`, `gamedata\`, `fsgame.ltx`, `fs*.ltx`), which the Rust
  editor reads, stays; what only the C++ editor read goes (its ImGui layout defaults, `fs_cs.ltx`,
  `fs_soc.ltx`) - check each against the Rust code (`rg`) before it goes.
- **The built C++ editor on disk stays where it is**: `Bin\x64\Release\LevelEditor.exe` and its
  DLLs are not tracked by git; nothing removes or overwrites them. They are the oracle that
  `tools/qa/diff_editors.py` can still record from while they exist.

## 2. The Rust editor under its own name and port

- `Build.ps1` builds the Rust editor in release (`cargo build --release` through the same
  settings as `tools/rust/cg.sh`) and puts `Grizzly.exe` and `grizzly-mcp.exe` into
  `Bin\x64\Release` beside - never over - `LevelEditor.exe`; it still builds the three
  compilers. It runs under pwsh 7 as today and says at the end what it built and how long each
  part took.
- **The default MCP port is 28016** (the editor's own; 28017 was for the time beside the C++
  editor). `-mcp_port` still names another.
- The release profile is the one authors run: check that `Cargo.toml`'s release profile is
  what a shipped editor wants (optimised, one codegen unit or thin LTO as the build time
  allows, debug info kept apart for the crash reports) and say what you chose and what a
  release build takes.
- **The bridge keeps working for clients that are already set up.** AI clients are configured
  with `tools/mcp/grizzly_mcp.py` today: keep that file as a small launcher of `grizzly-mcp.exe`
  (same stdio, same exit code; a clear error when the binary is not built), so no client's
  configuration breaks; `tools/mcp/AI_SETUP.md` documents the binary as the way from now on.
- **QA**: `tools/qa/qa_target.py` targets the Rust editor by default (no environment variable
  needed; `GRIZZLY_QA_EXE` picks a binary - the suites run against the release build once in
  this pass); the C++ target stays reachable for `diff_editors.py`'s recording only
  (`GRIZZLY_QA_TARGET=cpp`), and a suite refuses it with a sentence. `tools/qa/lint_ui_units.py`
  goes (D6). The scripts under `tools/mcp` that drove the C++ editor only go or are moved to
  the Rust one.

## 3. The documents

- **`CLAUDE.md` is rewritten for the new editor**, rule by rule: a rule that describes
  BEHAVIOUR carries over in the new editor's terms (the crate, the command, the file that holds
  it now - found in the code, not assumed); a rule that describes ImGui, a C++ class, a twin
  file under `Source/` or a migration that is over is dropped; the section "The move to Rust"
  becomes "How the editor is built" (the stores, the commands, the four entry points, `unsafe`
  only in `gz-win`, the copy rule of QA). Nothing a contributor or an agent relied on may
  silently vanish: at the end of your report list every rule you dropped and why. Keep its
  voice: short, exact, English, official. The sections about mods, the game's contract,
  releases, the catalog, characters, quests, behaviour graphs, the world, PDA, screens and
  dangerous operations describe behaviour and stay true - only their code references change.
- `tools/mcp/AI_SETUP.md`: the Rust bridge, the port, the catalogue (`crates/gz-mcp/tools.json`),
  the safety paragraphs unchanged word for word.
- `README.md`: what the repository is now, how to build and run it.
- `docs/UI_DESIGN.md`: where it names C++ (`XUI`, `XrUIKit.h`, `EditorUiContract.h`, ImGui),
  the Rust counterpart (`gz-ui`, the contract's place); its design content is not touched.
- `docs/RUST_MIGRATION.md` is the lead's: write what phase 7 changed in it (section 13's
  "side by side" paragraph, the status line) into `docs/rust/STATE.md`, "For the plan".

## 4. Proof

- The three compilers build after the deletion.
- `tools/rust/check.sh`, `tools/rust/test.sh --all`, `python tools/qa/diff_editors.py --all`
  (from the recorded oracle; it must not need the deleted sources), `python tools/qa/run_all.py
  --with-game` against the RELEASE build - all green; the times of the release build beside
  the debug build's (start-up, scene load, a module build).
- `rg` finds no reference to a deleted path in what stays (build files, scripts, documents).
- The repository's size before and after (`git ls-files | wc -l`, lines of C++ left).

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes the working tree or the index (no add, commit, rm,
  checkout, reset, restore, clean): delete files with the file system, read old files with
  `git show cpp-editor-last:<path>`. The lead commits and merges.
- The author's projects (`D:\XFinedProjects\*`) are only ever a source to copy from; the game
  under `D:\Games\Dead Air` is read only (the in-game suite runs in its isolated root as
  before); `find` over both before your first run and after your last shows nothing changed.
- Nothing that already passed is run again to check something new; the full proof once, at the
  end. A C++ build of the compilers takes minutes: start it once, to a file, and look at it
  often. On this machine: build C++ from a developer shell with `-vcvars_ver=14.50` (MSVC 14.51
  has no `<hash_map>`); use the Chocolatey ninja 1.13.2 by its full path for existing build
  folders (the Visual Studio one deletes `.ninja_log` and forces a full rebuild).
- Keep `docs/rust/STATE.md` current: a section "The cutover".

## Report

Under 450 words: what was deleted (folders, files and lines) and what stays for the compilers,
with how the closure was found; the compilers' build result; the release profile and its build
time; the four entry points against the release build with their times and the release-against-
debug numbers; every `CLAUDE.md` rule dropped and why; anything still open and exactly why.
