# Brief: text formats

Finish the three text primitives everything of phase 2 stands on. Read `docs/rust/CONVENTIONS.md`
and `docs/rust/STATE.md` first; then `crates/gz-base/src/{lib,text,path}.rs`,
`crates/gz-ltx/src/{lib,doc}.rs`, and what already exists of `crates/gz-luadata/src`.

An earlier pass started this work and was stopped half way. What it left is code: `gz-chunk`'s
`lzhuf.rs` and `lzo.rs` are finished (not yours to change), and `gz-luadata` holds `value.rs`
and `num.rs`. Read that code before writing your own - its comments say what was concluded from
the C++ - and build on it where it is right.

You own: `crates/gz-luadata/**`, `crates/gz-ltx/src/tree.rs` (+ `pub use` lines in
`gz-ltx/src/lib.rs`, nothing else there), `crates/gz-fmt-xml/**` (new; write `src/lib.rs`
before `Cargo.toml`).

## 1. `gz-luadata` - declarative Lua data

The editor's own assets (`.nqasset`, `.behasset`, `.npcasset`, `.pdaasset`) are `return { ... }`
Lua tables that carry data and never code. The game reads them the same way and refuses code.

- Spec: `Source/Editors/XrECore/Editor/Nq/NqLua.h` + `NqLua.cpp` (read both fully). See how it is
  called in `Nq/NqAsset.cpp`, `EditorCharacters.cpp`, `EditorPda.cpp`, `Nq/NqMcp.cpp` to learn
  what the API has to offer; do not port those files.
- `value.rs` and `num.rs` were written by an earlier, interrupted pass and are not wired into
  `lib.rs`. Review them against the C++; keep, fix or rewrite.
- Deliver:
  - the value model (`SNqValue` and friends) as a Rust enum / table type that keeps what the
    C++ keeps: key order, array part vs keyed part, how a number was written if the writer
    needs it, comments if the C++ keeps them;
  - `parse(&str) -> Result<Value, LuaDataError { line, column, message }>`: accepts exactly what
    `NqLua::ParseValue` accepts and refuses code exactly where it does, **with the same error
    texts** (they reach the author and QA);
  - the writer(s): output **byte-equal** to the C++ serializer for the same value - indentation,
    key quoting, number formatting, string escaping, line breaks, trailing commas. This is what
    makes "equal text after equal commands on both editors" true;
  - the pure helpers NqLua exposes (escape / unescape, identifier check, number formatting);
  - `Value <-> serde_json::Value` in the shape the MCP layer uses (see `NqMcp.cpp`), and
    `serde::Serialize` for `Value`.
- Why it matters: a quest file edited by the Rust editor must diff clean against the same edit
  made by the C++ editor, or authors' git histories fill with noise.

## 2. `gz-ltx/src/tree.rs` - the engine's resolved config view

The editor reads the linked game's `system.ltx` tree (about 14,000 sections, hundreds of files)
through this. `doc.rs` beside it is the other half (lossless editing) - do not change it.

- Spec: `Source/XrCore/Xr_ini.cpp` + `Xr_ini.h` (`CInifile`), read fully.
- Semantics to reproduce exactly: `#include "file"` (path resolution relative to the including
  file as the C++ does it), `[child]:parent1,parent2` (the copy / override order of the C++),
  duplicates, comment and quote rules, multi-line quoted values, what is lower-cased, and **the
  item order the C++ ends up with** (`insert_item` inserts sorted: `r_line(section, index)`
  must return the same pair at the same index).
- API: `section_exist`, `line_exist`, `line_count`, `r_line`, `r_string` (quotes stripped as
  `r_string_wb`), `r_bool`, `r_u8/u16/u32/s32`, `r_float`, `r_fvector2/3/4`, `r_ivector*`,
  `r_fcolor`, `r_color`, `r_clsid`, `r_token`, sections in file order. Getters return
  `Option` / `Result` that name section, key, file and line; what the C++ treats as fatal
  (missing section or key, duplicate section, unknown parent, missing include) is an `Err`.
- **Two phases, for speed:** `parse_file(text) -> ParsedFile` (items + the includes it names) so
  a caller can read and parse hundreds of files in parallel, then `assemble(root, files)` that
  gives the same result as the sequential C++ load. The loaded `Ltx` is immutable, `Send + Sync`,
  O(1) section lookup, `Arc`-shared strings.
- **The game's dialect.** The editor must also read what the game reads. Check the game's own
  reader (READ-ONLY, never edit that repository):
  `D:\Games\Dead Air\DeadAir-x64\src\xrCore\Xr_ini.cpp`, and grep `ltxp` in that repo's `src`
  and in `docs\dead-air\MODDING.md` / `XMS_ARCHITECTURE.md`. If it added syntax (section
  overrides, deletions, patch files, include wildcards), implement it as an explicit dialect
  option and describe it in the module's rustdoc.

## 3. `gz-fmt-xml` - the game's XML

- Spec: `Source/XrXMLParser/xrXMLParser.cpp` + `.h` (`ParseFile`, the `#include` expansion, the
  navigation API). Provide the same lookups over an immutable tree: navigate to node (by name
  and index, by attribute), read attribute with default, read text, count nodes.
- The game's files are sloppy: stray text, comments, duplicate attributes. Never fail a whole
  file for a recoverable defect; collect warnings as data. Tokenise with `quick-xml` or a
  tolerant parser of your own.
- **String tables** (`configs\text\<lang>\*.xml`, `<string id=".."><text>..</text></string>`):
  an immutable `id -> text` map built from many files in parallel (rayon), later files
  overriding earlier in the order the game applies. The rules to port are the editor's current
  reader: grep `string_table`, `StringTable`, `<text>` in `Source/Editors/XrECore/Editor`
  (`Nq/NqPickers.cpp`, `EditorGameModes.cpp`, `EditorCharacters.cpp`, `Nq/NqUtil.cpp`): which
  language folders, in which order, cp1251 to UTF-8, `\n` handling.
- **Texture sheets** (`textures_descr`: `<file name=".."><texture id=".." x= y= width= height=/>`)
  into `id -> (sheet, rect)`; see `EditorPdaPreview.cpp` and `Nq/NqPickers.cpp`.

## Rules for all three

- Pure functions over `&str` / `&[u8]`; no file access (callers pass text or a loader closure).
- Hostile input is an `Err` or a warning, never a panic, never an unbounded allocation.
- The API of each crate is listed in the rustdoc at the top of its `lib.rs` / module: later
  authors read only that.
- Tests are written, not run: accept / refuse cases and writer golden strings for luadata
  (derive the expected text by reading the C++ writer), inheritance / include / order cases
  for the tree, include + override order for XML.
- Compile: `bash tools/rust/cg.sh check -p gz-luadata -p gz-ltx -p gz-fmt-xml --tests`, zero
  warnings.

## Report

Under 300 words: what each piece offers (entry points), the game's dialect extensions found,
and exactly what is not done or not certainly byte-equal. Then update the rows of these crates
in `docs/rust/STATE.md`.
