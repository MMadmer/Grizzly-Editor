# NQ — Node Quest Graph. Architecture of the node-based quest graph

This document describes how the node-graph quest system lands in Grizzly Editor
and Dead Air Refined: the `.nqasset` format, the action catalog, the execution
model, the engine changes, the editor UI, MCP, export, tests and the phased
rollout plan. It is a working implementation guide: every decision is made, no
questions stay open (the decision register and its reasons are §18). All code
anchors follow the current trees `D:\Games\Grizzly-Editor` (the editor) and
`D:\Games\Dead Air\DeadAir-x64` (the engine); the unpacked game scripts are
`D:\Games\Dead Air\_analysis\original\scripts` (read-only, the reference).

Status: **implemented, covered by runtime regression tests and the editor MCP
build gate.** Until the editor's first release the format may change freely
(the rule from `CLAUDE.md`); after the release §2 freezes.

---

## 0. Decision summary (TL;DR)

1. **1 asset = 1 quest = 1 `.nqasset` file.** The file is valid Lua, but
   strictly declarative: `return { ... }` (one table, no code outside
   strings). Read by a human, an AI, the editor and the game the same way.
   **AI-first**: an AI creates and edits a quest as text, no screenshots — the
   kind schema is served by `quest_catalog`, node coordinates are optional
   (the editor lays them out itself), problems come back as text with codes and
   positions (`asset.check`), `quest_open` adds a textual graph outline (§4.5).
2. **The editor does NOT compile the graph into game Lua.** On build it only
   validates and puts the file into the module as is. All logic is executed by
   the **NQ runtime inside the game** (Dead Air Refined Lua scripts + ~200
   lines of C++). A game update fixes every mod at once — exactly what was
   required.
3. **Three pools of "what can be picked"** — triggers/main actions/extra
   actions/conditions — are described by the **catalog**
   (`configs\nq\catalog.ltx` + `configs\nq\kinds\*.ltx`), supplied by the game
   and scanned by the editor from the linked game (like modes) with a bundled
   fallback. Main actions come strictly from the catalog. Extra actions and
   conditions come from the catalog **plus** `lua` (custom code).
4. **The execution model is tokens** (the set of active nodes), a
   deterministic order, a deferred transition queue, `pcall` on every
   boundary: a broken quest brings down neither the game nor its neighbors.
5. **Dialogs** — phrases are nodes; the runtime assembles a `CPhraseDialog`
   from them through the engine's `init_func` path. Small engine changes are
   needed: registering "virtual" dialogs and hooking NPC dialog distribution
   (§7, §12.3).
6. **Quest state** — one `xms.save_data("xms.nq", …)` blob in `.scov` (the
   frozen save contract stays untouched), with a mandatory re-stage after
   load.
7. **Texts** — UTF-8 in the asset, converted to cp1251 on the game side at
   load (the engine's `translate()` returns the raw key, no string table
   needed).
8. **The editor**: a new asset type in the content browser (**Add > Quest**,
   the toolbar's [+ Add] or the grid's menu), a major document tab with an infinite canvas (RMB pan, 10
   zoom levels, top-down flow, nodes as cards that list their actions), the
   Graph panel on the left and the Details panel on the right, its own undo,
   full MCP (`quest_*`), validation = the build gate.
9. **Object references**: string story ids (CoC-style `[story_object]`),
   `ref` names for what the quest itself spawned, profiles/communities for
   "any NPC".
10. **PDA tasks** — the runtime drives `CGameTask` directly, bypassing
    `task_manager` (which reads its ltx once at script load and does not
    extend).
11. **No XML/ltx stubs from the editor**: info portions are unnecessary (the
    engine does not crash on unknown ids, and quest flags live in its
    variables), dialogs and tasks are registered by the runtime.
12. **Rollout is 6 phases**; the game and the editor are built in parallel by
    subagents after the contract is fixed (§17).
13. **NPC behaviour is its own asset on the same graph engine** (§19): a
    behaviour graph (`.behasset`) drives NPCs through **roles** that its caller
    binds, so one file serves any cast. It reaches the NPCs by generating logic
    text for the stock schemes and handing it over the way a smart terrain does
    — no second movement or combat system; where the schemes could not go (a
    destination nobody drew a path to) the ENGINE was taught to, not the
    scripts. A call has three exits: `started`, then `finished` or `aborted`.

---

## 1. Terms

| Term | Meaning |
|---|---|
| **Quest** | one `.nqasset`; in game its uid is `<module id>.<quest id>` |
| **Node** | a graph element: `id`, `kind`, `params`, `cond`, `on_enter[]`, `on_exit[]`, `out{}` |
| **Kind** | a catalog entry: what the node/action/condition does, its parameters and pins |
| **Trigger** | a root node with no input (the "T" oval on the canvas); armed while the quest is active |
| **Main action (main)** | a node kind; strictly from the catalog; instant or waiting |
| **Extra action (extra)** | an instant operation in `on_enter`/`on_exit`; from the catalog or `lua` |
| **Condition (cond)** | a predicate from the catalog or `lua`; used by triggers, branches, phrases |
| **Pin** | a named node output (`next`, `done`, `yes`, …); a pin has 0..N targets |
| **Token** | "the quest currently stands on this node"; at most one token per node |
| **Ref** | the name under which the quest remembered an object it created (a squad, item, anchor) |
| **NQ runtime** | the `xms_nq*.script` set in the Dead Air Refined loose layer |
| **Catalog** | `configs\nq\catalog.ltx` (+ `configs\nq\kinds\*.ltx`) — the kind registry |

---

## 2. The NQ contract: what freezes at release

After the editor's first public release the following freeze (per the game's
`PROJECT_RULES.md` §1 and the editor's `CLAUDE.md`):

1. **The `.nqasset` format** (§4) — the `nq = <version>` field; the game
   supports every released version; migrations live on the game side
   (`xms_nq_load.migrate`).
2. **Catalog semantics** (§6): a kind, once released, never changes meaning;
   new kinds and new optional parameters are added freely; removing a kind is
   forbidden (the implementation stays, the editor marks it "deprecated").
3. **The execution model** (§5) — the lifecycle, the order, `once`, the
   `flow.end` behavior, the "a pin activates all its targets" rule.
4. **The saved-state schema** (§11) — additive changes only, migrated by the
   `v` field.

Until the release all of this changes freely; old revisions are unsupported.
The game's save compatibility (`.scop`/`.scoc` byte-for-byte, `.scov` chunks
only) is never affected in any case: NQ writes exclusively through
`xms.save_data`.

---

## 3. Files and folders

### 3.1 The editor project and the module in the game

```
<project>\
  quests\wolf_debt.nqasset          ; the asset (may live in any project folder
                                    ;  except source-only rawdata\ and Content\)
<game>\modules\<id>\
  mod.ltx                           ; unchanged; quests are found by scanning
  quests\wolf_debt.nqasset          ; export copies the file as is
```

The runtime scans **the whole module root recursively** for `*.nqasset`
(the `xms_native_list_files` native, §12.3), so the file's location inside the
module does not matter. Export (`EditorMod::Export`,
`cpp-editor-last:Source/Editors/XrECore/Editor/EditorModManifest.cpp:843-1063`) already copies
every non-editor-only folder whole (`:939-960`) — `.nqasset` needs no separate
export hook, only the **validation gate** (§13.10).

### 3.2 Game (engine repo)

```
packaging\dead-air-x64\compatibility\gamedata\
  scripts\xms_nq.script            ; core: API, on_game_start, tick, dispatcher, state
  scripts\xms_nq_load.script       ; .nqasset discovery, parse, validation, canonicalization, migrations
  scripts\xms_nq_kinds.script      ; catalog kind implementations (main / extra / cond)
  scripts\xms_nq_dialog.script     ; CPhraseDialog assembly, engine callbacks
  scripts\xms_nq_task.script       ; CGameTask wrappers
  scripts\xms_nq_util.script       ; cp1251, reference resolve, places, time, serialization
  scripts\xms_nq_console.script    ; the `nq …` console
  configs\nq\catalog.ltx           ; the core catalog
  configs\nq\kinds\                ; catalog extensions (modules put their *.ltx here)
src\xrGame\xms_game.cpp            ; new natives (§12.3)
src\xrGame\PhraseDialog.{h,cpp}    ; virtual dialogs (§12.3)
src\xrGame\actor_communication.cpp ; dialog distribution hook (§12.3)
src\xrGame\console_commands.cpp    ; the `nq` command
docs\dead-air\NQ_RUNTIME.md        ; runtime specification (split out of this doc in phase 0)
```

No `script.ltx` change is needed: `axr_main.on_game_start()` iterates every root
`*.script` and calls its `on_game_start()` (`axr_main.script:198-232`), and the
`_G[name]` access itself autoloads the file.

### 3.3 Editor

The Rust editor's quest code is `gz-nq` (the model, reading and writing through `gz-luadata`, the catalog, the validator, the pickers, the documents and their commands) and `gz-ui-graph` (the tab, the canvas, the panels). The C++ files this section names are the tree at the tag `cpp-editor-last`.

```
Source\Editors\XrECore\Editor\Nq\        ; model, IO, catalog, validator, document, MCP, export gate
  NqAsset.h/.cpp        NqLua.h/.cpp        NqCatalog.h/.cpp
  NqValidate.h/.cpp     NqPickers.h/.cpp    NqDoc.h/.cpp
  NqExport.cpp          NqMcp.cpp
Source\Editors\LevelEditor\UI\QuestGraph\ ; the document tab, canvas, Graph and Details panels
  UIQuestGraph.h/.cpp   ; UIQuestGraph (document registry, Open/Update/Close) + UIQuestGraphWindow (per-document tab)
  NqCanvas.h/.cpp       ; the canvas, and NqLook: a kind's icon, family colour and title as the document says it
  NqInspector.h/.cpp    ; the Graph panel and the Details panel
gamedata\configs\nq\catalog.ltx           ; bundled catalog fallback (copy of the game's)
docs\NQ_ARCHITECTURE.md                   ; this document
docs\nq\examples\*.nqasset                ; reference quests (phase 0)
tools\mcp\grizzly_mcp.py, tools\mcp\AI_SETUP.md ; new quest_* tools
```

Layering: everything that is not UI lives in `XrECore` (so `EditorMod::Export`
can call the validator); UI lives in `LevelEditor`.

---

## 4. The `.nqasset` format

### 4.1 Rules

- Encoding UTF-8 without BOM, `\n`. Extension `.nqasset`.
- The content is **one `return { … }`**. Allowed: tables, strings (`"…"`, `[[…]]`),
  numbers, `true/false`, comments. Forbidden: function calls, operators, `local`,
  references to globals. The game loads the chunk via `loadstring` + `setfenv(chunk, {})`
  (an empty environment) — any "code" fails at load and the quest is rejected with
  diagnostics; the editor loads the same way through the LuaJIT C API (§13.3).
- Custom Lua is allowed **only** inside the string field `params.code` of a
  `lua`-kind action/condition.
- Table keys are identifiers (`id = …`), not `["id"] = …` (both variants are read;
  a key the editor adds is written as an identifier). The exception is keys that are Lua reserved words
  (the `else` pin of `flow.branch`, the `["not"]` field of conditions): those are
  written as `["else"] = …`.
- On save the editor **writes only what changed** (§13.3, `docs/ASSETS.md`
  section 3): comments, key order and layout written by hand or by an AI stay
  byte for byte, and no comment block is generated.
- A node's `pos` is optional: nodes without coordinates are laid out automatically
  on open (§4.5, item 2).

### 4.2 Schema

```lua
return {
  nq        = 1,                 -- format version (required)
  id        = "wolf_debt",       -- [a-z0-9_], unique in the module; in-game uid = "<module>.<id>"
  title     = "Долг вежливости", -- text (see types), for the journal/debug/PDA fallback
  activation= "auto",            -- "auto" (on start/install) | "manual" (via the quest.activate action)
  vars      = { <name> = <default>, … },        -- quest variables (bool/number/string)
  tasks     = { <task id> = { title=<text>, descr=<text>, type="additional"|"storyline",
                              target=<target_ref>|nil, icon=<string>|nil,
                              auto_complete=<bool>|nil,   -- default true
                              -- task steps: each has its own text and its OWN map spot
                              objectives = { { id=<ident>, title=<text>, descr=<text>,
                                               target=<target_ref>|nil,
                                               visible=<bool>|nil }, … } }, … },
  nodes     = { <node>, <node>, … },            -- order = file order (matters for determinism)
}

<node> = {
  id      = "meet_wolf",          -- [a-z0-9_], unique in the quest
  kind    = "dialog.topic",       -- a catalog kind, use ⊇ {main} or {trigger}
  once    = true,                 -- optional; the default depends on the kind
  params  = { … },                -- kind parameters
  cond    = { <cond>, … },        -- optional; AND list (for triggers, phrases, waits)
  on_enter= { <action>, … },      -- optional; run in order before the main action
  on_exit = { <action>, … },      -- optional; after the main action completes
  out     = { <pin> = "<node id>" | { "<id>", "<id>" }, … }, -- pins from the kind's catalog entry
  pos     = { x, y },             -- editor-only; canvas coordinates at zoom 1.0
  comment = "…",                  -- editor-only
}

<action> = { kind = "item.give", params = { section = "wpn_pm", count = 1 } }
<action> = { kind = "lua", params = { code = [[ nq.news("Привет") ]] } }
<cond>   = { kind = "var", params = { name = "greeted", op = "eq", value = false }, ["not"] = false }
<cond>   = { kind = "lua", params = { code = [[ return nq.actor:money() > 500 ]] } }
```

The universal rule: **every catalog entity is `{ kind, params }`**; conditions
additionally take `["not"] = true` to invert the result.

### 4.3 Parameter value types

| Type | Form in the file | Notes |
|---|---|---|
| `text` | `"string"` or `{ rus = "…", eng = "…" }` | UTF-8; placeholders `{var:name}`, `{actor}` |
| `string`, `int`, `float`, `bool`, `enum(a\|b)` | as is | |
| `duration` | `{ seconds = N }` (real) or `{ game_hours = N }` / `{ game_minutes = N }` | |
| `npc_ref` | `{ story = "esc_m_trader" }` \| `{ ref = "guard" }` \| `{ profile = "sim_default_stalker_1" }` \| `{ community = "stalker", level = "l01_escape" }` | resolve order §8 |
| `target_ref` | `npc_ref` \| `{ ref = "boars" }` (squad/object) \| `{ smart = "esc_smart_terrain_2_12" }` | for task targets/map spots |
| `place` | `{ level = "l01_escape", pos = {x,y,z}, radius = 5 }` \| `{ restrictor = "zone_name" }` \| `{ smart = "…" }` \| `{ ref = "anchor" }` \| `{ actor = true, distance = 2 }` | `ref` = where an object this quest created stands; `actor` = that many metres in front of the player, resolved when the action runs (spawn points only — E006 on reach/teleport/`actor_in_place`) |
| `item_list` | `{ { section = "bread", count = 2 }, { item = { ref = "prize" } } }` | rows of two shapes: section×N or a specific object; a section appears once |
| `loot_spawn` | exactly one key: `{ place = <place> }` \| `{ into = <object_ref> }` \| `{ stash = { pick = random\|nearest\|farthest, state = any\|full\|empty, within = R, beyond = R, scatter = bool } }` \| `{ npc = <npc_ref> }` \| `{ corpse = { pick, within, beyond } }` \| `{ new_corpse = { section = <spawn_section>, place = <place> } }` | where `objective.fetch` (`spawn`) and `item.spawn` (`where`) put the loot they create. Radii are measured from the player; nothing within the radius → the nearest match, nothing outside → the farthest one. `state`: full — the container holds something, empty — it is empty. `scatter` — each item goes to its own random stash. `new_corpse` — the corpse is created with the loot inside and drops as soon as it goes online |
| `spawn_section` | `"inventory_box"` | any section with `$spawn` (anything placeable in the world); the editor provides a picker |
| `character_id` | `"valera"` | a character of the project, by the `id` of its `.npcasset`; the runtime turns it into the section `<module>_<id>` the game makes of it. `"<module>.<id>"` names a character of another module |
| `npc_list` | `{ { character = "valera", ref = "valera" }, { section = "…", unique = true } }` | rows of `spawn.npc`: exactly one of `character` \| `section`, plus `ref`, `unique` (default: on for a character, off for a section), `dead`, `yaw` |
| `objective_id` | `"bread"` | a step of the task named by the neighboring `task` parameter. The editor provides a dropdown of that task's steps |
| `object_ref`, `squad_ref` | `{ story = "esc_m_trader" }` \| `{ ref = "stash" }` | ONE specific world object. Narrower than `npc_ref`: `profile`/`community` name a kind of creature, not a thing you can loot, remember under a ref or count the death of |
| `spawn_spec` | `{ section = "simulation_boar", smart = "…", place = <place>, restrictor = "…", spread = 1, ref = "boars", hold = true }` | for `spawn.squad` and `objective.kill` targets. Needs a section plus a smart OR a place; an empty `smart` = not set. `hold` keeps the squad where it was put: on the smart if the author picked one, and if the author gave a place, the squad binds to no smart at all (otherwise the leash would drag it out of that very restrictor) |
| `item_section`, `squad_section`, `level`, `smart`, `story_id`, `profile`, `community`, `info`, `var_name`, `task_id`, `quest_id` (`"<module>.<quest>"` or `"<quest>"` = own module), `ref_name`, `signal_name`, `spot_type`, `relation` (`enemy\|neutral\|friend`) | strings | the editor provides pickers (§13.6) |
| `lua` | `[[ code ]]` | see §14 |
| `cases` | `{ { name = "yes", cond = { … } }, … }` / `{ { name = "a", weight = 3 }, … }` | names = pins |

**Objectives.** The engine has carried them from day one: `CGameTask` holds a
vector of `SGameTaskObjective`, each with its own title, description, icon and
map spot (`set_map_location` / `set_map_object_id`). Index 0 is the task itself,
steps are numbered from one in declaration order. So "collect X, collect Y, report
to Z" is ONE PDA task with three rows and, if needed, three map spots — not three
quests. Step state lives in `qs.objectives[<task>][<objective>]`, survives saves
and is read by the `objective_status` condition, so the graph can branch on "bread
found, medkit not yet". Completing a step (`task.objective_complete`) leaves the
task itself active exactly until the last step: a task has `auto_complete` (bool,
default **true**) — once ALL its steps become `completed`, the task completes
itself, and no manual `task.complete` is needed after the last step. It is written
to the asset only when turned off. A failed step does not satisfy the rule: the
task stays open and the runtime notes it once in the log. The rule hangs only on
`task.objective_complete` — the load path (`restore_objectives`) writes state into
the engine directly and does not reopen the task — and runs AFTER the node
unwinds (the queue, op `call`), otherwise this very node's `on_exit` actions
would hit an already closed task.

**Step visibility.** `visible = false` in the declaration hides a step: it works
as usual (it can be completed, `objective_status` sees it) but takes no row in
the PDA — like Collapsed in UE UI. Showing/hiding mid-quest is
`task.set_objective_visible`. Whatever the quest changed by hand is remembered in
`qs.ovis[<task>][<objective>]` and survives saves; the engine serializes a task
WITHOUT its steps (`CGameTask::save` writes only itself), so after a load the
runtime rebuilds the steps from the declaration and restores their statuses and
visibility (`xms_nq_task.script`, `restore_objectives`).

### 4.4 Reference example

Stalker/smart ids are illustrative.

```lua
-- Долг вежливости: Sidorovich asks to wipe out the boars near the rookie village.
return {
  nq = 1,
  id = "wolf_debt",
  title = "Долг вежливости",
  activation = "auto",
  vars = { agreed = false },
  tasks = {
    kill_boars = { title = "Отстрел кабанов", descr = "Сидорович просил перебить кабанов у деревни новичков.",
                   type = "additional", target = { ref = "boars" } },
  },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "meet" }, pos = {0, 0} },

    { id = "meet", kind = "dialog.topic",
      params = { npc = { story = "esc_m_trader" }, text = "Слышал, у тебя проблемы с кабанами." },
      once = true,
      out = { next = "sid_1" }, pos = {0, 160} },
    { id = "sid_1", kind = "dialog.npc_phrase", params = { text = "Лезут из леса. Перебьёшь — не обижу." },
      out = { next = { "accept", "decline" } }, pos = {0, 300} },
    { id = "accept", kind = "dialog.actor_phrase", params = { text = "Помогу." },
      on_exit = { { kind = "var.set", params = { name = "agreed", value = true } },
                  { kind = "task.give", params = { task = "kill_boars" } } },
      out = { next = "kill" }, pos = {-160, 440} },
    { id = "decline", kind = "dialog.actor_phrase", params = { text = "Не моё дело." }, pos = {160, 440} },

    { id = "kill", kind = "objective.kill",
      params = { target = { spawn = { section = "simulation_boar", smart = "esc_smart_terrain_2_12", ref = "boars", hold = true } } },
      out = { done = "report" }, pos = {-160, 580} },

    { id = "report", kind = "dialog.topic",
      params = { npc = { story = "esc_m_trader" }, text = "Кабаны мертвы." },
      out = { next = "sid_thanks" }, pos = {-160, 720} },
    { id = "sid_thanks", kind = "dialog.npc_phrase", params = { text = "Молодец. Держи." },
      on_exit = { { kind = "item.give",  params = { section = "wpn_pm", count = 1 } },
                  { kind = "money.give", params = { amount = 1500 } },
                  { kind = "task.complete", params = { task = "kill_boars" } } },
      out = { next = "finish" }, pos = {-160, 860} },
    { id = "finish", kind = "flow.end", params = { status = "completed" }, pos = {-160, 1000} },
  },
}
```

### 4.5 AI-first: how an AI works with the asset without screenshots

The project requirement: an AI must read, write and edit quests **as text**,
never looking at the canvas. What guarantees this:

1. **The file is self-contained.** Everything the editor knows about a quest lives
   in `.nqasset`; nothing is stored in sidecars or editor state. Node order in the
   file is logical (top-down along the flow); the editor keeps the order the file
   has and puts a new node after the node it is linked from.
2. **Coordinates are optional.** A node without `pos` gets its position from the
   auto-layout on open, and a save writes it only once the author moved the node;
   an AI never has to write `pos`.
3. **The schema is served on demand.** `grizzly_quest_catalog` returns every kind
   with its groups, purpose (`use`), parameters (type/required/default/enum), pins
   and a one-line description — enough to write a quest from scratch. The same
   catalog lives in `docs\nq\NQ_FORMAT.md` (phase 0) and in the `AI_SETUP.md`
   prompt block.
4. **Problems come as text.** `asset.check` returns every problem of a file, a
   folder or the project with its code (§15), its sentence, its line and column and
   - where the fix is mechanical - what to write instead; an open document's
   `quest_open` / `quest_apply` answer `{code, severity, node, slot, message}`.
5. **Reading is text.** The file is the text. `quest_open` adds an
   **outline** — a linear textual description of the graph:
   ```
   quest madmer.cordon_tales.wolf_debt "Долг вежливости" (active: auto; vars: agreed; tasks: kill_boars)
   [T] start (trigger.start) -> meet
   meet (dialog.topic; npc=story:esc_m_trader; once) "Слышал, у тебя проблемы с кабанами."
       next -> sid_1
   sid_1 (dialog.npc_phrase) "Лезут из леса. Перебьёшь — не обижу."  next -> accept, decline
   accept (dialog.actor_phrase) "Помогу."  on_exit: var.set(agreed=true), task.give(kill_boars)  next -> kill
   decline (dialog.actor_phrase) "Не моё дело."  (leaf)
   kill (objective.kill; target=spawn simulation_boar@esc_smart_terrain_2_12 ref=boars)  done -> report
   …
   finish (flow.end; completed)
   problems: none            -- otherwise: "W011 sid_1: not all player replies are unconditional — …"
   ```
   `game.lookup type=quest_id` lists the project's quests; `asset.find` searches
   them.
6. **Editing — the file or by operations.** An AI usually writes the file with its
   own tools, and an open tab follows it; targeted edits of an open document go
   through `quest_apply` operations, also text (a Lua table).
7. **Names speak for themselves.** Kinds and parameters are short English
   identifiers with dotted namespaces (`dialog.topic`, `objective.kill`,
   `item.give`); texts are in the player's language.
8. **Screenshots are unnecessary but possible.** `grizzly_quest_view` +
   `grizzly_screenshot_editor` remain for visual checks, but no step of the AI
   workflow depends on them.

---

## 5. The execution model (the same for the editor simulator and the game)

### 5.1 Quest lifecycle

`inactive → active → completed | failed`. Activation:
- `activation = "auto"`: at the first safe moment (`actor_on_first_update`) of
  every game/load, if the module applies (`xms.module_applies(id)`, §12.3) and the
  state has no record of the quest. Works on an **existing save** too, right after
  installing the mod — the quest simply starts on the next load (the same
  principle as XMS spawn add-operations).
- `activation = "manual"`: via the `quest.activate` action from another quest or
  the `nq activate <uid>` console command.
- On activation: a state record is created, every `trigger.start` gets a token and
  fires, every `trigger.when` is armed.
- Terminal: `flow.end` (explicit status) **or** implicitly — when no tokens and no
  armed triggers remain (status `completed`). A quest with a `repeat=true` trigger
  stays active until an explicit `flow.end` (that is what a "background" quest is).
  `flow.end{restart=true}` resets state (tokens, `done`, `fired`, `joins`, timers;
  `vars` — back to defaults; `refs` and tasks — kept) and activates again
  (repeatable quests).
- `quest_status` for a quest that does not exist (module removed/not applicable) —
  `inactive`.

### 5.2 Node lifecycle

```
enter(node):
  if a token already exists → ignore (at most one token per node)
  if node.once and node ∈ done → ignore
  tokens[node] = { t0 = now }
  run_actions(node.on_enter)            -- synchronous, in order, each in pcall
  res = kind.begin(ctx)                 -- instant kind → {pin=…}; waiting kind → "wait"
  if res ≠ "wait" → complete(node, res.pin)

complete(node, pin):
  run_actions(node.on_exit)
  tokens[node] = nil; if node.once → done[node] = true
  for each target in node.out[pin] (in file order) → enqueue(enter(target))
```

- Waiting kinds get `kind.poll(ctx)` once per tick and/or `kind.on_event(ctx, evt)`
  per event; returning a pin name → `complete`. A waiting kind's `begin` runs the
  first check immediately: if the condition already holds (the target is already
  dead, the item is already in the inventory, the actor is already in the zone) —
  the node completes on entry.
- A pin with no targets — the branch ends. A pin activates **all** its targets
  (fork).
- `flow.join` counts arrivals against the number of incoming edges, fires on the
  full set, resets the counter.
- Cycles (a back edge) are allowed; `once` guards against re-entry.

### 5.3 Order, tick, deferred queue

- A single thread, everything in `actor_on_update` (registered in `on_game_start`)
  plus event callbacks (`actor_on_item_take`, `npc_on_death_callback`,
  `squad_on_unregister`, `actor_on_info_callback`, `actor_on_leave_dialog`,
  `on_level_changing` …, the full list — `axr_main.script:24-161`).
- Tick: the transition queue drains **every frame** (cheap, usually empty); `poll`
  nodes and triggers are polled **once per 250 ms** (`time_global()`).
- Transitions (`enter`) **always** go through the `queue`; the node's own
  `on_enter/on_exit` actions run synchronously. New transitions produced during a
  drain append to the tail — no reentrancy.
- Transitions born in dialog callbacks (a phrase was said) also go into the queue
  and drain on the next frame — otherwise a state change inside `SayPhrase` can
  filter out every continuation of the phrase and hit
  `R_ASSERT2("No available phrase to say")` (`PhraseDialog.cpp:129-130`).
- Traversal order is deterministic: quests by uid, nodes by file order, triggers
  by file order.
- Budget: while no quest is loaded, no callbacks are registered at all (0 cost in
  vanilla). With quests: O(waiting poll nodes) once per 250 ms; events go through
  an `event → {quest,node}` index.

### 5.4 Errors

- Every boundary (asset load, `begin/poll/on_event`, every action, every
  condition, dialog callbacks) — in `pcall`.
- Action error: log `! [nq] <uid>/<node>: <action#i kind>: <err>`, the action is
  skipped, the node continues (the remaining actions and the transition run).
- `begin/poll` error: the token stays, `errors[node] = err`, retried on a later
  tick no more than once per 5 s; `nq state <uid>` shows the error.
- Invalid asset: the quest does not load, log `! [nq] <file>: E0xx …`, other
  quests are unaffected; `nq list` shows `INVALID`.
- `nq debug 1` mode: every transition is mirrored to PDA news and the log.
- A game crash caused by `.nqasset` content is considered a runtime bug.

---

## 6. Kind catalog v1

### 6.1 Catalog format (`configs\nq\catalog.ltx` + `configs\nq\kinds\*.ltx`)

One kind = one section. LTX was chosen because both sides read it natively
(`ini_file` in the game, `CInifile` in the editor) and modules can extend it (a
file in `gamedata\configs\nq\kinds\` is visible through the module's VFS mount;
the editor scans the same folder through `EditorGameContent`).

```ini
[nq.catalog]                       ; catalog header (core catalog.ltx only)
version = 2                        ; catalog version; the editor and quest_catalog display it
api     = 1                        ; kind implementation contract version (begin/poll/… fields)

[nq.item.give]                     ; section = "nq." .. kind
group   = items                    ; dropdown group
title   = Give item                ; cp1251, what the author sees
use     = extra                    ; list of: trigger, main, extra, cond
params  = section, count           ; display order
section = item_section, required   ; <type>[, required][, default=<v>][, min=<v>][, max=<v>][, enum=a|b|c]
count   = int, default=1, min=1
pins    =                          ; for main/trigger: pin list; "cases" = pins come from params.cases
wait    = false                    ; main: waiting kind?
once    = false                    ; main: default node.once
impl    = core                     ; core | <module id> (who provides the implementation via xms.registry)
since   = 1                        ; catalog version the kind appeared in
```

**A semicolon starts a comment**, so it must never appear in a value: a `;`
inside `desc` cuts the description mid-sentence and the kind arrives in the
editor with truncated text and not a single complaint. In prose use a dash
instead.

`since` is the catalog version the kind APPEARED in, not the one it was last
edited in: bumping `since` on an existing kind hides it from an editor connected
to an older game and breaks already written quests. New PARAMETERS of an old kind
do not touch `since` (see `objective.fetch` in 6.3).

Extension: a module ships `gamedata\configs\nq\kinds\<mod>.ltx` and registers the
implementations in its `scripts\register.script`:
`xms.registry.get("nq.kinds"):add("<kind>", { begin=…, poll=…, on_event=…, run=…, test=… })`.
The editor shows such kinds tagged with the module. The core registers its own
kinds into the same registry from `xms_nq_kinds.script`.

A kind implementation (a table):

| Field | Purpose | Signature |
|---|---|---|
| `begin(ctx)` | main | → `{ pin = "…" }` or `"wait"` |
| `poll(ctx)` | main (wait=true) | → pin or `nil` |
| `on_event(ctx, evt)` | main (wait=true) | → pin or `nil`; `evt = { name, … }` |
| `cancel(ctx)` | main | external token removal (`flow.end`, reset) — clean up refs/subscriptions |
| `run(ctx, params)` | extra | instant |
| `test(ctx, params)` | cond | → bool |
| `save(ctx)`/`load(ctx, w)` | main | token sub-state (`tokens[node].w`) |

`ctx` = `{ quest, node, params, vars, refs, npc (dialog partner or nil), actor, now, log, emit(evt) }`.

### 6.2 Triggers (roots, `use = trigger`)

| kind | params | pins | semantics |
|---|---|---|---|
| `trigger.start` | — | `next` | fires once on quest activation |
| `trigger.when` | `repeat: bool=false`, `cooldown: duration?`; conditions — `node.cond` | `next` | fires on the **rising edge** of "all conditions true"; the arming moment counts as "false" — if the conditions already hold at quest activation, the trigger fires immediately; with `repeat` it re-arms once the conditions are false again (and `cooldown` has passed); `fired[node]` counts activations |

Event conditions (`event.*`) inside `trigger.when`/`wait.when` subscribe to
callbacks; level ones (`has_item`, …) are polled. An event condition is true
exactly at the moment of its event; the rest of the list is checked right there.

### 6.3 Main actions (`use = main`)

| kind | params | pins | wait | in-game implementation |
|---|---|---|---|---|
| `dialog.topic` | `npc: npc_ref`, `text`, `initiator: enum(actor\|npc)=actor` (+`node.cond`, `node.once` default **true**) | `next` (phrases), `done` (a leaf was said) | yes | §7 |
| `dialog.npc_phrase` | `text` (+`node.cond` = precondition) | `next` | no | NPC phrase |
| `dialog.actor_phrase` | `text` (+`node.cond`) | `next` | no | player phrase |
| `objective.kill` | `target: {story}\|{ref}\|{spawn=spawn_spec}`, `by_actor: bool=false`, `spot_task: task_id?`, `spot_objective: objective_id?`, `spot_done: bool=true` | `done` | yes | `npc_on_death_callback`/`monster_on_death_callback`/`squad_on_npc_death`/`squad_on_unregister` + polling `alife():object(id)` for offline deaths; `spawn` creates the target on entry (`SIMBOARD:create_squad`, `sim_board.script:176`) and remembers the `ref` |
| `objective.fetch` | WHAT to carry — the `items: item_list` list: a row = a section with a count OR one specific object `{ item = { ref/story } }`, row shapes mix ("THAT flash drive and 3 medkits" — one node). `section`+`count` is shorthand for a one-row list. Modifiers: `from: object_ref` — count only what was carried out of THIS container (section rows); `spawn: loot_spawn` — the node ITSELF creates the loot on entry and counts only those instances (a destroyed one is recreated; does not combine with object rows); `spot_task: task_id` (+`spot_objective`) — the node drives the TASK's marker (or its objective's) onto the nearest undelivered source, in the task's color; without it there is no spot; `spot_done: bool=true` — on closing, the node ITSELF completes the bound objective (or, without `spot_objective`, the whole task): disarmed when another node closes one of the objectives. `from`+`spawn` together are forbidden. `count` is in items: for ammo — boxes | `done` | yes | `spawn` places items via `alife():create(..., parent)`; `section` — actor inventory polling (+`actor_on_item_take`); `from` — container contents via `alife():get_children()` (works offline, which is normal for a stash), an id counts only if it was seen INSIDE the container and later found on the player; `item` — the server object's `parent_id` equals the actor |
| `task.objective_complete` / `task.objective_fail` | `task`, `objective` | — | no | marks ONE task step; the task itself stays active (`actor:set_task_state(state, tid, index)`) |
| `task.set_objective_target` | `task`, `objective`, `target?` | — | no | an INDIVIDUAL step's spot; without `target` only that spot is removed. Each step has its own (`SGameTaskObjective::change_map_location`) |
| `task.set_objective_text` | `task`, `objective`, `new_title?`, `new_descr?` | — | no | an individual step's text |
| `task.set_objective_visible` | `task`, `objective`, `visible: bool=true` | — | no | show/hide the step in the PDA; a hidden one works but takes no row |
| `objective.kill_count` | `count: int=1`, `by_actor: bool=true`, and at most one filter: `community` \| `squad: object_ref` \| `section: string` (E022 otherwise). No filter — any NPC death | `done` | yes | counts from node entry, dedup by id, the counter lives in the token (`mark_dirty`), survives saves. Offline deaths are polled only for the `squad` filter (the only one with a known candidate list) and only when `by_actor = false`: polling does not know the killer, and a player kill is always online and always delivers the callback |
| `objective.reach` | `place`, `spot_task: task_id?`, `spot_objective: objective_id?`; `spot_done: bool=true` — on closing, the node ITSELF completes the bound objective (or, without `spot_objective`, the whole task): disarmed when another node closes one of the objectives | `done` | yes | `level.name()` + distance / `db.zone_by_name[...]:inside()`; for a position a `space_restrictor` anchor is created (`alife():create`) — to carry the map spot and the task target, removed on exit |
| `wait.timer` | `duration` | `done` | yes | game time via `game.get_game_time()`; real time — a remaining-seconds counter decremented per tick |
| `wait.when` | `timeout: duration?`; conditions — `node.cond` | `done`, `timeout` | yes | like `trigger.when`, but with an input |
| `wait.any` | `cases: [{name, cond[]}]` | `<cases>` | yes | the first true case (in order) |
| `flow.branch` | `cases: [{name, cond[]}]` | `<cases>`, `else` | no | the first true one, otherwise `else` |
| `flow.random` | `cases: [{name, weight}]` | `<cases>` | no | weighted `math.random` |
| `flow.join` | — | `next` | yes | waits for arrivals on every incoming edge |
| `flow.step` | — | `next` | no | "Step": an empty main action, only `on_enter/on_exit` |
| `flow.end` | `status: enum(completed\|failed)`, `restart: bool=false` | — | no | terminal: removes all tokens (`cancel`), disarms the triggers, sets the status; `restart` — reset and re-activation |

Instant world operations (spawning, handouts, rewards) are **not** main actions
but extra ones (§6.4): "a node = a key stage", everything else is plumbing around
it. When a "stage without waiting" is needed — `flow.step`.

### 6.4 Extra actions (`use = extra`; all instant)

| kind | params | implementation (anchor) |
|---|---|---|
| `item.give` | `section`+`count=1` \| `items: item_list` (section rows; item units: for ammo — boxes, not rounds) | in a dialog context — `dialogs.relocate_item_section_to_actor` per row (UI feedback in the talk window); otherwise `alife():create` on the actor + `news_manager.relocate_item`. The two spellings are exclusive (`E022`) |
| `item.take` | `section`+`count=1` \| `"all"`, or `items: item_list` whose rows take the same `count` (items, boxes for ammo) | in a dialog — `dialogs.relocate_item_section_from_actor` per row; otherwise the inventory is walked and the objects released. The two spellings are exclusive (`E022`) |
| `item.spawn` | `section`+`count` \| `items: item_list` (section rows only, EACH with its own state), **`where: loot_spawn`** (the fetch forms) | a row carries `condition`, `ammo` (magazine, or the content of one box), `ammo_kind` (index into the weapon's `ammo_class`), `upgrades` and its own `ref`. **Unset means full**: an unwritten condition is a new item and an unwritten magazine is a full one - the engine's own zero rounds reads as a broken gift. All of it is written on the SERVER entity before it is registered; with an ONLINE host the client twin is built at once, so the entity is created unregistered (`create(..., false)`), filled, then `register`ed. Upgrade ids are checked against the section's own tree (`upgrades` -> `elements` -> `effects`) and deduped: the engine FATALs on a duplicate and crashes later on a foreign id. Several instances on a `place` spread over its radius |
| `money.give` / `money.take` | `amount` | `db.actor:give_money(±n)`; in a dialog `dialogs.relocate_money_to_actor/from_actor` (`:2025/:2031`) |
| `info.give` / `info.disable` | `info` | `db.actor:give_info_portion/disable_info_portion` — an unknown id does **not** assert (`InfoPortion.cpp:28-36`) |
| `var.set` / `var.add` | `name`, `value` / `delta` | quest variables (persistent) |
| `signal.send` | `name` | the `event.signal` event for triggers/waits (including other quests' — the global name `<module>.<name>` or your own) |
| `quest.activate` | `quest: quest_id` | activates another quest (`activation=manual`) |
| `task.give` / `task.complete` / `task.fail` / `task.remove` | `task` | §9 |
| `task.set_target` | `task`, `target: target_ref` | §9 |
| `task.set_text` | `task`, `new_title?`, `new_descr?` | §9 (not `title`: a reserved catalog key) |
| `spawn.squad` | `spawn_spec` (`section`, `smart`, `ref`, `hold=true`) | `SIMBOARD:create_squad(SIMBOARD.smarts_by_names[smart], section)` (`sim_board.script:176,142`) + `setup_squad_and_group` for the members (like `xr_effects.create_squad`, `xr_effects.script:3542`); `hold` → `squad.scripted_target = smart` (`sim_squad_scripted.script:102-106`), **not persisted by the engine** → the runtime re-applies it from `refs` on `squad_on_register`/first update |
| `spawn.npc` | `npcs: npc_list`, `place`, `spread=1.5` | per row: `unique` → one `alife_scan` for the section, each victim through `prepare_removal` + safe release (a squad it was alone in goes with it), every ref in every graph that named it cleared; then `alife():create(<module>_<id> \| section)` on a ring point snapped to the navmesh; `yaw`/`dead` as in `spawn.object`; the row's `ref` is set |
| `spawn.object` | `section`, `place`, `yaw=0`, `dead=false`, `ref?` | `alife():create`; `yaw` → the server entity's `angle`; `dead` → an online kill job (the new_corpse mechanism) for a creature section |
| `squad.move` | `target: {ref}\|{story}`, `smart` \| `follow_actor: bool` | `squad.scripted_target = smart` or `"actor"` (`sim_squad_scripted.script:128`) |
| `squad.remove` | `target` | `SIMBOARD:remove_squad` via `safe_release_manager.release` (`xr_effects.script:3642,3316`) |
| `npc.remove` | `npc_ref` | `safe_release_manager.release(alife():object(id))` |
| `npc.kill` | `npc_ref` | `obj:kill(obj)` online; offline — release |
| `relation.set` | `who: npc_ref` (incl. `{community=…}`), `value: relation` (attitude toward the player) | `npc:force_set_goodwill(±1000, actor)` / `squad:set_squad_relation` / `game_relations.change_factions_community_num` (`xr_effects.script:3821-3951`) |
| `relation.goodwill` | `who`, `delta` | `npc:change_goodwill`/`inc_faction_goodwill_to_actor` |
| `news.tip` | `text`, `sender: npc_ref?`, `icon?`, `duration=5` | `news_manager.send_tip(actor, text, nil, sender_go, showtime)` (`news_manager.script:67`), the text is raw cp1251 |
| `map.spot` / `map.unspot` | `target: target_ref`, `text?`, `spot: spot_type=secondary_task_location` | `level.map_add_object_spot(id, spot, hint)` / `map_remove_object_spot` (these wrappers are commented out in `xr_effects` — we call the engine directly) |
| `dialog.force` | `npc: npc_ref`, `allow_break: bool=true` | `db.actor:run_talk_dialog(npc, not allow_break)` (`xr_effects.force_talk`, `xr_effects.script:1741`) |
| `dialog.break` | — | `actor:stop_talk(); npc:stop_talk()` (`dialogs.break_dialog`, `dialogs.script:1624`) |
| `actor.teleport` | `place` (same level) | `db.actor:set_actor_position(pos)` (`xr_effects.teleport_actor`, `:2200`); a foreign level is validation error E031 (DA has no scripted cross-level teleport; transitions go through a level changer) |
| `sound.play` | `theme: string` | `xr_sound.set_sound_play(actor:id(), theme)` (like `xr_effects.play_sound_on_actor`, `:3368`) |
| `lua` | `code` | §14 |

### 6.5 Conditions (`use = cond`)

Level conditions (allowed anywhere, including phrases and `flow.branch`):

| kind | params | implementation |
|---|---|---|
| `has_info` | `info` | `has_alife_info(info)` (`_g.script:540`, works offline) |
| `has_item` | `section`, `count=1` (item units: for ammo — boxes, not rounds) | counted over the actor's inventory |
| `has_money` | `amount` | `db.actor:money()` |
| `var` | `name`, `op: enum(eq\|ne\|lt\|le\|gt\|ge)=eq`, `value` | quest variables |
| `node_done` | `node: string` | a `once` node has already run (`done`) |
| `quest_status` | `quest: quest_id`, `is: enum(inactive\|active\|completed\|failed)` | another quest's state |
| `task_status` | `task`, `is: enum(none\|active\|completed\|failed)` | §9 |
| `objective_status` | `task`, `objective`, `is: enum(none\|active\|completed\|failed)` | the task step is in the given status; state from `qs.objectives` |
| `actor_on_level` | `level` | `level.name()` |
| `actor_in_place` | `place` | like `objective.reach` |
| `npc_alive` / `npc_dead` | `npc_ref` | the server object's `se:alive()` / absence |
| `squad_alive` | `target` | `alife():object(id) ~= nil` |
| `actor_community` | `community` | `db.actor:character_community()` |
| `relation` | `who: npc_ref\|community`, `is: relation` | `game_relations`/`npc:relation(actor)` |
| `time_of_day` | `from: int`, `to: int` (hours) | `level.get_time_hours()` |
| `elapsed` | `from: enum(quest\|node)=quest`, `node?`, `duration` | by the quest's/token's `t0`, game or real time (the name is `from`, not `since`: `since` is a reserved catalog key, parameter keys must not collide with reserved ones) |
| `chance` | `percent` | `math.random(100) <= percent` (for phrases — on every display) |
| `lua` | `code` | §14, `return <bool>` |

Event conditions (`event.*`; allowed only in `trigger.when` / `wait.when` /
`wait.any`; in phrase `cond` and `flow.branch` — validation error E020):

| kind | params | source |
|---|---|---|
| `event.item_taken` / `event.item_dropped` / `event.item_used` | `section` | `actor_on_item_take/drop/use` |
| `event.npc_killed` | `who: npc_ref\|community\|any`, `by_actor=false` | `npc_on_death_callback`, `monster_on_death_callback` |
| `event.squad_dead` | `target` | `squad_on_unregister` + offline polling |
| `event.info_given` | `info` | `actor_on_info_callback` |
| `event.level_entered` | `level` | `actor_on_first_update` after `on_level_changing` |
| `event.talk_started` | `npc: npc_ref` | the dialog distribution hook (§12.3) — the moment the talk window opens |
| `event.talk_ended` | `npc` | `actor_on_leave_dialog(victim_id)` (`pda.script:81`) |
| `event.signal` | `name` | `signal.send` |
| `event.trade_done` | `npc?` | `actor_on_trade` |

Logical combinators: `["not"]` on any condition; `any` = `{ kind = "any", params = { of = { <cond>, … } } }`.

### 6.6 Catalog extension rules

- A new kind/parameter — a new section/key, `since = <version>`; the editor hides
  kinds newer than the connected game's catalog version only when there is no such
  game; otherwise it shows what the game actually served.
- Renaming a kind is forbidden; an `alias = old.kind` may be added (the runtime
  resolves it).
- A parameter cannot become required retroactively.

---

## 7. Dialogs

### 7.1 How the graph maps onto the engine

Engine facts (per `src\xrGame`): `CPhraseDialog::load_shared`
(`PhraseDialog.cpp:198-248`), when `<phrase_list>` is absent, calls `init_func(dialog)`
**once per id per process lifetime** (the data is `CSharedClass`, `shared_data.h:118-127`);
`AddPhrase(text, id, prev_id, goodwill)` on a repeated id **adds an edge** and
returns `nullptr` (`:252-282`) — cycles are legal, but preconditions/actions must
be attached on the first add; the speaker strictly alternates on every edge
(`SayPhrase`, `:78`), the root is phrase `"0"` (`:38-40`); the NPC picks a reply by
goodwill (threshold by attitude, then **randomly** within equal goodwill —
`AI_PhraseDialogManager.cpp:27-70`); if every continuation of a non-leaf phrase is
filtered out — `R_ASSERT2` (`:129-130`); a phrase precondition is
`f(first, second, dialog_id, phrase_id, next_phrase_id)` (`PhraseScript.cpp:139-170`,
not wrapped in try), an action is `f(first, second, dialog_id, phrase_id)` (`:172-191`,
wrapped); `script_text` is `f(first, second, dialog_id, phrase_id) → string` (`:79-92`);
phrase text not found in the string table is returned as is (`StringTable.cpp:455-463`).
The Lua surface: `CPhraseDialog:AddPhrase`, `CPhrase:GetPhraseScript`,
`CPhraseScript:AddPrecondition/AddAction/SetScriptText/AddHasInfo/…`
(`PhraseDialog_script.cpp`).
The mapping:

- Each `dialog.topic` node = one engine dialog with id
  `nq.<module>.<quest>.<node>`; root phrase `"0"` = the topic text. With
  `initiator=actor` the dialog lands in the actor's topic list; with `initiator=npc`
  it is assigned via `npc:set_start_dialog(id)` (`script_game_object_script3.cpp:291-293`)
  while the token sits on the topic and the NPC is online; removed by
  `restore_default_start_dialog()`. `xr_meet` may overwrite start_dialog by its own
  logic — the runtime re-applies the value once per tick if
  `npc:get_start_dialog() ~= id`.
- Each `dialog.*_phrase` node reachable from the topic over edges through phrase
  nodes = a phrase with the node's id; `next` edges = `AddPhrase(text, id, prev)`.
- **Alternation**: even depth from the root — the initiator, odd — the partner.
  The validator (E010) requires `dialog.actor_phrase`/`dialog.npc_phrase` to sit at
  depths of the correct parity along **all** paths from the topic; two consecutive
  replies of one side are expressed as two nodes joined through an "…" phrase of
  the other side (the editor offers to insert it automatically).
- **Deterministic NPC choice**: phrase goodwill = `-10000 - <ordinal in out.next>`;
  the engine sorts by descending goodwill and takes the first one passing the
  threshold → the **first in file order** phrase whose `cond` is true gets chosen.
  For the actor the same order = the display order in the list.
- **Assert protection**: the runtime adds a hidden unconditional leaf continuation
  ("Пока." on the actor side / "…" on the NPC side) when the phrase's continuations
  are **not all** unconditional — one unconditional sibling is not enough
  (`build_dialog`: `#kids > 0 and (uncond == 0 or uncond < #kids)`). An
  unconditional phrase has no `cond` **and** no `once` (`is_uncond` =
  `#node.cond == 0 and not node.once`): `once` works as a precondition. The leaf
  hangs at minimal goodwill, so it is taken only when no real candidate passed;
  but once reached — the talk closes and the topic does **not** count: no topic
  `on_exit`, no `done` pin, the token stays on the topic. Among NPC replies the
  leaf is unreachable (any unconditional sibling shadows it); in the player's
  reply list it is a real selectable row. The editor's W011 mirrors this split:
  for player replies — exactly the runtime condition, for NPC replies — only when
  there are no unconditional ones at all.
- **Leaving the dialog into the graph**: an edge from a phrase to a non-dialog
  node = "once the phrase is said, activate the target" (through the queue). A
  phrase with no dialog children = a leaf → the talk closes; at that moment the
  topic counts as passed: topic `on_exit`, token removal (with `once`), the
  `done` pin.
- Closing the talk window mid-branch completes nothing: the topic stays available
  from the start (X-Ray behavior; state — via `cond`/`var`).
- A phrase's `on_enter` runs in the phrase `action` **before** its mark, `on_exit` —
  right after (both synchronous, in one callback); so a `var.set` in a player
  reply's `on_exit` is already visible to the `cond` of the next NPC reply (the
  engine filters continuations after the action, `PhraseDialog.cpp:95,113`).
- Topic `once`: after a leaf the topic disappears; `once=false` — the topic is
  repeatable.
- NPC dead/nonexistent/offline — the topic is simply not offered, the token
  waits; for a "what if they are dead" fork — a parallel `wait.any` branch / a
  trigger with `npc_dead`.
- One NPC — any number of topics from different quests and modules: the topic
  list = the union over all active tokens, ordered by quest uid and node order.
- Texts: without placeholders — static cp1251 in `AddPhrase`; with `{…}` —
  `SetScriptText("xms_nq_dialog.text")`, the function substitutes values at
  display time.
- One function set serves all quests: `xms_nq_dialog.init(dialog)`, `.pre(...)`,
  `.act(...)`, `.text(...)`, `.query(npc, actor)`; demultiplexed by `dialog_id`.
  Everything inside is `pcall`ed; a precondition returns `false` on error.

### 7.2 Registration and NPC distribution (engine changes)

1. `xms_native_dialog_register(dialog_id, "xms_nq_dialog.init")` /
   `xms_native_dialog_unregister(id)` / `xms_native_dialog_invalidate(id)` — a
   registry of "virtual" dialogs in `CPhraseDialog` (a static
   `xr_flat_hash_map<shared_str, shared_str>`); `load_shared` checks the registry
   **before** `GetById` (`PhraseDialog.cpp:200`) and takes the XML-less
   `init_func` branch for a virtual id. `invalidate` drops the dialog's shared
   data (for `nq reload`).
2. A hook in `CActor::UpdateAvailableDialogs` (`actor_communication.cpp:193-221`):
   after the partner's `ActorDialogs()` are added — a Lua call
   `xms.dialogs_for(partner, actor)` (if the function exists), the returned ids →
   `AddAvailableDialog(id, partner)`. The runtime returns only topics with an
   active token, a matching NPC and true `cond` — that IS the availability filter
   (no dialog-level precondition needed). The same call = the `event.talk_started`
   event.
3. Registration is idempotent and happens on every runtime init (the Lua state is
   rebuilt on new game/load/level change).

---

## 8. Object and target references

- **`story`** — a CoC-style string story id: `[story_object] story_id = …` in the
  spawn's `custom_data`, the `story_objects.script` registry
  (`get_story_object_id(sid)`, `_g.script:1548`; not persisted, rebuilt from the
  spawn). A mod-placed NPC gets a story id through its `custom_data` in the
  `.xspawn` (`cpp-editor-last:Source/Editors/LevelEditor/Editor/EditorModScene.cpp:774-792` wrote `custom_data` as a file) —
  the editor appends `[story_object]` via the "assign story id" button in the
  picker (§13.6). Vanilla NPCs — by their story ids.
- **`ref`** — the name of an object this quest created (`spawn.*`,
  `objective.kill{spawn}`, the `objective.reach` anchor). Stored as
  `refs[name] = { id, kind, section, smart, hold }`, survives saves. Additionally
  the runtime registers the story id `nq.<module>.<quest>.<ref>` in
  `story_objects` (not persistent, re-applied on load) — so custom Lua and vanilla
  condlists can address the object.
- **`profile`** — any NPC with the given `character_profile` (`npc:profile_name()`).
- **`community`** (+`level`) — any NPC of the community (`npc:character_community()`).
- Resolving an `npc_ref` to a specific object: `ref` → `story` → the first online
  NPC by `profile`/`community`; for dialogs the check goes **from the NPC** ("does
  this partner match the ref").
- **`place`**: `{level,pos,radius}` — distance from the actor on that level;
  `{restrictor}` — `db.zone_by_name[name]:inside()` (`xr_conditions.actor_in_zone`,
  `xr_conditions.script:760`); `{smart}` — the smart's radius.
- **Position → vertices**: `level.vertex_id(pos)` on the current level,
  `xms.graph_vertex(level, x, y, z)` (`xms_game.cpp:563-587`) for any level.

---

## 9. PDA tasks

`task_manager.script` reads `misc\task_manager.ltx` **once at script load** and
has no registration API (`task_manager.script:1,50-53`); the engine's
`game_tasks.xml` is a singleton. So NQ drives tasks itself through the engine's
`CGameTask` (`GameTask_script.cpp:16-104`, `script_game_object_script3.cpp:226-238`):

- `task.give`: `t = db.actor:get_task(tid, true) or CGameTask(); t:set_id(tid);
  t:set_type(task.additional|task.storyline); t:set_title(text); t:set_description(text);
  t:set_priority(0); t:set_icon_name(icon or "ui_pda2_mtask_overlay");
  [t:set_map_location(spot); t:set_map_object_id(id); level.map_add_object_spot(id, blink, "")];
  db.actor:give_task(t, 0, true, 0)`.
  `tid = "nq.<module>.<quest>.<task>"`. No complete/fail functors: only the
  runtime changes state. The engine's actor callback already sends the `new` news.
- `task.complete`: `db.actor:set_task_state(task.completed, tid)`; the engine's
  actor callback already sends `complete`, so a manual call would create a
  duplicate.
- `task.fail`: `db.actor:set_task_state(task.fail, tid)` +
  `news_manager.send_task(db.actor, "fail", t)`. The binder deliberately does not
  send failure, and `task_manager.task_callback` silently returns for a dynamic
  NQ id (`task_manager.script:184-187`), so only failure stays manual.
- `task.set_target`: `t:remove_map_locations(false); t:change_map_location(spot, id)`;
  the `target_ref` resolves to an object (`ref`/`story`/`smart` — a smart has an
  id; for `place{pos}` — the restrictor anchor). A target on another level stays a
  spot on that level's map (we do not use `task_objects` guiders).
- `task.set_text`: `set_title/set_description`.
- Persistence: the tasks themselves live in the engine's actor registry (vanilla
  behavior); NQ keeps `tasks[tid] = "active"|"completed"|"failed"` in its own blob
  and reconciles with `db.actor:get_task(tid, false)` on load.
- Spot types: `secondary_task_location` / `storyline_task_location`, blink
  `ui_secondary_task_blink` / `ui_storyline_task_blink` (like
  `task_objects.script:122-161`).

---

## 10. Texts and localization

- The asset is UTF-8. `text` = a string (the default language is `rus`) or a
  per-language table.
- On load the runtime picks the language from `localization.ltx [string_table]
  language`, falls back to `rus`, then to any; converts **UTF-8 → cp1251** in pure
  Lua (`xms_nq_util.cp1251`: a Cyrillic table, `Ё/ё`, `— – « » “ ” … №`; anything
  else → `?`).
- Strings enter the engine raw: `AddPhrase`, `set_title`, `send_tip` — `translate()`
  returns the key when it is absent from the table. No module string table is
  generated.
- Placeholders: `{var:name}` (a quest variable), `{actor}` (the actor's name);
  no `{n}` inside count-dependent phrases — just `{var:…}`.
- Editor: the editing language is a toolbar selector; warning W040 flags a
  character outside cp1251.

---

## 11. Persistence

- The channel: `xms.save_data("xms.nq", marshal.encode(state))` / `xms.load_data("xms.nq")`
  (`xms_game.cpp:487-519`; chunk `0x584D0000|ns`, `xms_game.h:21-28`). The core
  gets a reserved pseudo-ns `xms.nq → 0xFF01` (§12.3) so it does not occupy a
  module's blob (a module author keeps their own `xms.save_data(<id>)`).
- **Re-staging is mandatory**: a loaded blob does not enter the next save by
  itself (`xms_game.cpp:338-342`, `GetLoadedBlob` does not stage). The runtime:
  after `load_data` on `actor_on_first_update` — an immediate `save_data`; then on
  every state change (`dirty`) at the end of the tick.
- `marshal` is available (`Externals\xrLuaFix\lua-marshal`, the `marshal` global,
  `USE_MARSHAL`).

The schema:

```lua
state = {
  v = 1,
  quests = {
    ["madmer.cordon_tales.wolf_debt"] = {
      status = "active",                -- inactive|active|completed|failed
      hash   = 0x1234abcd,              -- FNV-1a of the asset's canonical text at the last run
      tokens = { kill = { t0 = <game secs>, w = { … } } },
      vars   = { agreed = true },
      refs   = { boars = { id = 5123, kind = "squad", section = "simulation_boar", smart = "…", hold = true } },
      done   = { meet = true },         -- once nodes that have run
      fired  = { t_item = 2 },          -- trigger fire counters
      joins  = { j1 = { a = true } },
      tasks  = { kill_boars = "active" },
      errors = { kill = "…" },
      timers = { wait_1 = { real_left = 12.5 } },
      jobs   = { [5124] = { kill = true } },  -- online jobs: applied by the poll once the client object exists
    },
  },
}
```

- **A graph change between sessions** (the mod was updated): `hash` mismatch →
  reconciliation: a token on a nonexistent node or a node with a different `kind`
  is removed with a log entry; new `vars` get defaults; `refs`, `done`, `tasks`
  are kept; triggers re-arm against the new graph.
- **The mod was removed**: the record stays in the blob (tiny), ignored; once the
  mod is back — it continues.
- **A vanilla save / original DA**: `.scov` is ignored, `.scop/.scoc` untouched.
- Game time is stored via `utils.CTime_to_table` (`utils.script:546`); real time —
  as a remainder.

---

## 12. Game side: implementation

### 12.1 Initialization and order

1. The `xms_nq*.script` scripts are the Refined loose layer
   (`packaging\...\compatibility\gamedata\scripts`), installed with the game like
   the rest of `dead_air_x64_*`.
2. `xms_nq.on_game_start()`: `RegisterScriptCallback("actor_on_first_update", init)`.
   Nothing else — `db.actor` does not exist yet (`bind_stalker.script:38`), modes
   are not restored.
3. `init` (on the actor's first update, `bind_stalker_ext.script:80`):
   `xms_nq_load.scan()` → for every module of `xms.modules()` with `enabled` and
   `xms.module_applies(id)`: `xms.list_files(id, "", "*.nqasset", true)` →
   `xms.read_file` → parse in an empty environment → `validate` → `canonicalize`
   (texts to cp1251, defaults, indices) → the `quests[uid]` registry.
   Then `state = decode(xms.load_data("xms.nq"))` → reconciliation → dialog
   registration (§7.2) → activation of `auto` quests → restoration of `refs`
   (story id, `scripted_target`, start_dialog) → registration of the remaining
   callbacks (only if quests > 0) → blob re-stage.
4. The tick — `actor_on_update`; a level change — `on_level_changing` (the blob
   flush is already covered by the dirty logic; with a fresh Lua state `init`
   runs again and picks up the pending blob).

### 12.2 Event dispatcher

The `subs[event_name] = { {quest, node}, … }` index is built when a token enters
a waiting node and when a trigger arms; engine callbacks are turned into
`evt = { name = "npc_killed", npc = go, se = se, killer = who, story = sid, section = … }`
and distributed to subscribers through the queue. Every event is processed
deterministically (subscription order). Polling (`poll`) walks the `polled` list
once per 250 ms.

### 12.3 Engine changes (C++, all small)

| # | What | Where | Why |
|---|---|---|---|
| 1 | `xms_native_list_files(id, subdir, mask, recursive) → {relpaths}` | `xms_game.cpp` next to `xms_native_read_script (:460-485)`; on top of `XMS::ListFiles` (`xms_core.h:188-191`) + recursion | find `*.nqasset` in a module |
| 2 | `xms_native_read_file(id, relpath) → string\|nil` | same file; a guard like `valid_script_name (:446-458)`: `[A-Za-z0-9_./-]`, no `..`, inside `root` | read the asset |
| 3 | `xms_native_module_applies(id) → bool` | a wrapper over `XMS::ModuleApplies` (`xms_core.cpp:1215-1238`) | the `mode=` gate with the same semantics as the engine |
| 4 | The core's pseudo-ns for `save_data/load_data`: the `kCoreBlobs = { {"xms.nq", 0xFF01} }` table, resolved before the module lookup (`:487-519`) | `xms_game.cpp` | one NQ blob without touching module blobs |
| 5 | Virtual dialogs: `CPhraseDialog::RegisterVirtual/UnregisterVirtual/InvalidateVirtual`; a branch in `load_shared` before `GetById` (`PhraseDialog.cpp:200`); natives `xms_native_dialog_register/unregister/invalidate` | `PhraseDialog.{h,cpp}`, `xms_game.cpp` | dialogs without XML |
| 6 | A hook in `CActor::UpdateAvailableDialogs` → Lua `xms.dialogs_for(partner, actor)` (if the function exists; `try/catch`, error → log and empty) → `AddAvailableDialog(id, partner)` | `actor_communication.cpp:193-221` | quest topics on any NPC; the Lua side keeps an "NPC → topics with tokens" index, the call is cheap |
| 7 | `CMD1(CCC_XmsNq, "nq")` → `xms_nq_console.exec("<args>")` (modeled on `run_string`) | `console_commands.cpp:2442-2447` | debugging |
| 8 | The `xms.*` bootstrap: aliases `xms.list_files`, `xms.read_file`, `xms.module_applies`, `xms.dialog_register/unregister/invalidate` | `xms_game.cpp:592-765` | one API |

All of it is additive; not a single existing contract (saves, XMS, the Lua API)
changes. `PROJECT_RULES.md` §3 (verification against the original) is unaffected:
the changes are new entry points, not altered vanilla behavior; zero modules =
zero calls.

### 12.4 The `nq …` console

`nq list` (uid, status, errors) · `nq state <uid>` (tokens, vars, refs, tasks) ·
`nq activate <uid>` · `nq reset <uid>` · `nq jump <uid> <node>` (remove
everything, place a token) · `nq fire <uid> <node>` (force-complete a waiting
node with its default pin) · `nq reload` (re-read the assets + `dialog_invalidate`
on all own ids; state is kept, reconciliation as on a hash change) · `nq debug 0|1` ·
`nq validate` (re-validate everything, print the problems) · `nq dump` (json to
`$app_data_root$\nq_report.json`).

The log prefix: `* [nq]` / `~ [nq]` / `! [nq]`. The editor's graph canvas notes (framing, connected,
removed, bookmarked, a refused link) are `* [nq]` notes: the status bar counts only `~` and `!` lines.

### 12.5 Game QA

Per `PROJECT_RULES.md` §8: a hidden launch `xrEngine.exe -fsltx <qa.ltx> … -start server(<save>/single/alife/load)`,
the QA script hooks in via `[common]`, waits for `actor_on_first_update`, then
`nq jump/fire` plus checks of `nq state` and the log. Scenarios — §16.

---

## 13. Editor side: implementation

The Rust editor's quest code is `gz-nq` (the model, reading and writing through `gz-luadata`, the catalog, the validator, the pickers, the documents and their commands) and `gz-ui-graph` (the tab, the canvas, the panels). The C++ files this section names (and its line numbers) are the tree at the tag `cpp-editor-last`.

### 13.1 Model (`NqAsset.h`)

```cpp
struct SNqAction { xr_string kind; SNqParams params; };            // params: an ordered key→variant-value map
struct SNqCond   { xr_string kind; SNqParams params; bool negate; };
struct SNqNode   { xr_string id, kind, comment; bool once, once_set; SNqParams params;
                   xr_vector<SNqCond> cond; xr_vector<SNqAction> on_enter, on_exit;
                   xr_vector<std::pair<xr_string, xr_vector<xr_string>>> out; Fvector2 pos; };
struct SNqObjective { xr_string id; SNqValue title, descr, target; bool visible; };
struct SNqTask   { xr_string id, type, icon; SNqValue title, descr, target;
                   xr_vector<SNqObjective> objectives; bool auto_complete; };
struct SNqQuest  { int nq; xr_string id, title, activation; xr_vector<SNqVar> vars;
                   xr_vector<SNqTask> tasks; xr_vector<SNqNode> nodes; };
```

`SNqValue` — a variant (nil/bool/number/string/table with ordered keys) — whatever
the parser returned; typing comes from the catalog at validation/draw time.

### 13.2 Catalog (`NqCatalog`)

Loading: first `EditorGameContent::Find("configs\\nq\\catalog.ltx")` plus every
`configs\nq\kinds\*.ltx` of the connected game (`EditorGameContent` reads archives
`database\*.xdb*` archives and loose `gamedata\**`), plus the extensions of
installed modules — `<game>\modules\*\gamedata\configs\nq\kinds\*.ltx` and
`<game>\mods\*\…` (both roots, modeled on `EditorGameModes::ScanInstalledModules`);
otherwise — the bundle `<sdk>\gamedata\configs\nq\catalog.ltx`. Cached until
`Invalidate()` (link change). `title` — cp1251 → UTF-8 via `Cp1251ToUtf8`
(`EditorGameModes.cpp:17-31`).
`quest_catalog` and the toolbar show the source (`game`/`bundled`) and `version`.

### 13.3 Reading/writing (`NqLua`)

- Reading goes through the LuaJIT C API (already linked:
  `cpp-editor-last:cmake/EditorTargets.cmake:118-126`, `lua51.dll` is staged): `luaL_newstate`
  without opening libraries → `luaL_loadbuffer` → an empty environment
  (`lua_newtable; lua_setfenv`) → `lua_pcall` → walking the result (`lua_next`)
  into `SNqValue`. A syntax error/non-table → a message with line:column from
  Lua. One `lua_State` per operation, closed immediately.
- Custom `code` syntax check: `luaL_loadbuffer` of the chunk without execution
  (the same path) — warning W050 "Lua syntax error".
- Writing — the difference into the file (`gz-luadata`'s `LuaDoc::apply_diff`,
  `gz-asset`'s save path): a changed value is rewritten where it stands, a new key
  goes where the kind's order puts it (`nq, id, title, activation, vars, tasks,
  nodes`; node keys `id, kind, once, params, cond, on_enter, on_exit, out, pos,
  comment`), a new node after the node it is linked from, laid out like the file
  around it; nodes are matched by `id`. A new file (the template) is written
  whole: the header first, strings in `"…"`, multi-line ones `[[…]]`, 2-space
  indent, `out` a string for one target and a list for several.
- The outline (§4.5) is the document's (`quest_open`, `quest_apply`,
  `quest_edit`); nothing writes it into the file.

### 13.4 Document and undo (`NqDoc`)

- `NqDoc { path, quest, dirty, undo/redo (a ring of SNqQuest snapshots, 200), selection,
  view {cx, cy, zoom_idx} }`. Every mutation goes through `NqDoc::Apply(op)` (the
  same operations as MCP `quest_apply`, §13.11) → a snapshot into undo →
  re-validation (incremental is unnecessary: the graph is small).
- Open documents — the `UIQuestGraph::Docs()` registry; the path is the key.
  Re-opening focuses the tab.
- `Ctrl+Z/Y`, `Ctrl+S`, `Del`, `Ctrl+C/V/A` are handled by the document window
  while it is focused (`ImGui::IsWindowFocused(RootAndChildWindows)`); the scene
  commands `COMMAND_UNDO/REDO` do not fire then, the scene undo stack is
  untouched.
- An external file change (an AI rewrote it) reaches the document through the
  project's watcher: a document without unsaved edits reads the file again and
  keeps its view; one with unsaved edits rebases them onto the file
  (`docs/ASSETS.md` section 4). A save writes only the values the document
  changed, into the file's text as it is.

### 13.5 Validator (`NqValidate`)

One rule list for the editor and the runtime (codes §15). Returns
`SNqProblem { severity, code, node_id, slot ("enter:2"/"exit:0"/"cond:1"/"param:npc"), message }`.
Errors block the build; warnings do not.

### 13.6 Game-data pickers (`NqPickers`)

Indices build lazily from the connected game, cached in
`_appdata_\nq_index_<hash>.ltx` (project sources — `story_id` from `custom_data`
and the whole `restrictor` — bypass the cache):

| Type | Source |
|---|---|
| `item_section` | `pSettings` (already the game's, `EditorGameConfigs`), sections with `inv_name` |
| `squad_section` | `pSettings`, sections with `class = ON_OFF_S` |
| `level` | `configs\game_levels.ltx` |
| `smart` | sections of `configs\misc\simulation.ltx` (names) + positions/levels from the `all.spawn` scan (below) |
| `story_id` | two sources: (1) `pSettings` — sections with a `story_id` key (this is how DA declares unique NPCs: `configs\creatures\spawn_sections_*.ltx`, e.g. `esc_2_12_stalker_wolf`, `esc_2_12_stalker_trader`; `story_objects.check_spawn_ini_for_story_id` reads `system_ini():r_string_ex(section,"story_id")`) — the main and cheap one; (2) a scan of the connected game's `spawns\all.spawn` (`[story_object] story_id` in `custom_data` — restrictors, anomaly spots) through `XrSE_Factory`, cached in `_appdata_`; plus `[story_object]` from the current project's `.xspawn`. Without `all.spawn` — only (1) and the project ones, no W060 is issued |
| `profile` | `system.ltx [profiles] specific_characters_files` → `character_desc_*.xml` (id + name through the string table) |
| `community` | `configs\creatures\game_relations.ltx [communities]` |
| `info` | `[info_portions] files` → `gameplay\info_*.xml` |
| `spot_type` | the fixed core list |
| `restrictor` | **project only**: `rawdata\levels\<level>\spawn.part` (plain ltx text), objects with the `space_restrictor` spawn section — the object NAME is taken (key `000003`), because the runtime looks the zone up by NAME (`db.zone_by_name`), not by story id. Built bypassing the cache, so a freshly placed restrictor is visible immediately; `extra` = the level |
| `quest_id`/`task_id`/`var_name`/`ref_name`/`node id` | from the open document and the project's other `.nqasset` files |

A picker = a combo with search; manual input is always allowed (a value outside
the index — W060). `place` has a "take from the viewport" button: the scene's
current level + the point under the cursor / the selected object's position.
`npc_ref` has a "scene objects" tab (spawn points marked as mod content): picking
an object without a story id offers to write
`[story_object] story_id = nq_<mod>_<name>` into its `custom_data` (the scene
goes dirty).

### 13.7 Content browser and window

- `UIContentBrowser.cpp`: the `IsQuestExt(".nqasset")` predicate; the quest's
  vector icon on the tile; `OpenAsset` → `UIQuestGraph::Open(path, err)`; the
  **Add** menu (`DrawAddMenu`: the toolbar's [+ Add] and the grid's empty-space
  menu) offers **Quest** - off, saying why, outside the open project's own
  source -, creation modeled on `CreateFolder`: a
  free name `NewQuest.nqasset`, `NewQuest2.nqasset`, the template = a minimal
  quest (`trigger.start → flow.end`), `BeginRename` — **generalized to files**
  (folder-only until now, `:2286-2359`): renaming a file = `MoveFileA`; the `id`
  inside the asset is not synchronized (the id and the file name are independent;
  the id is edited in the Graph panel, W061 when they differ — a hint only).
- The window: a `UIQuestGraphWindow` per document, a major tab of the
  editor's shell (`XUI::Shell::Open`, ids `quest:<rel>` / `beh:<rel>`, reopened
  from `<project>\temp\session.ltx`) that owns the whole work area: the tab's
  toolbar sits on the shell's toolbar row, its Graph menu in the caption, its
  counts in the status bar. `UIQuestGraph::Update()` is pumped from
  `CLevelMain::OnDrawUI()`. The work area: the Graph panel, the canvas, the
  Details panel, and a drawer under them (par. 13.9).
- Nothing that redraws the frame may be called from `Draw()` (the `CLAUDE.md`
  rule, `XrUIManager::InUIPass()`).

### 13.8 Canvas (`NqCanvas`)

An in-house `ImDrawList` implementation (the tree carries no node libraries,
ImGui docking in `cpp-editor-last:Source/Editors/XrEUI/`; third-party ones
(imnodes/imgui-node-editor) give no vertical flow and are version-picky).

```
 Save Reload | Undo Redo | Validate  Auto Layout  Frame All | Back Forward | Find  Find in Project | Bookmarks ▾ ‹ ›      75% ▾ Minimap Shortcuts Graph Details
┌ Graph ───────────┬─────────────────────────────────────────────────┬ Details ─────────────────────┐
│ ▾ Quest          │          ╭ ▶ Quest start ·········· start ╮      │ ◆ Dialog: topic         meet │
│ ID     wolf_debt │          ╰────────────────┬───────────────╯      │ ▾ Node                       │
│ Title  Wolf debt │                           ▼                      │ ID        meet               │
│ ▾ Tasks        2 │      ┌ ◆ Dialog: topic ········· meet  ⚠ ┐      │ Kind      Dialog: topic      │
│   debt           │      │ npc:   story:esc_m_trader          │      │ NPC       story            ▾ │
│   deliver        │      │ text:  Heard you owe the Wolf…     │      │  Story ID esc_m_trader     ⋯ │
│                  │      │ ✉ PDA message  story:esc_m_trader  │      │ ▾ Actions                  2 │
│──────────────────│      │ ⌖ Map spot                    exit │      │ ✉ PDA message     On enter 1 │
│ Variables      1 │      │     next                done       │      ├──────────────────────────────┤
│ debt · number 0  │      └───────●────────────────────●───────┘      │ ✉ PDA message   On enter 1   │
│ Name     debt    │                                  ┌ minimap ┐     │ Kind      PDA message        │
│ Default  0       │                                  └─────────┘     │ help footer                  │
├──────────────────┴─────────────────────────────────────────────────┴──────────────────────────────┤
│ Problems (1) | Find Results (0)                                                                 × │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Infinite canvas**: world coordinates are floats, in pt at zoom 1.0; the
  transform is `screen = (world - center) * zoom * ui_scale + origin`. The scale
  is the zoom level times the UI scale (`XUI::Dpi()`), applied once
  (`NqCanvas::Scale`), so a graph reads the same at 100% and at 150%. Panning -
  held **RMB** (as in UE) and MMB; an RMB click without movement - the context
  menu.
- **10 zoom levels**: `{0.2, 0.3, 0.4, 0.5, 0.65, 0.75, 1.0, 1.25, 1.5, 2.0}`
  (index 6 = 1:1), the wheel steps the level by ±1 anchored under the cursor;
  `Home` - frame everything, `F` - the selection. **Frame All never zooms below
  0.75** (index 5, `NqCanvas::FrameMinLevel`), the level at which a title is 12
  px at 100%. What it fits is the graph's extent - the node boxes and the channels
  of the edges routed back up (`NqCanvas::Extent`), with 32 pt around. A graph
  that does not fit at 0.75 is shown AT 0.75 - the most of it that stays
  readable - from its trigger: the trigger (every root node when there is none)
  and the two levels under it are in view with the top of the view on them, the
  view across is the whole graph's middle as far as those allow, and it never
  runs past an end of the graph; the status says "The quest is larger than the
  view at 75%: framed from its start" (a behaviour graph: "The graph ..."). When
  even those levels are too wide, the trigger is the middle. Framing the
  selection and an explicit `zoom_level` may go lower.
- **Type**: titles 16 pt (Segoe UI Semibold), lines 15 pt, ids and pin names
  14 pt, each drawn at `round(pt * zoom * ui_scale)` px, so every level has one
  glyph size. **Level of detail** follows the size a title is really drawn at:
  10 px and up - everything; 7-10 px - the header's title only, the lines and the
  actions as bars; below - blocks.
- **Top-down flow**: the input is the node's top edge (one point, accepts N
  edges), the outputs are the bottom edge, one pin per catalog name. The pin
  names are written INSIDE the node, just above its bottom edge where no wire
  runs, each cut to the room its pin has; a single `next` pin needs no name.
  Edges are vertical cubic béziers with an arrow. An edge whose input is not
  below its pin (a loop back to a wait, a node beside its source) would cut
  through every node on the way as a bézier, so it is **routed around them**:
  straight runs with round corners - a slant down and aside out of the pin,
  sideways to a free vertical channel, up it, over the target and a slant down
  into its input. The channel keeps 24 pt from every node between the two heights
  and 12 pt from the channels of the edges routed before it
  (`NqCanvas::RouteBack`). The ends slant (24 pt aside) because every edge that
  comes down into an input arrives on the input's own vertical: a routed edge
  that came down it too read as one wire with them. A second routed edge through
  the same pin to the same side slants 12 pt steeper, so no two share a leg; the
  arrow follows the last piece of its wire.
- **A node** is a card 240 pt wide whose height follows what it says, measured
  by `NqLayout::Box` - the one source of that geometry for the layout pass, the
  canvas's drawing and hit tests, the minimap and `quest_layout`:
  - a 28 pt header: the fill tinted by the kind's family and a family line under
    it, the kind's icon, the title, the muted node id, and badges (a problem, a
    bookmark, `once`);
  - up to 3 lines, 20 pt each: the node's conditions first - what a wait waits
    for, what a trigger fires on - each as its title and what it checks
    (`Event: signal shmyandex_accept`, `Not has money 1000`), then the catalog
    parameters that are set as "key: value", in catalog order (the case names are
    pins already); what does not fit ends in a "+N more" line;
  - up to 4 action rows, 20 pt each - on_enter, then on_exit, and an on_exit row
    says so - with the action's title and what it acts on; more than four end in
    a "+N more" row;
  - the pin names, 18 pt.
  A trigger is the same card with round ends and no input. The colours are theme
  tokens: the family of a kind (trigger green, dialog blue, tasks orange, waits
  yellow, the end red, NPC steps lavender) tints its header, its icon and its
  minimap block. An error/warning badge; an unreachable node is dimmed. A
  behaviour graph never says "quest": its start and end read "Start" / "End"
  (`NqLook::KindTitle`).
- **Interaction**: LMB - select/drag (everything selected), a rubber band on
  empty space, `Ctrl`+click - toggle; dragging from a pin into another node's
  body/input - an edge; dropping onto empty space - the "create a node and
  connect" menu; `Alt`+click on the lit edge or `D` - delete it; `Del` - delete
  nodes (undo, no modal); `Ctrl+Z/Y`, `Ctrl+C/V` (the clipboard = a Lua fragment
  `return { nodes = {…} }`, paste dedups ids and offsets), `Ctrl+A`, `Ctrl+S`,
  `F2` - rename the id. An action row: a click opens the action in the Details
  panel, a double click focuses its kind, a drag moves it - within its list,
  between on_enter and on_exit, or onto another node (dropped on a header it is
  that node's last on_enter action); RMB on a row - duplicate, move up/down, move
  to the other list, remove. A new action comes from the node's context menu or
  the Details panel's Add Action. Snap to a 16 pt grid on release.
- **Navigation**: `Ctrl+F` - the Find Results drawer: search over the whole node
  content (id, kind, parameters, conditions, actions, links, comments),
  `F3`/`Shift+F3` - next/previous result; `Ctrl+Shift+F` - Find in Project;
  `Ctrl+B` - a transient bookmark on the selected nodes; `Alt+Left/Right` -
  back/forward through the transient view history (center, zoom, selection,
  Details slot). The minimap - 180 x 120 pt in the bottom-right corner, shown only
  for a graph of 12 nodes and more - shows the whole graph and the viewport
  rectangle; clicking/dragging it pans the canvas. Under 12 nodes its toggle is
  off and disabled, its tooltip says why, and the author's choice waits for the
  graph to grow. Without a minimap the canvas says where the graph goes on: a
  pill at the middle of each edge counts the nodes wholly beyond it ("2 more",
  a chevron pointing out) and a click pans just far enough to bring the nearest
  of them in; `quest_view` answers the counts as `beyond` either way. The zoom
  menu's label is the zoom itself (`75%`), no icon: the toolbar folds labels to
  icons when it runs short, and a value must not fold. It keeps one width at every
  level - a two-digit zoom is led by a figure space (U+2007), as wide as Segoe
  UI's tabular digits - or a zoom one digit wider would fold and unfold the
  labels beside it. Search, bookmarks, history and
  minimap visibility are not serialized and do not touch dirty/undo. The
  Shortcuts card over the canvas lists every key and gesture.
- **Layout** - a deterministic pin-aware top-down layout: branches follow the
  visual pin order; the full bounds of nested subtrees are separated by a small
  gap, whole trigger trees - by a larger one. A descendant shared by several
  branches becomes a single merge block between them and does not shift the
  center of the exclusive part of any branch or trigger. Rows are as tall as
  their tallest node's content.
- The view state (center, zoom) lives in `NqDoc`, not in the file.

### 13.9 The Graph and Details panels (`NqInspector`)

The tab's work area: the **Graph panel** (left, 280 pt, `graph.panel_w`), the
canvas, the **Details panel** (right, 360 pt, `graph.details_w`), and a
**drawer** under them (`graph.drawer_h`) with Problems (n) and Find Results. The
panels keep their pt size and the canvas takes every resize; when the canvas
would get less than 400 pt the Graph panel folds to a rail whose buttons open it
over the canvas. The toolbar's Graph and Details buttons show and hide the
panels. The sizes live in the layout (`[GrizzlyLayout]`, MCP `ui.split`).

Every field of both panels is a row of the UI kit's property grid: the label in
its column, the value beside it, the help in the label's tooltip and in the
Details panel's help footer. A kind's description is the help of its Kind row -
no paragraph of prose sits in a form. Rows nest (an objective of a task, a row of
a list, the value a mode picks); a value that takes one of several forms (a
place, an NPC reference) picks the form on its row and asks for what that form
needs on rows under it; what comes from a list is picked (⋯ opens the picker).
A text of more than one line and a Lua body show their first line and open in an
Edit Text / Edit Lua dialog (Check Syntax).

- **Graph panel**: the quest's header (ID, Title, Activation) or the behaviour
  graph's (ID, Title); then Tasks - a section per task with its objectives,
  Rename and References - or, in a behaviour graph, Roles - a section per role:
  Name, Title, Binding (the Group and Optional boxes on one row), Default and
  Delete Role; under a splitter (`graph.vars_split`) the Variables: a section
  like the others, with the count on its header - a list, and the variable picked
  from it as a form (Name, Type, Default). Folded, the Variables header sits at
  the bottom and the pane above takes the room (the split is kept for when they
  open); `scroll=variables` opens them.
- **Details panel**: nothing selected - an empty state pointing at the Graph
  panel; one node - its kind, id and problems, then the Node section (ID, Kind,
  Once, the catalog parameters - generated from their types: a changed kind
  resets the parameters, undo - and Comment), Conditions (`[not] [kind ▾]`,
  parameters under it; `wait.any/flow.branch` get the `cases` editor), Actions
  (a list row per action - its icon, its name, `On enter N` / `On exit N` at the
  right, the whole row one target and highlighted when selected; a newly selected
  action's row is scrolled into view, opening the section - then Add Action) and
  Outputs; under a splitter
  (`graph.action_split`) the selected action: its kind (extra actions by group +
  "Lua"), its parameters, move up/down and remove.
- **Drawer**: Problems (n) - a click focuses the node and slot, right-click
  copies; it opens when Validate finds something. Find Results - the search
  field, previous/next and the results.
- A file changed on disk behind the document shows a banner only when there is
  something to say: the file does not read (the graph keeps the last state that
  read, the banner names the reader's line), the file is gone (Save writes it
  again), or both sides changed some values ("Changed on disk too: <n> values",
  each with Take Theirs).
- Rename Task (Cancel is the default), Task References and Find in Project are
  dialogs of the kit.

### 13.10 Export and build

- In `EditorMod::Export`, before copying (after the mode gate,
  `EditorModManifest.cpp:855-862`) — `NqExport::Validate(project_root, err)`:
  finds every project `*.nqasset` (except source-only), parses, validates, checks
  `id` uniqueness within the module; on errors —
  `err = "quest graph errors: N (see log)"` + a per-line log, the build
  **refuses** (like "no target game mode"); MCP receives the same `err`.
- Assets are copied as is (already the shared folder-copy path); no generated
  files.

### 13.11 MCP

C++ — the graph commands in `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp` (the tabs, `UIQuestGraph::Mcp*`) and
`NqMcp.cpp`; the bridge's tools are the catalogue `crates/gz-mcp/tools.json` (the Rust editor's commands: `gz-nq`'s
and `gz-ui-graph`'s `cmd.rs`), the table in `tools/mcp/AI_SETUP.md`. Structured arguments travel as **a string
holding a Lua table** (the parser already exists — `NqLua`), responses are JSON
built by string emitters (no JSON library).

| Tool | Arguments | Response |
|---|---|---|
| `grizzly_quest_catalog` | — | kinds with groups, `use`, parameters (type/required/default/enum), pins; the catalog version; the source (game/bundled) |
| `grizzly_quest_open` / `grizzly_quest_close` | `path`[, `discard`] | |
| `grizzly_quest_undo` / `grizzly_quest_redo` | `path` | document undo/redo (the scene `undo` does not touch quests) |
| `grizzly_quest_apply` | `path`, `ops` = a Lua table of operations: `add_node{node}`, `set_node{id, patch}`, `rename_node{id,new_id}`, `remove_node{id}`, `connect{from,pin,to}`, `disconnect{from,pin,to}`, `add_action{id,slot,index,action}`, `set_action{id,slot,index,patch}`, `move_action{...}`, `remove_action{...}`, `set_quest{patch}`, `set_pos{id,x,y}` | problems after applying |
| `grizzly_quest_save` | `path` | writes the values the document changed; a change on disk meanwhile is merged first |
| `grizzly_quest_reload` | `path` | re-read from disk |
| `grizzly_quest_layout` | `path` | auto-layout; every position carries `w`/`h`, the node's content-sized box (`NqLayout::Box`) |
| `grizzly_quest_view` | `path`, `frame:"all"\|<node>`, `zoom_level`, `cx`/`cy` (override `frame`), `select:"<node>"\|"a,b"\|"none"` (the Details panel follows the selection), `slot:"enter:0"\|"exit:1"\|"none"` (open the action in the Details panel), `graph_panel`/`details`/`keys`/`problems:"show"\|"hide"\|"toggle"` (the Graph panel - on a rail it opens over the canvas -, the Details panel, the Shortcuts card, the Problems drawer), `graph_width`/`details_width` (pt), `action_split` (the share of the Details column the node keeps) / `vars_split` (the share of the Graph panel the Variables get), `scroll:"top"\|"bottom"\|"action:top"\|"action:bottom"\|"tasks"\|"task:<id>"\|"task:<id>/steps"\|"variables"` (an error when that pane is not on screen) | the resulting `zoom_level`/`zoom` (the level's factor) + `ui_scale`, `title_px`, `center`/`selected`/`slot`, `graph_panel` (`open`/`rail`/`overlay`/`hidden`), `details`, `drawer`, `minimap`, `beyond` (`{above, below, left, right}`: the nodes wholly past each edge of the view), `graph_width`/`details_width`, `panes` (graph, variables, canvas, details, node, action, drawer; pt of the main viewport, null when not drawn), the splits and the panes' scroll `[y, max]`; the answer describes the frames this call changed (a deferred `frame:all`, a selection, a slot, a pane, a width, a split, a tab that was not on screen; a scroll, and the node pane bringing a newly selected action's row into view, land a frame later): the handler marks it `pending:true` and the MCP pump holds the request like a screenshot until they are drawn, then answers by reading again with `answer_only:true` (applies nothing - a toggle must not flip back), so a raw socket caller gets it in one call too; then `grizzly_screenshot_editor` |
| `grizzly_quest_edit` | `path`, `op`, `nodes` (instead of the current selection), `from`/`pin`/`to` | every graph edit the canvas can do, without a mouse: `connect`, `disconnect` (without `to` — the whole pin, without `pin` — all pins), `bypass` (the node stays in place, the chain closes over it: A→B→C becomes A→C), `delete`, `duplicate`, `connect_nearest`, `select_all`. The response carries the outline — the result reads without a screenshot |
| `grizzly_quest_find` | `path`, `action`, `query`, `select`, `limit` | full-text results in file order, the active result and the view state; `get` with `query` atomically changes the query without navigating, `next`/`previous` focus the node |
| `grizzly_quest_rename_task` | `path`, `from`, `to`, `ack_runtime_identity:true` | a safe task rename inside the document: a preflight of every catalog-typed area, then one `Snapshot` + one mutation of the declaration/references. Refuses before the snapshot on unknown/Lua/malformed/partial; the ack is mandatory because of runtime/save identity |
| `grizzly_quest_bookmarks` | `path`, `action`, `node` | transient bookmarks (`get/add/remove/toggle/next/previous/clear`) and the view state |
| `grizzly_quest_history` | `path`, `action` | the transient history (`get/back/forward/clear`) with all view states |
| `grizzly_quest_minimap` | `path`, `action` | `visible` (the toggle), `available` (the graph has `min_nodes`, 12, nodes or more; the toolbar's toggle is disabled while it has not), `shown` (drawn last frame: available, and a canvas with room for it), graph/viewport bounds and the view state (`get/show/hide/toggle`) |
| `game.lookup` | `type`, `query`, `limit`, `offset`, `path` | picker data (§13.6) |
| `asset.check` | `path?`, `fail_on?` | the build's checks over a file, a folder or the project, every problem at its line and column |
| `asset.find` | `query?` / `refs_to?`, `path?` | the lines that hold a text, or the values and keys that name an id, across the project's assets |
| `asset.take_theirs` | `path`, `index?` | an open document takes the file's value of a conflict (all of them without `index`) |

A quest is a file: an agent reads and writes it with its own tools, and the open
tab follows. The destructiveness rule: MCP commands show no dialogs; `quest_close`
without `discard` on a dirty document returns an `unsaved` error.

`NqProjectIndex` never calls `NqDocs::Open`: it reads the sorted file list
directly and reuses a parse only on an exact content-hash match. An open
**dirty** document overlays its file read-only; a clean document does not hide an
external edit on disk. The fingerprint is built from normalized paths, the full
content hash, the source and the parseability status, so an identical project
snapshot yields an identical value.

`NqReferences` infers no meaning from names like `task`, `id` and so on: a
reference exists only when the active catalog declared that specific parameter as
`task_id` (or another requested type). For `cond_list` and `cases_cond` the kind
of every nested condition again comes from the catalog. An unknown
kind/parameter, a wrong shape, custom Lua or a partially read file mean the proof
is incomplete. That is why rename task first runs over the whole document on a
copy, and only at zero diagnostics creates one undo snapshot and changes the
declaration + the exact references. The UI makes Cancel the safe default button
and explicitly warns that the task id is already written into runtime/save
state; existing state does not migrate.

---

## 14. Custom Lua

- Where: the extra action `{ kind = "lua", params = { code = [[…]] } }` and the
  condition `{ kind = "lua", params = { code = [[ return … ]] } }`. Main actions —
  never.
- Compilation: `loadstring(code, "<uid>/<node>/<slot>")` once at quest load; a
  compile error is a validation error (the quest does not load, E050 with the
  position).
- The environment: `setfenv(fn, env)`,
  `env = setmetatable({ nq = ctx_api }, { __index = _G })` — full read/call
  access to the game API; global writes land in `env` (not in `_G`).
- `nq` in the environment: `nq.quest`, `nq.node`, `nq.actor` (= `db.actor`), `nq.npc`
  (the dialog partner or nil), `nq.vars` (a proxy over the variables, a write =
  `var.set`), `nq.ref(name)` → the server object, `nq.ref_go(name)` → the game
  object or nil, `nq.signal(name)`, `nq.news(utf8_text)`, `nq.tr(utf8)` → cp1251,
  `nq.log(...)`, `nq.give_item(section, n)`, `nq.take_item(section, n)`,
  `nq.has_item(section, n)`, `nq.money(delta)`, `nq.time()` (game seconds),
  `nq.done(node_id)`.
- A runtime error — like any action's (§5.4); a condition that errors = `false`.
- Texts inside code: the author writes UTF-8 and uses `nq.tr`/`nq.news`.

---

## 15. Validation (the shared rule list)

`E` — an error (build/load refused), `W` — a warning.

| Code | Rule |
|---|---|
| E001 | `nq` missing / not supported by the runtime |
| E002 | quest `id` not `[a-z0-9_]` or duplicated in the module/project |
| E003 | the file is not a single table / contains code outside strings |
| E004 | node `id` not `[a-z0-9_]` or duplicated |
| E005 | unknown `kind` (for the given position: main/trigger/extra/cond) |
| E006 | a required parameter is missing / wrong type / value outside the enum |
| E007 | the pin is not declared on the kind; the edge target does not exist; the edge leads into a trigger |
| E008 | not a single trigger |
| E009 | `flow.join` with no incoming edges |
| E010 | speaker alternation broken in a dialog (a `dialog.*_phrase` node at a depth of wrong parity along at least one path; or unreachable from the topic) |
| E011 | a phrase node has an input not from a phrase/topic (an input from the world) |
| E020 | an event condition (`event.*`) in a phrase/topic `cond` or in `flow.branch` `cases` (allowed only in `trigger.when` / `wait.when` / `wait.any`) |
| E021 | `cases` are empty / case names duplicate / `weight <= 0` |
| E030 (objectives) | `objective` names a step the given `task` does not have, or there is no `task` parameter next to it |
| E022 | several mutually exclusive parameter forms picked at once, or none at all (`objective.fetch`: `item` vs `section`/`count`/`from`; `objective.kill_count`: more than one filter). The forms are declared in `NqCatalog::ExclusiveForms` — one table serving both the validator and the Details panel, which dims the losing fields |
| E030 | a `task`/`quest`/`ref`/`var`/`node` reference to a nonexistent declaration |
| E031 | `actor.teleport` to another level |
| E050 | a custom Lua syntax error |
| W011 | continuations of a non-leaf phrase: among player replies not all are unconditional (a hidden exit past `done` will appear), or among NPC replies there is not a single unconditional one (an auto-reply will appear). Unconditional = no `cond` and no `once` |
| W012 | a `dialog.topic` with no continuation phrase |
| W013 | a topic with `once=false` and no conditions — will be offered forever |
| W020 | an unreachable node |
| W021 | a waiting node with no outgoing edges (the quest will hang here) |
| W022 | one target listed twice on one pin; the repeated edge is ignored |
| W030 | a `ref` is used before the node that creates it (not along all paths) |
| W031 | `spawn.squad` without `hold` — the squad may wander off into the simulation |
| W040 | a character outside cp1251 in a text, or in a string `vars` default (`{var:name}` splices it into text) |
| W050 | Lua syntax not checked (no LuaJIT) — editor only |
| W060 | the value is not found in the game index (story id/section/level/smart/profile/info portion) — editor only |
| W061 | the quest `id` is not the one the file name gives (lower case, other characters as `_`) — editor only |
| W062 | a spawn row sets a field its section cannot carry (a magazine on a medkit, doses on a rifle) — editor only, and only when the linked game knows the section |
| W063 | a spawn row's value is over the section's own ceiling (`ammo_mag_size` / `box_size`) — the runtime clamps it — editor only |
| W070 | a quest without `flow.end` (will finish implicitly) |

The runtime prints the same codes to the log; the editor — to the problems row
and MCP.

---

## 16. Test plan

**Editor (headless, `Grizzly.exe -headless -project <project> -nodlg` + MCP):**
1. `file.create kind=quest` → the file is the template, `asset.check` finds 0 errors.
2. The reference quest (§4.4) written as a file → 0 errors; opened and saved
   unchanged it is byte for byte the same file (`tools/qa/test_assets.py`).
3. `quest_apply` of an operation set → the expected problems (E007 on a broken edge,
   E010 on broken alternation, E020, W011 …).
4. `quest_catalog` contains every bundle kind; with a game linked — the game's.
5. `mod_export` with a broken quest → `ok:false`, the error text; with a valid one →
   the file in `<game>\modules\<id>\…\*.nqasset` is identical to the project's.
6. UI smoke: `quest_view` → `shot.window` (the tab, nodes, action rows,
   edges); `quest_view` at zoom 0.2 and 2.0; `quest_view frame=all` never leaves
   a title under 12 px (`title_px`); `quest_layout` does not change semantics
   (the outline is equal) and reports every node's `w`/`h`
   (`tools/qa/test_graph_ui.py`).
7. Undo: `quest_apply` ×3 → `quest_undo` ×3 → the original text; `quest_redo` ×3 →
   the final one.
8. The UI-less AI cycle: a file written beside the editor → `asset.check` names
   its problems at their lines; an open tab follows the file, and unsaved edits in
   it rebase onto the file's (`tools/qa/test_assets.py`).

**Game (the QA script per §12.5, the three reference quests from `docs\nq\examples`):**
1. `linear_fetch`: start → task → take the item → the reward → `flow.end`;
   `nq state` checked at every step via `nq fire`; a save/load in the middle →
   the token is in place.
2. `dialog_branching`: topics on Sidorovich, two player replies, an NPC branch by
   `var`; checks of `xms.dialogs_for` (the topic list), the auto-reply when every
   continuation is conditional, `once`.
3. `parallel_triggers`: `trigger.start` + `trigger.when{event.item_taken}` +
   `objective.kill{spawn}`; the squad is created, `hold` survives a load; an
   offline squad death (an `nq` release command) → `done`.
4. A level change there and back: state and refs are kept; `scripted_target`, the
   refs' story ids and start_dialog are re-applied.
5. The mod removed → the save loads, the log has no `!`; the mod returned → the
   quest continues.
6. A changed graph (a node deleted) → the token is removed with a log entry, the
   rest stays alive.
7. Zero modules with quests → the log shows no `[nq]` callback registration
   (zero cost).
8. An invalid asset (code outside the table, a broken pin) → the quest is
   `INVALID`, the game lives on.
9. `.scov`: after a load and an immediate save the NQ blob is present (the
   re-stage).

---

## 17. Rollout plan

Done: every phase below. Since the move to Rust the editor's half is `gz-nq` and `gz-ui-graph`
(`docs/RUST_MIGRATION.md`); the table is the record of how it was rolled out.

The working rule: every phase runs as separate subagents (per the `CLAUDE.md`
rules); once the contract is fixed, the game and editor phases go **in
parallel**. Commits follow the phases, each verified by its own DoD.

| Phase | Repo | What we do | Who | DoD |
|---|---|---|---|---|
| **0. Contract** | both | split `docs\dead-air\NQ_RUNTIME.md` (game) and `docs\nq\NQ_FORMAT.md` (editor: format+catalog+validation) out of this doc; write `configs\nq\catalog.ltx` v1 (game) and its copy in `<sdk>\gamedata\configs\nq\`; 3 reference `.nqasset` in `docs\nq\examples\` | 1 agent | both repos hold the same catalog; the examples pass a manual §15 check |
| **1. Engine** | game | the §12.3 changes (#1–#8), an incremental `tools\build\build_x64.ps1` build, smoke: the `nq` command answers, `xms.list_files/read_file` see a test module | 1 agent | the incremental build is clean; vanilla with no modules — zero new log lines |
| **2a. Runtime core** | game | `xms_nq.script`, `xms_nq_load`, `xms_nq_util`, `xms_nq_kinds` (flow.*, wait.*, trigger.*, extra: item/money/var/info/signal/news/quest, level+event conds), persistence, the console | 1 agent (parallel with 2b) | game test plan items 1, 6, 7, 8, 9 |
| **2b. Runtime dialogs/objectives/tasks** | game | `xms_nq_dialog`, `xms_nq_task`, the `dialog.*`, `objective.*`, `spawn.*`, `squad.*`, `map.*`, `relation.*` kinds | 1 agent | items 2, 3, 4, 5 |
| **3. Editor core** | editor | `Nq*` in `XrECore`: the model, `NqLua` (C API), the catalog, the validator, the pickers (indices), `NqDoc`+undo, the `NqExport` gate, `NqMcp` + python + `AI_SETUP.md`; the creation template | 1–2 agents (IO/validator/catalog ↔ pickers/MCP/export), parallel with 1–2 | editor test plan items 1–5, 7 (no UI) |
| **4. Editor UI** | editor | `UIQuestGraph`, `NqCanvas`, `NqInspector`, the content browser (Create/Open/file rename), the toolbar, the problems row, hotkeys, `quest_view/layout` | 1 agent after 3 (model/ops stable) | item 6; a manual check against the sketch |
| **5. Integration and QA** | both | e2e: MCP builds the 3 references → `mod_export` → the game per §12.5; fixes | 1 agent | every §16 item green |
| **6. Documentation** | both | `MODDING.md` (the NQ section, a link to `NQ_RUNTIME.md`), `MOD_QUICKSTART.md` (the "Quests" section), `AI_SETUP.md`, README lines; project memory | 1 agent | the docs describe what was built |

Risks and how they are closed:

| Risk | Closure |
|---|---|
| The dialog hook misbehaves in some UI path (trade, healing) | the hook only adds ids; topics always carry a root precondition = token presence; test item 2 runs on the trader |
| The "No available phrase" assert | the auto-continuation (§7.1) + W011 |
| NQ state falls out of the save | the re-stage on first update + dirty (§11), test item 9 |
| `xr_meet` overrides start_dialog | re-set once per tick while the token exists |
| The game and editor catalogs drift apart | the source is the game; the bundle is only a fallback; `quest_catalog` shows the source and version |
| An AI writes invalid files | identical error codes on both sides, `asset.check` returns the problems at their lines, the game does not crash |
| Id collisions between mods | uid = `<module>.<id>`; ref story ids and dialog/task ids carry the `nq.<module>.<quest>` prefix |
| A level change rebuilds the Lua state | `init` is idempotent, everything is restored from the blob + `refs` |
| An AI cannot see the canvas and is forced into screenshots | the file is the text, the outline in `quest_open`, `pos` optional, the catalog on demand, problems as text with positions (§4.5) |

---

## 18. Decision register (why this way)

| Decision | Alternative | Why |
|---|---|---|
| Interpretation in the game, not compilation in the editor | generate Lua/XML at build | the user's requirement + mods get fixed by a game update; an editor without the engine cannot verify generated code |
| `.nqasset` = declarative Lua (`return {…}`) | JSON/LTX/a full Lua DSL | Lua is readable by AIs and humans, loads identically in the game and the editor (LuaJIT exists in both), the empty environment makes it data; the game has nothing to parse JSON with, LTX is poor at nesting |
| The catalog in LTX + Lua implementations via `xms.registry` | a catalog in Lua | the editor has no game Lua runtime to execute a Lua catalog, while LTX is read by both sides; modules extend it with a file in the VFS |
| Instant operations are extra actions only; main = key stages | duplicate spawn/handout in main too | one way to write one thing; matches the user's description ("a node = a key action") |
| Conditions — the catalog + `lua` | the catalog only | triggers without custom conditions are too narrow; the "custom only in extras" rule was consciously extended to conditions, main stays strict |
| Phrases are individual nodes | a dialog editor inside a node | the user: "a node = a phrase"; the graph link of phrases to the rest of the quest (an edge from a phrase into the world) gives non-linearity for free |
| Tokens + a deferred queue | immediate recursive transitions | reentrancy inside `SayPhrase` and engine callbacks; determinism; the engine assert when every continuation is filtered |
| The dialog distribution hook + virtual dialogs in the engine | XML stubs from the editor / `character_dialogs.xml` | without an engine change a topic cannot be added to a specific NPC dynamically, and editor-made stubs violate the interpretation principle; the change is ~100 lines and additive |
| An in-house canvas on ImDrawList | imnodes / imgui-node-editor | neither is in the tree, both are horizontal, imnodes has no zoom, imgui-node-editor is pinned to an ImGui version (ours is 1.88 with patches); we need vertical flow, nodes that list their actions, 10 fixed zooms and MCP view control |
| One `xms.nq` blob in `.scov` | a blob per module / `.scoc` via `save_state` | do not fight the module author for their blob; `.scov` is the only sanctioned channel (SAVE_COMPATIBILITY); removing a mod breaks nothing |
| Tasks via `CGameTask` directly | `task_manager` + ltx | `task_ini` is read once at script load, there is no registration |
| Texts as raw cp1251 | generate a string table | `translate()` returns the key; a string table = one more generated file, and we generate none |
| Quest flags are `vars`, not info portions | info portions | info portions are not persisted outside the actor's `.scop` and the engine does not need them; `info.give` is kept for vanilla interop |
| String story ids | numeric `xms.story_id` | game scripts address objects by string ids (`story_objects`), the editor writes them into `custom_data` with no engine changes |
| A recursive module scan | a list in the manifest | the manifest cannot be read from Lua, one more desync point; the folder is the single source |
| Validation is the build gate | warn and export anyway | the project rule: the build refuses with a clear message; an AI gets the same codes through MCP |
| Node deletion without confirmation | a modal | the document has undo; the file is untouched until save; closing a dirty tab asks and names the asset |
| The outline, `pos` optional | "the graph is readable only on the canvas" | the user's requirement: an AI creates and edits quests as text, without screenshots |
| The editor keeps the file's node order and writes only what changed | a canonical rewrite on save | the author's (and an agent's) order, comments and layout are theirs; a diff shows exactly the edit |
## 19. Behaviour graphs — the module's NPC assets

### 19.1 A behaviour graph is not a quest, and not a quest file

A quest tracks the player: tasks, PDA text, dialogs, map spots. A **behaviour
graph** drives NPCs and never touches the player's screen: a squad walks to a
courtyard, digs in and opens fire on whoever the caller points them at; a guide
walks the player to a door and plays an animation there; a courier walks his
route and turns hostile at the end of it.

It is **its own asset**, not a quest wearing a flag:

| | quest | behaviour graph |
|---|---|---|
| extension | `.nqasset` | `.behasset` |
| header | `nq = 1` | `beh = 1` |
| editor | Quest Graph window | Behaviour Graph window |
| starts itself | yes (`activation`) | never — a caller runs it |
| addresses | the player, the world | **roles** the caller binds |
| has | `tasks`, dialogs, objectives | `roles` |

The extension IS the kind of document. There is no `type` key (writing one is an
error); the header key has to AGREE with the extension - `nq` in a `.nqasset`,
`beh` in a `.behasset` - so a renamed file is refused (`E001`) instead of being
believed. The content browser therefore knows what it is looking at without
opening it, and the editor opens the right window from the double click alone.

**The graph ENGINE is shared, and that is the point.** A behaviour instance is
an instance of the same interpreter as a quest (§5): the same node walk, the
same pins, the same conditions, the same persistence (§11). An author who knows
one graph knows both. What is not shared is the **vocabulary** (§19.10) and the
lifecycle.

### 19.2 Nothing about NPC behaviour is invented

X-Ray already moves, posts, animates and fights with NPCs — that is what the
logic schemes are (`xr_walker`, `xr_camper`, `xr_animpoint`, `xr_remark`,
`xr_companion`, `xr_combat_ignore`, the `sr_*` family). Smart terrains drive
them by **generating logic as text** and handing it over:
`create_ini_file(<text>)` followed by `xr_logic.configure_schemes(...)` — see
`xr_gulag.script`, which has done exactly this since CoP.

A behaviour step does the same thing. It writes a small logic section, applies
it to the bound NPCs and steps on. This is the whole implementation strategy,
and it is deliberate:

- combat, cover, danger, wounded, death and dialog keep working, because the
  stock schemes keep running them;
- an animation is a state the game already knows, not a bone track the editor
  invents;
- when the behaviour lets an NPC go, the NPC returns to whatever logic owned it.

A behaviour graph therefore cannot do what the schemes cannot do. That is a
feature: the alternative is a second AI fighting the first one. **If a step
cannot be expressed with a stock scheme, the answer is a scheme — one more
scheme of the module's own, written the way the game writes its own** (see
`xr_gather_items`, which walks a stalker to a dropped item). Never a movement
or combat loop bolted onto the interpreter.

### 19.3 The document

```lua
return {
  beh   = 1,
  id    = "bridge_ambush",       -- [a-z0-9_], unique in the module
  title = "Засада у моста",      -- author-facing, never shown to the player
  roles = {
    leader    = { title = "Главный", default = { story = "esc_wolf" } },
    attackers = { title = "Налётчики", many = true },
  },
  vars  = { … },                 -- as in a quest, scoped to the instance
  nodes = { … },
}
```

`roles` replaces `tasks`; `activation` does not exist here — a behaviour graph
is started by a caller, never by the world.

### 19.4 Roles

A role is a named slot the caller fills.

```lua
roles = {
  guard = { title = "Охранник",            -- author-facing
            many  = false,                 -- default false: exactly one NPC
            default = { story = "esc_wolf" },   -- optional
            optional = false },                 -- default false
}
```

- **Binding** happens at the call site (§19.6) with the object reference forms
  of §8: `{ story = … }`, `{ ref = … }`, `{ squad = … }`, `{ profile = … }`,
  `{ community = … }`. `many = true` accepts a squad or a list; `many = false`
  refuses more than one.
- A role with a `default` needs no binding — a one-off graph written for one
  named NPC reads as it should, and the same file stays reusable.
- A binding that resolves to nobody fails the run (`optional = true` makes the
  role simply empty instead, and steps addressed to it do nothing).
- Roles are resolved **once, on entry**, and kept in the instance: the cast
  cannot change under the graph's feet. Members that die drop out; a step to an
  empty role is a no-op, not an error.

This is the answer to "bind the asset to an NPC, or pick the NPC in the node":
both, and the asset stays reusable either way.

### 19.5 Steps

Every action of group `npc` carries `who` — one role name or a list of them.
The graph provides the ordering, the branching and the waiting; a step itself is
instant unless the catalog says otherwise.

| kind | what it does |
|---|---|
| `npc.goto` | Sends the role to a **place** (§19.5.1). `move = walk\|run`, `look` aims the look path when the place is a path, `arrive = true` makes the node wait until everyone is there. |
| `npc.hold` | Posts the role at a place and lets it fight from there (`camper`). `shoot = none` posts it without shooting first. A path is what it digs in on; an object is what it stands by. |
| `npc.anim` | Plays a state/animation where the NPC stands (`remark`/`animpoint`), `duration` or until the node is left. |
| `npc.follow` | The role follows the actor, another role or a **ref** (`companion`), `distance` in metres. |
| `npc.attack` | Points the role at a target (a role, the actor, a story id, a community) and makes it hostile to it. |
| `npc.stance` | Relation to the actor, goodwill, `combat_ignore`, invulnerability. |
| `npc.set_name` | The visible name of an NPC. The engine sets it directly — no character profile needed. |
| `npc.release` | Gives the NPC back to the logic that owned it before the behaviour touched it. |

#### 19.5.1 A destination, and one that is not always there

`npc.goto`, `npc.hold` and `npc_at_place` take a **`destination`** - its own
parameter type, because a step can be sent to more things than a `place` names:

```lua
place = { path  = "amb_walk" }        -- a waypoint path (the WayPoint tool)
place = { ref   = "dropped_case" }    -- something the CALLER spawned
place = { story = "esc_wolf" }        -- an object of the level
place = { role  = "leader" }          -- wherever that role's first living member stands
place = { smart = "esc_smart_1" }     -- ... and everything a place is:
place = { restrictor = "esc_zone" }
place = { pos = {12.3, 0.5, -44.0}, level = "l01_escape" }
place = { actor = true }
```

**X-Ray walks patrol paths and nothing else**, and that used to be the end of
it: a stalker could not be sent to "that case" or "over there" without a second
movement system in the scripts. Grizzly removes the limit in the engine
instead of working around it. `xms.patrol_point(name, x, y, z)` registers a
ONE-POINT RUNTIME PATROL PATH at the navmesh cell nearest to a position, so the
very same stock walker / camper walks to any destination, with danger, cover and
combat intact (game repo, `NQ_RUNTIME.md` section 15.4):

- a **drawn path is still the best destination** - it gives the walker a route,
  its look points and its stops, where anything else gives it one point;
- a runtime path is level data, not save data: nothing about it reaches
  `.scop` / `.scoc` / `.scov`, and a script that never calls the function sees no
  difference - the original game, its mods and its saves are untouched;
- the engine never frees a runtime path, it MOVES its point: the path manager of
  an NPC keeps a raw pointer, so a moved path is safe where a deleted one is not.
  The runtime pools the names (`xms_rt_<n>`) so their number stays bounded.

**A ref is not permanent, and the step does not assume it is.** A ref names
something a quest spawned and may remove again:

- **not there yet** - the step is PARKED in the instance state, exactly as it is
  for an NPC that is offline, and applied the moment the destination resolves.
  The same goes for a place on another level (a walker walks within a level) and
  for a place with no walkable cell within 15 m;
- **moved** - the path is moved with it and the people on their way are sent
  again (past `max(2 m, radius)`, at most once every two seconds);
- **gone mid-step** - the step ends as if it had arrived: the node leaves by
  `done` and the graph moves on. A behaviour never blocks forever on something
  that is not coming back.

`arrive` is measured against the path's own point - where the engine put it - so
a case on a table is reached at the foot of the table.

**A behaviour graph sees the refs of whoever runs it.** Ref lookup starts in the
instance and walks up to the caller, so `{ ref = "dropped_case" }` names the
case the QUEST dropped. That is why a ref a behaviour graph never creates is
legal there (`E030` does not apply): which quest will call it cannot be known
when the graph is validated. For a graph that must stay independent of its
caller's ref names, hand the thing over as a **role** and write `{ role = }`.

### 19.6 The caller side — `behaviour.run` and its three exits

| kind | what it does |
|---|---|
| `behaviour.run` | Starts a behaviour graph. `asset` names it, `roles` binds its roles, `ref` saves the running instance. |
| `behaviour.stop` | Ends an instance and releases its NPCs. |
| `behaviour_running` / `behaviour_done` | Conditions on an instance. |

**`behaviour.run` has three pins, and the wiring IS the mode.** There is no
`wait` flag to learn:

| pin | fires |
|---|---|
| `started` | immediately, once the cast is bound and the graph is armed |
| `finished` | when the graph reaches `flow.end` on its own |
| `aborted` | when it ended without getting there: the whole cast died, `behaviour.stop` cut it short, or the asset went missing after an update |

- wire `started` only → fire and forget, the quest carries on;
- wire `finished` only → the quest waits for the behaviour to play out;
- wire both → the quest carries on AND picks the thread back up at the end;
- wire `aborted` → handle the ambush that never happened, without writing a
  single condition for it.

`finished` and `aborted` are exclusive: exactly one of them fires, exactly once.
Nothing wired means nothing happens — a pin left loose is not an error.

```lua
{ id = "spring_the_trap", kind = "behaviour.run",
  params = {
    asset = "bridge_ambush",
    roles = {
      leader    = { story = "esc_wolf" },
      attackers = { ref = "ambush_squad" },
    },
    ref = "ambush" },
  out = {
    started  = "tell_the_player",
    finished = "after_the_fight",
    aborted  = "they_never_showed" } }
```

The same graph, another cast, another quest — that is the point of roles.

**A behaviour graph may call another one**, with the same node: nesting is how a
long routine is built out of short ones. A graph may not call itself, directly
or through a chain (`E113`).

### 19.7 Lifetime

- A behaviour ends when its graph reaches `flow.end` (→ `finished`), when
  `behaviour.stop` says so, when every role is empty, or when its asset is gone
  after an update (→ `aborted`).
- **On end every NPC it touched is released**, in reverse order of being taken.
  Release restores the logic the NPC had on entry; an NPC that had none goes
  back to its smart terrain.
- Instances survive saves like quests do (§11): the instance holds its asset id,
  the resolved role members by id, the node state and the vars.
- A caller that finishes while a behaviour it started is still running stops it
  and releases — nothing is left pinned in a scheme nothing drives.

### 19.8 Validation

On top of the shared rules (§15):

- A kind may only appear in a document the catalog allows (`docs`, §19.10):
  `E023`. This is a hard error on both sides — the editor refuses the build and
  the game's loader refuses the file.
- `who` must name a declared role (`E024`); a role never declared and never
  used is a warning, a role used but not declared is an error.
- `who` naming a `many = false` role from a step that only makes sense for a
  group is not an error — one NPC is a group of one.
- `behaviour.run` must name a behaviour graph that exists in the project
  (`E030`) and may only bind roles that graph declares; a role without a
  `default` and without a binding is an error at build time, not in game. A
  graph that runs itself is `E113` - directly in the document's own check,
  through a chain of other graphs in the project check that runs before a
  build. The runtime refuses it too (a module is not always built by the
  editor).
- Quests and behaviour graphs share ONE id space per module: the game names both
  by `<module>.<id>`, so two documents with the same id are `E002` whatever
  their kinds.
- A behaviour graph may name a ref it never creates (it sees its caller's);
  every other document still gets `E030` for one.

### 19.9 What a behaviour graph deliberately does not do

- It does not author NPCs. Who exists in the world is the spawn layer's job
  (the scene) or `spawn.squad` / `spawn.object`.
- It does not talk to the player. Dialogs stay in quests (§7).
- It does not write to the player's screen: no tasks, no map spots, no news, no
  money.
- It does not carry its own movement, cover or combat code. See §19.2.

### 19.10 The vocabulary, and why most of it is shared

The catalog tags every kind with `docs`: `quest`, `behaviour`, or both (the
default when the line is absent). The split is not half and half, and should not
be:

- **Shared (~58 kinds)** — everything that is the GRAPH rather than the subject:
  `flow.*`, `wait.*`, `var.*`, `signal.*`, `trigger.*`, `logic.any`,
  `custom.lua`, the world conditions (`npc_alive`, `relation`, `time_of_day`,
  `elapsed`, `actor_in_place`, …), the world events, `info.*`, `relation.*`,
  `spawn.*`, `squad.*`, `sound.play`, `npc.remove`, `npc.kill`, `npc.set_name`.
  Sharing these is what keeps the second graph cheap to learn.
- **Behaviour only (10)** — the role vocabulary: `npc.goto`, `npc.hold`,
  `npc.anim`, `npc.follow`, `npc.attack`, `npc.stance`, `npc.release`, and the
  conditions `npc_at_place`, `npc_in_combat`, `role_alive`.
- **Quest only (~34)** — everything the PLAYER sees or that belongs to the quest
  system: `task.*`, `objective.*`, `dialog.*`, `quest.activate`, `behaviour.*`
  and its conditions, `money.*`, `news.tip`, `map.spot`, `map.unspot`,
  `actor.teleport`, `item.give`, `item.take` (both are worded "the player"),
  and the conditions `quest_status`, `task_status`, `objective_status`.

The rule for a NEW kind: it is quest-only if it names the player or the quest
system, behaviour-only if it addresses a role, shared otherwise. An untagged
kind is shared, so a player-facing kind MUST carry `docs = quest` — the catalog
parity test in the game repo fails the build when the tag and the implementation
disagree.

---

## 20. The mode's world and standing orders

A mode is a new game, and a story mode usually wants its world to itself: fewer
(or none) of the game's own NPCs, no jobs and storylines of the base game, the
player kept to a few levels and started where the author put them, the author's
own NPCs placed and ordered about. This section is the contract for that; the
game side is `docs/dead-air/MODDING.md` "World rules" and `NQ_RUNTIME.md` of the
game repo.

### 20.1 What the author decides, and where it lives

| decision | lives in | written by |
|---|---|---|
| what the mode keeps of the original game: `original_npcs`, `random_stalkers`, `mutants`, `npc_quests`, `story_quests` (true = keep) | `mod.ltx [world]` | Mod > World Rules, `world_set` |
| where those rules apply (`levels`, empty = everywhere) | `mod.ltx [world]` | same |
| the rules the player may change on the new-game screen (`player_choice`, three at most) | `mod.ltx [world]` | same |
| the playable area (`area`, empty = anywhere) | `mod.ltx [world]` | same |
| transitions that start closed (`locked`, `locked_levels`) | `mod.ltx [world]` | same |
| where a new game starts | the spawn layer, `[obj:<name>] op = start` | the scene bake of a Player Start |
| what a new game begins with, in place of the new-game screen's loadout (items, money, whether the screen's perks stay) | `mod.ltx [start_kit]` | World Rules tab (Starting Kit), `world_set start_*` |
| the module's own patrol paths | the spawn layer, `[path:<name>]` | the scene bake of a way path |
| what a placed stalker does from its first frame | its custom data, `[xms_order]` | the scene bake of its Order properties |
| what changes while the game runs | the quest graph: `world.*`, `order.*`, `transition_open` | the quest editor |

Every rule is spelled out once the section exists (`Save` writes all five), so
the file reads as the whole decision; a world equal to the stock game writes no
section at all.

### 20.2 Non-destructive by construction

- **Nothing of the game is written.** The editor reads the linked game's
  `all.spawn` - its game graph for the levels, its objects for the level
  changers, its patrols for the names a module path must not take - and never
  writes it. The engine applies a module's world in memory while it composes a
  new game's spawn, over the game's own file.
- **Only the original population is culled.** The spawn composer removes base
  objects the rules do not keep BEFORE any module op runs; a module's own
  objects are never "the original game". Scripted spawns of the base game (the
  initial fill of the simulation, smart terrain respawns, story effects) are
  filtered on the Lua side by the same rules.
- **Modules compose.** A module's rules apply in its mode and on its `levels`;
  areas of several modules are united; a quest that opens a transition beats
  every lock; the first module's Player Start stands and a second one is logged
  as a conflict; a module path named like one of the game's is refused by the
  engine and not even exported by the editor.
- **Saves keep their world.** The world is resolved once per new game and kept
  in the save (`.scov` blob `0x584DFF02`); a module changed later does not
  rewrite a running playthrough. A save without the blob plays the stock game.

### 20.3 The editor side

- `EditorWorld` (XrECore): the all.spawn reader (levels, level changers with the
  level they stand on and the one they lead to, the game's patrol names), the
  project's Player Starts (the spawn layer, with the open scene standing in for
  its own level), validation, the World Rules tab and the MCP commands
  `world_get` / `world_set` / `world_transitions` (`world_view` looks at the tab).
- The tab saves every edit at once, like Exclude from Build: the manifest is
  re-read first, so the Mod Settings tab's own edits are never written over, and
  every tab showing a part of `mod.ltx` follows `EditorMod::ManifestGeneration`.
- **Player Start** is a point type of the Spawn tool (`ptPlayerStart`,
  `$player_start`): the actor figure, an arrow for the facing, never part of the
  level build. `place_spawn section=player_start` places it over MCP.
- A stalker's **Order** properties (`SXmsOrder` on `CSpawnPoint`, chunk
  `0xE430`) - kind, route, look path, radius, run, target, distance, hostile -
  are appended to its custom data as `[xms_order]` by the bake. An `[xms_order]`
  typed into the custom data by hand is replaced, with a warning.
- Way paths bake to `[path:<name>]` with their points, flags, point names and
  links - the same data the level build writes for a level's own patrols. MCP
  `place_way` draws one from points; the way path picker lists the linked game's
  own patrols after the scene's, since a stock route is often exactly what an
  order wants.
- `level_changer` is a picker type (`NqPickers::tLevelChanger`, fed by
  `EditorWorld`); a quest's `{ role = }` in a destination or target is `E024`.
- Codes: `E201` a second Player Start, `E202` a fourth player choice - both stop
  a build; `W203` a level and `W204` a transition the linked game lacks, `W205`
  the start outside the area, `W206` an area with no Player Start.
- **The start kit** (`SStartKit`, `mod.ltx [start_kit]`) is edited in the same tab
  and saved the same way. A row is born with its item (the picker adds it): the tab
  saves at once, and a row without one would not survive the save. The picker is
  the Characters tab's kit picker (`EditorCharacters::ItemPicker`) - one widget for
  every kit. *Game's Base Kit* reads `[shared]` of the linked game's
  `plugins\faction_loadouts.ltx`. `E207` a row with no item, a count below 1 or a
  repeated item stops a build; `W208` an item the linked game lacks only warns
  (the module may bring it - the game leaves it out rather than stop). The game
  half is `xms_world.script` (the game repo's MODDING.md, "The start kit").

### 20.4 Standing orders

An order is what an NPC does until told otherwise. It is kept per NPC in the
runtime state (`orders[id]`), not per quest: a quest ending, a save and a load,
and the NPC going offline and back all leave it in force. It is applied the way
every behaviour step is - a logic section handed to a stock scheme (walker,
camper, companion) - so combat, cover and danger stay the game's own. A running
behaviour graph that holds the NPC outranks the order; the order resumes when
the graph lets go. A new order replaces the old one; `order.clear` hands the NPC
back to whatever owned it. A placed stalker's `[xms_order]` is adopted once, the
first time it comes online, so a quest that dismissed it later is not undone by
the next load.

### 20.5 Decisions

- **Rules apply to a new game.** A save is a contract (the game's
  SAVE_COMPATIBILITY.md); a world changing under a running playthrough would
  break quests mid-way. The story changes the world through `world.*`.
- **Three player choices** - the new-game screen of the linked game has three
  free rows next to the campaign column; a fourth would be a rule the player
  never sees. Refused in the window, the MCP path and the build.
- **Only named squads count as original characters.** A squad section with a
  `story_id` is a character the game placed on purpose; a monster faction is a
  mutant; everything else is the simulation. Story effects that spawn random
  squads are part of a story beat, so only original characters are refused
  there.
- **A developer's console jump ignores the area** (`jump_to_level`), and the
  engine's `xms_world` console command shows and edits the live state - QA has to
  be able to go anywhere.
