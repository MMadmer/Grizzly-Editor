# Brief S - the Start page, the Content Browser, previews and libraries

Read first: `docs/plan/RULES.md`, `docs/plan/PLAN.md` (1-3, 5, 6), `docs/COMMANDS.md`, `docs/ASSETS.md` (6, 8, 9),
`docs/UI_DESIGN.md` (1a, the kit, and the sections of your surfaces), the reports you build on
(`docs/plan/reports/K.md`: kit widgets, `drop`, actions, the page layout; `A2.md`: projects, the default game,
`file.*`, the library, `project.collect`; `L1a.md`: `.scene`, groups as assets), and the audits
`Intermediate/plan2/ux_rest.md` **B.2** and `Intermediate/plan2/ux_level.md` **B11, C ("Content Browser",
"Libraries and previews"), D4-D6, E.12, E.24** with their pictures.

You own: `crates/gz-ui-app` (the Start page, Link Game, Recovery, the Output Log drawer), `crates/gz-ui-content`,
`crates/gz-ui-previews`. Stream L2 works beside you in `gz-ui-level` / `gz-ui-viewport` and takes your drag
payloads: the payload types are the kit's (`gz_ui::drop`), so agree through it.

## 1. The Start page

- New Project is a card in the grid's first slot (nothing moves when its form opens; Create is the Primary
  while the form is up - audit B.2); the search is capped at 24 em; the grid has no frame around nothing.
- A card shows the project by its **project file** (stream A2): name, path with a start ellipsis, the preview,
  Missing / Not a project badges as now; **Open Project File...** beside Open Folder... (the system dialog
  filtered to `*.grizzly`); dropping a `.grizzly` file or a project folder on the page opens it.
- Link Game appears only when no game is known (A2); it lists the candidates the editor found, the first one
  selected, **Link** the Primary.
- No job of a closed project shows here (stream K stops them; check).
- The page says the editor's version in a corner and shows the update chip like any other page.

## 2. The Content Browser lists assets, not files of the SDK

- **One tile per asset** (audit B11): an `.object` with its `.thm` is ONE tile whose picture is the thumbnail;
  a `.thm`, a `.~` backup and other sidecars are never tiles. Folders that hold only sidecars are empty.
- **Sources**: Project (the project's files, by its own layout) and **Library** - what can be brought into the
  project or placed, merged from the linked game and the editor's library and shown by kind: Objects, Groups,
  Textures, Sounds, Particles, **Spawn Sections** (the game's sections by their in-game names, grouped as the
  Place panel groups them), Base Scenes. No "Editor" and "Game" as two trees of raw folders.
- A scene is one tile (`<name>.scene`, its folder hidden as part of it); an SDK `.level` shows with an
  "imported on open" hint; the project file is not a tile.
- **Drag payloads** (the kit's `drop`): every placeable tile - object, group, particle, sound, spawn section -
  drags into the viewport (stream L2 places it) and into fields that take that kind (a Characters' Model field
  takes a model, a foliage list takes a mesh); files dragged in from Explorer import into the open folder
  (pictures become `.dds` where the target is a texture folder, as Screens & Music's import does - one import
  routine, `file.import`).
- **Place in Level** on a tile's menu places on the surface under the viewport's centre; **Bring into Project**
  on a Library tile (`file.import`); a Library asset that a scene uses and the project lacks is marked.
- The defects (audit C, E.24): the View popup's width, captions that break mid-word (list view under 64 pt
  tiles for folders), Fit to Height's minimum, the duplicate "Project" root, the upload budget for thumbnails
  (D4), `can_write` read in the list job (D6).
- Commands, typed: `content.view` (source, folder, selection, tile size, search - what the browser shows; to
  look at and to drive), replacing `content_browser_navigation`, `content_browser_selection`,
  `content_browser_open`, `content_browser_copy`; file operations are stream A2's `file.*` (your UI calls
  them).

## 3. Previews and libraries

- **Model and texture previews**: titles and words in the kit's style (no lower-case engine phrases); **Place in
  Level** and **Show in Content Browser** on the preview's toolbar; the texture preview's pipeline is made
  before it is needed (audit D5).
- **Libraries** (Texture, Sound, Light Animations): one empty state per tab (not three for one fact), the
  action that fills it as the single Primary; "Check New Textures", "Synchronize" and "Rewrite" become what
  import does by itself, with the manual commands under the tab's More (audit B11).
- Commands, typed: `preview.model`, `preview.texture`, `library.view` (`kind`, selection, the open dialog),
  `shot.asset` - replacing `preview_model`, `texture_preview`, `library`, `asset_preview` and the bridge's
  `asset_preview_model`.

## 4. The Output Log

The first visible line is never cut in half; a line's level is its colour and a word a filter can pick; the
SDK's `!` / `~` prefixes inside messages are gone at the source (grep the crates for messages that start with
them and write sentences); the drawer's search box is the kit's.

## 5. Documents and proof

- `docs/UI_DESIGN.md`: your surfaces' sections in the present tense.
- QA: `tools/qa/test_project_browser.py` (the Start page), `test_asset_icons.py`, `test_previews.py` rewritten
  for the new commands and extended: an `.object` and its `.thm` are one tile; a Library spawn section drags as
  a payload of the right kind; `perf.stalls` empty after scrolling a folder of 300 objects.
- Pictures into `Intermediate/plan/S/` with a caption each in your report; look at each yourself.

## Done means

PLAN section 5 "S"; every audit item named above fixed or answered; no local widget the kit has; one Primary per
surface.
