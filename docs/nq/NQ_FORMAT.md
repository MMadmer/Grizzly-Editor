# NQ — the `.nqasset` quest format

One quest = one `.nqasset` file = a declarative Lua table. The editor does
**not compile** it: at mod build it only validates the asset and puts it into
the module as is, and the graph is executed by the Dead Air Refined NQ runtime.
The format is therefore a contract with the game, not an internal editor
representation.

This document describes the format, the kind catalog, the validation codes, the
MCP tools, and walks through a complete reference quest. The game side (the
execution model, persistence, engine entry points) — `docs/dead-air/NQ_RUNTIME.md`
in the game repo. The architecture contract — `docs/NQ_ARCHITECTURE.md`.

---

## 1. File rules

- UTF-8 without BOM, `\n` line endings, the `.nqasset` extension.
- The content is **one `return { … }`**. The file loads with an empty
  environment (`setfenv(chunk, {})`) in both the editor and the game: no
  libraries, no globals. Any code — a function call, a global access, `local`,
  a second return value — fails at load, the quest is rejected with `E003`,
  neighboring quests are unaffected.
- Custom Lua is allowed in exactly one place: the string parameter `code` of the
  `lua` kind (an extra action or a condition).
- Table keys are written as identifiers (`id = …`). Keys that collide with Lua
  reserved words go in brackets: `["else"] = …` (the `else` pin of
  `flow.branch`), `["not"] = true` (condition inversion), `["repeat"] = true`
  (a `trigger.when` parameter).
- A node's `pos` is **optional** — editor canvas coordinates, the game ignores
  them. Nodes without `pos` are laid out automatically on open. `comment` is an
  editor-only field too.
- On save the editor rewrites the file **canonically**: a fixed key order, a
  topological node order, 2-space indent, and a generated comment block with
  the graph outline at the top. Hand-written comments are lost in the process
  (deliberate: diffs stay stable). Writing is idempotent —
  `write(read(x)) == x`.

## 2. Schema

```lua
return {
  nq         = 1,                     -- format version (required, currently 1)
  id         = "wolf_debt",           -- [a-z0-9_], unique in the module; in-game uid = "<module>.<id>"
  title      = "Долг вежливости",     -- text
  activation = "auto",                -- "auto" (default) | "manual"
  vars       = { <name> = <bool|number|string default>, … },
  tasks      = { <task id> = { title = <text>, descr = <text>,
                               type = "additional"|"storyline",
                               target = <target_ref|place>, icon = <string>,
                               auto_complete = <bool>,     -- default true
                               -- task steps: each has its own text and its OWN map spot
                               objectives = { { id = <ident>, title = <text>, descr = <text>,
                                                target = <target_ref>, visible = <bool> }, … } }, … },
  nodes      = { <node>, <node>, … },
}

<node> = {
  id       = "meet_wolf",             -- [a-z0-9_], unique in the quest
  kind     = "dialog.topic",          -- a catalog kind with use ⊇ {main} or {trigger}
  once     = true,                    -- optional; the default comes from the catalog
  params   = { … },                   -- kind parameters
  cond     = { <cond>, … },           -- optional; AND list
  on_enter = { <action>, … },         -- in order, before the main action
  on_exit  = { <action>, … },         -- in order, after it
  out      = { <pin> = "<node id>" | { "<id>", "<id>" }, … },
  pos      = { 0, 160 },              -- editor-only
  comment  = "…",                     -- editor-only
}

<action> = { kind = "item.give", params = { section = "wpn_pm", count = 1 } }
<action> = { kind = "lua",       params = { code = [[ nq.news("Привет") ]] } }
<cond>   = { kind = "var", params = { name = "agreed", op = "eq", value = true } }
<cond>   = { kind = "has_money", params = { amount = 500 }, ["not"] = true }
```

The universal rule: **every catalog entity is `{ kind, params }`**; a condition
additionally accepts `["not"] = true`.

A pin with no targets — the branch ends. A pin with several targets activates
**all** of them (a fork). An edge into a trigger is forbidden (`E007`).

Task steps (`objectives`) are extra PDA rows of one task, each with its own map
spot; their state is read by the `objective_status` condition. With
`auto_complete` on (the default) the task completes itself once every step is
`completed`; a failed step leaves the task open.

A list parameter whose kind declares a **row schema** is typed row by row.
Three kinds do: `item.give` and `item.take` take plain rows (a section and a
count - `"all"` for a take row), and `item.spawn` adds state to its rows: a row
is what to make and everything about the thing it makes, so two rifles and a
medkit in one list do not share a condition. All three accept the one-line
spelling `section` + `count` as well - it IS one row - and refuse both
spellings at once (`E022`).

```lua
{ kind = "item.spawn", params = {
    where = { into = { story = "esc_stash_1" } },
    items = {
      { section = "wpn_ak74", condition = 0.72, ammo = 30,
        ammo_kind = "ammo_5.45x39_ap", upgrades = "up_firsta_ak74", ref = "prize_rifle" },
      { section = "medkit", count = 2, condition = 0.4 },     -- both worn the same
      { section = "ammo_5.45x39_ap", count = 3, ammo = 45 },   -- three boxes of 45
    } } }
```

**Everything unset means full.** A quest that hands out a rifle means a working
rifle: an unwritten `condition` is a new item, and an unwritten `ammo` is a full
magazine (or a full box for an ammo section) — the engine's own default of zero
rounds reads as a broken gift. Write `ammo = 0` for a deliberately empty one.

The editor reads the linked game's configs and offers a row only the fields its
section can carry: an epinephrine has no magazine and no wear, a knife has no
cartridge to choose. `upgrades` picks from the set the section can really
install (`upgrades` names GROUPS, and what installs is a group's `elements`,
plus whatever an element's `effects` unlocks), because the engine aborts on a
duplicate and crashes later on an id that does not belong. With no game linked,
or a section the game does not know, every field is offered and no capability
warning is raised — a module ships sections the linked game has never heard of.
The one-line spelling (`section` + `count`) is one row, so it may carry the same
state beside it; next to an `items` list those keys belong to no row and are
refused (`E022`).

## 3. Parameter value types

| Type | How it is written | Notes |
|---|---|---|
| `text` | `"string"` or `{ rus = "…", eng = "…" }` | UTF-8; placeholders `{var:name}`, `{actor}` |
| `string` | `"…"` | a free string (e.g. the `section` filter of `objective.kill_count`) |
| picker strings | `"…"` | `item_section`, `squad_section`, `spawn_section` (any section with `$spawn`), `level`, `smart`, `restrictor`, `community`, `info`, `var_name`, `task_id`, `objective_id` (a step of the task named by the neighboring `task` parameter), `quest_id` (`"<module>.<quest>"` or `"<quest>"` = own module), `ref_name`, `signal_name`, `spot_type`, `node_id` — each has its own picker (`quest_lookup`) |
| `int` | `5` | integer; catalog `min`/`max` are checked |
| `float` | `2.5` | `min`/`max` are checked |
| `bool` | `true` / `false` | strictly boolean |
| `enum` | `"actor"` | one of the catalog's `enum=a\|b\|c` values |
| `relation` | `"enemy"` \| `"neutral"` \| `"friend"` | |
| `value` | bool, number or string | for `var.set` / `var` |
| `count_or_all` | an integer ≥ 1 or `"all"` | `item.take` |
| `duration` | `{ seconds = N }` (real time), `{ game_minutes = N }`, `{ game_hours = N }`, `{ game_seconds = N }` (game time), or a plain number (real seconds) | |
| `npc_ref` | `{ story = "esc_2_12_stalker_wolf" }` \| `{ ref = "guard" }` \| `{ profile = "…" }` \| `{ community = "stalker", level = "l01_escape" }` | resolve: `ref` → `story` → the first online NPC by `profile`/`community` |
| `target_ref` | any `npc_ref`, or `{ ref = "boars" }` (squad/object), or `{ smart = "…" }` | |
| `object_ref`, `squad_ref` | `{ story = "…" }` \| `{ ref = "…" }` | ONE specific world object; narrower than `npc_ref` — `profile`/`community` name a kind of creature, not a thing |
| `kill_target` | `{ story = … }` \| `{ ref = … }` \| `{ spawn = { section, smart\|place, restrictor, spread, ref, hold } }` | only on `objective.kill`; `spawn` creates the squad on entry, `ref` saves it, `hold = true` pins it in place |
| `place` | `{ level = "l01_escape", pos = {x,y,z}, radius = 5 }` \| `{ restrictor = "zone_name" }` \| `{ smart = "…" }` \| `{ ref = "anchor" }` \| `{ actor = true, distance = 2 }` | `radius` defaults to 5; `ref` is where an object this quest created stands; `actor` is that many metres in front of the player, resolved when the action runs — a spawn point, not a destination (refused on `objective.reach`, `actor_in_place`, `actor.teleport`) |
| `item_list` | `{ { section = "bread", count = 2 }, { item = { ref = "prize" } } }` | rows of two shapes: a section×N or one specific object; a section appears once. A kind that CREATES or CONSUMES the items declares a **row schema** in the catalog (`rows` + `row.<name>`): its rows are sections only, typed by that schema — `item.give`, `item.take` (a row may say `count = "all"`), `item.spawn` (each row carries its own state) |
| `loot_spawn` | exactly one key: `{ place = <place> }` \| `{ into = <object_ref> }` \| `{ stash = { pick, state, within, beyond, scatter } }` \| `{ npc = <npc_ref> }` \| `{ corpse = { pick, within, beyond } }` \| `{ new_corpse = { section = <spawn_section>, place = <place> } }` | where `objective.fetch` (its `spawn`) and `item.spawn` (its `where`) put the loot they create; radii are measured from the player |
| `cases_cond` | `{ { name = "yes", cond = { … } }, … }` | case names = pins |
| `cases_weight` | `{ { name = "a", weight = 3 }, … }` | the default weight is 1, must be > 0 |
| `cond_list` | `{ <cond>, … }` | the `of` parameter of `any` |
| `lua` | `[[ … ]]` | compiled once at load |

A missing parameter takes the catalog default; a missing required one is
`E006`; a wrong type is `E006` too, the parameter is dropped.

## 4. AI-first: working with a quest as text

The project requirement: an AI creates and edits quests **as text**, without
canvas screenshots. What guarantees this:

1. **The file is self-contained.** Everything the editor knows about a quest
   lives in `.nqasset`. No sidecars.
2. **Coordinates are optional.** Write nodes without `pos` — the editor lays
   them out itself (`quest_layout` or simply opening the document). The
   canonical writer always puts the editor fields (`pos`, `comment`) last in a
   node.
3. **The schema on demand.** `xfined_quest_catalog` returns every kind with its
   groups, `use`, parameters (type/required/default/min/max/enum), pins, the
   `wait`/`once`/`event` flags and a one-line description. That is enough to
   write a quest from scratch.
4. **Problems as text.** `xfined_quest_write` / `quest_apply` /
   `quest_validate` return `problems` — a list of `{code, severity, node, slot,
   message}` with the same codes the game prints. A Lua syntax error position
   is line:column.
5. **Reading as text.** `xfined_quest_get` returns the canonical file text
   (`lua`) and an **outline** — a linear description of the graph (§5). Works
   for an unopened file too. `xfined_quest_list` gives a summary of every
   project quest.
6. **Editing — whole-file or by operations.** The usual loop is to rewrite the
   whole file (`quest_write`); targeted edits — `quest_apply` with an operation
   list.
7. **The generated header.** The canonical writer puts a comment block with the
   same outline at the top of the file: opening the `.nqasset` in any editor, a
   human or an AI sees the quest structure without the graph.
8. **Screenshots are possible but unnecessary.** `xfined_quest_view` +
   `xfined_screenshot_editor` remain for visual checks; no step of the workflow
   depends on them.

The recommended loop: `quest_catalog` → write the text → `quest_write` → read
`problems`/`outline` → `quest_apply` for spot fixes → `quest_save` →
`quest_check_all` (the same gate the build uses) → `mod_export`.

## 5. The outline format

One line per node in topological order from the triggers. This is what
`quest_get` / `quest_validate` return in the `outline` field, and the same
thing the editor writes as the comment block at the top of the file on save.

```
quest dialog_branching "Долг вежливости" (active: auto; vars: agreed, asked; tasks: kill_boars)
[T] start (trigger.start) -> meet
meet (dialog.topic; npc=story:esc_2_12_stalker_wolf; once) "Слышал, у тебя проблемы с кабанами."  next -> wolf_1
wolf_1 (dialog.npc_phrase) "Лезут из леса каждую ночь. Перебьёшь их — не обижу."  next -> accept, ask_more, decline
ask_more (dialog.actor_phrase) "А что дашь?"  on_exit: var.add(name=asked, delta=1)  next -> wolf_rich, wolf_poor
wolf_rich (dialog.npc_phrase) "…"  cond: has_money(amount=1000)  next -> back
decline (dialog.actor_phrase) "Не моё дело."  (leaf)
kill (objective.kill; target=spawn simulation_boar@esc_smart_terrain_3_16 ref=boars)  done -> report
finish (flow.end; status=completed)
```

How to read it:

- `[T]` — a trigger (root node). For a trigger with a single connected pin the
  pin name is not printed: just ` -> target`.
- In parentheses — the kind and a summary of the parameters written in the file
  (`name=value`, comma-separated); `text` goes separately in quotes; `once` is
  printed when set explicitly on the node (`; once` / `; once=false`).
- `cond:` — the node conditions joined with `and`, `not X` — inversion.
- `on_enter:` / `on_exit:` — the extra actions in order.
- At the end of the line — the pins: `pin -> target, target`. `(leaf)` — no pin
  connected.
- Problems are **not** part of the editor's outline: they come as the separate
  `problems` field of the same MCP response (§8).

> The game prints its own outline (`nq state <uid>`) by the same rules with
> three differences: the header carries the full uid `<module>.<quest>`, the
> conditions sit under an `if` key instead of `cond:`, parameters are separated
> with `;` and only those differing from the catalog default are shown, and the
> last line is `problems: none` or a `; `-separated list of
> `CODE node[slot]: message`.

## 6. Kind catalog v1

The catalog is supplied by **the game** (`configs\nq\catalog.ltx` +
`configs\nq\kinds\*.ltx` of the linked install and of installed modules); the
editor bundle (`gamedata\configs\nq\catalog.ltx`) is only the fallback when no
game is linked. `quest_catalog` shows the source (`game` / `bundled`) and the
version.

The titles and descriptions below come from the catalog. Required parameters
are **bold**, defaults follow `=`.
### 6.1 Triggers (`use = trigger`)

A trigger is a root node with no input; it stays armed while the quest is
active.

| Kind | Title | Parameters | Pins | `once` | Description |
|---|---|---|---|---|---|
| `trigger.start` | Quest start | — | `next` | `true` | Fires once when the quest activates. |
| `trigger.when` | When conditions hold | `repeat: bool = false`, `cooldown: duration`, `check_every: float = 0.25` | `next`, `off` | `false` | Fires on the rising edge of the node conditions. repeat re-arms after they turn false, cooldown limits the rate, check_every sets the poll period. The off pin fires once per trigger when the conditions stop holding. |

Trigger conditions live in the node's `cond`, not in `params`. When the list
contains an event condition (`event.*`) the trigger stops polling and is
checked only at the event's moment — the rest of the list is checked right
there.

### 6.2 Main actions (`use = main`)

| Kind | Title | Parameters | Pins | wait | `once` | Description |
|---|---|---|---|---|---|---|
| `dialog.topic` | Dialog: topic | **`npc: npc_ref`**, **`text: text`**, `initiator: enum(actor\|npc) = actor` | `next`, `done` | ✔ | `true` | Adds a talk topic to the NPC while the node token and conditions hold. next: phrases, done: a leaf phrase was said. |
| `dialog.npc_phrase` | Dialog: NPC phrase | **`text: text`** | `next` |  | `false` | NPC line. Node conditions gate visibility, the first matching phrase is spoken. |
| `dialog.actor_phrase` | Dialog: player phrase | **`text: text`** | `next` |  | `false` | Player reply option. Node conditions gate visibility. |
| `objective.kill` | Kill target | **`target: kill_target`**, `by_actor: bool = false`, `spot_task: task_id`, `spot_objective: objective_id`, `spot_done: bool = true` | `done` | ✔ | `false` | Waits for the target to die: an NPC by story id, a saved ref, or a squad from spawn (created on entry, saved under ref). spot_task points the bound task marker (spot_objective: its objective) at the target. spot_done completes the bound task or objective when the node closes. |
| `objective.kill_count` | Kill N targets | `count: int = 1`, `by_actor: bool = true`, `community: community`, `squad: object_ref`, `section: string` | `done` | ✔ | `false` | Counts kills from node entry until count is reached, survives saves. by_actor counts only player kills. Optional single filter: community, squad, or section. |
| `objective.fetch` | Fetch items | `section: item_section`, `count: int = 1`, `items: item_list`, `from: object_ref`, `spawn: loot_spawn`, `spot_task: task_id`, `spot_objective: objective_id`, `spot_done: bool = true` | `done` | ✔ | `false` | Waits until the player holds the items. items rows: a section with a count, or one concrete object (ref or story id), mixed freely. section+count is the one-row shorthand. from credits only items taken from that container. spawn creates the loot on entry and credits only those instances. spot_task (spot_objective: its objective) points the bound marker at the nearest pending source. spot_done completes the bound task or objective on close. count is in items (ammo: boxes). |
| `objective.reach` | Reach location | **`place: place`**, `spot_task: task_id`, `spot_objective: objective_id`, `spot_done: bool = true` | `done` | ✔ | `false` | Waits until the player is inside the place: a point with radius, a restrictor, a smart, or a ref created by this quest. spot_task (spot_objective: its objective) points the bound marker. spot_done completes it on close. |
| `wait.timer` | Wait | **`duration: duration`** | `done` | ✔ | `false` | Waits for game time (game_hours, game_minutes) or real seconds. |
| `wait.when` | Wait for conditions | `timeout: duration` | `done`, `timeout` | ✔ | `false` | Waits until the node conditions hold. timeout is an optional limit. |
| `wait.any` | Wait for any | **`cases: cases_cond`** | per case | ✔ | `false` | Several named condition sets, the first one to hold wins. Each case is an output pin. |
| `flow.branch` | Branch | **`cases: cases_cond`** | per case, `else` |  | `false` | Instantly takes the first case whose conditions hold, otherwise else. |
| `flow.random` | Random choice | **`cases: cases_weight`** | per case |  | `false` | Instantly picks a case at random, honoring weights. |
| `flow.join` | Join | — | `next` | ✔ | `false` | Passes through once tokens arrived over every incoming edge. |
| `flow.step` | Step | — | `next` |  | `false` | Empty main action: on_enter and on_exit only. |
| `flow.end` | Quest end | `status: enum(completed\|failed) = completed`, `restart: bool = false` | — |  | `false` | Ends the quest with the given status. restart resets state and re-activates the quest. |

Instant world operations (spawning, handouts, rewards) are **not** main
actions but extra ones (§6.3). A node is a key stage of the quest, everything
else is plumbing around it. When a stage without waiting is needed —
`flow.step`.

### 6.3 Extra actions (`use = extra`, all instant)

| Kind | Title | Parameters | Description |
|---|---|---|---|
| `item.give` | Give items | `section: item_section`, `count: int = 1`, `items: item_list` | Gives the player items: one section with a count, or a list of rows. In dialog: handed over by the NPC with a notification. count is in items (ammo: boxes). |
| `item.take` | Take items | `section: item_section`, `count: count_or_all = 1`, `items: item_list` | Takes items from the player: one section with a count, or a list of rows. count = all takes every one. count is in items (ammo: boxes). |
| `item.spawn` | Spawn items | `section: item_section`, `count: int = 1`, `items: item_list`, **`where: loot_spawn`**, `condition: float = 1`, `ammo: int`, `ammo_kind: item_section`, `upgrades: string`, `ref: ref_name` | Creates items where you say - the same destination forms as fetch. Every row carries its own state, full by default. |
| `money.give` | Give money | **`amount: int`** | Gives the player money. |
| `money.take` | Take money | **`amount: int`** | Takes money from the player. |
| `info.give` | Give info portion | **`info: info`** | Gives the player an info portion (bridge to vanilla logic). |
| `info.disable` | Remove info portion | **`info: info`** | Removes an info portion from the player. |
| `var.set` | Set variable | **`name: var_name`**, **`value: value`** | Assigns a value to a quest variable. |
| `var.add` | Add to variable | **`name: var_name`**, **`delta: float`** | Adds a number to a numeric quest variable. |
| `signal.send` | Send signal | **`name: signal_name`** | Emits a signal for event.signal triggers and waits, including other quests. |
| `quest.activate` | Activate quest | **`quest: quest_id`** | Activates another quest declared with activation = manual. |
| `task.give` | Give task | **`task: task_id`** | Creates a PDA task from the quest tasks declaration. |
| `task.complete` | Complete task | **`task: task_id`** | Marks a PDA task completed. |
| `task.fail` | Fail task | **`task: task_id`** | Marks a PDA task failed. |
| `task.remove` | Remove task | **`task: task_id`** | Removes a task from the PDA without a status. |
| `task.set_target` | Task target | **`task: task_id`**, `target: target_ref` | Re-points the task marker at a target. No target clears the marker. |
| `task.set_text` | Task text | **`task: task_id`**, `new_title: text`, `new_descr: text` | Changes the task title and/or description. |
| `task.objective_complete` | Complete objective | **`task: task_id`**, **`objective: objective_id`** | Marks one task objective completed. With auto_complete on, the last one also closes the task. |
| `task.objective_fail` | Fail objective | **`task: task_id`**, **`objective: objective_id`** | Marks one task objective failed. |
| `task.set_objective_target` | Objective target | **`task: task_id`**, **`objective: objective_id`**, `target: target_ref` | Re-points one objective marker. No target clears it. Each objective owns its marker. |
| `task.set_objective_text` | Objective text | **`task: task_id`**, **`objective: objective_id`**, `new_title: text`, `new_descr: text` | Changes one objective title and description. |
| `task.set_objective_visible` | Objective visibility | **`task: task_id`**, **`objective: objective_id`**, `visible: bool` | Shows or hides one objective in the PDA. A hidden objective still runs and can be completed. |
| `spawn.squad` | Spawn squad | **`section: squad_section`**, `smart: smart`, `place: place`, `restrictor: restrictor`, `spread: float = 1`, `ref: ref_name`, `hold: bool = true` | Creates a squad of the section. smart is optional when place is set (the nearest one is used). place puts them there, inside a restrictor each member gets its own navmesh point and spread pulls the area toward the center. restrictor pens them in, hold pins the squad to its smart, ref saves the squad. |
| `spawn.object` | Spawn object | **`section: spawn_section`**, **`place: place`**, `yaw: float = 0`, `dead: bool = false`, `ref: ref_name` | Creates an object of any spawn section at a place. yaw turns it (degrees), dead lays a creature down as a corpse. ref saves it. |
| `squad.move` | Move squad | **`target: target_ref`**, `smart: smart`, `follow_actor: bool = false` | Sends the squad to a smart or after the player (follow_actor). |
| `squad.remove` | Remove squad | **`target: target_ref`** | Removes the squad from the world. |
| `object.remove` | Remove object | **`target: object_ref`** | Removes an object created by this quest (spawn.object) or one by story id. The ref is cleared afterwards. |
| `npc.remove` | Remove NPC | **`npc: npc_ref`** | Removes the NPC from the world. |
| `npc.kill` | Kill NPC | **`npc: npc_ref`** | Kills the NPC. |
| `npc.set_name` | Set NPC name | **`npc: npc_ref`**, **`name: text`** | Renames the NPC - the name over their head, in the dialog window and in the PDA. No character profile is needed for it. An NPC that is offline is renamed the moment it comes online, so the name survives being set far from the player. |
| `relation.set` | Set relation | **`who: npc_ref`**, **`value: relation`** | Makes an NPC, squad, or community an enemy, neutral, or friend of the player. |
| `relation.goodwill` | Change goodwill | **`who: npc_ref`**, **`delta: int`** | Adds (or subtracts) NPC or community goodwill toward the player. |
| `news.tip` | PDA message | **`text: text`**, `sender: npc_ref`, `icon: string`, `duration: float = 5` | Shows a news tip to the player. sender sets the author. |
| `map.spot` | Map spot | **`target: target_ref`**, `text: text`, `spot: spot_type = secondary_task_location` | Puts a map spot on the target. |
| `map.unspot` | Remove map spot | **`target: target_ref`**, `spot: spot_type = secondary_task_location` | Removes the map spot from the target. |
| `dialog.force` | Start dialog | **`npc: npc_ref`**, `allow_break: bool = true` | Force-opens a dialog with the NPC (must be nearby and online). |
| `dialog.break` | Break dialog | — | Closes the current dialog. |
| `actor.teleport` | Teleport player | **`place: place`** | Moves the player to a point on the current level. |
| `sound.play` | Play sound | **`theme: string`** | Plays a sound theme on the player. |
| `lua` | Custom Lua | **`code: lua`** | Custom Lua. As an action it runs, as a condition it must return true or false. The nq object and the full game API are available. |

### 6.4 Conditions (`use = cond`)

Allowed anywhere a `cond` exists: on a node, on a phrase, in `flow.branch`
and `wait.any` cases.

| Kind | Title | Parameters | Description |
|---|---|---|---|
| `lua` | Custom Lua | **`code: lua`** | Custom Lua. As an action it runs, as a condition it must return true or false. The nq object and the full game API are available. |
| `has_info` | Has info portion | **`info: info`** | The player has the info portion. |
| `has_item` | Has item | **`section: item_section`**, `count: int = 1` | The player holds at least count items of the section. count is in items (ammo: boxes). |
| `has_money` | Has money | **`amount: int`** | The player has at least amount money. |
| `var` | Variable | **`name: var_name`**, `op: enum(eq\|ne\|lt\|le\|gt\|ge) = eq`, **`value: value`** | Compares a quest variable with a value. op: eq, ne, lt, le, gt, ge. |
| `node_done` | Node done | **`node: node_id`** | A once node has already run. |
| `quest_status` | Quest status | **`quest: quest_id`**, **`is: enum(inactive\|active\|completed\|failed)`** | Another quest is in the given status. |
| `task_status` | Task status | **`task: task_id`**, **`is: enum(none\|active\|completed\|failed)`** | A quest task is in the given status. |
| `objective_status` | Objective status | **`task: task_id`**, **`objective: objective_id`**, **`is: enum(none\|active\|completed\|failed)`** | One task objective is in the given status, so the graph can branch on a single objective. |
| `actor_on_level` | Player on level | **`level: level`** | The player is on the level. |
| `actor_in_place` | Player in place | **`place: place`** | The player is inside a point with radius, a restrictor, a smart, or a quest-created ref. |
| `npc_alive` | NPC alive | **`npc: npc_ref`** | The NPC exists and is alive. |
| `npc_dead` | NPC dead | **`npc: npc_ref`** | The NPC is dead or does not exist. |
| `squad_alive` | Squad alive | **`target: target_ref`** | The squad (ref or story) exists. |
| `actor_community` | Player community | **`community: community`** | The player belongs to the community. |
| `relation` | Relation | **`who: npc_ref`**, **`is: relation`** | The NPC or community is an enemy, neutral, or friend of the player. |
| `time_of_day` | Time of day | **`from: int`**, **`to: int`** | The in-game hour is within from..to (midnight wrap supported). |
| `elapsed` | Time elapsed | `from: enum(quest\|node) = quest`, `node: node_id`, **`duration: duration`** | At least duration passed since quest activation (from=quest) or node entry (from=node). False when the node has no live token. |
| `chance` | Chance | **`percent: int`** | Randomly true with the given percent. |
| `any` | Any of | **`of: cond_list`** | True when at least one nested condition is true. |

### 6.5 Event conditions (`use = cond`, `event = true`)

True exactly at the moment of the event. Allowed **only** in `trigger.when`,
`wait.when` and `wait.any` — anywhere else it is `E020`.

| Kind | Title | Parameters | Description |
|---|---|---|---|
| `event.item_taken` | Event: item taken | `section: item_section` | The player picked up an item (empty section: any). |
| `event.item_dropped` | Event: item dropped | `section: item_section` | The player dropped an item. |
| `event.item_used` | Event: item used | `section: item_section` | The player used an item. |
| `event.npc_killed` | Event: NPC killed | `who: npc_ref`, `by_actor: bool = false` | An NPC died (empty who: any). by_actor: only player kills. |
| `event.squad_dead` | Event: squad dead | **`target: squad_ref`** | The squad (ref or story) ceased to exist. |
| `event.info_given` | Event: info given | **`info: info`** | The player received an info portion. |
| `event.level_entered` | Event: level entered | **`level: level`** | The player arrived on the level. |
| `event.talk_started` | Event: talk started | `npc: npc_ref` | The player started a dialog with the NPC (empty npc: any). |
| `event.talk_ended` | Event: talk ended | `npc: npc_ref` | The player closed a dialog with the NPC. |
| `event.signal` | Event: signal | **`name: signal_name`** | A signal.send signal was received. |
| `event.trade_done` | Event: trade | `npc: npc_ref` | The player completed a trade (empty npc: any). |

### 6.6 Extending the catalog from a module

A module ships `gamedata\configs\nq\kinds\<mod>.ltx` with new `[nq.*]` sections
and registers the implementations in its own `scripts\register.script` via
`xms.registry.get("nq.kinds"):add("<kind>", { … })`. The editor scans the same
folder of the linked game and of installed modules (both roots — `modules\` and
`mods\`), so such kinds show up in the dropdowns by themselves. The kind
implementation contract — `NQ_RUNTIME.md` §4.3.

---

## 7. Validation codes and what to do about them

`E` blocks the mod build and the quest's load in the game, `W` only warns.
A problem line: `CODE node[slot]: message`, where the slot is `param:<name>`,
`cond:<i>`, `enter:<i>`, `exit:<i>`, `out:<pin>` or `cond:<i>/case:<j>`.

| Code | What is wrong | What to do |
|---|---|---|
| `E001` | no `nq` or the version is unsupported | put `nq = 1` as the first line of the table |
| `E002` | the quest `id` is not `[a-z0-9_]` or taken by another quest | rename the `id`; it need not match the file name (a mismatch is only `W061`) |
| `E003` | the file is not one declarative table | remove everything that is not data: calls, `local`, concatenation, a second `return`. The syntax error position is in the message |
| `E004` | a node `id` is empty, not `[a-z0-9_]` or duplicated | give the nodes unique lowercase ids |
| `E005` | an unknown kind, or a kind in the wrong position | check `quest_catalog`: the kind's `use` must contain `main`/`trigger` for a node, `extra` for `on_enter`/`on_exit`, `cond` for a condition |
| `E006` | a required parameter is missing / wrong type / outside the range or enum | look the type up in the catalog (§3, §6); the same code catches bad `activation`, `vars`, `tasks`, `once` |
| `E007` | the pin is not declared on the kind; the edge target does not exist; the edge leads into a trigger | take pins from the catalog (for `wait.any`/`flow.*` the pins = case names); check the target id for a typo; a trigger can only be a source |
| `E008` | not a single trigger, or no `nodes` list | add a `trigger.start` (or `trigger.when`) |
| `E009` | `flow.join` with no incoming edges | connect branches to it or remove the node |
| `E010` | speaker alternation broken, or a phrase unreachable from its topic | put a phrase of the other side ("…") between two lines of the same side; a phrase must be reachable from a `dialog.topic` over `next` |
| `E011` | an edge into a phrase from a non-dialog node | a dialog is entered only through `dialog.topic`; route world edges into the topic, not into a phrase |
| `E020` | an event condition outside `trigger.when` / `wait.when` / `wait.any` | replace it with a level condition (`has_item` instead of `event.item_taken`) or move the event into its own `trigger.when` |
| `E021` | `cases` are empty, a case name is bad or duplicated, `weight ≤ 0` | every case needs a unique `[a-z0-9_]` `name`; weights strictly > 0 |
| `E022` | several mutually exclusive parameter forms at once, or none at all (`objective.fetch`: `items` vs `section`, `from` vs `spawn`; `objective.kill_count`: more than one filter); or `item.spawn` state written beside an `items` list | pick exactly one form; the inspector dims the losing fields. For `item.spawn` move the state into the row it applies to |
| `E030` | a reference to an undeclared `var` / `task` / `objective` / `node` / `quest`, or a `ref` nothing creates | declare the variable in `vars`, the task (and its steps) in `tasks`, the object via `spawn.*` or `objective.kill{spawn}` before use; for a foreign quest write `"<module>.<quest>"`. The game answers with the same code, so a quest with such a reference simply will not load there |
| `E031` | `actor.teleport` to a place on another level | Dead Air has no cross-level teleports — transitions go through a level changer |
| `E050` | a syntax error in custom Lua | fix the `code`; the "check syntax" button in the inspector verifies it separately |
| `W011` | continuations of a non-leaf phrase: among player replies **not all** are unconditional, or among NPC lines not a single one is (unconditional = no `cond` and no `once`) | the runtime will add a hidden leaf ("Пока." / "…"). For NPC lines it is unreachable, so one unconditional line is enough. For player replies it is a visible row that closes the talk past the topic's `done` — make every reply unconditional, or move the logic off `done` into the replies themselves |
| `W012` | a `dialog.topic` with no continuation phrases | connect `next` to a phrase, otherwise the topic closes right after opening |
| `W013` | a topic with `once = false` and no conditions | add a `cond` or `once = true`, otherwise the topic hangs on the NPC forever |
| `W020` | a node unreachable from the triggers | connect it or delete it |
| `W021` | a waiting node with no outgoing edges | the quest will hang on it — connect a pin or finish with `flow.end` |
| `W022` | one target listed twice on one pin | remove the duplicate; the repeated edge is ignored |
| `W030` | a `ref` is used before the node that creates it (not along all paths) | order the graph so `spawn.*` / `objective.kill{spawn}` runs first on every path to the use |
| `W031` | `spawn.squad` with `hold = false` | the squad may wander off into the simulation; keep `hold = true` when the target must wait for the player in place |
| `W040` | a character outside cp1251 in a text, or in a string `vars` default | drop the emoji/exotic typography — in the game they become `?`. A string variable counts because `{var:name}` splices it into task and phrase text |
| `W050` | Lua could not be checked (no LuaJIT) | editor only; review the code by hand |
| `W060` | the value is not found in the linked game's index | a typo in the section/story id/level/smart/profile/info portion — or it is your own mod's object, then the warning is expected |
| `W061` | the quest `id` is not the one its file name gives (lower case, anything outside `[a-z0-9_]` as `_`: `NewQuest.nqasset` gives `newquest`) | a hint, not an error; rename the file or the `id` if you want order |
| `W062` | a spawn row sets a field its section cannot carry (a magazine on a medkit, a cartridge on a knife) | drop the field, or check the section — it is only a warning, because the editor reads the LINKED game's configs and a module's own sections are not in them |
| `W063` | a spawn row's value is over the section's own ceiling (`ammo_mag_size`, `box_size`) | the runtime clamps it, so this is a warning; an installed upgrade can legitimately raise a magazine |
| `W070` | the quest has no `flow.end` | it will finish implicitly when tokens and triggers run out; add a `flow.end` if you want an explicit status |

---

## 8. The `quest_*` MCP tools

The editor listens on TCP `127.0.0.1:28016`, one JSON object per line.
Structured arguments travel as **a string holding a Lua table**, responses are
JSON. Common response fields: `ok`, on failure `error`. Most commands respond
with `path`, `id`, `title`, `nodes`, `errors`, `warnings`, `problems`
(a list of `{code, severity, node, slot, message}`), `outline`, `open`,
`dirty`, `undo`, `redo`.

Destructiveness: MCP commands show no dialogs. `quest_close` on an unsaved
document answers with an `unsaved` error unless `discard: true` is passed.

| Tool | Arguments | Response |
|---|---|---|
| `xfined_quest_catalog` | — | `source` (`game`/`bundled`), `version`, `api`, `files`, `count`, `kinds[]` — per kind: `id`, `group`, `title`, `desc`, `use[]`, `params[]` (`name`, `type`, `required`, `default`, `min`, `max`, `enum[]`), `pins[]`, `wait`, `once`, `event`, `impl`, `since` |
| `xfined_quest_list` | — | `count`, `quests[]`: `path`, `readable`, `id`, `title`, `nodes`, `errors`, `warnings`, `open`, `dirty`. An unreadable file — `readable:false` + `error` |
| `xfined_quest_new` | `path` | creates the file from the template (`trigger.start → flow.end`, `id` = the file name, title "New quest"), opens the tab. Refuses when the file exists. Responds with the summary and `lua` |
| `xfined_quest_open` | `path` | opens the document (reads the file, lays out `pos`-less nodes, validates). Re-opening just returns the state |
| `xfined_quest_close` | `path`, `discard?` | `{ok:true}` |
| `xfined_quest_get` | `path` | the summary + `lua` (canonical text) + `outline`. Works for an unopened file too (reads from disk) |
| `xfined_quest_write` | `path`, `lua` | replaces the quest whole. An open document — in memory with an undo snapshot (`saved:false`, then `quest_save`); a closed or new file — the canonical text goes to disk (`saved:true`). Problems are returned but do not block the write — only the build |
| `xfined_quest_undo` / `xfined_quest_redo` | `path` | undo/redo in the document's own ring (the scene undo is untouched). Error `nothing to undo` when empty |
| `xfined_quest_apply` | `path`, `ops` | applies the operation list (all or nothing, one undo step). The response + `applied` — how many operations went through |
| `xfined_quest_validate` | `path` | the summary + `outline`; an open document is re-validated |
| `xfined_quest_save` | `path`, `force?` | `path`, `errors`, `warnings`. Refuses with `modified externally` when the file changed on disk after loading — then `force: true` |
| `xfined_quest_reload` | `path` | re-reads the open document from disk (drops unsaved edits and the undo history) |
| `xfined_quest_layout` | `path`, `all?` | a deterministic pin-aware top-down layout: full bounds of nested branches and of exclusive trigger trees do not intersect, shared descendants take their own merge blocks between the incoming branches. `all:false` moves only `pos`-less nodes. Response: `moved`, `positions[]` (`id`, `x`, `y`) |
| `xfined_quest_view` | `path`, `frame?` (`"all"` or a node id), `zoom_level?` (0..9, 6 = 100%), `cx?`, `cy?`, `select?` (`"<node>"`/`"a,b"`/`"none"` — the inspector follows), `slot?` (`"enter:0"`/`"exit:1"`/`"none"` — opens the action in the inspector), `scroll?` (`"top"`/`"bottom"`/`"task:<id>"`/`"task:<id>/steps"` — scrolls the inspector; `"action:top"`/`"action:bottom"` — the action panel, which scrolls on its own) | opens/focuses the graph tab and sets the view. The raw response: `path`, `zoom_level`, `zoom`, `center`, `nodes`, `open_tabs`, `pending`; the first `frame:all` may be `pending:true` until Draw, the Python bridge transparently re-reads the final view after the frame. Then — `xfined_screenshot_editor` |
| `xfined_quest_edit` | `path`, `op`, `nodes?` (instead of the current selection), `from`/`pin`/`to` | every graph edit the canvas can do, without a mouse: `connect`, `disconnect` (without `to` — the whole pin, without `pin` — all pins), `bypass` (the node stays, the chain closes over it: A→B→C becomes A→C), `delete`, `duplicate`, `connect_nearest`, `select_all`. The response carries the outline — the result reads without a screenshot |
| `xfined_quest_find` | `path`, `action?`, `query?`, `select?`, `limit?` | full-text node search over ids/kinds/parameters/conditions/actions/links/comments; `get` with `query` atomically changes the query without navigating, `next`/`previous` focus the result. Tokens are AND-joined, quotes keep a phrase, `-token` excludes |
| `xfined_quest_project_find` | `query`, `limit?` | a read-only search over all sorted project `*.nqasset` without opening documents; the query syntax matches `quest_find`. A dirty open document overlays its disk copy. Results carry `source` (`disk`/`memory`); the response — `complete`, `diagnostics`, `generation`, a deterministic `fingerprint` |
| `xfined_quest_references` | `type`, `value`, `path?`, `limit?` | exact references only in parameters the current catalog typed as `type`; `path` narrows to one quest and is effectively mandatory for a local `task_id`. The walk includes nested `cond_list`/`cases_cond`; an unknown kind/parameter, a malformed value, Lua or an unreadable file give `complete:false` with precise diagnostics rather than fake completeness |
| `xfined_quest_rename_task` | `path`, `from`, `to`, `ack_runtime_identity:true` | renames the task declaration and every catalog-proven `task_id` reference inside one document as one undo step. Refuses before the `Snapshot` on an unknown/Lua/malformed/partially read area. The ack is mandatory: the task id is stored in runtime/save state, old state does not migrate |
| `xfined_quest_bookmarks` | `path`, `action?`, `node?` | transient node bookmarks (`get/add/remove/toggle/next/previous/clear`); do not touch the asset/dirty/undo |
| `xfined_quest_history` | `path`, `action?` | the transient view history (`get/back/forward/clear`): center, zoom, selection and the inspector slot |
| `xfined_quest_minimap` | `path`, `action?` | the interactive transient minimap (`get/show/hide/toggle`); the response carries the graph and viewport bounds |
| `xfined_quest_lookup` | `type`, `query?`, `limit?` (default 50), `path?` | picker data. `type`: `item_section`, `squad_section`, `level`, `smart`, `story_id`, `profile`, `community`, `info`, `spot_type` (from the linked game, cached) or the document-side `task_id`, `var_name`, `ref_name`, `node_id` (need `path`), `quest_id` (project quests). A `ref_name` entry carries its category (`squad`/`object`/`item`) in `extra`, the same hint the picker shows a human |
| `xfined_quest_check_all` | — | the build gate: `passed`, `errors`, `warnings`, `error`, `log[]` (lines `<path>: <code> <node>: <message>`) |

### 8.1 `quest_apply` operations

`ops` is the text of a Lua list (`return` optional). Indices are **1-based**.

```lua
{
  { op = "add_node",     node = { id = "ask", kind = "dialog.actor_phrase",
                                  params = { text = "А что дашь?" }, out = { next = "wolf_rich" } } },
  { op = "set_node",     id = "ask", patch = { once = true, params = { … }, cond = { … },
                                               on_enter = { … }, on_exit = { … }, out = { … },
                                               kind = "…", pos = { 0, 160 }, comment = "…" } },
  { op = "rename_node",  id = "ask", new_id = "ask_more" },   -- updates references
  { op = "remove_node",  id = "ask" },                        -- drops the edges leading into it
  { op = "connect",      from = "wolf_1", pin = "next", to = "ask_more" },
  { op = "disconnect",   from = "wolf_1", pin = "next", to = "ask_more" }, -- without `to` — the whole pin
  { op = "add_action",   id = "ask_more", slot = "exit", index = 1,
                         action = { kind = "var.add", params = { name = "asked", delta = 1 } } },
  { op = "set_action",   id = "ask_more", slot = "exit", index = 1, patch = { kind = "…", params = { … } } },
  { op = "move_action",  id = "ask_more", slot = "exit", index = 1,
                         to_id = "accept", to_slot = "enter", to_index = 1 },
  { op = "remove_action",id = "ask_more", slot = "exit", index = 1 },
  { op = "set_quest",    patch = { id = "…", title = "…", activation = "manual",
                                   vars = { … }, tasks = { … } } },
  { op = "set_pos",      id = "ask_more", x = 0, y = 320 },
}
```

---

## 9. The reference quest, node by node

`docs/nq/examples/dialog_branching.nqasset` in full:

```lua
-- nq: dialog_branching "Долг вежливости"
-- Wolf (Cordon, the rookie village) wants the boars culled. The reference for the test plan (§16 game item 2):
-- two topics on one NPC, a player reply fork, an NPC phrase gated by a condition (has_money),
-- a phrase loop through a player reply, once on the topic, a dialog exit into objective.kill{spawn} and
-- the reward in the on_exit of the final phrase.
return {
  nq = 1,
  id = "dialog_branching",
  title = "Долг вежливости",
  activation = "auto",
  vars = { agreed = false, asked = 0 },
  tasks = {
    kill_boars = { title = "Отстрел кабанов",
                   descr = "Волк просил перебить кабанов у деревни новичков.",
                   type = "additional", target = { ref = "boars" } },
  },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "meet" } },

    -- topic 1: the offer
    { id = "meet", kind = "dialog.topic",
      params = { npc = { story = "esc_2_12_stalker_wolf" }, text = "Слышал, у тебя проблемы с кабанами." },
      once = true,
      out = { next = "wolf_1" } },
    { id = "wolf_1", kind = "dialog.npc_phrase",
      params = { text = "Лезут из леса каждую ночь. Перебьёшь их — не обижу." },
      out = { next = { "accept", "ask_more", "decline" } } },

    { id = "ask_more", kind = "dialog.actor_phrase", params = { text = "А что дашь?" },
      on_exit = { { kind = "var.add", params = { name = "asked", delta = 1 } } },
      out = { next = { "wolf_rich", "wolf_poor" } } },
    { id = "wolf_rich", kind = "dialog.npc_phrase",
      params = { text = "Да ты, я смотрю, и так не бедствуешь. Ствол дам, патроны — с меня." },
      cond = { { kind = "has_money", params = { amount = 1000 } } },
      out = { next = "back" } },
    { id = "wolf_poor", kind = "dialog.npc_phrase",
      params = { text = "Ствол дам и денег подкину. По карманам вижу — не помешает." },
      out = { next = "back" } },
    { id = "back", kind = "dialog.actor_phrase", params = { text = "Понял. Так что там с кабанами?" },
      out = { next = "wolf_1" } },

    { id = "accept", kind = "dialog.actor_phrase", params = { text = "Помогу." },
      on_exit = {
        { kind = "var.set", params = { name = "agreed", value = true } },
        { kind = "task.give", params = { task = "kill_boars" } },
      },
      out = { next = "kill" } },
    { id = "decline", kind = "dialog.actor_phrase", params = { text = "Не моё дело." } },

    -- the objective
    { id = "kill", kind = "objective.kill",
      params = { target = { spawn = { section = "simulation_boar", smart = "esc_smart_terrain_3_16", ref = "boars", hold = true } } },
      out = { done = "report" } },

    -- topic 2: the report
    { id = "report", kind = "dialog.topic",
      params = { npc = { story = "esc_2_12_stalker_wolf" }, text = "Кабаны мертвы." },
      once = true,
      out = { next = "wolf_thanks" } },
    { id = "wolf_thanks", kind = "dialog.npc_phrase", params = { text = "Молодец. Держи, как обещал." },
      on_exit = {
        { kind = "item.give",  params = { section = "wpn_pm", count = 1 } },
        { kind = "money.give", params = { amount = 1500 } },
        { kind = "task.complete", params = { task = "kill_boars" } },
      },
      out = { next = "finish" } },

    { id = "finish", kind = "flow.end", params = { status = "completed" } },
  },
}
```

What happens in the game, node by node:

**The header.** `activation = "auto"` — the quest activates on the actor's
first update in any game and on any load, as long as the module is enabled,
matches by `mode=`, and the state has no record of the quest. So the quest
reaches an already started playthrough too — it simply starts on the next
load. The variables `agreed` and `asked` are created with their defaults;
`tasks.kill_boars` is only a declaration so far, there is no PDA task yet.

**`start` (`trigger.start`).** Arms on activation and fires immediately: the
`next` pin queues a transition to `meet`. The node is `once` by the kind's
default, so it never fires twice.

**`meet` (`dialog.topic`, `once = true`).** A token lands on the node and
stays (the kind waits). While the token is here, the runtime answers the
engine's "which topics does this NPC have" query: the topic enters Wolf's
list because `npc = { story = "esc_2_12_stalker_wolf" }` matches the partner
and the node has no conditions. The engine builds one dialog
`nq.<module>.dialog_branching.meet` out of this branch, with root phrase `"0"`
being the topic text.

**`wolf_1` (`dialog.npc_phrase`).** Depth 1 from the root — the partner speaks
(the initiator is `actor`, so odd depth = the NPC). The line is unconditional.
Its `next` lists three player replies **in display order**: `accept`,
`ask_more`, `decline`.

**`ask_more` → `wolf_rich` / `wolf_poor`.** The player reply "А что дашь?"
bumps `asked` by 1 in its `on_exit` — synchronously, right after the phrase is
said, so the next level of phrases already sees the new value. Then the NPC
picks the **first in order** line whose condition holds: `wolf_rich` carries
`has_money(amount = 1000)`, `wolf_poor` has no conditions. A player with money
hears the first, a poor one — the second. The order is no accident: the
runtime hands the phrases goodwill `-10000 - index` and the engine sorts by
descending goodwill — "the first declared wins". The unconditional `wolf_poor`
at the end of the list clears `W011` and rules out
`R_ASSERT2("No available phrase to say")`: `ask_more` has a continuation that
can never be filtered out. The runtime still adds its hidden leaf — it is
added whenever **not all** continuations are unconditional, and `wolf_rich` is
conditional — but here the leaf is harmless: it sits at minimal goodwill,
below `wolf_poor`, and NPC lines are picked by the engine, so the queue never
reaches it. With player replies the same layout is already dangerous: every
passing option is displayed, the leaf becomes a visible row, and choosing it
closes the talk without crediting the topic — no topic `on_exit`, no `done`
pin, the token stays on the topic. That is why for `dialog.actor_phrase` the
`W011` complains already at "not all are unconditional".

**`back` → `wolf_1`.** A loop: the player reply leads back to the NPC's first
line. Cycles in the phrase graph are legal — on a repeated id the engine just
adds an edge. The player may ask "а что дашь?" any number of times (`asked`
counts them) until picking `accept` or `decline`.

**`accept`.** Two extra actions in `on_exit`, in order: `var.set` sets
`agreed = true`, `task.give` creates the PDA task
`nq.<module>.dialog_branching.kill_boars` with the title and description from
the `tasks` declaration. The task has `target = { ref = "boars" }` — at this
point the squad does not exist yet, so the map spot appears later, once the
`ref` is created. `out.next = "kill"` is **an exit from the dialog into the
world**: `kill` is not a phrase node, so the transition is queued and runs on
the next frame, not inside `SayPhrase`. `accept` has no dialog children, so
the phrase is a leaf: the talk closes, the topic counts as passed, `once`
removes the token from `meet` and the `done` pin fires (unconnected on `meet` —
the branch simply ends there).

**`decline`.** A leaf with no `out` at all: the player refused, the topic
closed, the quest stays active with a token… nowhere. No tokens remain, no
armed triggers either (`start` has fired and it is `once`) — the runtime closes
the quest **implicitly** with status `completed` and logs it. The validator
does not mind: `W070` looks at the quest as a whole, and the quest does have a
`flow.end` (`finish`), so this asset reports `problems: none`. Want an
explicit status on refusal — route `decline` to its own `flow.end` with
`status = "failed"`.

**`kill` (`objective.kill`).** On entry `begin` resolves the target: it has
`spawn`, so the runtime creates a `simulation_boar` squad on smart
`esc_smart_terrain_3_16` and remembers it under `ref = "boars"` (`hold = true`
pins the squad to the smart — otherwise the simulation would walk the boars
away; the engine does not persist `scripted_target`, so the runtime re-applies
it after every load and on `squad_on_register`). The ref is also registered as
story id `nq.<module>.dialog_branching.boars`, and the task marker gets its
object. Then the node waits: it is subscribed to `npc_killed`,
`squad_npc_death`, `squad_dead`, plus polls the server object every 250 ms —
offline deaths deliver no callbacks. When nobody is left in the squad, the
`done` pin fires → `report`.

**`report` (`dialog.topic`, `once = true`).** A second topic on the same Wolf.
It appears in the list only now, when it holds a token — before that it simply
did not exist. One NPC comfortably carries any number of topics from
different quests and modules.

**`wolf_thanks`.** An NPC line at depth 1. Three actions in `on_exit`, in
order: `item.give` (in a dialog context the handover goes through the NPC, so
the player sees the "received" window), `money.give` (same for the money) and
`task.complete` — the PDA task closes as completed with a news line. The
phrase has no phrase children → a leaf: the talk closes, the `report` topic is
passed, `once` removes the token, and `out.next = "finish"` takes the flow
into the world.

**`finish` (`flow.end`).** The terminal: removes all tokens (none are left),
disarms the triggers and sets status `completed`. An explicit end — unlike the
`decline` branch, where the implicit completion kicked in.

To check in the game: `nq list` shows
`<module>.dialog_branching [active] "Долг вежливости" tokens=1`,
`nq state <uid>` — the token, the `agreed`/`asked` values, the `boars` ref,
the task status and the full graph outline.

## 10. Scenarios — graphs that drive NPCs

A quest tracks the player. A **scenario** drives NPCs: a squad walks to a
courtyard, digs in and opens fire on whoever the quest points it at. It is the
**same file format and the same interpreter**, with `type = "scenario"`, and it
is started by a quest node, never by itself.

```lua
return {
  nq    = 1,
  type  = "scenario",
  id    = "bridge_ambush",
  title = "Засада у моста",
  roles = {
    leader    = { title = "Главный", default = { story = "esc_wolf" } },
    attackers = { title = "Налётчики", many = true },
  },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "move_in" } },
    { id = "move_in", kind = "npc.goto",
      params = { who = { "leader", "attackers" }, path = "bridge_walk", move = "run" },
      out = { done = "dig_in" } },
    { id = "dig_in", kind = "flow.step",
      on_enter = {
        { kind = "npc.hold",   params = { who = "attackers", path = "bridge_post", look = "bridge_look" } },
        { kind = "npc.attack", params = { who = "attackers", target = { actor = true } } },
      },
      out = { next = "finish" } },
    { id = "finish", kind = "flow.end" },
  },
}
```

### 10.1 Roles

A role is a slot the **calling quest** fills, which is what makes one scenario
serve any cast.

| field | meaning |
|---|---|
| `title` | author-facing, never shown to the player |
| `many` | `true` lets the role hold a group; `false` (default) refuses more than one |
| `optional` | `true` makes an empty role legal - steps addressed to it do nothing |
| `default` | the binding used when the caller names none: a reference table |

Bindings take the object reference forms of §3 — `{ story = … }`, `{ ref = … }`,
`{ squad = … }`, `{ profile = … }`, `{ community = … }` — or a list of them. A
ref resolves against the **calling quest's** refs, so a squad that quest made
with `spawn.squad` is bound by its name.

Members are resolved **once, on entry**, and kept: the cast cannot change under
the graph's feet. Members that die drop out; a step addressed to an empty role
is a no-op, and a scenario whose every role is empty ends by itself.

### 10.2 Steps

Every action of group `npc` takes `who` — one role name or a list of them. The
graph provides the ordering, branching and waiting; a step is instant unless the
catalog says it waits.

| kind | what it does |
|---|---|
| `npc.goto` | **`who: role`**, **`path: way_path`**, `look`, `move` (`walk`\|`run`), `arrive` (default true), `radius` → pin `done`. Walks a waypoint path (walker). `arrive` holds the node until everyone is within `radius` of the path's end. |
| `npc.hold` | **`who`**, **`path`**, `look`, `radius`, `shoot`, `no_retreat`. With a look path it digs in and watches it (camper); without one it stands guard on the path. |
| `npc.anim` | **`who`**, `state` (default `wait`), `look: target_ref`, `sound`. Plays an animation state where the NPC stands (remark). |
| `npc.follow` | **`who`**, **`target: role_or_actor`**, `distance`. Follows the player or another role (companion). |
| `npc.attack` | **`who`**, **`target: attack_target`**. Makes the role hostile to the target - the game's own combat does the rest. |
| `npc.stance` | **`who`**, `to_actor: relation`, `goodwill`, `combat_ignore`, `invulnerable`. Only the fields you set change. |
| `npc.release` | **`who`**. Hands the role back early; a scenario releases everyone it touched when it ends. |
| `npc_at_place` | cond: **`who`**, **`path`**, `radius`. Every living member is at the path's end. |
| `npc_in_combat` | cond: **`who`**. Any living member has an enemy. |
| `role_alive` | cond: **`who`**, `all`. A member is alive (`all = true`: every one). |

Everything else a scenario needs — `flow.*`, `wait.*`, `var.*`, `signal.*`,
`sound.play`, `relation.*`, `item.*`, `npc.kill`, `npc.remove`, `npc.set_name`,
`spawn.*`, `squad.*` and every condition — is the quest vocabulary, shared.

**A place is a waypoint path, not a coordinate.** Paths are authored with the
editor's WayPoint tool and ship in the module's spawn layer, which is exactly
what `path_walk`/`path_look` want; X-Ray walks patrol paths, and a path invented
at run time would be a second movement system.

### 10.3 Running one from a quest

| kind | what it does |
|---|---|
| `scenario.run` | **`asset: scenario_id`**, `roles: role_bindings`, `wait_end` (default true), `ref` → pin `done`. Starts the scenario. `wait_end` holds the node until it ends; `ref` saves the instance. |
| `scenario.stop` | **`target: ref_name`**. Ends it and releases its NPCs. |
| `scenario_running` / `scenario_done` | conds on that ref. |

```lua
{ id = "spring_the_trap", kind = "scenario.run",
  params = { asset = "bridge_ambush", wait_end = true, ref = "ambush",
             roles = { leader = { story = "esc_wolf" }, attackers = { ref = "ambush_squad" } } },
  out = { done = "after_the_fight" } }
```

### 10.4 How a step reaches an NPC

It does not move anything itself. A step writes a small logic section and hands
it to the stock scheme with `create_ini_file` + `xr_logic.configure_schemes` —
the same thing a smart terrain does. Combat, cover, danger, wounded and death
keep working because the same schemes keep running them, and a scenario can only
do what the schemes can do. That is the point: the alternative is a second AI
fighting the first.

An NPC that is **offline** has no game object to take a scheme, so the step is
parked and applied the moment it spawns in.

### 10.5 What the validator refuses

| code | meaning |
|---|---|
| `E023` | a kind used in the wrong document (`npc.goto` in a quest, `task.give` in a scenario) |
| `E024` | `who` names a role the scenario does not declare |
| `E006` | `tasks`/`activation` in a scenario, `roles` in a quest, a scenario with no roles |
