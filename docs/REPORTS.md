# Grizzly Editor reports

An author tells the editor's author what went wrong or what they would like - from inside the editor, with an
anonymous diagnostic package when they let it go - and a crash of the editor is offered at the next start. This
document is the contract of that: the package, what it never says, the hub it goes to, the window and the commands.
It is changed here first.

The design is the one Dead Air: Refined ships (the game repo's `docs/dead-air/DIAGNOSTIC_REPORTS.md`): one report hub
for both programs, one privacy rule, a package the hub reads by the keys of its `report.json`.

## 1. What a report is

- **A bug report or a suggestion** (`kind = bug | suggestion`): a title (5 to 200 characters) and a text (20 to 10000),
  sent as written.
- **With diagnostics** (on for a bug and a crash, off for a suggestion; the author decides): the package of section 2.
  Without them the report is its words and the editor's version alone.
- **With a crash** of the editor (section 4): its files go with the package; a crash always goes with diagnostics.

## 2. The package

A ZIP (deflated entries, UTF-8 names) whose root holds:

| Entry | What it holds |
|---|---|
| `report.json` | schema `grizzly-editor.report/1` - below |
| `editor.json` | the editor's state as its own commands answer it: `state`, `app.about`, `project.info`, `game.info`, `level.status`, `doc.list`, `job.list` (without what a finished job answered: a release's account, a module's repository), `perf.stalls`, `update.status` (each asked once, 3 seconds at most; an answer over 512 KB says only its size; none of them changes what it answers by being asked), and what the window's layers say (`gpu`: the adapters and the active one; `ui`: the language and the theme) |
| `logs/editor.log` | the last 4 MB of this run's log, from a whole line |
| `settings/grizzly_editor.ltx` | the editor's settings |
| `crash/crash.txt` | the crash's own report (gz-crash: thread, place, message, exception, module, offset, the frames as module and offset) |
| `crash/run.log` | the last 4 MB of the crashed run's log |
| `crash/crash.dmp` | the crash's minidump, made anonymous (section 3); one over 8 MB (a build of the sources writes whole dumps) is left out |

`report.json`:

```json
{
  "schema": "grizzly-editor.report/1",
  "type": "manual | crash",
  "kind": "bug | suggestion",
  "report_id": "32 hex digits",
  "created_utc": "RFC 3339",
  "build": { "version", "commit", "architecture", "build_id": "SHA-256 of the program", "edition": "setup | portable | development", "profile" },
  "runtime": { "renderer", "location": "the tab's kind", "language", "uptime_s", "window", "memory" },
  "system": { "os", "cpu", "ram_mb", "logical_cpus", "arch" },
  "crash": { "name", "time_utc", "fatal", "exception_name", "module", "module_rva", "thread", "at", "message", "version" },
  "files": [ { "name", "bytes" } ],
  "left_out": [ "what was left out and why" ]
}
```

The hub reads `type`, `report_id`, `schema`, `build.version`, `build.commit`, `build.architecture`, `build.build_id`,
`runtime.renderer`, `runtime.location`, `crash.exception_name`, `crash.module` and `crash.module_rva`; the rest is for
the person who reads the report. `files` names every other entry of the package; `left_out` says everything a
report could have held and does not, and why. `build.commit` is the commit the program was built from (`grizzly`'s
build script). `edition` is `setup` when Setup installed the editor (`unins000.dat` beside the
programs), `portable` for the archive, `development` without an installation manifest.

**Inside the hub's limits.** The hub takes 5 MB packed and 16 MB expanded. A package stays under 4.75 MB and 15 MB:
the logs are cut to their last megabyte first, then the dump, the settings and the state are left out, in that order,
and `left_out` says each.

## 3. Privacy

A report never says who sent it or from which computer. Every text of the package goes through one sanitizer
(`gz-feedback`'s `sanitize.rs`), in this order:

1. **The machine's folders** become markers and keep what lies below them: the editor's (`<editor>`), its data
   (`<editor_data>`), its home (`<home>`), the project (`<project>`), the linked game and the default game (`<game>`),
   the temp folder, the local and roaming application data, OneDrive (personal and work), Documents, Desktop,
   Downloads and the profile (`<temp>`, `<localappdata>`, `<appdata>`, `<onedrive>`, `<documents>`, `<desktop>`,
   `<downloads>`, `<profile>`) - by the environment and by the shell's own folders, in either slash and any case.
2. **Every other absolute path** - a drive's, the long form's (`\\?\D:\`), a `file:///` address's, a server's, its
   backslashes escaped or not - becomes `<path>` and its last name (`E:\Mods\x\a.ltx` → `<path>\a.ltx`).
3. **A GitHub account**: the owner in an address of GitHub (`github.com/<owner>/`, `api.github.com/repos/<owner>/`)
   and the account the project's module releases from (`mod.ltx [update] github`) wherever it stands (`<owner>`).
4. **The user's, the computer's and the domain's names** (`<user>`, `<computer>`, `<domain>`) wherever they stand as a
   word of their own - an underscore ends a word (`grizzly_<user>.log`): from the environment, the profile folder's
   name and the system's own names of the computer. A name of one or two letters is masked inside paths alone.
5. **What looks like a secret**: the value after `token`, `password`, `passwd`, `secret`, `api_key`, `apikey`,
   `authorization`, `bearer`, `private_key` with a separator, four characters or more, and GitHub's keys (`ghp_`,
   `gho_`, `ghs_`, `ghu_`, `github_pat_`) become `<secret>`.
6. **E-mail addresses** (`<email>`) and **IPv4 addresses** (`<ip>`) but loopback and four numbers that are a version:
   one written with a leading zero (`23.20.11.01`), one starting with 0, one after `version`, `build` or `driver`.

JSON answers are cleaned value by value, their keys too. Comparisons fold letters to lower case one by one, keeping
each letter's length in bytes, so a place found is the same place in the text.

**The dump** (`minidump.rs`, the game's own sanitizer of a crash's dump ported): the stream directory is read first,
so nothing the pass zeroes can hide a stream from it; thread stacks and every memory range are erased and their
descriptors emptied (a range that points into the header - a stack a compact dump left out, its size kept and its
`Rva` 0 - only loses its descriptor); a module's name and an unloaded module's are cut to the file name, and so is
the PDB path of its CodeView (`RSDS`, `NB10`) record and an ANSI misc record (a UTF-16 one is erased); the time zone
of misc info is zeroed. Only the streams of a list are kept - threads, modules, unloaded modules, the exception, the
system, misc info, memory info, thread info and names, function tables, the process's and the system's memory
counters; any other stream (handles with the names they point at, a token, a comment) is erased and marked unused.
A dump whose structure does not hold is not sent (`left_out` says so). What stays is the registers, the
modules with their versions and times, the exception and the system: enough to find the instruction that failed,
with the PDBs of the symbols archive.

What the author writes - the title and the text - goes as written: the dialog says so.

## 4. Crashes

gz-crash writes `grizzly_crash_<unix seconds>_<pid>` into the log folder: `.txt` always (a panic the editor caught
at a command or a job's boundary, or one that ended it), `.dmp` when the process ended. The next start of the
editor (`gz-feedback`'s `crashes.rs`, on the IO pool):

- binds the crashed run's log: the newest crash that ended the editor gets the last 4 MB of the `*.previous.log` the
  log rotation set aside whose first line names the crash's process (`Grizzly Editor <version> starting, process
  <pid>`; the crash's name carries the pid) - never by a time, which a run idle long before its fault would miss,
  and never another program's log;
- keeps the ten newest crashes and removes the files of the rest;
- offers the newest crash that ended the editor - the one its run's log is bound to - when it is at most two weeks
  old and was neither sent nor declined: a toast in the window, `feedback.status`'s `offer`.

A crash sent or declined gets `.handled` beside it and is offered no more; its files stay until the ten newer ones
push them out.

## 5. The hub

`POST https://dead-air-refined-madmer.mia0.amvera.tech/api/v1/reports` (the hub of the game's reports, the DARF
server), `multipart/form-data`: `title`, `description`, `version` (the editor's `MAJOR.MINOR.PATCH`), `product =
grizzly-editor`, `kind`, and `report` (the ZIP, `grizzly-report-<report_id>.zip`) when there is a package. The
headers: `Authorization: Bearer <the build's upload key>`, `Idempotency-Key: <report_id>` (a retry after a cut answer
stores nothing twice), `User-Agent: Grizzly-Editor/<version>`. `201` answers `{report: {id, ...}, duplicate}`. A
network failure, `408`, `429` and `5xx` but `507` are tried again after 3 and 10 seconds; any other answer is the
hub's refusal, said in a sentence with the hub's own message.

**The upload key** is the game's (one hub, one key). It is never in the repository: the release build reads it from
`GRIZZLY_REPORT_UPLOAD_TOKEN` at build time or from the ignored `crates/gz-feedback/report_token.local`, and keeps it
out of a plain look at the program the way the game does; `Build.ps1 -Package` refuses a release without it. A
build without one cannot send: `feedback.status` says
`available: false`, the window's Send is off with the reason, `feedback.send` refuses. The key, the request's body and
the package are never written into a log or an answer.

**QA** (`-qa_github`, `GRIZZLY_QA_GITHUB`): the editor speaks to this computer alone - `GRIZZLY_REPORT_URL` names the
mock (`tools/qa/github_mock`'s `/api/v1/reports`), any other address is the dead `127.0.0.1:9` - and the key is
`GRIZZLY_REPORT_TOKEN`, never the build's.

## 6. The window and the commands

- Help > Send Feedback... (`help.feedback`) opens `Send Feedback###dlg.feedback` (docs/UI_DESIGN.md, "Feedback").
- `feedback.status` (read): `available`, `offer` (the crash a start offers), `last` (what the last report sent came
  to), `crashes` (name, time, fatal, handled, summary, whether a dump and a log are there).
- `feedback.send kind= title= text= [diagnostics=] [crash=<name>|latest] [dry_run=] [wait=]` (write; an agent sends
  one only when the user asks for it): a job of the editor (`feedback` group, Stop stops it: code `stopped`) the
  command waits for; `latest` is the newest crash that ended the editor (else the newest of any kind); `dry_run=true` makes the package and answers it
  (`report` is `report.json`, `files`, `left_out`, `bytes`) and sends nothing. Sent, it answers `sent`, `report_id`,
  the hub's `id`, `duplicate`, `bytes`, `files`, `left_out`, `crash`; a crash it carried is marked handled.
- `feedback.dismiss crash=<name>|latest` (write): the crash is offered no more; its files stay.

QA: `tools/qa/test_feedback.py` (a home of its own, the mock hub): the refusals, a dry run read for every trace of the
machine, a bug and a suggestion sent and captured by the mock, a crash planted in the log folder offered with its
process's log (not another's) and on the dialog's card, sent with its dump and offered no more. `gz-feedback`'s tests
feed a real compact dump of the editor through the dump pass.
