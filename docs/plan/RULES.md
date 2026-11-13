# Rules for every engineer of the 0.7 work

You are one engineer of a larger piece of work on Grizzly Editor (`docs/plan/PLAN.md`). The architect wrote the plan,
owns every decision in it, reviews your result and makes the commits. Read, in this order: `CLAUDE.md` (the project
rules - they override everything except this file's hard rules), `docs/plan/PLAN.md` sections 1-3 and the section
of your stream, then your brief. Then work until your brief's "Done means" is true. Do not stop early, do not ask
questions: where the brief is silent, decide the way the plan's principles point and write the decision into your
report.

## Hard rules

1. **No sub-agents.** Do the work yourself.
2. **No git command that changes anything** (no add, commit, checkout, stash, reset, restore, clean, worktree). Read
   with `git diff`, `git log`, `git show`. If you must undo your own edit, edit it back.
3. **Your files only.** Your brief names the crates and files you own. A change you need elsewhere: if it is a call
   site that your own API change breaks, fix exactly that and list it in your report; anything else goes into your
   report under "Requests" and you work around it without leaving a stub.
4. **Never** open, list or write anything under `D:\XFinedProjects` (the author's real projects) except as the source
   of `qa_target.copy_project`; never talk to port 28016 (the author's own editor); never start an editor except
   through the QA harness (its own port, its own `-home`, a copy of a project), and close it over MCP, never kill it.
5. **Nothing leaves this computer**: no push, no GitHub / VirusTotal / catalog request (the mock `tools/qa/github_mock`
   under `-qa_github` only), no publishing of anything.
6. **The game repository** `D:\Games\Dead Air\DeadAir-x64` is read-only for you and you run no git command in it,
   unless your brief gives you a game task; then only the files it names.
7. **No test writes a program file** (CLAUDE.md, "Build and verification"). No change of an antivirus or system
   setting.
8. **No `unsafe` outside `gz-win`.** No `static` holding state. Nothing waits on the UI thread (CLAUDE.md).
9. **Nothing you start may wait for ever.** Every program a script or a command of yours starts runs under a time
   limit and is ended with everything it started when the limit runs out: `python tools/rust/limited.py <seconds>
   <program> ...` in a shell or a script, the QA harness's own limits in a suite. Never start an editor with a
   window outside the harness. Before you finish, no process of yours is left running.

## How to build and prove

- Every cargo command through `bash tools/rust/cg.sh <cargo args>`.
- While working: `bash tools/rust/cg.sh check -p <your crate>` and `bash tools/rust/cg.sh clippy -p <your crate>`,
  tests of your crates with `bash tools/rust/cg.sh test -p <your crate>`.
- The programs, only when you need to run them: `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge`
  (`Intermediate\cargo\debug`). Other engineers work in the same checkout on other crates: when the build stops in a
  crate that is not yours, wait a few minutes and build again - do not touch their crate. When a running QA editor
  holds the executable, wait for it.
- QA: only your suites, `python tools/qa/run_all.py <suite> [--only <phases>]`. Never the whole set, never the
  release build - the architect runs the entry points at the end.
- Nothing that already passed is run again to check something new.

## How to write

- All product text is English in the official tone, short; a message is a sentence for the author, never a code or
  an OS error number alone. Comments only where the "why" is not obvious, short, in English.
- **No C++ and no X-Ray SDK in the words.** The C++ editor is gone: no comment, name, message or document says "as
  the C++ editor did", names a `Source/...` file or a C++ function, or explains a behaviour by its history. When
  you touch a file, rewrite such comments to say what the code does and why, on its own terms. Where a FILE FORMAT
  is X-Ray's (`.ogf`, `.object`, `level.geom`, LTX, ...) say so plainly - that is a fact about the format, not
  history.
- `cargo fmt` (run `bash tools/rust/cg.sh fmt -p <crate>`), clippy without a warning, `#![forbid(unsafe_code)]`
  through the workspace lints.
- UI: the kit (`gz-ui`) only, `docs/UI_DESIGN.md` first (when your brief changes a surface, the document is changed
  in the same piece of work), sizes in points, colours from the palette, an MCP way to everything a person can do.
- Commands: the command model of `docs/COMMANDS.md` - a name `area.verb`, a typed parameter struct with a doc
  comment on every field, a one-line summary, the effect class. No command without them.
- No stubs, no TODO, no "later": what your brief asks is finished, or your report says exactly what stopped it and
  what you did instead.

## Your report

Write `docs/plan/reports/<your stream>.md` (create the folder) before you finish:

1. **Built** - what exists now, by file.
2. **Decisions** - every place the brief was silent or you went another way, and why.
3. **Contract changes** - documents you changed (`UI_DESIGN.md`, `COMMANDS.md`, `ASSETS.md`, ...), commands added,
   renamed or removed.
4. **Requests** - what another stream or the architect has to do because of your work.
5. **Proof** - the exact commands you ran and what they answered (tests, suites, phases), and what you could not
   prove.

Your final message is the report's path and its summary in at most twenty lines.
