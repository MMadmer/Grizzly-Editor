# PDA interfaces (`.pdaasset`)

A module can give the PDA its own screens: pages, buttons, lists, menus, a slot for
the live map. The device, the map, the tasks and every stock section stay the game's;
what changes is the interface inside the device. This document is the contract between
the editor and Dead Air Refined; the game repo's `docs/dead-air/MODDING.md`, "PDA
layouts", is its game half.

## How it fits together

- A layout is a `.pdaasset` file anywhere in the project: declarative Lua, like a quest
  (`return { ... }`, no code, no globals).
- The module names the one it uses in `mod.ltx`:

  ```ini
  [pda]
  layout = pda\app.pdaasset
  ```

  **Mod > PDA Interface** in the editor, the preview's **Use in Module** button and
  MCP `pda_use` all write that line.
- The build ships the file as it is. Nothing of the game is written: no `pda.xml`, no
  script of the game. A module without `[pda]` leaves the PDA alone, and a game without
  a module layout draws the stock PDA byte for byte.
- In the game, `scripts\xms_pda_layout.script` reads the file into a flat item list, and
  the engine (`CUIPdaLayout`) draws it inside the device's screen. The editor runs the
  **same** script, from the linked game (the bundled copy
  `gamedata\configs\pda\xms_pda_layout.script` without one), for its checks and its
  preview. The two cannot read a layout differently.
- Only one layout shows. When two modules that apply to the game both bring one, the
  later one in the load order wins, and the conflict is logged (`xms_report.json`).
- The layout applies when its module does (`[module] mode`, `XMS::ModuleApplies`), so
  a mod's own mode gets its own PDA and the plain game keeps the stock one.

## The file

```lua
return {
  pda   = 1,                          -- the format; anything else is E301
  id    = "shmyandex",                -- lowercase letters, digits, _ (E302)
  title = "ШМЯндекс",                 -- or { rus = "…", eng = "…" }; the {title} placeholder
  canvas = { 960, 680 },              -- optional, the default; the coordinates below use it
  start = "orders",                   -- the page the PDA opens on (default: the first)
  keys  = { map = "map", tasks = "orders", contacts = "stock:eptContacts" },
  device = { noise = true, close = true },   -- the stock screen noise and close button
  colors = { accent = "#FFD21E", card = "#1E2026CC" },
  styles = { tab = { kind = "button", color = "none", text_selected = "accent" } },
  vars   = { current = "No order" },  -- {current} in texts; scripts and actions change it
  chrome  = { … },                    -- widgets under every page
  pages   = { { id = "orders", title = "Orders", widgets = { … } },
              { id = "journal", title = "Journal", stock = "eptTasks" } },
  overlay = { … },                    -- widgets over everything, stock sections included
  preview = { level = "l03_agroprom", vars = { money = "12450" } },   -- the editor's only
}
```

### The canvas

Coordinates are canvas units, `960 x 680` unless `canvas` says otherwise. The canvas is
laid over the device's screen - the rect of `noice_static` in `pda.xml` (4:3) or
`pda_16.xml` (16:9) of the game - each axis scaled on its own, so the same file fills
the screen in both shapes. Children of a group or a list item are relative to it.

### Colours

`"#RRGGBB"`, `"#RRGGBBAA"`, or a name: one of `colors`, or `white`, `black`, `none`
(`transparent`). A name that is not a colour is `E311`; a wrong palette entry is
reported once, where it is declared.

### Texts

A string, or `{ rus = "…", eng = "…" }` (the game's language, `rus` when it has none).
Files are UTF-8; the game converts to cp1251, so a character cp1251 lacks shows as `?`
(the game's fonts are cp1251 fonts - `→`, `★` and `×` are not in them; use the icons).
Dead Air's fonts also draw `—` and `–` blank although cp1251 has them: use `-`, `»` or
`·`. `letterica25` is Dead Air's 16 px texture; the big fonts are `graffiti19`,
`graffiti22`, `graffiti32` and `graffiti50`.

`{name}` placeholders are filled in by the game every quarter second:

| placeholder | what |
|---|---|
| `{time}` `{date}` | game time (HH:MM) and date |
| `{actor}` `{money}` `{rank}` `{community}` | the player |
| `{level}` | the level, as the game names it |
| `{task}` | the active task's title |
| `{title}` `{page}` | the layout's title, the page's title |
| any other | a var: `vars`, `var:` actions, `xms.pda.set` |

A name that is none of these is `W319`. The preview fills the game's own from
`preview.vars` or a stand-in.

### Pages, chrome, overlay

- `chrome`: under every page (a background, a top bar, a tab bar).
- `pages`: `{ id, title, widgets }`, or `{ id, title, stock = "<section>" }` - a page
  that shows a stock PDA section (`eptTasks`, `eptRanking`, `eptRelations`,
  `eptContacts`, `eptLogs`, `eptEncyclopedia`, ...). While a stock section shows, the
  chrome and the pages are hidden and the overlay stays on top: put the way back there.
- `overlay`: over everything - menus, dialogs, a back button.

Draw order is file order; a later widget is on top, and gets the click first.

### Widgets

Every widget: `kind`, `x`, `y`, `w`, `h` (`E306` when one is missing), and optionally
`id` (lowercase, unique: `E307`), `visible = false`, `style`, `hint` (shown on hover).
A key a kind does not know is `W318` (a typo check).

| kind | keys | what |
|---|---|---|
| `rect` | `color`, `radius`, `border`, `border_w` | a (rounded) rectangle; with a colour it takes the clicks that land on it |
| `image` | `texture`, `color` (tint), `radius`, `stretch` | a texture id of `textures_descr` (the game's, Refined's sheet, the module's own) or a file under `textures\` |
| `text` | `text`, `font`, `color`, `align` (`left`/`center`/`right`), `valign` (`top`/`center`/`bottom`), `wrap` | a label; one line cut with `...`, or wrapped |
| `button` | a rect's keys, a text's keys (`text_color` is the label), `hover`, `press`, `selected`, `text_hover`, `text_selected`, `icon`, `icon_size`, `icon_color`, `icon_selected`, `action`, `page`, `selected_on` | clickable |
| `group` | `widgets`, a rect's keys | a container: a panel, a menu, a dialog |
| `list` | `item` (the widgets of one row), `rows`, `row_h`, `gap`, `action`, `row_color`, `row_hover`, `row_selected`, `row_radius`, `empty`, `font`, `text_color` | rows that scroll |
| `map` | `color` (under the map), `zoom` (`actor`, the default, or `global`) | the live map: the level, the player, every map spot and task marker, zoom and drag as in the stock PDA; one per layout (`E313`) |

`font` is one of the game's: `letterica16`, `letterica18` (the default), `letterica25`,
`graffiti19`, `graffiti22`, `graffiti32`, `graffiti50`, `arial_14`, `medium`, `small`,
`di`. Anything else is `W316` and draws in the default.

`hover` and `press` default to a lighter and a darker shade of `color`.

#### Buttons

- `page = "<id>"` is the tab shorthand: the action `page:<id>` (after any `action`), and
  the button is `selected` while that page shows. `selected_on` names the pages itself.
- The icon and the caption are placed like this, in canvas units:
  - icon alone: centred; icon size `min(icon_size, w, h)` (`icon_size` defaults to
    `min(h * 0.6, 32)`);
  - `align = "center"` and `valign` other than `center`: the icon above the caption -
    icon at `(w - s) / 2, h / 2 - s + 2`, caption from `h / 2 + 2` down;
  - otherwise the icon at `x = 12`, vertically centred, and the caption from
    `12 + s + 8`;
  - a caption aligned left or right keeps `12` from the edges.
- A button without an action (and without a hint) is `W321`.

#### Lists

`item` is the template of one row; its strings take `{field}` from each row, colours
and textures included (`color = "{status_color}"`). `rows` are the rows the file ships;
a script replaces them at run time with `xms.pda.rows(id, rows)`. `action` is what a
click on a row does (with the row's fields); the clicked row becomes the list's
selection (`row_selected`). `empty` shows while there are no rows. A map or another
list inside a row is refused (`E314`, `E312`). At most 200 rows show (`W322`).

### Actions

`action` is one string or a list, run in order.

| action | what |
|---|---|
| `page:<id>` | open a page (`E308` when there is none) |
| `back` | the page before, or out of a stock section |
| `close` | close the PDA |
| `show:<id>` `hide:<id>` `toggle:<id>` | a widget (`E308` when no widget has the id) |
| `stock:<section>` | a stock PDA section (the page for it, when the layout has one) |
| `map:actor` `map:zoom_in` `map:zoom_out` `map:global` | the map (its page opens first) |
| `var:<name>=<value>` | set a var; `{placeholders}` in the value are filled in first |
| `signal:<name>` | `event.signal` of any quest hears it, as `<name>` or `<module>.<name>` |
| `lua:<script>.<function>` | the game calls `script.function(item_id, row)` |

The game's own placeholders cannot be set (`E308`).

### Keys

What the game's keys open while the layout is up: `map` (the map key, `Show_MapWnd`,
and a request for `eptTasks`/`eptMap` from anywhere - Dead Air's map key asks for
`eptTasks`), `tasks` (the task list of a tutorial), `contacts`. Each is a page id,
`page:<id>` or `stock:<section>`. Defaults: `map` - the map's page (the start page
without a map), `tasks` - the start page, `contacts` - `stock:eptContacts`. Any other
section a script asks for is shown as it is, over the layout, under the overlay.

### Styles

`styles = { name = { any widget keys } }`; a widget's own keys win over its style's.
A style may name another (`style = "base"`). A style may give the `kind`, so
`{ style = "h1", x = …, text = … }` is a whole widget. An unknown style is `E310`.

### preview

The editor's alone; the game never reads it.

- `level` - the level whose map the preview's map shows;
- `actor = { x, z }` - where the player stands on it;
- `markers = { { x, z, icon, hint, spot } }` - spots on it; `spot` names a `map_spots` node whose look the marker takes;
- `vars` - values for the preview only, `{time}`, `{money}` and the like included.

### map_spots

The look of the game's own map spots while the module is on - the PDA map, the
minimap, the stock task markers:

```lua
map_spots = {
  actor_level_spot = { icon = "ui_xms_icon_circle", color = "#FFFFFF", size = 22, level = -1,
                       inner = { icon = "ui_xms_icon_circle", color = "accent", size = 14 } },
  secondary_task_spot = { icon = "ui_xms_icon_pin", color = "accent", size = 30 },
  secondary_task_spot_mini = { icon = "ui_xms_icon_pin", color = "accent", size = 16, arrows = "secondary_task_spot" },
},
```

A key is a stock spot NODE of `ui\map_spots.xml` (`actor_level_spot`, `actor_level_spot_p`,
`green_spot`, `secondary_task_spot`, `storyline_task_spot`, their `_mini` twins, ...); the
module only restyles, never adds a spot type - a script that names a type the game does not
know stops it. `icon` is a texture id, or a texture file (the project's own `gamedata	extures\...` too) with `rect = { x, y, w, h }` - the piece of it in its pixels; `color` a colour (tints the icon), `size` 4..128 (24), `level` the
`location_level` (5), `heading = true` turns it with the player, `inner` an icon drawn on top
(`icon`, `color`, `size`, `x`, `y` inside the spot; centred by default), `arrows` the name
the minimap's `_below` / `_above` textures start with. While the layout is on the game
applies the table to the spots it reads from `ui\map_spots.xml` (`ui\map_spots_16.xml` on a wide
screen). The preview draws the player with `actor_level_spot`. A bad entry is
`E323`, an unknown key in one `W324`.

## The preview tab (the editor)

**Mod > PDA Interface**, a double click on a `.pdaasset` and MCP `pda_preview action=open`
bring up the editor's `pda` tab: one of them, switching to the next layout it is given.

- **Toolbar:** Save, Revert, Reload | Aspect (4:3 / 16:9), Language (rus / eng), Zoom
  (Fit / 100%) | Reset State | Source, Details. Reload reads the game's textures, fonts and
  layout reader again, and the file when the Source pane holds no edits.
- **Canvas:** the device centred; Fit fits the device itself (its texture, the screen and the
  close button - on 16:9 the texture covers only the middle of the game's screen) on both
  axes with 16 pt round it, 100% is the game's 1024 x 768 PDA pixel for pixel. The wheel
  zooms at the mouse (over a list or the map it scrolls or zooms
  them, the way the game does; Ctrl+wheel always zooms), the right or middle button drags,
  F fits. The left button plays the layout. The device's art runs on past the game's
  screen - its bottom grilles are cut where the monitor ends, on both aspects - so every
  side of the device that lies on the screen's edge gets that edge drawn, a little longer
  than the device.
- **Details** (320 pt, beside the canvas): the file and whether the module uses it (**Use in
  Module** writes `[pda] layout`), then the tabs **Problems** (first: the reader's error with
  its line, and every coded problem), **Pages** (click one to go there; the map, contacts and
  tasks keys below), **Vars** and **Log** (the actions run, newest first).
- **Source** (under the canvas): the text in Cascadia Mono with line numbers and the reader's
  error line marked. **Apply** (Ctrl+Enter) shows the text without saving it.
- Unsaved text is never lost without a question: Revert names the file first, and closing
  the tab or opening another layout asks Save / Don't Save / Cancel. Over MCP nothing asks:
  a call that would drop the edits answers `{"ok":false,"error":"unsaved","dirty":[...]}`
  unless it carries `save=true` or `discard=true`.

## At run time (the game)

Scripts change a live layout through `xms.pda` (feature-test `xms.pda_api == 1`):

| call | what |
|---|---|
| `xms.pda.active()` | the layout's id, nil for the stock PDA |
| `xms.pda.set(name, value)` / `get(name)` | a var; nil clears. Vars ride the save |
| `xms.pda.page(id)` / `current()` | the page |
| `xms.pda.show(id, on)` / `hide(id)` / `toggle(id)` | a widget |
| `xms.pda.text(id, text)` | a text's or a button's caption |
| `xms.pda.rows(id, rows)` | a list's rows: `{ { field = value, … }, … }` |
| `xms.pda.click(id, row)` | what a click on the item does; with `row`, a list's row (1-based) |
| `xms.pda.open(page)` / `close()` | the PDA itself ("a new order came in") |
| `xms.pda.describe()` | the live layout as text: pages, items with an id and what is shown, and while the map slot is up one `spot <object id> shown\|pointer <x>,<y> <hint>` line per map spot it draws (listed only while the slot holds it; a pointer is the edge arrow to one outside) - how a script or an agent knows the player and a quest's marker are on the app's map |

Values may be UTF-8 or cp1251. Pages, visibility and rows start over with every level
load; vars do not (engine blob `0x584DFF03` of the `.scov`).

The texture sheet `ui_xms_pda` ships with Refined: `ui_xms_pda_white`, the fades
`ui_xms_pda_fade_down|up|left|right` and white icons to tint - `ui_xms_icon_<name>` for
star, star_empty, star_half, circle, ring, dot, pin, box, bag, map, list, chat, user,
clock, check, cross, plus, minus, chevron_left|right|up|down, arrow_left|right, menu,
dots, search, bell, home, settings, info, warning, coin, route, phone, lock, heart,
flag, target, battery, signal, eye, camera, skull.

## Codes

| code | meaning |
|---|---|
| `E301` | not a layout: the file returns no table, or `pda` is not 1 |
| `E302` | `id` missing or not lowercase letters, digits, `_` |
| `E303` | no pages |
| `E304` | a page without an id, a page id used twice, a bad `stock` |
| `E305` | an unknown widget kind, or a widget that is not a table |
| `E306` | a missing or wrong geometry, number or text |
| `E307` | a bad or repeated widget id |
| `E308` | an action that cannot run: unknown verb, missing page or widget, a game placeholder set |
| `E309` | `start`, `keys` or `selected_on` names a page that does not exist |
| `E310` | an unknown style, or a style that inherits from itself |
| `E311` | a colour that is not one |
| `E312` | a list without `item`, or a list inside a list |
| `E313` | a second map |
| `E314` | a map inside a list |
| `W315` | a stock section the game does not have (a global mod may add one) |
| `W316` | a font the game does not have |
| `W317` | a widget outside the canvas |
| `W318` | a key that means nothing to that kind |
| `W319` | a placeholder that is neither the game's nor a var |
| `W320` | a texture neither the linked game nor the project has (editor check) |
| `W321` | a button without an action |
| `W322` | more than 200 rows |
| `E323` | a `map_spots` entry that is not a stock spot node name = { icon, ... } |
| `W324` | a key a map spot does not know |

An error stops the build; in the game, a layout with an error is not shown and the
stock PDA stays (the log says why).

## MCP

| command | what |
|---|---|
| `pda_list` | the project's layouts, the used one marked |
| `pda_get` | one layout: text, outline, problems |
| `pda_write` | create (template) or replace; `use` picks it |
| `pda_delete` | delete (final); clears `[pda]` when it was the used one |
| `pda_use` | set or clear `[pda] layout` |
| `pda_validate` | check a text or a file |
| `pda_preview` | open / state / click (`id`, `id` + `row`, `n`, or a canvas `x`,`y`) / page / key / aspect / lang / reset / reload / close; apply (`text=` into the Source pane, unsaved) / save / revert. With any action: `pane=source\|details\|both\|none`, `tab=problems\|pages\|vars\|log`, `zoom=fit\|100\|<0.25..4>`, and `save=` / `discard=` for unsaved edits. The answer gives `active`, `text_dirty`, `panes` (`source`, `details`, `tab`, `details_width`, `source_height` in pt), `zoom`, `scale` (what the frame drew at), `device_gap` (pt between the device and the canvas' nearest edge, below 0 when cut), `screen_edges` (the device's sides that lie on the game's screen edge: `top`, `bottom`, `left`, `right`), `read_line`; the state gives `map.bound`, the preview level's world rect, and `map.spots`: the player and the `preview.markers` the slot shows, in canvas units. The answer is the frame the call asked for - the editor holds it until the tab was drawn, so an opener's panes, `scale`, `device_gap` and `screen_edges` need no second call (`pending` is false) |

The preview follows the file: a write shows at once, unless the Source pane holds edits.
`open` activates the tab; every other action leaves the active tab alone.
`screenshot_editor` after `pda_preview action=open` is what the author sees.
