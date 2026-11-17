# Loot tables (`.lootasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-items/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-items` fails when this file is not what the tables say.

One file anywhere in the module is a table of what the dead carry: an item, how many, and the chance - for everybody and for a community or a community and a rank. A module may keep one table or several. It is plain data like every asset (`docs/ASSETS.md`).

**A file says what its thing is, and nothing of another thing** (`docs/ASSETS.md`, "One thing, one place"): the rows are written here, in the file of the side that owns them, and never in an item (`ITEM.md`). The item shows them as a fact - `items.info` answers `found`, the Items tab lists them under Where It Is Found with this file to open. Mod > Economy edits the three kinds side by side and saves as it is edited; `economy.list` answers every file with its rows and problems. The module ships the file as it is and the game reads it after the module's items, so a row may name an item the module makes.

```lua
return {
loot = 1,
drops = {
{ item = "wpn_sil_diy", count = { 0, 1 }, chance = 0.01, by = { bandit = 0.03, stalker_veteran = 0.02 } },
},
}
```

## What the game makes

`count` is the item's line of `misc\death_generic.ltx [item_count]` (`{ min, max }` as `min, max`); `chance` its line of `misc\death_items_by_communities.ltx [base]`, which every community and rank inherits; each `by` entry its line of that community's own section. They are patches of those configs, as a `.ltxp` `#file` makes them. An item of the game has its count already; one the module makes needs a row that gives it one, or the death manager never spawns it (`I430`).

## The keys

| Key | Takes | Left out | Means |
|---|---|---|---|
| `loot` | integer | - | The format version (1). A file of a newer version is refused by name. |
| `drops` | a list of drops (below) | - | What the dead carry. |
| `editor` | anything | - | The editor's notes. The game reads none of it. |

## A row of `drops`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `item` | string (a section) | - | The item: a section of the game or of the module. |
| `count` | number or `{ min, max }` | - | How many the dead carry when they carry it: its line of `[item_count]`, a number or `{ min, max }`. Without one the death manager never spawns the item. |
| `chance` | number, 0 to 1 | - | Its line of `[base]`, which every community and rank inherits. |
| `by` | `{ community = chance }` | - | Lines of their own: `{ bandit = 0.03, stalker_veteran = 0.01 }` - a community, or a community and a rank. |

## Problems

`asset.check` names every problem at its row; the build refuses a module with an error, with the first one. The game takes a file whole or not at all: an error of its shape leaves the whole file out.

| Code | Severity | What |
|---|---|---|
| `I400` | error | The file does not read. |
| `I412` | warning | A chance outside 0 to 1. |
| `I417` | warning | An item a row names that is no section of the game or of the module. |
| `I423` | warning | A recipe's result, tool or part that is not there. |
| `I424` | warning | A trader whose config is not there, or lacks the stock section a row names. |
| `I425` | warning | A death loot community that is not there. |
| `I426` | note | No game is linked: what only the game knows was not looked up. |
| `I427` | error | A value outside what its key takes, or a row the game cannot read. |
| `I430` | warning | A drop of an item the module makes with no count in any loot table: the death manager never spawns it. |
