# Traders (`.tradeasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-items/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-items` fails when this file is not what the tables say.

One file anywhere in the module is what the module puts on the shelf of ONE trader of the game. Several files may speak of one trader; they are read in the order of their paths, a later row of the same item and stock section winning. A character of the module is no such trader: its shop is `trade` of its own `.npcasset` (`CHARACTER.md`). It is plain data like every asset (`docs/ASSETS.md`).

**A file says what its thing is, and nothing of another thing** (`docs/ASSETS.md`, "One thing, one place"): the rows are written here, in the file of the side that owns them, and never in an item (`ITEM.md`). The item shows them as a fact - `items.info` answers `found`, the Items tab lists them under Where It Is Found with this file to open. Mod > Economy edits the three kinds side by side and saves as it is edited; `economy.list` answers every file with its rows and problems. The module ships the file as it is and the game reads it after the module's items, so a row may name an item the module makes.

```lua
return {
trade = 1,
trader = "trade_military",       -- misc\trade\trade_military.ltx
stock = {
{ item = "wpn_sil_diy", count = 1, chance = 0.5 },                         -- [supplies_1] wpn_sil_diy = 1, 0.5
{ item = "ammo_9x39_ap", supplies = { "supplies_2", "supplies_3" }, count = 40 },
{ item = "vodka", count = 0 },                                             -- taken off the shelf
},
}
```

## What the game makes

A row is a line of the trader's config: `[<supplies>] <item> = <count>, <chance>` for each stock section it names, `supplies_1` when it names none; a count of 0 takes the line away. They are patches of that config, as a `.ltxp` `#file` makes them: a line of a stock section reaches the sections that inherit it (the game's traders stock their later levels over the first), and a config the game writes is read as written. What a trader pays and asks for an item, and which goods category of `misc\trade\trade_presets.ltx` an item is in, no trader file says: that is a patch of those configs (`patch\*.ltxp`).

## The keys

| Key | Takes | Left out | Means |
|---|---|---|---|
| `trade` | integer | - | The format version (1). A file of a newer version is refused by name. |
| `trader` | string | - | The trader of the game the file is about: its config's name (`trade_military` is `misc\trade\trade_military.ltx`), or a path under `configs` ending in `.ltx`. A module's own character keeps its shop in its `.npcasset`. |
| `stock` | a list of stock rows (below) | - | What the module puts on the trader's shelf. |
| `editor` | anything | - | The editor's notes. The game reads none of it. |

## A row of `stock`

| Key | Takes | Left out | Means |
|---|---|---|---|
| `item` | string (a section) | - | The item: a section of the game or of the module. |
| `supplies` | string or list (sections) | "supplies_1" | The stock section of the trader's config the row goes into, or a list of them. A section that inherits another takes its rows. |
| `count` | number, 0 to 1000000 | 1 | How many are on the shelf after a restock. 0 takes the item off the shelf. |
| `chance` | number, 0 to 1 | 1 | The chance they are. |

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
