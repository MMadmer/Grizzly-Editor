# Brief: releases, publishing and the Mod Browser windows (`gz-ui-release`)

Read `docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first; then `docs/UI_DESIGN.md`
sections 1, 1a, 2, 4 and, in section 6, "Publish Release", "Releases", "Submit to Mod Browser"
- **the contract**; `CLAUDE.md` sections "Mod releases" (the Publish Release dialog, Releases)
and "The Mod Browser" (the Submit dialog: its cards, the VirusTotal card, the required tone,
the withdraw sheet, the footer) - every sentence there about what the windows show is binding;
the rustdoc at the top of `gz-ui`, `gz-app` (`ShellBuilder`: `document`, `dialog`, menu items,
`project_toolbar`, `TaskCardTable`), `gz-publish`, `gz-catalog`, `gz-github`, `gz-release`
(the key line's state), `gz-mod`, `gz-editor`.

You own: `crates/gz-ui-release/**` (new; `src/lib.rs` before `Cargo.toml`), one `install` line
inside `install_ui` and one `register` line (if you register anything) in
`crates/grizzly/src/main.rs`, and the dependency in its `Cargo.toml`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorModPublish.cpp`: the DRAWING half - the Publish Release
  dialog, the Releases tab, `dlg.edit_release`, the delete confirmation's text.
- `Source/Editors/XrECore/Editor/EditorCatalogSubmit.cpp`: the Submit dialog's drawing - cards,
  footer, the withdraw sheet; `EditorCatalog.cpp`: `DrawKeyLine`.
- The hooks the domain crates left for the windows: `ReleaseList::set_tab_open`,
  `set_publish_dialog_open`, `take_edit_request`, `Catalog::set_window`,
  `wants_withdraw_question` (read the rustdoc of `gz-publish` and `gz-catalog`).
- What QA asserts about the windows: `tools/qa/test_mod_publish.py`,
  `tools/qa/test_mod_catalog.py` (dialog ids, the Submit dialog staying inside the picture and
  keeping its height while a job runs).

## What to build

- **Publish Release** (L dialog, `ui_dialog open id=publish_release`): it only SHOWS the module
  in one Release line (name, version, repository) above the Notes; Tag, Title, Options, Requires
  Game, the Author Key and the account wait under More unless they are what stops the release.
  It never writes `mod.ltx`. It reads the repository's releases as it opens (Publish stays off
  until the read ends; a read that fails is only a note) and lists a version that is already
  out as a problem before the click, with Open Mod Settings and Open Releases. Every problem is
  listed at its top. The device code, when shown, in Title Mono with Copy. It closes when the
  job starts: the progress is the Task Center's card (Stop Publishing), the result the kit's
  `result` dialog.
- **The toolbar** (`ShellBuilder::project_toolbar`): Publish Release and Releases after Build
  Mod. Publish Release keeps its label while a job runs - the state is in its tip - and a click
  brings the job's card back. Preferences has nothing to build; the Releases tab does not link
  to itself. Add the Mod menu rows Publish Release..., Releases, Submit to Mod Browser... .
- **Releases** (document kind `releases`, a singleton tab): a sortable table - Tag (fixed at
  the widest tag) | Title | State ("Published" in secondary text for one that is not the
  latest) | Published (local time) | Files ("3 files - 15 KB") - with the selected release's
  details beside it; Edit... (`dlg.edit_release`), Delete... (GhostDanger in the details
  toolbar, the row's context menu in the error colour, Del) through the kit's `confirm`, which
  names the release, its files and the tag and says what players get instead when it is the
  latest one, Cancel the default. A failure is a banner sentence naming what failed.
- **Submit to Mod Browser** (L dialog `Submit to Mod Browser###dlg.catalog_submit`, of a FIXED
  height: what the check finds scrolls inside, and its height never moves while a job runs).
  Cards (the kit's `card`): Author Key; Requirements (failures first, the met ones folded under
  "Met (n)", VirusTotal findings left out); "VirusTotal (required)" - the one place the verdict
  appears, its open state in the Required tone, never grey - with [Scan] [Open Report] [Forget
  VirusTotal Key]; Submission ("Submitting opens a public issue at github.com/<catalog> from
  <login>."; "Not submitted yet." only once GitHub answered); Withdrawal (Withdraw... is
  GhostDanger and raises the sheet `Withdraw from Mod Browser###dlg.catalog_withdraw`, the
  Danger Withdraw and Cancel the default). Every card button is Secondary. While a job runs
  the footer shows the busy line with the stage, a Stop button named after the job (Stop
  Checking, Stop Scanning, Stop Submitting, ...) and Hide; the submit button keeps the width of
  its widest label ("Scan and Submit", "Scan and Update", "Update Submission", "Submit").
- **The key line** (shared by Package Release in `gz-ui-mod` and by these windows): if
  `gz-ui-mod` already exposes one, use it; otherwise build it here as a public function that
  `gz-ui-mod` can switch to.

## Rules

- These windows are faces of commands that already exist (`mod_publish`, `mod_releases`,
  `mod_catalog`): every button posts one; nothing here talks to the network or writes a file,
  and no check is weakened or made optional by the UI. No secret is ever drawn except the
  device code the author must type.
- No editor text may say that a maintainer reviews or merges a submission: the catalog judges
  by itself.
- Draw only with the kit; a widget it lacks is `local_<name>` here, named in your report.
- A frame never computes and never touches a file.
- Compile: `bash tools/rust/cg.sh check -p gz-ui-release -p grizzly`, zero warnings. Then build
  (`bash tools/rust/cg.sh build -p grizzly`), run and LOOK - **with `-qa_github` in
  `-EditorArgs` and without starting any job**: the windows must be pictured without a single
  request leaving the machine. Open each window over `ui_dialog` / `ui_tab` on a temp COPY of a
  project and picture it in its idle state (the read it starts under `-qa_github` goes to an
  address that does not answer - that is the "could not be read" note, and it is fine). Never
  publish, submit, scan or delete anything.
- **Running the editor:** only through `pwsh -NoProfile -File tools\rust\run-editor.ps1 -Driver
  <your script.py> -EditorArgs '-qa_github ...'` (a machine-wide lock: waiting minutes is
  normal). Never start `Grizzly.exe` by hand. If `grizzly` does not build because of someone
  else's crate, wait and retry a few times, then say so. Do not run the QA suites.

## Report

Under 300 words: what is built, `local_*` widgets, what the pictures showed, deviations from
UI_DESIGN and from the C++. Then add a row for `gz-ui-release` to `docs/rust/STATE.md`.
