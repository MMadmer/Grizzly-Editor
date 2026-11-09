# Brief: packaging a release (`gz-release`)

Read `docs/rust/CONVENTIONS.md` ("An area crate") and `docs/rust/STATE.md` first; then
`CLAUDE.md` sections "Mod releases" and "The Mod Browser" (author key, Package Release signs,
content rules, thumbnail and tags), and the rustdoc at the top of `gz-mod`, `gz-build`,
`gz-ltx`, `gz-fmt-tex`, `gz-editor`.

**The formats here are FROZEN** - the one exception to the editor's pre-release freedom: a
published release lives on GitHub and has to install forever. The normative texts are in the
game repository (READ-ONLY): `D:\Games\Dead Air\DeadAir-x64\docs\dead-air\MOD_UPDATES.md`
(descriptor, file index, package names and layout) and `MOD_CATALOG.md` (sections 3, 4.1, 8,
10.3: signatures, the key file, content rules, the script report). Read both before the C++.

You own: `crates/gz-release/**` (new; `src/lib.rs` before `Cargo.toml`) and the registration
lines in `crates/grizzly`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorModManifest.cpp`: `PackageRelease` and what it calls
  (`ReadPackageLayout`, the descriptor and index writers, `kPartInputLimit`, the `[card]`).
- `Source/Editors/XrECore/Editor/EditorCatalog.h/.cpp` (1.6k): the author key file
  (`CreateKey`, open / lock, the password scheme), signing, `CheckStagedContent` (extensions,
  magic bytes, dot names, the script tokenizer and its report), `TagVocabulary`, the thumbnail.
  Its `Draw*` functions are UI.
- The catalog's own tools the key file must stay byte-compatible with: look for
  `xms_catalog.py` / `catalog.py` under `tools/` here or in a sibling `darf-mods` checkout; QA
  cross-checks both ways (`tools/qa/test_mod_catalog.py` - read what it asserts about keys and
  signatures).
- Handler (frozen wire): `mod_package` (`Source/Editors/LevelEditor/Editor/Mcp/LevelMcpMod.cpp`),
  and the key actions of `mod_catalog` that are not network (create / open / lock / status of
  the author key). Schemas: `crates/gz-mcp/tools.json`.

## Design

- **Packaging is a job** (`"package"`, group `"build"`: one build-like job at a time). It
  stages the COMPLETE module through the real export (`gz-build` into the project's
  `temp\release_stage_*`), so a release ships what a build ships and refuses what a build
  refuses; then writes into `<project>\releases\<version>`: the packages, the file index, the
  descriptor. Never a patch or a delta.
- **Packages** (`<id>-<version>.zip`, `.partN.zip` when split so every package stays under the
  650 MB input limit unless one file alone is larger): entries `modules/<id>/` + index path,
  UTF-8 names, no path twice. Compress entries IN PARALLEL on the CPU pool (deflate each file
  to memory or a temp chunk), write them in order, and take each file's SHA-256 while it is fed.
- **The index states what is ON DISK**: offsets, packed lengths and methods are READ BACK from
  the finished archive (walk the local file headers / central directory yourself or with the
  `zip` crate's reader), never worked out from what the writer was asked to do.
- **The descriptor** `<id>.update.ltx`: built as text exactly as the C++ builds it (key order,
  spacing, line ends - it is signed byte for byte). `[card]` (the manifest with its string ids
  resolved, the mode, the tags, the thumbnail and `mod.ltx` pinned by size and hash) and, last,
  `[signature]` over every byte before it. A missing or locked key gives an unsigned release
  and a warning, never a refused build. Nothing edits a signed descriptor afterwards.
- **Crypto**, pure Rust (workspace: `sha2`, `hmac`, `pbkdf2`, `p256`): ECDSA P-256 with SHA-256,
  the signature written `<key id>:<r||s>` base64url; the key file with PBKDF2-SHA256 (600000),
  the HMAC-SHA256 stream and the MAC of MOD_CATALOG 4.1, byte-compatible both ways. The opened
  key lives in memory for the session in a type that zeroes itself on drop and is never
  written to a log or an MCP answer. `create_key` never overwrites a file. The key's default
  place is `Documents\Grizzly Editor\Keys\author.xkey`, remembered in
  `Documents\Grizzly Editor\catalog.ltx`; under `-qa_github` the home is `GRIZZLY_CATALOG_HOME`.
  (A signature differs in bytes from BCrypt's for the same input - ECDSA picks a nonce - and
  verifies all the same: the contract is verification.)
- **Content rules** (`CheckStagedContent`): the tokenizer, the rules and the messages are the
  catalog's `catalog.py`, ported - change both or neither. Run over the staged files in
  parallel; the result is warnings plus `catalog_content` in the answer, and the release is
  written anyway.
- `releases\` is editor-only; inside the project a release may only be written under it.
  Replacing an earlier package of the version is a question for the UI (data here) and plain
  over MCP.

## Done means

- `bash tools/rust/cg.sh check -p gz-release --tests` clean, zero warnings. Tests written (not
  run): a key file written here opens here with the right and refuses the wrong password; sign
  then verify; a three-file package whose index matches offsets read back from the archive;
  descriptor text for a fixture manifest; each content rule.
- The API at the top of `lib.rs`; commands registered.

## Report

Under 300 words: commands, anything that could not be made byte-compatible with the frozen
texts (be exact - this is the part that must not drift), what is left. Then add a row for
`gz-release` to `docs/rust/STATE.md`.
