# NQ — Node Quest Graph. Architecture of the node-based quest graph

This document describes how the node-graph quest system lands in XFined Editor
and Dead Air Refined: the `.nqasset` format, the action catalog, the execution
model, the engine changes, the editor UI, MCP, export, tests and the phased
rollout plan. It is a working implementation guide: every decision is made, no
questions stay open (the decision register and its reasons are §18). All code
anchors follow the current trees `D:\Games\XFined-Editor` (the editor) and
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
   (the editor lays them out itself), problems come back as text with codes,
   `quest_get` returns the canonical text plus a textual graph outline (§4.5).
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
8. **The editor**: a new asset type in the content browser (RMB → Create →
   Quest Graph), a separate document tab with an infinite canvas (RMB pan, 10
   zoom levels, top-down flow, on_enter/on_exit strips on a node), the
   inspector on the right, its own undo, full MCP (`quest_*`), validation =
   the build gate.
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
`Source/Editors/XrECore/Editor/EditorModManifest.cpp:843-1063`) already copies
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

```
Source\Editors\XrECore\Editor\Nq\        ; model, IO, catalog, validator, document, MCP, export gate
  NqAsset.h/.cpp        NqLua.h/.cpp        NqCatalog.h/.cpp
  NqValidate.h/.cpp     NqPickers.h/.cpp    NqDoc.h/.cpp
  NqExport.cpp          NqMcp.cpp
Source\Editors\LevelEditor\UI\QuestGraph\ ; tab window, canvas, inspector
  UIQuestGraph.h/.cpp   ; UIQuestGraph (document registry, Open/Update/Close) + UIQuestGraphWindow (per-document window)
  NqCanvas.h/.cpp       NqInspector.h/.cpp
gamedata\configs\nq\catalog.ltx           ; bundled catalog fallback (copy of the game's)
docs\NQ_ARCHITECTURE.md                   ; this document
docs\nq\examples\*.nqasset                ; reference quests (phase 0)
tools\mcp\xfined_mcp.py, tools\mcp\AI_SETUP.md ; new quest_* tools
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
- Table keys are identifiers (`id = …`), not `["id"] = …` (both variants are read,
  the canonical one is written). The exception is keys that are Lua reserved words
  (the `else` pin of `flow.branch`, the `["not"]` field of conditions): those are
  written as `["else"] = …`.
- On save the editor **rewrites the file canonically** (key order and formatting
  are fixed, §13.3): comments written by hand/AI are lost (deliberate — diffs stay
  stable), and a generated comment block with the quest outline is written at the
  top of the file (§4.5, item 7).
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
   file is logical (top-down along the flow); on save the editor sorts nodes
   topologically from the triggers (stable: links unchanged — order unchanged).
2. **Coordinates are optional.** A node without `pos` gets its position from the
   auto-layout on open; an AI never has to write `pos`. Editor fields (`pos`,
   `comment`) always come last in a node so they do not get in the way of reading.
3. **The schema is served on demand.** `xfined_quest_catalog` returns every kind
   with its groups, purpose (`use`), parameters (type/required/default/enum), pins
   and a one-line description — enough to write a quest from scratch. The same
   catalog lives in `docs\nq\NQ_FORMAT.md` (phase 0) and in the `AI_SETUP.md`
   prompt block.
4. **Problems come as text.** `xfined_quest_write` (write the whole file) /
   `xfined_quest_validate` return a list of `{code, severity, node, slot, message}`
   (§15) — the same format the game prints. A Lua syntax error position is
   line:column.
5. **Reading is text.** `xfined_quest_get` returns the canonical file text plus an
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
   The outline is available without opening a tab (`xfined_quest_list` gives a
   summary of every quest).
6. **Editing — whole-file or by operations.** An AI usually rewrites the whole
   file (`quest_write` or a direct file write + `quest_reload`); targeted edits go
   through `quest_apply` operations, also text (a Lua table).
7. **The generated header.** The canonical writer prepends a comment block with
   the same outline (rebuilt on every save, ignored on read) — opening the file in
   any editor, a human or an AI sees the quest structure without the graph.
8. **Names speak for themselves.** Kinds and parameters are short English
   identifiers with dotted namespaces (`dialog.topic`, `objective.kill`,
   `item.give`); texts are in the player's language.
9. **Screenshots are unnecessary but possible.** `xfined_quest_view` +
   `xfined_screenshot_editor` remain for visual checks, but no step of the AI
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
| `item.give` | `section`, `count=1` (item units: for ammo — boxes, not rounds) | in a dialog context — `dialogs.relocate_item_section_to_actor(npc, actor, section, count)` (`dialogs.script:1976`, with the "received" UI); otherwise `alife():create(section, actor pos, lvid, gvid, actor:id())`×count (ammo — `create_ammo`, `_g.script:1438`) + `news_manager.relocate_item(actor,"in",…)` (`news_manager.script:247`) |
| `item.take` | `section`, `count=1` \| `"all"` (same units as `item.give`: items, boxes for ammo) | in a dialog — `dialogs.relocate_item_section_from_actor` (`:2058`); otherwise iterate `db.actor:iterate_inventory` + `alife():release` |
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
  `.xspawn` (`EditorModScene.cpp:774-792` already writes `custom_data` as a file) —
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

The log prefix: `* [nq]` / `~ [nq]` / `! [nq]`.

### 12.5 Game QA

Per `PROJECT_RULES.md` §8: a hidden launch `xrEngine.exe -fsltx <qa.ltx> … -start server(<save>/single/alife/load)`,
the QA script hooks in via `[common]`, waits for `actor_on_first_update`, then
`nq jump/fire` plus checks of `nq state` and the log. Scenarios — §16.

---

## 13. Editor side: implementation

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
  `cmake/EditorTargets.cmake:118-126`, `lua51.dll` is staged): `luaL_newstate`
  without opening libraries → `luaL_loadbuffer` → an empty environment
  (`lua_newtable; lua_setfenv`) → `lua_pcall` → walking the result (`lua_next`)
  into `SNqValue`. A syntax error/non-table → a message with line:column from
  Lua. One `lua_State` per operation, closed immediately.
- Custom `code` syntax check: `luaL_loadbuffer` of the chunk without execution
  (the same path) — warning W050 "Lua syntax error".
- Writing — the canonical serializer: quest key order
  `nq, id, title, activation, vars, tasks, nodes`; nodes — in a stable
  topological order (a walk from the triggers in file order, over pins in catalog
  order, over targets in `out` order; unreachable ones — at the end in their
  previous order); node keys `id, kind, once, params, cond, on_enter, on_exit,
  out, pos, comment`; strings in `"…"` with escaping, multi-line ones (`code`,
  long texts) — `[[…]]`/`[==[…]==]`; 2-space indent; `out` — a string for one
  target, a list for several. The file always starts with the generated comment
  block: `-- nq: <id> "<title>"` and the graph outline (§4.5, item 5) — the only
  comments the editor writes; on read they are ignored and rebuilt on every save.
- The outline (§4.5) is produced by the same function for the writer, `quest_get`
  and `quest_list`.

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
- An external file change (an AI rewrote it): the `Reload` button/command; on
  save the editor compares mtime and, if the file changed outside after opening,
  asks (UI) / overwrites (MCP with `force`).

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

- `UIContentBrowser.cpp`: the `IsQuestExt(".nqasset")` predicate; a "Quest Graph"
  icon/caption on the tile; `OpenAsset` (`:846-959`) → `UIQuestGraph::Open(path, err)`;
  `DrawGridContextMenu` (`:2113-2151`) gains the **Create → Quest Graph** submenu
  (only `m_Source == 0`), creation modeled on `CreateFolder` (`:2242-2284`): a
  free name `NewQuest.nqasset`, `NewQuest2.nqasset`, the template = a minimal
  quest (`trigger.start → flow.end`), `BeginRename` — **generalized to files**
  (folder-only until now, `:2286-2359`): renaming a file = `MoveFileA`; the `id`
  inside the asset is not synchronized (the id and the file name are independent;
  the id is edited in the inspector, W061 when they differ — a hint only).
- The window: a `UIQuestGraphWindow : XrUI` class per document,
  `ImGui::Begin("Quest: <name>##<path>")` with `ImGuiWindowFlags_MenuBar`, docked
  as a tab in the center (`DockCenter`), pumped from `CLevelMain::OnDrawUI()`
  (`UI_LevelMain.cpp:3322-3341`) through `UIQuestGraph::Update()`. Inside — the
  toolbar, the canvas|inspector splitter (like `UIContentBrowser::DrawSplitter`),
  the problems row.
- Nothing that redraws the frame may be called from `Draw()` (the `CLAUDE.md`
  rule, `XrUIManager::InUIPass()`).

### 13.8 Canvas (`NqCanvas`) — per the sketch

An in-house `ImDrawList` implementation (the tree carries no node libraries,
ImGui 1.88 docking in `Source/Editors/XrEUI/`; third-party ones
(imnodes/imgui-node-editor) give no vertical flow and are version-picky).

```
┌ Quest: wolf_debt ─────────────────────────────────────┬─ Node ─────────────────┐
│ [Save][Validate][Layout][Frame][100%▾][rus▾]          │ id    [meet          ] │
│                                                       │ kind  [Dialog: topic▾] │
│        ( T · start )                                  │ npc   [Сидорович  …  ] │
│             │                                         │ text  [Слышал, у тебя…]│
│    ┌────────▼───────────┐                             │ once  [x]              │
│    │ 1 │ 2 │ 3 │ +      │  ← on_enter                 │ conds [+]              │
│    ├────────────────────┤                             │ comment [            ] │
│    │ Dialog: topic      │  ← main action              ├─ Action (enter #2) ────┤
│    │ Сидорович: «Слышал…│                             │ kind  [Map spot     ▾] │
│    ├────────────────────┤                             │ target [Сидорович  … ] │
│    │ 1 │ 2 │ +          │  ← on_exit                  │ text   [Сидорович    ] │
│    └──┬────────────┬────┘                             │                        │
│    next│           │done                              │                        │
│  ┌─────▼────┐  ┌───▼─────┐                            │                        │
│  │ …        │  │ …       │                            │                        │
│ ▸ Problems: 0 errors, 1 warning                       │                        │
└───────────────────────────────────────────────────────┴────────────────────────┘
```

- **Infinite canvas**: world coordinates are floats; the transform is
  `screen = (world - center) * zoom + origin`. Panning — held **RMB** (as in UE)
  and MMB; an RMB click without movement — the context menu.
- **10 zoom levels**: `{0.2, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0, 1.25, 1.5, 2.0}`, the
  wheel steps the level by ±1 anchored under the cursor; `Home` — frame
  everything, `F` — the selection. LOD: at zoom ≤ 0.4 chips are not drawn (only
  counter strips), texts shorten.
- **Top-down flow**: the input is the node's top edge (one point, accepts N
  edges), the outputs are the bottom edge, one pin per catalog name, captions
  under the node; edges are vertical cubic béziers with an arrow.
- **A node**: the top chip strip `on_enter` ("1 | 2 | 3 | +"), the body (the kind
  title, a summary line of key parameters: NPC/text/section…), the bottom
  `on_exit` strip ("1 | 2 | +"). Colors follow the kind family
  (dialog/objective/wait/flow/end), a trigger is an oval "T · <kind>". An
  error/warning badge; an unreachable node is dimmed; `once` nodes get a ↻̸ mark.
- **Interaction**: LMB — select/drag (everything selected), a rubber band on
  empty space, `Ctrl`+click — toggle; dragging from a pin into another node's
  body/input — an edge; dropping onto empty space — the "create a node and
  connect" menu; `Alt`+click on an edge or edge selected + `Del` — delete it;
  `Del` — delete nodes (undo, no modal); `Ctrl+Z/Y`, `Ctrl+C/V` (the clipboard =
  a Lua fragment `return { nodes = {…} }`, paste dedups ids and offsets),
  `Ctrl+A`, `Ctrl+S`, `F2` — rename the id; a double click on a chip — focus the
  lower inspector; "+" — the extra-action menu (catalog + Lua); dragging chips
  reorders them; RMB on a chip — delete/duplicate.
  Snap to a 16 px grid on release.
- **Navigation**: `Ctrl+F` — search over the whole node content (id, kind,
  parameters, conditions, actions, links, comments), `F3`/`Shift+F3` —
  next/previous result; `Ctrl+B` — a transient bookmark on the selected nodes;
  `Alt+Left/Right` — back/forward through the transient view history (center,
  zoom, selection, inspector slot). The minimap shows the whole graph and the
  viewport rectangle; clicking/dragging it pans the canvas. Search, bookmarks,
  history and minimap visibility are not serialized and do not touch dirty/undo.
- **Layout** — a deterministic pin-aware top-down layout: branches follow the
  visual pin order; the full bounds of nested subtrees are separated by a small
  gap, whole trigger trees — by a larger one. A descendant shared by several
  branches becomes a single merge block between them and does not shift the
  center of the exclusive part of any branch or trigger.
- The view state (center, zoom) lives in `NqDoc`, not in the file.

### 13.9 Inspector (`NqInspector`)

The upper section is **Node** (or **Quest** when nothing is selected): `id`, the
kind (a combo of main kinds by group; changing the kind resets the parameters,
undo), catalog-generated parameter widgets (type → widget/picker), `once`, `cond`
(a list of rows: `[not] [kind ▾] parameters… [×]`, `+ condition`; for
`wait.any/flow.branch` — the `cases` editor), the comment. For the quest: `id`,
`title`, `activation`, the `vars` table, the `tasks` table. The lower section is
**Action**: the selected chip — the kind (extra actions by group + "Lua"), the
parameters or a multi-line code editor with a "Check" button (§13.3). The
problems row under the canvas expands into a list; a click focuses the node/slot.

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

C++ — new branches in `XFinedInspector` (`UI_LevelMain.cpp:1356-2861`) via
`NqMcp.cpp`; Python — entries in `TOOLS` + `CMD_MAP` (`tools/mcp/xfined_mcp.py`),
the table in `tools/mcp/AI_SETUP.md`. Structured arguments travel as **a string
holding a Lua table** (the parser already exists — `NqLua`), responses are JSON
built by string emitters (no JSON library).

| Tool | Arguments | Response |
|---|---|---|
| `xfined_quest_catalog` | — | kinds with groups, `use`, parameters (type/required/default/enum), pins; the catalog version; the source (game/bundled) |
| `xfined_quest_list` | — | every project `.nqasset`: path, id, title, error/warning counts |
| `xfined_quest_new` | `path` (project-relative) | creates from the template, opens the tab |
| `xfined_quest_open` / `xfined_quest_close` | `path`[, `discard`] | |
| `xfined_quest_get` | `path` | `{ lua: "<canonical text>", outline: "<text outline, §4.5>", problems: [...], stats }`; works for an unopened file too (reads from disk) |
| `xfined_quest_write` | `path`, `lua` | replace the whole document (parse → validation → undo snapshot); problems and outline in the response; the file need not be open (then it is written to disk after validation, errors do not block the write — they only block the build) |
| `xfined_quest_undo` / `xfined_quest_redo` | `path` | document undo/redo (the scene `undo` does not touch quests) |
| `xfined_quest_apply` | `path`, `ops` = a Lua table of operations: `add_node{node}`, `set_node{id, patch}`, `rename_node{id,new_id}`, `remove_node{id}`, `connect{from,pin,to}`, `disconnect{from,pin,to}`, `add_action{id,slot,index,action}`, `set_action{id,slot,index,patch}`, `move_action{...}`, `remove_action{...}`, `set_quest{patch}`, `set_pos{id,x,y}` | problems after applying |
| `xfined_quest_validate` | `path` | the problem list |
| `xfined_quest_save` | `path`[, `force`] | |
| `xfined_quest_reload` | `path` | re-read from disk |
| `xfined_quest_layout` | `path` | auto-layout |
| `xfined_quest_view` | `path`, `frame:"all"\|<node>`, `zoom_level`, `cx`/`cy` (override `frame`), `select:"<node>"\|"a,b"\|"none"` (the inspector follows the selection), `slot:"enter:0"\|"exit:1"\|"none"` (open the action in the inspector), `scroll:"top"\|"bottom"\|"task:<id>"\|"task:<id>/steps"\|"action:top"\|"action:bottom"` (scroll the inspector, or the action panel that scrolls on its own) | the resulting `zoom_level`/`center`/`selected`/`slot`; the raw response marks a deferred first `frame:all` as `pending:true`, the Python bridge re-reads the view after the next Draw; then `xfined_screenshot_editor` |
| `xfined_quest_edit` | `path`, `op`, `nodes` (instead of the current selection), `from`/`pin`/`to` | every graph edit the canvas can do, without a mouse: `connect`, `disconnect` (without `to` — the whole pin, without `pin` — all pins), `bypass` (the node stays in place, the chain closes over it: A→B→C becomes A→C), `delete`, `duplicate`, `connect_nearest`, `select_all`. The response carries the outline — the result reads without a screenshot |
| `xfined_quest_find` | `path`, `action`, `query`, `select`, `limit` | full-text results in file order, the active result and the view state; `get` with `query` atomically changes the query without navigating, `next`/`previous` focus the node |
| `xfined_quest_project_find` | `query`, `limit` | a read-only search over a direct sorted scan of all `*.nqasset`, opening no documents; dirty documents act as a memory overlay. The response: `source` on each result, `complete`/`diagnostics`, the cache `generation` and a deterministic content `fingerprint` |
| `xfined_quest_references` | `type`, `value`, `path?`, `limit?` | exact references only over the current catalog's types, including nested `cond_list`/`cases_cond`; anything unknown, malformed, Lua or unreadable explicitly marks the result incomplete |
| `xfined_quest_rename_task` | `path`, `from`, `to`, `ack_runtime_identity:true` | a safe task rename inside the document: a preflight of every catalog-typed area, then one `Snapshot` + one mutation of the declaration/references. Refuses before the snapshot on unknown/Lua/malformed/partial; the ack is mandatory because of runtime/save identity |
| `xfined_quest_bookmarks` | `path`, `action`, `node` | transient bookmarks (`get/add/remove/toggle/next/previous/clear`) and the view state |
| `xfined_quest_history` | `path`, `action` | the transient history (`get/back/forward/clear`) with all view states |
| `xfined_quest_minimap` | `path`, `action` | visibility, graph/viewport bounds and the view state (`get/show/hide/toggle`) |
| `xfined_quest_lookup` | `type`, `query`, `limit` | picker data (§13.6) |
| `xfined_quest_check_all` | — | the same as the build gate |

The destructiveness rule: MCP commands show no dialogs; `quest_close` without
`discard` on a dirty document returns an `unsaved` error, `quest_write`
overwrites the document (undo exists).

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
| E022 | several mutually exclusive parameter forms picked at once, or none at all (`objective.fetch`: `item` vs `section`/`count`/`from`; `objective.kill_count`: more than one filter). The forms are declared in `NqCatalog::ExclusiveForms` — one table serving both the validator and the inspector, which dims the losing fields |
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
| W061 | the file name does not match the quest `id` — editor only |
| W062 | a spawn row sets a field its section cannot carry (a magazine on a medkit, doses on a rifle) — editor only, and only when the linked game knows the section |
| W063 | a spawn row's value is over the section's own ceiling (`ammo_mag_size` / `box_size`) — the runtime clamps it — editor only |
| W070 | a quest without `flow.end` (will finish implicitly) |

The runtime prints the same codes to the log; the editor — to the problems row
and MCP.

---

## 16. Test plan

**Editor (headless, `LevelEditor.exe -project D:\XFinedProjects\Test -nodlg` + MCP):**
1. `quest_new` → the file is created, canonical, `quest_get` = the template, 0 errors.
2. `quest_write` of the reference quest (§4.4) → 0 errors; a repeated `quest_get.lua`
   is byte-for-byte equal (canonicalization is idempotent).
3. `quest_apply` of an operation set → the expected problems (E007 on a broken edge,
   E010 on broken alternation, E020, W011 …).
4. `quest_catalog` contains every bundle kind; with a game linked — the game's.
5. `mod_export` with a broken quest → `ok:false`, the error text; with a valid one →
   the file in `<game>\modules\<id>\…\*.nqasset` is identical to the project's.
6. UI smoke: `quest_open` → `screenshot_editor` (the tab, nodes, chips, edges);
   `quest_view` at zoom 0.2 and 2.0; `quest_layout` does not change semantics
   (`quest_get` without `pos` is equal).
7. Undo: `quest_apply` ×3 → `quest_undo` ×3 → the original text; `quest_redo` ×3 →
   the final one.
8. The UI-less AI cycle: `quest_write` of a file absent on disk → created;
   `quest_get` of an unopened file → the outline and problems match the opened
   one's; `quest_list` gives the summary.

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
| An AI writes invalid files | identical error codes on both sides, `quest_write` returns the problems, the game does not crash |
| Id collisions between mods | uid = `<module>.<id>`; ref story ids and dialog/task ids carry the `nq.<module>.<quest>` prefix |
| A level change rebuilds the Lua state | `init` is idempotent, everything is restored from the blob + `refs` |
| An AI cannot see the canvas and is forced into screenshots | the text outline in `quest_get`/`quest_list` and in the file header, `pos` optional, the catalog on demand, problems as text (§4.5) |

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
| An in-house canvas on ImDrawList | imnodes / imgui-node-editor | neither is in the tree, both are horizontal, imnodes has no zoom, imgui-node-editor is pinned to an ImGui version (ours is 1.88 with patches); we need vertical flow, chips, 10 fixed zooms and MCP view control |
| One `xms.nq` blob in `.scov` | a blob per module / `.scoc` via `save_state` | do not fight the module author for their blob; `.scov` is the only sanctioned channel (SAVE_COMPATIBILITY); removing a mod breaks nothing |
| Tasks via `CGameTask` directly | `task_manager` + ltx | `task_ini` is read once at script load, there is no registration |
| Texts as raw cp1251 | generate a string table | `translate()` returns the key; a string table = one more generated file, and we generate none |
| Quest flags are `vars`, not info portions | info portions | info portions are not persisted outside the actor's `.scop` and the engine does not need them; `info.give` is kept for vanilla interop |
| String story ids | numeric `xms.story_id` | game scripts address objects by string ids (`story_objects`), the editor writes them into `custom_data` with no engine changes |
| A recursive module scan | a list in the manifest | the manifest cannot be read from Lua, one more desync point; the folder is the single source |
| Validation is the build gate | warn and export anyway | the project rule: the build refuses with a clear message; an AI gets the same codes through MCP |
| Node deletion without confirmation | a modal | the document has undo; the file is untouched until save; closing a dirty tab asks and names the asset |
| The outline and the generated file header, `pos` optional | "the graph is readable only on the canvas" | the user's requirement: an AI creates and edits quests as text, without screenshots; the outline is the same function for the writer, `quest_get` and `quest_list`, so it cannot diverge |
| Canonical node order is topological | creation order | the file reads top-down as the quest's story; the order is stable, diffs do not jump |
