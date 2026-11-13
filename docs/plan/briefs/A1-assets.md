# Brief A1 - assets: lossless documents, the kinds, the watcher

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5), **`docs/ASSETS.md`** (the contract you implement:
sections 1-6), `docs/COMMANDS.md`, and the survey `Intermediate/plan2/assets.md` (sections 1, 2, 4 - what exists,
with `file:line`).

You own: `crates/gz-luadata`, a new crate `crates/gz-asset`, `crates/gz-characters`, `crates/gz-pda`, the asset
reading/writing and file commands of `crates/gz-nq` (`asset.rs`, the io half of `service.rs`, `cmd.rs`), the watcher
of `crates/gz-project`, `gz-base`'s `Problem`. In the UI crates you touch only the load / save / reload / "changed
on disk" code paths of the graph tab (`gz-ui-graph`), the Characters tab (`gz-ui-world`) and the PDA tab
(`gz-ui-pda`) - no layout, no widgets (other streams restyle those files; re-read a file right before each edit).

## 1. `LuaDoc` - `gz-luadata` keeps a file as a lossless document

Today `gz-luadata` parses to values only and its writer is canonical: a save loses comments, key order, layout and
unknown keys (survey section 2). Add a document layer beside the value reader - the value reader (`parse`,
`parse_named`, the declarative scan, every message and limit) stays the judge of what is data.

- `LuaDoc::parse(text) -> Result<LuaDoc, LuaDataError>`: a concrete syntax tree of `return { ... }`: every token
  with its exact text and span, every comment and blank line attached as trivia. `doc.text()` of an unchanged
  document is the input byte for byte (CRLF and LF kept as found, a BOM kept).
- `doc.value() -> Value` agrees with `parse_named` on every input the latter accepts (property-test it on the
  corpus and on the existing unit inputs); a text `parse_named` refuses is refused with the same message, and the
  error now carries the exact line AND column (no guessed column).
- Paths: `Seg::Key(name) | Seg::Index(n) | Seg::Item { field, value }` (the item of a list whose `id` / `name`
  equals a value). `doc.span(&path) -> Option<Span { line, column, end_line, end_column }>` for positions of
  problems; `doc.get(&path) -> Option<Value>`.
- Edits, each touching only its own bytes: `set(path, &Value)`, `remove(path)` (with the comment lines attached
  directly above it and its trailing same-line comment), `insert(table_path, key_or_position, &Value)`,
  `move_item`. A new fragment is laid out by the canonical writer at the indentation of the table it goes into
  (the indent unit is read from the file: tabs or n spaces), one item per line when the table around it is so.
- **`doc.apply_diff(old: &Value, new: &Value, rules: &DiffRules)`** - how every typed model saves: `old` is the
  model's value as it was read, `new` as it is now. Keys in `new` and not in `old` are inserted (in the place
  `rules` gives: after which known sibling), keys in `old` and not in `new` removed, changed scalars rewritten in
  place, tables recursed. Keys the document holds and neither value names are not touched: that is how unknown
  keys survive. Lists: `rules` says which lists are keyed (`nodes` by `id`, a scene's `objects` by `name`) - those
  match by key and keep the file's order for items both sides have; other lists are positional. A value that is
  equal as data (`1` and `1.0`, `"a"` and `'a'`) is not a change. Nothing changed: the text is identical.
- **Rebase**: `LuaDoc::rebase(base: &Value, mine: &Value, theirs: &str, rules) -> Result<(LuaDoc, Vec<Conflict>)>`:
  parses `theirs`, applies `base -> mine` on it; where `theirs` also changed the same path away from `base` to
  something else, mine is kept and the path, both values and the span are reported as a `Conflict`.
- Limits become a parameter (`Limits::asset()` as today, `Limits::scene()` for files of a few megabytes: a scene's
  `spawns.lua` holds tens of thousands of tables). Speed, release build: parse 5 MB under 250 ms, a one-value
  `apply_diff` on it under 50 ms; a benchmark test prints both and asserts generous bounds (x4).
- The canonical writer stays for new files and for fragments; its "generated header" (the `-- nq:` line and the
  outline in comments) is no longer written by anyone (ASSETS 2.6).

Write the first milestone note `docs/plan/reports/A1-luadoc.md` (the public API, in twenty lines) the moment
`LuaDoc` with `apply_diff` is usable and tested: another stream starts on it.

## 2. `gz-asset` - the one list of kinds

A new crate above `gz-project` and below the kind crates. `gz-project`'s `asset_kind.rs` stays the static table
(extension, icon, template kind) and gains the header key and the current version; `gz-asset` hangs behaviour on it:

```rust
pub struct KindOps {
    pub kind: AssetKind,
    /// Steps from version n to n + 1, in order; applied in memory on read.
    pub migrations: &'static [fn(&mut LuaDoc)],
    /// The problems of one file, positioned. Reads snapshots, never blocks.
    pub check: fn(&Editor, &AssetFile) -> BoxFuture<Vec<Problem>>,
    /// The kind's reference as text: the format document plus what the linked game adds.
    pub reference: fn(&Editor) -> BoxFuture<String>,
    pub diff_rules: fn() -> DiffRules,
}
```

- `AssetFile` = path + `LuaDoc` + the version it was read at. One read path for every kind: read, check the header
  agrees with the extension (`E001`), refuse a newer version (code `newer_version`, the sentence of ASSETS 2.2),
  migrate an older one in memory, report unknown keys of the kind's known levels as notes.
- One save path: `apply_diff`, insert a missing header, write atomically through `fsx` + `WriteGuard`, only when
  the text changed. Registering the write with the watcher so the editor's own save is not an "outside change".
- `Problem` (in `gz-base`) carries `file`, `line`, `column`, `code`, `severity` (`error | warning | note`), `text`
  and an optional `fix` (what to write instead). Every validator you own fills positions through `doc.span`:
  quests and behaviour graphs (a problem names a node, a row or a param - map that to a path), characters, PDA
  (the game's reader names ids - find them in the document; a problem with no findable place points at the
  header).
- Migrations are real machinery with a real test: a test kind at version 3 with two steps reads v1, v2 and v3
  files, refuses v4, and `asset.migrate` rewrites v1 to v3 keeping comments.

Commands (typed, `docs/COMMANDS.md`; area `asset`, registered in `gz-asset`):

| Command | Effect | What |
|---|---|---|
| `asset.kinds` | read | the kinds: extension, header, version, where the reference is |
| `asset.check` | read | `path` (a file or folder; left out: the project). Problems with positions, counts by severity; `fail_on=error|warning` makes the command itself fail (`code: failed`) when such problems exist - the shell's exit code. Covers every kind and `mod.ltx` (call the existing manifest / world / screens checks of `gz-mod`, `gz-world`, `gz-screens` - do not rewrite them) |
| `asset.reference` | read | `kind`: the reference text. Quests and behaviour graphs: `docs/nq/NQ_FORMAT.md` (embedded) plus the linked game's kind catalogue rendered as text (every kind: its params with types, pins, which documents may use it); PDA: `docs/pda/PDA_FORMAT.md`; character: `docs/assets/CHARACTER.md` (write it: every key, its type, default, what the game does with it - from `gz-characters`' model and the game's `MODDING.md` "A module's characters") |
| `asset.find` | read | `query` and / or `refs_to` (a character id, a quest id, a task, a variable): files and lines that mention it. Replaces `quest_project_find` and `quest_references` |
| `asset.migrate` | write | `path` or the project: writes files of older versions in the current one; `dry_run` lists them |

Topic: `reg.topic("assets", ...)` - a page-long text: assets are files, the syntax in five lines, the loop (edit,
`asset.check`, `asset.reference`), pointing at `docs/ASSETS.md`.

## 3. The kinds you own move onto it

- **Quests and behaviour graphs** (`gz-nq`): a document keeps the `LuaDoc` it was read from and the base value;
  Save is `apply_diff`. Node order in the file is the author's (new nodes go after the node that links to them,
  else last). `nq` / `beh` missing is added on save, never written as 0. The generated header is gone. A closed
  file is never rewritten canonically by anyone.
- **Characters** (`gz-characters`): the form saves through `apply_diff` (today it rewrites and drops unknown keys,
  survey fact 3). `npc` is read: older migrates, newer is refused.
- **PDA** (`gz-pda`): the text is the author's and is written as sent; the version policy applies (`pda` newer than
  the editor knows is refused with the sentence; equal or older reads). The PDA reader script is the game's and
  stays untouched (`gz-pda`'s `read.rs` runs it); do not change the bundled copy.
- **Removed** - pure file work an agent does with its own tools: `quest_get`, `quest_write`, `quest_new`,
  `quest_list`, `quest_validate`, `quest_check_all`, `quest_project_find`, `quest_references`, `characters_list`,
  `characters_get`, `characters_write`, `characters_delete`, `pda_list`, `pda_get`, `pda_write`, `pda_delete`,
  `pda_validate`. What they did beyond the file moves: validation into `asset.check`; "the first PDA layout of a
  module becomes the used one" into the file-created hook of the watcher (a `.pdaasset` appearing in a module
  with no `[pda] layout` sets it, as `content_create` does today); templates stay `content_create` (it becomes
  `file.create` in stream A2). `pda_use`, `pda_preview`, `quest_open/close/save/reload/apply/edit/layout/...`,
  `characters_view` stay as they are for the streams that own those surfaces.
- Their callers: the Characters tab saves through the service (not the removed command); the QA suites that used
  the removed commands write the file with Python and call `asset.check` instead (`tools/qa/test_characters.py`,
  `test_pda.py`, `test_behaviours.py`, `test_graph_ui.py`, `test_unicode_names.py`, `test_asset_icons.py`,
  `test_build_contents.py`, `test_world.py` - only those call sites); the catalogue file
  `crates/gz-mcp/tools.json` loses the removed tools (the architect deletes the file itself later).

## 4. The watcher - one per project

`gz-project` has the content browser's watcher (`notify`). Make it the project's one source of "a file changed":

- `ProjectWatch` (a service of the project session): `subscribe(filter) -> Receiver<Change>`; changes debounced
  (100 ms), coalesced per path, tooling and editor scratch ignored by the existing rules (`rules.rs`), the editor's
  own writes filtered by a token the save path registers (path + size + write time + content hash).
- Subscribers: the content browser (as today), the characters roster and the quest index (they stamp the tree
  today - keep that as the fallback when a watch fails), `ManifestService`, and **every open document**: a graph
  tab, the Characters tab, the PDA tab (its polling goes).
- An open document on an outside change: clean - reload and keep the view (selection, scroll, zoom); dirty -
  `rebase` (section 1): its unsaved edits stay applied, and conflicts show in the tab's bar "Changed on disk too:
  <n> values" with the list and **Take Theirs** per item (kit banner; if the kit's banner cannot hold a list, show
  the count and put the list in the tab's Problems - no new widget). A file that no longer parses: the tab keeps
  the last good state, shows the reader's sentence with line and column, and recovers by itself.
- `quest_save`'s "modified externally" refusal and its `force` go: there is nothing to force.

## 5. Tests and proof

- `gz-luadata`: byte-identity of unchanged documents over the corpus (`docs/nq/examples`, `tools/qa/fixtures`, every
  inline sample of the unit tests); `value()` parity with `parse_named`; each edit kind changes only its bytes
  (assert on the diff's line set); `apply_diff` cases: add / remove / change / reorder of keyed lists / unknown
  keys kept / comments kept / CRLF kept; rebase with and without conflicts; the speed test.
- Tests that assert the old canonical rewrite or the generated header are deleted, not adapted.
- `gz-asset`: versions (older, current, newer, missing header), migration chain, positions of problems.
- QA: `python tools/qa/run_all.py test_characters test_pda test_behaviours test_graph_ui` green, plus a new phase
  in `test_graph_ui.py` (or a small `test_assets.py`): open a quest tab, edit the file from outside (Python) with a
  comment and an unknown key added, the tab shows the change; make an unsaved edit in the tab (`quest_edit`),
  change another value outside, Save - both changes and the comment are in the file; `asset.check` positions point
  at the right lines; `Grizzly-Cmd.exe run asset.check` exits 1 on a broken file when the commandlet exists (the
  architect lands it beside you - if `crates/grizzly/src/commandlet.rs` is not there yet, prove the command over
  the socket and say so).

## Done means

Every acceptance sentence of PLAN section 5 "A1" is true and proven by a test or a suite phase you name in the
report; `docs/assets/CHARACTER.md` exists; `docs/nq/NQ_FORMAT.md` and `docs/pda/PDA_FORMAT.md` say how the editor
writes now (no canonical rewrite, no header, versions); no code path of your crates rewrites a file it did not
change.
