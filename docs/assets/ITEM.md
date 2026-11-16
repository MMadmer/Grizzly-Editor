# Items (`.itemasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-items/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-items` fails when this file is not what the tables say.

One file anywhere in the module is one item of the inventory - a weapon, a tool, a consumable, an outfit, a plain thing: a NEW one, or a CHANGE of one that exists (the game's, or another module's). The module ships the file as it is; the game reads it when its configs come up and makes the sections, the strings and the list entries itself. It is plain data like every asset (`docs/ASSETS.md`): `return { ... }` of literals, a key the game does not read is passed by, a key the editor does not know is kept through every save and noted once.

```lua
-- a new item
return {
item = 1,
id = "sil_diy",                 -- the file name when left out
section = "wpn_sil_diy",        -- "<module>_<id>" when left out
base = "wpn_sil_pbs1",          -- the section it starts from, or a list of them
name = { rus = "...", eng = "DIY silencer" },
description = { eng = "..." },
icon = "icons/sil_diy.png",     -- a picture beside the asset: the build bakes it into the module's atlas
set = { cost = 1200, inv_weight = 0.4, visual = [[dynamics\weapons\wpn_sil\wpn_sil_diy]] },
sections = {
{ name = "wpn_sil_diy_sounds", base = "base_sounds", set = { snd_shoot = [[weapons\sil\diy_shot]] } },
},
craft = { { at = "itm_repairkit_tier_1", parts = { "jar", "textile_patch_m", "rope", "flashlight_broken" } } },
loot  = { count = { 0, 1 }, chance = 0.01, by = { bandit = 0.03 } },
sold  = { { by = "trade_military", count = 1, chance = 0.5 } },
}
```

```lua
-- a change of an item that exists
return {
item = 1,
edit = "wpn_ak74",
set   = { snd_shoot = [[weapons\ak74\ak74_shoot]], flame_particles = [[weapons\new_effects\ak74]] },
unset = { "snd_shoot1" },
add   = { ammo_class = { "ammo_5.45x39_ap" } },
drop  = { ammo_class = { "ammo_5.45x39_fmj" } },
hud   = { set = { hands_position = { 0.02, -0.01, 0.1 } }, unset = { "anm_bore" } },
icon  = "icons/wpn_ak74.png",   -- the game's item gets a new picture; the game's atlas is not touched
}
```

## New or change

- **A new item** names what it starts from (`base`) and says what is different. Start from a section of the same kind - a pistol from a pistol, a medkit from a medkit - and write the difference: nothing is defaulted that the base gives. `id`, `section` and `abstract` make a file a new item too.
- **A change** names the section it changes (`edit`) and says only what changes. It needs no `base`.
- **Never both**: `edit` with `id`, `section`, `base` or `abstract` is refused. A new item whose section exists already is refused as well - write it as a change. A template (`abstract = true`) needs no base.

## What the game makes

Of a new item: `[<section>]:<base>` with `set` (a list joined by `, `), `unset`, `add` and `drop` (the list rewritten joined by `,`); the strings - `<section>_name`, `<section>_short` and `<section>_descr` with `inv_name`, `inv_name_short` (the name's id when `short_name` is left out) and `description`, over what `set` says; `icons_texture` and `inv_grid_*` of the icon; `<section>_hud` from `hud`, over the HUD section the bases name when `hud` names no base; the `sections`; the recipes (`[xms_craft] <n> = at, section, count, part, ...`, `n` a zero-padded number after the highest there); then the lines of other configs: the death loot, the traders' stock, the use scene (`items\items\animations_settings.ltx`: `sound` is `snd`, `camera` `cam`, `time` `tm`, `helmet` `helm`), `lists`. A template (`abstract = true`) is its section and nothing else. Of a change: the same keys on the section it edits, its HUD section changed through the section's own `hud` key. **Any malformed part refuses the whole item**: a key no config can hold, a value with a line break, a recipe of five parts.

## Order

A module's characters, then its items - the new sections first (an item made from another item of the module after that one, else in the order of their paths), then the changes in the order of their paths -, then its `configs\xms\*.ltx`, then its `patch\*.ltxp`: what the module's own overlays and patches say of an item still counts. A section an item makes that the module's overlay defines again is merged, the later one winning - its own sections, and the ones made for the rows it adds (`[xms_craft]`, a scene's HUD section, a section of `lists`). Every change reaches the sections that inherit the changed key and do not set it themselves: `add = { ammo_class = ... }` on `wpn_ak74` reaches `wpn_ak74_kobra`. The order of paths is the game's: the paths compared byte by byte as the game reads them, ASCII letters lower-cased and `/` between folders - `items/weapons/ak` before `items/weapons2/ak`, `B` after `a`; of two changes of one key the later path wins.

A refused item stops nothing: the game leaves it out with one line in its log (`! XMS: [<module>] <file>: left out - <why>`) - no base, an unknown base, a name that cannot be a section, an edit of a section that is not there.

## Icons

The game knows one form: a rectangle of a texture in cells of 50 px (`icons_texture`, `inv_grid_x`, `inv_grid_y`, `inv_grid_width`, `inv_grid_height`) - `icon = { texture, x, y, w, h }`. An author may give a picture instead (`icon = "icons/ak74.png"`, any size; or `{ file = "icons/ak74.png", w = 2, h = 1 }`): the module build packs every such picture into `gamedata\textures\ui\<module>_icons.dds` - by the items' sections, each scaled to its cells; `w` and `h` left out are its size in cells, rounded up (100 x 50 px is 2 x 1) - and ships each asset with `icon = { texture = [[ui\<module>_icons]], x, y, w, h }`, nothing else of its text changed. The pictures are sources and never ship. An icon on a change gives the game's item a new picture without touching the game's atlas. An icon is its texture and its rectangle together: a section that states its own rectangle - a variant with its own cells - keeps its texture when its base gets a picture.

## The catalog

What a key of an item's section means is the game's: its `configs\item_catalog.ltx` (the editor's bundled copy when no game is linked) names the families of items by their engine class and, for every key of an item's or its HUD's section, its type, group, title, unit, whether the game stops without it and a sentence of help. The check reads it - a key the game stops without, a value of the wrong type, a model, sound, texture, particle effect, section, string id or motion nobody has - and `asset.reference kind=item` lists it after this page. `items.info item=<file or section>` answers what an item resolves to: every key of its section with its value and where the value comes from (the item, a base by name, the game, the module), its HUD section, for a change the game's values before it.

## The keys

| Key | Takes | Left out | Means |
|---|---|---|---|
| `item` | integer | - | The format version (1). A file of a newer version is refused by name. |
| `id` | string (`a-z 0-9 _`) | the file's name | A new item's id: `a-z 0-9 _`, not starting with a digit, up to 48 characters. |
| `section` | string (a section) | `<module id>_<id>` | A new item's section in the game: `a-z 0-9 _ . -`, under 64 characters (a save reads it into 64). The author who names it answers for it not meeting another module's. |
| `base` | string or list (sections) | - | A new item's parent section, or a list of them, as `[section]:a, b` (a later one wins over an earlier one): a section of the game, of a module loaded before, or of another item of this module. |
| `abstract` | boolean | false | A new item that is a template: its section and nothing else - no strings, no icon keys, no list entry. |
| `edit` | string (a section) | - | Instead of `id`, `section` and `base`: the section this file changes. It must exist when the module's items are applied; two files of one module that edit one section are applied in the order of their paths. |
| `name` | string or `{ lang = text }` | the base's (a new item); unchanged (a change) | What the player reads. The game makes the string id `<section>_name` and sets `inv_name` (over what `set` says), in `configs\text\<lang>\<module>_items.xml` for `rus`, `eng` and every language an item names; a language the asset lacks falls back to `rus`. To name a string id that exists, leave `name` out and write the key itself: `set = { inv_name = "st_wpn_ak74" }`. |
| `short_name` | string or `{ lang = text }` | the name's string id | The short name of the inventory cell: `<section>_short`, `inv_name_short`. |
| `description` | string or `{ lang = text }` | the base's; unchanged | The description: `<section>_descr`, `description`. |
| `icon` | table or string | the base's; unchanged | `{ texture, x, y, w, h }`: a rectangle of a texture in cells of 50 px (`icons_texture`, `inv_grid_*`), `texture` left out is the game's atlas (`icons_texture` goes), `w` and `h` left out keep the section's size; or a picture beside the asset, `"<picture>"` or `{ file = "<picture>", w, h }`, which the module build bakes into the module's atlas (below). |
| `set` | `{ key = value }` | - | Keys of the section as the game reads them: `key = value`. A string, a number, a boolean (`true` / `false`), or a list (`{ a, b }` is `a, b`; numbers likewise: a vector). |
| `unset` | list of keys | - | Keys taken away. |
| `add` | `{ key = member or list }` | - | Members added to a comma list (`ammo_class`, `scopes`, `upgrades`): `{ ammo_class = "ammo_5.45x39_ap" }` or a list of them. |
| `drop` | `{ key = member or list }` | - | Members taken out of a comma list, the same way. |
| `hud` | the HUD section (below) | - | The item's HUD section. A new item: `{ base, set, ... }` makes `<section>_hud` - refused when that section exists - and sets `hud` to it (unless `set.hud` names another). A change (no `base`): `{ set, unset, add, drop }` change the section the item's `hud` key names once `set` is applied. |
| `sections` | a list of sections (below) | - | Other sections the item owns or changes: `{ name, base, set }` makes one (the name as written), `{ edit, set, unset, add, drop }` changes one - sound mixins, scope variants, upgrade sections, a use scene's HUD. |
| `craft` | a list of recipes (below) | - | Recipes that make the item. |
| `loot` | death loot (below) | - | What the dead carry: lines of the game's death loot - the count in `misc\death_generic.ltx`, the chances in `misc\death_items_by_communities.ltx`. |
| `sold` | a list of stock rows (below), or one row alone | - | Stock of the game's traders. A module's own character keeps its stock in its `.npcasset`. |
| `scene` | string or table | - | The use animation: `"vodka"` uses the scene of that item; a table sets the keys of `items\items\animations_settings.ltx` (below), `[<section>]` there over `like`. |
| `lists` | a list of lines (below) | - | Any other line the item needs somewhere: a key of any config, or a member of a list in one. |
| `editor` | anything | - | The editor's notes. The game reads none of it. |

## `icon`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `texture` | string | the game's atlas | The texture of the rectangle, by its game name (`ui\ui_icon_equipment`). |
| `x` | integer, 0 to 4095 | - | The rectangle's left edge, in cells of 50 px (`inv_grid_x`); a rectangle needs it. |
| `y` | integer, 0 to 4095 | - | Its top edge, in cells (`inv_grid_y`); a rectangle needs it. |
| `w` | integer, 1 to 64 | the section's; a picture's: its width in cells | Its width in cells (`inv_grid_width`). |
| `h` | integer, 1 to 64 | the section's; a picture's: its height in cells | Its height in cells (`inv_grid_height`). |
| `file` | string (a picture beside the asset) | - | Instead of a rectangle: a picture beside the asset (`.png`, `.tga`, `.jpg`, `.bmp`, `.dds`), scaled to `w` x `h` cells; left out, they are its size in cells, rounded up. |

## `hud`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `base` | string or list (sections) | none | A new item's HUD section starts from this one (`wpn_ak74_hud`). Without one it starts from the HUD section its bases name, or from nothing when they name none. A change takes none: it changes the section its `hud` names. |
| `set` | `{ key = value }` | - | Keys of the HUD section. |
| `unset` | list of keys | - | Keys taken away (a change). |
| `add` | `{ key = member or list }` | - | Members added to a list (a change). |
| `drop` | `{ key = member or list }` | - | Members taken out of a list (a change). |

## A row of `sections`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `name` | string (a section) | - | A section the item makes, named as written. |
| `base` | string or list (sections) | none | What the made section starts from: a section that is there, or one the item itself makes before it - its own section, its HUD section, an earlier row. |
| `set` | `{ key = value }` | - | Its keys. |
| `edit` | string (a section) | - | Instead of `name`: a section that exists, changed (it keeps its parents: no `base`). |
| `unset` | list of keys | - | Keys taken away (a change). |
| `add` | `{ key = member or list }` | - | Members added to a list (a change). |
| `drop` | `{ key = member or list }` | - | Members taken out of a list (a change). |

## A row of `craft`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `at` | string (a section) | - | The tool or the workbench the recipe is in: the section the craft window is opened with (`itm_repairkit_tier_1`). |
| `count` | integer, 1 to 1000 | 1 | How many the recipe makes. |
| `parts` | one to four sections | - | What it takes. The game writes the recipe as `[xms_craft] <n> = at, section, count, part, ...`. |

## `loot`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `count` | number or `{ min, max }` | - | How many the dead carry when they carry it: its line of `[item_count]`, a number or `{ min, max }`. Without one the death manager never spawns the item. |
| `chance` | number, 0 to 1 | - | Its line of `[base]`, which every community and rank inherits. |
| `by` | `{ community = chance }` | - | Lines of their own: `{ bandit = 0.03, stalker_veteran = 0.01 }` - a community, or a community and a rank. |

## A row of `sold`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `by` | string | - | The trader's config: `trade_military` (`misc\trade\trade_military.ltx`), or a path under `configs` ending in `.ltx`. |
| `supplies` | string or list (sections) | "supplies_1" | The section of it, or a list of them. |
| `count` | number, 0 to 1000000 | 1 | How many are on the shelf. |
| `chance` | number, 0 to 1 | 1 | The chance they are. |

## `scene` as a table

| Key | Takes | Left out | Means |
|---|---|---|---|
| `like` | string (a section) | - | The item whose scene this one starts from (`vodka`). |
| `sound` | string | - | The scene's sound (`snd`). |
| `camera` | string | - | The camera effect (`cam`). |
| `time` | number, 0 to 600000 | - | How long the scene lasts, milliseconds (`tm`). |
| `helmet` | string, number, boolean or list | - | What a helmet does to it (`helm`), as `animations_settings.ltx` says it. |
| `anm` | string | - | The scene's motion. |
| `hud_anm` | string | - | The motion of the scene's HUD. |
| `hud` | `{ base, set }` | - | The scene's own HUD section, `item_ea_<section>_hud` of the system config; `anm` left out is then `item_ea_<section>`. |

## A row of `lists`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `file` | string | the system config | A config by its path under `configs` (`plugins\itms_manager.ltx`). |
| `section` | string (a section) | - | The section of it the line goes into. |
| `key` | string | the item's section | The key. |
| `value` | string, number, boolean or list | a bare key | `key = value`. |
| `add` | string | - | Instead of `value`: a member added to the list `key` holds. |
| `drop` | string | - | Instead of `value`: a member taken out of it. |

## Problems

`asset.check` names every problem at its key; the build refuses a module whose items have an error, with the first one.

| Code | Severity | What |
|---|---|---|
| `I400` | error | The file does not read. |
| `I401` | error | Neither a new item nor a change: nothing says what the file is. |
| `I402` | error | Both: `edit` with `id`, `section`, `base` or `abstract`. |
| `I403` | error | An id that cannot be one (`a-z 0-9 _`, not starting with a digit, up to 48). |
| `I404` | error | A section name that cannot be one (`a-z 0-9 _ . -`, under 64 characters). |
| `I405` | error | A new item without a base. |
| `I406` | error | Two items of the module make one section (on both). |
| `I407` | error | A new item, or a section it makes, whose section the game or the module already has. |
| `I408` | error | A base nothing has: not the game's, not a module's loaded before, not another item's. |
| `I409` | error | An edit of a section that is not there (the item's, its HUD's, a row of `sections`). |
| `I410` | error | The bases of the module's items lead back to themselves. |
| `I411` | error | A key the catalog says the game stops without, which the section does not get (a base of the game that lacks it too shows the game does without it). |
| `I412` | warning | A value that is no value of the key's type. |
| `I413` | warning | A model nobody has. |
| `I414` | warning | A sound nobody has. |
| `I415` | warning | A texture nobody has. |
| `I416` | warning | A particle effect nobody has. |
| `I417` | warning | A section nobody has. |
| `I418` | warning | A string id nobody has. |
| `I419` | warning | A motion the HUD model does not have. |
| `I420` | warning | A text with characters the game's code page (cp1251) lacks. |
| `I421` | error | An icon picture that is not there, lies outside the project or is no picture. |
| `I422` | warning | An icon rectangle outside its texture. |
| `I423` | warning | A recipe's tool or part that is not there. |
| `I424` | warning | A config a row names that is not there, or lacks the section. |
| `I425` | warning | A death loot community that is not there. |
| `I426` | note | No game is linked: what only the game knows was not looked up. |
| `I427` | error | A value outside what its key takes, or a part the game cannot read: the whole item is left out. |
| `I428` | warning | A config of the module that does not compose. |
| `I429` | warning | A new item's HUD section without a base, whose bases name no HUD section either. |
| `I430` | warning | A new item's death loot without a count: the death manager never spawns it. |
| `I431` | warning | A use scene's `like` on an item that has a scene already: the game keeps the scene it has. |
