# Characters (`.npcasset`)

A module's own NPC is one `.npcasset` file anywhere in the module: declarative Lua (`return {
... }` of literals, `docs/ASSETS.md` section 1) that the game reads as it is. The editor makes
nothing of it: the game makes the specific character, the profile, the strings and a trader's
config when its configs come up (the game's `docs/dead-air/MODDING.md`, "A module's
characters"). This page is what `asset.reference kind=character` prints.

```lua
-- Wolf, the guide at the rookie village
return {
  npc = 1,
  id = "guide",
  base = "guard",
  name = { rus = "Проводник", eng = "Guide" },
  community = "stalker",
  visual = [[actors\stalker_neutral\stalker_neutral_1]],
  icon = "ui_inGame2_neutral_2",
  start_dialog = "my_mod_guide_hello",
  rank = 300,
  money = { min = 100, max = 500 },
  supplies = { { section = "medkit", count = 2 }, "bread" },
}
```

## The keys

Everything but the header may be left out; an unset key is unset, not its default - a character
with a `base` inherits what it does not set, and the defaults apply once the chain is walked.

| Key | Type | Default | What the game does with it |
|---|---|---|---|
| `npc` | number | - | The header: the format version (1). The editor refuses a file of a newer version. |
| `id` | string | the file name (lower case, `_` for anything else) | `a-z 0-9 _`, not starting with a digit, up to 48. `<module>_<id>` is the character's name in the game: the spawn section (`[<module>_<id>]:stalker`), the `character_profile`, the specific character; `<module>_<id>_squad` is its one-man squad. A quest names the character by `id`. |
| `base` | string | none | Another character of the module to start from: every key this one does not set is inherited, through a chain if need be. |
| `abstract` | boolean | `false` | `true`: a template other characters start from, never a character in the game (no profile, no spawn section, no string). Needs no name and no model. |
| `name` | string or `{ rus = ..., eng = ... }` | none (required) | What the player reads. A plain string is Russian; a table carries every language at once, and a language the install lacks falls back to `rus`. |
| `bio` | as `name` | none | The character's description in the PDA. |
| `community` | string | `stalker` | The community, by its id in the linked game (the Characters tab lists them by the names the player reads). |
| `visual` | string | none (required) | The model, a path under `meshes` (`actors\...`) of the game or the module. It is the model the NPC keeps: an outfit changes an NPC's protection, never its look. |
| `icon` | string | `ui_inGame2_neutral_2` | The portrait: a texture id of a portrait sheet. |
| `start_dialog` | string | none | The dialog the NPC starts a talk with. |
| `spec_rank` | string | none | A rank by its name; `rank` by its number wins over it. |
| `rank` | number | `0` | The rank as a number (Dead Air counts to 14000, in eight names). |
| `reputation` | number | `0` | The reputation. |
| `money` | `{ min, max, infinite }` | none | Money at spawn: between `min` and `max` (`max` below `min` reads as `min`); `infinite = true` for a trader that never runs out. |
| `supplies` | list | none | The starting kit: rows `{ section = "medkit", count = 2 }` or a bare section (`"bread"`, one item). A row of a section the base gives takes its count and keeps its place; `count = 0` takes an inherited row away. |
| `squad` | boolean | `true` | `false`: no one-man squad section is made for the character. |
| `trade` | `false` or a table | none | A shop (below). Inherited or replaced as a whole; `trade = false` takes a base's shop away. |
| `editor` | table | none | The editor's own notes; the game reads none of it. |

The game always gives the profile a `community` (`stalker` when unset), a `rank` and a
`reputation` (0 when unset): the engine asserts on all three. The profile names the specific
character outright, so the engine never picks another one of its class.

### A shop: `trade`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `sells` | a category or a list of them | nothing | What the trader sells: categories of the linked game's `misc\trade\trade_presets.ltx`. |
| `buys` | as `sells` | nothing | What it buys. |
| `markup` | number | `1` | What it asks, as a part of an item's cost. |
| `pays` | number | `0.3` | What it pays, as a part of an item's cost (never below 0). |
| `restock` | number | `24` | Game hours between restocks (at least 1). |
| `stock` | list | none | What is always on the shelf after a restock: rows `{ section, count = 1, chance = 1 }` (chance in 0..1) or a bare section. |

The game makes its own trade config of it (`misc\trade\<module>\<id>.ltx`, named in the spawn
section as `xms_trade`): what the trader does not sell or buy is switched off item by item, the
stock is always for sale, the prices go through `discounts`. The shop holds under any logic the
NPC runs.

## How the editor reads and writes it

- **The header is `npc`.** A file of a newer version is refused ("made by a newer Grizzly
  Editor"); there is no older version yet. A file without the header reads as the current
  version and gets it with its next save.
- **A save writes only what changed** (`docs/ASSETS.md` section 3): the Characters tab compares
  the character as it read it with the character as it is and writes that difference into the
  file. Comments, the layout, the order of the keys and keys the editor does not know stay as
  they are.
- **A new file** (`file.create kind=character`, the tab's New) is written in the canonical
  layout: the header first, the keys in the order of the table above, tab indentation.
- `asset.check` gives every problem with its line and column.

## Problems

| Code | Severity | What |
|---|---|---|
| `E100` | error | The file does not read (its sentence names the line and column). |
| `E101` | error | Two characters share an id. |
| `E102` | error | The id is not usable. |
| `E103` | error | No name. |
| `E104` | error | No visual. |
| `W105` | warning | The linked game has no model at that path. |
| `W107` | warning | A text holds characters cp1251 does not have: they do not reach the player. |
| `E108` / `E109` | error | A supply row without a section, or asking for less than one item. |
| `E110` | error | Money `max` below `min`. |
| `E111` / `E112` | error | A base that does not exist; a base chain that loops. |
| `W114` | warning | A community the linked game does not have (right only when the module brings it). |
| `W115` / `W116` / `W117` | warning | A portrait, a dialog, a kit item the game and the project do not have. |
| `E118` / `E119` | error | A stock row that cannot work; prices or restock out of range. |
| `W120` / `W121` / `W122` | warning | An unknown trade category, an unknown stock item, a trader with nothing to sell. |
| `E123` | error | The model lacks a bone the stalker damage names: the game stops when the NPC comes online. |
