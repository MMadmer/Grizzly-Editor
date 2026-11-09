# Brief: PDA interfaces and screens (`gz-lua`, `gz-pda`, `gz-screens`)

Read `docs/rust/CONVENTIONS.md` ("An area crate") and `docs/rust/STATE.md` first; then
`CLAUDE.md` sections "PDA interfaces (EditorPda)" and "Screens and music (EditorScreens)" - the
behaviour contract - `docs/pda/PDA_FORMAT.md`, and the rustdoc at the top of `gz-luadata`,
`gz-game`, `gz-mod`, `gz-project`, `gz-fmt-tex`, `gz-build`.

You own: `crates/gz-lua/**`, `crates/gz-pda/**`, `crates/gz-screens/**` (new; `src/lib.rs`
before `Cargo.toml`) and the registration lines in `crates/grizzly`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorPda.h/.cpp` (1k), `EditorPdaInternal.h`: the asset, how
  the game's reader script is run, validation, the build gate, `[pda] layout`.
- `Source/Editors/XrECore/Editor/EditorPdaPreview.cpp` (2.4k): mostly drawing (a later UI
  package). Take from it only the STATE the preview keeps and how actions change it - pages,
  vars, the action log, list rows, keys - because `pda_preview state` answers it.
- `Source/Editors/XrECore/Editor/EditorScreens.cpp` (1.4k): the model of `[screens]`,
  `[screen.<id>]`, `[track.<id>]`, `Import`; its drawing is UI.
- Handlers (frozen wire): `pda_list`, `pda_get`, `pda_write`, `pda_delete`, `pda_use`,
  `pda_validate`, `screens_get`, `screens_set` (`rg -n '"<command>"'` in
  `Source/Editors/LevelEditor/Editor/Mcp` and `XrECore/Editor`). `pda_preview` and
  `screens_view` are UI state: not yours. Schemas: `crates/gz-mcp/tools.json`.

## Design

**`gz-lua` - running the game's own pure-Lua readers.**
- What a PDA layout MEANS is the game's `scripts\xms_pda_layout.script`: pure Lua that turns the
  asset into a flat item list plus coded problems. The editor runs that SAME file - the linked
  game's, or the bundled `gamedata\configs\pda\` copy (`gz_game` gives "linked or bundled") - and
  must not grow a rule of its own.
- `mlua` with LuaJIT, vendored (workspace dependency `mlua`, features `luajit`, `vendored`,
  `send`): the game runs LuaJIT, and table iteration order must match it. If the vendored
  LuaJIT does not build on this MSVC toolchain, fall back to vendored Lua 5.1 and say so in the
  report - then check the reader for `pairs` order dependence.
- A Lua state is not shared between threads: keep a small pool of states (one per worker that
  asks), each with the reader loaded once per script revision. The sandbox loads no `io`, `os`
  or `package` and has an instruction limit - the script comes from the game, an asset from
  anywhere. Values cross as `gz_luadata::Value`.
- API: `LuaHost::call(script: Arc<str>, function, args) -> Result<Value, LuaError>` and what
  the PDA reader needs; nothing PDA-specific in this crate.

**`gz-pda`**
- The asset is text (`gz-luadata`), `pda = 1`. `pda_write` with no text writes the template;
  `use` names it in `[pda] layout` through `gz-mod`'s manifest store; `pda_delete` is final and
  clears `[pda]`.
- `evaluate(asset_text, aspect, lang, state) -> Layout { items, problems }` by calling the
  game's reader through `gz-lua`: a `Query` keyed by the text, the script's revision and the
  inputs, run on the CPU pool. `pda_validate` and the build gate read its problems (E3xx /
  W3xx; `W320` a texture neither the game nor the project has - look sheets up through
  `gz-game`). Register a `BuildCheck`: a used layout with an error, a missing file, or one the
  build leaves out refuses the build.
- The preview's state machine (pages, vars, action log, list rows, keys; what `signal:` and
  `lua:` actions do outside the game - a toast and a log line) as pure data and transitions,
  so the UI tab and `pda_preview` later only draw and forward.

**`gz-screens`**
- The model lives in `mod.ltx` (`[screens]`, `[screen.<id>]`, `[track.<id>]`) through
  `gz-mod`'s manifest store, saved as edited. A screen is a stack of layers, one file each; THE
  KIND IS THE FILE'S EXTENSION, nothing is declared.
- `import(file)`: the one way a file enters. Inside the project it stays; from outside it is
  copied to `screens\`; PNG / JPG / TGA / BMP becomes a `.dds` (`gz-fmt-tex`); a `.ps` gets its
  `.s`. Runs on the IO pool; several files import in parallel.
- Validation and a `BuildCheck` with the C++ messages; `screens_get`, `screens_set`.

## Done means

- `bash tools/rust/cg.sh check -p gz-lua -p gz-pda -p gz-screens --tests` clean, zero warnings
  (the first build compiles LuaJIT: let it finish, and say in the report how long it took).
- Tests written (not run): the sandbox refuses `io` / `os`; the bundled reader evaluates the
  template; screens model round trips; import decisions by extension.
- APIs at the top of each `lib.rs`; commands registered.

## Report

Under 300 words: commands, which Lua was built, deviations from the C++ wire, what is left.
Then add rows for the three crates to `docs/rust/STATE.md`.
