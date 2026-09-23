# XFined Editor — MCP setup for AI agents

Copy-paste this file (or its relevant part) into any AI assistant that supports
MCP. It contains everything needed to connect to and control the editor.

## What is available

The XFined Editor hosts a local automation endpoint whenever it is running:

* transport: TCP, line-based JSON, `127.0.0.1:28016`
* alive from the very first frame — the project browser stage included
* an MCP stdio bridge is bundled at `tools/mcp/xfined_mcp.py` (Python 3, stdlib only)

Scene-load performance can be checked against the same endpoint without an MCP
client. Keep the expected object count in the command so a faster incomplete
load fails the benchmark instead of looking like an improvement:

```powershell
python tools/mcp/benchmark_scene_load.py l01_escape.level --warmup 1 --repeats 5 --expected-total 6002
```

## Starting the editor on a project

Launch it with the project already open and the picker skipped — otherwise an
unattended run stops on a modal nobody is there to click:

```powershell
LevelEditor.exe -project "D:\XFinedProjects\Test"
```

`-project` takes a project folder or the name of one in the recent list (quote
paths with spaces). An unopenable value is logged and the picker comes up as
usual. `xfined_open_project` does the same thing on an editor that is already
running.

The project opens on the first frame tick, whether or not the window is ever
drawn: a background editor does not draw, and an unattended one is always in the
background. The last scene auto-loads only once the window is drawn, so an
unattended run loads its scene with `xfined_open_scene`.

**Always add `-nodlg` to an unattended run.** A fatal error then never opens a
system-modal dialog: the log gets a real, symbolised stack trace, a minidump is
written next to the logs, and the process terminates (exit code 3) instead of
hanging on a box nobody will click. Also useful: `-flushlog` (log written line
by line, so a crash keeps what was printed), `-trace` (frame markers and
capture diagnostics).

## Registering the MCP server

**Claude Code:**

```bash
claude mcp add -s user xfined-editor -- python "<repo>/tools/mcp/xfined_mcp.py"
```

**Any other MCP client** — generic server config:

```json
{
  "mcpServers": {
    "xfined-editor": {
      "command": "python",
      "args": ["<repo>/tools/mcp/xfined_mcp.py"]
    }
  }
}
```

Replace `<repo>` with the folder the editor is installed in.

## Tools

| Tool | What it returns |
|---|---|
| `xfined_ping` | editor presence check |
| `xfined_state` | active project name/path, opened scene, FPS |
| `xfined_screenshot_viewport` | PNG image of the 3D viewport render target. Held for its frames the same way, so a camera move or an edit sent just before it is in the picture |
| `xfined_screenshot_editor` | PNG image of the whole editor window (all panels), read from the presented frame, so it works even when the editor is covered by other windows. **One call**: the editor holds the request until the frames it asked for are presented, so the picture shows the state AFTER every command sent before it - a background editor draws nothing unasked, and this is what makes it draw. ~0.3 s; the first one after a start can take seconds |
| `xfined_scene_info` | opened scene file, modified flag, object counts per tool class |
| `xfined_selection` | objects selected in the viewport: name, class, position/rotation/scale |
| `xfined_object_info` | one object by exact `name`: class, selected/visible, transform |
| `xfined_list_projects` | recent projects (name + folder path) |
| `xfined_open_project` | open a project by `path`; its last scene auto-loads |
| `xfined_open_scene` | load a `.level` scene in the active project (synchronous); refuses to replace unsaved work unless `discard=true`, and preserves the current scene if loading fails |
| `xfined_list_objects` | scene object names; filters: `class`, `name_contains`, `limit` |
| `xfined_focus_object` | frame the camera on object `name` like the F key; `select=true` also selects it |
| `xfined_mod_manifest` | XMS mod manifest of the active project: id, name, version, Mods menu metadata (`author`, `description`, `website`, `github`), `api`, `target`, target game `mode`, declared `provides_mode` list, requires/after/before/conflicts, existing module dirs |
| `xfined_mod_set_manifest` | update mod.ltx fields: `id`/`name`/`version`/`mode` strings (`version` = 1-4 numbers separated by dots), Mods menu metadata `author`, `description` (line breaks allowed, 4095 bytes at most), `website` (`https://ap-pro.ru/...` or `https://moddb.com/...` only), `github` (`owner/repo`; a repository URL is reduced to it), `provides_mode_id`+`provides_mode_title` (declare a game mode, empty id removes — the title is what the player reads, any language), `target` (`default`/`existing`/`new`, inferred from the mode fields when omitted, but "the ordinary game" must be said out loud), `requires`/`after`/`before`/`conflicts` comma-joined lists. An invalid value answers `ok:false` and nothing is written; an empty string clears an optional key. `exclude` replaces mod.ltx `[exclude]` - what the build leaves out: project paths (a folder takes everything below it) or patterns (`*.psd` = that name at any depth, `docs/*.md` = a path), one per line or comma separated |
| `xfined_mod_exclude` | the content browser's Exclude from Build / Include in Build for one project-relative `path`; `excluded` = true (default) or false. Stored in mod.ltx `[exclude]`, so it travels with the repository; the build, the package, the quest list and the character list all leave the entry out. Including an entry another rule still takes out fails and names the rule; mod.ltx cannot be excluded. Tooling (.git, .gitignore, .github, .vscode, Thumbs.db, desktop.ini) never ships and needs no entry |
| `xfined_mod_export` | export the project to `<target>/modules/<id>/` as an XMS module (no `target` = deploy to the linked game's `modules/`; **not** `mods/` — that is JSGME's folder, and a module sitting there is one JSGME offers to copy over the game); a **clean build** — the target module folder is emptied first, so nothing a previous build left can survive; refuses targets inside the game's `gamedata`; `flat=true` for a plain `gamedata_<id>` overlay + compatibility report. A module that declares a mode also gets `gamedata/configs/ui/<id>_modes.xml` (and `<id>_modes_16.xml`), `gamedata/configs/text/rus/<id>_modes.xml` and `scripts/mode_register.script` written for it — the checkbox that puts the mode on the new-game screen, measured against the layout the linked game really ships. The answer's `left_out` lists what the build passed by - tooling at any depth, `[exclude]` entries, links; a target that is a link or overlaps the project is refused before anything is written. `warnings` also names every file the game cannot open - a character outside cp1251 in its name; such files ship all the same |
| `xfined_mod_publish` | package the project and publish it as a GitHub release of the manifest's `github` repository - the release the game's Mods menu updates from. Draft first, every asset verified (name, size, SHA-256), then published as the latest release. Args `action` (`start` default, `status`, `cancel`, `account`, `save_token` + `token`, `sign_in`, `sign_in_status`, `sign_out`), `tag` (default `v<version>`), `title`, `notes`, `prerelease`, `latest` (default true), `requires_game`, `wait` (bridge only, default true). Returns `state` (`idle`/`uploading`/`done`/`failed`/`cancelled`), `pending`, `stage`, `sent`, `total`, `url`, `served`, `warnings`, `error`, and from `start` also `assets` `[{name,size,sha256}]`, `files`, `requires_game`, `baked`, `package_warnings`. The account comes from the environment, the Credential Manager (`save_token`) or the GitHub CLI |
| `xfined_mod_releases` | the releases the manifest's `github` repository holds, newest first, drafts included. Reads only. Args `action` (`list` default, `status`, `cancel`), `wait` (bridge only, default true). Returns `releases` `[{id, tag, title, notes, draft, prerelease, latest, published, url, bytes, assets}]`, `listed`, `state`, `pending`, `error` |
| `xfined_mod_release_edit` | change the title and/or the notes of one release, named by `tag` or `id`; an omitted field keeps its value. Files, tag and the latest mark are not touched. Returns `note` and the refreshed `releases` |
| `xfined_mod_release_delete` | delete ONE of those releases, named by `tag` or `id` (only `id` names one of several drafts that share a tag), and then its git tag. Permanent - GitHub keeps no copy - and the editor asks nothing on this path; the tool carries MCP's `destructiveHint`, so a client that wants a permission asks for it on its own side. Returns `note`, `tag_kept` and the refreshed `releases` |
| `xfined_mod_package` | package a release for the game's module updater: the descriptor `<id>.update.ltx`, the file index `<id>-<version>.files` and `<id>-<version>.zip` (`.partN.zip` when it has to split under GitHub's 2 GiB asset limit), every package holding the complete module under `modules/<id>/`. The index states every file's SHA-256 and the byte range of its data inside a package, read back from the finished archive - the game downloads only the files that differ. Args `out_dir` (default `<project>/releases/<version>`; inside the project only `releases/` is accepted), `requires_game` (`MAJOR.MINOR.PATCH`; omitted = the linked game's version, empty = no requirement). Runs the pre-build bake and the same gate as `xfined_mod_export`, and also refuses a version that is not 1-4 numbers separated by dots. No questions asked: files an earlier packaging of the same version left in `out_dir` are replaced (deleted, no Recycle Bin). Returns `descriptor`, `out_dir`, `index` `{name,size,sha256}`, `packages` `[{name,size,sha256}]`, `files`, `unpacked`, `packed`, `requires_game`, `baked`, `warnings` and `left_out` (what the build passed by, as for `xfined_mod_export`). Cancelable through `xfined_progress` |
| `xfined_characters_list` | the module's OWN NPCs, one `.npcasset` each. Returns `characters` `[{id, file, name, community, visual, supplies, squad}]`, `module` and `problems` (`E...` blocks a build, `W...` does not). The game knows a character as `<module>_<id>`: that is its spawn section, the squad section `<module>_<id>_squad` and the `character_profile` a placed stalker carries. A quest needs none of those names: `spawn.npc` takes rows `{ character = "<id>", ref = "..." }` (or `section = ` for anybody of the game's), one NPC and one ref per row; `unique` removes every NPC of that kind first and is ON by default for a character, OFF for a section |
| `xfined_characters_get` | one character by `id` or `path`: its parsed fields, its `problems` and the whole file `text` |
| `xfined_characters_write` | create or replace a character. `text` is the whole file - declarative Lua, `return { npc = 1, id, name, community, visual, icon, rank, reputation, money = { min, max, infinite }, supplies = { { section, count }, ... }, bio, start_dialog, squad }`. `name`/`bio` are a string or a table keyed by language (`{ rus = ..., eng = ... }`). `base` names another character to start from: unset fields are inherited and the starting KITS ARE MERGED - a repeated section takes the new count, `count = 0` drops an inherited row. `abstract = true` makes it a template only (no profile, no spawn section, no name or model needed), which is how several characters share one kit. `E111` is a base that does not exist, `E112` a chain that loops. Note the model is FIXED: an outfit changes an NPC's protection, not its visual. Without `path`/`id` the file lands at `characters/<id>.npcasset`. Canonical: write(read(x)) is byte-identical. A portrait, dialog or kit item the linked game (and the project) does not have is W115 / W116 / W117 - look them up with `xfined_quest_lookup` type `character_icon` / `dialog_id` / `item_section` |
| `xfined_characters_delete` | remove a character (deleted for good, no Recycle Bin; the next build takes it out of the module). Destructive - the editor asks nothing, so ask the user first unless they already said to |
| `xfined_mod_export_spawn` | export placed objects as an additive spawn layer `spawn/<level>.xspawn` (`op=add` + section + world transform + visual override) — the composable way to add things to a base level; only AUTHORED objects ship (see `xfined_mark_authored`); a project-local model is staged into `gamedata/meshes/`, a model found nowhere skips its op instead of shipping a crash; args `level`, `selected_only`, `section`, `mode`. `xfined_mod_export` runs this bake automatically for the open scene (mirror semantics: the scene is the level file's source of truth) |
| `xfined_mark_authored` | flip the mod-content flag on the selection (`authored`, default true) — the build ships only authored objects; an imported base level's objects stay base and never export |
| `xfined_asset_preview_model` | render a `.ogf` model preview as a base64 PNG (game meshes have no baked thumbnail); `name` + `source` = `visual` (editor FS) or `darf` (linked game install) |
| `xfined_mod_export_cut` | subtractive delta: selected objects act as volumes — their bounds become cut boxes (base collision removed) + `hide` entries (base visuals detached); args `level`, `selected_only`, `overlap`, `grow` |
| `xfined_mod_export_xcform` | bake scene objects into `levels/<level>/overlay.xcform` (world-space collision triangles + game materials); args `level`, `sector`, `selected_only` |
| `xfined_mod_export_ogf_probe` | export selected static objects as world-space `.ogf` into `levels/<level>/visuals/` + register them in `overlay_visuals.ltx`; skeleton/dynamic refs are skipped |
| `xfined_game_link` | report the project's linked game install (`linked`, `game_root`, `reason`), or link one by `path` (needs fsgame.ltx + `database\` or `gamedata\`); the game folder is only ever read |
| `xfined_darf_list` | files of the linked game install (`database\*.xdb*` archives + loose `gamedata\`) as one gamedata-relative listing; filters `filter`, `ext`, `limit` (default 200); each entry has `path`, `size`, `source` (`archive`/`loose`) |
| `xfined_darf_copy` | copy one game file (`path` from `darf_list`) into `<project>/gamedata/<same relative path>`; `overwrite` defaults to false and an existing target is reported instead of replaced |
| `xfined_scene_stats` | scene_info + selection count + camera position/rotation |
| `xfined_camera_get` | camera position, `hpb` rotation (radians), view direction |
| `xfined_camera_set` | move/rotate the camera: `x`/`y`/`z`, `h`/`p`/`b`; omitted fields stay |
| `xfined_select_objects` | select objects by exact `names` (comma list); clears previous selection, returns missing names |
| `xfined_deselect_all` | clear the selection in every tool |
| `xfined_object_set_position` | set object position by `name` (`x`/`y`/`z`, partial ok); undoable |
| `xfined_object_set_rotation` | set object rotation in radians by `name` (`x`/`y`/`z`, partial ok); undoable |
| `xfined_object_set_scale` | set object scale by `name` (`x`/`y`/`z`, partial ok); undoable |
| `xfined_place_object` | place a library object (`ref`) with a full transform: `x/y/z`, `rx/ry/rz` (radians), `sx/sy/sz` or `scale`, plus `snap_to_ground`/`align_normal`; no position = in front of the camera |
| `xfined_drop_objects` | drop objects onto the surface below (the End key, Unreal Snap-to-Floor semantics); `mode` `box` (swept bounds, default) or `line` (pivot ray), `names` comma list or the selection, `align_normal` |
| `xfined_list_spawn_sections` | the gameplay-object roster: every config section of the **linked game** with `$spawn` — actors, NPC, items, restrictors/triggers, anomalies; `filter` substring, `limit`; feeds `place_spawn` |
| `xfined_place_spawn` | place a gameplay object by config `section` (spawn element, like the old SDK's Spawn Element mode): server entity from the section, declared shapes attached, properties (logic/custom_data) editable in Properties; optional `x`/`y`/`z`, `snap_to_ground` |
| `xfined_view_mode` | read/set viewport rendering: `preset` (`unlit` = normal textured view, `lit` = 8-slot fixed-function lighting so most geometry goes black, `wireframe`, `point`), `fill`, `shade`, and the raw `lighting`/`textures`/`edged_faces`/`filter_linear`/`fog`/`environment`/`grid`/`safe_rect` toggles |
| `xfined_list_assets` | list available assets: `category` (Objects by default), `filter`, `limit`; names feed `place_object`'s `ref` |
| `xfined_asset_preview` | PNG thumbnail of one asset (`name`, optional `category`) for judging its look before placing |
| `xfined_rename_object` | rename a scene object (`name` -> `new_name`, must be unique) |
| `xfined_delete_selected` | delete every selected object; returns the removed count; undoable |
| `xfined_save_scene` | save the opened scene (its current file, or an explicit `file`) |
| `xfined_recovery` | project-scoped crash recovery: `get` reports the session owner, dirty/pending state, validated snapshot and last error; `snapshot` synchronously writes a non-mutating crash snapshot; `restore`/`discard` accept only the startup offer and are deferred to the next safe draw |
| `xfined_outliner_show` | open (`open=true`, default) or close the World Outliner panel |
| `xfined_game_modes` | game modes a module can target, scanned from the LINKED game: campaigns only (rule options like "one life" are excluded, so are checkboxes the game comments out), each with `id`, `caption` (what the player sees), `key` and `source`. A module's target is set in the manifest and the module export refuses to run without one |
| `xfined_outliner_filter` | drive the outliner's search box and funnels: `text` (Unreal grammar - every word must match, `-word` excludes, `"two words"` match together), `types` (`;`-separated classes to show, empty shows all), `selected_only`, `visibility` (`all`/`visible`/`hidden`); omitted fields keep their state and an invalid type/mode changes nothing; returns the complete filter state plus `shown`/`total` |
| `xfined_scene_tree` | the World Outliner's data: groups per object tool class sorted by class name, each with `count`/`total` and `objects` (name, selected, visible); filters `filter`, `class`, `limit` (default 200) |
| `xfined_content_browser_open` | reveal an asset **or a folder** in the Content Browser: opens the panel, switches `source` (`project`/`editor`/`darf`), navigates there and selects it; a folder path (`levels`, `levels\l07_military`) is how the grid is walked into a folder without a mouse; `open=true` also opens an asset's viewer (the double-click action). A file written a moment ago is found: on a miss the project is listed again once |
| `xfined_content_browser_selection` | read the browser's open/source/folder/ordered selection state, or use `action=select` / `toggle` / `range` / `clear` on an exact entry in the complete current view (offscreen virtualized rows included; `select` establishes the range anchor). `action=rename` opens the inline rename box on the entry (F2 without a keyboard); the reply's `rename` `{entry, text, select}` says what the box holds - one of the editor's own assets shows its name WITHOUT the extension, any other file selects the name and leaves the extension out. `clear` closes the box unrenamed |
| `xfined_content_browser_copy` | copy assets out of a READ-ONLY source into the project: `names` (`;`-separated) or `folder` (recursive), optional `source`/`category`, `overwrite` (default true). With `dst` they land in that project-relative folder, a folder keeping its own name and subtree — the same clipboard and paste the browser's menu uses. Without `dst` each asset is mirrored where the engine expects it instead: an Objects entry in `<project>/rawdata/objects/...`, a mesh in `<project>/gamedata/meshes/...`, a texture with its `.thm` |
| `xfined_content_rename` | rename a file or folder of the project in place (`path`, `name` = the new leaf name). A change of case alone is a rename. One of the editor's own assets (`.nqasset` / `.behasset` / `.npcasset`) ALWAYS keeps its extension - it is the kind: the name comes in as a name, dots and all (`route.v2` → `route.v2.behasset`), the same extension typed back is fine, another one of the three is refused. Open quest documents follow the renamed file or folder |
| `xfined_content_copy` / `xfined_content_move` | copy/move files or folders by path; `src` accepts a `;`-separated list, `dst` is a folder inside the project, `recursive`/`overwrite`. Moving out of a read-only source is refused — a move deletes its source |
| `xfined_content_delete` | delete files/folders inside the project; `recursive` defaults to false so a non-empty folder is reported instead of wiped |
| `xfined_content_mkdir` | create a folder (with missing parents) inside the project |
| `xfined_content_create` | the browser's Create menu without the menu: `kind` quest / behaviour / character, made in the folder the browser has open under the first free `New*` name. Returns `made`; rename it with `xfined_content_rename` |
| `xfined_list_commands` | the editor's own command registry: every action the menus and shortcuts can perform, as `{id, name, menu, presets}` — the editor's full control surface, and commands added in future builds appear here automatically |
| `xfined_exec_command` | execute ANY registry command — the same dispatch menus and shortcuts use. Address by `id`, `COMMAND_*` `name` or menu path; `p1s`/`p2s` pass strings, `p1i`/`p2i` integers. Prefer a dedicated tool when one exists; this is the escape hatch for everything else |
| `xfined_theme` | read (`action=get`), persist (`action=set`, `preset=xfined-purple` or `graphite`) or reset the live editor theme; omitting `action` reads it |
| `xfined_frame_pacing` | read live requested/effective frame interval, wait reason/counters and active mode; persist `active_idle_fps` 30..240 (`action=set`) or reset it to 120. PIE/realtime and explicit work are never delayed, and messages/MCP wake the wait immediately |
| `xfined_command_palette` | query the ranked Ctrl+Shift+P catalog, open/close the palette, inspect its state, or execute an exact returned command/subcommand pair |
| `xfined_viewport_navigation` | read/control the viewport orientation widget: six canonical axis views, perspective reset, and frame all/selection |
| `xfined_progress` | read live nested tasks with percentage/detail/elapsed time plus truthful `cancelable`/`cancel_requested` state. Scene loading remains responsive to this read but refuses cancellation until it has a rollback transaction. The full SceneBuilder Compile path is cooperatively cancelable; standalone MakeGame/AIMap/Details/HOM/SOM jobs remain non-cancelable until their file stages are transactional |
| `xfined_property_inspector` | search/filter the selection or world property tree, expand/collapse all, and open/close the corresponding panel |
| `xfined_content_browser_navigation` | inspect or drive Content Browser Back/Forward/Up/Home, source/category/folder breadcrumbs and process-session Favorites |
| `xfined_undo` | undo the last scene operation |
| `xfined_redo` | redo the last undone scene operation |
| `xfined_quest_catalog` | the quest kind catalog (NQ, see below): every trigger / main action / extra action / condition kind with `group`, `title`, `desc`, `use`, `params` (name, type, required, default, min/max, enum), `pins`, `wait`/`once`, `event`; plus catalog `version` and `source` (`game` = read from the linked install, `bundled` = the editor's fallback copy). Read it before writing a quest |
| `xfined_quest_list` | every graph document of the project - `*.nqasset` (quests) and `*.behasset` (behaviour graphs): path, id, title, node count, errors/warnings, open/dirty (unparsable files: `readable:false` + the parse error), and `kind`: `quest` or `behaviour` |
| **behaviour graphs** | A `.behasset` (header `beh = 1`) is a graph that drives NPCs instead of tracking the player (NQ_FORMAT.md par. 10) - its own asset, not a quest with a flag: the EXTENSION is the kind, there is no `type` key, and the header key has to agree with the extension (`E001`). It declares **roles** that its caller binds with `behaviour.run`; its steps are `npc.goto`, `npc.hold`, `npc.anim`, `npc.follow`, `npc.attack`, `npc.stance` and `npc.release`, and every step takes `who` = one role name or a list of them. `npc.goto` / `npc.hold` take a **destination**: `{ path = }` (a way path, the best one), `{ ref = }` (something the caller spawned - the step waits for it, follows it, lets go when it is gone), `{ story = }`, `{ role = }`, or a place (`{ smart = }`, `{ restrictor = }`, `{ pos = }`, `{ actor = true }`). `behaviour.run` has THREE pins and no wait flag - the wiring is the mode: `started` fires at once, then exactly one of `finished` (the graph ended by itself) or `aborted` (stopped, the cast died, `flow.end status = failed`). A behaviour graph sees the refs of the quest that runs it, so a ref it never creates is legal there. The same `xfined_quest_*` tools read and write both kinds - `xfined_quest_new` makes a behaviour graph when the path ends in `.behasset` - and `xfined_quest_lookup` answers `behaviour_id`, `role` (needs `path`) and `way_path` (the open scene). A kind used in the wrong document is `E023`; a `who` naming no role is `E024`; a graph that runs itself is `E113`. |
| `xfined_quest_new` | create a quest from the minimal template (`trigger.start -> flow.end`, id = file name) at a project-relative `path` and open it; refuses to overwrite |
| `xfined_quest_open` / `xfined_quest_close` | open a quest document (validates, lays out nodes without `pos`) / close it (`discard:true` drops unsaved edits, otherwise a dirty document answers `unsaved`) |
| `xfined_quest_get` | the quest as text: `lua` (canonical file text), `outline` (one line per node in flow order + the problems), `problems` `[{code,severity,node,slot,message}]`, counts. Works for files that are not open |
| `xfined_quest_write` | replace a quest with the whole file text `lua`. Open document: replaced in memory with an undo snapshot (`saved:false`, then `xfined_quest_save`). Closed or new file: written to disk canonically when it parses (`saved:true`). Validation problems are returned but never block the write - only the build refuses errors |
| `xfined_quest_apply` | point edits: `ops` = Lua text of a list of operations (`add_node`, `set_node`, `rename_node`, `remove_node`, `connect`, `disconnect`, `add_action`, `set_action`, `move_action`, `remove_action`, `set_quest`, `set_pos`; 1-based indexes), all-or-nothing, one undo step; opens the document when needed and leaves it dirty |
| `xfined_quest_undo` / `xfined_quest_redo` | the quest document's own undo ring (200 steps; the scene undo never touches quests) |
| `xfined_quest_validate` | problems of a quest (open document or file): `E001-E050` block the build, `W011-W070` warn |
| `xfined_quest_save` / `xfined_quest_reload` | write the open document canonically (`force:true` overrides "modified externally") / re-read it from disk |
| `xfined_quest_layout` | deterministic pin-aware top-down layout; separates complete nested branch bounds and exclusive trigger trees, while shared descendants become merge blocks between their incoming branches (`all:false` = only nodes without `pos`); returns the positions |
| `xfined_quest_view` | open/focus the graph tab and set the view (`frame:"all"|<node>`, `zoom_level` 0..9, `cx`/`cy` which override `frame`, `select:"<node id>"`/`"a,b"`/`"none"` to set the graph selection - the inspector follows it, so this is what puts a node in it, `slot:"enter:0"`/`"exit:1"` to open that action of the selected node in the inspector, `"none"` to clear; `action_split`/`vars_split` 0.1..0.9 move the inspector's two splitters, so a form taller than its pane fits a screenshot - they are the author's saved preference and the reply reports both, put them back). The reply also carries `node_scroll` / `action_scroll` as `[y, max]` of the last frame drawn: `max` above `y` means there is more below, and `scroll` is how to get there. Raw editor replies expose `pending:true` while first-frame geometry is deferred; the bridge polls a read-only view until the next draw resolves it. Answers with the resulting `zoom_level`/`center`/`selected`/`slot`; follow with `xfined_screenshot_editor` |
| `xfined_quest_edit` | every graph edit the canvas offers, without a mouse. `op` = `connect` (`from`+`pin`+`to`), `disconnect` (`from`, plus `pin`/`to` to name one link - omit them to drop a whole pin or node), `bypass` (the node stays put and the chain closes over it: A->B->C becomes A->C), `delete`, `duplicate`, `connect_nearest`, `select_all`. `nodes` (comma separated) picks what the op acts on instead of the current selection. The reply carries the outline, so the result is readable without a screenshot |
| `xfined_quest_find` | graph-wide node search across ids, kind ids/titles, parameters, conditions, actions, links and comments. `action` = `get`/`set`/`next`/`previous`/`clear`, `query`, `select`, `limit`; `get` with `query` atomically replaces the current query without navigation. Terms are ANDed, quoted phrases stay together, `-term` excludes. Next/previous frames the active result and the response includes deterministic file-order results plus the exact view state |
| `xfined_quest_project_find` | read-only search through every sorted `*.nqasset` without opening tabs. `query` uses the same AND/quoted/`-term`/UTF-8 case-folding rules as `xfined_quest_find`; `limit` defaults to 200. Dirty open documents overlay their disk entry. Every result says `source:"disk"|"memory"`; `generation` and `fingerprint` identify the exact indexed view, while `complete:false` + `diagnostics[]` names unreadable or partial files |
| `xfined_quest_references` | exact catalog-typed references: `type`, `value`, optional `path` scope and `limit`. Use `path` for document-scoped ids such as `task_id`. The walker includes nested `cond_list`/`cases_cond` and never guesses from parameter names. Unknown kinds/params, malformed values, Lua, unreadable or partial files produce `complete:false` with exact path/node/slot diagnostics; results report `source:"disk"|"memory"` plus index/catalog generations |
| `xfined_quest_rename_task` | safe intra-document task rename: `path`, `from`, `to`, **required** `ack_runtime_identity:true`. Changes the declaration and only proven catalog-typed `task_id` parameters (including nested conditions) as one document undo step. Any unknown/Lua/malformed/partial region refuses before an undo snapshot. The acknowledgement is mandatory because task ids persist in runtime/save state and existing state is not migrated |
| `xfined_quest_bookmarks` | transient node bookmarks: `action` = `get`/`add`/`remove`/`toggle`/`next`/`previous`/`clear`, optional `node` (otherwise the sole selection). They never touch `.nqasset`, dirty state or undo |
| `xfined_quest_history` | read/control transient graph navigation history: `action` = `get`/`back`/`forward`/`clear`; every state includes center, zoom, selection and inspector slot |
| `xfined_quest_minimap` | read/control the interactive transient minimap: `action` = `get`/`show`/`hide`/`toggle`; returns visibility, graph/viewport bounds and the exact view state |
| `xfined_quest_lookup` | search picker data: `type` = `item_section`, `squad_section`, `level`, `smart`, `story_id`, `profile`, `community`, `info`, `spot_type` (from the linked game) or `task_id`, `var_name`, `ref_name`, `node_id`, `role` (need `path`), `quest_id` (project quests), `behaviour_id` (project behaviour graphs), `character_id` (project characters - what a `spawn.npc` row names; a template says so in `extra`), `dialog_id` (dialogs of the linked game and the project, captioned with their first line), `character_icon` (portraits; `extra` = `sheet|x|y|w|h`, `project:` in front for the project's), `rank` (the linked game's ranks, `extra` = the value a rank starts at), `way_path` (way paths of the open scene); `query` substring, `limit` |
| `xfined_quest_check_all` | the build gate on its own: validates every quest of the project exactly like `xfined_mod_export` does (`passed`, counts, log lines) |

Everything that writes is clamped to the project folder: the linked game install
and the shared SDK library are sources you copy **out of**, never into.

`xfined_recovery` has no path or force argument. A snapshot preserves the current
scene file name, recents and dirty state. Restore first validates and loads the
recorded source scene, then overlays the validated editable snapshot and leaves
the result dirty; it never writes the source. Discard only tombstones and removes
recovery generations. The bridge briefly polls an accepted restore/discard by its
`operation_id`; if the draw has not completed, it returns truthful `pending:true`
for a later `action=get` poll.

While a guarded main-thread operation is running, the editor extracts only
`xfined_progress` requests from the MCP queue. Scene/object mutations remain queued
until the operation ends, so a progress poll cannot observe or modify a partially
loaded scene. Normal MCP dispatch also executes one queued request per frame.

### World Outliner UI

The Outliner is a virtualized table, so large object classes only submit visible
rows. Its Visible, Name, Type and Owner columns can be resized and reordered;
Name, Type and Owner can also be hidden from the header menu while Visible stays
pinned. ImGui's existing layout settings retain those choices. Owner exposes the
scene's existing group relationship without adding a
new hierarchy or changing scene serialization. Clicking Name cycles natural
A-Z, Z-A and original scene order.

Rename and Delete stay disabled for read-only tools and non-editable group
members. Rename resolves the object by its stable original name when the modal
is accepted. Delete names the object in a confirmation modal, defaults to
Cancel, and remains undoable. MCP uses the existing non-modal
`xfined_rename_object` and `xfined_delete_selected` paths.

## Quest workflow for AI (NQ quest graphs)

A quest is one `.nqasset` file = declarative Lua (`return { nq = 1, id = ...,
title = ..., activation = "auto", vars = {...}, tasks = {...}, nodes = {...} }`)
that the game interprets at runtime; the editor never compiles it. Everything is
text, no screenshots needed: coordinates (`pos`) are optional, problems come
back as codes, `xfined_quest_get` returns the canonical file plus an outline of
the graph. Full contract: `docs/NQ_ARCHITECTURE.md` (§4 format, §6 kinds, §15
codes), examples: `docs/nq/examples/*.nqasset`.

1. `xfined_quest_catalog` — learn the kinds: node kinds (`use` main/trigger)
   with their `params` and `pins`, extra actions for `on_enter`/`on_exit`,
   conditions for `cond` (`event = true` ones only in `trigger.when` /
   `wait.when` / `wait.any`).
2. `xfined_quest_write` with the whole file (or `xfined_quest_new` + `xfined_quest_apply`).
   Ids are `[a-z0-9_]`; a node is `{ id, kind, params, cond, on_enter, on_exit, out = { pin = "id" | { "id", ... } } }`;
   phrases (`dialog.npc_phrase` / `dialog.actor_phrase`) hang off a `dialog.topic`
   through `next` and must alternate speakers; refs to things the quest spawns
   are `{ ref = "name" }`, world objects `{ story = "story_id" }` (look them up
   with `xfined_quest_lookup`).
3. Read the answer: `errors` must be 0 (`W`-codes are hints: `W060` = value not
   found in the linked game, `W011` = a phrase with only conditional replies).
   Fix and write again; `xfined_quest_get` shows the canonical text and outline.
4. `xfined_quest_check_all`, then `xfined_mod_export` — the export refuses with
   `quest graph errors: N (see log)` while any quest has an `E` problem; the
   `.nqasset` files ship into `<game>\modules\<id>\` unchanged.

## Releasing a mod

A release is what players install and update from the game's Mods menu: the
descriptor `<id>.update.ltx`, the file index `<id>-<version>.files` and one or
more package zips, attached to a GitHub release. Every release is complete; the
index tells the game where each file's bytes sit inside a package, so it
downloads only what differs from the installed module - there is no patch to
build. The format is a frozen contract with the game
(`docs/dead-air/MOD_UPDATES.md` in the game repository).

1. `xfined_mod_set_manifest` — `version` (1-4 numbers separated by dots, higher
   than the last release, otherwise players are offered nothing), `github`
   (`owner/repo` of the repository that will carry the releases), optionally
   `author`, `description` and `website` (AP-PRO or ModDB only).
2. `xfined_mod_publish` — packages (exactly as `xfined_mod_package` does, into
   `<project>/releases/<version>/`) and publishes the result to that repository:
   the assets go into a draft, every stored asset is checked against what was
   packaged (name, size, SHA-256), and only then the draft becomes the **latest**
   release. `served=true` in the answer means the address the game itself asks
   already returns the new descriptor. A tag that is already published is
   refused - raise the version; an interrupted attempt leaves a draft that the
   next one reuses. Without `tag` the editor picks `v<version>`, or
   `v<version>-r2`, `-r3`, ... when this project deleted that tag within the last hour:
   a release deleted and published again under the SAME tag keeps its download
   address, and GitHub goes on serving the deleted files from that address for
   minutes (`served=false` with a warning that says so). A fresh tag is a fresh
   address.
3. `xfined_mod_package` alone stops after step one of that and returns the
   descriptor path, the index and the packages with their sizes and SHA-256 -
   for a release that is attached by hand: every file under its own name, the
   zip byte for byte as written, since the index addresses it by offset,
   published as the **latest** release. The package also serves as the manual
   install: extracted into the game folder it lands in `modules/<id>/`.

The editor takes the account from `XFINED_GITHUB_TOKEN` / `GH_TOKEN` /
`GITHUB_TOKEN`, from a saved token (Windows Credential Manager) or from the
GitHub CLI (`gh auth login`), in that order; `action=account` tells which one is
in effect. A token is never part of an answer or a log.
The upload runs on a worker, so the editor stays responsive: `action=start`
answers as soon as the release is packaged, the bridge then follows
`action=status` for up to 150 s (`wait=false` returns at once), and
`action=cancel` stops the job before anything is published.

`xfined_mod_releases` lists what is already out; `xfined_mod_release_edit`
changes the title and the notes of one release and nothing else - not its files,
not its tag, not which release is the latest (GitHub's default for an edit would
move that mark, so the editor always states where it is). `xfined_mod_release_delete`
deletes one release together with its git tag - the release first, the tag
second, because a published release whose tag vanishes first turns into a draft
instead of going away. It is the one command here that destroys something nobody
can restore: GitHub keeps no copy of a release, its files or its tag. The editor
asks nothing on the MCP path, ever - an unattended agent runs straight through.
Whether to ask is the caller's side of the line: the tool is a separate one and
carries `destructiveHint`, so a client can put its own permission prompt on
exactly this call; an agent calls it when the user explicitly asked for the
deletion and asks in its conversation when they did not. Deleting the latest
release makes GitHub serve the newest remaining one; a player who already has the
deleted version keeps it - and when that version is published again the game
compares content, not numbers, and offers it to that player as a fix. When GitHub refuses to delete the tag (a tag ruleset), the answer
says `tag_kept=true` and quotes GitHub in `note`.

Signing in over MCP: `action=save_token` stores the `token` it is given in the
Windows Credential Manager (a fine-grained token needs **Contents: Read and
write** on the repository) - the same thing the window's Save Token does.
`action=sign_in` starts GitHub's device sign-in and returns `user_code` and
`verification_uri`; once the code is confirmed on GitHub the token goes straight
into the Credential Manager, and `action=sign_in_status` reports
`starting`/`waiting`/`done`/`failed`. `action=sign_out` forgets the saved token
and leaves a GitHub CLI or environment sign-in alone. The device sign-in exists
only in a build that carries the editor's OAuth client id - `action=account`
says so in `device_sign_in`.

Packaging compresses at the highest level, so a large mod can outlast the
request timeout (three minutes by default; raise `XFINED_MCP_REQUEST_TIMEOUT_MS`
in the environment of both the editor and the bridge). The work continues
either way: `xfined_progress` reports it, and `action=cancel` stops it with the
output folder untouched.

## Raw protocol (if you don't want the bridge)

One JSON object per line in, one per line out:

```
> {"cmd":"ping"}
< {"ok":true,"name":"XFined Editor","version":"1.0"}

> {"cmd":"state"}
< {"ok":true,"project_active":true,"project":"D:/Mods/MyMod","project_name":"MyMod","scene":"...","fps":240}

> {"cmd":"screenshot_viewport"}
< {"ok":true,"png_base64":"iVBORw0KGgo..."}

> {"cmd":"screenshot_editor"}
< {"ok":true,"png_base64":"iVBORw0KGgo..."}
```

Errors come back as `{"ok":false,"error":"..."}`. Requests execute on the
editor's main thread; a busy editor can take up to three minutes before the
endpoint returns a timeout. The server accepts up to 16 concurrent clients and
queues at most 64 main-thread requests; each request buffer is capped at 1 MiB.

## Prompt block for the agent

> The XFined Editor (S.T.A.L.K.E.R. X-Ray SDK level editor) is running locally
> with an MCP endpoint. Use `xfined_state` to learn which project and scene are
> open, and `xfined_screenshot_editor` / `xfined_screenshot_viewport` to see
> the editor with your own eyes before and after any change you make. The
> screenshots work even when the editor window is covered — always verify the
> actual UI state visually instead of assuming. A screenshot is one call: the
> editor draws the frames it needs first, so the picture always shows the state
> after the commands you sent before it.
>
> If you start the editor yourself, launch it as
> `LevelEditor.exe -project "<project folder>"` so it opens straight into that
> project instead of waiting on the project picker. On a running editor,
> `xfined_open_project` switches projects.
>
> The project folder is the only writable place. The shared SDK library
> ("Editor Content") and the linked game install ("DARF Content") are read-only
> sources: pull assets out of them with `xfined_content_browser_copy`, which
> puts each one where the engine expects it.
>
> Quests are `.nqasset` files (declarative Lua graphs, see the "Quest workflow"
> section): `xfined_quest_catalog` for the kinds, `xfined_quest_write` /
> `xfined_quest_apply` to author, `xfined_quest_get` for the canonical text and
> outline, `xfined_quest_validate` / `xfined_quest_check_all` before
> `xfined_mod_export`. Never draw the graph from screenshots - the outline and
> the problem codes are the working view.
