# Grizzly Editor UI 2 - design specification

This document is the contract of the editor's UI. It is implemented - first by the C++ editor (phases 0-2 of its plan, three visual tour rounds, QA green), now by the Rust editor (`gz-ui`, `gz-app` and the surface crates, section 8) - and CLAUDE.md, AI_SETUP.md, NQ_ARCHITECTURE.md, NQ_FORMAT.md and PDA_FORMAT.md follow it. Where the implementation deliberately went another way, the text below says what was built. A UI change is a change of this document first; where it disagrees with another document, it wins.

- **pt** is a logical point (egui's unit). px = pt x the context's pixels per point.
- **#n** is an issue in `audit_live.json`, in file order. **Tn** is a tour step (section 8).
- Targets are 1707x912 pt (2560x1440 at 150%) and 1280x720 pt, each at UI scale 1.0 and 1.5.

## 1. Principles

1. **Documents own the window.** Every asset editor is a full-window major tab. **Level** is tab 1 and cannot be closed; its centre holds only the viewport.
2. **The canvas absorbs change.** The viewport, graph, preview or tile grid takes every resize. Side panels, drawers and label columns keep their pt size. Below its minimum a container scrolls; it never clips.
3. **Nothing clips silently.** Text fits, ellipsizes with a tooltip, or wraps if it is prose. Rows overflow into `»`. `ui_audit` checks it.
4. **One size system, one kit.** Sizes are pt (`gz-ui`'s `metrics`, `units::em`), and DPI enters in one place. The kit's types leave no room for a raw px size or a colour literal (D6). Each job has one `gz-ui` widget.
5. **Quiet chrome, loud content.** Accent means active, selected, focused or primary; semantic colours mean state. Accent `7971bd` is never text colour, and required is never grey.
6. **Name first.** Labels name the thing. Help lives in tooltips. Values from a list are picked.
7. **Safe and honest.** Destructive actions name what goes, and Cancel is the default. Unsaved work is never lost silently. A label that depends on state keeps its width.
8. **Agent-drivable.** Every surface can be opened, closed, focused, scrolled and looked at over MCP. Activation is synchronous, only the spinner animates, and MCP never prompts.

## 1a. Simple, visible, and an agent's as much as a person's

The rule every surface is held to; the Screens & Music tab is its reference. A screen that fails it is a bug, whoever wrote it.

- **One task, one view.** A list of the things on the left, the selected thing on the right. No dashboards of controls, no second window for what fits in the tab.
- **Show the common case, fold the rest.** What most authors touch is on the surface; everything else is under one folded **More**. Nothing is shown just because it exists.
- **Nothing to declare that can be worked out.** The kind of a thing comes from its file, a name from the file's name, a default from the module. A field is the last resort.
- **Direct manipulation.** + on the list's header, a cross on the row, drag to reorder, drop a file where it belongs. No "Add" dialogs and no Apply: an edit is saved as it is made.
- **Show the result, not its numbers.** A preview, a picture, the thing as it will look; a thing that cannot be shown is marked where it goes.
- **Problems stay with what they are about.** A dot on the row, a sentence for the author on its details. No codes on screen, no log to go and read.
- **Everything a person can do, an agent can do, by the same code.** Every action has an MCP command; every result of it is readable text on disk (`mod.ltx`, the declarative assets), so an agent can also just edit the file. A surface without both is not finished.

**Applied (section 1a):** the graph toolbar keeps Save, Undo, Redo, the problems count when there are any, Frame All, Find, the panels and the zoom; the rest is its More menu. The Graph panel adds tasks and roles with + on the header and removes them with a cross on the row, ID and a role's details under More. Mod Settings saves as it is edited; Characters saves on Save (Ctrl+S), with a dirty dot on the row and a question before unsaved edits are left. World Rules keeps Original Game, Player Start and Starting Kit on the surface, the rest under More, and shows a problems bar only when there are problems. Publish Release is one Release line and the Notes; Package Release is one click; Submit shows a working key as one line and one spinner per job. The Start page is the projects, its form opens on New Project.

## 2. Tokens

**Colours.** The theme's `Palette` (`gz-ui`'s `theme.rs`) holds the tokens below and a colour at an opacity; nothing else writes a colour. Existing values are in **bold**.

| Token | Value | Use |
|---|---|---|
| chrome | 17161b | caption, status bar, inactive tabs, dock strips, window brush |
| base | **202025** | panels, documents, toolbar, active tab |
| inset | **1c1b20** | lists, tables, grids, tiles, log |
| canvas | 151419 | viewport, graph, previews |
| raised | 26252c | headers, cards, node bodies |
| overlay | 28272e | menus, popups, tooltips, dialogs, toasts; pills at 92% |
| input | 2a2931, hover 302e38 | replaces 333333 |
| hover / pressed / button | 2e2c35 / 36343f / 34323d (hover 3d3a47) | |
| border | subtle 2c2a33, **3d3947**, strong 524d5f | |
| text | **f2effb**, secondary bdb8c9, muted **8e8a99**, disabled 5f5b69 | contrast on inset 15.2 / 8.6 / 5.1 / 2.6 |
| accent | **7971bd** | active tool, checkbox fill, toggles that are on, drag |
| accent.bright | **a79fed** | focus, check mark, caret, links, tab overline, active icons, tile outline, marquee, info |
| selection | **464150**; focused **554f72** | |
| primary | 6a62ae, hover 7971bd, pressed 5c5599 | white semibold label, 5.3:1 |
| danger | 8a3b43, hover a3464f, pressed 74323a | |
| status | error **e06c75**, warning **e5c07b**, success **98c379**, required d19a66 | |
| axes | X e06c75, Y 98c379, Z 61afef | |
| tint / dim | tone at 16% / 0c0b0e at 60% | |

Graphite changes only the accent family: accent 727782, bright 9399a5, selection 454951 and 50545d, primary 60656f.

**Metrics (pt).**
- Spacing steps: 2, 4, 6, 8, 12, 16, 24, 32.
- Padding: WindowPadding 8,8, FramePadding 8,4, ItemSpacing 8,4, ItemInnerSpacing 6,4, CellPadding 6,3.
- Other metrics: Indent 14, Scrollbar 10, GrabMin 12, DockingSeparator 2, Overline 2, TitleAlign 0,0.5, no window menu button, FrameBorder 1.
- DisabledAlpha 0.4, so existing `BeginDisabled` code still dims.
- Radius: 4 for controls, 6 for popups and cards, 8 for dialogs, 0 when docked. Focus ring 2 pt accent.bright.

**Sizes (pt).**
- **Chrome:** caption 32 (window buttons 46x32); tab row 30, major tab 28 tall at its foot, 96-220 wide; toolbar 36 (buttons 28, glyph 16, group gap 12); panel tab strip 26; panel toolbar 32; status bar 24.
- **Controls:** frame 24; row 22; property row 26; section header 28; inline icon button 24.
- **Dialogs:** S 400, M 560, L 760, XL min(1040, 90% W). Never wider than W-32 or taller than H-48. Title 36, footer 52, buttons at least 88x28.
- **Other:** tooltips and toasts up to 360; drawers 40% of H, clamped to 260-480; forms up to 760.

**Minimum sizes (pt).** Viewport 360x240. Outliner 220x160. Details 280x200. Tools 240x200. Content Browser 360x150. Output Log 320x110. Document side panel 220. Document canvas 400x260.

**Type.** One Segoe UI face covers Latin and Cyrillic.
- A font size is the line box, and the Segoe em is 0.75 of it, so Body 16 is a 12 px em (Windows 9 pt). No text is smaller than 14.
- Three files are read (segoeui, seguisb, CascadiaMono; `gz-ui`'s `font.rs`), before the window opens, and the startup profile times them as its `Loading fonts` stage.
- egui's own fonts are the fallback where Segoe UI is missing.

Type styles:
- **Body:** segoeui 16. The default.
- **Strong:** seguisb 16. Headers, node titles, Primary labels.
- **Small:** segoeui 14. Status bar, table headers, captions, hints, badges, keycaps, pins.
- **Title:** seguisb 18. Dialog titles, document titles, empty states.
- **Page:** seguisb 26. The Start page.
- **Mono:** CascadiaMono 15, falling back to consola. Log, source, stats.

**DPI in one place.** Sizes are points and egui scales them by the context's pixels per point, the one place DPI enters: side panels keep their pt size and dialogs recentre when the scale changes. A monitor's scale is taken when the window meets it; `-uiscale <f>` and MCP `ui_scale` (tests only, never saved) win.

## 3. Iconography

Icons are vector only, in `gz-ui`'s `icons.rs` (geometry in `icons/geometry.rs`, drawing in `icons/paint.rs`):
- a 16-unit grid with a 1-unit margin, built once and then only scaled;
- stroke 1.5 units, at least 1 px, pixel-snapped at 20 px and below;
- round joins, outlines only; fills only for play/stop, dots, the pupil and on-states.

`icons::draw` draws one in a square, `icons::draw_in` centres one in a rectangle; `Icon::name` is its stable snake_case name (`chevron_down`), the one MCP answers and QA read. Sizes are 16 in toolbars, 14 in rows, 12 in the status bar, and 20-48 in empty states and tiles (a tile never draws its icon above 48).

How they draw: the geometry is built once, arcs tessellated then; at 20 px and below points snap to the pixel grid and a backtrack pass drops the spikes snapping would make; above 2 px of an opaque stroke, joins and caps are round; dashed paths are stretched to whole dashes. New glyph arcs start exactly where the previous line ends (within 0.02 units). Combo buttons, tree nodes, submenus and sorted columns draw the kit's chevron. `-iconsheet [first]` opens a developer sheet of every glyph at 16, 24 and 48 pt (`icons::sheet`).

Icon colours by state:
- rest: text.secondary
- hover: text
- checked: accent.bright on selection.focused
- disabled: text.disabled
- status: its own tone

The content browser's own-asset icons (`gz-ui-content`'s `icons.rs`, the kinds in `gz-project`'s `asset_kind.rs`) draw through this engine; their names stay (QA).

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

The kit is the crate `gz-ui` (`crates/gz-ui`), on egui; its modules are the kit's parts and its API is the crate's own (`cargo doc -p gz-ui`). The C++ kit's API this section once listed (`XUI`, `cpp-editor-last:Source/Editors/XrEUI/XrUIKit.h`) is `cpp-editor-last:docs/UI_DESIGN.md`.

- `theme` - the tokens: `Palette`, `Tone`, `Accent`, `metrics`; `font` - the six styles; `icons` - `Icon`; `text` - measured and ellipsized text (`Clip`, `prose`); `units` - `em`, `button_width`.
- `button` - `Button` with its `Kind` (Primary, Secondary, Ghost, GhostDanger, Danger), `icon_button`, `segmented`, `keycaps`.
- `toolbar` - `Toolbar::begin` .. `end`: `group`, `separator`, `spring`, `status`, `action`, `split`, `menu`, `search`, `segmented`, `widget`; labels fold, then items overflow into `»`.
- `menu` - `menu_item` with `MenuItem` (icon, keycaps flush right, danger), popups by name.
- `grid` - `PropertyGrid` per role: sections, rows with flags, a help footer, the value widgets (`text_field`, `number`, `switch`, `combo`, `color`, `vec3_field`, `pick_field`, `path_field`, `chips`, `text_block`, `read_only`).
- `list` - lists with stable row ids and row actions, `nav_row`, `table_headers`; `search` - `SearchField` and the one text filter.
- `splitter` - `splitter`, `splitter_tail`, `splitter_at`; `layout` - the sizes the author dragged, by key.
- `state` - `empty_state`, `badge`, `status_dot`, `spinner`, `busy_line`, `banner`, `toast`.
- `card` - `card`, `page`, `page_card` + `card_footer`.
- `dialog` - `Dialog` (`dlg.<name>`, S/M/L/XL, `sheet`), `Footer` (`status`, `busy`, `button`), the queued questions `confirm`, `unsaved_prompt`, `notice`, `result`, answered per `Asker`; `open_dialogs`, `modal_on_top`, `close_dialog` for MCP.
- `audit` - what a frame cut short or let escape, for `ui_audit`; `file_dialog` - the system's file dialog, for a person only.

**What drawing code may use**
- The kit, the theme's tokens and nothing else: colours through the context's `Palette` and its tones, sizes in pt through `metrics` and `units`. A shortcut shown in a menu is the kit's `MenuItem`.
- A widget the kit lacks is written as a `local_<name>` function in its surface crate's `local.rs` and promoted into the kit once a second surface needs it (section 8 lists what is still local).

**Buttons**
- One Primary per surface.
- The Danger fill goes only on the destructive action. The button that opens the question ("Delete...", "Discard...", "Withdraw...") is `Kind::GhostDanger`: a Ghost with error-coloured text. A menu entry that leads to a deletion is a `MenuItem` with `danger`.
- Disabled: input fill, disabled text, and the reason (`why`) as the tooltip.
- A label that changes with the state passes every label it can show (`label_set`), so the button keeps the width of the widest.
- Row actions are `RowActions`: squares as tall as the row (22 pt) with 14 pt icons, shown on hover - not 24 pt, so they fit inside the row. There is no small button.

**SearchField**
- A magnifier and a clear x. Esc clears; Ctrl+F focuses. Spaces separate terms that must all match, ignoring case (Cyrillic too, yo read as ye). It is the one text filter (`search::matches`).

**Toolbar**
- Never wraps. When too wide, labels collapse to icons first. Then items move into `»` (`toolbar_overflow`), rising priority first, rightmost first on a tie.
- It lays out from the last frame's measures. A folded label unfolds again only with 24 pt to spare, and a row keeps its layout while it still fits: a value that changes width never folds its neighbours. A value label keeps one width (the graph's zoom: a figure space leads a two-digit level).
- A toolbar's `segmented` measures itself; in the overflow its choices become radio items under its caption. A `Widget` has no overflow form and is hidden there.
- `Split` is an action with a chevron menu (Build Mod); in the overflow it is the action plus "<label> Options". `Menu` takes the name `ui_popup` opens it by (`request`); off, it keeps its caret and width, is drawn disabled with its reason as the tip and never opens.
- A standalone `Segmented` becomes a combo when narrow.

**Menus.** Every item that shows a shortcut is the kit's `MenuItem`: keycaps flush right in their column, at least 24 pt after the longest label, showing what is bound NOW, never a literal.

**Property grid**
- Label column:
  - Ratio .40 (a role may start at another share: Level Settings .50), stored per role in `grid.<role>`, draggable within Em(6)..width-Em(8).
  - Below Em(18), the label moves above the value.
  - Labels are Title Case secondary text with an ellipsis (Output Folder, Requires Game, Module ID); hints, messages and prose are sentences. The tooltip shows label, help and key.
  - No bullets. Each level indents 12.
  - Categories are full-row headers that toggle on click. A level lists its own rows first, then its categories, each in the order given: a row after a category would read as one of its rows.
- Values:
  - Every value starts at the left of its field, numbers included.
  - `Drag*` without steppers. A number shows its unit after it (m, °, fps); typing takes the number.
  - A value that needs more than its column on one line (`Vec3Field` under Em(15), a `Segmented`, a row of buttons) takes the line under its label, as every row does below Em(18).
  - A switch (`PropInline`) stays on its label's line when labels go above values: the box, then the label.
  - Colours as a swatch plus hex.
  - `PickField`: leaf name with a start ellipsis, plus browse and clear.
  - Multi-line text: first line plus Edit, which opens an M dialog. Edit is offered only by a text that holds several lines or is an ini text (Custom Data, User Data); a name or a path has none.
  - A row keeps the height it had when it was last drawn, so a tall value no longer makes the rows under it jump as it scrolls away. Rows are never culled in a window that sizes itself to its content: there a culled tall row grew one frame at a time. A value that still reads better as a block (a long text, a picture) is a `TextBlock` / `BlockLabel`: its label above it, as wide as the column.
  - Read-only: selectable secondary text; a path keeps its end (a start ellipsis, the whole path in the tooltip).
  - Differing values: "Multiple values".
  - Playback controls (First Frame ... Last Frame) are icon buttons named by their tooltips.
- Flags: Required shows a dot. Invalid shows a bar and a message. Modified shows a Reset icon. A message is a sentence for the author: problem codes (E103, W115) stay in MCP answers and the log.
- `HelpFooter` shows the hovered row's label and help, else the last one hovered while it is shown, else "Point at a property to see what it does."; one footer serves every grid of its panel. Text is drawn as text, never as a format string.

**Lists and tables**
- Rows are 22 pt, with stable ids.
- `NavRow` is the row of a nav, a page list or a sources pane - icon, ellipsized label, a status dot or a count, one hover action - in or out of `BeginList` (Mod Settings, Preferences, the Content Browser's favorites).
- A one-column list has no header. Otherwise the headers are `TableHeaders`: Small secondary text, centred over checkbox columns; a sortable table still sorts.
- Row actions appear on hover.
- An empty list shows `EmptyState`, which fits the height it gets: in a short region it drops the icon first, then the margins.

**Splitters.** A 1 pt line with a 6 pt hit area, re-clamped every frame so it builds no debt. A double-click resets it. Its value is stored in the layout store (`layout.rs`, `[layout]` of `grizzly_editor.ltx`; `ui_split`). `splitter` sizes the pane before the line, `splitter_tail` the pane after it (a side or bottom panel that keeps its pt size while the other takes every resize, section 1.2), `splitter_at` a pane laid out by hand (the graph tab's edges).

**Cards and pages.** `card` is a raised box as tall as what it holds, around one part of a surface, with a tone strip while it asks for attention (Submit's cards). `page` is the whole work area under the caption (the Start page). `page_card` / `card_footer` is a centred card with a dialog-like footer (status left, buttons right, Enter presses the default) on a page that is not modal: Link Game and Recover Unsaved Scene ask their questions this way.

**Dialogs**
- Modal, pinned, never placed from a saved layout. The id is `Title###dlg.<name>`, with no "Grizzly Editor - " prefix.
- Centred on open and whenever the work area changes; clamped instead if the user moved it.
- Height is at most H-48. The body scrolls, the footer stays put right under it, and the window itself never scrolls. A running job never resizes it.
- A dialog is measured on its first two frames; a measuring frame whose height is about to change is not shown, so the first frame anyone sees - a person, a screenshot, `ui_audit` - is the final layout. Keys never answer a dialog on a frame nobody saw. A held screenshot waits for the frames that show it.
- Footer: status on the left; [Primary|Danger] [Cancel] on the right. While a job runs, `Footer::busy(stage)` puts the spinner and the stage on the left, and the job's stop is a labelled footer button, "Stop <job>" - never an icon-only square.
- Enter presses the default and Esc cancels. In a destructive dialog both cancel. The keyboard focus opens on the default; the title bar's cross takes none.
- A dialog raised from a dialog is a sheet (`Dialog::sheet`): under its parent's title, no wider than it, the parent dimmed (#34). A `Confirm` raised inside a dialog opens as one. A sheet the author or MCP explicitly put up shows whatever `-nodlg` says.
- `Confirm` lists up to 10 items, then "and N more", the counts, and "Deleted for good - this cannot be undone." in warning colour.
- `UnsavedPrompt` offers [Save] [Don't Save] [Cancel]. The exit prompt lists the scene and every dirty document, with [Save All].
- `Result` is the one result dialog of build, export, package, publish and bake (`dlg.build_result`, `dlg.export_result`, `dlg.package_result`, `dlg.bake_result`): a toned title, rows, Not Shipped, up to 8 warning rows, [Open Folder] [OK]. It opens only for runs started from the UI.
- The queued kit dialogs (`Confirm`, `UnsavedPrompt`, `Notice`, `Result`) are drawn at the end of the frame by `draw_overlays`, nested above any modal open that frame; `then` runs outside every window.

**Precedence** (for kit questions; who answers is the caller's `Asker`)
1. `Mcp`: the caller has decided.
   - `confirm` answers Yes, and the action runs before the reply.
   - Losing data needs `save=` or `discard=`; otherwise the command answers `{error:"unsaved", dirty}`.
   - Nothing opens: a note answers OK, logged.
   - Under `-nodlg`, `window_control close` and `project_close` discard, as they always did.
2. `Unattended` (`-nodlg`): the logged answers - a question answered No is `~ [dlg suppressed]`, a note `* `, an error `! `.
3. `Ui`: the dialog opens, and `then` runs after the frame that answered.

Errors and notes for the author become queued notices once the UI draws. A command that asks first raises the kit's `confirm` and runs again on its Yes.

**Truncation and tooltips**
- Truncation: names at the end, paths at the start, "title - id" in the middle.
- Prose wraps at 36 em (`text::prose`). Numbers are never cut.
- Text painted by hand that was cut reports itself to the audit, so `ui_audit` sees it like the kit's own.
- Tooltips appear after 0.45 s, and never in the background or during a held shot.

**Audit**
- Kit widgets log `truncated`, `escaped` and `controls` (label, enabled, why, help).
- The audit also flags any window whose content exceeds its inner size on an axis that cannot scroll, measured from what THIS frame laid out, and skips a child whose root window is hidden.

## 5. Frame

Chrome totals 122 pt:
- **Caption (32):** logo, menus, a drag band of at least 48, the project name (dropped first), – ▢ ✕.
- **Tab row (30):** the major tabs and ⌄, from the left edge, in the chrome colour - a row of its own, so that no tab moves when the active one brings other menus into the caption (Level's Scene, Build and View, a graph tab's Graph). It is client area, not part of the drag band. The Start page has no tab row (nor a toolbar).
- **Toolbar (36):** the tab's `toolbar()`, then Spring, then [Build Mod ▾ (Primary split)] [Publish Release] [Releases]. Publish Release keeps its label while a job runs: the state is in its tip, and the button stays lit meanwhile.
- **Status bar (24).**

**Caption**
- It is drawn in every state (`gz-app`'s `chrome.rs`). The Start page, Link Game and Recovery show no menus.
- The caption registers the menus and window buttons as HTCLIENT each frame; the rest is HTCAPTION (the window's `WM_NCHITTEST`, `gz-win`'s `caption.rs`). A frame that does not draw the caption invalidates the band.
- The OS title reads "<project> - Grizzly Editor". The toolbar and status bar are bars of the window's frame.

**Menus**, in order:
- **File:** New / Open / Recent Scene, Save, Save As (the second preset of Save, Ctrl+Shift+S), Project ▸ (Import Base Scene..., Open Project Folder, Switch Project...), Quit.
- **Edit:** Undo, Redo, Cut, Copy, Paste, Duplicate, Delete, Find, Command Palette..., Preferences. These route to the active tab (its `Document::edit`).
- **Level only:** Scene (Find in World Outliner, Mark Selection as Mod Content / as Base Level, ...); Build (formerly Compile: Make ▸ and Compile ▸ - Geometry & Light, Detail Object Light, both compiled in the editor's process; no row starts a program: the AI map, the game graph and the spawn are built in the editor too); View (`DrawViewportOptions()` plus F11 Maximize Viewport).
- **Graph only:** Graph (the graph tab's own caption menu).
- **Mod:** Mod Settings, Characters, World Rules, Screens & Music, PDA Interface | Build Into Game, Build to Folder..., Export Flat Gamedata..., Bake Scene Layer ▸ | Package Release..., Publish Release..., Releases | Submit to Mod Browser....
- **Libraries** (formerly Editors): Light Animations, Texture Library, Sound Library, Check New Textures, Synchronize Textures / Sounds.... The Minimap Editor stub is removed.
- **Window:** panels, Content Drawer (Ctrl+Space), Log Drawer (Ctrl+\`), documents, Reset Layout.
- **Help:** Keyboard Shortcuts (Preferences on its Keyboard page), About.

Items show `ShortcutLabel` as keycaps. `Desc` strings are MCP ids and stay byte-identical. Under 1100 pt the menus fold into a single Menu button (`titlebar_info folded`; `ui_popup menu:<name>` opens it with that menu inside).

**Major tabs.** A custom strip, not docking, so tabs cannot be torn off.
- A tab shows an icon, the name without its own extension, and one slot: the dirty dot, or an x on hover and on the active tab. The slot takes room only when it holds something; otherwise the name takes it too.
- The active tab uses the base fill with a 2 pt accent.bright overline, and keeps its whole name (up to 220) while the others give way.
- Tabs shrink to 96; the rest go under ⌄, which then reads "+N". Names that would still cut to the same text are cut in the middle.
- Middle-click closes and dragging reorders.
- Right-click: Close, Close Others, Close All, Close Saved, Copy Path, Show in Content Browser, Show in Explorer.
- Level is pinned, shows the scene name and a warning dot while dirty.

**Documents**
- Per file: `quest:<rel>`, `beh:<rel>`.
- Singletons that switch to a new target, asking first if dirty: `pda`, `model`, `texture`.
- Singletons: `characters`, `mod_settings`, `world_rules`, `releases`, `preferences`, `light_animations`, `texture_library`, `sound_library`.
- Everything else stays a dialog: Package, Publish, Build to Folder, Export Flat, Submit, Import Base Scene, pickers, confirms, results, the Palette.

**Hosting:** `gz-app` (`shell.rs`, `doc.rs`, `tabs.rs`, `chrome.rs`).

A major tab is a `Document` (`gz-app`'s `doc.rs`): `id`, `kind`, `label`, `icon`, `path`, `tooltip`, `closable`, `dirty`, `missing`, `padded`, `alive`, `body`, `status`, `menus`, `shortcut`, `edit`, `save`, `discard`, `on_activate`, `on_deactivate`, `on_close`, `inactive`, `restore_arg`, and for MCP `mcp_state`, `panel_state`, `panel`.

API:
- `Open(desc, activate)`, `Activate(id)`, `Active()`, `IsActive(id)`, `IsOpen(id)` (a loaded document; a restored placeholder is not).
- `RequestClose(id)` for the UI; it asks when unsaved.
- `Close(id, IfClean|SaveFirst|Discard, err)`, `SaveTab(id)`, `DiscardTab(id)` for MCP.
- `List()`, `Info(id)`, `Cycle(step)`, `RegisterFactory(kind, fn)`, `SaveActive()`, `Edit(action, run)`, `DrawDocMenus()`, `RequestExit()`, `FlushSession()`, `CloseProjectTabs()`.
- The frame: `FrameHooks` (caption, project toolbar, status right, log state, busy card, palette, quit, F11, session file, reveal in browser, the play-in-editor lock), `BeginFrame`, `BeginCaption` / `EndCaption`, `DrawTabStrip`, `DrawToolbar`, `DrawStatusBar`, `DrawWorkArea`, `EndFrame`, `DrawBusyFrame`, `WorkRect`.

- Activation lands in the same frame. It replaces the `m_Focus = 3` retries and `DockNextWindowWith("Render")`.
- The active document draws in `###doc.host` at `WorkPos`/`WorkSize` (NoDecoration, NoDocking, NoSavedSettings, main viewport). Inactive documents are not submitted; their `inactive()` hook runs instead.
- The PDA's file poll, the browser watcher and validation run off the frame.
- Tabs persist per project in `<project>\temp\session.ltx` (`tabN = id|kind|icon|label|arg`, the arg a project-relative path or `restore()`) and load lazily: a restored tab is a placeholder (`loaded:false`) until it is activated and its kind's factory opens it. A tab whose file is gone shows "File not found"; MCP reports it as `missing`. A close inside a frame frees the document at the next `BeginFrame`.
- Switch Project and `project_close` settle the dirty tabs first, write the session and close the project's tabs; they come back when it opens again.

**Level workspace**
- The Level toolbar sits over `DockSpace(ImHashStr("GrizzlyLevel_v2"), NoWindowMenuButton, &levelClass)`.
- `Viewport###Render` is the central node, with NoTabBar and NoDockingOverMe.
- The ids stay the same behind the new labels: `Tools###LeftBar`, `World Outliner`, `Details###Properties`, `Level Settings###World Properties`, `Content Browser`, `Output Log###Log`.
- Panel tabs have no ▼ button, show their close x on hover, and sit on a 26 pt strip.
- Raising a panel also selects its tab in its dock slot, so a panel raised over MCP stays raised when the tab bar is rebuilt.
- When Level is inactive:
  - its dock keeps its state;
  - panels are not submitted and the scene is not rendered;
  - the viewport keeps its size, and `screenshot_viewport` gets a pass of its own.
- PIE activates Level and locks the tabs.
- The layout lives in the layout store (`level.*` keys of `grizzly_editor.ltx`); nothing is migrated from the C++ editor's `level_imgui.ini` (D3).

| Class | When | Layout | Viewport |
|---|---|---|---|
| Wide | W ≥ 1400 and H ≥ 640 | Tools on the left, 260. Right column 340: Outliner (40%) above Details + Level Settings. Bottom strip 240, under the viewport only: Content Browser + Output Log | 1103x578 at 1707x912 (today ~850x450) |
| Compact | otherwise | Tools 240, right column 300. Browser and log live only in the drawers | 736x628 at 1280x720 |

- Side nodes keep their pt size; the centre absorbs every change (rule 1.2).
- Crossing a size class rebuilds the layout, unless the user customized it. Customized means the node signature (splits and sizes rounded to 8 pt) differs from the built one; the flag is `[ui] level_layout_custom`.
- A customized layout shrinks its side nodes to their minimums first. Below that, panels scroll.
- Reset Layout rebuilds for the current size class. F11 hides the panels.

**The viewport's mouse** (`gz-ui-viewport`'s `pane.rs`, `camera.rs`), as in Unreal:
- A left click selects (or places, with the Place action) when the button comes up, and only when the pointer moved less than 4 pt since the press. A left drag past that moves the camera (the vertical flies forward and back, the horizontal turns) and selects nothing; Ctrl+drag draws the marquee. A press on a gizmo handle takes the selection at once, and so do the tools' strokes (the AI map's brush, the Sector tool's paint and Box Pick).
- While the camera holds the mouse - RMB look and fly, a left drag, MMB or LMB+RMB pan, Alt+LMB orbit, Alt+RMB dolly - the cursor is hidden and held where the press was: the camera turns without end, at no screen edge. It comes back there when the buttons are up. The camera reads the mouse's own motion (raw input), not the cursor's place.

**Level toolbar.** The groups, lowest priority first:
- Save | Undo, Redo
- Build Collision, Build AI Map, Build Game Graph, Build All
- Play ▾, Play in Game, Play Clean, Stop
- Reload Configs, Game Folder

New and Open move to the File menu (#29).

**Status bar.** Drawn in chrome, including by `DrawProgressOnly`.
- **Left:** [Content Drawer], and [Output Log] with the last line worth reading: an editor report or a counted error or warning - never render chatter, and never a job's start, stage or finish line (those are notes in the log only). MCP `progress` answers the line as `status_line`.
- **Centre:** `status()` (Level: selection count, grid, angle).
- **Right:** the "Rebuild: Collision · AI Map · Game Graph" chip with a [Build] per item (#30), Tasks, error and warning counts, and the MCP dot.
- The counts are the Output Log's own Warnings / Errors: counted since the project opened (the start before it), each distinct line once - a repeated one does not count again. Notes the author need not act on (graph canvas notes, run command lines) are `*` notes, not `~` warnings. The log marks with a rule where the count starts, a chip opens the log on its lines and a second click closes it.
- Popovers opened from the bar (Rebuild, Tasks) stand above their chip and have the theme's padding and spacing.

**Drawers.** The Content (Ctrl+Space) and Log (Ctrl+\`) drawers cover the bottom of any tab, one at a time. A drawer shows the same object as its panel; the visible host calls `DrawBody()` - one host per frame: on Level with the panel on screen, a drawer request raises the panel instead. Esc, a click outside, a drag that leaves the drawer, or another tab coming up closes it. Its cross takes no keyboard focus.

**Tasks.** The Task Center is a status chip and a card (`Tasks###tasks_card`, 400 pt) in the bottom right corner of the window, over the work and in nobody's way: no dim, no focus taken. Each row shows the title, what it is about, its steps (`Check` - `Copy` - `Install`, the current one ringed), a 6 pt bar that is filled as far as the job has come or crossed by a light while nobody can tell how long it takes, the stage, the elapsed time and what is left, bytes and files, the file at hand, and [Stop <job>]. A chevron hides the card; the chip shows or hides it, and a click on the button that started the job brings it back. Toasts stand on top of it. A modal up hides it for as long as it is up.
- A build, an export, a packaging, a publish and every Mod Browser job run beside the editor. Nothing of them holds a frame: the file work has a thread of its own (the job's thread touches nothing of the editor and sends its log lines to the main thread), and what has to read the editor's own state - the scene's bake, the checks of quests, world rules, PDA, screens and characters - reads the snapshots of the stores off the UI thread.
- UI tasks open the card. MCP tasks never open it or any dialog (`ui_popup show=tasks` puts it up to look at).
- A job that runs beside the editor adds its rows to the Task Center (`gz-app`'s `tasks.rs`); MCP `progress` lists them under `background`, with `steps`, `step`, `numbers`, `file`, `eta_ms`. A packaging and the upload that follows it share one row key: the card goes on from one to the other.
- A build answers an MCP caller when it has ended (`FollowsJob`: the handler says `"job_pending":true`, the pump holds the request and asks again with `answer_only`), and every other command goes on meanwhile. `progress action=cancel` stops the jobs beside the editor when no blocking job runs.
- During a blocking job, `DrawProgressOnly` draws a busy frame: the caption, the status bar, and a dimmed work area with a task card.
- `PumpProgressRequests` admits `screenshot_editor`.
- **A job's progress draws at a rate of its own and wakes no scene.** The status chip redraws ten times a second, the open card twenty, a spinner thirty (`gz-ui`'s `SPINNER_FRAME`); a frame egui asks for later is drawn then, never at once (`gz-app`'s `window.rs`). The Level viewport builds a scene pass only when something it shows changed - the camera, the scene's snapshot, the selection, the view options, the pane's size or scale, the theme, an asset that arrived, a gesture, something animating in view (`gz-ui-viewport`'s `service.rs`, `due`); a UI frame that changes none of it shows the picture the pane has. `frame_pacing` counts the two apart (`ui_frames`, `ui_frame_causes`, `scene_passes`, `scene_pass_reasons`).

**Floating windows.**
- `ViewportsEnable` is off unless the preference "Allow Panels Outside the Window" (General; `[ui] floating_panels`) is set, which takes a restart. With it off, `outside` is empty.
- Floating Level panels draw only on Level and are clamped inside the window (`ClampNextFloatingWindow`).
- A modal is drawn in the main window, recentred when the work area changes, and never sized past the work area (a taller one scrolls).
- A minimized editor asked for a picture draws its frame as if restored (`gz-app`'s `window.rs`).

**Keys**, first taker wins:
1. A text field.
2. The shell: Ctrl+Shift+P, Ctrl+(Shift+)Tab, Ctrl+F4, Ctrl+Space, Ctrl+\`, Ctrl+S for the active tab, F11 on Level (`gz-app`'s `keys.rs`). None of them acts while a modal is up. A colliding scene binding is logged once and ignored.
3. The document's `shortcut()`.
4. The scene's bindings, only when `scene_keys_allowed` (`gz-app`'s `shell.rs`): Level active, no popup or dialog open, and the focused panel not one that owns its keys (the Content Browser, the Output Log, the drawers).

`BlockShortCuts` is deleted, so Delete, Ctrl+D, Ctrl+C and Ctrl+S can never reach the scene from a document. F11 is the shell's fixed chord, not a rebindable command.

## 6. Surfaces

**World Outliner (#18)**
- Toolbar: SearchField; Filters with a badge (Selected Only, visibility, classes with counts, sort); ⋯ More (By Type/Hierarchy, Show All, Show/Hide Selected).
- Columns: eye, then Name with icon and ellipsis. Type and Owner are hidden, back from the header's right-click menu. The header does not sort: sorting is the Filters menu's. Class rows show an eye and a count.
- Empty states: "The scene is empty", "Nothing matches" [Clear Filters]. Footer: "2 objects · 1 selected".
- Rename is `dlg.rename_object`; Delete asks through `dlg.delete_object`, and the row menu's Delete... is in the error colour.
- Object List is retired (#17). `COMMAND_SHOW_OBJECTLIST` (Scene > Find in World Outliner) now focuses the search.

**Tools (#19)**
- Tiles in 1-3 columns, each [icon][label][eye] with the eye always shown. The active tool is filled with accent. A tile shows the tool's title or a short form of it; its header and the Command Palette say "<title> Tool".
- A hidden class shows eye-off and toggles with one click, sharing the flag with the Outliner.
- A sticky "<Tool> Tool" header sits over the panel's own scroll.
- A tool with a list (Object, Spawn, Particles) draws it first: a SearchField, then the list filling what the body leaves under it; the search opens every folder holding a match, with no flat result list. With less than four rows of room the list flows in the body's scroll: one scroll at a time, never a list scrolling inside a scrolling body. The tool's other sections start closed.
- Renamed controls:
  - `=% +% + -` become Select Same, Deselect Same, Select % of Same.
  - `M+/M-` becomes Segmented.
  - The AI Map D-pad becomes icon buttons.
- Snap Objects is a closed Section. "Are you sure" prompts become `Confirm`.
- Detail Objects is the L dialog `Detail Objects###dlg.detail_objects`: Objects (Add... / Remove / Clear All... / Import... / Export..., the list, a thumbnail and the settings grid) and Colour Indices (Add Index / Clear All..., one block per colour with Add / Take Out / Remove).

**Details and Level Settings (#5, #21, #35)**
- Header: icon plus name. Search: "Search 30 properties", plus a menu with Expand/Collapse All.
- Grids: role `selection` or `world`, with a help footer.
- In Level Settings, ranges move into help, Quality becomes Segmented, and choose rows become PickFields.
- `target=world` raises Level Settings.

**Content Browser (#20)**
- Toolbar: [+ Add ▾] (off, saying why, outside the open project's own source) | Back Forward Up | breadcrumb, whose leading parts fold into "…" | SearchField | View ▾ | ☆ | Favorites ▾.
- Add: New Folder | Quest, Behaviour Graph, Character, PDA Interface; the grid's empty-space menu has it too.
- Sources pane (220, hidden under 480): Segmented [Project | Editor | Game], Favorites, tree. When it cannot hold six tree rows under the section headers, the headers go and the favorites (if any) sit above the tree. Its width is the Layout key `content_browser.sources`.
- View As: Tiles or List, per host. Tiles are 48-192 pt, with vector icons; the docked panel and the drawer keep a size each. Until the author picks one, a host fits its tiles: the largest size up to its default (panel 64, drawer 96) at which the grid shows two rows, in 8 pt steps, 48 at least - 48 in the Wide strip, which holds under two rows even of those. View > Tile Size > Fit to Height turns fitting back on.
  - The list is one 22 pt line per entry (an 18 pt picture or a 16 pt glyph, the name cut in the middle with its extension kept) in columns of at least 12 em. The docked panel starts as a list, since the Wide strip holds a row and a half of tiles; the drawer starts as tiles. Ctrl+wheel under the smallest tile goes to the list.
  - There is no text in the square and no ".." tile. Folders and files without a picture draw a vector glyph (`glyphs` over MCP).
  - A picture with transparency lies on a checkerboard and is made for the size it is drawn at (the file's own mip, then an alpha-weighted average); the tooltip names the file's size and its transparency.
  - Captions take 2 lines and never break right before the extension; a last line that does not fit keeps the extension and loses the middle of the name (reported as `truncated`).
  - Tiles and the marquee share one selection look.
  - The rename box is up to 24 em, with a 512-byte buffer.
- A status strip counts the folder shown and the selected entries (a folder picked in the tree is not one). With no game linked, an EmptyState offers [Link Game...].

**Viewport (#10, #11, #30)**
- The viewport runs edge to edge, with pills on top.
  - Left: [View Options ▾], [Perspective ▾] (views, Frame All, Frame Selection), [Show ▾] (Stats, Grid, Safe Rect).
  - Right: Segmented Select/Place/Move/Rotate/Scale, Object Snap, and the grid/scale/angle snaps with `%g` values; their steps are the preferences Tools > Snap > Grid / Angle / Scale Step (one value each, saved).
  - Every menu item with a command shows the keys bound to it now; Maximize Viewport shows F11.
- A 64 pt clickable axis gizmo sits at the top right; it hides under 480. It is the only orientation widget: nothing is drawn into the render target for it.
- Stats are off by default and appear as a Mono pill; nothing is printed into the render target (the Rebuild chip says what NEED REBUILD said).
- **Game Lighting ▸** (View Options, after Lighting): Enabled, the error line when the weather cannot be read, Weather ▸ (Default - a new game's first night - then every `environment\weathers` file of the linked game), Hour ▸ (18:00 first), Exposure ▸ (Game Tonemap, x1 ... x4), Indoor Sky Light ▸ (0 ... 100 %), Rebake Roof Map, and a disabled line "N lights, <weather> at hh:mm". It lights the scene pass per pixel the way the game does (`EGameLighting`, `GameLightingScene`, shaders `editor\common.h` `glit_light`); the background becomes the weather's fog colour. MCP `game_lighting`, `view_mode game_lighting=` / `preset=game`.
- The render target grows in 64 px buckets (recreated only when the view outgrows it or shrinks two buckets below) and the view is its top-left corner; the render size stays exact. Whatever reads the target back crops to the render size (`screenshot_viewport`). A splitter drag no longer recreates the targets.

**Output Log (#16)**
- Docked in the Wide layout; a drawer otherwise.
- Toolbar: search, All / Warnings / Errors with counts (the status bar's counts), Auto-scroll, Copy, Clear.
- Mono lines, virtualized, with a console field below. A muted row with a rule marks where the count starts ("The project opened - warnings and errors are counted from here"). The Warnings / Errors filter lists each distinct line once. The view opens at its end whenever it comes up (panel tab picked, drawer opened).

**Command Palette (#26)**
- Top-centred, min(720, 70% W) wide; its height follows the results, up to 480, and the field never moves.
- Each row shows an icon, name, path and keycaps. The id appears only in the tooltip.
- Ranking prefers word-prefix matches; letters in order must each start a word or follow the one before. Only what a row shows is matched - label, path, keys -, never the COMMAND_* id. Panels, drawers and open tabs are commands too.

**Quest and Behaviour graph (#2-#4, #12-#15)**
- Toolbar: Save, Reload | Undo, Redo | Validate (badge), Auto Layout, Frame All | Back, Forward | Find (F3), Find in Project | Bookmarks ▾, Previous, Next | Spring | Zoom ▾, Minimap, Shortcuts ?, Graph panel, Details panel.
- Graph panel, 280, always present: header fields, Tasks or Roles, and Variables under a splitter (`vars_split`) - the graph's own, then **Global**: the module's global variables (mod.ltx [vars]) in the same list-and-form, Add Global, written into the manifest once an edit is let go of. It becomes a rail when the canvas would drop below 400; on a rail the toolbar's button opens it over the canvas, and hiding folds it back to the rail.
- Details, 360: the Node section - its actions are list rows (icon, name, `On enter N` at the right, the whole row one target; a newly selected one is scrolled into view) - then the selected action under a splitter (`action_split`). Kind descriptions become tooltips and the help footer, which removes the ~30 `Hint` paragraphs.
- The sizes are Layout keys: `graph.panel_w`, `graph.details_w`, `graph.drawer_h`, `graph.vars_split`, `graph.action_split`, and the grids `grid.graph_*`. Nothing is migrated from the C++ editor's files (D3).
- Bottom drawer: Problems (n) and Find Results. It opens when Validate finds errors or warnings.
- An external change shows a Banner with [Reload] and [Overwrite...].
- Canvas:
  - scale is `kZoom[level] x Dpi()`, applied once; the levels are `{0.2, 0.3, 0.4, 0.5, 0.65, 0.75, 1.0, 1.25, 1.5, 2.0}` (index 6 = 1:1);
  - text is 16/15/14, with glyph px rounded per zoom level;
  - level of detail: full detail for titles of 10 px and up, titles only at 7-10, blocks below;
  - Frame All fits the graph's extent (nodes and back-edge channels) at the highest level up to 1:1, never below 0.75 (zoom index 5: a title is then 12 px at 100%); a graph that doesn't fit is framed AT 0.75 from its start - the trigger plus two levels in view at the top, the rest of the view filled with graph - and the status says so.
- Node, 240 wide and clipped:
  - a 28 pt header with family strip, icon, title, muted id and badges;
  - up to 3 "key: value" lines;
  - up to 4 action rows, then "+N more";
  - pin labels inside the node.
- `gz-nq`'s `layout.rs` (`node_box`) computes the box for the layout, drawing, hit tests, minimap and `quest_layout`.
- The minimap is 180x120 and hidden under 12 nodes; without it a pill at the middle of each canvas edge counts the nodes wholly beyond it ("2 more") and a click brings the nearest in. Behaviour graphs never say "quest" (`gz-nq`'s `text.rs`).

**PDA Interface (#24)**
- Toolbar: Save, Revert, Reload | Aspect, Language, Zoom (Fit/100%) as a segmented group | Reset State | Source, Details.
- The canvas is centred; it pans (right or middle drag) and zooms at the cursor (Ctrl+wheel always; over a PDA list or the map the wheel is the game's), F fits, and Fit fits both axes with 16 pt round the device. Every side of the device that lies on the game's screen edge gets that edge drawn.
- Details (320) has the file with Use in Module, then tabs: Problems first, Pages (with the keys), Vars, Log.
- Source is a bottom split: Mono, gutter, the reader's error line, Apply (Ctrl+Enter).
- Unsaved text asks before Revert (naming the file), close or retarget (Save / Don't Save / Cancel).

**Model Preview (#25) and Texture Preview (new)**
- Model: Frame, Reset View | Wireframe, Bones | Segmented Studio/Unlit | Details (320). Studio is a headlight along the view plus a key from above over an ambient. The view is drawn opaque (the target's alpha is a texture's gloss mask, not coverage). The model stands on a floor grid, and a visual only the linked game holds is drawn with the game's textures (`textures[].from`).
- Texture: Fit, 1:1, Zoom ▾ | Segmented RGBA/R/G/B/A | Mip ▾ | Details. It pans and zooms at the cursor, and the checker is clipped to the view. Details name where the texture lives (`origin`), its container and format, and whether it is shown decoded.

**Characters (#22)**
- List (260) with search, dirty dots and Template badges, then a splitter.
- Form, up to 760: Identity, Name & Bio (a row per language), Appearance, Profile, Dialogs, a Starting Kit table, and Trade.
- Trade (badge "Trader" while the character keeps a shop): not a trader - a line saying so or naming the base's shop, Make a Trader (Change Trade over a base's), Not a Trader; a trader - Sells and Buys (a combo of ticks over the linked game's trade categories, a category it does not list stays ticked and says so), Markup, Pays, Restock, then a Stock table (Item | Count | Chance | remove), Add Item, Not a Trader.
- Money: from / to fields, empty while the character sets none (the base's amount as the hint), Never runs out beside them, a Clear icon to unset.
- Problems show as sentences under their fields; the codes stay in MCP and the build. Delete... is GhostDanger and asks through `dlg.delete_character`.
- Unsaved edits ask before any switch. Validation runs on change.

**Screens & Music** (`screens`, Mod > Screens & Music, MCP `screens_get` / `screens_set` / `screens_view`). Saves to mod.ltx as it is edited, like World Rules.
- Toolbar: `Loading | Menu` (which surface the tab edits), then `Screens [Mix In | Only Mine]` and `Music [Mix In | Only Mine]` for that surface, then the status.
- Left (260 pt): Screens (rows of that surface) and Music, each with a + at its header; under Music `Repeat | Next Track`. A row's dot is a problem of it; its hover action removes it.
- Right (a column of up to 640 pt): for a screen a 16:9 preview (pictures as they are, the rest marked where it goes), Layers, Music, and More folded (Name, Show On, Levels, Show Over It, Weight). For a track: File, Shuffle On, Volume, Weight.
- Layers: as many as the author adds, the top row is the top layer. + adds an empty one, the row's cross removes it, a row is dragged to reorder. A click opens that layer's File, Fit, Opacity and folded Position; a shader adds Texture, a video Its Sound when the screen plays it.
- Files arrive by drag and drop, from Explorer or the Content Browser (the tab's drop zones, `gz-ui-pda`'s `drop.rs`; a tile lights up what is under it, a file from Explorer only lands; inner zones are asked first): on a layer it replaces its file, on Layers it adds layers, on Screens or the empty tab each becomes a new screen, on Music tracks; an `.ogg` is a track wherever it lands. Files from outside are copied into `screens\`, a PNG/JPG/TGA/BMP becomes a `.dds`, a `.ps` gets its `.s`.

**World Rules (#6, #23)**
- Toolbar: Reload Game Data, Empty World... (confirms through `dlg.empty_world`). The status reads "Saved to mod.ltx as you edit".
- One scroll up to 900: Original Game (Rule | Keep | Player Decides), Applies On (chips), Playable Area, Transitions (a filterable Transition | Leads To | Closed table), Player Start (Place in Scene).
- Problems go in a drawer.

**Mod Settings (#9)**
- Nav, 200, one `NavRow` per page with its icon and a problem dot: General, Mod Browser, Game Mode, Relations, Build Contents, PDA, Commands.
- Form, up to 44 em, with validation under each field. The mod name and the mode title stay separate.
- Mod Browser: Picture (the picture, its state and Choose Picture... / Remove beside it) and Tags (the fixed vocabulary as checkboxes) are rows of the same label-left grid.
- Relations: four rows of the same grid (Requires, Load After, Load Before, Conflicts), each a chip list with Add Module...: a searchable list of the modules installed in the linked game (`modules\` and `mods\`), and a typed id for one that is not installed.
- Build Contents: Not Shipped, a block of paths and patterns.
- Commands: a `PropertySection` per console command of `[commands]` (its name, a `cheat` badge) with Name (Delete as its trailing icon), Function (the chevron opens a searchable list of the project's `<script>.<function>`, Refresh reads the scripts again), Help, Arguments and Cheat; Add Command under them. The scripts and the linked game's modules are read when the page is picked, never per frame.
- Sticky footer: what stops the save on the left, [Revert] [Save] on the right. File counts are taken on open, never per frame.

**Build dialogs**
- **Build to Folder and Export Flat (M):** a PathField, and Writes To: the folder the build fills, read from its end (`<folder>\modules\<id>`, `<folder>\gamedata_<id>`). The native "files will be DELETED" box (default Yes) becomes a two-phase question: the build refuses, then `dlg.clean_target` names up to ten of the files it would delete, Cancel the default, and Yes re-runs it with `clean`.
- **Package Release (M, #32):** Output Folder, Requires Game, and the Author Key row (read-only unless the key asks for its password), sized in em. Replacing an earlier package of the version asks through `dlg.replace_release`.
- **Result:** the kit's `Result` (section 4).

**Publish Release (L, #31)**
- The Module section (Name, Repository, Version, Requires Game, Author Key) is read-only: Mod Settings edits the first ones, Requires Game is the linked game's version (MCP `requires_game=` names another), a locked key asks for its password in its row. The dialog no longer writes the repository or the version into mod.ltx.
- It reads the repository's releases as it opens (Publish is off meanwhile): a version already out - `v<version>` or `v<version>-rN` published - is a problem before the click, with [Open Mod Settings] and [Open Releases]. A read that fails is a muted note: GitHub checks again at publishing.
- Every problem is listed at its top.
- The device code is shown in Title Mono with Copy.
- It closes when the job starts; the progress is the Task Center's card ([Stop Publishing]) from the packaging through the upload, and a click on the toolbar's Publish button brings it back; the result is the kit's `Result`.

**Releases (#33)**
- A sortable table: Tag (fixed at the widest tag) | Title | State ("Published" in secondary text for one that is not the latest) | Published (local) | Files ("3 files · 15 KB"), column lines in the header only, plus Details of the selected release (`mod_releases action=select`).
- Edit... (`dlg.edit_release`) and Delete... (GhostDanger in the details toolbar, the row's context menu in the error colour, Del). Delete names the release, its files and tag, and what players get instead.
- A failure is a banner sentence naming what failed.

**Submit to Mod Browser (L modal, #8, #27)**
- Id `###dlg.catalog_submit`. Fixed height; the body scrolls.
- Cards (`BeginCard`):
  - Author Key.
  - Requirements, failures first, the met ones folded under "Met (n)", VirusTotal findings left out.
  - "VirusTotal (required)", in the Required tone and never grey. This is the only place the verdict appears. [Scan] [Open Report] [Forget VirusTotal Key]; Open Report is on only while a package has a finished report, and Forget never touches the author key. While a check has not looked the packages up yet, the card says "Looking the packages up on VirusTotal..." instead of claiming they are not scanned.
  - Submission: the verdict, and "Submitting opens a public issue at github.com/MMadmer/darf-mods from <login>." "Not submitted yet." shows only once GitHub answered; until then it looks the submission up.
  - Withdrawal: Withdraw... (GhostDanger) raises the sheet `Withdraw from Mod Browser###dlg.catalog_withdraw`, the Danger Withdraw and Cancel the default.
- Every card button is Secondary.
- Footer: `FooterBusy` with the stage, [Stop <job>] (Stop Checking, Stop Scanning, Stop Submitting, ...) and [Hide] while a job runs; the submit button - Submit, Scan and Submit, Update Submission or Scan and Update, at the width of the widest - and [Close].

**Start page (#28)**
- A page filling the work area under the caption (`Start Page###start_page`, `BeginPage`), not a modal.
- Its status bar has no centre: the centre is a document's `status()`, and the page is no document (`ui_list` answers `status_centre: null` here, the rect of the Level tab's status on Level).
- Left, 320: New Project (Location, then Name with its rule as the hint, then [Create]), and [Open Folder...].
- Right: search, Segmented Last Opened / Name (kept in `_projects.ini [browser] sort`), and 232x184 cards.
  - Each card has its preview cropped to the card, the name on a row of its own with the ⋯ menu, and the path with a start ellipsis.
  - Duplicate names keep the parent folder that differs whole in the path line, in the text colour.
  - A Missing / Not a project badge.
  - The ⋯ menu (or a right click), with icons: Show in Explorer, Delete... in the error colour, Remove from List.
- The preview is a pass of its own (`gz-ui-level`'s `doc.rs`): the open scene rendered once more from the viewport's camera, without grid, pivot, selection, gizmo, axis triad, stats or captions. A picture of nothing keeps the last good one.
- Delete opens a top-level `Delete Project###dlg.delete_project` (M; Delete is Danger, Cancel the default); a project that opens under it takes it down.
- Link Game and Recovery are M cards on this page (`BeginPageCard`): Link Game [Close Project] [Link]; Recovery defaults to [Restore], [Discard...] confirms, naming the snapshot time and the older snapshots.
- Import Base Scene is an M dialog with search; it reads its list once.

**Preferences (#1)**
- Layout: search, a nav list with an icon per page (as Mod Settings), and the grid. A page is at most 44 em, a value at most 16 em.
- Keyboard is a Command | Shortcut | [Assign...] [Clear] table, grouped as the menus, under the official names (the palette's), with keycaps; the id is the tooltip.
- Settings apply live and are saved when a field loses focus and on close.
- Labels become official. `bp_cull` and `bp_depth_tolerance` get their own keys. Tools > Snap holds the viewport toolbar's steps, not settings of their own. Recovery reads Enabled, Interval, Delay After Input, Snapshots Kept (units in their help).

**Libraries**
- Light Animations, Texture Library and Sound Library become documents: list | preview or ImDrawList timeline | grid. An unsaved prompt replaces the destructor's MessageBox.
- The Texture Library reads the `.tga` sources of `$textures$` (rawdata\textures). Check New Textures turns a library tab without unsaved changes into New Textures (from `$import$`); the finished import brings the library back. Empty states name the folder or file each library reads.
- Detail Object List becomes an L dialog (`dlg.detail_objects`, above).
- Choose (`###dlg.choose`, L): search opens folders. Every list is picked, never typed (`gz-ui-level`'s `lists.rs`, read off the UI thread and kept for the session): a spawn's Model\Animation lists its model's motions (its own and those of the `.omf` files it names), Fixed bones and a light's bones its model's bones (Fixed bones takes several, the ones it has picked already), Motion the game's `anims\*.anm` / `*.anms`. An empty list says what is missing ("The model ... has no motions.", "The object has no model."). `property_choose` answers the same list (`list`, `total`, `items` with `search` / `limit`, `multi`, `empty`); `property_inspector` names a choose row's `list` and gives the object's own (`choices`).
- Edit Text drops its crashing Load/Save.

## 7. MCP

Rects in answers are px, with a `dpi` field. Inputs are pt. An opener activates its target in the same frame. State-only calls never switch the tab: `outliner_filter`, `property_inspector state`, `content_browser_selection get`, `pda_preview state`, and the data commands.

Frame-reading commands are held by `gz-app`'s `shots.rs` until the frames they ask for are presented (`screenshot_editor`, `screenshot_viewport`, `ui_list`, `ui_audit`, `titlebar_info`). Commands that change what is drawn and answer with where it landed run at once and, when their answer is `pending`, are held for those frames and read again with `answer_only` (`quest_view`, `pda_preview`, `preview_model`, `texture_preview`) - one call, raw socket included.

**New commands**

| Command | Behaviour |
|---|---|
| `ui_list` | tabs {id, kind, label, path, dirty, active, missing, closable, loaded}, panels {id, window, open, visible, docked, focused, closable, host, rect}, drawer, dialogs, popups, work_area, start_page, `status_centre` (the rect the active document's status drew in, null on the Start page), `toasts` (the notes up), size_class, layout_custom, viewport_maximized, `ui_scale` |
| `ui_tab action=list\|activate\|open\|close\|next\|prev id= [kind= path=] [save=]` | a dirty close without save answers `unsaved`; `kind` is any of `ui_contract` `document_kinds` |
| `ui_panel action=open\|close\|focus\|toggle id=` | `tools`, `world_outliner`, `details`, `level_settings`, `content_browser`, `output_log`, `viewport`; answers `host` |
| `ui_drawer action=open\|close id=content\|log` | |
| `ui_dialog action=list\|open\|close id=` | `package_release`, `publish_release`, `build_to_folder`, `export_flat`, `import_base_scene`, `submit`; close = Esc, and also takes a kit dialog's `dlg.<name>` |
| `ui_popup show=<id>\|none` | look only, one at a time: `menu:<name>`, `tab_menu:<tab>`, `tab_overflow`, `toolbar_overflow`, `toolbar_split:<id>`, `cb_context:<path>`, `cb_add` (refused where the browser cannot add), `cb_view`, `cb_favorites`, `cb_grid`, `outliner_filters`, `outliner_more`, `viewport:<menu>`, `tasks` (while a job runs), `rebuild` (while the scene needs it) |
| `ui_scroll id= to=top\|bottom\|<pt>\|item:<name>` | `item:` also reaches a row of a tool's list or a section (`ScrollTarget`) |
| `ui_split [key= value=]` | with no key, lists every key |
| `ui_audit [id=]` | truncated, escaped, controls (enabled, why, help) |
| `ui_scale [value=1.0..2.5]` | tests only; not saved |
| `ui_contract` | constants from `gz-app`'s `contract.rs`: tile range, defaults and error text, view defaults, theme keys, categories, card menu labels, panels, drawers, dialog ids and titles, kit dialogs, document kinds, popups, menus, viewport menus, the palette's id, the scale range |
| `window_control action=minimize\|maximize\|restore\|resize\|close` | `resize width= height=` in pt; `close [save=]` |
| `project_close [save=]` | goes to the Start page |
| `project_create name= [parent=]` | the Start page's New Project form |
| `preferences action=get\|set\|show key= value= [page= search=]` | the .ini names; `show` opens the Preferences tab on a page |
| `shortcuts action=list\|set\|clear` | the Keyboard page |
| `texture_preview action=open\|state\|close path= source=` | |
| `library kind= action=open\|state\|select` | the three library tabs |
| `mod_settings_view`, `characters_view`, `world_view` | open or report the tabs; `page=`, `select=`, `drawer=` |
| `property_set key= value= [target=]` | one row of Details or Level Settings, as editing it |
| `scene_visibility`, `viewport_controls`, `snap_objects` | the eyes, the viewport's right pills, the Tools panel's Snap Objects |

Discarding is possible only through bridge tools marked `destructiveHint`: `grizzly_ui_tab_discard`, `grizzly_window_close_discard`, `grizzly_project_close_discard`.

**Changed commands**

| Command | Change |
|---|---|
| `state` | + `active_tab`, `tabs`, `dialog`, `start_page`, `ui_scale` |
| `screenshot_editor` | + `active_tab`, `busy`, `dpi`, `modal.id`. Hover is suppressed. Works during blocking jobs. `kShotFrames` stays 3 |
| `screenshot_viewport` | forces a scene pass while Level is hidden; crops to the render size |
| `titlebar_info` | keeps `band`, `menus_end`, `buttons_x`; + `tabs`, `tab_row` (null on the Start page), `interactive`, `caption_h`, `folded`. Valid on every page |
| `reset_layout [scope=level\|all]` | rebuilds `GrizzlyLevel_v2` and reopens panels; `all` also resets Layout |
| `progress` | + `background` (jobs beside the editor), `errors`, `warnings`, `status_line` |
| `quest_*` openers | activate synchronously; `pending` is resolved by the pump before the reply |
| `quest_view` | `action_split` = the Details node/action split. `vars_split` = the Variables share. `keys` = the Shortcuts card. + `graph_panel`, `details`, `problems`, `graph_width`, `details_width`, `scroll=variables\|tasks\|task:<id>\|action:*` (an error when off screen). The reply + `panes` in pt, `beyond`, `title_px`; `zoom` = level + `ui_scale` |
| `quest_layout`, `quest_minimap` | content-sized `w`/`h` per node; `available`, `shown` from 12 nodes |
| `pda_preview` | `open` activates `pda`; + `pane=`, `tab=`, `zoom=`, `apply`/`save`/`revert`, `save=`/`discard=`; the answer + `text_dirty`, `panes`, `scale`, `device_gap`, `screen_edges`, `read_line` |
| `preview_model` | activates `model`, answers `tab`, `panes`, `textures` |
| `content_browser_open` and visual navigation | raise the docked panel on Level (also from behind the Output Log's tab), otherwise the Content Drawer. + `visible`, `host`, `view`, `tile_range`, `tile_unit`. `tile` stays 48..192 (now pt, the side drawn), with the same error text; + `tile_fit`, `view=tiles\|list`, `search`, `sources_pane`, `can_add`; `up`/`home` at the root succeed |
| `content_browser_selection` | + `glyphs`, `action=reveal` |
| `outliner_show`, `property_inspector open` | activate Level and raise the panel; `target=world` raises Level Settings. Rows + `label`, `help`, `unit`, `mixed`, in the grid's order |
| `mod_releases` | + `tab`, `selected`, `action=select`, `show=edit` |
| `mod_catalog` | `confirm=withdraw` with `show` shows the sheet; `submission.known` |
| `project_browser_menu` | `browser` = Start page; anchored at the card ⋯; the confirm is `###dlg.delete_project`; labels unchanged; + `search`, `sort`, `cards` |

Also changed:
- `outliner_filter` gets `layout` and `sort`.
- `view_mode` gets `stats`; `place_spawn` takes `env_mod` and `rpoint`.
- `command_palette` lists panels, drawers and open tabs besides the commands (`kind`, `label`, `id`; `execute id=panel:<id>|drawer:<id>|tab:<id>`).

Unchanged: `exec_command`, `list_commands`, `theme`, `viewport_navigation` (its description names the axis gizmo and the Perspective pill), and the data commands.

**Bridge and docs.** The bridge carries a tool per command (156). `initialize`, `PLAYER_SAFETY` and `WITHDRAW_SAFETY` stay verbatim. AI_SETUP.md, CLAUDE.md, NQ_ARCHITECTURE.md (§13.7-13.9), NQ_FORMAT.md and PDA_FORMAT.md follow this document.

## 8. Implementation

**Status.** Built twice: by the C++ editor (its phases 0-2, the nine packages A-I and their plan are the record at `cpp-editor-last:docs/UI_DESIGN.md`) and by the Rust editor, which carries every surface of this document and replaced the C++ one at the cutover (`docs/RUST_MIGRATION.md`, phase 7).

**Where it lives.**
- The kit: `gz-ui` (section 4). The shell, the frame, the keys, the screenshots and the constants QA pins: `gz-app` (`shell.rs`, `chrome.rs`, `keys.rs`, `shots.rs`, `contract.rs` - the ONE place for a constant `ui_contract` answers).
- One crate per surface, each with one `install(builder)`: `gz-ui-app` (Start page, Output Log, Preferences, the keyboard page), `gz-ui-content` (Content Browser), `gz-ui-level` (the Level tab's dock and panels), `gz-ui-viewport`, `gz-ui-graph` (quests, behaviour graphs), `gz-ui-previews` (model, texture, the libraries), `gz-ui-pda` (PDA preview, Screens & Music), `gz-ui-mod` (Mod Settings, the build dialogs), `gz-ui-world` (Characters, World Rules), `gz-ui-release` (Package Release, Publish Release, Releases, Submit).
- The surfaces' MCP commands are in each crate's `cmd.rs`, registered in the editor's command registry; `tools/rust/check.sh` holds the registry to the catalogue `crates/gz-mcp/tools.json`.

**Promoted** because two surfaces carried a copy: `gz-ui`'s `form` - `section` (a row-tall folding header: Preferences, the Start page, the PDA preview's Details), `raised_section` (a raised folding strip with a pill at its right: World Rules, the Sound Library), `check` (a box and its label: Mod Settings, World Rules, the Characters tab, the item picker), `hint_field` (a text field with a hint: the Start page, Mod Settings, Package Release, the Characters tab). A surface's `local_*` of the same name only names the id its state lives under.

**Still local**, each used by one crate (`local.rs`); a second user promotes one into `gz-ui`. Widgets that share a name and not a look are not copies: the Content Browser's section (28 pt, a muted count at its right) is not `form::section`; the switch-and-label checks of `gz-ui-graph` and `gz-ui-pda` are not `form::check`; the two checkers (the Content Browser's sized to the tile, the previews' lined up with the picture) and the two pick lists (the graph's picker body, Mod Settings' popup with Enter) differ in what they do.
- `gz-ui-app`: rule. `gz-ui-content`: tree row, section, breadcrumbs, rename box, checker, menu heading, marquee.
- `gz-ui-graph`: fold header, pane header, check, kind combo, pick list. `gz-ui-level`: eye toggle, button flow, panel header, tool tile, thumbnail, badge button, tab, tree list.
- `gz-ui-mod`: password field, value column, picture, pick list. `gz-ui-pda`: list header, tab strip, check, multi combo, code view, toggle row, drop mark.
- `gz-ui-previews`: checker, heading, frame, texture pick, wheel. `gz-ui-release`: secret field and row, code, card title, fold, table head.
- `gz-ui-world`: number text, cell check, dirty dot, problems drawer, level picker.

**Not built, because no surface asks for them** (proposals of the C++ redesign; one is built with the surface that first needs it): a badge on a toolbar action, trailing actions on a grid section, per-chip tone and tooltip, an ellipsizing toolbar status, tabs that always draw the selected overline, one shared "keys bound to a command" helper.

**QA of the UI** (each on a copy, in a home of its own): `tools/qa/test_ui_shell.py` (the frame over MCP), `test_ui_tour.py` (the 35 audit issues, `--before` for the "before" shots), `test_graph_ui.py`, `test_previews.py`, `test_properties.py`, beside every surface's suite; the raw-pixel lint is gone (D6). Tour steps T1-T22 are the `audit_live.json` tour, T23 the texture preview, T24 the libraries. A tour step passes at `ui_scale 1.5` / 1707x912 and at `ui_scale 1.0` / 1280x720: it opens and closes over `ui_*`, reports no `escaped` entries and an empty `outside`, keeps its footers and overflow reachable and resizes both ways.
