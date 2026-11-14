# Grizzly Editor UI - design specification

This document is the contract of the editor's UI: the kit (`gz-ui`), the shell (`gz-app`) and the surface crates (section 8). CLAUDE.md, AI_SETUP.md, NQ_ARCHITECTURE.md, NQ_FORMAT.md and PDA_FORMAT.md follow it. A UI change is a change of this document first; where it disagrees with another document, it wins.

- **pt** is a logical point (egui's unit). px = pt x the context's pixels per point (the **scale**).
- Every surface is held at 1707 x 912 pt and 1280 x 720 pt, at the scales 1.0 and 1.5.

## 1. Principles

1. **Documents own the window.** Every asset editor is a full-window major tab. **Level** is tab 1 and cannot be closed; its centre holds only the viewport.
2. **The canvas absorbs change.** The viewport, graph, preview or tile grid takes every resize. Side panels, drawers and label columns keep their pt size. Below its minimum a container scrolls; it never clips.
3. **Nothing clips silently.** Text fits, ellipsizes with a tooltip, or wraps if it is prose. Rows overflow into `»`. `ui.audit` checks it.
4. **One size system, one kit, one widget per job.** Sizes are pt (`metrics`, `units::em`), and DPI enters in one place. The kit's types leave no room for a raw px size or a colour literal. Each job has exactly one `gz-ui` widget (section 4).
5. **Quiet chrome, loud content.** Accent means active, selected, focused or primary; semantic colours mean state, and node families have hues of their own. Accent `7971bd` is never text colour, and required is never grey.
6. **Name first.** Labels name the thing. Help lives in tooltips. Values from a list are picked.
7. **Safe and honest.** Destructive actions name what goes, and Cancel is the default. Unsaved work is never lost silently. A label that depends on state keeps its width.
8. **Stable chrome.** Switching tabs moves nothing of the frame: the caption, the tab row and every tab in it, the toolbar row, the project's group at its right end, the status bar.
9. **Agent-drivable.** Everything the window does is an action that runs a registry command, so every menu row, button, key and palette row has an MCP equivalent with the same code behind it. Activation is synchronous, only the spinner animates, and MCP never prompts.
10. **The UI thread never waits.** No file, network, lock a job may hold, GPU readback or pipeline compile on it. The stall monitor (section 5) proves it.

## 1a. Simple, visible, and an agent's as much as a person's

The rule every surface is held to; the Screens & Music tab is its reference. A screen that fails it is a bug, whoever wrote it.

- **One task, one view.** A list of the things on the left, the selected thing on the right. No dashboards of controls, no second window for what fits in the tab.
- **Show the common case, fold the rest.** What most authors touch is on the surface; everything else is under one folded **More**. Nothing is shown just because it exists.
- **Nothing to declare that can be worked out.** The kind of a thing comes from its file, a name from the file's name, a default from the module. A field is the last resort.
- **Direct manipulation.** + on the list's header, a cross on the row, drag to reorder, drop a file where it belongs. No "Add" dialogs and no Apply: an edit is saved as it is made.
- **Show the result, not its numbers.** A preview, a picture, the thing as it will look; a thing that cannot be shown is marked where it goes.
- **Problems stay with what they are about.** A dot on the row, a sentence for the author on its details. No codes on screen, no log to go and read.
- **Everything a person can do, an agent can do, by the same code.** Every action has an MCP command; every result of it is readable text on disk (`mod.ltx`, the declarative assets), so an agent can also just edit the file. A surface without both is not finished.

**Applied (section 1a):** the graph toolbar keeps Save, Undo, Redo, the problems count when there are any, Frame All, Find, the panels and the zoom; the rest is its More menu. The Graph panel adds tasks and roles with + on the header and removes them with a cross on the row, ID and a role's details under More. Mod Settings, World Rules and Screens & Music save as they are edited and say so in the one place, the toolbar row; Characters saves on Save (its toolbar, Ctrl+S), with a dirty dot on the row and a question before unsaved edits are left, and shows the character as the game will. World Rules keeps Original Game, Player Start and Starting Kit on the surface, the rest under More, and shows a problems bar only when there are problems. Publish Release is one Release line and the Notes; Package Release is one click; Submit shows a working key as one line and one spinner per job. The Start page is the projects, its form opens on New Project.

## 2. Tokens

**Colours.** The theme's `Palette` (`gz-ui`'s `theme.rs`) holds every colour; a surface never writes one of its own.

| Token | Value | Use |
|---|---|---|
| chrome | 17161b | caption, tab row, status bar, inactive tabs, dock strips, window brush |
| base | 202025 | panels, documents, toolbar, active tab |
| inset | 1c1b20 | lists, tables, grids, tiles, log |
| canvas | 151419 | viewport, graph, previews |
| raised | 26252c | headers, cards, node bodies |
| overlay | 28272e | menus, pick lists, tooltips, dialogs, toasts, the Task card; pills at 92% |
| input | 2a2931, hover 302e38 | every field, combo and number box |
| hover / pressed / button | 2e2c35 / 36343f / 34323d (hover 3d3a47) | |
| border | subtle 2c2a33, 3d3947, strong 524d5f | |
| text | f2effb, secondary bdb8c9, muted 8e8a99, disabled 5f5b69 | contrast on inset 15.2 / 8.6 / 5.1 / 2.6 |
| accent | 7971bd | active tool, checkbox fill, switches that are on, drag |
| accent.bright | a79fed | focus, check mark, caret, links, tab overline, toggle underline, active icons, tile outline, marquee, info |
| selection | 464150; focused 554f72 | |
| primary | 6a62ae, hover 7971bd, pressed 5c5599 | white semibold label, 5.3:1 |
| danger | 8a3b43, hover a3464f, pressed 74323a | |
| status | error e06c75, warning e5c07b, success 98c379, required d19a66 | state only, never a family |
| axes | X e06c75, Y 98c379, Z 61afef | |
| graph.family.0..9 | ten hues of one lightness (`Palette::family(n)`) | node families, the pick list's row tints |
| wire.* | base 8e8a99, hot f2effb, selected accent.bright, dim 3d3947 | graph links |
| pin.* | exec bdb8c9, input 8e8a99, hot f2effb, target accent.bright | graph pins |
| frame.* | fill and title: 9a98d6 at 7% / 18% | graph comment frames |
| tint / dim | tone at 16% / 0c0b0e at 60% | |

Graphite changes only the accent family: accent 727782, bright 9399a5, selection 454951 and 50545d, primary 60656f.

**Elevation.** Shadows are tokens: `shadow.popup` (menus, pick lists, tooltips; 0,4 blur 14), `shadow.dialog` (0,10 blur 32 spread 2), `shadow.card` (drawers, toasts, the Task card, graph nodes; 0,2 blur 10). Popups and dialogs are a hairline (`border.subtle`) plus their shadow, never a heavy border.

**Inputs read as inputs.** A field, combo or number box has a 1 pt `border` outline at rest (`border.subtle` is the input's own fill, give or take, and is for separators), `border.strong` on hover and the 2 pt accent.bright focus ring when it has the keys; it is 26 pt tall. A wrong value has the error outline and its sentence under it.

**Metrics (pt).**
- Spacing steps: 2, 4, 6, 8, 12, 16, 24, 32. Item spacing 8,4; frame padding 8,4; indent 14; scrollbar 10; focus ring 2.
- Radius: 4 for controls, 6 for popups and cards, 8 for dialogs, 0 when docked.
- Disabled widgets dim to 0.4.

**Sizes (pt).**
- **Chrome:** caption 32 (window buttons 46 x 32); tab row 30, a major tab 28 tall at its foot, 96-220 wide (a folded tool tab 36); toolbar 36 (buttons 28, glyph 16, group gap 12); panel tab strip 26; panel toolbar 32; status bar 24.
- **Controls:** frame 26; row 22; property row 26; section header 28; inline icon button 24; menus 200 wide at least.
- **Dialogs:** S 400, M 560, L 760, XL min(1040, 90% W). Never wider than W-32 or taller than H-48. Title 36, footer 52, buttons at least 88 x 28.
- **Pages:** a settings-like tab is a centred column of cards, 760 at most (cards raised, radius 6, padding 12, gap 12); a panel is Sections separated by a hairline. Label columns are in em, never a share of the width.
- **Other:** tooltips and toasts up to 360; drawers 40% of H, clamped to 260-480.

**Minimum sizes (pt).** Viewport 360x240. Outliner 220x160. Details 280x200. Place 240x200. Content Browser 360x150. Output Log 320x110. Document side panel 220. Document canvas 400x260.

**Type.** One Segoe UI face covers Latin and Cyrillic.
- A font size is the line box, and the Segoe em is 0.75 of it, so Body 16 is a 12 px em (Windows 9 pt). No text is smaller than 14.
- Three files are read (segoeui, seguisb, CascadiaMono; `font.rs`) before the window opens; the startup profile times them as its `Loading fonts` stage. egui's own fonts are the fallback where Segoe UI is missing.
- Styles: **Body** segoeui 16 (the default); **Strong** seguisb 16 (headers, node titles, Primary labels); **Small** segoeui 14 (status bar, table headers, captions, hints, badges, keycaps, pins); **Title** seguisb 18 (dialog and document titles, empty states); **Page** seguisb 26 (the Start page); **Mono** CascadiaMono 15, falling back to consola (log, source, stats).

**Scale in one place.** Sizes are points and egui scales them by the context's pixels per point, the one place DPI enters: side panels keep their pt size and dialogs recentre when the scale changes. **Interface Scale** (Preferences > Interface; `[ui] scale`: Auto, 100, 125, 150, 175, 200 %) applies at once; Auto follows the monitor the window is on. `-uiscale <f>` and `ui.scale` win for a run and are never saved.

## 3. Iconography

Icons are vector only, in `gz-ui`'s `icons.rs` (geometry in `icons/geometry.rs`, drawing in `icons/paint.rs`):
- a 16-unit grid with a 1-unit margin, built once and then only scaled;
- stroke 1.5 units, at least 1 px, pixel-snapped at 20 px and below;
- round joins, outlines only; fills only for play/stop, dots, the pupil and on-states.

`icons::draw` draws one in a square, `icons::draw_in` centres one in a rectangle; `Icon::name` is its stable snake_case name (`chevron_down`), the one MCP answers and QA read. Sizes are 16 in toolbars, 14 in rows, 12 in the status bar, and 20-48 in empty states and tiles (a tile never draws its icon above 48).

How they draw: the geometry is built once, arcs tessellated then; at 20 px and below points snap to the pixel grid and a backtrack pass drops the spikes snapping would make; above 2 px of an opaque stroke, joins and caps are round; dashed paths are stretched to whole dashes. New glyph arcs start exactly where the previous line ends (within 0.02 units). Combo buttons, tree nodes, submenus and sorted columns draw the kit's chevron. `-iconsheet [first]` opens a developer sheet of every glyph at 16, 24 and 48 pt (`icons::sheet`).

Icon colours by state: rest text.secondary; hover text; checked accent.bright on selection.focused; disabled text.disabled; status its own tone.

Every major tab kind has an icon of its own, and so does every Level panel: a tab row or the Window menu never shows two alike (Mod Settings is a package, Preferences the gear). The content browser's own-asset icons (`gz-ui-content`'s `icons.rs`, the kinds in `gz-project`'s `asset_kind.rs`) draw through this engine; their names stay (QA).

`gz_ui::Icon`:
- **Chrome:** Minimize Maximize Restore Close ChevronUp/Down/Left/Right Caret More Overflow Menu Dot Grip Pin External SidebarLeft SidebarRight PanelBottom.
- **Actions:** New Open Save SaveAll Reload Undo Redo Cut Copy Paste Duplicate Rename Delete Plus Minus Clear Check Search Filter Sort SortAsc SortDesc Settings Eye EyeOff Lock Unlock Link Unlink Star StarFilled Bookmark BookmarkFilled BookmarkPrev BookmarkNext History Back Forward Up Home Reveal Info Help Keyboard ExpandAll CollapseAll MoveUp MoveDown Globe Key Shield Tag Picture Terminal Refresh.
- **Status:** Error Warning Success Required Spinner Dirty.
- **Build:** BuildCollision BuildAIMap BuildGameGraph BuildAll Play PlayGame PlayClean Stop ReloadConfigs GameFolder BuildMod Package Publish Releases.
- **Viewport:** Select Place Move Rotate Scale SnapGrid SnapAngle SnapScale SnapObject FrameAll FrameSelection Perspective Ortho Lit Unlit Wireframe Bones Stats ViewOptions.
- **Classes:** SceneObject Light SoundSource SoundEnv Glow Shape Spawn Way Sector Portal Group Particles DetailObjects AIMap Wallmark FogVolume.
- **Files:** Folder FolderOpen File FileTexture FileModel FileSound FileScript FileConfig FileXml FileText FileArchive Level, plus the 5 own kinds: Quest Behaviour Character CharacterTemplate PdaInterface.
- **Graph:** Flow Wait Condition Variable SpawnNpc Dialog Item PdaMsg World Relation Squad Order Timer Trigger End Task Role Minimap AutoLayout Validate FindInProject ZoomIn ZoomOut ZoomFit Zoom100.
- **Other:** Person Community Rank Money Kit Area Transition PlayerStart Mode Relations Exclude Aspect Language Tasks Mcp Log Problems Drawer.

## 4. UI kit

The kit is the crate `gz-ui`, on egui; its API is the crate's own (`cargo doc -p gz-ui`). `-kitsheet` opens a sheet of every widget below in every state, and the tokens (`kitsheet.rs`): it is how a surface stream sees what exists.

**What drawing code may use.** The kit, the theme's tokens and nothing else: colours through the context's `Palette` and its tones, sizes in pt through `metrics` and `units`. A widget the kit lacks starts as a `local_<name>` function in its surface crate's `local.rs` and moves into the kit once a second surface needs it (section 8).

**One widget per job.** Each job below has exactly this widget; a surface that keeps a copy of its own is a bug of that surface.

| Widget | Job | States |
|---|---|---|
| `Button` (`Kind` Primary, Secondary, Ghost, GhostDanger, Danger), `icon_button`, `segmented`, `keycaps` | a click that does something | rest, hover, pressed, off with its reason as the tip, a toggle that is on (a Ghost with a 2 pt accent.bright underline) |
| `Check` | one of several: list rows, multi-choice | off, on, mixed, disabled |
| `Switch` | a setting that applies at once | off, on, disabled |
| `Combo` | a value picked from a list, with the kit's chevron; opens a `PickList` | rest, open, mixed ("Multiple values"), invalid (a value the list lacks, kept and shown), disabled |
| `PickList` | the popup list: ranked search (`rank.rs`: whole-word prefixes, then letters in order), virtualized rows with an icon and a family tint, a second muted line on the highlighted row, sections Recent and Favourites with a star per row, full keyboard use (arrows, Page Up/Down, Ctrl+Home/End, Enter, Esc), measured once when it opens | rows on, off, current (a check), highlighted; "No matches", "Nothing to pick" |
| `Section` | the fold header of a part of a panel: chevron, Strong title, a count, a trailing + and menu; no fill; 28 pt; its open state kept in the layout store by id | open, closed |
| `ItemRows` | a small editable table: named columns, a cross per row, an Add row, drag to reorder | rows, empty with its Add row |
| `Table` | records: sortable, resizable columns, virtualized rows, selection, a context menu | sorted up/down, selected, hovered |
| `PropertyGrid` | label-value rows per role, sections, a help footer; values `text_field`, `number` / `integer`, `switch`, `combo`, `color`, `vec3_field`, `pick_field`, `path_field`, `chips`, `text_block`, `read_only` | required, invalid, modified (Reset), read-only, mixed |
| `SearchField`, `search::matches` | the one text filter | empty with its hint, typed, cleared by Esc |
| `Toolbar` | a row of a tab's or a panel's actions | labels, folded to icons, overflowed into `»` |
| `menu_item` / `MenuItem` | a menu row | on, off with its reason, checked, danger |
| `banner`, `banner_list` | a toned bar over a surface, with up to a short list and an action per item | info, warning, error |
| `toast`, `toast_with` | a note in the window, optionally with one button that runs an action | info, success, warning, error |
| `empty_state`, `badge`, `status_dot`, `spinner`, `busy_line` | states of a surface | |
| `card`, `page`, `page_card` + `card_footer` | a raised part of a page; a page filling the work area; a centred card with a dialog-like footer | a tone strip while it asks for attention |
| `Dialog`, `Footer`; `confirm`, `unsaved_prompt`, `notice`, `result` | modal work and the kit's questions | S, M, L, XL; a sheet of its parent; busy |
| `list`, `nav_row`, `table_headers` | rows with stable ids, a nav list | selected, hovered, a dot or a count |
| `splitter`, `splitter_tail`, `splitter_at` | a resizable edge; its value in the layout store | |
| `drop` (`Payload`, `Zone`) | typed drag payloads - an asset with its kind, files from Explorer - and drop zones any surface declares | a zone lights up under what it takes |
| `text::fit` | a cut text kept per (text, face, width), for what is drawn every frame | |
| `upload::Tiled` | a big picture that reaches the GPU a few tiles a frame, so no frame uploads megabytes | loading, whole |
| `audit` | what a frame cut short or let escape, for `ui.audit` | |
| `file_dialog` | the system's file dialog, for a person only | |

**Buttons**
- One Primary per surface: the kit asserts it in debug builds for a dialog footer and a toolbar.
- The Danger fill goes only on the destructive action. The button that opens the question ("Delete...", "Discard...", "Withdraw...") is GhostDanger: a Ghost with error-coloured text. A menu row that leads to a deletion is a `MenuItem` with `danger`.
- Secondary has a hairline. A toolbar toggle that only shows a panel is a Ghost with the underline while on - never an accent fill.
- Disabled: input fill, disabled text, and the reason (`why`) as the tooltip.
- A label that changes with the state passes every label it can show (`label_set`), so the button keeps the width of the widest.
- Row actions are `RowActions`: squares as tall as the row (22 pt) with 14 pt icons, shown on hover.

**Toolbar**
- Never wraps. A button shows its label while there is room; under pressure labels fold to icons first, then items move into `»`. Each `group(priority)` gives way at its priority, lowest first, rightmost first on a tie.
- It lays out from the last frame's measures. A folded label unfolds again only with 24 pt to spare, and a row keeps its layout while it still fits: a value that changes width never folds its neighbours. A value label keeps one width.
- A toolbar's `segmented` measures itself; in the overflow its choices become radio items under its caption. A `widget` has no overflow form and is hidden there.
- `split` is an action with a chevron menu (Build Mod); in the overflow it is the action plus "<label> Options". `menu` takes the name `ui.popup` opens it by; off, it keeps its caret and width, is drawn disabled with its reason as the tip and never opens.
- A toolbar that holds nothing of its own is never drawn empty: it shows its tab's status (`Document::toolbar_status`, "Saved to mod.ltx as you edit").

**Menus.** A popup reserves one icon column when any row has an icon or a check, and every label starts there; a check takes the icon's place. Shortcuts stand right-aligned in muted text and show what is bound NOW, never a literal (a surface's own row reads them with `gz_app::actions::keys_label`, or is drawn by `actions::menu_row`). A menu is 200 pt wide at least.

**SearchField.** A magnifier and a clear x. Esc clears; Ctrl+F focuses. Spaces separate terms that must all match, ignoring case (Cyrillic too, yo read as ye).

**Property grid**
- Label column: in em - each role starts at its own width, stored per role in `grid.<role>`, draggable within Em(6)..width-Em(8). Below Em(18) of room the label moves above the value. Labels are Title Case secondary text with an ellipsis; hints, messages and prose are sentences; the tooltip shows label, help and key. Each level indents 12. Categories are full-row headers that toggle on click; a level lists its own rows first, then its categories.
- Values start at the left of their field, numbers included. `Drag*` without steppers; a number shows its unit after it (m, °, fps) and typing takes the number. A value that needs more than its column on one line takes the line under its label. A switch stays on its label's line. Colours are a swatch plus hex. `PickField`: the leaf name with a start ellipsis, plus browse and clear. Multi-line text: the first line plus Edit (an M dialog), offered only by a text that holds several lines or is an ini text. A row keeps the height it had when last drawn. Read-only values are selectable secondary text; a path keeps its end. Differing values read "Multiple values".
- Flags: Required shows a dot. Invalid shows a bar and a message. Modified shows a Reset icon. A message is a sentence for the author: problem codes stay in MCP answers and the log.
- `help_footer` shows the hovered row's label and help, else the last one hovered while it is shown, else "Point at a property to see what it does."; one footer serves every grid of its panel.

**Lists and tables.** Rows are 22 pt with stable ids. `nav_row` is the row of a nav, a page list or a sources pane - icon, ellipsized label, a status dot or a count, one hover action. A one-column list has no header; otherwise `table_headers` / `Table`'s header: Small secondary text, centred over check columns. An empty list shows `empty_state`, which fits the height it gets (in a short region it drops the icon first, then the margins).

**Splitters.** A 1 pt line with a 6 pt hit area, re-clamped every frame. A double-click resets it. Its value is in the layout store (`layout.rs`, `[layout]` of `grizzly_editor.ltx`; `ui.split`). `splitter` sizes the pane before the line, `splitter_tail` the pane after it, `splitter_at` a pane laid out by hand.

**Cards and pages.** `card` is a raised box as tall as what it holds, with a tone strip while it asks for attention. `page` is the whole work area under the caption (the Start page). `page_card` / `card_footer` is a centred card with a dialog-like footer (status left, buttons right, Enter presses the default) on a page that is not modal.

**Toasts.** Up to four notes stand at the bottom centre of the work area, above everything, each with a cross; a note stays 6 s, an error or a note with a button 10 s. A note's one button (`toast_with`: "Details", "What's New") runs an action by its id and takes the note down. The Task card keeps its corner: toasts never share it.

**Dialogs**
- Modal, pinned, never placed from a saved layout. The id is `Title###dlg.<name>`.
- Centred on open and whenever the work area changes; clamped instead if the user moved it.
- Height is at most H-48. The body scrolls, the footer stays put right under it, and the window itself never scrolls. A running job never resizes it.
- A dialog is measured on its first two frames; a measuring frame whose height is about to change is not shown, so the first frame anyone sees - a person, a picture, `ui.audit` - is the final layout. Keys never answer a dialog on a frame nobody saw.
- Footer: status on the left; [Primary|Danger] [Cancel] on the right. While a job runs, `Footer::busy(stage)` puts the spinner and the stage on the left, and the job's stop is a labelled footer button, "Stop <job>". Label columns inside a dialog are in em.
- Enter presses the default and Esc cancels; in a destructive dialog both cancel. The keyboard focus opens on the default; the title bar's cross takes none.
- A dialog raised from a dialog is a sheet (`Dialog::sheet`): under its parent's title, no wider than it, the parent dimmed. A `confirm` raised inside a dialog opens as one.
- `confirm` lists up to 10 items, then "and N more", the counts, and "Deleted for good - this cannot be undone." in warning colour. `unsaved_prompt` offers [Save] [Don't Save] [Cancel]; the exit prompt lists the scene and every dirty document, with [Save All]. `result` is the one result dialog of build, export, package, publish and bake (`dlg.build_result`, `dlg.export_result`, `dlg.package_result`, `dlg.bake_result`), for runs started from the UI.
- The queued kit dialogs are drawn at the end of the frame (`draw_overlays`), above any modal open that frame; `then` runs outside every window.

**Who answers a question** (the caller's `Asker`)
1. `Mcp`: the caller has decided. `confirm` answers Yes and the action runs before the reply; losing data needs `save=` or `discard=`, otherwise the command answers `unsaved` with what is dirty; nothing opens, a note is logged. Under `-nodlg`, `app.quit` and `project.close` discard.
2. `Unattended` (`-nodlg`): the logged answers - a question answered No is the warning `[dlg suppressed] <question>  -> auto-answer: No`, a note is logged as information, an error as an error.
3. `Ui`: the dialog opens, and `then` runs after the frame that answered.

Errors and notes for the author become queued notices once the UI draws.

**Truncation and tooltips.** Names are cut at the end, paths at the start, "title - id" in the middle. Prose wraps at 36 em (`text::prose`). Numbers are never cut. Text painted by hand that was cut reports itself to the audit. Tooltips appear after 0.45 s, never in the background or during a held picture.

**Audit.** Kit widgets log `truncated`, `escaped` and `controls` (label, enabled, why, help). The audit also flags any window whose content exceeds its inner size on an axis that cannot scroll, measured from what this frame laid out, and skips a child whose root window is hidden.

## 5. Frame

Chrome totals 122 pt:
- **Caption (32):** logo, menus, a drag band of at least 48, the project name (dropped first), – ▢ ✕.
- **Tab row (30):** the major tabs and ⌄, from the left edge, in the chrome colour - a row of its own, so that no tab moves when the active one brings other menus into the caption (Level's Scene, Build and View, a graph tab's Graph). It is client area, not part of the drag band. The Start page has no tab row (nor a toolbar).
- **Toolbar (36):** the tab's `toolbar()` (or its status), then Spring, then the project's group on every tab - [Build Mod ▾ (Primary split)] [Publish Release] [Releases]. Releases is checked on the Releases tab and stays where it is. Publish Release keeps its label while a job runs: the state is in its tip.
- **Status bar (24).**

**Caption**
- It is drawn in every state (`gz-app`'s `chrome.rs`). The Start page, Link Game and Recovery show no menus.
- The caption registers the menus and window buttons as HTCLIENT each frame; the rest is HTCAPTION (`gz-win`'s `caption.rs`). The OS title reads "<project> - Grizzly Editor".
- Under 1100 pt the menus fold into a single Menu button (`ui.frame` `folded`; `ui.popup show=menu:Menu`).

**Menus**, in order. Every row is an action (section 5, Actions); a list that changes (Recent Scene, the open documents, the Level panels) draws its rows from what is there:
- **File:** New Scene (Ctrl+N), Open Scene... (Ctrl+O), Recent Scene ▸ | Save (Ctrl+S), Save As... (Ctrl+Shift+S) | Project ▸ (Import Base Scene..., Collect Library Assets..., Open Project Folder, Switch Project...) | Quit. New, Open and Save carry their icons.
- **Edit:** Undo, Redo, Cut, Copy, Paste, Duplicate, Delete, Find, Command Palette..., Preferences. These route to the active tab (its `Document::edit`); on Level they act on the selection, whatever kinds it holds (a light and a glow selected are deleted together), and Find puts the World Outliner's search up.
- **Level only:** Scene, Build, View (section 6, "Level toolbar and menus").
- **Graph only:** Graph (the graph tab's).
- **Mod:** Mod Settings, Characters, World Rules, Screens & Music, PDA Interface, Link Game... | Build Into Game, Build to Folder..., Export Flat Gamedata..., Bake Scene Layer ▸ | Package Release..., Publish Release..., Releases | Submit to Mod Browser....
- **Libraries:** Light Animations, Texture Library, Sound Library | Check New Textures, Synchronize Textures..., Synchronize Sounds... | Reload Objects, Reload Textures, Rewrite Texture Descriptions....
- **Window:** the Level panels, each with its own icon and a check while open; Content Drawer, Log Drawer; the open documents; Reset Layout.
- **Help:** Keyboard Shortcuts (Preferences on its Keyboard page), About Grizzly Editor | Check for Updates... (a check, then the Update dialog with what it found).

**Major tabs.** A custom strip, not docking, so tabs cannot be torn off.
- A tab shows an icon, the name without its own extension, and one slot: the dirty dot, or an x on hover and on the active tab. The slot takes room only when it holds something.
- The active tab uses the base fill with a 2 pt accent.bright overline.
- When the row is short, tool tabs (the singletons) fold to their icon first, the rightmost first, whichever tab is up - so a switch moves no tab; a folded tab names itself in its tip. Then tabs shrink to 96; the rest go under ⌄, which then reads "+N". Names that would still cut to the same text are cut in the middle.
- Middle-click closes and dragging reorders. Right-click: Close, Close Others, Close All, Close Saved, Copy Path, Show in Content Browser, Show in Explorer.
- Level is pinned, shows the scene name and a warning dot while dirty.

**Documents**
- Per file: `quest:<rel>`, `beh:<rel>`.
- Singletons that switch to a new target, asking first if dirty: `pda`, `model`, `texture`.
- Singletons: `characters`, `mod_settings`, `world_rules`, `screens`, `releases`, `preferences`, `light_animations`, `texture_library`, `sound_library`.
- Everything else stays a dialog: Package, Publish, Build to Folder, Export Flat, Submit, Import Base Scene, Update, pickers, confirms, results, the palette.

A major tab is a `Document` (`gz-app`'s `doc.rs`): `id`, `kind`, `label`, `icon`, `path`, `tooltip`, `closable`, `dirty`, `missing`, `body`, `status`, `toolbar`, `toolbar_status`, `menus`, `key_part`, `shortcut`, `edit`, `save`, `reload`, `discard`, `on_activate`, `on_deactivate`, `on_close`, `inactive`, and for MCP `mcp_state`, `panel_state`, `panel`, `as_any_mut`.
- Activation lands in the same frame. The active document draws in `###doc.host` at the work area; inactive documents are not drawn, their `inactive()` hook runs instead.
- Tabs persist per project in this machine's state of it (`<home>\projects\<project id>.tabs.ltx`, `docs/assets/PROJECT.md`) and load lazily: a restored tab is a placeholder (`loaded:false`) until it is activated. A tab whose file is gone shows "File not found"; MCP reports it as `missing`.
- Switch Project and `project.close` settle the dirty tabs first, write the session and close the project's tabs; they come back when it opens again.

**Level workspace**
- The Level toolbar sits over the Level dock; the viewport is its central node, without a tab bar.
- Panels (Place, World Outliner, Details, Level Settings, Content Browser, Output Log, Problems) have no ▼ button, show their close x on hover, and sit on a 26 pt strip. Raising a panel also selects its tab in its dock slot.
- When Level is inactive its dock keeps its state, panels are not drawn and the scene is not rendered; `shot.viewport` gets a pass of its own.
- The layout lives in the layout store (`level.*` keys of `grizzly_editor.ltx`).

| Class | When | Layout |
|---|---|---|
| Wide | W ≥ 1400 and H ≥ 640 | Place (the mode's panel) on the left, 260. Right column 340: Outliner (40%) above Details + Level Settings. Bottom strip 240, under the viewport only: Content Browser + Output Log + Problems |
| Compact | otherwise | Place 240, right column 300 (Problems a tab beside Details and Level Settings). Browser and log live only in the drawers |

- Side nodes keep their pt size; the centre absorbs every change. Crossing a size class rebuilds the layout unless the author customised it (`[ui] level_layout_custom`). Reset Layout rebuilds for the current size class.

**Status bar.** Drawn in chrome.
- **Left:** [Content Drawer], [Output Log] (none on the Start page: it has no project to browse) and the last line worth reading: a line of the levels that report (`gz-log`'s `Level::reports`: information, warnings, errors) - never a detail (profiles, stage times, render chatter), never a job's start, stage or finish line. The line is kept as a field the log updates on every push, never searched for per frame; a click on it opens the Output Log.
- **Centre:** the document's `status()` (Level: selection count, grid, angle). The Start page has none.
- **Right:** the area chips (the Build chip of the level, the update chip), Tasks, error and warning counts, the MCP dot.

- The counts are the Output Log's own Warnings / Errors: counted since the project opened, each distinct line once. A chip opens the log on its lines; a second click closes it.

**Drawers.** The Content (Ctrl+Space) and Log (Ctrl+\`) drawers cover the bottom of any tab, one at a time. A drawer shows the same object as its panel; one host per frame: on Level with the panel on screen, a drawer request raises the panel instead. Esc, a click outside, a drag that leaves the drawer, or another tab coming up closes it.

**Tasks.** The Task Center is a status chip and a card (`Tasks###tasks_card`, 400 pt) in the bottom right corner of the work area, over the work and in nobody's way: no dim, no focus taken.
- Each running job's row: the title, what it is about, its steps (the current one ringed), a 6 pt bar (filled as far as the job has come, or crossed by a light while nobody can tell how long it takes), the stage, the elapsed time and what is left, bytes and files, the file at hand, and [Stop <job>].
- Under them the last five jobs that ended, dimmed, each with how it ended ("Done in 00:12", "Stopped", "Failed: <why>").
- The chip shows a spinner and the job while one runs, and a history icon while only ended jobs are listed; a click shows or hides the card. The card a UI job opened goes a few seconds after the last job ended; one the author opened stays until they close it. MCP jobs never open it (`ui.tasks show=true` puts it up). A modal up hides it.
- **A job belongs to the editor or to the project that was open when it started** (`gz-task`'s `Owner`; `job.list` answers `owner`). Closing a project stops its jobs; the editor's own (an update) go on.
- **A job's progress draws at a rate of its own and wakes no scene.** The status chip redraws ten times a second, the open card twenty, a spinner thirty; the Level viewport builds a scene pass only when something it shows changed (`gz-ui-viewport`'s `service.rs`, `due`). `perf.frames` counts UI frames and scene passes apart.

**Floating windows.** Panels get windows of their own only with Preferences > General > Allow Panels Outside the Window (`[ui] floating_panels`, read at start); `outside` is empty without it. A modal is drawn in the main window. A minimized editor asked for a picture draws its frame as if restored (`gz-app`'s `window.rs`).

### Actions

An **action** (`gz-app`'s `actions.rs`) is what the author can trigger by name: a registry command with its arguments worked out from what is in front of the author, plus what the window shows of it - an id (`graph.frame_all`), a title, an icon, default keys, a group, a scope, whether it is on (and why not) and whether it is checked. An area adds its actions with `ShellBuilder::action`; menus (`menu_action`), toolbar buttons (`actions::toolbar_button`), keys and the palette name them by id. Running one is `Editor::call` with the window's origin, off the UI thread: a question the command raises still opens a dialog for a person, and a failure is a notice. The table is also an `Editor` service (`Actions::of`), so a surface draws its own menu rows and toolbar buttons from it (`actions::menu_row`, `keys_label`).
- **Scope** says where its keys are live: `Global`, `Tab(kinds)`, or `Part(kinds, part)` - while the document says that part of it has the keys (`Document::key_part`: the graph's canvas). Its menu and palette rows are there wherever it is on.
- **Groups** are the palette's categories and the Keyboard page's sections: File, Edit, Window, Help, Mod, Libraries, Graph, and the Level tab's Scene, Selection, Build and Viewport.
- The shell's own (`shell_actions.rs`): `file.save` (Ctrl+S), `project.open_folder`, `project.switch`, `app.quit`, `edit.undo` (Ctrl+Z), `edit.redo` (Ctrl+Y, Ctrl+Shift+Z), `edit.cut` / `copy` / `paste` / `duplicate` / `delete` / `find` (Ctrl+X, C, V, D, Delete, Ctrl+F), `edit.palette` (Ctrl+Shift+P), `app.preferences`, `job.stop_running`, `window.content_drawer` (Ctrl+Space), `window.log_drawer` (Ctrl+\`), `window.next_tab` / `previous_tab` (Ctrl+Tab, Ctrl+Shift+Tab), `window.close_tab` (Ctrl+F4), `window.reset_layout`, `help.keyboard`, `help.about`.
- The graph's (`gz-ui-graph`'s `actions.rs`, group Graph): each runs one of the graph's commands on the tab that is up - `graph.edit` or `graph.arrange` on the selection, `graph.view` for the view, `graph.find` for the search. Live on the canvas: Frame All (Home), Frame Selection (F), Add Node... (Tab, Space), Replace Node..., Rename Node (F2), Remove the Wire Under the Pointer (D), Select All Nodes (Ctrl+A), Select Connected (Ctrl+L) / Upstream / Downstream, Connect to the Nearest (Q), Bypass Node (Alt+D), Duplicate with Wires (Ctrl+Shift+D), Disable / Enable (Ctrl+Shift+E), Collapse / Expand (Ctrl+Shift+H), Bookmark the Node (Ctrl+B), Set Bookmark 1-9 (Ctrl+Shift+1..9), Add Comment Frame (C), Auto Layout Selection / Branch, Align Left (Shift+A) / Centre / Right (Shift+D) / Top (Shift+W) / Middle / Bottom (Shift+S), Distribute Horizontally / Vertically, Straighten Connections (Shift+Q). Live on the tab: Back (Alt+Left), Forward (Alt+Right), Go to Bookmark 1-9 (Ctrl+1..9), Previous / Next Bookmark, Find (Ctrl+F), Find Next (F3), Find Previous (Shift+F3), Find in Project (Ctrl+Shift+F), Show Problems, Auto Layout, Flow Left to Right / Top to Bottom, the wire styles (Wires as in Preferences, Curved, Straight, Circuit, Angled, Subway), Reload. A new one is a name `graph.<verb>`, its title, keys, scope and the command with its arguments.
- The Level tab's (`gz-ui-level`'s `actions.rs`, keys live on Level): New Scene (Ctrl+N), Open Scene... (Ctrl+O), Open Last Scene, Save As... (Ctrl+Shift+S); Select All (Ctrl+A), Deselect All, Invert Selection (Ctrl+I), Hide Unselected (Shift+H); Ungroup (Ctrl+Shift+G), Place Group..., Save Selection as Group..., Mark as Mod Content, Mark as Base Level, Summary Info, Create Recovery Snapshot, Maximize Viewport (F11); Build Level (F5), Rebuild All, one per step (Build Game Files, Build AI Map, ... - each `level.build only=<step> force=true`), Build at Draft / Final Quality, Validate Scene, Show Problems, Clear Problems, Export Scene / Selection as OBJ..., Compile a Level Folder...; Play (F9), Play Clean, Play Without Building, Stop, Reload Game Configs, Open Game Folder. They run `scene.*`, `level.*` and, for a window of the tab, `level.show`. A scene command that refuses for unsaved work (`code: unsaved`) is asked about with Save / Don't Save / Cancel and sent again with the answer - for a person; the command itself never asks.
- The viewport's (`gz-ui-viewport`'s `actions.rs`, group Viewport): the modes, the transform tools, the camera, Hide Selected (H), Show All Hidden (Ctrl+H), Group (Ctrl+G), the view modes, what is shown, the snapping, the camera bookmarks.

**Keys**, first taker wins (`gz-app`'s `shell.rs`):
1. A text field, and a row of the Keyboard page that is recording keys.
2. An action whose keys are live: one of this tab (or its part) before one live everywhere, and only one that is on. While a panel that answers its own keys has the focus (the Content Browser, the Output Log, the drawers), only the frame's own keys (Save, the palette, the tabs, the drawers) are taken.
3. The document's `shortcut()` (a gesture's Esc).

While a popup is up, as while a key-owning panel has the focus, only the frame's own keys are taken.

None of them acts while a modal is up. `app.keys action=resolve keys=<chord>` answers what a chord runs now, found the same way.

**Command Palette** (Ctrl+Shift+P; `Command Palette###grizzly_command_palette`)
- Top-centred, min(720, 70% W) wide; its height follows the results, up to 480, and the field never moves.
- Its rows: every action (on or off - an off row says why in its tip), the open tabs (their files by project path), the Level panels. Each row is an icon, the name, a muted category column (the action's group, Tab, Panel) at one x on every row, and the keys bound now, right-aligned.
- Ranking (`gz-ui`'s `rank.rs`, the pick list's): whole-word prefixes first, then letters in order that each start a word or follow the one before. Only what a row shows is matched - name, category, keys -, never its id. With an empty query the last eight actions run from it come first, marked recent (`palette.recent` in the layout store). The ranking is worked out when the query changes, not per frame.

**Preferences** (a singleton tab): search, a nav with an icon per page, and the page - a centred column of cards, 760 pt at most.
- Pages: General, Interface (Scale, Accent), Viewport, Tools, Objects, Recovery, the areas' pages (Game, Library), Keyboard last.
- An area adds a group of rows to a page with `ShellBuilder::pref_group` (`PrefGroup`: page, icon, section, rows, and a `note` - one read-only row instead of the rows while they mean nothing here). A row (`PrefRow`) is a `PrefKind`: a `Setting` of the settings file (edited as its type says; `app.prefs` lists and sets it), a `Switch` the area keeps itself, a `ReadOnly` value, or a `Button` with the sentence of what it last found. The editor's updates (Check at Startup, Installed Version, Check Now) and the project's Game, Library and Project Files rows are such groups.
- A change applies at once and is saved; the toolbar row says so.

**Keyboard page** (Preferences > Keyboard; `app.keys`): every action, grouped as the palette groups them; the rows virtualized. A row: the title, its keys as keycaps, and on hover [Reset] when it differs from its default. A click on the keycap cell records ("Press keys..."): the next chord binds, Esc cancels, Backspace clears; a cross on hover clears too. A binding another action of an overlapping scope had moves to this one, and the page names it. Reset All puts every default back. The id is the tooltip.

**The stall monitor** (`gz-app`'s `stalls.rs`). The window times every UI pass and every event it handles; one over 100 ms is a stall, kept with its milliseconds, when it happened, the tab that was up and what ran (`ui pass`, `event: resized`, `command: <function>` for a command's work on the UI thread), and logged as `[stall] 412 ms in <tab>: <what ran>` - a detail, or a counted warning with `GRIZZLY_DEBUG_STALLS=1`. The first frame of a start is no stall (it makes the font atlas and the first pipelines). `perf.stalls` answers the list; every suite may assert it is empty.

## 6. Surfaces

**World Outliner**
- Toolbar: SearchField (Ctrl+F focuses it; every word has to match, `-word` leaves those out, `"two words"` match together); Filters with a badge (Selected Only, Visibility All / Visible / Hidden, Sort Scene / A-Z / Z-A, the kinds shown, each a check with its count); ⋯ More (By Type / By Hierarchy, Show All, Show Selected, Hide Selected).
- Columns: eye, lock, then Name with the kind's icon. A row shows the leaf of its name (the part after the last `\`), cut in the middle; the tooltip has the whole name. Type and Owner are hidden, back from the header's right-click menu. The header does not sort: sorting is the Filters menu's.
- By Type: a row per kind, ordered by the kind's name ("Fog Volumes", "Glows", "Lights", ... "Way Paths"), with its eye, its lock and its count ("3 of 120" while filtering); its objects under it. By Hierarchy: groups with their members, way paths with their points (a point's row selects the point).
- The eye hides an object or a whole kind in the viewport; the lock keeps it drawn but out of picking and editing. A lock shows on a locked row, an open one only under the pointer.
- A selection made elsewhere (the viewport, a command) unfolds its kind or its owners and scrolls its row into view - on the first listing of the scene as it is then, never on an older one.
- The row menu acts on the selection (a right click on a row outside it selects the row first): Focus, Select All Made From the Same, Select All <Kind>, Hide / Show, Lock / Unlock, then what the selection's kinds can do - Group (two or more), Ungroup, Save Group As..., Center the Pivot, Pivot to an Object..., Reload From Its Asset; a way path's links (Link Selected Points One Way / Both Ways, Turn the Links Round, Remove the Selected Points' Links, Make Every Link One Way / Both Ways); Join Shapes, Split Into Parts...; Group / Ungroup Fog Volumes; Give the Shape to the Spawn Point, Detach Their Shapes...; Reset Surfaces - then Rename... and Delete... (in the error colour).
- Empty states: "The scene is empty" (drag something in from the Place panel or the Content Browser), "Nothing matches" [Clear Filters]. Footer: "2 objects · 1 selected".
- Rename is `dlg.rename_object`; Delete asks through `dlg.delete_object`, naming what goes.

**The left panel: Place, Paint, Zones**
- The panel left of the viewport shows what the mode in the viewport works with, and its tab says which: **Place** in Select and Place mode, **Paint** in Paint mode, **Zones** in Zones mode (`ui.panel id=place`, window `Place###LeftBar`).
- **Place**: a SearchField ("Search everything to place"), then one virtualized list of fold sections (the kit's `Section`, their open state kept): Recent (the last twelve placed; only once something was), Objects (the library's tree, each object with its picture), Lights (Light, Glow), Spawn Points (Player Start, Environment Modifier, Respawn Point, then the linked game's sections by the names the player reads - an item's `inv_name` -, grouped as the game groups them, the section muted at the row's end), Shapes and Volumes (Sphere Shape, Box Shape, AI Map Bounds, Fog Volume, Sound Environment), Sounds (Sound Source, then the game's sounds), Particles, Way Paths (Way Path), Groups. Long lists start folded, the short ones open. A search opens every section and folder that holds a match and leaves the rest out; "Nothing matches" [Clear Search].
- A row is dragged into the viewport (a ghost follows the pointer there; dropping places it) or clicked to arm it: Place mode then places it at each click in the viewport until Esc. The armed row is selected and a line under the list says "Placing <name> · Esc stops" with a cross; a click on the armed row stops placing too. A row's tooltip shows its picture, what it is and what a click does.
- What a row drags is the kit's payload `Payload::Row { surface: "place", key }`, the key `<kind>` or `<kind>:<reference>` (`object:props\box`, `spawn:<section>`, `shape:sphere`, `shape:ai_map_bounds`; `gz_ui_viewport::place::Placeable`). Arming puts Place mode up with the kind (`SceneCmd::Arm`) and hands the viewport the reference, the section or, for a shape, `sphere`, `box` or `ai_map_bounds`.
- **Paint**: a segmented control [AI Map | Foliage | Decals] picks the layer (`scene.mode mode=ai_map|details|wallmarks`), then its panel:
  - AI Map: how many nodes and how many are selected (counted off the UI thread); where it is generated - inside the level's AI Map Bounds volume, or over the whole level with [Place Bounds Volume]; Can Climb and Can Drop (m); [Generate] (Primary), Clear... (GhostDanger). More: the brush (a drag on the ground adds nodes, Ctrl + click selects them; Ignore Constraints, Auto Link, Brush Size), the links (Side Buttons Add / Remove / Invert, the side pad, Select Unlinked, Invert Selection, Smooth Selected, Lay Selected Level, Remove Links, Turn Links Round, Delete Selected...), Ignored Materials (+ on its header, a cross per row), Drawing (Smooth Height, Drawn Within, Draw Nodes).
  - Foliage: the meshes (+ on the header adds library objects; objects dropped from the Place panel or the Content Browser are added; a cross takes one out, after a question), a click picks the mesh the brush paints; the picked mesh's Density, Smallest, Largest and Sways in the Wind; the brush's Brush Size (m) and Strength (a drag paints, Shift + drag erases); [Fill the Ground], Clear.... More: the Colour Picture with Import Again, Colour Indices... (the L dialog `Colour Indices###dlg.colour_indices`: a block per colour with its swatch and its meshes, Add Colour, Clear All..., Import... / Export... of the `.dti` file), Objects per Square, Draw Foliage.
  - Decals: what a click lays - Texture, Width, Height, Turn; Delete Selected.... More: Shader, Alignment By Camera / By World Axis, Draw Decals, Select All, Invert Selection.
- **Zones**: a first line that says what the build does by itself (one cell for a level without any, the meshes no cell holds into the default cell, the portals the cells lack), then [New Cell From Selection] (Primary), [Compute Portals], [Validate]; Cells and Portals, a list each with an eye per row (a click selects the row's cell or portal); Remove Selected... (GhostDanger). More: what a click on a mesh does (Edit the Cell / New Cell / New Cells per Object, Add / Take Out, Pick by a Dragged Box), Take In What Is Inside, Turn Portals Round, Remove the Default Cell..., the Level Bound (Edit the Level Bound, Fit to the Level).
- Every panel posts the scene's commands and never changes the scene itself; what a command answers comes back as a note or an error. One danger style for what loses work (GhostDanger and the kit's confirmation naming what goes); a button that cannot act now is drawn off with its reason as the tip.
- `level.view` answers all of it - the mode, the panel, the Place panel's search, categories, rows (each with its `payload`) and the armed thing, the Paint and Zones panels, the Outliner's filter, kinds and rows - and sets what a person sets there: `place_search`, `place` (+ `ref`, `section`, `box`, `usage`; `none` stops placing), `category` + `open` (+ `folder`), `outliner_search`, `selected_only`, `visibility`, `sort`, `layout`, `kinds`, `colour_indices`. It answers the Outliner as drawn: with the panel on the screen it waits for the frames in which the Outliner lists the scene as it is and has brought a selection made elsewhere into view.

**Details and Level Settings** (`gz-ui-level`'s `details.rs`)
- Header: the selection's icon and name, and its kind ("Light", "3 objects - Light", "Several kinds - the rows they share"). Search: "Search 1 property" / "Search 30 properties", plus a menu with Expand All / Collapse All.
- The rows are `gz-scene`'s (`scene.get` answers the same): plain labels in Title Case, a unit after every number (m, °, s), words for choices, a help line with the range; engine names never show.
- At the top, the rows of no category (Name). Then one kit `Section` per category, in the order the rows come - Transform first - each with its row count; its fold is kept in the layout store (`section.level.details.<category>`). Inside, a sub-category folds as the grid's own header, and the rows most authors never touch (the flag words, the editor's own switches) wait under the section's folded **More**.
- A position, a rotation (degrees) or a scale is one line of three fields, X Y Z, on the line under its label; where labels stand above values (a grid under 18 em) it is that line anyway.
- A value that differs from its default carries a Reset arrow at its row's end; Reset puts the default back as one undo step.
- With several objects selected, Details shows the rows they all have; a value that differs reads "Multiple values". Nothing in common says so ("Nothing shared").
- A value from a list is a pick field (the leaf name, browse, clear) that opens the chooser; a value of a few words is a Combo.
- An edit is sent when the drag or the typing ends; a transform's drag is merged into one undo step.
- Level Settings holds only what is the level's: its folder and name prefix, the build's quality and settings, the sun, the scene's light controls. View toggles are the viewport's Show pill, tool parameters their mode's panel, a kind's eye and lock the Outliner's.
- Grids: role `selection` or `world`, with a help footer.

**Chooser** (`Select <what>###dlg.choose`, L; `gz-ui-level`'s `choose.rs`)
- Most of the window's height (72%, 280 to 760 pt of rows): a search over the list and its rows, the highlighted item's preview on the right (240 pt) for models, library objects, textures and sounds.
- The search ranks like the palette (whole-word prefixes of the leaf name first, then letters in order); the order is worked out on the CPU pool when the search changes, and only the rows in view are laid out. Up / Down / Page Up / Page Down move the highlight (and, choosing one item, the pick); Enter selects, Esc cancels; a double-click picks.
- A row of a list with pictures is 46 pt: a 40 pt picture (the model drawn by the chooser's own renderer from three quarters above, the texture at the size it is shown), the leaf name and, muted under it, its folder. Other rows are one line: the leaf, the folder muted after it. The current value carries a check.
- The preview: the picture at 240 pt, the name and the folder; a sound has Play and Stop.
- Several items (Fixed Bones, a respawn point's profiles): a click adds or takes out, the picked list stands under the rows.
- Footer: "Now: <current>" | [Select] [None] [Cancel].
- Pictures are made on the pools and taken into the GPU three a frame, 256 at most kept; the scene's own lists (way paths, story ids) are read off the UI thread too.
- `scene.choose` puts it up on a property (`key=`, `object=`, `world=`), searches (`query=`), highlights (`highlight=`) or closes it, and answers what it shows; `scene.choices` answers the list itself.

**Level toolbar and menus** (`gz-ui-level`'s `toolbar.rs`, `menus.rs`, `status.rs`, `problems.rs`)
- Toolbar: [Save] [Build Level ▾] [Play ▾] [Stop] [More ▾], labelled (Ghost; the project group's Build Mod is the one Primary). Build Level's click builds what is stale (`level.build`); its menu lists every step with its state on the right - Fresh, Stale, Never Built, Failed, Not Needed - and why in the row's tip, a click building that step and what it needs; then Rebuild All and Advanced ▸. Play's click builds what is stale and starts the game; its menu holds Play Clean and Play Without Building. Stop is on while a build, the compiler or the game runs. More: Reload Game Configs, Open Game Folder | Validate Scene, Show Problems.
- **Scene:** Select All, Deselect All, Invert Selection | Hide Selected, Hide Unselected, Show All Hidden | Group, Ungroup, Save Selection as Group... | Mark as Mod Content, Mark as Base Level | Summary Info.
- **Build:** Build Level, Build Step ▸ (the steps with their states), Rebuild All | Play, Play Clean, Play Without Building, Stop | Validate Scene, Show Problems, Clear Problems | Advanced ▸ (Build at Draft Quality, Build at Final Quality | the mod's level layers: Spawn Layer..., Collision Overlay..., Visual Overlay..., Level Cut... | Export Scene as OBJ..., Export Selection as OBJ..., Compile a Level Folder... | Reload Game Configs, Open Game Folder).
- **View:** the viewport's (`gz_ui_viewport::view_menu`) | Maximize Viewport (F11).
- The status bar's **Build chip** names the steps a build would run ("Build: Game Files · AI Map") in the warning tone; a click opens the same popover as Build Level's menu (the steps, Build Level, Rebuild All). It reads the level build's status query - keyed on the scene's revision and the build's record, never polled - so it is right after a restart.
- **Problems** (`Problems###level.problems`, beside the Output Log; Compact: beside Details): what the level's last builds found, a `Table` of the sentence with its severity icon, the object and the step. A click selects the object and frames it, or frames the place of a problem that has only one. The list is read when the panel comes up, when a build ends and when Clear Problems answered - never per frame. Nothing found: "No problems".

**Content Browser** (`gz-ui-content`; `content.view`)
- Toolbar: [+ Add ▾] (off, saying why, outside the project) | Back Forward Up | breadcrumb, whose leading parts fold into "…" | SearchField | View ▾ | ☆ | Favorites ▾.
- Add: New Folder | Quest, Behaviour Graph, Character, PDA Interface; the grid's empty-space menu has it too.
- Two sources. **Project** is the project's files by its own layout. **Library** is what can be brought into the project or placed, the linked game and the editor's library (Preferences > Library) merged, by kind: Objects, Groups, Textures, Sounds, Particles, Spawn Sections, Base Scenes. Its top lists the kinds as folders; the breadcrumb reads Library > Objects > props. Textures and sounds are the game's first, then what only the library has; Spawn Sections are the game's sections that declare `$spawn`, in the folders of their `$spawn` path (as the Place panel groups them), an item by its in-game name (a name several sections share adds the section), the tooltip naming the section and its class. A kind that could not be read says so in a banner over the grid ([Link Game...] when no game is linked).
- **One tile per asset.** What belongs to an asset is never a tile: a `.thm`, a backup (`.~object`, `name~`, `.bak`, `.old`), an object's `.skl` / `.skls` / `.bones`, a scene's folder (`escape\` of `escape.scene`, the part folder of an X-Ray SDK `.level`), and the project file. They travel with their asset: copied, moved, renamed and deleted with it. A folder that holds only such files is empty. An object's and a group's picture is its `.thm` thumbnail (rendered when it has none); an SDK `.level` says in its tooltip that it is imported into a `.scene` when it opens.
- A Library object or group that a scene of the project uses and the project lacks (the project file's `library` list) has a dot in the warning tone; its tooltip says Bring into Project copies it in.
- Sources pane (220, hidden under 480): Segmented [Project | Library], Favorites, tree. The project's tree has the project's name as its root row; the Library's has its kinds, the open one with its folders. When it cannot hold six tree rows under the section headers (`Section`), the headers go and the favorites (if any) sit above the tree. Its width is the Layout key `content_browser.sources`.
- View As: Tiles or List, per host. Tiles are 64-192 pt with vector icons, and **under 64 pt the folder is the list** (a 48 pt tile cut a folder's name mid-word). Until the author picks a size, a host fits its tiles: the largest size up to its default (panel 64, drawer 96), in 8 pt steps, at which the grid shows two rows - or every row when the folder needs fewer (seven folders in a low drawer keep 96). View > Tile Size > Fit to Height turns fitting back on; the size field is as wide as its value, and the View popup keeps the width of its longest row.
  - The list is one 22 pt line per entry (an 18 pt picture or a 16 pt glyph, the name cut in the middle with its extension kept) in columns of at least 12 em. The docked panel starts as a list; the drawer starts as tiles. Ctrl+wheel under 64 goes to the list.
  - There is no text in the square and no ".." tile. Folders and files without a picture draw a vector glyph (`glyphs` over MCP).
  - A picture with transparency lies on a checkerboard and is made for the size it is drawn at (the file's own mip, then an alpha-weighted average); the tooltip names the file's size and its transparency. Pictures reach the GPU within a budget a frame (a megapixel), the rest on the next frames.
  - An entry of a Library kind goes by its name alone, like the editor's own assets: the kind lists one extension and the breadcrumb names it.
  - Captions take 2 lines and never break right before the extension; a last line that does not fit keeps the extension and loses the middle of the name (reported as `truncated`).
  - Tiles and the marquee share one selection look.
  - The rename box is up to 24 em, with a 512-byte buffer.
- An entry's menu: Open | Place in Level (objects, groups, sounds, particles, spawn sections: the viewport's own drop at its centre - the surface under it, the snapping that is on, one undo step; `scene.place` on the surface ten metres along the camera's view while the Level tab is not up) | Bring into Project (the Library: `file.import`, an object with its textures and thumbnail; a base scene into `rawdata\levels`) | Copy, Cut, Paste, Rename, Delete... | Exclude from Build / Include in Build | Copy Path, Show in Explorer.
- **Drag payloads** (the kit's `drop`, `gz_ui_content::payload_of`): a tile drags `Payload::Assets`, every entry `{path, kind, file}`. `kind` is the `scene.place` kind for what is placed - `object`, `group`, `sound`, `particles`, `spawn` - else `texture`, `model`, `scene`, `folder`, one of the editor's own (`quest`, `behaviour`, `character`, `character_template`, `pda`) or `file`. `path`: an object or a group by its file with its extension (below the library's `rawdata\objects` / `rawdata\groups`, or project-relative), a sound or a particle effect by its reference, a spawn by its section, a texture by its name below `textures`, anything else by its path in its source; `file` where it is on disk. The viewport places what it can; a field takes the kind it holds; a project folder of the browser (a tile, a tree row, the grid) moves the project's own and copies the Library's in. Files from Explorer dropped there are copied in (`file.copy`); a picture (PNG, JPG, TGA, BMP) dropped into a folder under `gamedata\textures` becomes a `.dds` there.
- A status strip counts the folder shown and the selected entries (a folder picked in the tree is not one); the Library says Read only.
- MCP: `content.view action=get|go|back|forward|up|home|refresh|reveal|select|toggle|range|clear|rename|favorite_add|favorite_remove|favorite_open` (`source`, `kind`, `folder`, `path`, `open`, `query`, `view`, `tile`, `tile_fit`, `sources_pane`, `index`, `limit`, `offset`). It answers where the browser is and how its host shows it, the folder's `entries` (each `path`, `name`, `glyph`, `icon`, `picture`, `wanted`, `excluded`, its `payload`, and for the Library what places it - `place {kind, ref|section}` - and what brings it in - `import {from, path}` for `file.import`), the selection with its `icons` and `glyphs`, and the rename box.

**Viewport**
- The viewport runs edge to edge; what stands over it is one overlay toolbar of pills (`gz-ui-viewport`'s `pane.rs`), and a narrow viewport folds its right side into `»` first. Every row and button of it is an action of the group Viewport (`actions.rs`) with the keys bound to it now; the steps' and the weathers' lists set `viewport.view`.
  - Left: the **mode control** [Select | Place | Paint | Zones] (keys 1-4, `scene.mode`); [Perspective ▾] (Perspective, Top, Bottom, Front, Back, Left, Right, Orthographic, Frame Selection F, Frame All Home, Camera Bookmarks ▸ - go Ctrl+1..9, set Ctrl+Shift+1..9 -, Maximize Viewport F11); [View Mode ▾] (Studio Alt+4, Unlit Alt+3, Game Lit Alt+5, Wireframe Alt+2; while Game Lit is up its Weather ▸, Hour ▸, Exposure ▸, Indoor Sky Light ▸, Rebake Roof Map and a line "N lights, <weather> at hh:mm"); [Show ▾] (Grid, Safe Frame, Stats | Spawn Shapes, Spawn Names, Light Names, Glow Crosses, AI Map | Fog) - what is shown has this one place.
  - Right: Select Q, Move W, Rotate E, Scale R; World / Local (`); Surface Snap with its chevron (Align to Surface, Snap to Vertices); Grid Snap [0.1 m ▾], Angle Snap [15° ▾], Scale Snap [x0.25 ▾].
- Steps: move 0.01 0.05 0.1 0.25 0.5 1 2 5 10 m; turn 5 10 15 22.5 30 45 90°; scale x0.05 0.1 0.25 0.5 1 2. They are Preferences > Tools > Snap's (`snap_grid_step`, `snap_angle_step`, `snap_scale_step`).
- Defaults: Studio; Grid, Spawn Shapes and AI Map shown; Grid Snap and Angle Snap on, Scale Snap off; Surface Snap on, Align to Surface and Snap to Vertices off; World. The view mode, what is shown, the snapping and the space are the editor's settings (`[preferences] viewport_*`, read and set by `app.prefs` too).
- **View modes.** Studio: a light along the view, a key light from above and an ambient, so every surface reads. Unlit: the textures as they are. Game Lit: the scene pass lit per pixel the way the game lights the level, at the weather and hour of the linked game (a new game's first night by default; the weather is read off the UI thread the first time it comes up), the background the weather's fog colour. Wireframe: every mesh's edges. How the viewport renders - Render Scale, Real Time -, the graphics adapter, the GPU preference and Mute Sounds are Preferences > Viewport.
- A 64 pt clickable axis gizmo sits at the top right; it hides under 480. It is the only orientation widget: nothing is drawn into the render target for it.
- **Labels** (spawn and light names, a selected way's points and links, fog and AI map marks) are the nearest 256 of what is in view: more than that is a screen nobody reads, and every label is text the UI pass lays out.
- Stats are off by default and appear as a Mono pill; nothing is printed into the render target (the Build chip says what needs building).
- The render target grows in 64 px buckets (recreated only when the view outgrows it or shrinks two buckets below) and the view is its top-left corner; the render size stays exact. Whatever reads the target back crops to the render size (`shot.viewport`). The render thread never waits for the GPU on a frame: a frame is published as it is submitted (the window samples it later on the same queue), the next one waits until the GPU has finished the last by polling, and the labels' occlusion answers come back a frame or so late.

**The viewport's mouse**
- The camera: a right drag flies (W A S D Q E while it is held; the wheel sets the speed meanwhile), a middle drag pans, Alt + left drag orbits, the wheel dollies. The cursor is hidden and held where the press was, so the camera turns without end, and comes back there. A left drag no mode takes flies the camera and selects nothing. Shift and Ctrl belong to the tools: with one held a left press is the mode's (the marquee, an erasing stroke, a decal's move), never the camera's, and a right or middle drag is the same drag as without it.
- A left click acts when the button comes up, under 4 pt of movement; past it the press is a drag.
- What a left press does is the mode's (`gz_scene::SceneMode`, the scene's view state; `scene.mode`). A mode owns the clicks while it is on and never filters what Select can select. Esc leaves Place, Paint and Zones for Select.
  - **Select:** a click selects any visible, unlocked object (Ctrl toggles it); a member of a group that is not editable by itself picks its group; the point of a way path under the click is selected with its way, so the gizmo moves that point. Ctrl + drag is the marquee over every kind, added to the selection. A drag of a gizmo handle transforms the selection; Alt + drag moves copies, made first as an undo step of their own.
  - **Place:** each click places the armed thing (the Place panel arms it; `viewport.view arm=`) on the surface under the pointer, selected, until Esc. With a way path armed and one selected a click adds a point to it; Attach on Click attaches what is clicked.
  - **Paint:** the layer the Paint panel picks. AI Map: a drag paints nodes with the map's brush; Foliage: a drag paints the brush's mesh at its radius and density (Shift erases), as one `scene.details` call when the button comes up; Decals: a click lays a wallmark, Shift + click moves the selected one there. Ctrl + click and Ctrl + drag pick the layer's own nodes, slots or marks. A ring on the ground under the pointer is the brush, in the error colour while it erases. The gizmo moves and turns the AI map's selected nodes.
  - **Zones:** the Zones panel's switches make a cell from the mesh clicked, paint meshes into the selected cell or box them; otherwise a click selects.

**The gizmo**
- Move: an arrow per axis, a square per plane between two arrows (XY, XZ, YZ), and a square at the centre that moves in the view's plane. Rotate: a ring per axis and an outer ring about the view; a turn under way draws its arc on the ring and the angle beside the centre. Scale: a line with a cube per axis, the plane squares, and the centre for every axis at once.
- Handles are 3 pt in the axes' colours; the one under the pointer, or held, is drawn in the text colour at 4 pt. They are hit-tested on the screen as they are drawn - 6 pt round a line, inside a square (`gizmo.rs`, `hit`).
- World / Local: the handles follow the world's axes or the first selected object's. Scale always works along each object's own axes; the AI map's nodes take Y of Move and X and Z of Rotate.
- What it offers follows the selection: a tool some selected object cannot take is off, its reason the tip. A drag is one undo step. The grid step snaps a move (World: the moved world axes onto the grid; Local: whole steps along the moved axes), the angle step the turn, the scale step each scaled axis.
- It is drawn a fixed distance along the ray through the pivot, so it keeps one size on the screen. The pivot is one object's, the middle of several, or the selected points of a way path.

**Dropping into the viewport**
- The viewport is a drop zone of the kit (`gz_ui::drop`). It takes a row of the Place panel - `Payload::Row { surface: "place", key }`, the key `<kind>` or `<kind>:<reference>` (`object:props\box_wood_01`, `spawn:stalker`, `light`, `shape:box`; `gz_ui_viewport::place::Placeable` writes and reads it) - and the Content Browser's entries: an `.object`, a `.group`, a particle effect (`.pe`, `.pg`), a sound (`.ogg`, `.wav`), a spawn section (kind `spawn_section`).
- While one is over the viewport a ghost stands where it would land - an object's model outlined, a box and a cross for any kind - on the surface under the pointer with the snapping that is on, and "Place <name>" by the pointer. Let go, each thing is placed with `scene.place` as one undo step and selected.
- MCP: `viewport.camera action=get|set|focus|frame|bookmark` (`position`, `rotation` in degrees, `view`, `orthographic`, `object`, `scope`, `slot`, `store`); `viewport.view` (`mode`, `tool`, `space`, `show` / `hide` lists, the snaps and steps, Game Lit's `weather` / `hour` / `exposure` / `indoor_hemi` / `rebake`, `weathers`, `lights`, the brush, `arm`, `real_time`, `mute_sounds`, `gpu`; it answers all of it with `supports`); `viewport.play what=particles|animation`; `viewport.gesture kind=click|double_click|drag|hover|drop` (`payload` or `assets` for a drop) - played through the pane's own input, answering what was picked, placed, moved (a way's points with them), removed, whether the camera moved, and where the gizmo's handles are (`gizmo`: pivot, x, y, z, xy, xz, yz, screen); `shot.viewport`; `perf.frames`.

**Output Log**
- Docked in the Wide layout; a drawer otherwise.
- Toolbar: the kit's SearchField, All / Warnings / Errors with counts (the status bar's counts), Auto-scroll, Copy, Clear.
- Mono lines, virtualized and whole: the view holds whole rows, and a scroll that comes to rest between two settles on the nearer, so the first row is never cut in half. A muted row with a rule marks where the count starts ("The project opened - warnings and errors are counted from here"). The Warnings / Errors filter lists each distinct line once. The view opens at its end whenever it comes up (panel tab picked, drawer opened).
- A line's level is its colour, and its word - error, warning, info - is a word the search finds it by (`warning texture`). A message's level is a value (`gz_base::Finding`, a log record's level) and no text carries a mark for it: the level compiler's own lines have theirs read once, where they enter the build (`gz-levelbuild`'s `compile.rs`), and only the log file writes one in front of a line (`! `, `~ `, `* `). The status line is the sentence alone.
**Command Palette:** section 5.

**Quest and Behaviour graph**
- Toolbar: Save, Undo, Redo, the problems count (only when there are any), Frame All, Find | Spring | Graph panel, Details panel, Minimap (quiet toggles: a 2 pt accent line under a toggle that is on, never a fill), Zoom ▾ (its label the percentage, one width at every level) | More ▾: Reload, Auto Layout, Flow Direction ▸, Wires ▸, Find in Project, Back, Forward, Bookmarks ▸ (the named list, Bookmark the Node, Previous, Next), Keyboard Shortcuts. No Primary.
- Graph menu: Save, Reload | Add Node..., Show Problems | Flow Direction ▸ (Left to Right, Top to Bottom - lays the graph out again, one undo step), Wires ▸ (As in Preferences first, then the five styles, then Colour by Pin, Dim Unrelated, Flow Bubbles, Arrowheads: the graph's own over Preferences) | Auto Layout, Auto Layout Selection, Auto Layout Branch, Align ▸, Select ▸, Node ▸ | Frame All, Frame Selection, Find, Find Next, Find Previous, Find in Project | Bookmarks ▸ | the panels and the Shortcuts card as toggles. Every row of the group Graph is its action, with its keys as bound now.
- Graph panel, 280: ONE scroll - the header (Title, Activation; the ID under More), Tasks or Roles, Variables and **Global**, each a kit `Section` with a `+` on its header and its count. A variable's form is titled by its name (Name, Type, Default); Global is the module's variables (mod.ltx [vars]) in the same list-and-form, written into the manifest once an edit is let go of. The panel becomes a rail when the canvas would drop below 400; on a rail the toolbar's button opens it over the canvas.
- Details, 360: the Node section - its actions are list rows (icon, name, `On enter N` at the right, the whole row one target; a newly selected one is scrolled into view) - then the selected action under a splitter (`action_split`). Under 1400 pt of the tab's width it folds to a rail on the right whose button opens it over the canvas's right edge. Kind descriptions are tooltips and the help footer.
- The sizes are Layout keys: `graph.panel_w`, `graph.details_w`, `graph.drawer_h`, `graph.action_split`, and the grids `grid.graph_*`; a section's fold is `section.graph.<name>`.
- Bottom drawer: Problems (n) - each a sentence, the node it is on first, the code in its tooltip; a click frames the node - and Find Results.
- An external change is taken in without a question (docs/ASSETS.md section 4): a clean graph reads the file again, unsaved edits rebase onto it. A Banner says only what is left: the file does not read (Error, with the reader's line; the last good state stays), the file is gone (Save writes it again), or "Changed on disk too: <n> values" with one row and [Take Theirs] per value both sides changed. The Characters tab lists the same rows under its header; the PDA tab's Details pane shows its note with [Take Theirs].
- Canvas:
  - the scale is the zoom level times the UI scale, applied once; 21 levels from 5% to 200% (`gz_nq::doc::ZOOM`, level 16 = 1:1);
  - a graph flows left to right (inputs on a node's left, outputs down its right edge) or top to bottom (`canvas.flow`; a new graph takes Preferences > Graph > Flow);
  - text: title 16, lines and actions 15 pt line boxes, rounded to whole px per level, never drawn under 11 px. Semantic zoom: where a body line would be under 11 px a node shows its title alone at a fixed 13 px over its whole box; a node under 48 px wide is a block of its family's colour, its title on hover (`graph.view` answers `detail` and `title_px`);
  - Frame All fits the whole extent (nodes, routed wires, frames) at whatever level that takes, up to 1:1;
  - wires: one polyline per wire in its style - Curved, Straight, Circuit (rounded right angles, lanes of their own), Angled (45-degree diagonals; at the start, middle or end), Subway (a shared trunk, parallel tracks); thickness 1-4 px, never under 1 px; colour by pin role (`wire.base` for the way on, the danger hue for failures, the families' hues for the other ways out; `wire.hot` under the pointer, `wire.selected` touching the selection); Dim Unrelated (15%); Flow Bubbles (off by default, frames only while they move on screen); arrowheads at the input's rim; back edges and the wires of a disabled node dashed; a pin that is not the way on named on its wire's first stretch;
  - comment frames (`frame.fill`, a title strip in a family tone, a resize corner) under the nodes; reroutes as dots on their wire; alignment guides while dragging; the cut line of Ctrl+right drag in the danger hue.
- Node, 180-360 wide by its content:
  - a 28 pt header: the family icon in its `graph.family` colour, the title, the muted id, badges (the problems' count, a bookmark, `once`, `off`);
  - the author's comment as an italic line;
  - up to 3 "key: value" lines;
  - up to 4 action rows - the action's title and its catalog `summary` in words - then "+N more";
  - a red top line with errors, amber with warnings, the first problem's sentence on the badge's hover; the `shadow.card` shadow; pins in their role's colour, a hollow ring when unwired, each with a tooltip of what it means;
  - collapsed: the header and the pins; disabled: 40%, a hatched header.
- `gz-nq`'s `layout.rs` (`node_box`) computes the box for the layout, drawing, hit tests, minimap and `graph.get`.
- The picker is the kit's `PickList`: ranked search, the kind's icon and family colour, its description on the highlighted row, Recent (per project) and Favourites (a star per row) on top. Tab or Space opens it at the cursor; the empty canvas's menu is Paste, Add Comment Frame, Frame All, Auto Layout over the same list; a wire dropped on empty space opens it filtered by its pin.
- The minimap never takes more than a fifth of the canvas's height and shows from 12 nodes; without it a pill at the middle of each canvas edge counts the nodes wholly beyond it ("2 more") and a click brings the nearest in. Behaviour graphs never say "quest" (`gz-nq`'s `text.rs`).
- Every menu and the picker can be put up to look at: `graph.view menu=node|wire|canvas|frame|picker|replace` (a picker from a pin: `from_pin=node.pin`, `into=node`).

**PDA Interface**
- Toolbar: Save, Revert, Reload | Aspect [4:3 | 16:9], Language [rus | eng], Zoom [Fit | 100%] - three groups, each a muted caption before its segmented choice, a group gap between them | Reset State | Source, Details.
- The canvas is centred; it pans (right or middle drag) and zooms at the cursor (Ctrl+wheel always; over a PDA list or the map the wheel is the game's), F fits, and Fit fits both axes with 16 pt round the device. Every side of the device that lies on the game's screen edge gets that edge drawn along the device, never past it. The game's sheets (the device, the icons, the fonts, the map) reach the GPU in tiles of 512, a megapixel a frame (the kit's `upload::Tiled`), and are drawn tile by tile.
- Details (320) has the file with Use in Module, then tabs: Problems first, Pages (with the keys), Vars, Log.
- Source is a bottom split: Mono, gutter, Lua data in four colours of the palette (keys accent.bright, strings success, numbers warning, comments muted; above 256 KB one colour), lines unwrapped (the pane scrolls sideways), the reader's error line tinted and underlined with a squiggle; at the pane's foot the status (the reader's sentence, Unsaved edits, Saved), Line n when the reader stopped, and Apply (Ctrl+Enter). A text over 16 KB is laid out 8 KB a frame before the field shows it ("Laying out the text: n%"); the galley cache keeps every line apart, so an edit lays out its own line again. Only the line numbers on screen are laid out.
- Unsaved text asks before Revert (naming the file), close or retarget (Save / Don't Save / Cancel). `pda.preview` answers in canvas units; its `revert` and a retarget lose edits only with `discard`.
**Model Preview and Texture Preview** (`gz-ui-previews`; `preview.model`, `preview.texture`, `shot.asset`)
- Model: Frame, Reset View | Wireframe, Bones | Segmented Studio/Unlit | Details (320) | Place in Level, Show in Content Browser. Studio is a headlight along the view plus a key from above over an ambient. The view is drawn opaque (the target's alpha is a texture's gloss mask, not coverage). The model stands on a floor grid, and a visual only the linked game holds is drawn with the game's textures (`textures[].from`). Details say what it is in Title Case (Static Mesh, Rigid Skeleton, Library Object) and where each texture comes from (Project, Library, Game, Not found). Place in Level is on for a library object; Show in Content Browser selects it in the Project or the Library.
- Texture: Fit, 1:1, Zoom ▾ | Segmented RGBA/R/G/B/A | Mip ▾ | Details | Show in Content Browser. It pans and zooms at the cursor, and the checker is clipped to the view. Details name where the texture lives (`origin`: Project, Library, Game folder, Game archive), its container and format, and whether it is shown decoded.
- The picture's pass (a WGSL pipeline) is made on a thread of its own as the tab opens, while the picture loads: no frame compiles a pipeline.
- `preview.model name= source=library|project|game lighting= wireframe= bones= details= frame= reset=`, `preview.texture action=open|state|close path= source=project|library|game zoom= channel= mip= details=`: each answers the frame that shows what it asked for. `shot.asset name= kind=objects|groups|textures|raw_textures|visuals [source=game]` is a picture of an asset: its thumbnail, or the model rendered.
**Characters**
- Toolbar: Save (the tab's one Primary while the form holds edits, Ghost otherwise), Revert, Delete... (GhostDanger, asks through `dlg.delete_character`) | Reload.
- Three columns. The list (260, a splitter): search, + on its header, a row per character with its portrait (a 22 pt face; the kind's glyph while there is none), id and name, Template badges and the dirty dot. The form, up to 760: Identity, Name & Bio (a row per language), Appearance, Profile, Dialogs, a Starting Kit table, and Trade. The preview, in the room the form leaves (300 pt at least, else it is not drawn): the character's model in the model viewer (`gz-ui-previews`' model view, its own orbit), then "In the Game" - the portrait at 96 pt, the name and the community as the player reads them; values the character leaves to its base are the base's.
- Starting Kit is the kit's `ItemRows` (Item | Count | cross, Add Item, drag to reorder): the base's rows first, muted, "from <base>" after the item, their cross takes one out of this character (a count of 0); then the character's own rows.
- Trade (badge "Trader" while the character keeps a shop): not a trader - a line saying so or naming the base's shop, Make a Trader (Change Trade over a base's), Not a Trader; a trader - Sells and Buys (a combo of ticks over the linked game's trade categories, a category it does not list stays ticked and says so), Markup, Pays, Restock, then a Stock table (Item | Count | Chance | remove), Add Item, Not a Trader.
- Money: from / to fields, empty while the character sets none (the base's amount as the hint), Never runs out beside them, a Clear icon to unset.
- Template and Squad are the grid's switches. Problems show as sentences under their fields; the codes stay in MCP and the build.
- Unsaved edits ask before any switch. Validation runs on change. An item, a portrait or a dialog the module brings itself is no unknown one (the module's own sections, `gz-mod`'s `sections.rs`).
- MCP: `characters.view` (select, path, show=<picker>, save / discard).

**Screens & Music** (`screens`, Mod > Screens & Music; `screens.info`, `screens.set`, `screens.import`, `screens.view`). Saves to mod.ltx as it is edited, like World Rules; the toolbar row says so.
- Toolbar: `Loading | Menu` (which surface the tab edits), then `Screens [Mix In | Only Mine]` and `Music [Mix In | Only Mine]` for that surface - each a muted caption with a group gap before its choice - then the status.
- Left (260 pt): Screens (rows of that surface) and Music, each with a + at its header; under Music `Repeat | Next Track`. A row's dot is a problem of it; its hover action removes it.
- Right (a column of up to 1040 pt, centred): for a screen a 16:9 preview (pictures as they are, the rest marked where it goes) as wide as the column, Layers, Music, and More folded (Name, Show On, Levels, Show Over It, Weight). For a track: File, Shuffle On, Volume, Weight.
- Music is the kit's combo: Any Track, Silence, the video's own sound (on the loading screen), then the surface's tracks, each ticked on and off.
- Layers: as many as the author adds, the top row is the top layer. + adds an empty one, the row's cross removes it, a row is dragged to reorder. A click opens that layer's File, Fit, Opacity and folded Position; a shader adds Texture, a video Its Sound when the screen plays it.
- Files arrive by drag and drop, from Explorer or the Content Browser (the tab's drop zones, `gz-ui-pda`'s `drop.rs`; a tile lights up what is under it, a file from Explorer only lands; inner zones are asked first): on a layer it replaces its file, on Layers it adds layers, on Screens or the empty tab each becomes a new screen, on Music tracks; an `.ogg` is a track wherever it lands. Files from outside are copied into `screens\`, a PNG/JPG/TGA/BMP becomes a `.dds`, a `.ps` gets its `.s` (`screens.import` is the same path for an agent).

**World Rules**
- Toolbar: Reload Game Data, Empty World... (confirms through `dlg.empty_world`). The status reads "Saved to mod.ltx as you edit".
- One scroll up to 900, kit `Section`s: Original Game (its count "n of 3 offered"), a row per rule - its label, then one segmented control [Remove | Keep | Player Decides]; Player Decides keeps the rule unless the player turns it off, and a fourth one is refused with a sentence; Keep Everything under them. Player Start (Place in Scene). Starting Kit: Money, Keep the perks, and the kit's `ItemRows` (Item | Count | cross, Add Item, drag to reorder; a row is sent once it has its item), Game's Base Kit and Remove Kit - an ordinary button: Edit > Undo on the tab puts the kit back. More (folded): Applies On (chips), Playable Area, Transitions (a filterable Transition | Leads To | Closed table).
- Problems go in a drawer. An item the module brings itself is no unknown item (W208).
- MCP: `world.info`, `world.set`, `world.lookup type=levels|transitions|patrols`, `world.view`.

**Mod Settings**
- Nav, 200, one `NavRow` per page with its icon and a problem dot: General, Mod Browser, Game Mode, Relations, Build Contents, PDA, Commands. A dot says only what was looked at: the project's scripts are read when the tab opens, on the pools, and a function is not called missing before they are.
- The page, up to 760: its title and what it is for, then one kit card holding the page's label-left grid, a tone strip while the page has an error or a required field; validation under each field. The mod name and the mode title stay separate.
- Mod Browser: Picture (the picture, its state and Choose Picture... / Remove beside it) and Tags (the fixed vocabulary as checkboxes) are rows of the same grid.
- Relations: four rows of the same grid (Requires, Load After, Load Before, Conflicts), each a chip list with Add Module...: a searchable list of the modules installed in the linked game (`modules\` and `mods\`), and a typed id for one that is not installed.
- Build Contents: Not Shipped, a block of paths and patterns.
- Commands: the kit's `ItemRows` - Name | Function (its chevron picks a `<script>.<function>` of the project's scripts) | Arguments | Cheat | cross, Add Command, drag to reorder; a field with an error has the error outline. Under the table the selected command's Help and every sentence about it. Read the Scripts Again reads the scripts once more.
- The tab saves as it is edited: the toolbar row says so ("Saved to mod.ltx as you edit", or what stops the save). While a new picture waits, the row holds Save (Primary) and that status. There is no footer.
- MCP: `mod.view` (page, ask_game_updates, dialog); the data is `mod.info` / `mod.set` / `mod.thumbnail`.

**Build dialogs**
- **Build to Folder and Export Flat (M):** a PathField, and Writes To: the folder the build fills, read from its end (`<folder>\modules\<id>`, `<folder>\gamedata_<id>`). The label column is 8 em. A clean build that would delete files it did not write refuses; `dlg.clean_target` names up to ten of them, Cancel the default, and Yes builds again with `clean`.
- **Package Release (M):** Output Folder, Requires Game, and the Author Key row (read-only unless the key asks for its password), a label column of 9 em. Replacing an earlier package of the version asks through `dlg.replace_release`.
- **Result:** the kit's `Result` (section 4).
- The Mod menu's Build to Folder..., Export Flat Gamedata... and Package Release... are `mod.view dialog=`; Build Into Game is `mod.build target=game` (a person in the window is asked what the build needs, a command asks nothing).

**Publish Release (L)**
- One Release line (name · version · repository, read-only: Mod Settings edits them) and the Notes - a bordered field with a hint, Markdown - on the surface; Tag, Title, Options, Requires Game (the linked game's version; `release.publish requires_game=` names another), the Author Key and the account under More unless they are what stops the release. The label column is 9 em.
- It reads the repository's releases as it opens (Publish is off meanwhile): a version already out - `v<version>` or `v<version>-rN` published - is a problem before the click, with [Open Mod Settings] and [Open Releases]. A read that fails is a muted note: GitHub checks again at publishing.
- Every problem is listed at its top - except "not signed in", said once, in the Account row with Sign In / the token field beside it.
- The device code is shown in Title Mono with Copy.
- It closes when the job starts; the progress is the Task Center's card ([Stop Publishing]) from the packaging through the upload, and a click on the toolbar's Publish button brings it back; the result is the kit's `Result`.

**Releases**
- A sortable table: Tag (fixed at the widest tag) | Title | State ("Published" in secondary text for one that is not the latest) | Published (local) | Files ("3 files · 15 KB"), column lines in the header only, plus Details of the selected release (label column 8 em; `release.view select=`).
- Edit... (`dlg.edit_release`; `release.view select= edit=true`) and Delete... (GhostDanger in the details toolbar, the row's context menu in the error colour, Del). Delete names the release, its files and tag, and what players get instead.
- An empty state's action (Open Mod Settings, Sign In..., Publish Release...) is a Secondary button: Build Mod is the one Primary of the row.
- A failure is a banner sentence naming what failed.

**Submit to Mod Browser (L modal)**
- Id `###dlg.catalog_submit`, answered as `submit`. Fixed height; the body scrolls.
- Cards (`card`), their sentences wrapped at the card's width:
  - Author Key (label column 8 em).
  - Requirements, failures first, the met ones folded under "Met (n)", VirusTotal findings left out.
  - "VirusTotal (required)", in the Required tone and never grey. This is the only place the verdict appears. [Scan] [Open Report] [Forget VirusTotal Key]; Open Report is on only while a package has a finished report, and Forget never touches the author key. While a check has not looked the packages up yet, the card says "Looking the packages up on VirusTotal..." instead of claiming they are not scanned.
  - Submission: the verdict, and "Submitting opens a public issue at github.com/MMadmer/darf-mods from <login>." "Not submitted yet." shows only once GitHub answered; until then it looks the submission up.
  - Withdrawal: Withdraw... (GhostDanger) raises the sheet `Withdraw from Mod Browser###dlg.catalog_withdraw`, the Danger Withdraw and Cancel the default.
- Every card button is Secondary.
- Footer while a job runs: the kit's busy line with the job's stage ("Waiting for VirusTotal: the report on 2 of 3 packages") and one [Stop <job>] (Stop Checking, Stop Scanning, Stop Submitting, ...); the title's cross hides the window and the job goes on. Idle: the submit button - Submit, Scan and Submit, Update Submission or Scan and Update, at the width of the widest - and [Close].
- The project closing stops its job and takes the window and its results with it.
- MCP: `release.catalog_status action=status|check|submission`, `release.catalog action=scan|submit|withdraw`, `release.key`, `release.account`; `release.view submit= withdraw_question=` puts the window and its sheet up to be looked at.

**Start page** (`gz-ui-app`'s `start.rs`; `project.page`)
- A page filling the work area under the caption (`Start Page###start_page`), not a modal; the editor's version ("Grizzly Editor 0.7.0") in its bottom right corner, the status bar's chips as on any page.
- Its status bar has no centre: the centre is a document's `status()`, and the page is no document (`ui.list` answers `status_centre: null` here, the rect of the Level tab's status on Level).
- Head: [Open Folder...] [Open Project File...] (the system dialog over `*.grizzly`), the SearchField (24 em at most), and Segmented Last Opened / Name at the right (kept in `_projects.ini [browser] sort`).
- The cards, 232x184 pt, in rows on the page itself (no frame round them). **New Project is the first slot**: a card with a + that turns into the form in its place - Name with its rule as the hint, one line under it (what is wrong, else the location with its start cut), Location under More, [Cancel] [Create] at its foot, Create the Primary and the only one. Nothing else moves. With no project on the list the form is up by itself.
  - A card shows the project by its project file: the name of `<Name>.grizzly` (the folder's for a folder that holds none), the preview cropped to the card, the name on a row of its own with the ⋯ menu, and the path with a start ellipsis.
  - Duplicate names keep the parent folder that differs whole in the path line, in the text colour.
  - A Missing / Not a project badge.
  - The ⋯ menu (or a right click), with icons: Show in Explorer, Delete... in the error colour, Remove from List.
- A project file or a project folder dropped from Explorer anywhere on the page opens.
- The preview is a pass of its own (`gz-ui-level`'s `doc.rs`): the open scene rendered once more from the viewport's camera, without grid, pivot, selection, gizmo, axis triad, stats or captions. A picture of nothing keeps the last good one.
- Delete opens a top-level `Delete Project###dlg.delete_project` (M; Delete is Danger, Cancel the default); a project that opens under it takes it down.
- Link Game and Recovery are M cards on this page (`BeginPageCard`): Link Game [Close Project] [Link]; Recovery defaults to [Restore], [Discard...] confirms, naming the snapshot time and the older snapshots.
- Link Game takes the page only while no valid game is known at all - neither the editor's default game (found by itself: a game this home knew, the installed game, a game beside the editor) nor the project's own. It lists every place the editor looked at as a pick list (the folder, why it is or is not a game, where it was found), the first one in the Game Folder field; the field's hint says what the folder is. Link sets the project's own game on this machine (`game.link scope=project`). Nothing is said twice: an empty field reads "Browse for the game folder."
- A project opens from a card, Open Folder..., Open Project File..., a drop, `Grizzly.exe <project file | mod.ltx | folder>` or a double click on its project file (`.grizzly`, registered for the user at start). A second start of the editor on the same home hands its project to the running one, which asks about unsaved work in the Switch Project way (`unsaved_prompt` `open_project`) and comes to the front.
- Import Base Scene is an M dialog with search; it reads its list once.
- No job of a closed project is shown: closing a project stops its jobs.
- MCP `project.page [show=menu|confirm|none path=] [query=] [sort=recent|name] [new_project=]`: the cards as drawn (`path`, `name`, `state`, `parent`, `preview`), the search, the sort, `new_project`, and a card's menu or the confirmation put up to look at, never to act.

**Preferences**
- Layout: search, a nav list with an icon per page (as Mod Settings), and the grid. A page is at most 44 em, a value at most 16 em.
- The pages, the groups areas add and the Keyboard page: section 5. Interface holds Scale (Auto, 100 % ... 200 %) and Accent.
- Settings apply live and are saved when a field loses focus and on close.
- Labels become official. `bp_cull` and `bp_depth_tolerance` get their own keys. Tools > Snap holds the viewport toolbar's steps, not settings of their own. Recovery reads Enabled, Interval, Delay After Input, Snapshots Kept (units in their help).
- Game is a page of its own (`gz-ui-app`'s `game_prefs.rs`): Default Game - Folder, Found (how), Version, [Change...], [Find Again] (`game.link scope=default` / `find=true`); This Project - Game ("Uses the default game" or its own), [Change...], [Use Default Game] (`game.link scope=project`). Mod > Link Game... opens it.
- Library is a page of its own: Folder (the editor's library, beside it by default), [Change...], [Use the Editor's] (`project.set_library`), and what the open project still takes from it.
- Project Files is a group of the General page: Open .grizzly Files with This Editor (on; `project.associate on=`) and File Association: [Register Now] with what Windows opens project files with now.
- File > Project > Collect Library Assets... (`ui.dialog open id=collect_library`) works out what the project's scenes take from the library (`project.collect dry_run=true`), names the objects and asks once - "Copy 12 files (3.4 MB) from the editor's library into the project?" (the kit's confirm, `dlg.collect_library`, not a danger: nothing is lost) - then runs `project.collect`, whose job has a card. Nothing to collect is a note.
- Updates is a group of the General page (`gz-ui-app`'s `update.rs`, a `PrefGroup`): Check at Startup (on by default; `update.set check_at_startup=`), Installed Version, and Check Now with what the last check said. A development build has one read-only row instead: This Build - "This is a development build: it does not update itself."

**Updates (docs/UPDATES.md)**
- One status chip, right of the Build chip: `Update 0.7.1` (available; Error tone after a failed download), `Updating 42%` (downloading), `Waiting` (parked until the connection is back), `Restart to Update` (ready), `Updating` (applying). Nothing while idle, checking or off. Its tip is `update.status`'s `text`; a click opens the dialog.
- The dialog `Update###dlg.update` (M; MCP `ui.dialog action=open id=update`): "Grizzly Editor 0.7.1 is available. You have 0.7.0.", "38 MB of 121 MB to download" (the delta of the archive), the release notes in a scroll area as plain paragraphs (`#` lines in Strong, `-` and `*` lines as bullets). Footer by state: Download and Install (Primary) | Later; while it works the kit's busy line with the job's stage, Stop Updating, Hide; ready: Restart and Update (Primary, off with the job named while a build, a compile, Play or a packaging runs) | Update on Exit, which closing the dialog means too; manual (a newer schema, a folder the editor cannot write): the sentence and Open Release Page | Later; idle and off: the sentence and Close.
- Restart and Update leaves the editor the way Quit does - unsaved work is asked about, and Cancel there cancels the restart, leaving Update on Exit. The update is applied once the window is gone, before the process ends.
- One toast at startup when the check finds an update ("Grizzly Editor 0.7.1 is available", with [Details]), one at the first start of the new version ("Updated to 0.7.1", with [What's New]); both buttons run `help.update`, the dialog with the notes. Help > Check for Updates... (`help.check_updates`) starts a check and opens the dialog.

**Libraries** (`gz-ui-previews`'s `library`; `library.view`)
- Light Animations, Texture Library and Sound Library are documents: list | preview or timeline | grid. An unsaved prompt guards their edits. The Libraries menu opens the three and Reload Objects; what a library does by hand is under its tab's More.
- **One empty state for one fact.** A library with nothing in it says so once, over the whole tab, naming the folder or file it reads, with what fills it as the single Primary (the Texture Library: Import New Textures). With entries and none picked, the preview says "Pick ..." and the details stay blank.
- The Texture Library reads the `.tga` sources of `rawdata\textures`. As it opens it looks at the import folder by itself and offers what is new there over the list ("3 new pictures in the import folder." [Import]); Import turns the tab into New Textures, and the finished import - which builds the game textures - brings the library back. More: Check New Textures, Synchronize Textures..., Rewrite Texture Descriptions..., Reload Textures. The Sound Library's More: Synchronize Sounds.... Whether a library's folder can be written is looked at on the IO pool as the tab opens (until it answers, the tab is not read-only).
- Detail Object List becomes an L dialog (`dlg.detail_objects`, above).
- Choose (`###dlg.choose`, the Chooser above). Every list is picked, never typed (`gz-ui-level`'s `lists.rs`, read off the UI thread and kept for the session):
 a spawn's Model\Animation lists its model's motions (its own and those of the `.omf` files it names), Fixed bones and a light's bones its model's bones (Fixed bones takes several, the ones it has picked already), Motion the game's `anims\*.anm` / `*.anms`. An empty list says what is missing ("The model ... has no motions.", "The object has no model."). `scene.choices` answers the same list (`list`, `total`, `choices` with `query` / `limit`, `several`, `empty`); `scene.get` names a pick property's `list`.
- Edit Text drops its crashing Load/Save.

## 7. MCP

The window's commands are typed cards of the registry (`docs/COMMANDS.md`): `help <name>` gives each one's parameters with their doc. **Lengths are pt both ways**; a rect is `[x, y, w, h]` pt. A picture is px with its `scale`. A window command in a run without a window answers `no_window`. An opener brings its target up in the same frame; reads never switch the tab.

**Held for frames.** The editor draws on demand, so a command that reads what is drawn waits for the frames it needs before it answers (`editor.frames.after(n)`): `shot.window`, `ui.list`, `ui.audit`, `ui.frame`, `ui.scroll`, `shot.viewport`. Commands that change what is drawn and answer with where it landed are held for the frames that show it, in the same call (`graph.view`, `pda.preview`, `preview.model`, `preview.texture`, `content.view`). Nothing answers `pending`.

| Command | What it does |
|---|---|
| `state` | one orientation answer: the project, the scene, the open tabs with their dirty flags, the active tab, the jobs that run, the problem counts, the modal on top, `window` (there is one) |
| `ping` | the editor's name and the version it was built as |
| `app.window action=state\|minimize\|restore\|maximize\|resize\|move\|focus [width= height= x= y=]` | the window; `resize` in pt of the client area |
| `app.quit [save= \| discard=]` | ends the editor; unsaved work refuses with `unsaved` unless one of the two is given (`discard` is destructive) |
| `app.switch_project [save= \| discard=]`, `app.about` | the File and Help rows |
| `app.prefs action=get\|set\|show [key= value= page= search=]` | the preferences: a set applies at once and is saved; `show` brings Preferences up on a page; `accent` and `scale` are keys too |
| `app.keys action=list\|bind\|clear\|reset\|resolve [id= keys= query=]` | every action with its keys, defaults, scope and conflicts (`keys`); `bind` answers what let a chord go (`moved_from`); `resolve` what a chord runs now |
| `doc.list`, `doc.save`, `doc.undo`, `doc.redo`, `doc.reload [discard=]`, `doc.edit action=cut\|copy\|paste\|duplicate\|delete\|find [doc=]` | the active tab's (or `doc`'s) file and Edit actions |
| `ui.tab action=list\|activate\|open\|close\|next\|prev [doc= kind= path= source= save= discard=]` | the major tabs; a dirty close without `save` or `discard` answers `unsaved`; `kind` is any of `ui.contract` `document_kinds` |
| `ui.list` | what the window shows: tabs, panels {id, window, open, visible, docked, focused, closable, host, rect}, drawer, dialogs, popups (with each row's `label_x`), work_area, start_page, status_centre, toasts and `toast_notes` (with their buttons), size_class, layout_custom, scale |
| `ui.frame` | the frame's geometry: band, caption_h, menus_end, buttons_x, interactive, folded, tab_row, tabs {id, rect, icon_only}, toolbar, project_toolbar, toolbar_status, status_bar, status_line, status_chips, scale |
| `ui.panel action=open\|close\|focus\|toggle id= [floating=]` | the Level panels: `place`, `world_outliner`, `details`, `level_settings`, `content_browser`, `output_log`, `viewport`; answers `host` |
| `ui.drawer action=open\|close\|toggle [id=content\|log]` | |
| `ui.dialog action=list\|open\|close [id=]` | `package_release`, `publish_release`, `build_to_folder`, `export_flat`, `import_base_scene`, `submit`, `update`; close = Esc, and takes a kit dialog's `dlg.<name>` |
| `ui.popup show=<id>\|none` | to look at, never to act, one at a time: `menu:<name>`, `tab_menu:<tab>`, `tab_overflow`, `toolbar_overflow`, `toolbar_split:<id>`, `cb_context:<path>`, `cb_add`, `cb_view`, `cb_favorites`, `cb_grid`, `outliner_filters`, `outliner_more`, `viewport:<menu>`, `tasks` (once a job ran), `rebuild` (the Build chip's popover, while a step is stale), `level_more` (the Level toolbar's More) |
| `ui.toast text= [tone= action= label=]` | a note, optionally with a button that runs an action |
| `ui.scroll id= to=top\|bottom\|<pt>\|item:<name>` | a panel, a drawer, a dialog or the work area; answers `scroll_y`, `scroll_max_y` |
| `ui.split [key= value=]` | the layout store's sizes; with no key, every key |
| `ui.audit [id=]` | what the next frame cut short (`truncated`), let escape (`escaped`), its `controls` (enabled, why, help), and `loading`: the busy lines it shows - a surface that has not got its content yet, so a caller waits for an empty list before it judges or pictures one |
| `ui.scale [value=]` | the scale for tests until the editor ends; never saved |
| `ui.contract` | the constants QA pins (`gz-app`'s `contract.rs`): panels, drawers, dialogs, kit dialogs, document kinds, popups, menus, viewport menus, the palette's id, the Task card, tile range and defaults, accents, categories, card menu labels, the scale range, the interface scales, the stall limit |
| `ui.reset_layout [scope=level\|all]` | Window > Reset Layout; `all` also forgets every split |
| `ui.reveal [path= dry_run=]` | a file or folder of the project in Explorer |
| `ui.palette action=state\|query\|open\|close\|run [query= id= limit=]` | the Command Palette: ranked rows {id, kind, label, category, detail, shortcut, enabled, why, recent}; `run` runs a row by its id - an action's, `tab:<id>`, `panel:<id>` |
| `ui.tasks [show=]` | the Task Center as drawn: `running`, `finished` (with `result`), `card_open`, the counts, `status_line` |
| `shot.window [frames=]` | a picture of the window as the commands before it left it: `png_base64`, `width`, `height`, `scale`, `active_tab`, `busy`, `modal` {title, id, rect px}, `outside` |
| `perf.stalls [clear=]` | the UI passes and events over 100 ms: `stalls`, `worst_ms`, `limit_ms` |
| `level.show [dialog= popover= maximize= problem=]` | the Level tab's own windows for a person: a file dialog (`open_scene`, `save_as`, `place_group`, `save_group`, `export_obj`, `export_selection_obj`, `compile_folder`; refused for a caller who cannot answer one, naming the command to run instead) or Summary Info (`summary`); a popover to look at (`build`, `play`, `more`, `rebuild`, `none`); the viewport over the tab (`on`, `off`, `toggle`); a problem gone to as a click does; answers `maximized`, `popover_open`, `chooser`, `summary_open`, `problems`, `build` |
| `scene.choose [action=open\|move\|close\|state key= object= world= query= highlight=]` | the chooser of a property for a person: opened, searched, a row highlighted, closed; answers `chooser` {title, list, query, shown, first, highlight, picked, current, several, preview} |
| `job.list`, `job.get`, `job.wait`, `job.stop` | every job of the editor, with its `owner` |
| `project.open path= [save= discard= from_launch=]`, `project.close [save= discard=]`, `project.create name= [parent=]` | a project in the window: a folder, a project file or a `mod.ltx`; `from_launch` asks in the window when unsaved work is in the way; a close goes to the Start page |
| `game.link [path= scope=project\|default find=]`, `game.info [candidates=]` | Link Game, Preferences > Game |

Discarding is a call through the bridge's `grizzly_destroy` (`destructiveHint`): `app.quit discard=true`, `ui.tab action=close discard=true`, `project.close discard=true` and the other commands that declare `discard` (`docs/COMMANDS.md`).

The surfaces' own commands are in their crates (`graph.view`, `viewport.view`, `viewport.camera`, `viewport.gesture`, `pda.preview`, `screens.view`, `world.view`, `characters.view`, `mod.view`, `release.view`, `pda.open`, `content.view`, `project.page`, `preview.model`, `preview.texture`, `shot.asset`, `library.view`, `library.*`, `level.view`, `level.show`, `scene.choose`, ...).

## 8. Implementation

**Where it lives.**
- The kit: `gz-ui` (section 4); `-kitsheet` shows it, `-iconsheet` the icons.
- The shell: `gz-app` - `shell.rs` (the state of the window, keys, tabs), `chrome.rs` (the frame, menus, status bar, the palette, the Task card), `actions.rs` and `shell_actions.rs` (actions), `palette.rs`, `tabs.rs`, `tasks.rs`, `stalls.rs`, `shots.rs` (pictures held for frames), `cmds.rs` and `app_cmds.rs` (the commands of section 7), `contract.rs` (the ONE place for a constant `ui.contract` answers), `settings.rs` (`grizzly_editor.ltx`).
- One crate per surface, each with one `install(builder)` for its documents, actions, menu rows, toolbar items, preference groups and dialogs, and one `register(reg)` for its commands: `gz-ui-app` (Start page, Output Log, Preferences, the Keyboard page, updates), `gz-ui-content` (Content Browser), `gz-ui-level` (the Level tab's dock and panels), `gz-ui-viewport`, `gz-ui-graph` (quests, behaviour graphs), `gz-ui-previews` (model, texture, the libraries), `gz-ui-pda` (PDA preview, Screens & Music), `gz-ui-mod` (Mod Settings, the build dialogs), `gz-ui-world` (Characters, World Rules), `gz-ui-release` (Package Release, Publish Release, Releases, Submit).

**What the surfaces still carry of their own** (`local.rs`), to be replaced by the kit's one widget of its job in their own streams: fold headers (`form::section`, `form::raised_section`, the graph's fold header, the release fold) by `Section`; checks (`form::check`, the graph's, the PDA's, the world's cell check) by `Check` / `Switch`; pick lists (the graph's kind picker, Mod Settings', the world's item picker) by `PickList` and `Combo`; item rows (Characters', World Rules') by `ItemRows`; table heads (the release table head, `list::table_headers`) by `Table`; per-surface drop code by `drop` (Screens & Music's); ellipsizing in hot paths (the graph canvas) by `text::fit`. Other local widgets, each used by one crate:
- `gz-ui-app`: rule, the hint field's audit wrapper. `gz-ui-content`: tree row, breadcrumbs, rename box, checker, menu heading, marquee.
- `gz-ui-graph`: pane header. `gz-ui-level`: eye toggle, lock toggle, button flow, panel header, thumbnail, badge button, tab, tree list.
- `gz-ui-mod`: password field, value column, picture. `gz-ui-pda`: list header, tab strip, code view (with the Lua colouring and the layout over frames), drop mark.
- `gz-ui-previews`: checker, heading, frame, texture pick, wheel. `gz-ui-release`: secret field and row, code, card title.
- `gz-ui-world`: number text, dirty dot, problems drawer, level picker.

**QA of the UI** (each on a copy of a project, in a home of its own): `tools/qa/test_ui_shell.py` (the frame, tabs, drawers, panels, dialogs, size classes, menus and their one label column, the palette and its actions, rebinding, stable chrome across every tab kind, the stall monitor, the Task Center), `test_tour.py` (every tab kind, dialog and menu of `ui.contract`, audited and pictured at both sizes), `test_graph_ui.py`, `test_previews.py`, `test_properties.py`, beside every surface's suite. The kit's own tests (`cargo test -p gz-ui`) hold the pick list's ranking and keys, `Section`'s fold, `ItemRows`' add, remove and reorder, the one-Primary assert, the menu's label column and `text::fit`'s cache.
