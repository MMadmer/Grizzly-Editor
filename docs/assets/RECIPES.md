# Recipes (`.recipeasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-items/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-items` fails when this file is not what the tables say.

One file anywhere in the module is a list of recipes: what is made, at which tool or workbench, from one to four parts. A module may keep one file or several. It is plain data like every asset (`docs/ASSETS.md`).

**A file says what its thing is, and nothing of another thing** (`docs/ASSETS.md`, "One thing, one place"): the rows are written here, in the file of the side that owns them, and never in an item (`ITEM.md`). The item shows them as a fact - `items.info` answers `found`, the Items tab lists them under Where It Is Found with this file to open. Mod > Economy edits the three kinds side by side and saves as it is edited; `economy.list` answers every file with its rows and problems. The module ships the file as it is and the game reads it after the module's items, so a row may name an item the module makes.

```lua
return {
recipe = 1,
recipes = {
{ makes = "wpn_sil_diy", at = "itm_repairkit_tier_1", parts = { "jar", "textile_patch_m", "rope", "flashlight_broken" } },
{ makes = "ammo_9x18_fmj", count = 40, at = "verstak_ammo", parts = { "ammo_bad", "ammo_bad" } },
},
}
```

## What the game makes

One line a recipe in the system config's `[xms_craft]`: `<n> = <at>, <makes>, <count>, <part>, ...`, `n` a zero-padded number after the highest the section holds, so the recipes are listed in the order they are written, after the craft window's own. `at` is the section the craft window is opened with: a tool (`itm_repairkit_tier_1`, `itm_drugkit`) or a workbench. A result whose section holds `ammo_` is made as that many rounds, in boxes.

## The keys

| Key | Takes | Left out | Means |
|---|---|---|---|
| `recipe` | integer | - | The format version (1). A file of a newer version is refused by name. |
| `recipes` | a list of recipes (below) | - | The recipes. |
| `editor` | anything | - | The editor's notes. The game reads none of it. |

## A row of `recipes`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `makes` | string (a section) | - | What the recipe makes: a section of the game or of the module. |
| `count` | integer, 1 to 1000 | 1 | How many it makes. |
| `at` | string (a section) | - | The tool or the workbench the recipe is in: the section the craft window is opened with (`itm_repairkit_tier_1`). |
| `parts` | one to four sections | - | What it takes. The game writes the recipe as `[xms_craft] <n> = at, makes, count, part, ...`. |

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
