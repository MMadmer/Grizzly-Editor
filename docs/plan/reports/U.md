# Report U - the editor updates itself

## 1. Built

**`crates/gz-update`** (new; domain, no window; `#![forbid(unsafe_code)]` through the workspace lints)

- `version.rs` - strict `MAJOR.MINOR.PATCH`.
- `manifest.rs` - `update-manifest.txt`: `schema=grizzly-editor.update/1`, `version=`, rows; refuses absolute paths,
  `..`, empty or dot segments, a path twice (ignoring case), the updater's own files; an unknown schema is its own
  error (`UnknownSchema` -> state `manual`).
- `zipdir.rs` - the end of central directory (a comment allowed), the ZIP64 locator and end record (asks for a
  further read when the end record lies before the bytes given), the central directory with its ZIP64 fields, each
  entry's record span (header to the next record / the directory), the local header's length. Parsed from byte
  slices: the same code reads a ranged tail and a file on disk. `gz-release`'s `zip.rs` is untouched (its reader is
  file-bound; its record structs did not fit a byte-slice parser).
- `prepare.rs` - the check's "Prepare": tail, directory (a second read only when it is not in the tail), the
  manifest's record unpacked and CRC-checked; `ranged_ok` when every row is in the directory at its size, stored or
  deflated, not encrypted. `Source` is the ranged HTTP reader or the local archive.
- `delta.rs` - merging under a 256 KiB gap, the 90 % rule.
- `hashes.rs` - SHA-256 cache `<home>\update\hashes.ltx` by path, size and write time.
- `record.rs` - one record into the stage: raw bytes appended to `<path>.raw` as they arrive, unpacked as they
  arrive into `<path>.part` (stored or raw-deflate), renamed to `<path>` when size, CRC and SHA-256 pass; a resume
  replays the `.raw` and continues inside the record (also inside the local header).
- `stage.rs` - `.update\<version>\` layout and `state.ltx` (written whole, tmp + rename).
- `schedule.rs` - retries: four quick ones (1 s x n), the fifth failure in a row parks: 30 s, 1, 2, then every
  5 min; an attempt that got bytes ends the streak.
- `net.rs` - the client (`reqwest`, native TLS, system proxy): anonymous, `User-Agent: Grizzly-Editor/<version>`,
  connect 15 s, a 30 s timeout per read; redirects followed by hand so the storage address is remembered and
  dropped when it refuses; `200` to a range is `Ranged::Whole`; under `-qa_github` only loopback addresses.
- `apply.rs` - verify (staged files and every untouched row), the journalled transaction (`dir` / `move` lines,
  `sync_data` before each action, the manifest last, `complete`), five seconds of retries per move, rollback in
  reverse, journal-driven rollback after a crash or a failed restart, empty folders of dropped files removed.
- `sys.rs` - the two Win32 needs from `gz-win`.
- `service.rs` - `UpdateService` (an `Editor` service): where the installation is (`resolve`), state, the jobs
  (group `update`, `start_for_editor`: they go on across project closes), the Task Center card (Stop Updating),
  `.update\lock` while a job writes the stage, the startup housekeeping, the restart, `at_exit`.
- `cmd.rs` - `update.status` (read), `update.check`, `update.download`, `update.stop`, `update.apply`,
  `update.set` (write), area `update`.
- Tests: 22 (parsers, merging, manifest refusals, journal rollbacks, schedule with a fake clock, a record cut at
  every interesting byte, and an end-to-end test with an HTTP server of its own: check -> cut download -> apply ->
  next start).

**`crates/gz-win/src/update.rs`** (+ 3 lines in `lib.rs`): `rename_in_place` (`MoveFileExW` with no flags: never a
copy, never a replace) and `NamedEvent` (`CreateEventW` / `SetEvent` / `WaitForSingleObject`), each `unsafe` with its
`// SAFETY:` line, the handle kept as a number (no `unsafe impl Send`).

**`crates/gz-ui-app/src/update.rs`** - the status chip, `Update###dlg.update`, Help > Check for Updates...
(`UPDATE_CHECK`), the toasts, `prefs_group` (Preferences > General > Updates).

**`tools/rust/package.py`**, **`Build.ps1 -Package`**, **`tools/qa/github_mock/assets.go`** (+ edits in
`server.go`, `github.go`, `main.go`), **`tools/qa/test_update.py`**, its entry in `tools/qa/run_all.py`.

Hooks outside my crates (each a few lines, files re-read before each edit):

- `Cargo.toml` (workspace): `gz-update` dependency line.
- `crates/grizzly/Cargo.toml`: `gz-update`. `crates/grizzly/src/cli.rs`: `-updated <version>` (+ a test line).
- `crates/grizzly/src/main.rs`: `gz_update::register`; `gz_update::install(&editor, Start{..})` after the area
  installs; the endpoint bind retried for 5 s in a run started with `-updated` (the predecessor lets go of its port);
  `gz_update::ready` before a headless editor waits for its quit; `gz_update::at_exit` after `close_project` on the
  window/headless path and the commandlet path; a test that an unpacked installation is its own data root.
- `crates/gz-ui-app/Cargo.toml` (`gz-update`), `src/lib.rs` (`pub mod update`, `update::install(b)`),
  `src/prefs.rs` (5 lines at the end of `draw_page_rows`: the Updates group on General).
- `gz-app`: **no edit**. The chip, the dialog and the menu row go through `ShellBuilder`
  (`status_chip`, `dialog`, `command` + `menu_item`); the "exit with an update staged" step is `gz_update::at_exit`
  in `main`, after the window and the project closed (see Decisions).

## 2. Decisions

- **Installed version = the installation manifest's version**, not the compiled one: the manifest is the
  installation's identity (UPDATES 1) and for a real installation the two are equal; under QA the fake installation
  moves through versions while the binary stays. UPDATES 3 says so now.
- **Apply moments.** `update.apply` (MCP, commandlet) runs verify + transaction inside the command, so a failure is
  its answer and the editor runs on; on success it asks the editor to quit 150 ms later (the answer goes out first)
  and, with `restart=true`, `at_exit` starts the new version and waits for its report. The window's Restart and
  Update arms the exit and asks the shell to quit the usual way (unsaved work asked about; a cancelled question turns
  the restart back into Update on Exit, watched for a few frames); Update on Exit (also: closing the ready dialog)
  arms the exit. Both apply in `at_exit`, after the window and the project closed. **No busy sheet at exit**: the
  window is gone by then; holding it open needs a step inside the shell's event loop (`gz-app`, under K's restyle).
  The transaction is renames and takes milliseconds; a failure there (and a restart that did not report) is written
  to `.update\<v>\apply.failed` and said by the next start as a toast, the installation rolled back.
- `update.apply` takes `restart`, `on_exit`, `save`, `discard` (`destroys_with discard`); not ready, it checks and
  downloads first (what `Grizzly-Cmd.exe update` needs); nothing newer: `ok` with `updated:false` (exit 0).
- **State `manual`** added to UPDATES 6 (brief: unknown schema; also a folder the editor cannot write).
- **Check at Startup** lives in `<home>\update\settings.ltx` with `update.set`: the domain crate decides it and
  cannot read `gz-app`'s settings; K's new `PrefGroup` API holds settings rows only (no button, no version row).
- **Write probe** with `std` (create, rename, delete in `.update\`), not `gz-win`: no `unsafe` needed.
- **The record in flight**: raw bytes in `<name>.raw`, unpacked bytes in `<name>.part` (UPDATES 4 says so): a
  deflate stream cannot be resumed from its output.
- **Continuation** runs in any start but a commandlet (headless included: that is how QA proves it); it is the
  continuation of an asked download, not a check. A plan is reused only while `available`/`waiting`; after a failure
  the next download checks again.
- **Restart handshake**: the event is `Local\GrizzlyEditor-updated-<pid of the new process>`; both sides open it
  (whoever is first creates it), so no name travels on the command line. Under QA the program restarted is the
  running executable (the installation holds no program).
- **Toasts carry no action**: the kit's toast has no button. The "Details" / "What's New" actions are the chip:
  `Update 0.7.1` opens the dialog, and the first start after an update shows a `What's New` chip until clicked
  (UI_DESIGN says so). See Requests.
- **Mock**: GET `releases/latest` of a public `--repo` answers without a token (as GitHub); `/__asset/drop` arms only
  on an answer longer than its byte count (so it never hits the small tail or manifest reads by accident).
- **Left out of the package** (`package.py` `LEFT_OUT`): `gamedata/shaders/**` (162 files: HLSL of an earlier
  renderer; no Rust code opens a `.ps`/`.vs`/`.h` - shader names map to built-in WGSL materials in `gz-render`) and
  `gamedata/DOT_NOT_DELETE.txt` (a git placeholder). Kept: `fs.ltx` (`find_root`), `fsgame.ltx` (`gz-lc`'s
  `Sources::of_root`), `gamedata/configs/nq|pda/**` (bundled catalog, PDA reader and sheet), `gamedata/textures/ed/**`
  (textures by name, e.g. the AI map arrows), `rawdata/objects/editor/**` (library objects).
- The mock's cut is a tidy close after N bytes, not a reset: a reset lets Windows drop bytes the client had not read
  yet, so "resumes at its byte" could not be checked against what the server sent.
- `cg.sh fmt -p gz-win` formats the whole crate: A2's `registry.rs` / `instance.rs` (in progress there) went through
  rustfmt with it.
- `find_root` needed no change: an unpacked archive has `fs.ltx` beside the programs; a test proves it.

## 3. Contract changes

- `docs/UPDATES.md`: 3 (the installed version is the manifest's; no `Authorization`, the User-Agent; the setting's
  file; continuation in any run but a commandlet), 4 (`.raw` beside `.part`; `.update\lock`), 5 (MCP `save`/
  `discard`; journalled folders; when the apply happens and how a failure at exit is said; the `update.apply` path),
  6 (`manual`, the parameters, `update.set`), 7 (the harness routes, loopback only).
- `docs/UI_DESIGN.md` section 6: Preferences' Updates group and a new **Updates** entry (chip, dialog by state,
  toasts, Help menu).
- Commands added: `update.status`, `update.check`, `update.download`, `update.stop`, `update.apply`, `update.set`.
- CLI: `-updated <version>`.

## 4. Requests

- **K**: (a) a toast with an action button (Details / What's New) in the kit; the chip stands in until then.
  (b) Optional: a shell step before the window closes, if the "Updating to 0.7.1..." busy sheet at exit is wanted.
  (c) My `UiCommand` `UPDATE_CHECK` + `menu_item("Help", 20, 0, ..)` and the `prefs.rs` hook are call sites for the
  actions / Preferences rework; `contract.rs` `DIALOGS` may list `("update", "Update###dlg.update")`.
- **Architect**: remove `gamedata/shaders/**` and `gamedata/DOT_NOT_DELETE.txt` from the tree if nothing else needs
  them; the first real release is made with `Build.ps1 -Package` and published by hand.

## 5. Proof

- `bash tools/rust/cg.sh test -p gz-update -p gz-win` - gz-update 22 passed (among them the end-to-end test against
  its own HTTP server: check reads 2 ranges, a cut inside a local header resumes at its byte, the unchanged 300 KB
  file is never asked for, apply, next start `updated_from`); gz-win 6 passed (the named event, the rename that never
  replaces).
- `bash tools/rust/cg.sh test -p grizzly` - 11 + 6 passed (the `-updated` option, an unpacked installation is its
  own data root, every card worth reading).
- `bash tools/rust/cg.sh clippy -p gz-update -p gz-win -p gz-ui-app -p grizzly --tests` - no warning in my files
  (the warnings it prints are in gz-asset, gz-characters, gz-pda, gz-ui-world). `cg.sh fmt -p gz-update -p gz-win`,
  `rustfmt` on `gz-ui-app/src/update.rs`; `main.rs` / `cli.rs` / `prefs.rs` edits fmt-clean.
- `python tools/rust/check_test_files.py` - exit 0.
- `go vet` of the mock; `python tools/qa/test_github_mock.py` - 401/401.
- `python tools/rust/package.py` functions: two packs of the same input are byte-identical; `runtime_files()` lists
  130 files. `Build.ps1 -Package` itself was not run (it builds the release: the architect runs release builds).
- `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge`, then `python tools/qa/run_all.py test_update` -
  **PASS, 44 checks, 15.8 s** (A-I: check reads only the end and the manifest, no token; only the delta's ranges, a
  cut resumes at its byte, no byte twice; a held file rolls back to the exact installation; apply replaces, drops,
  touches nothing unmanaged; the new start says so and tidies up; a mismatching record refused, nothing applied; a
  restart mid-download continues from the byte; the restart handshake answers `updated_from`; a 200 falls back to
  the whole archive; `perf.stalls` empty).
- Not proven by a run: the window's surfaces (chip, dialog, Preferences group, Help menu) - built and type-checked,
  not driven (the suite is headless); a real release over GitHub (by rule never).
