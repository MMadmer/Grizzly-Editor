# Grizzly Editor

The level and mod editor for **Dead Air: Refined** (the S.T.A.L.K.E.R. X-Ray 1.6 family), written in
Rust. A mod is a project folder of its own - plain text files and a scene; the editor builds it into a
module the game loads from `modules\<id>`, packages and publishes its releases, and lists it in the
game's Mod Browser. Mods target Dead Air Refined strictly.

### What it does

* **Projects that stand alone** - a project is a folder with a `<Name>.grizzly` file: move it, rename
  it, clone it on another computer and open it by that file. Nothing inside names a place outside it;
  the linked game is found by the editor.
* **The level** - one selection over every kind of object, the modes Select, Place, Paint and Zones,
  a Place panel with one search over everything placeable, the World Outliner, Details and Level
  Settings, an Unreal-style viewport (RMB fly + WASD/QE, MMB pan, Alt orbit, gizmos). **Build Level**
  is one job that builds what is stale - game files, the AI map, the game graph, the spawn, geometry
  and light, with the level compiler inside the editor - and **Play** starts the linked game on the
  result. A mod's level changes ship as additive layers (`.xspawn`, `overlay.xcform`, visuals).
* **Content Browser** - the project and the editor's Library in one browser, with thumbnails, search,
  drag and drop and one set of commands for files and folders.
* **Quests and behaviour graphs** - node graphs in `.nqasset` / `.behasset` files that the game's NQ
  runtime interprets: two flow directions, five wire styles, comment frames, reroutes, bookmarks, a
  searchable node picker, auto layout. See [docs/nq/NQ_FORMAT.md](docs/nq/NQ_FORMAT.md).
* **The module's own content** - characters (`.npcasset`), PDA interfaces (`.pdaasset`), loading
  and menu screens with their music, the mode's world rules and start kit.
* **Assets are text** - every asset is a declarative Lua file a person or an agent edits directly;
  the editor keeps comments and unknown keys, rewrites only what changed, follows the files as they
  change on disk, and reads every older version of a format. See [docs/ASSETS.md](docs/ASSETS.md).
* **One-click build and release** - `Mod → Build Mod into Game` (`Ctrl+B`) does a clean build into
  `<game>\modules\<id>` beside the editor; **Publish Release** packages the module and publishes it
  as a GitHub release the game's Mods menu updates players from. See
  [docs/MOD_QUICKSTART.md](docs/MOD_QUICKSTART.md).
* **It updates itself** - a released build looks for a newer version, downloads only what differs,
  continues a download that was cut, and replaces itself with a rollback if anything fails. See
  [docs/UPDATES.md](docs/UPDATES.md).

### Building

One command after a clean clone (needs pwsh 7, Rust through [rustup](https://rustup.rs) -
`rust-toolchain.toml` picks the toolchain - and Visual Studio 2022+ with the *Desktop development
with C++* workload: its C compiler builds the vendored LuaJIT and libvorbis, its Windows SDK gives the
libraries the editor links):

```powershell
pwsh -NoProfile -File .\Build.ps1
```

It builds the editor in release - `Grizzly.exe` (the level compilers are part of it), `Grizzly-Cmd.exe`
(the editor for a command line) and the MCP bridge `grizzly-mcp.exe` - and puts them with their PDBs
and the third-party notices (`licenses\`) into `Bin\x64\Release`, touching nothing else there, and says
how long it took. `-Debug` builds the debug configuration, `-Out <folder>` puts the files elsewhere,
`-Package` also makes the release archive the updater installs from.

While working on the editor: `bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge` (the debug
build in `Intermediate\cargo\debug`). Three entry points prove a change: `bash tools/rust/check.sh`,
`bash tools/rust/test.sh` and `python tools/qa/run_all.py`. The architecture and the contract with the
game are [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); the rules of the repository are
[CLAUDE.md](CLAUDE.md).

### Commands: the window, a shell, an agent

Everything the editor can do is a command with a name, typed parameters and one answer
([docs/COMMANDS.md](docs/COMMANDS.md)). The window's menus, buttons and keys run commands; a shell
runs them without a window; an agent runs them over MCP.

```powershell
Bin\x64\Release\Grizzly.exe "<project folder or .grizzly file>"     # the editor on a project
Bin\x64\Release\Grizzly-Cmd.exe help                                # every command, by area
Bin\x64\Release\Grizzly-Cmd.exe check                               # every problem of the project in this folder
Bin\x64\Release\Grizzly-Cmd.exe reference quest                     # the format of a kind of asset
Bin\x64\Release\Grizzly-Cmd.exe build                               # the module into the linked game
Bin\x64\Release\Grizzly-Cmd.exe run level.build                     # any command; `script <file>` runs several
```

`Grizzly.exe` is a window program; `Grizzly-Cmd.exe` beside it is the same editor for a shell: it waits,
passes the output on and ends with 0, 1 (a command failed) or 2. `-nodlg` never opens a dialog (a
question gets its logged answer, a crash writes a symbolised stack and a dump and exits with code 3).
`-home <folder>` keeps the project list, settings and logs there; `-mcp_port <n>` moves the control
endpoint; `-minimized` starts the window minimised.

### For an agent

A project is plain text: an agent edits the files with its own tools and checks them with
`Grizzly-Cmd.exe check` - no connection to the editor is needed, and an open editor follows every
change. To drive a running editor, connect the stdio bridge `grizzly-mcp.exe`: six tools - state, help,
run, destroy, release, look - over every command. Setup is one page:
[`tools/mcp/AI_SETUP.md`](tools/mcp/AI_SETUP.md). Claude Code one-liner:

```bash
claude mcp add -s user grizzly-editor -- "<repo>/Bin/x64/Release/grizzly-mcp.exe"
```

### Credits

* GSC Game World - the original X-Ray engine and its SDK, whose file formats and level compilers this
  editor implements. This project is not sanctioned by GSC Game World; they remain the copyright
  holders of the original source code.
* [RedPanda Projects](https://github.com/RedPandaProjects/XRayEngine) - the x64 port of the X-Ray SDK
  this editor grew out of.

## License

Grizzly Editor is released under the MIT License ([LICENSE](LICENSE)). The third-party code the two
programs carry is listed with its licenses in
[licenses/THIRD_PARTY_NOTICES.txt](licenses/THIRD_PARTY_NOTICES.txt), made from what they link
(`python tools/rust/notices.py`); `Build.ps1` puts both files beside the programs.
