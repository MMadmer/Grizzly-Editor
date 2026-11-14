# Brief for the critic - is 0.7 finished?

You are the critic of the Grizzly Editor 0.7 work. You change nothing: you read, run what reading cannot settle,
and write findings. The architect fixes them and comes back to you until you have none.

## What was asked for

The author's request, in his words (translated), is the measure - not the plan, not the reports:

1. The interface seriously reworked: it "still follows the old X-Ray pipelines" (placement, drawing, layout, UX,
   buttons, colours). "Do not hesitate to cut the X-Ray editor's legacy out entirely and do things your own way,
   even from scratch. Quality is what is needed."
2. Graphs: several wire styles (sharp "electric" circuit-like ones, curved as before, others popular in engines
   and plugins), better nodes, a node list / picker, extra node actions.
3. The editor checks for updates and updates itself from inside, the way Dead Air Refined itself does: deltas
   whatever the gap between versions, resume, automatic continuation after a break.
4. All new and, ideally, old assets in the format most suitable for an LLM to create and edit directly (not
   through MCP), extensible, old versions opening in newer editors.
5. MCP reworked totally: AI-friendly, less fat, universal and compact, still covering everything.
6. Commandlets as in Unreal: full headless work.
7. Whatever works synchronously and should not, must not block: feedback instead of a frozen editor.
8. Projects stand alone: the folder moved anywhere opens by its project file or from the editor, and "the
   editor is not the only way a project works correctly".
9. Excess tests removed; the C++ editor and every trace of it removed.
10. "Nothing unfinished at the end. No suggestions to finish something later. If something needs work, it is
    done." Version 0.7.0. No public release.

## What to read

- `CLAUDE.md` (the rules of the repository as they now stand) and the contracts it lists: `docs/ARCHITECTURE.md`,
  `docs/COMMANDS.md`, `docs/ASSETS.md`, `docs/UPDATES.md`, `docs/UI_DESIGN.md`, `docs/STATE.md`,
  `tools/mcp/AI_SETUP.md`, `README.md`.
- The code: the whole change is `git diff 80ac92c7..HEAD` (too large to read as one diff - go by area:
  `git diff --stat 80ac92c7..HEAD -- crates/<crate>` and read the files). `docs/plan/reports/*.md` say what each
  engineer built, decided and did NOT prove - treat every "not proven", "not done", "request" line there as a
  lead and check whether it was closed.
- The editor itself, without a window: `python tools/rust/limited.py 60 Intermediate/cargo/debug/Grizzly-Cmd.exe help`
  (every command documents itself), `... help <area>`, `... help <command>`, `... reference <kind>`. Use that debug
  build: it is the current tree. `Bind\Release` holds an OLDER build that is rebuilt only at the very end - do
  not judge it and do not report that it is stale.
- Pictures of every tab kind, dialog and menu as they are NOW (taken by `tools/qa/test_tour.py` at 1707x912 / 1.5):
  `Intermediate/plan/tour/*.png` - look at all 31. Older pictures the engineers took while building, some of
  states since changed: `Intermediate/plan/<stream>/*.png` (use them for surfaces the tour does not show: the Start
  page, the graph canvas with nodes and each wire style, the node picker, the Place panel, Details, the chooser).
- The reports of the last two engineers are `docs/plan/reports/Q1.md` (the suites and checks) and `Q2.md` (the
  sweep of the code's words, typed findings); HEAD is the tree after both and the architect's fixes on top.

## What to look for

- **The request against the result**, point by point: what of points 1-10 is not there, is there only in part,
  or is there in a way the author would not call quality. For the interface look at the pictures and at
  `docs/UI_DESIGN.md`: what still reads as the SDK's editor (a control, a step, a word that exists because the
  old editor had it); what is inconsistent between surfaces; what a person could not find.
- **Unfinished work**: stubs, `todo!`, `unimplemented!`, "for now", "later", "not yet", dead code, a command
  nobody can reach or that answers "not implemented", a parameter that does nothing, a setting with no effect,
  a document that describes what the code does not do, a test that asserts nothing, a suite that skips its point.
- **Contracts against code**: a sentence of a contract the code makes untrue (and the other way round: behaviour
  no contract states). The command model: a command without a card, a destroying call that passes the ordinary
  door, a question asked on a command's path, a "pending, ask again" answer. Assets: a save path that rewrites
  what it did not change or loses a comment or an unknown key; a kind without version handling.
- **The rules of `CLAUDE.md`**: `unsafe` outside `gz-win`; a `static` holding state; the UI thread waiting on a
  file, the network, a lock a job may hold, a GPU read-back; a writer that does not pass the write guard; a UI
  that draws with a colour or a pixel size of its own; a test that writes a program file.
- **Safety**: the bridge's `PLAYER_SAFETY` / `WITHDRAW_SAFETY` paragraphs intact and on the release tool; no
  path by which a QA run reaches GitHub, VirusTotal or the catalog; no token in a log or an answer; nothing that
  opens or writes the author's real projects (`D:\XFinedProjects`) or the installed game.
- **Traces**: any mention of the C++ editor, an oracle, parity with another program, a removed command's name,
  in code, tools, documents or messages.

## Rules

- Read-only: no edit, no git command that changes anything, no editor with a window, nothing that reaches the
  network. Every program you start runs under `python tools/rust/limited.py <seconds> ...`. Never open anything
  under `D:\XFinedProjects`; never talk to port 28016.
- Do not run the three entry points (they take long and the architect runs them); run a single command or test
  when reading cannot settle a point.
- Judge, do not list: a finding is something the author would want fixed before he calls this done. Leave out
  taste and what a later feature would add.

## What to answer

Write `docs/plan/reports/CRITIC.md` (overwrite it each round) and answer with the same text:

- **Findings**, most serious first, each: what is wrong, where (`file:line` or the command), why it matters
  against the request or a rule, and what would close it. Mark each `blocker` (the request is not met or a rule
  is broken), `should` (the author would want it fixed) or `minor`.
- **Checked and found sound**: one line per area, so the architect knows what you covered.
- When you have no blocker and no should left: say **"Nothing open."** and nothing else of substance.

## Round 2

Round 1's report is `docs/plan/reports/CRITIC-1.md` (one blocker, eight `should`, seven `minor`). Since then
(`git log --oneline 0dcaf211..HEAD`) the architect and three engineers changed the tree; their reports are
`docs/plan/reports/E1.md` (the library commands, a surface's error and busy states, the character's model),
`E2.md` (the viewport's camera, the pick list, the comments) and `E3.md` (the kit merges). You are a new reader:
you did not write round 1.

1. **Is each finding of round 1 closed?** Check every one in the code and the documents as they are now - not in
   the commit messages and not in the engineers' reports. For the blocker look hardest: is there any way left by
   which a program a suite starts reaches GitHub, VirusTotal, the catalog, the module's website or the author's
   own key file (`tools/qa/qa.py`, every `subprocess` in `tools/qa/*.py`, `crates/gz-github/src/endpoints.rs`,
   `crates/gz-release/src/keys.rs`, `crates/grizzly/src/main.rs`, the updater's `crates/gz-update/src/net.rs`)?
2. **The new work, by the same measure as round 1**: the commands `library.set` / `play` / `import` / `rebuild`
   and what `library.view` and `library.reload` gained; `scene.summary` and the doors of `scene.save`,
   `scene.convert`, `scene.recover`; `ui.audit`'s `loading` and `errors`; `Executors::thread`; the label limit of
   the viewport; the kit's new widgets (`tabs`, `checker`, the `Combo`'s new states, `form::row_field`,
   `secret_field`, `table_headers_ruled`) and what each `local.rs` still holds against `docs/UI_DESIGN.md`
   section 8; `tools/rust/check_words.py` as widened. Unfinished work, contract sentences the code makes untrue,
   rules of `CLAUDE.md` broken, traces - as before.
3. **What the engineers say they did not prove** (the "not proven" sections of E1, E2, E3 and of `docs/STATE.md`
   section 2): judge each - is it a gap the author would want closed before he calls this done, or is the
   statement in `docs/STATE.md` enough?

Pictures as they are now: `Intermediate/plan/tour/*.png` (1707 x 912 pt at 1.5) and `Intermediate/plan/tour/small/*.png`
(1280 x 720 pt at 1.0) - look at both sets. The last runs on this tree: `check.sh` clean but for the step that
reads the old release build, 1393 unit tests green, the 26 QA suites green (two of them on the runner's second
run alone, after a stall beside the others: `docs/STATE.md`, "The window", says what those are).

Write `docs/plan/reports/CRITIC.md` (round 2) and answer with the same text, in the form of round 1.
