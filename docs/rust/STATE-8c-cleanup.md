# Phase 8c, second engineer: the last C++ leaves, one build, the icon, tests create no program files

The brief `docs/rust/briefs/lc-cleanup.md`, 12 Nov, branch `phase8-cleanup` (a worktree of the repository at
`a2a0a9c5`). Nothing was committed: the lead commits and merges. The tags `cpp-editor-last` and
`cpp-compilers-last` keep every deleted file.

## 1. The last C++ leaves

**Deleted** (with the file system; the index still lists them until the lead's commit): 1,084 tracked files,
346,390 text lines and 4 binaries (`SDK/Lib/x64/d3dx9.lib`, `DxErr.lib`, `Source/External/RedImage/ISPC/win/ispc.exe`,
`.vs/slnx.sqlite`).
- `Source/` (1,050 files: `Editors/{XrLC, XrLCLight, xrDO_Light, XrDXT, XrETools, XrQSlim, FreeMagic, Public}`,
  `XrCore`, `XrCDB`, `XrAPI`, `External/{zlib, LZO, Ogg, Vorbis, RedImage, Public}`, the two `.rc`, the notices),
  `SDK/` (24), `cmake/` (5), `CMakeLists.txt`, `CMakePresets.json`.
- `.vs/` (3: `CMake Overview`, `ProjectSettings.json` naming the CMake configuration `x64-Debug`, `slnx.sqlite`):
  Visual Studio's state of the CMake project, read by nothing else.
- Of them C and C++: 1,059 files (608 `.h`, 364 `.cpp`, 52 `.c`, 25 `.inl`, 6 `.ch`, 4 `.hpp`), 338,835 lines.
- `.gitignore` (`Source/.vs/`, `Source/External/RedImage/libraries/`) and `.gitattributes` (the LZO rule) lost their
  lines for the deleted folders; `tools/lint/check_encoding.py` its two skips of them.

**No C or C++ is tracked**: of `git ls-files` minus the deleted paths, no `.c .cc .cpp .cxx .h .hpp .hxx .inl .ipp
.ch`, project (`.vcxproj`, `.props`, `.sln` of C++) or CMake file is left but the HLSL of `gamedata/shaders`
(37 `editor/*`, 128 `r5/*`: shader headers named `.h` and the HLSL shared-items project `Shaders.sln` /
`Shaders.vcxitems(.filters)`). They are shader data of the kept C++ editor binary (its renderer reads them from the
repository root while it records an oracle), not C or C++, and nothing under `gamedata\` is deleted by these passes.

**Notices.** The Rust editor ships three pieces of third-party C or C-derived code, found from the build graph
(`cargo tree -i cc -e normal,build`: `luajit-src` through `mlua-sys`, `aotuv_lancer_vorbis_sys` and `ogg_next_sys`
through `vorbis_rs`; `miniz_oxide` and `texpresso` are Rust) and from the sources: LuaJIT, libogg/libvorbis with
aoTuV, and the LZO1X port of `crates/gz-chunk/src/lzo.rs` (statement for statement from LZO 2.10, GPL v2+).
`licenses/THIRD_PARTY_NOTICES.txt` is the old notices rewritten for those three (zlib and Info-ZIP MiniZip went with
the compilers); `licenses/LZO_LICENSE.txt` is `Source/External/LZO/COPYING` byte for byte. The CMake build used to
put both beside the binaries; `Build.ps1` does now.

**`Build.ps1`** builds the Rust editor and nothing else: `cargo build -p grizzly -p gz-mcp-bridge [--release]` with
the settings of `cg.sh`, then copies `Grizzly.exe`, `Grizzly.pdb`, `grizzly-mcp.exe`, `grizzly_mcp.pdb` and the two
notices into `Bin\x64\Release` (`-Debug`: `Bin\x64\Debug`; `-Out <folder>`), nothing else written or removed there,
and prints what it copied and the seconds. Gone: `-Only`, `-Target`, `-Clean`, `-KeepGoing`, the developer shell,
`<hash_map>` toolset search, CMake, Ninja, the encoding gate (now a step of `check.sh`, below). Visual Studio's C++
workload is still needed: `cc` compiles LuaJIT and libvorbis with `cl`, and the link takes the Windows SDK's libraries.
Its run in this copy: 386.9 s for the first release build here, from nothing (cargo 6 min 26 s); 1.0 s again with
nothing changed. `Grizzly.exe` 73.0 MB, `grizzly-mcp.exe` 0.7 MB.

**References to deleted paths** (`Intermediate/cleanup/refs.py` over the tracked files that stay, the records -
briefs, `PARITY.md`, `STATE*.md` - aside): none left but
- Rust comments naming `Source/...` as the oracle of a function: kept, `docs/rust/CONVENTIONS.md` says once that they
  mean the tags;
- documents that already name the tag (`docs/NQ_ARCHITECTURE.md` 3.3, `docs/UI_DESIGN.md`);
- `docs/RUST_MIGRATION.md` (the lead's: see "For the plan"), `crates/gz-levelbuild/src/runs.rs` and
  `tools/qa/diff_compile.py` (the other engineer's), `tools/qa/compare_compile.py` (`XrLC.exe` / `xrDO_Light.exe`
  as the names of the C++ runs inside the records);
- unrelated strings that only look alike (`Source\WAVE name` of a sound source's properties).

**The comparison tools and the oracle binaries.** `tools/qa/qa_target.py`'s C++ target, `diff_editors.py`,
`tools/mcp/run_benchmark.py` and `tools/qa/bench_build.py` start the C++ editor from
`Bin\x64\CppEditor\LevelEditor.exe` (the kept self-contained copy) instead of `Bin\x64\Release`. `diff_editors.py`
keys its records with the fixed `CPP_STAMP`, so the move invalidates none; a scenario without a record and no binary
there is now said plainly and left out ("no record and no oracle binary to make one"), as a busy port already was;
the two benchmarks answer the same way.

**Documents.** `README.md` (Building), `CLAUDE.md` (the C++ bullet of "How the editor is built", `check.sh`'s list,
the Build paragraphs: one build, the icon, no program files in tests; the MCP safety paragraphs untouched),
`tools/mcp/AI_SETUP.md` (the bridge's build line), `docs/rust/CONVENTIONS.md` (the oracle is the tags, the two
`build.rs`, `gz-ico`, a Tests section), `tools/mcp/grizzly_mcp.py` (its hint named `-Only editor`).

## 2. The icon

- **`crates/gz-ico`** (new, layer 0, no dependency but `thiserror`, no `unsafe`): `images` reads an `.ico`,
  `icon_res` writes the compiled resource file of it - the empty 32-bit marker resource, every image as `RT_ICON`
  1..n, the `RT_GROUP_ICON` 1 over them, language 0x0409, the memory flags `rc.exe` sets - and `embed` is what a
  build script calls: `<OUT_DIR>/icon.res`, handed to the linker with `cargo:rustc-link-arg-bins` (`rust-lld` in
  its `lld-link` flavour takes a `.res` as an input and converts it itself). No resource compiler runs, no crate is
  downloaded, nothing generated is in the tree. Three unit tests (the directory, refusals, the `.res` layout).
- `crates/grizzly/build.rs` (the C runtime rule kept) and the new `crates/gz-mcp-bridge/build.rs` embed
  `assets/grizzly-editor-icon.ico`.
- **One source.** The window icon and the caption logo were the 512 px `.png` box-filtered to 64; they now read the
  same `.ico` the executables carry (`gz_app::chrome::logo_rgba`: `gz_ico::for_side`, the 64 px image as drawn for
  that size, filtered only when no image of the side exists). The `.png` stays as the generator's large render
  (`tools/branding/grizzly_editor_icon.py`); no code reads it.
- **The proof**: `tools/rust/check_icon.py <exe>...` reads each executable's first `RT_GROUP_ICON` from its resource
  section, wants as many images as the `.ico` holds and the largest one byte for byte equal to the `.ico`'s largest
  (256 px, 9,856 bytes). On the release build: "Grizzly.exe: the editor's icon (10 images, the 256 px one
equal to the .ico's)", the same for `grizzly-mcp.exe` (and on the debug build before it). A negative control: a build script's executable answers "carries no icon: no
  resource section", a missing file "cannot be read". `check.sh` runs it on `Bin/x64/Release` when both binaries
  are there and says so when they are not.

## 3. Tests create no program files

`tools/rust/check_test_files.py` (a step of `check.sh`): test code is every file under a crate's `tests/`, every item
under `#[cfg(test)]` (a `mod name;` takes its whole file) and every `#[test]`; in it, a string literal naming a file
with a program's extension (`.exe .dll .com .scr .bat .cmd .ps1 .msi`; `.com` not inside a URL), a bare extension
literal (`"exe"`, ... for `with_extension`), a literal starting with `MZ`, or the bytes `b'M'` .. `b'Z'` /
`0x4D, 0x5A` written in code is a finding, unless its line or the line above says `// program-files: in memory`
(the data never reaches the disk). A light Rust lexer (comments, raw and byte strings, char literals) keeps comments
out.

On the tree as found (`git archive a2a0a9c5 crates`) it reports 13 lines in 5 files:
- `gz-release/src/game_version.rs` 138, 183, 185 - **wrote** a fake PE (`MZ`, a `.rsrc` with a version resource) as
  `xrEngine.exe` into the temp folder, then `b"not a program"` under that name. Changed: the reader takes any
  `Read + Seek` (`version_of`; `exe_version(path)` opens the file and calls it), the test reads the fake PE from a
  `Cursor` in memory and asks `linked_game_version` of a folder that does not exist. Nothing is written.
- `gz-editor/src/fsx.rs` 192 - a path `<game>\bin\xrEngine.exe` given to `WriteGuard::check` (never written):
  now `<game>\bin\fsgame.ltx`, the same rule.
- `gz-game/src/version.rs` 38, 47; `gz-release/src/content.rs` 580, 581, 598 - in-memory buffers (`exe_version` of
  bytes, `path_allowed` of a string, `bytes_allowed` of a buffer): marked `// program-files: in memory`.
- `gz-levelbuild/src/play.rs` 256, 257, 265, 266 - **writes** `xrEngine.exe` and `xrCore.dll` into a fake game: the
  other engineer's crate (the brief leaves it to them); on this branch the step fails on exactly these four lines
  until their change is merged.

## Proof (this copy)

- `bash tools/rust/cg.sh check --workspace --all-targets`: clean.
- `cargo clippy --workspace --all-targets`: 0 warnings; `cargo fmt --all -- --check`: clean.
- `bash tools/rust/cg.sh test --workspace --lib`: 1,124 passed, 0 failed (57 test binaries).
- `pwsh -NoProfile -File .\Build.ps1`: 386.9 s (release, from nothing); `check_icon.py` on both release binaries: the
  editor's icon, 10 images, the 256 px one equal.
- `check.sh`'s new block: encoding clean; test files: only the four lines of `gz-levelbuild/src/play.rs`; icon: both.
- Not run here (no data, no editor runs in this copy): the corpus tests, the QA suites, `diff_editors.py`,
  `diff_compile.py` - the lead runs them after the merge.

## Decisions

- `.vs/` counted as "what only the C++ build read" (the IDE's view of the CMake configuration) and deleted.
- Section 1 does not name them, so the lead decided, and deleted with the rest: `T-6638.7z` (20 MB, an archive of
  old X-Ray sources - 7,512 C/C++ files inside it), `ccc__temp__vertices` (a debug dump `XrLCLight` wrote), the empty
  `engine.log`, the empty `.gitmodules`, `rawdata\{actor,particle,shader}_imgui_default.ini` and
  `rawdata\ActorEditorLevel.cform` (of the Actor, Particle and Shader editors, which no kept binary is),
  `gamedata\shaders\r2.zip` and the HLSL project files `gamedata\shaders\r5\Shaders.{sln,vcxitems,vcxitems.filters}`
  (nothing reads them at run time). The shaders themselves stay: the kept C++ editor draws with them when it records.
- `gz-ico` is a workspace dependency like every other crate (`Cargo.toml`, `gz-ico.workspace = true`).
- `Build.ps1` writes the two notices besides the four binaries: they are the distribution of those binaries (the
  CMake build wrote the same two names there), not a third program.
- The icon resource is written by a crate of ours rather than `winresource` / `embed-resource`: neither is in the
  lock, both run `rc.exe` from a Windows SDK they have to find; the `.res` format is 30 lines, and `lld-link`
  converts it.
- `tools/lint/check_encoding.py` was the C++ build's gate; it moves to `check.sh` (the gamedata half - cp1251 for
  the game's parsers - still means something; the C++ half finds no file now).
- The docs name `Bin\x64\CppCompilers` as the compilers' oracle, as the brief says; `tools/qa/diff_compile.py` on
  this branch still reads `Bin\x64\CppEditor` (`CPP_BIN`) - the other engineer's file: the merge has to make the two
  agree.

## For the plan

`docs/RUST_MIGRATION.md` should now say:
- I5: the oracle is the kept binaries - `Bin\x64\CppEditor\LevelEditor.exe` (tag `cpp-editor-last`) and
  `Bin\x64\CppCompilers` (tag `cpp-compilers-last`) - and their records under `Intermediate/oracle`; `Source/`
  is no longer in the tree.
- Phase 8: "Then the rest of the C++ goes" is done: `Source/`, `SDK/`, `cmake/`, the CMake files, `.vs/` deleted;
  the repository holds no C or C++ (the HLSL of `gamedata/shaders` is shader data of the kept C++ editor).
  `Build.ps1` is cargo alone; the notices of the C code the editor links are `licenses/`.
- Section 6 (Rust rules) / 7 (workspace): `gz-ico` is a layer-0 crate; the two binary crates have a `build.rs`
  (the C runtime, the icon) - the one exception to "no build.rs".
- A rule beside the others: no test writes a program file (`check_test_files.py` in `check.sh`).
- Section 13 (working rules): `git diff <synced>..main -- Source/Editors docs tools` names a folder that is gone;
  `docs tools crates` is what is left to compare.
