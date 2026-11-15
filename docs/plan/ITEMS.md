# Items: the working plan

The plan of one task, removed when the task is closed (what stays is the contracts it changes). Two goals:

1. **Any inventory item is added simply**: a weapon, a tool, a consumable, an outfit, a plain item - one text file
   an author or an agent writes, as flexible as a classic mod's config.
2. **Weapons Evolution 2026 becomes a module** on this system, with nothing of the game replaced that need not be,
   and every capability it needs added to the game and the editor as a general one.

What the surveys found (the scratch reports of this task; their numbers are quoted here because the reports go):

- Weapons Evolution is no pack of new items. It reworks the HUD, the animations, the sounds and the particles of the
  game's own weapons and consumables: 862 game sections carry values of its own (about 15 000 keys: animations
  8172, HUD geometry 3723, sounds 1851, particles 332), 74 sections are new (50 HUD variants, 11 use-animation
  pseudo weapons, a silencer, a tomahawk, three gauss variants, mixins).
- It was made over the OLD Dead Air (`configs.xdb0` + `xtra.xdb0`), by replacing whole files. Against the game of
  today its files also roll back 3071 keys (prices, damage, wear) and lose 3089 sections the game added since.
  The author's work is **the mod minus its base**, never the mod minus the game.
- It replaces the shared icon atlas (548 cells are its own; the whole file shows 1707 of the game's items with
  older art), `particles.xr` (a superset: +1028 effects, +100 groups, 60 changed), 5 game scripts (two of them
  for one line each: a craft recipe, three start kits), 40 configs scripts open by name (its own edits are 1 to 15
  lines each), 362 models, 210 textures, 76 sounds.
- The game today: everything the ENGINE reads of an item is additive (sections, strings, `icons_texture`, XML by
  `.xmlp`, use scenes by `ea_addon_*.ltx`, loose particles). Everything the game's SCRIPTS open with
  `ini_file("...")` - traders, death loot, loadouts, stacks, upgrade costs, tasks, smart logic - is out of a
  module's reach but by replacing the file; craft recipes, disassembly and cooking are Lua tables. A `.ltxp`
  patch of a section does not reach the sections that inherit from it (parents are copied when a file is parsed).

## 1. The contract

### 1.1 The item asset (`*.itemasset`, header `item = 1`)

One file, anywhere in the module, is one item: a NEW one, or a CHANGE of one that exists. The module ships the
file; the game reads it when its configs come up and makes the sections, the strings and the list entries itself
(`xms_items.cpp`, beside the characters). Plain data, like every asset; a key the game does not read is passed by.

```lua
-- a new item
return {
  item = 1,
  id = "sil_diy",                 -- a-z 0-9 _ ; the file name when left out
  section = "wpn_sil_diy",        -- its section in the game; "<module>_<id>" when left out; under 64 characters
  base = "wpn_sil_pbs1",          -- the section it starts from, or a list of them
  name = { rus = "...", eng = "..." },     -- or one string (Russian)
  short_name = "...",             -- the name when left out
  description = { rus = "..." },
  icon = { texture = [[ui\we_icons]], x = 3, y = 0, w = 2, h = 1 },   -- cells of 50 px; see "Icons"
  set = { cost = 1200, inv_weight = 0.4, visual = [[dynamics\weapons\wpn_sil\wpn_sil_diy]] },
  hud = { base = "wpn_addon_silencer_hud", set = { item_visual = [[...]] } },
  sections = {
    { name = "wpn_sil_diy_sounds", base = "base_sounds", set = { snd_shoot = [[weapons\sil\diy_shot]] } },
  },
  craft = { { at = "itm_repairkit_tier_1", parts = { "jar", "textile_patch_m", "rope", "flashlight_broken" } } },
  loot  = { count = { 0, 1 }, chance = 0.01, by = { bandit = 0.03 } },
  sold  = { { by = "trade_military", count = 1, chance = 0.5 } },
}

-- a change of an item that exists (the game's, or another module's)
return {
  item = 1,
  edit = "wpn_ak74",
  set   = { snd_shoot = [[weapons\ak74\ak74_shoot]], flame_particles = [[weapons\new_effects\ak74]] },
  unset = { "snd_shoot1" },
  add   = { ammo_class = { "ammo_5.45x39_ap" } },
  drop  = { ammo_class = { "ammo_5.45x39_fmj" } },
  hud   = { set = { hands_position = { 0.02, -0.01, 0.1 }, anm_reload = "ak74_reload, ak74_reload_w" },
            unset = { "anm_bore" } },
  icon  = "icons/wpn_ak74.png",
}
```

| Key | Takes | Means |
|---|---|---|
| `item` | integer | The format version (1). A newer one is left out with the line every asset reader says. |
| `id` | string | A new item's id: `a-z 0-9 _`, not starting with a digit, up to 48; the file name when left out. |
| `section` | string | A new item's section in the game. `<module>_<id>` when left out. `a-z 0-9 _ . -`, under 64 characters (a save reads it into 64). The author who names it answers for it not meeting another module's. |
| `base` | string or list | A new item's parent section(s), as `[section]:a, b` (a later one wins over an earlier one). A section of the game, of a module loaded before, or of another item of this module. |
| `abstract` | boolean | A new item that is a template: its section and nothing else - no strings, no icon keys, no list entry. |
| `edit` | string | Instead of `id` / `section` / `base`: the section this file changes. It must exist when the module's items are applied. Two files of one module that edit one section are applied in the order of their paths. |
| `name`, `short_name`, `description` | string or `{ lang = text }` | What the player reads. The game makes the string ids (`<section>_name`, `<section>_short`, `<section>_descr`) and sets `inv_name`, `inv_name_short`, `description`. A language the asset lacks falls back to `rus`. Left out: the base's (a new item), unchanged (an edit). To name a string id that exists, write the key itself: `set = { inv_name = "st_wpn_ak74" }`. |
| `icon` | table or string | `{ texture, x, y, w, h }`: a cell rectangle of a texture, in cells of 50 px (`icons_texture`, `inv_grid_*`); `texture` left out is the game's atlas. The authoring forms `"<picture>"` and `{ file = "<picture>", w, h }` name a picture beside the asset: the module build bakes every such picture into the module's atlas and ships the asset with the first form (1.4). The game reads the first form only. |
| `set` | table | Keys of the section as the game reads them: `key = value`. A string, a number, a boolean (`true` / `false`), or a list (`{ a, b }` is `a, b`; numbers likewise: a vector). |
| `unset` | list | Keys taken away. |
| `add`, `drop` | `{ key = member or list }` | Members added to / taken out of a comma list (`ammo_class`, `scopes`, `upgrades`). |
| `hud` | table | The item's HUD section. New item: `{ base, set, ... }` makes `<section>_hud` and sets `hud` to it (unless `set.hud` says otherwise). Edit: `{ set, unset, add, drop }` change the section the item's `hud` key names once `set` is applied. |
| `sections` | list | Other sections the item owns or changes: `{ name, base, set }` makes one (the name as written); `{ edit, set, unset, add, drop }` changes one. Sound mixins, scope variants, upgrade sections, a use scene's HUD. |
| `craft` | list of `{ at, count = 1, parts }` | Recipes that make the item: `at` is the tool or the workbench the recipe is in (the section the craft window is opened with), `parts` up to four sections. |
| `loot` | `{ count, chance, by }` | Death loot: `count` (a number or `{ min, max }`) is its line of `[item_count]`; `chance` its line of `[base]`, which every community and rank inherits; `by = { <community or community_rank> = chance }` lines of their own. |
| `sold` | list of `{ by, supplies = "supplies_1", count = 1, chance = 1 }` | Stock of the game's traders: `by` is the trader's config (`trade_military`, or a path ending in `.ltx`), `supplies` the section of it or a list of them. A module's own character keeps its stock in its `.npcasset`. |
| `scene` | string or table | The use animation: `"vodka"` uses the scene of that item; `{ like = "vodka", sound, camera, time, helmet, anm, hud_anm }` the keys of `animations_settings.ltx`, over `like` when given; with `hud = { base, set }` the scene's own HUD section `item_ea_<section>_hud`. |
| `lists` | list of rows | Any other line the item needs somewhere: `{ file, section, key, value }` sets `key = value` (`key` left out: the item's section; `value` left out: a bare key), `{ file, section, key, add = member }` / `drop` change a list. `file` is a config by its path under `configs` (`plugins\itms_manager.ltx`); left out, the system config. |
| `editor` | table | The editor's notes. The game reads none of it. |

- **New or change, never both**: `edit` with `id`, `section`, `base` or `abstract` is refused. A new item whose
  section exists already is refused ("write it as a change: `edit`").
- **What the game makes** of a new item: `[<section>]:<base>` with `set`; the strings; `icons_texture` and
  `inv_grid_*`; `<section>_hud`; the `sections`; then the lists. Nothing is defaulted that the base gives: the
  author starts from a base that works (a section of the same kind) and writes the difference.
- **Order**: a module's characters, then its items (new sections first, then changes in path order), then its
  `configs\xms\*.ltx`, then its `.ltxp` - what the module's own overlays and patches say of an item still counts.
  A section an item makes that the module's overlay defines again is merged (the later wins, one line in the
  log), never a stop of the game.
- **A refused item stops nothing**: one line `! XMS: [<module>] <file>: left out - <why>` (no base, an unknown
  base, a name that cannot be a section, an edit of a section that is not there, a key that is no name).

### 1.2 Patches reach what inherits (the config engine)

- A section remembers its parents and which keys are its own lines. A change of a key by a module - an overlay
  merge, a `.ltxp` directive, an item - reaches every section that inherits that key from the changed one: not
  set by a line of its own, and not given by a later parent. `+ammo_class` on `wpn_ak74` reaches
  `wpn_ak74_kobra`; a line added to `[base]` of the death loot reaches every community.
- `[section] : parents` on a section that exists stays what it is today: the parents are not changed.
- The editor's config reader (`gz-ltx`) composes the same way: change both or neither.

### 1.3 Patches of any config (`#file`)

- A `.ltxp` line `#file <path under configs>` sends the directives after it to that file instead of the system
  config: `#file misc\death_generic.ltx`. `#file` alone goes back to the system config. The operators are the
  same (`key = v`, `+key`, `-key`, `!key`, `[!section]`, `[new] : parents`).
- The patch is applied to every ini that READS that file by name - opened by itself (a script's
  `ini_file("misc\\death_generic.ltx")`, an engine reader) or pulled in by an `#include` of another - when it has
  been read whole, with 1.2's inheritance. So a line added to `trade_presets.ltx [merchant]` is in every trader's
  file. Modules apply in their load order; the ledger and `xms_why` name the file.
- What an item's `loot`, `sold`, `scene` and `lists` say becomes such patches, made by the game.
- A game that does this states `config-file-patches` and `item-assets` in its `EngineAbilities`.
- The module stage runs when the game's own overlays are all in, so a patch reaches their sections too.

### 1.4 Icons

- The game knows one form: a cell rectangle of a texture (`icons_texture` + `inv_grid_*`, cells of 50 px).
- An author gives a picture per item (`icon = "icons/ak74.png"`, any size; `w`, `h` in cells, worked out from the
  picture's size when left out: a picture of 100 x 50 px is 2 x 1). The module build packs every such picture into
  `gamedata\textures\ui\<module>_icons.dds` (deterministic: by the items' sections; each scaled to its cells) and
  ships each asset with `icon = { texture = [[ui\<module>_icons]], x, y, w, h }`. Sources where they lie never ship.
- An icon of a CHANGE gives the game's item a new picture without touching the game's atlas.

### 1.5 Lists the game takes from data

- **Craft**: the craft window adds, to the recipes of the tool or workbench it is opened with, the recipes modules
  bring: the system config's `[xms_craft]`, one key a recipe, `<n> = at, result, count, part, part, part, part`
  (`-` or nothing: no part; the game writes it from the items' `craft`; a module may write the section itself). A
  recipe whose result or part is no section is left out with a line. As done: the game's own `xms_items.script`
  WRAPS the stock craft window - no changed copy of a stock script ships, because plain Dead Air and Revolution II
  have different versions of the same scripts and one copy would replace one game's with the other's.
- **Start kits** of the new-game screen: `[xms_start_kits]`, one key a kit (the value is not read), each kit a
  section: `title = <string id>`, `cost = 0..100`, `items = row, row` (a row is `section[:N][@min-max | @c]`, and
  `a|b|c` is one of them at random; ammo counts are boxes), optional `descr = <string id>` and
  `after = <script>.<function>` called with the kit's section. Offered after the stock kits, on the same 100
  points; given at a new game. A module mode's `[start_kit]` still replaces everything.
- **Upgrade costs**: an upgrade or a property of a module that `item_upgrades.ltx` lacks is read from the system
  config by the same stock logic (wrapped); a missing `cost` is free, with a line.
- **The item catalog** `configs\item_catalog.ltx` (the game's; `docs\dead-air\ITEM_DATA.md` there): the scopes
  (`item`, `hud`, `upgrade`, and the sections an item names: `immunities`, `absorbation`, `bones`, `sound_layers`,
  `model`, `global`), the families of items by engine class with the sections a new one starts from, the groups,
  and for every key the game reads (898) `[<scope>.<key>]`: type, families, `required` (`true` or the classes
  that stop without it), `presence`, default, and for the 392 an author works with a title, a group, a unit, a
  sentence of help, `common` (84). A series is one entry with a `pattern`. Generated in the game's repository
  (`tools\items\make_item_catalog.py`), byte for byte. The editor reads the linked game's (a bundled copy is the
  fallback, kept byte-identical); its form and its reference are made from it. No key is known to the editor's
  code by name.

### 1.6 The editor

- `gz-items`: the kind (`AssetKind::Item`, `.itemasset`, `item`), the model (unset stays unset), what an item
  RESOLVES to against the linked game's config tree and the module's own sections and items, the check
  (positioned, coded `I4xx`: identity, a base or an edit target that is not there, a key the game stops without
  that the base chain does not give, a value that is no value of the key's type, a model / sound / texture /
  section / string a key names that neither the game nor the project has, a name over 63 characters, a picture
  that is not there, texts cp1251 lacks), the reference (`asset.reference kind=item`: the format above and the
  linked game's catalog), the build's publish (the icon atlas, the shipped text), `items.list` / `items.info`
  (read: what the module's items are and what one resolves to, for an agent).
- **Mod > Items** (a tab): the module's items - new ones and changes - with their icons; New Item... starts from an
  item of the game picked in the chooser (by the names the player reads); Change a Game Item... picks the one to
  change. The form: Name and Description (a row per language), Icon (a picture dropped or picked), the keys by
  the catalog's groups for the item's family - the common ones on the surface, the rest under More -, each showing
  the base's value muted until the author sets it (a Reset arrow unsets), the HUD section the same way, Where It
  Is Found (loot, sold by, craft), and the raw rows for a key the catalog lacks. Every edit is the difference
  written into the file.
- The content browser: an Item icon; Add > Item (`file.create kind=item`).
- The catalog's allow-list takes `itemasset` (four sides).

### 1.7 A classic mod, three ways

`tools/classic_mod/classic_to_module.py` takes the mod's BASE as well (`--base` the archives the mod was made
from; worked out per file as the closest archived version when left out): what the mod changes is the mod minus
its base, applied to the game of today. A value the mod only carries from its base is not the mod's. Item
sections become `.itemasset` files (a change per game item with its HUD and the mixins it owns; a new item per
new section), the rest `patch\*.ltxp` (with `#file` for configs outside the tree), XML `patch\*.xmlp`, strings
the ids the mod adds or changes against its base, media the files that are the mod's own (not the game's bytes,
not an older version of the game's file), icons the pictures that are the mod's own, cut out of its atlas,
particles the effects and groups it adds or changes, as loose `.pe` / `.pg`.

### 1.8 A save the original game still loads (asked for while the task ran)

The author's wish: as far as it can be done, a module must not cost a save its compatibility with the original
Dead Air - what a module brought simply is not there when the save is loaded in the original game. Characters
and everything else a module adds included.

- **What a module brings never enters the original game's save format.** `.scop` always describes a world the
  original game can load; everything of a module lives in `.scov` (the game's `docs\dead-air\SAVE_COMPATIBILITY.md`:
  the one extension container, chunks, ignored by the original game) and is laid over the `.scop` world when
  Refined loads the save. So a save can go to the original game and come back, and the module's content is no
  problem there and not lost here.
- **Objects**: an ALife object is a module's - and is written to `.scov`, by module - when its section was made
  by a module (an item, a character, a squad), when a module's spawn layer added it, when it stands on a level a
  module added, when a module's script declared it its own (what a quest spawns into the world; not what it hands
  to the player), or when it is inside such an object (a stock medkit in a module character's pockets).
- **Its id stays taken**: `.scop` holds, under the same id, an inert placeholder of a class the original game has
  always had (no visual, no shape, never online). The original keeps it and never gives the id to anything else;
  Refined replaces it by the object it stands for. No id is ever remapped, so every reference - a quest's, a
  smart terrain's - means the same object after any trip. A placeholder is no content.
- **The lineage**: the placeholders carry the id of the Refined save whose `.scov` the world belongs with. The
  original game carries it forward under any save name; Refined, loading a `.scop` whose own sidecar is missing or
  stale, finds the `.scov` of that lineage among the saves and takes the module families from it.
- **State of the game's own objects that names a module's data** follows the weapon-extension precedent: the
  `.scop` record keeps what the original game understands, `.scov` the rest - an upgrade of a module installed on
  a stock item, an ammo type a module added to a stock weapon's list that is loaded in it.
- **What scripts save**: nothing a stock script writes into `.scop` or `.scoc` names a module's object or task -
  such entries are taken out of the stock stream and kept with the module's data.
- **A module that is absent - switched off or removed, the same case - is kept, not erased.** The player plays on
  with a working save and without that module's content, and gets it back where it stopped when the module
  returns (an update in between, a story mod set aside). Its records stay in `.scov` byte for byte through every
  save made without it, its placeholders keep its ids, the quest runtime keeps its quests untouched and not run.
  What cannot come back safely is dropped with a line saying why, when the module is there again: a section the
  module no longer has, a section whose class changed (the record is in the old class's format), an item whose
  owner is gone. `xms_forget <module>` drops a module's kept content for a player who will not bring it back.
- Limits are said, not hidden: the player saved on a module's level; a module that changes the game's own objects
  in place; a mode of a module (a game of its own, which the original never starts).
- A game with no module, and a save with nothing of a module in it, are written byte for byte as before.

## 2. Weapons Evolution 2026 as a module

The project: `D:\XFinedProjects\WeaponsEvolution2026` (new; the old conversion `WeaponsEvolution` stays as it
is), module id `weapons_evolution`. What is done by hand on top of the converter:

- `ayykyu_fp_death`, `item_animations`: module scripts (`scripts\`, a registrar), on the game's callbacks.
- `enhanced_animations` is NOT replaced: the mod's item table becomes scenes of the game's own system (`scene`
  of each item; the pseudo weapons `anm_*` become `item_ea_*_hud` sections - no spawnable section for a scene).
- `ui_itm_craft` (+1 recipe): the silencer's `craft`. `ui_mm_faction_select` (+3 kits): the start kits registry.
- `dar2_aimscript` (an option gate removed): a hook. `dinamic_hud`: a rollback, not carried.
- `game_tutorials.xml`, `ui_detector_artefact.xml`, `character_desc_bar.xml`: `.xmlp`.
- Trade, tasks, smart logic, `debug_spawner`, death loot, the Refined item scene files, the 3D PDA offsets: `#file`
  patches of the author's own lines.

Proof: the module builds; the game starts with it alone on a fresh root; the log holds no `!` line of the module;
a sample of its weapons, consumables and the silencer spawn, draw their HUD, shoot and reload with the mod's
motions and sounds; a use scene plays; the silencer is crafted; the icons are the mod's.

## 3. Work packages

| Id | Owner | What | Where |
|---|---|---|---|
| G1 | agent | 1.2, 1.3, 1.1's game half (`xms_items.cpp`), the abilities, tests, the game's docs | game: `src\xrCore` (`xr_ini.*`, `XMS\`), `src\xrServerEntities\script_ini_file.cpp`, `src\xrEngine\x_ray.cpp`, `tools\qa`, `docs\dead-air` |
| G2 | agent | 1.5: the item catalog, craft from data, start kits, upgrade costs of a module's upgrades | game: `res\gamedata`, the stock scripts Refined overrides, `tools\` |
| E1 | agent | `gz-ltx` (1.2, 1.3), `gz-items` (domain, kind, check, reference, publish), the allow-list | editor: `crates\gz-ltx`, `crates\gz-items` (new), `crates\gz-project`, `crates\gz-build`, `crates\gz-release`, `crates\grizzly` |
| E2 | agent | Mod > Items, the browser's icon, QA `test_items.py` | editor: `crates\gz-ui-world` (or a crate of its own), `gz-ui`, `gz-ui-content`, `tools\qa` |
| P1 | agent | 1.7 and section 2 | editor: `tools\classic_mod`; the new project |
| G3 | agent, after G1 | 1.8: objects and item state in `.scov`, a switched-off module's records kept, ids reserved | game: `src\xrGame` (the ALife registry's save and load, the save extensions, `xms_game.cpp`), the stock scripts' save hooks, `docs\dead-air\SAVE_COMPATIBILITY.md`, `tools\qa` |
| G4 | agent, with G3 | 1.8: what scripts save about a module's objects, the quest runtime keeping an absent module's quests, per-module data kept | game: `packaging\dead-air-x64\compatibility\gamedata\scripts` (`xms_*.script`), `tools\nq` |
| - | lead | this plan, the contracts' final text, integration, the game build and deploy, the in-game proof, the one full read by the critic and its fixes |

## 4. State

- 2026-11-15: the surveys are in; the contract above is written; G1, G2, E1 and P1 run. E2 (the Items tab) starts
  on E1's API; P2 (section 2's hand work: scripts, use scenes, XML, start kits) on P1's project and G1/G2's game.
- 2026-11-15, later: the author asks for saves the original game still loads with modules on (1.8, G3).
- G2 is in (the catalog, craft, start kits and upgrade costs from data; 134 headless tests over the stock scripts of
  both game layers). To see in the running game: a data recipe crafts; a data kit is offered and given; a module
  upgrade shows its cost and installs; with no module the log has no `xms_items` line. G4 (saves, scripts) runs.
- The scratch of the task (surveys, the key table, the unpacked game views) is the session's scratchpad folder
  `we\`; nothing in the repository depends on it.
