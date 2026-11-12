# Grizzly Editor

The level and mod editor for **Dead Air: Refined** (the S.T.A.L.K.E.R. X-Ray 1.6 family), written in
Rust. A mod is a project folder of its own; the editor builds it into a module the game loads from
`modules\<id>`, packages and publishes its releases, and lists it in the game's Mod Browser. Mods
target Dead Air Refined strictly.

### What it does

* **Mod projects** — a Start page of project cards; every write (scenes, builds, imports, logs)
  lands in the project, the editor's own library (`rawdata`, `gamedata`) and the linked game stay
  read-only sources to copy out of.
* **The level** — the scene of the X-Ray SDK (objects, lights, spawn points, way paths, shapes,
  sectors and portals, details, the AI map, wallmarks, particles, sounds), an Unreal-style viewport
  (RMB fly + WASD/QE, MMB pan, Alt orbit, gizmos), the World Outliner, Details and Level Settings.
  The level's changes ship as additive deltas (`.xspawn`, `overlay.xcform`, visuals); **Play**
  builds the level and starts the linked game on it.
* **Content Browser** — the project, the editor's library and the linked game's files in one
  browser, with thumbnails, search, drag and drop and one set of commands for files and folders.
* **Quests and behaviour graphs** — node graphs in `.nqasset` / `.behasset` files (declarative Lua)
  that the game's NQ runtime interprets; the editor validates and ships them. See
  [docs/nq/NQ_FORMAT.md](docs/nq/NQ_FORMAT.md).
* **The module's own content** — characters (`.npcasset`), PDA interfaces (`.pdaasset`), loading
  and menu screens with their music, the mode's world rules and start kit.
* **One-click build and release** — `Mod → Build Mod into Game` (`Ctrl+B`) does a clean build into
  `<game>\modules\<id>` beside the editor; **Publish Release** packages the module and publishes it
  as a GitHub release the game's Mods menu updates players from. See
  [docs/MOD_QUICKSTART.md](docs/MOD_QUICKSTART.md).

### Building

One command after a clean clone (needs pwsh 7, Rust through [rustup](https://rustup.rs) -
`rust-toolchain.toml` picks the toolchain - and Visual Studio 2022+ with the *Desktop development
with C++* workload: its C compiler builds the vendored LuaJIT and libvorbis, its Windows SDK gives the
libraries the editor links):

```powershell
pwsh -NoProfile -File .\Build.ps1
```

It builds the editor in release - `Grizzly.exe` (the level compilers are part of it) and the MCP bridge
`grizzly-mcp.exe` - and puts them with their PDBs and the third-party notices (`licenses\`) into
`Bin\x64\Release`, touching nothing else there, and says how long it took. `-Debug` builds the debug
configuration, `-Out <folder>` puts the files elsewhere. The repository holds no C or C++ of its own;
the C++ editor and compilers it replaced are the tags `cpp-editor-last` and `cpp-compilers-last`.

While working on the editor: `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge` (the debug
build in `Intermediate\cargo\debug`). Five entry points prove a change: `bash tools/rust/check.sh`,
`bash tools/rust/test.sh`, `python tools/qa/diff_editors.py`, `python tools/qa/diff_compile.py` and
`python tools/qa/run_all.py`.
The architecture and the contract with the game are [docs/RUST_MIGRATION.md](docs/RUST_MIGRATION.md);
the rules of the repository are [CLAUDE.md](CLAUDE.md).

### Command line

```powershell
Bin\x64\Release\Grizzly.exe -project "<project>"
```

`-project <folder>` opens that project at start, skipping the Start page. `-nodlg` never opens a
dialog (a question gets its logged answer, a crash writes a symbolised stack and a dump and exits
with code 3) - for unattended runs. `-headless` runs everything but the picture, driven over MCP;
`-home <folder>` keeps the project list, settings and logs there; `-mcp_port <n>` moves the
control endpoint.

### AI integration (MCP)

The editor hosts a local automation endpoint on `127.0.0.1:28016` from its start (the Start page
included, and without a window under `-headless`). Every action of the UI has a command, and the
stdio bridge `grizzly-mcp.exe` serves them as MCP tools - the catalogue
[`crates/gz-mcp/tools.json`](crates/gz-mcp/tools.json): screenshots that are always current
(working even when the window is covered or minimized), the scene, the content browser, quests,
characters, builds and releases, plus `grizzly_list_commands` / `grizzly_exec_command` for the
editor's whole command table.

Setup instructions for any AI agent - including a ready-to-paste prompt block - are in
[`tools/mcp/AI_SETUP.md`](tools/mcp/AI_SETUP.md). Claude Code one-liner:

```bash
claude mcp add -s user grizzly-editor -- "<repo>/Bin/x64/Release/grizzly-mcp.exe"
```

### Credits

* GSC Game World — the original X-Ray engine and editors. This project is not
  sanctioned by GSC Game World; they remain the copyright holders of the
  original source code.
* [RedPanda Projects](https://github.com/RedPandaProjects/XRayEngine) — the
  x64 port of the SDK the C++ editor was based on.
