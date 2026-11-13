# Grizzly Editor updates

The editor checks for a newer version of itself, downloads what differs and replaces itself, from inside the editor.
This document is the contract of that: the release, the client, the application. It is changed here first.

The design is the one Dead Air: Refined ships (the game repo's `docs/dead-air/AUTO_UPDATE.md`, *Delta updates*): a
release is ONE complete archive, and a client reads out of it, by HTTP range, only the files it does not hold - one
version behind or twenty, the download is what actually differs, and nobody publishes a patch per pair of versions.

## 1. The release

A version is one GitHub release of `MMadmer/Grizzly-Editor` whose tag is a strict Semantic Version,
`MAJOR.MINOR.PATCH` (no `v`). It carries:

- `Grizzly-Editor-MAJOR.MINOR.PATCH.zip` - the complete installation (section 2) and the source of every update. The
  name is fixed: it is the one asset every client looks for.
- `Grizzly-Editor-MAJOR.MINOR.PATCH-symbols.zip` - the PDBs of the three programs. Optional for a user (a crash
  report without them carries addresses instead of names); never read by the updater.
- The release notes are the release's body (Markdown). The update dialog shows them.

The archive: entries stored or deflated, nothing encrypted, UTF-8 names with forward slashes, ZIP64 when needed. Its
root is the installation's root. It holds every managed file exactly once and `update-manifest.txt`:

```text
schema=grizzly-editor.update/1
version=MAJOR.MINOR.PATCH
<sha256 hex><TAB><size><TAB><relative path, forward slashes>
```

One row per managed file, the manifest itself excluded, sorted by path. The manifest is the installation's identity:
an installed editor keeps it beside its programs, and the files it names are the **managed set** - the only files an
update ever replaces or removes. Everything else in the folder (the settings, the project list, logs, local game
data, a file the author put there) is not the updater's.

`pwsh -NoProfile -File .\Build.ps1 -Package` makes both archives and the manifest in `Bin\x64\Package` from the
release build and the tracked runtime data (`tools/rust/package.py` holds the file list). Nothing is published by
any tool of this repository: a release goes out by the author's hand.

The schema is frozen once the first release is out: the updater that applies an archive is the one already
installed. A client that meets a schema it does not know offers the release page instead of an update.

## 2. An installation

```text
<root>\
  Grizzly.exe  Grizzly-Cmd.exe  grizzly-mcp.exe      the programs
  LICENSE.txt  THIRD_PARTY_NOTICES.txt
  update-manifest.txt                                the managed set of this version
  fs.ltx  gamedata\...  rawdata\...                  the runtime data the editor ships
  .update\                                           the updater's work (section 5), absent when idle
```

The data root is the folder of the programs (`find_root` meets `fs.ltx` there). An editor without
`update-manifest.txt` beside its programs is a development build - the repository's own `Bin\x64\Release` - and has
no updater: it checks nothing and offers nothing.

## 3. The check

- When: once per start, after the first frame is on the screen (Preferences > Updates > Check at Startup, on by
  default; kept in the editor's home, `update\settings.ltx`), and on Help > Check for Updates. Never in a run without
  a window unless asked (`update.check`); a download a start finds staged continues in any run but a commandlet
  (section 4).
- What: `GET /repos/MMadmer/Grizzly-Editor/releases/latest`, anonymous - no account token, no `Authorization`
  header is ever sent for an update; the `User-Agent` is `Grizzly-Editor/<version>`. The tag is compared with the
  installed version: the one `update-manifest.txt` names, which is the version the installed programs were built as.
  Not newer, no such asset, a network error: nothing is shown at startup (the menu command says "Grizzly Editor is
  up to date." or the error in a sentence).
- **Prepare**, in the same job: the end of the archive - the end of central directory record, ZIP64 locator and
  record, the central directory - is read with `Range` requests, then the record of `update-manifest.txt`, which is
  unpacked and checked against its CRC. Every row is compared with the installation by size and SHA-256 (nothing
  installed is taken on trust; hashes are cached by path, size and write time in the editor's home). What differs or
  is missing is the delta: the local records of those files, merged into one request wherever two lie less than
  256 KiB apart. Their total is the size the editor shows.
- The archive has to describe itself the way the applier needs: every manifest row present in the directory at its
  declared size, no encrypted entry, nothing packed other than stored or deflated. Otherwise, or when the server
  answers a ranged request with `200`, or when the delta is 90 % of the archive or more, the client takes the whole
  archive (resumed by range where the server allows) and unpacks the same files out of it.
- A found update is a chip in the status bar ("Update 0.7.1") and, at startup, one toast. Never a modal. The chip
  opens the Update dialog: installed version, new version, download size, the notes, **Download and Install** /
  **Later**. Later dismisses the toast until the next start; the chip stays.

## 4. The download

A job of the `update` group (one at a time), with a card in the Task Center (Stop Updating) and the chip showing its
percent. It never blocks anything: the editor is fully usable meanwhile.

- Each record is downloaded, unpacked as it arrives and checked against the manifest - size, CRC, SHA-256 - into
  `.update\<version>\files\<path>`; a file is written as `<name>.part` and renamed when it has passed.
- **Nothing downloaded is lost.** `.update\<version>\state.ltx` names the release (version, asset id, size, the
  manifest's hash) and every file that has passed; the raw bytes of the record in flight stay in `<name>.raw` beside
  its `.part`, and a resume unpacks them again and goes on. A dropped connection resumes at the byte it stopped at,
  inside a record if need be. The storage URL the first request was redirected to is reused and dropped when it
  refuses. While a job writes the stage it holds `.update\lock`: two editors never stage into one installation.
- **It continues by itself.** Five failed attempts in a row park the job as "Waiting for the connection" and it
  tries again after 30 s, 1, 2 and then every 5 minutes until it is stopped. An editor that starts and finds a
  download of the still-latest release continues it without asking (a newer release since: the old stage is
  dropped, files that are still right are reused by hash).
- Stop keeps what is there; Download continues from it.
- When every file of the delta is staged and verified the update is **ready**: the chip reads "Restart to Update",
  the dialog offers **Restart and Update** / **Update on Exit** (the default when the dialog is closed).

## 5. The application

Applied by the running editor itself, at the moment it would exit - no second program is copied or started to do
it. Windows lets a running program's file be renamed, which is all this needs.

1. Documents close the usual way (unsaved work is asked about; Cancel cancels the update, not the work). Over MCP
   `update.apply` refuses unsaved work unless it is told `save` or `discard`.
2. The stage is verified once more against the manifest (size and SHA-256 of every staged file; every file the
   delta did not need is checked to still be what the manifest wants).
3. The transaction, journalled in `.update\<version>\apply.journal` (a line per move, flushed before the move):
   every file to replace moves to `.update\<version>\old\<path>`, the staged one moves into place; files the old
   manifest names and the new one does not move to `old\`; the new `update-manifest.txt` goes in last. All moves
   are renames on one volume; a folder the transaction makes is journalled before it is made.
4. A move that fails is tried for five seconds (a scanner reading a fresh program is the usual reason); then the
   journal is rolled back in reverse and the editor says what stopped it. The installation is then exactly what it
   was.
5. **Restart and Update** starts the new `Grizzly.exe` with `-updated <old version>` and the original command line,
   and waits up to 30 s for it to report its first frame (a named event; a run without a window reports once it is
   up). No report, or the new program ended: the journal is rolled back and the next start says so. **Update on
   Exit** applies and exits; the next start is the new version. From the window both happen once the window is gone,
   before the process ends; a failure there is said by the next start, and the installation is then what it was.
   `update.apply` runs the transaction inside the command, so a failure is its answer and the editor runs on.
6. The first start of the new version (`-updated`, or a journal marked complete) deletes `.update\`, off the UI
   thread, and shows one toast: "Updated to 0.7.1" with **What's New** (the notes).

A folder the editor cannot write (probed when the check finds an update) gets no download: the dialog says so and
offers the release page.

Play, the level compiler, a build or a packaging job running: Restart and Update (and `update.apply` without
`on_exit`) is refused with the job named; Update on Exit waits for the exit anyway.

## 6. Commands

`update.status` (version, latest, state: `off | idle | checking | available | downloading | waiting | ready |
applying | failed | manual`, the sizes, the notes, the release page, `updated_from`, the error in a sentence;
`manual` is a release this client cannot install itself - a schema it does not know, a folder it cannot write - and
offers the release page), `update.check`, `update.download` (a check first when need be), `update.stop`,
`update.apply restart=<bool>` (checked and downloaded first when need be; `on_exit=true` leaves it for the exit;
`save` / `discard` settle unsaved work) and `update.set check_at_startup=<bool>`. From a shell: `Grizzly-Cmd.exe
update` checks, downloads and applies in one run and exits `0` (updated or already up to date) or `1`.

## 7. QA

`tools/qa/test_update.py` drives all of it against `tools/qa/github_mock` (`-qa_github`): the release is a mock
release whose archive the suite builds from harmless files (no program is written by a test), and the installation
is a temp folder named by `GRIZZLY_UPDATE_ROOT`, honoured only under `-qa_github` and only with the mock's loopback
API, as the repository is `GRIZZLY_UPDATE_REPO`. The mock's harness routes `/__asset/drop`, `/__asset/noranges` and
`/__asset/stats` cut a download, ignore ranges and count every read. It proves: only the delta's ranges are requested; a connection cut inside a record resumes at
its byte; an editor restarted mid-download continues; a `200` to a ranged request falls back to the whole archive; a
record that does not match the manifest is refused and nothing is applied; apply replaces, removes what the new
version dropped and touches nothing unmanaged; a file held open rolls the whole transaction back; the restart
handshake reports the new start. Nothing is ever fetched from, or published to, GitHub itself.
