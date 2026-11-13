# Brief: Play runs the game in place and no other way; the game's last short buffers

The lead's decision after `docs/rust/briefs/fix-game.md` (its work is the section "The last
review, the game and Play" of `docs/rust/STATE.md`; the game's part is committed in the game
repository as `232c5db9f`, the editor's part lies uncommitted in this tree - build on it).

**The copy path leaves the editor.** Play starts the linked game where it is installed
(`-fsltx <root>\fsgame.ltx`) or it does not start it. Why:

- the editor has not shipped, and the game that states the ability ships before it does (the
  project's rule: no fallback to an earlier revision of either side before the release);
- the copy path is what the author's antivirus answers to - a fresh unsigned editor copying 57
  programs into a temp folder and running the copy - and its only honest proof makes a test do
  exactly that on every run (the previous brief asked for that proof; it was the wrong call);
- it is the larger half of `play.rs`: the import-table reader, the junction into the game's
  archives, the content records copied, a second path limit, a second way to name the config.

**Another engineer works in the editor's tree at the same time** on the shell (`crates/grizzly`,
`crates/gz-win`, `crates/gz-app`, `crates/gz-ui-level`, `crates/gz-scene`, `crates/gz-entities`,
the chooser entries of `crates/gz-mcp/tools.json`). Yours: `crates/gz-levelbuild`,
`crates/gz-game`, `tools/qa/test_play_game.py`, the `grizzly_level_build` entry of
`crates/gz-mcp/tools.json` (patch it by exact anchors, never rewrite the file), one new file in
`crates/gz-win` (point 5), a section of your own appended to `docs/rust/STATE.md` ("Play in
place only"), and the game repository (part B). If a build fails in a file that is not yours,
wait a minute and build again; never edit it.

## A. The editor

1. `crates/gz-levelbuild/src/play.rs`: one way. `EngineChoice`, `PlayMode::Copied` and everything
   only the copy needs go: `copy_changed`, `link_archives`, `top_files`, `has_extension`,
   `close_over`, `programs_of`, the content files copied into `.dead-air-x64`, the config named
   by its bare name, `GamePrograms`' `libraries` and `runtime_list`, `PlayRoot`'s `programs` /
   `program_bytes`, and `crates/gz-levelbuild/src/pe.rs` with its tests when nothing else reads
   it (say so if something does). The module's head comment describes what is left, not what
   was.
2. **An engine that cannot is refused before anything is written or built**, in one sentence
   for the author: which program does not state `fsltx-in-place` (the engine, or the libraries
   of a mixed installation by name) and what to do - Play needs a Dead Air Refined build that
   states it; update the game. No error number, no code. The refusal is `play_place`'s (before
   a build starts), not found out after the compile.
3. **A root an earlier build made with copies stays Play's**: its marker's `program =` lines,
   its `database` link and its `.dead-air-x64` are removed when Play prepares it (the link
   unlinked, never followed) - what the in-place branch does today. Play writes no `program =`
   line any more. The rules of `play_root.rs` (what may be a root, the claims, what is removed)
   do not change.
4. The path limit: `long-paths` and Windows long paths on - 519; `long-paths` and off - 259; an
   engine in place without `long-paths` - 255 as today. `WORKING_FOLDER_LIMIT` stays.
5. `windows_long_paths()` starts `reg.exe` to read one value. Read it in the process instead:
   `gz_win::long_paths_enabled()` - a new file `crates/gz-win/src/registry.rs` (`RegGetValueW`
   on `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem`, `LongPathsEnabled`; the `unsafe` block
   with its `// SAFETY:` line), `mod registry;` + `pub use` + its line in the list at the top of
   `crates/gz-win/src/lib.rs`, the `Win32_System_Registry` feature in its `Cargo.toml`. The
   other engineer has uncommitted lines in those two files (`console`): add yours by `Edit`,
   touch nothing else there.
6. `cmd.rs`: `engine=` goes; the answers drop `play`, `play_reason`, `programs`,
   `program_bytes` and keep `exe` / `exe_path`; the log line says the installed engine was
   started in place. `process::console` stays for `taskkill` (and the tests' `mklink`).
7. `crates/gz-mcp/tools.json`, `grizzly_level_build`: the `engine` property and every word about
   copies go; the text says what Play does now, and what it answers for a game that cannot.
8. Unit tests follow the code: the refusals (an engine without the word, a mixed installation
   naming its library), an old copied root taken over, the limits. Stand-ins keep harmless
   names (no test writes a program file: `tools/rust/check_test_files.py`).
9. `tools/qa/test_play_game.py`: one run, in place - no `--engine`, no copied run, no check
   that reads `play`. It keeps: the installed game listed equal before and after, the root
   holding no program file and no link after Play, the level loaded. Add: the answer carries
   no `play` / `programs` keys.

## B. The game: the last short buffers on a path

`D:\Games\Dead Air\DeadAir-x64`. Your predecessor left three places where a path below a deep
fs root passes through a `MAX_PATH` wide buffer. In the two dumps the conversion of a longer
path leaves the buffer without a terminator (`mbstowcs(wpath, path, MAX_PATH)`) or unset
(`MultiByteToWideChar` failing), and the file is then saved by a name read out of bounds:

- `src/Layers/xrRenderPC_R4/r4_rendertarget_phase_cloud_map.cpp` `dump_cloud_map` (three
  conversions);
- `src/Layers/xrRender/da_trees_bake.cpp` (the dump near line 1930, two conversions);
- `src/Layers/xrRenderPC_R4/da_dstorage.cpp` `ArchiveFile` (falls back today; give it the
  whole path, the fallback stays for a refusal by DirectStorage).

Size each buffer by `string_path` and make a failed conversion a failed dump (its log line),
never a save. Look for more of the kind first (`rg -n "MAX_PATH|mbstowcs|MultiByteToWideChar"
src --glob "*.{cpp,h}"`): what takes a path of the fs (`FS.update_path`, an archive, a file of
`$app_data_root$`) is yours; what takes the application folder, the system folder or a module's
own file name is not. Then the document: `docs/dead-air/CONTENT_BUNDLES.md` loses its sentence
about the renderers' `MAX_PATH` (and says what is true now), `TEST_MATRIX.md` gets the row of
your proof.

Prove one dump on a deep root: your predecessor's `Intermediate/fix-game/prove_long.py` and
`run_in.py` (in the editor's tree) make and run the 330-character root; the game's QA harness
(`tools/qa` in the game repository) has the way to give a running game console commands - run
`da_dump_cloud_map` (the cloud map dump) there and find its PNG at a full path over 260
characters. Then the build (`build_x64.ps1 -Configuration Release`, incremental), `git diff
--check`, and the deploy by the game's `PROJECT_RULES.md` section 8: the game closed, a backup
named for this change (`_predeploy_widepaths_20261113`), the whole runtime list, SHA-256
against `bin\x64\Release`. If `xrEngine.exe` of the installed game is running and you did not
start it, touch nothing of the installation and say the deploy waits.

## How to work

Fully autonomous: every decision on the way is yours, no question to anyone, no sub-agent. No
git command that changes a working tree or an index in either repository (the lead commits),
and no git command at all in the game repository while `test_play_game` runs (it lies inside
the installation the suite lists). `bash tools/rust/cg.sh <cargo args>` for cargo. A wait is
300 s at most, with the log read between waits; nothing that passed is run again to check
something new. The author's projects (`D:\XFinedProjects\*`) are only copied
(`qa_target.copy_project`); an editor on port 28016 is never touched; nothing is pushed or
sent anywhere; no antivirus or system setting is changed.

Proof at the end, once each: `bash tools/rust/test.sh`; `python tools/qa/run_all.py
test_scene_tools test_play_game` (after the deploy); `bash tools/rust/check.sh` (say which
steps fail in files that are not yours); the game's build, its checks and the dump on the deep
root.

## Report

Under 300 words: what left the editor (lines, files) and what an author with an older game
reads now; the registry read; every buffer of the game found and resized, the dump's proof;
the game's build and deploy (files, SHA-256, the backup); every game file changed; the proofs
with their times; a "For the plan" list of what `docs/RUST_MIGRATION.md` and `CLAUDE.md` say
that is no longer so. Nothing left open: if something cannot be closed, say exactly why.
