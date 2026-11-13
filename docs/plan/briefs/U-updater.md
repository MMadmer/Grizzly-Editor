# Brief U - the editor updates itself

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3), **`docs/UPDATES.md`** (the contract you implement, every
sentence), `docs/COMMANDS.md`. Reference for the design: the game repo's `docs/dead-air/AUTO_UPDATE.md`, sections
"Delta updates" and "Update application" (read-only), and the editor's own ranged reading for mods:
`crates/gz-release/src/zip.rs`, `crates/gz-github/src/http.rs`.

You own: a new crate `crates/gz-update` (domain, jobs, `service.rs`, `cmd.rs`), `crates/gz-ui-app/src/update.rs`
(the dialog and the Preferences page's Updates group), `Build.ps1`'s `-Package` switch, `tools/rust/package.py`,
the release-asset side of `tools/qa/github_mock` (Go, stdlib), `tools/qa/test_update.py` and its entry in
`tools/qa/run_all.py`. In `gz-app` you add only the hooks you need (the status-bar chip, the Help menu entry, the
"exit with an update staged" step) - another stream restyles that crate at the same time: keep your edits small,
re-read a file right before each edit, and list every hook in your report.

## 1. `gz-update` - the domain (no window)

```text
check    -> the latest release, the archive's directory and manifest by Range, the delta against the installation
download -> the delta's records (or the whole archive), verified, staged; resumable; continues by itself
apply    -> the journalled swap, the restart handshake, the rollback
```

- **Where the installation is**: the folder of the running executable (`AppPaths::exe_dir`) when it holds
  `update-manifest.txt`; without one the updater is `off` (a development build). Under `-qa_github` only:
  `GRIZZLY_UPDATE_ROOT` names the installation and `GRIZZLY_UPDATE_REPO` the repository, and the API base is the
  mock's (`GRIZZLY_GITHUB_API` / `GRIZZLY_GITHUB_WEB`, as `gz-github`'s `endpoints.rs` reads them). Outside
  `-qa_github` those variables are ignored - a test can never point a real editor at a mock, nor a mock run at
  GitHub.
- **HTTP**: `reqwest` through a small client of your own in this crate (anonymous: never the account token, never
  an `Authorization` header; a `User-Agent` of `Grizzly-Editor/<version>`). Ranged GET with the redirect target
  remembered and dropped when it refuses; a `200` to a ranged request is "no ranges here" (fall back to the whole
  archive); timeouts per read, not per file.
- **The archive's tail**: end of central directory, ZIP64 locator and record, the central directory - parse them
  yourself from ranged reads (the directory of `gz-release`'s `zip.rs` reads a local file; share its record
  structs if they fit, do not bend that crate). Entries: stored or deflated only, no encryption, sizes as the
  manifest says - anything else is "take the whole archive".
- **The manifest**: `schema=grizzly-editor.update/1`, `version=`, rows `<sha256>\t<size>\t<path>`; paths refused
  when absolute, with `..`, duplicated, or outside the root. An unknown schema is `state: manual` with the release
  page (no download).
- **The delta**: every row compared with the installed file by size and SHA-256 (hashed on the IO pool; a cache
  `update\hashes.ltx` in the editor's home keyed by path, size and write time); ranges merged under 256 KiB gaps;
  90 % of the archive or more -> the whole archive.
- **Staging** in `<installation>\.update\<version>\`: `state.ltx` (release version, asset id, archive size, the
  manifest's SHA-256, the files that passed), `files\<path>` (written as `.part`, renamed when size, CRC and
  SHA-256 pass), the record in flight kept as raw bytes so a resume continues inside it.
- **Continuation**: five failures in a row park the job (`waiting`), retries after 30 s, 1 min, 2 min, then every
  5 min; an editor start that finds a stage of the still-latest release continues without a question; a newer
  release reuses staged files that still match by hash and drops the rest.
- **Apply** (`UPDATES.md` 5): verify the stage and the untouched files, journal, rename out to `old\`, rename in,
  the new manifest last, five seconds of retries per move, rollback in reverse on any failure. All renames - no
  copy of a program is ever made, and nothing but the staged files is written as a program. The restart: the new
  `Grizzly.exe -updated <old> <original arguments>`, a named event the new process signals at its first frame
  (headless: when it is ready), 30 s, else rollback. The cleanup at the next start, off the UI thread.
- The whole thing is jobs of the group `update` with stages and fractions the Task Center shows; Stop keeps the
  stage.

Commands (typed; area `update`, `reg.area("update", "The editor's own updates")`):
`update.status` (read), `update.check` (write: it may start a job; `wait`), `update.download` (write; `wait`),
`update.stop` (write), `update.apply` (write; `restart: bool`; refuses with the running job named when a build,
a compile, Play or a packaging runs). The answer of `status` is `UPDATES.md` 6.

`gz-win` (the only crate with `unsafe`): what you need from Win32 that `std` lacks - the named event, moving a file
that is in use (`MoveFileExW`), a write probe of a folder. Each `unsafe` block with its `// SAFETY:` line.

## 2. The window

- **Status bar chip** (beside the tasks chip): `Update 0.7.1` (available), `Updating 42%` (downloading; `Waiting`
  parked), `Restart to Update` (ready). Clicking opens the dialog. Nothing when idle or off.
- **The dialog** `Update###dlg.update` (kit dialog, M): "Grizzly Editor 0.7.1 is available. You have 0.7.0." The
  download size ("38 MB of 121 MB" = the delta of the archive). The notes: the release body rendered as plain
  paragraphs and bullet lines in a scroll area (no Markdown engine: headings as Strong, `-` lines as bullets, the
  rest as text). Footer: **Download and Install** (Primary) | **Later**. While downloading: the kit's busy line
  with the stage, **Stop Updating**, **Hide**. Ready: **Restart and Update** (Primary) | **Update on Exit**.
  A folder that cannot be written, or `manual`: the sentence and **Open Release Page**.
- **One toast** at startup when a check finds an update ("Grizzly Editor 0.7.1 is available" with **Details**),
  one after an update ("Updated to 0.7.1" with **What's New**, which opens the dialog on the notes).
- **Preferences > Updates** (a group on the General page, or its own page if the stream that owns Preferences
  has made pages by then): Check at Startup (on by default), the installed version, **Check Now** with the
  result as a sentence. A development build shows "This is a development build: it does not update itself."
- **Help > Check for Updates...** opens the dialog and starts a check.
- Exit with a staged update and "Update on Exit": the apply step runs after the documents closed, before the
  process ends, with the kit's busy sheet ("Updating to 0.7.1...").
- MCP ways to look at it: `ui.dialog` opens `update`; `update.status` answers everything the dialog shows.

## 3. Packaging - `Build.ps1 -Package`

`tools/rust/package.py` (Python 3, stdlib): takes the release build (`Bin\x64\Release`: the three programs,
`LICENSE.txt`, `THIRD_PARTY_NOTICES.txt`) and the tracked runtime data the editor reads at run time (find what it
actually reads: `git ls-files gamedata rawdata fs.ltx fsgame.ltx` minus what no Rust code opens - list what you
left out and why in your report; the architect removes dead tracked data separately), writes
`update-manifest.txt`, `Grizzly-Editor-<version>.zip` (deflate, UTF-8 names, forward slashes, the manifest
included, deterministic: sorted entries, fixed timestamps) and `Grizzly-Editor-<version>-symbols.zip` (the PDBs)
into `Bin\x64\Package`, then verifies the archive against the manifest by reading it back. The version is the
workspace's (`Cargo.toml`). `Build.ps1 -Package` builds release and runs it; it says what it wrote and the sizes.
It publishes nothing and there is no command anywhere that does.

The installed layout has the programs at the data root (`UPDATES.md` 2): check that `find_root`
(`crates/grizzly/src/main.rs`) resolves an unpacked archive correctly (`fs.ltx` beside the programs) and that an
editor started from it works with its home there; fix what does not.

## 4. QA - `tools/qa/test_update.py` against the mock

The mock (`tools/qa/github_mock`): the latest-release endpoint and ranged asset downloads exist for mods; add what
the suite needs through harness routes (as `/__catalog/*` do): cut the connection after N bytes of an asset
(`/__asset/drop`), answer a ranged request with `200` (`/__asset/noranges`), count requests and bytes per asset
(`/__asset/stats`).

The suite builds two "versions" of a fake installation out of harmless files (text and binary blobs with neutral
names and extensions: no program names, no `MZ` head - CLAUDE.md, "No test writes a program file"), packs version
2 the way `package.py` does (import its functions), publishes it to the mock, starts a headless QA editor with
`-qa_github`, `GRIZZLY_UPDATE_ROOT`, `GRIZZLY_UPDATE_REPO`, and proves every sentence of `UPDATES.md` 7. For the
locked-file rollback the suite itself holds a file of the installation open without share-delete. For the restart
handshake: `update.apply restart=true` restarts the same QA editor binary (the installation is the fake folder,
so no program is replaced) and the suite reconnects and reads `update.status` (`updated_from`).

Rust tests: the directory parser on archives made in memory (with and without ZIP64 records, a comment, a wrong
signature), range merging, manifest parsing and refusals, the journal's rollback over a temp folder of harmless
files, the continuation schedule with a fake clock.

## Done means

`docs/UPDATES.md` is true sentence by sentence; where you had to go another way the document is changed in the same
work and the report says why. `python tools/qa/run_all.py test_update` passes in under two minutes. A development
build never contacts anything. Nothing in your work can publish, and no test fetches from the network.
