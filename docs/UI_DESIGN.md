# Grizzly Editor UI 2 - design specification

This document is the contract of the editor's UI. It is implemented (phases 0-2 of section 8, three visual tour rounds, QA green), and CLAUDE.md, AI_SETUP.md, NQ_ARCHITECTURE.md, NQ_FORMAT.md and PDA_FORMAT.md follow it. Where the implementation deliberately went another way, the text below says what was built. A UI change is a change of this document first; where it disagrees with another document, it wins.

- **pt** is a logical point. px = pt x `XUI::Dpi()`.
- **#n** is an issue in `audit_live.json`, in file order. **Tn** is a tour step (section 8).
- Targets are 1707x912 pt (2560x1440 at 150%) and 1280x720 pt, each at UI scale 1.0 and 1.5.

## 1. Principles

1. **Documents own the window.** Every asset editor is a full-window major tab. **Level** is tab 1 and cannot be closed; its centre holds only the viewport.
2. **The canvas absorbs change.** The viewport, graph, preview or tile grid takes every resize. Side panels, drawers and label columns keep their pt size. Below its minimum a container scrolls; it never clips.
3. **Nothing clips silently.** Text fits, ellipsizes with a tooltip, or wraps if it is prose. Rows overflow into `»`. `ui_audit` checks it.
4. **One size system, one kit.** Only `XUI::Pt`/`Em` are used, and DPI enters in one function. Raw px and colour literals fail `lint_ui_units.py`. Each job has one `XUI` widget.
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

**Applied (section 1a):** the graph toolbar keeps Save, Undo, Redo, the problems count when there are any, Frame All, Find, the panels and the zoom; the rest is its More menu. The Graph panel adds tasks and roles with + on the header and removes them with a cross on the row, ID and a role's details under More. Mod Settings and Characters save as they are edited. World Rules keeps Original Game, Player Start and Starting Kit on the surface, the rest under More, and shows a problems bar only when there are problems. Publish Release is one Release line and the Notes; Package Release is one click; Submit shows a working key as one line and one spinner per job. The Start page is the projects, its form opens on New Project.

## 2. Tokens

**Colours.** `GrizzlyTheme::ColorToken` keeps its 13 tokens and appends new ones. `U32(token, a)` and `Vec4(token, a)` replace the 5 `ThemeColor` copies and every colour literal. Existing values are in **bold**.

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
- **Chrome:** caption 32 (window buttons 46x32); major tab 28 tall, 96-220 wide; toolbar 36 (buttons 28, glyph 16, group gap 12); panel tab strip 26; panel toolbar 32; status bar 24.
- **Controls:** frame 24; row 22; property row 26; section header 28; inline icon button 24.
- **Dialogs:** S 400, M 560, L 760, XL min(1040, 90% W). Never wider than W-32 or taller than H-48. Title 36, footer 52, buttons at least 88x28.
- **Other:** tooltips and toasts up to 360; drawers 40% of H, clamped to 260-480; forms up to 760.

**Minimum sizes (pt).** Viewport 360x240. Outliner 220x160. Details 280x200. Tools 240x200. Content Browser 360x150. Output Log 320x110. Document side panel 220. Document canvas 400x260.

**Type.** One Segoe UI face covers Latin and Cyrillic.
- An ImGui size is the line box, and the Segoe em is 0.75 of it, so Body 16 is a 12 px em (Windows 9 pt). No text is smaller than 14.
- There is one `ImFont` per face; the size goes to `PushFont(font, size)`. Three files are read (segoeui, seguisb, CascadiaMono), and the startup profile times them as its `Loading fonts` stage; a frame drawn before that stage loads them itself.
- `XrUIFonts.cpp` (once `spectrum.cpp`) keeps the embedded Source Sans only as a fallback where Segoe UI is missing, with Cyrillic merged from tahoma or arial. `spectrum.*` and the dead style are gone.
- `GetFontSize()` is still 16 x dpi, so legacy px code scales as it did.

Type styles:
- **Body:** segoeui 16. The default.
- **Strong:** seguisb 16. Headers, node titles, Primary labels.
- **Small:** segoeui 14. Status bar, table headers, captions, hints, badges, keycaps, pins.
- **Title:** seguisb 18. Dialog titles, document titles, empty states.
- **Page:** seguisb 26. The Start page.
- **Mono:** CascadiaMono 15, falling back to consola. Log, source, stats.

**DPI in one place.** `XUI::SetDpi(s)` is the only writer. It:
- rebuilds the style from pt metrics and calls `ScaleAllSizes(s)`;
- sets `FontSizeBase` to 16 and `FontScaleDpi` to s;
- rescales every dockspace node, floating window and window setting by new/old, so side panels keep their pt size;
- recentres dialogs.

Inside a frame it waits for the next one. The layout store writes `[GrizzlyLayout][Dpi] Scale=`: a layout saved at another scale is rescaled once, when the ini is applied. `ScaleForDpi(dpi)` is the scale a monitor gets; `-uiscale <f>` wins.

It is called from:
- `Initialize`, with the HWND's DPI or `-uiscale`;
- `WM_DPICHANGED` in device.cpp, after `SetWindowPos` to the suggested rect;
- the progress pump (ui_main.cpp:1232), deferred until the pump exits;
- MCP `ui_scale`.

Everything else uses `Pt` for geometry and `Em(n)`, n x the font size, for text-bound widths. Canvases fold `Dpi()` into their world scale.

## 3. Iconography

Icons are vector only, in `XrEUI/XrUIIcons.{h,cpp}`, built with the EditorAssetIcons technique:
- a 16-unit grid with a 1-unit margin, built once and then only scaled;
- stroke 1.5 units, at least 1 px, pixel-snapped at 20 px and below;
- round joins, outlines only; fills only for play/stop, dots, the pupil and on-states.

`XUI::DrawIcon(ImDrawList*, Icon, ImVec2 min, float size_pt, ImU32)` draws them; `DrawIconRect` centres one in the largest square of a px rect; `IconName(Icon)` is its stable snake_case name (`chevron_down`), the one MCP answers and QA read. Sizes are 16 in toolbars, 14 in rows, 12 in the status bar, and 20-48 in empty states and tiles (a tile never draws its icon above 48).

How they draw (`XrUIIcons.cpp`): the geometry is built once, arcs tessellated then; at 20 px and below points snap to the pixel grid and a backtrack pass drops the spikes snapping would make; above 2 px of an opaque stroke, joins and caps are round; dashed paths are stretched to whole dashes. New glyph arcs start exactly where the previous line ends (within 0.02 units). ImGui's own triangles - combo buttons, tree nodes, submenus, sorted columns - draw the kit's chevron through `GImGuiRenderArrow` (`imgui_user.h`, one `Grizzly Editor:` block in `imgui_draw.cpp`). `-iconsheet [first]` opens a developer sheet of every glyph at 16, 24 and 48 pt (`XrUIIconSheet`).

Icon colours by state:
- rest: text.secondary
- hover: text
- checked: accent.bright on selection.focused
- disabled: text.disabled
- status: its own tone

`EditorAssetIcons` draws through this engine (`IconFor(kind)`, an `ImDrawList*` overload). Its `Name()` strings stay (QA). The `textures/ed/bar` bitmaps are gone; the 20 pt logo is the only bitmap left.

`XUI::Icon`:
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

The kit is `XrEUI/XrUIKit.{h,cpp}` (with `XrUIKitToolbar`, `XrUIKitPanels`, `XrUIKitDialogs` and the XrEUI-only `XrUIKitInternal.h` / `XrUIKitHooks.h`), namespace `XUI`, exported with `XREUI_API`. The header includes only `imgui.h` and `XrUIIcons.h`; `imgui_internal.h` stays inside XrEUI .cpp files. XrEUI's `stdafx.h` includes the kit, so every module's PCH carries it. `std::function` is safe across DLLs because they share one /MD CRT.

```cpp
using cstr = const char*; using Strings = xr_vector<xr_string>;
// scale, and who is asking
float Dpi(); float Pt(float); float Em(float n = 1); void SetDpi(float); float ScaleForDpi(unsigned dpi);
bool Unattended(); bool FromMcp(); enum class Answer { Yes, No, Cancel };
struct McpScope { explicit McpScope(Answer unsaved = Answer::Cancel); ~McpScope(); }; bool McpUnsaved(Strings* what = 0);
// type and text
enum class Font { Body, Strong, Small, Title, Page, Mono }; struct FontScope { FontScope(Font); ~FontScope(); };
ImFont* GetFont(Font); float FontSize(Font); bool LoadFonts();
enum class Tone { Muted, Info, Success, Warning, Error, Required };
enum class Kind { Secondary, Primary, Danger, Ghost, GhostDanger }; enum class Clip { End, Middle, Start };
ImU32 ToneColor(Tone, float a = 1);
bool TextEllipsis(cstr, float w = -1, Clip = Clip::End); void StatusText(Tone, cstr); void Prose(cstr);
bool TextAt(ImDrawList*, Font, ImVec2 pos, ImU32, cstr, float max_w = -1, Clip = Clip::End); float TextWidth(Font, cstr);
bool EllipsizeText(cstr, float w, Clip, xr_string& out); bool EllipsizeFace(ImFont*, float px, cstr, float w, Clip, xr_string& out);
void AuditTruncated(cstr full, cstr shown, ImVec2 min, ImVec2 max);
void Tip(cstr, cstr keys = 0); void SuppressHover(bool);
float Keycaps(ImDrawList*, ImVec2 pos, cstr keys, float h = 0, bool enabled = true);
bool MenuItem(cstr label, cstr keys = 0, bool selected = false, bool enabled = true);	// also (label, keys, bool*, enabled)
bool MenuItem(cstr label, Icon, cstr keys = 0, bool enabled = true, bool danger = false);
// buttons and fields
bool Button(cstr, Kind = Kind::Secondary, bool on = true, cstr why = 0, const cstr* label_set = 0, int n = 0);
bool IconButton(cstr id, Icon, cstr tip, bool checked = false, bool on = true);
bool SplitButton(cstr id, Icon, cstr label, cstr tip, Kind = Kind::Secondary);
bool Segmented(cstr id, int* cur, const cstr* labels, int n, const Icon* = 0);
bool SearchField(cstr id, char* buf, size_t n, cstr hint = "Search", float min_em = 8); bool Match(cstr query, cstr text);
bool PathField(cstr id, char* buf, size_t n, bool folder); bool BrowsePath(char* buf, size_t n, bool folder);
int  PickField(cstr id, cstr value, bool clearable);	// PickNone | PickBrowse | PickClear
bool Vec3Field(cstr id, float v[3], float speed); int Chips(cstr id, const Strings&, bool removable);
bool TextBlock(cstr label, cstr help, cstr id, char* buf, size_t n, int lines); void BlockLabel(cstr label, cstr help);
struct Toolbar { Toolbar(cstr id, int style = ToolbarRow /* Row Panel Overlay */); ~Toolbar();
  void Group(int priority = 0); void Separator(); void Spring(); void Status(cstr, Tone = Tone::Muted);
  bool Action(cstr id, Icon, cstr label, cstr tip, bool on = true, bool checked = false, Kind = Kind::Ghost);
  bool Split(cstr id, Icon, cstr label, cstr tip, bool* menu_open, bool on = true, Kind = Kind::Primary);
  bool Menu(cstr id, Icon, cstr label, cstr tip, cstr request = 0, bool on = true);
  bool Search(char*, size_t, cstr hint, float min_em = 10);
  bool Segmented(cstr id, cstr caption, int* cur, const cstr* labels, int n, cstr tip = 0, bool on = true);
  bool Widget(cstr id, float w_pt); };
// property grid
enum PropFlags { PropReadOnly = 1, PropRequired = 2, PropInvalid = 4, PropModified = 8, PropMixed = 16, PropInline = 32 };
bool BeginPropertyGrid(cstr role, float label_frac = .40f); void EndPropertyGrid();
bool PropertySection(cstr, bool open = true, cstr badge = 0); void EndPropertySection();
bool PropertyRow(cstr label, cstr help = 0, int flags = 0, int depth = 0); void PropertyText(cstr);
bool PropertyTrailing(cstr id, Icon, cstr tip); void PropertyMessage(Tone, cstr); void HelpFooter();
// sections, lists, tables, splitters
bool Section(cstr title, bool open = true, cstr right = 0); void EndSection();
bool BeginList(cstr id, ImVec2 size_pt); bool ListRow(cstr stable_id, bool sel); void EndList();
int  RowActions(const Icon*, const cstr* tips, int n);
struct NavRowSpec { Icon icon; cstr tip; bool dot; Tone tone; cstr count; bool dim; Icon action; cstr action_tip; };
bool NavRow(cstr stable_id, cstr label, bool selected, const NavRowSpec&, bool* action_clicked = 0);
void TableHeaders(u32 centred = 0);
bool Splitter(cstr key, bool vertical, float* size_pt, float min_a, float min_b, float def);
bool SplitterTail(cstr key, bool vertical, float* tail_pt, float min_head, float min_tail, float def_tail);
bool SplitterAt(cstr key, bool vertical, ImVec2 line, float length, float sign, float lo, float hi, float def);
namespace Layout { float Get(cstr, float def); void Set(cstr, float); bool Has(cstr); void ResetAll(); void Keys(Strings&); }
// states, cards, pages
bool EmptyState(Icon, cstr title, cstr text = 0, cstr action = 0);
void Badge(cstr, Tone = Tone::Muted); void StatusDot(Tone, cstr tip); void Spinner(); void BusyLine(cstr);
void Toast(Tone, cstr); int Banner(Tone, cstr text, cstr a1 = 0, cstr a2 = 0);
void BeginCard(cstr id, cstr title, Tone = Tone::Muted, bool attention = false, bool tone_title = false); void EndCard();
bool BeginPage(cstr title_id); void EndPage();
struct CardAction { cstr label; Kind kind; bool is_default; bool on; cstr why; };
void BeginPageCard(cstr id, cstr title, float width_pt); int CardFooter(cstr status, Tone, const CardAction*, int n); void EndPageCard();
// dialogs
enum class DialogSize { S, M, L, XL };
bool BeginDialog(cstr title_id, bool* open, DialogSize, bool resizable = false);
bool BeginSheet(cstr title_id, bool* open, DialogSize = DialogSize::M); void BeginDialogBody();
bool BeginDialogFooter(cstr status = 0, Tone = Tone::Muted); void FooterBusy(cstr stage); void EndDialog();
bool DialogButton(cstr, Kind, bool is_default = false, bool on = true, cstr why = 0);
struct ConfirmSpec { cstr id, title, headline; Strings items; u32 total; cstr consequence, action; bool destructive; };
void Confirm(const ConfirmSpec&, std::function<void(Answer)> then);
void UnsavedPrompt(cstr id, const Strings& what, std::function<void(Answer)> then);
void Notice(Tone, cstr title, cstr text);
void Result(cstr id, cstr title, Tone, cstr headline, const xr_vector<std::pair<xr_string, xr_string>>& rows,
            const Strings& left_out, const Strings& warnings, cstr action = 0, std::function<void()> on = {});
void OpenDialogs(Strings&); bool CloseDialog(cstr name);
// panels, surfaces, MCP
bool BeginPanel(cstr title_id, bool* open, int flags = 0); void EndPanel();
struct SurfaceDesc { cstr id, label; int kind; std::function<void(int op)> control; std::function<bool()> is_open; };
void RegisterSurface(const SurfaceDesc&); void Surfaces(Strings&); bool SurfaceControl(cstr id, int op);
bool SurfaceIsOpen(cstr id); bool SurfaceInfo(cstr id, xr_string* label, int* kind);
void RequestScroll(cstr surface, cstr target); void ScrollTarget(cstr name); cstr ScrollTargetWanted();
void ShowPopup(cstr name); xr_string ShortcutLabel(const xr_shortcut&);
void AuditSnapshot(xr_vector<AuditEntry>&, cstr window = 0); void AuditJson(xr_string&, cstr window = 0);
void DrawOverlays();
```

**What drawing code may use**
- The kit, the theme's tokens and nothing else: colours through `GrizzlyTheme::U32` / `Vec4(token, a)` or `XUI::ToneColor`, sizes through `Pt` / `Em`. `tools/qa/lint_ui_units.py` fails a raw px size (`colour`, `px` rules), a colour literal, and an `ImGui::MenuItem` with a shortcut outside the kit (`keys`); a line that really means it carries `// ui-units: ok <why>` (a texture's own pixel size, a channel tint).
- `-FLT_MIN` is +FLT_MAX in this code base (xrCore defines `FLT_MIN` as `-FLT_MAX`): `SetNextItemWidth(-FLT_MIN)` asks for an infinite width. Pass `ImGui::GetContentRegionAvail().x`; the lint flags it.
- A widget the kit lacks is written as `XUI_Local<Name>` in the package's own file and promoted into the kit once a second surface needs it (section 8 lists what is still local).

**Buttons**
- One Primary per surface.
- The Danger fill goes only on the destructive action. The button that opens the question ("Delete...", "Discard...", "Withdraw...") is `Kind::GhostDanger`: a Ghost with error-coloured text. A menu entry that leads to a deletion is `MenuItem(label, icon, keys, enabled, danger = true)`.
- Disabled: input fill, disabled text, and the reason (`why`) as the tooltip.
- A label that changes with the state passes every label it can show (`label_set`), so the button keeps the width of the widest.
- Row actions are `RowActions`: squares as tall as the row (22 pt) with 14 pt icons, shown on hover - not 24 pt, so they fit inside the row. `SmallButton` is never used.

**SearchField**
- A magnifier and a clear x. Esc clears; Ctrl+F focuses. Spaces separate terms that must all match, ignoring case (Cyrillic too, yo read as ye). It replaces `ImGuiTextFilter` and `strstr`.

**Toolbar**
- Never wraps. When too wide, labels collapse to icons first. Then items move into `»` (`toolbar_overflow`), rising priority first, rightmost first on a tie.
- It lays out from the last frame's measures. A folded label unfolds again only with 24 pt to spare, and a row keeps its layout while it still fits: a value that changes width never folds its neighbours. A value label keeps one width (the graph's zoom: a figure space leads a two-digit level).
- `Toolbar::Segmented` measures itself; in the overflow its choices become radio items under its caption. A `Widget` has no overflow form and is hidden there.
- `Split` is an action with a chevron menu (Build Mod); in the overflow it is the action plus "<label> Options". `Menu` takes the name `ui_popup` opens it by (`request`); off, it keeps its caret and width, is drawn disabled with its reason as the tip and never opens.
- A standalone `Segmented` becomes a combo when narrow.

**Menus.** Every item that shows a shortcut is `XUI::MenuItem`: keycaps flush right in their column, at least 24 pt after the longest label, showing what is bound NOW, never a literal.

**Property grid**
- Label column:
  - Ratio .40 (a role may start at another share: Level Settings .50), stored per role in `grid.<role>`, draggable within Em(6)..width-Em(8).
  - Below Em(18), the label moves above the value.
  - Labels are Title Case secondary text with an ellipsis (Output Folder, Requires Game, Module ID); hints, messages and prose are sentences. The tooltip shows label, help and key.
  - No bullets. Each level indents 12.
  - Categories are full-row headers that toggle on click. A level lists its own rows first, then its categories, each in the order given: a row after a category would read as one of its rows.
- Values:
  - Every value starts at the left of its field, numbers included (`GImGuiDragTextAlignX`, one `Grizzly Editor:` block in `DragScalar`).
  - `Drag*` without steppers. A number shows its unit after it (m, °, fps); typing takes the number.
  - A value that needs more than its column on one line (`Vec3Field` under Em(15), a `Segmented`, a row of buttons) takes the line under its label, as every row does below Em(18).
  - A switch (`PropInline`) stays on its label's line when labels go above values: the box, then the label.
  - Colours as a swatch plus hex.
  - `PickField`: leaf name with a start ellipsis, plus browse and clear.
  - Multi-line text: first line plus Edit, which opens an M dialog. Edit is offered only by a text that holds several lines or is an ini text (Custom Data, User Data); a name or a path has none.
  - A row keeps the height it had when it was last drawn, so a tall value no longer makes the rows under it jump as it scrolls away. Rows are never culled in a window that sizes itself to its content (AlwaysAutoResize / AutoResizeY): there a culled tall row grew one frame at a time. A value that still reads better as a block (a long text, a picture) is a `TextBlock` / `BlockLabel`: its label above it, as wide as the column.
  - Read-only: selectable secondary text; a path keeps its end (a start ellipsis, the whole path in the tooltip).
  - Differing values: "Multiple values".
  - Playback controls (First Frame ... Last Frame) are icon buttons named by their tooltips.
- Flags: Required shows a dot. Invalid shows a bar and a message. Modified shows a Reset icon. A message is a sentence for the author: problem codes (E103, W115) stay in MCP answers and the log.
- `HelpFooter` shows the hovered row's label and help, else the last one hovered while it is shown, else "Point at a property to see what it does."; one footer serves every grid of its panel. Text is drawn only with `TextUnformatted`.

**Lists and tables**
- Rows are 22 pt, with stable ids.
- `NavRow` is the row of a nav, a page list or a sources pane - icon, ellipsized label, a status dot or a count, one hover action - in or out of `BeginList` (Mod Settings, Preferences, the Content Browser's favorites).
- A one-column list has no header. Otherwise the headers are `TableHeaders`: Small secondary text, centred over checkbox columns; a sortable table still sorts.
- Row actions appear on hover.
- An empty list shows `EmptyState`, which fits the height it gets: in a short region it drops the icon first, then the margins.

**Splitters.** A 1 pt line with a 6 pt hit area, re-clamped every frame so it builds no debt. A double-click resets it. Its value is stored in the `[GrizzlyLayout][Data]` ini section (`ui_split`). `Splitter` sizes the pane before the line, `SplitterTail` the pane after it (a side or bottom panel that keeps its pt size while the other takes every resize, section 1.2), `SplitterAt` a pane laid out by hand (the graph tab's edges).

**Cards and pages.** `BeginCard` is a raised box as tall as what it holds, around one part of a surface, with a tone strip while it asks for attention (Submit's cards). `BeginPage` is a window over the whole work area under the caption (the Start page). `BeginPageCard` / `CardFooter` / `EndPageCard` is a centred card with a dialog-like footer (status left, buttons right, Enter presses the default) on a page that is not modal: Link Game and Recover Unsaved Scene ask their questions this way.

**Dialogs**
- Modal, `NoSavedSettings`, pinned. The id is `Title###dlg.<name>`, with no "Grizzly Editor - " prefix.
- Centred on open and whenever the work area changes; clamped instead if the user moved it.
- Height is at most H-48. The body scrolls, the footer stays put right under it, and the window itself never scrolls. A running job never resizes it.
- A dialog is measured on its first two frames; a measuring frame whose height is about to change is not shown, so the first frame anyone sees - a person, a screenshot, `ui_audit` - is the final layout. Keys never answer a dialog on a frame nobody saw. `kShotFrames` stays 3.
- Footer: status on the left; [Primary|Danger] [Cancel] on the right. While a job runs, `FooterBusy(stage)` puts the spinner and the stage on the left, and the job's stop is a labelled footer button, "Stop <job>" - never an icon-only square.
- Enter presses the default and Esc cancels. In a destructive dialog both cancel. The keyboard focus opens on the default; the title bar's cross takes none.
- A dialog raised from a dialog is a sheet (`BeginSheet`): under its parent's title, no wider than it, the parent dimmed (#34). A `Confirm` raised inside a dialog opens as one. A sheet the author or MCP explicitly put up shows whatever `-nodlg` says.
- `Confirm` lists up to 10 items, then "and N more", the counts, and "Deleted for good - this cannot be undone." in warning colour.
- `UnsavedPrompt` offers [Save] [Don't Save] [Cancel]. The exit prompt lists the scene and every dirty document, with [Save All].
- `Result` is the one result dialog of build, export, package, publish and bake (`dlg.build_result`, `dlg.export_result`, `dlg.package_result`, `dlg.bake_result`): a toned title, rows, Not Shipped, up to 8 warning rows, [Open Folder] [OK]. It opens only for runs started from the UI.
- The queued kit dialogs (`Confirm`, `UnsavedPrompt`, `Notice`, `Result`) are drawn at the end of the frame by `DrawOverlays`, nested above any modal open that frame, never in a busy frame; `then` runs outside every window.

**Precedence** (for kit questions and `ELog.DlgMsg`)
1. `FromMcp()`: the caller has decided. `GrizzlyMCP` runs every request inside an `McpScope`.
   - `Confirm` answers Yes, and the action runs before the reply.
   - Losing data needs `save=` or `discard=` (the command opens a nested `McpScope(Answer::Yes|No)`); otherwise `McpUnsaved()` makes it answer `{error:"unsaved", dirty}`.
   - `ELog.DlgMsg` never opens a box: it answers as under `-nodlg` (a confirmation No, a note OK), logged.
   - Under `-nodlg`, `window_control close` and `project_close` discard, as they always did.
2. `Unattended()` (`-nodlg`): the logged answers - a question answered No is `~ [dlg suppressed]`, a note `* `, only an error box `! `.
3. Otherwise the dialog opens, and `then` runs on a later frame.

`ELog.DlgMsg` itself:
- `mtError` and `mtInformation` become a queued `Notice` and return mrOK. They stay native before the UI's second frame and during blocking operations.
- A command that asks first goes through `ConfirmCommand(cmd, p1, p2, spec)` (UI_MainCommand.h): the kit's `Confirm`, and its Yes runs the command again with `kCommandConfirmed` in p2.
- Scene-IO confirmations stay synchronous under rules 1-2: SceneObjectIO, SpawnPoint, GroupObject, ESceneGroupTools, scene.cpp.
- `COMMAND_BREAK_LAST_OPERATION` no longer asks: a blocking job's frame draws no kit dialog, and the busy card's Cancel does not ask either.

**Truncation and tooltips**
- Truncation: names at the end, paths at the start, "title - id" in the middle.
- Prose wraps at 36 em (`Prose`, `StatusText`). Numbers are never cut. `ImGui::Text(non_literal)` is banned.
- Text painted by hand that was cut reports itself with `AuditTruncated`, so `ui_audit` sees it like the kit's own.
- Tooltips appear after 0.45 s, and never in the background or during a held shot (`SuppressHover` also sets `NoMouse`, so legacy tooltips stay away too).

**Audit**
- Kit widgets log `truncated`, `escaped` and `controls` (label, enabled, why, help).
- XrEUI also flags any window whose content exceeds its inner size on an axis that cannot scroll. It measures what THIS frame laid out, not ImGui's `ContentSize` (the frame before), and skips a child whose root window is hidden.

## 5. Frame

Chrome totals 92 pt:
- **Caption (32):** logo, menus, major tabs, ⌄, a drag band of at least 48, the project name (dropped first), – ▢ ✕.
- **Toolbar (36):** the tab's `toolbar()`, then Spring, then [Build Mod ▾ (Primary split)] [Publish Release] [Releases]. Publish Release keeps its label while a job runs: the state is in its tip, and the button stays lit meanwhile.
- **Status bar (24).**

**Caption**
- It is a `BeginMainMenuBar`, drawn in every state. The Start page, Link Game and Recovery show no menus (`EditorRecovery::OfferPending`).
- `UITitleBar` registers the menus, tabs, ⌄ and window buttons as HTCLIENT each frame; the rest is HTCAPTION. A frame that does not draw the caption invalidates the band.
- The OS title reads "<project> - Grizzly Editor". The toolbar and status bar are `BeginViewportSideBar`s.
- Gone: the `MenuBar` host flag and its 28 px dead strip, `GetMenuBarHeight`, `RealResetUI`, `rawdata/level_imgui_default.ini`, `UITopBarForm_ButtonList.h`.

**Menus**, in order:
- **File:** New / Open / Recent Scene, Save, Save As (the second preset of Save, Ctrl+Shift+S), Project ▸ (Import Base Scene..., Open Project Folder, Switch Project...), Quit.
- **Edit:** Undo, Redo, Cut, Copy, Paste, Duplicate, Delete, Find, Command Palette..., Preferences. These route to the active tab (`DocDesc::edit`).
- **Level only:** Scene (Find in World Outliner, Mark Selection as Mod Content / as Base Level, ...); Build (formerly Compile: Make ▸ and Compile ▸; AI Map High calls `RunXrAI_AIMap(true)`); View (`DrawViewportOptions()` plus F11 Maximize Viewport).
- **Graph only:** Graph (`Shell::BeginCaptionMenu`).
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

**Hosting:** `XrEUI/XrUIShell.{h,cpp}`, namespace `XUI::Shell`.

The document descriptor is `DocDesc{id, kind, path, icon, closable, padded, label(), tooltip(), dirty(), missing(), alive(), menus(), toolbar(), body(), status(), save(err), discard(), shortcut(key, mods), edit(action, run), on_activate(), on_deactivate(), on_close(), inactive(), restore()}`.

API:
- `Open(desc, activate)`, `Activate(id)`, `Active()`, `IsActive(id)`, `IsOpen(id)` (a loaded document; a restored placeholder is not).
- `RequestClose(id)` for the UI; it asks when unsaved.
- `Close(id, IfClean|SaveFirst|Discard, err)`, `SaveTab(id)`, `DiscardTab(id)` for MCP.
- `List()`, `Info(id)`, `Cycle(step)`, `RegisterFactory(kind, fn)`, `SaveActive()`, `Edit(action, run)`, `DrawDocMenus()`, `RequestExit()`, `FlushSession()`, `CloseProjectTabs()`.
- The frame: `FrameHooks` (caption, project toolbar, status right, log state, busy card, palette, quit, F11, session file, reveal in browser, the play-in-editor lock), `BeginFrame`, `BeginCaption` / `EndCaption`, `DrawTabStrip`, `DrawToolbar`, `DrawStatusBar`, `DrawWorkArea`, `EndFrame`, `DrawBusyFrame`, `WorkRect`.

- Activation lands in the same frame. It replaces the `m_Focus = 3` retries and `DockNextWindowWith("Render")`.
- The active document draws in `###doc.host` at `WorkPos`/`WorkSize` (NoDecoration, NoDocking, NoSavedSettings, main viewport). Inactive documents are not submitted; their `inactive()` hook runs instead.
- `EditorPda::Poll`, the browser watcher and validation move to ticks.
- Tabs persist per project in `<project>\temp\session.ltx` (`tabN = id|kind|icon|label|arg`, the arg a project-relative path or `restore()`) and load lazily: a restored tab is a placeholder (`loaded:false`) until it is activated and its kind's factory opens it. A tab whose file is gone shows "File not found"; MCP reports it as `missing`. A close inside a frame frees the document at the next `BeginFrame`.
- Switch Project and `project_close` settle the dirty tabs first, write the session and close the project's tabs; they come back when it opens again.

**Level workspace**
- The Level toolbar sits over `DockSpace(ImHashStr("GrizzlyLevel_v2"), NoWindowMenuButton, &levelClass)`.
- `Viewport###Render` is the central node, with NoTabBar and NoDockingOverMe.
- The ids stay the same behind the new labels: `Tools###LeftBar`, `World Outliner`, `Details###Properties`, `Level Settings###World Properties`, `Content Browser`, `Output Log###Log`.
- Panel tabs have no ▼ button, show their close x on hover, and sit on a 26 pt strip (`GImGuiDockTabBarExtra`, one `Grizzly Editor:` block in `imgui.cpp`).
- `Shell::FocusWindow` also selects the window's tab in its dock node, so a panel raised over MCP stays raised when the tab bar is rebuilt.
- When Level is inactive:
  - `DockSpace` still runs, with `KeepAliveOnly`;
  - panels are not submitted and the scene is not rendered;
  - `RTSize` is kept, and `screenshot_viewport` forces one pass.
- PIE activates Level and locks the tabs.
- `[ui] layout_version` ≠ 2 skips `level_imgui.ini` once, which drops its stale `nq_*`, preview, `[Table]` and old dock entries.

| Class | When | Layout | Viewport |
|---|---|---|---|
| Wide | W ≥ 1400 and H ≥ 640 | Tools on the left, 260. Right column 340: Outliner (40%) above Details + Level Settings. Bottom strip 240, under the viewport only: Content Browser + Output Log | 1103x578 at 1707x912 (today ~850x450) |
| Compact | otherwise | Tools 240, right column 300. Browser and log live only in the drawers | 736x628 at 1280x720 |

- Side nodes keep their pt size; the centre absorbs every change (ImGui dock rule 3).
- Crossing a size class rebuilds the layout, unless the user customized it. Customized means the node signature (splits and sizes rounded to 8 pt) differs from the built one; the flag is `[ui] level_layout_custom`.
- A customized layout shrinks its side nodes to their minimums first. Below that, panels scroll.
- Reset Layout rebuilds for the current size class. F11 hides the panels.

**Level toolbar.** The groups, lowest priority first:
- Save | Undo, Redo
- Build Collision, Build AI Map, Build Game Graph, Build All
- Play ▾, Play in Game, Play Clean, Stop
- Reload Configs, Game Folder

New and Open move to the File menu (#29).

**Status bar.** Drawn in chrome, including by `DrawProgressOnly`.
- **Left:** [Content Drawer], and [Output Log] with the last line worth reading: an editor report (`ELog`) or a counted error or warning - never render chatter, and never a job's start, stage or finish line (those go through `ELog.Progress`: a note in the log only). MCP `progress` answers the line as `status_line`.
- **Centre:** `status()` (Level: selection count, grid, angle).
- **Right:** the "Rebuild: Collision · AI Map · Game Graph" chip with a [Build] per item (#30), Tasks, error and warning counts, and the MCP dot.
- The counts are the Output Log's own Warnings / Errors: counted since the project opened (the start before it), each distinct line once - a repeated one does not count again. Notes the author need not act on (graph canvas notes, run command lines) are `*` notes, not `~` warnings. The log marks with a rule where the count starts, a chip opens the log on its lines and a second click closes it.
- Popovers opened from the bar (Rebuild, Tasks) stand above their chip (`Shell::NextPopupAbove`) and have the theme's padding and spacing.

**Drawers.** The Content (Ctrl+Space) and Log (Ctrl+\`) drawers cover the bottom of any tab, one at a time. A drawer shows the same object as its panel; the visible host calls `DrawBody()` - one host per frame: on Level with the panel on screen, a drawer request raises the panel instead. Esc, a click outside, a drag that leaves the drawer, or another tab coming up closes it. Its cross takes no keyboard focus.

**Tasks.** The Task Center is a status chip and a card (`Tasks###tasks_card`, 400 pt) in the bottom right corner of the window, over the work and in nobody's way: no dim, no focus taken. Each row shows the title, what it is about, its steps (`Check` - `Copy` - `Install`, the current one ringed), a 6 pt bar that is filled as far as the job has come or crossed by a light while nobody can tell how long it takes, the stage, the elapsed time and what is left, bytes and files, the file at hand, and [Stop <job>]. A chevron hides the card; the chip shows or hides it, and a click on the button that started the job brings it back. Toasts stand on top of it. A modal up hides it for as long as it is up.
- A build, an export, a packaging, a publish and every Mod Browser job run beside the editor. Nothing of them holds a frame: the file work has a thread of its own (the job's thread touches nothing of the editor and sends its log lines to the main thread), and what has to read the editor's own state - the scene's bake, the checks of quests, world rules, PDA, screens and characters - runs a slice per frame from `TUI::Idle`, drawn or not, after a walk of the project on a thread has put its folders in the file cache. A check that holds a frame for over 100 ms says so in the log.
- UI tasks open the card. MCP tasks never open it or any dialog (`ui_popup show=tasks` puts it up to look at).
- A job that runs beside the editor adds its rows through `UIProgressCenter::RegisterSource`; MCP `progress` lists them under `background`, with `steps`, `step`, `numbers`, `file`, `eta_ms`. A packaging and the upload that follows it share one row key: the card goes on from one to the other.
- A build answers an MCP caller when it has ended (`FollowsJob`: the handler says `"job_pending":true`, the pump holds the request and asks again with `answer_only`), and every other command goes on meanwhile. `progress action=cancel` stops the jobs beside the editor when no blocking job runs.
- During a blocking job, `DrawProgressOnly` draws a busy frame: the caption, the status bar, and a dimmed work area with a task card.
- `PumpProgressRequests` admits `screenshot_editor`.

**Floating windows.**
- `ViewportsEnable` is off unless the preference "Allow Panels Outside the Window" (General; `[ui] floating_panels`) is set, which takes a restart. With it off, `outside` is empty.
- Floating Level panels draw only on Level and are clamped inside the window (`ClampNextFloatingWindow`).
- The imgui.cpp modal pin (`Grizzly Editor:`) keeps a modal on the main viewport, recentres it when the work area changes, and never sizes it past the work area (a taller one scrolls).
- A minimized editor asked for a picture draws its frame as if restored (`XrUIManager::NewPlatformFrame`).

**Keys**, first taker wins:
1. A text field.
2. The shell: Ctrl+Shift+P, Ctrl+(Shift+)Tab, Ctrl+F4, Ctrl+Space, Ctrl+\`, Ctrl+S for the active tab, F11 on Level (`Shell::ShellChord`). None of them acts while a modal is up. A colliding scene binding is logged once and ignored.
3. The document's `shortcut()`.
4. `ApplyShortCut`, only when `Shell::SceneKeysAllowed()`: Level active, no popup or dialog open, and the focused window not one that owns its keys (the Content Browser, the Output Log, the drawers - `RegisterKeyOwner`).

`BlockShortCuts` is deleted, so Delete, Ctrl+D, Ctrl+C and Ctrl+S can never reach the scene from a document. F11 is the shell's fixed chord, not a rebindable command.

## 6. Surfaces

**World Outliner (#18)**
- Toolbar: SearchField; Filters with a badge (Selected Only, visibility, classes with counts, sort); ⋯ More (By Type/Hierarchy, Show All, Show/Hide Selected).
- Columns: eye, then Name with icon and ellipsis. Type and Owner are hidden, back from the header's right-click menu. The header does not sort: sorting is the Filters menu's. Class rows show an eye and a count.
- Empty states: "The scene is empty", "Nothing matches" [Clear Filters]. Footer: "2 objects · 1 selected".
- Rename is `dlg.rename_object`; Delete asks through `dlg.delete_object`, and the row menu's Delete... is in the error colour.
- Object List is retired (#17). `COMMAND_SHOW_OBJECTLIST` (Scene > Find in World Outliner) now focuses the search, replacing the in-tree `UI_LevelTools.cpp` hunk.

**Tools (#19)**
- Tiles in 1-3 columns, each [icon][label][eye] with the eye always shown. The active tool is filled with accent. A tile shows the tool's title or a short form of it; its header and the Command Palette say "<title> Tool".
- A hidden class shows eye-off and toggles with one click, sharing the flag with the Outliner.
- A sticky "<Tool> Tool" header sits over the panel's own scroll.
- A tool with a list (Object, Spawn, Particles) draws it first: a SearchField, then the list filling what the body leaves under it; the search opens every folder holding a match (`UIItemListForm::SetFilter`), with no flat result list. With less than four rows of room the list flows in the body's scroll: one scroll at a time, never a list scrolling inside a scrolling body. The tool's other sections start closed.
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
- Stats are off by default and appear as a Mono pill; nothing is printed into the render target (`CLevelMain::OnStats` is empty: the Rebuild chip says what NEED REBUILD said).
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
- The sizes are Layout keys: `graph.panel_w`, `graph.details_w`, `graph.drawer_h`, `graph.vars_split`, `graph.action_split`, and the grids `grid.graph_*`. The old level.ini `quest_*_split` keys are gone, with no migration.
- Bottom drawer: Problems (n) and Find Results. It opens when Validate finds errors or warnings.
- An external change shows a Banner with [Reload] and [Overwrite...].
- Canvas:
  - scale is `kZoom[level] x Dpi()`, applied once; the levels are `{0.2, 0.3, 0.4, 0.5, 0.65, 0.75, 1.0, 1.25, 1.5, 2.0}` (index 6 = 1:1);
  - text is 16/15/14, with glyph px rounded per zoom level;
  - level of detail: full detail for titles of 10 px and up, titles only at 7-10, blocks below;
  - Frame All fits the graph's extent (nodes and back-edge channels) at the highest level up to 1:1, never below 0.75 (index 5, `NqCanvas::FrameMinLevel`: a title is then 12 px at 100%); a graph that doesn't fit is framed AT 0.75 from its start - the trigger plus two levels in view at the top, the rest of the view filled with graph - and the status says so.
- Node, 240 wide and clipped:
  - a 28 pt header with family strip, icon, title, muted id and badges;
  - up to 3 "key: value" lines;
  - up to 4 action rows, then "+N more";
  - pin labels inside the node.
- `NqLayout::Box` (and `NodeSize` on it) computes the box for the layout, drawing, hit tests, minimap and `quest_layout`.
- The minimap is 180x120 and hidden under 12 nodes; without it a pill at the middle of each canvas edge counts the nodes wholly beyond it ("2 more") and a click brings the nearest in. Behaviour graphs never say "quest" (`NqLook::KindTitle` / `KindDesc`).

**PDA Interface (#24)**
- Toolbar: Save, Revert, Reload | Aspect, Language, Zoom (Fit/100%) as `Toolbar::Segmented` | Reset State | Source, Details.
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
- Files arrive by drag and drop, from Explorer or the Content Browser (`XUI::FileDropZone`; a tile lights up what is under it, a file from Explorer only lands; inner zones are asked first): on a layer it replaces its file, on Layers it adds layers, on Screens or the empty tab each becomes a new screen, on Music tracks; an `.ogg` is a track wherever it lands. Files from outside are copied into `screens\`, a PNG/JPG/TGA/BMP becomes a `.dds`, a `.ps` gets its `.s`.

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
- **Package Release (M, #32):** Output Folder, Requires Game, and the Author Key row (`EditorCatalog::DrawKeyLine`, read-only unless the key asks for its password), sized in em. Replacing an earlier package of the version asks through `dlg.replace_release`.
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
- Left, 320: New Project (Location, then Name with its rule as the hint, then [Create]), and [Open Folder...].
- Right: search, Segmented Last Opened / Name (kept in `_projects.ini [browser] sort`), and 232x184 cards.
  - Each card has its preview cropped to the card, the name on a row of its own with the ⋯ menu, and the path with a start ellipsis.
  - Duplicate names keep the parent folder that differs whole in the path line, in the text colour.
  - A Missing / Not a project badge.
  - The ⋯ menu (or a right click), with icons: Show in Explorer, Delete... in the error colour, Remove from List.
- The preview is a pass of its own (`EditorProject::CapturePreview`): the open scene rendered once more from the viewport's camera, without grid, pivot, selection, gizmo, axis triad, stats or captions. A picture of nothing keeps the last good one.
- Delete opens a top-level `Delete Project###dlg.delete_project` (M; Delete is Danger, Cancel the default); a project that opens under it takes it down.
- Link Game and Recovery are M cards on this page (`BeginPageCard`): Link Game [Close Project] [Link]; Recovery defaults to [Restore], [Discard...] confirms, naming the snapshot time and the older snapshots.
- Import Base Scene is an M dialog with search; it reads its list once.

**Preferences (#1)**
- The crash is fixed by the in-tree `ShortcutText.h`.
- Layout: search, a nav list with an icon per page (as Mod Settings), and the grid. A page is at most 44 em, a value at most 16 em.
- Keyboard is a Command | Shortcut | [Assign...] [Clear] table, grouped as the menus, under the official names (the palette's), with keycaps; the id is the tooltip.
- Settings apply live and are saved when a field loses focus and on close.
- Labels become official. `bp_cull` and `bp_depth_tolerance` get their own keys. Tools > Snap holds the viewport toolbar's steps, not settings of their own. Recovery reads Enabled, Interval, Delay After Input, Snapshots Kept (units in their help).

**Libraries**
- Light Animations, Texture Library and Sound Library become documents: list | preview or ImDrawList timeline | grid. An unsaved prompt replaces the destructor's MessageBox.
- The Texture Library reads the `.tga` sources of `$textures$` (rawdata\textures). Check New Textures turns a library tab without unsaved changes into New Textures (from `$import$`); the finished import brings the library back. Empty states name the folder or file each library reads.
- Detail Object List becomes an L dialog (`dlg.detail_objects`, above).
- Choose (`###dlg.choose`, L): search opens folders.
- Edit Text drops its crashing Load/Save.

## 7. MCP

Rects in answers are px, with a `dpi` field. Inputs are pt. An opener activates its target in the same frame. State-only calls never switch the tab: `outliner_filter`, `property_inspector state`, `content_browser_selection get`, `pda_preview state`, and the data commands.

Frame-reading commands are held by `GrizzlyMCP::Pump` until the frames they ask for are presented (`IsShot`: `screenshot_editor`, `screenshot_viewport`, `ui_list`, `ui_audit`, `titlebar_info`). Commands that change what is drawn and answer with where it landed run at once and, when their answer is `pending`, are held for those frames and read again with `answer_only` (`FollowsFrames`: `quest_view`, `pda_preview`, `preview_model`, `texture_preview`) - one call, raw socket included. `GrizzlyMCP` hides ImGui's nav cursor before a command, as a mouse click does.

**New commands**

| Command | Behaviour |
|---|---|
| `ui_list` | tabs {id, kind, label, path, dirty, active, missing, closable, loaded}, panels {id, window, open, visible, docked, focused, closable, host, rect}, drawer, dialogs, popups, work_area, start_page, size_class, layout_custom, viewport_maximized, `ui_scale` |
| `ui_tab action=list\|activate\|open\|close\|next\|prev id= [kind= path=] [save=]` | a dirty close without save answers `unsaved`; `kind` is any of `ui_contract` `document_kinds` |
| `ui_panel action=open\|close\|focus\|toggle id=` | `tools`, `world_outliner`, `details`, `level_settings`, `content_browser`, `output_log`, `viewport`; answers `host` |
| `ui_drawer action=open\|close id=content\|log` | |
| `ui_dialog action=list\|open\|close id=` | `package_release`, `publish_release`, `build_to_folder`, `export_flat`, `import_base_scene`, `submit`; close = Esc, and also takes a kit dialog's `dlg.<name>` |
| `ui_popup show=<id>\|none` | look only, one at a time: `menu:<name>`, `tab_menu:<tab>`, `tab_overflow`, `toolbar_overflow`, `toolbar_split:<id>`, `cb_context:<path>`, `cb_add` (refused where the browser cannot add), `cb_view`, `cb_favorites`, `cb_grid`, `outliner_filters`, `outliner_more`, `viewport:<menu>`, `tasks` (while a job runs), `rebuild` (while the scene needs it) |
| `ui_scroll id= to=top\|bottom\|<pt>\|item:<name>` | `item:` also reaches a row of a tool's list or a section (`ScrollTarget`) |
| `ui_split [key= value=]` | with no key, lists every key |
| `ui_audit [id=]` | truncated, escaped, controls (enabled, why, help) |
| `ui_scale [value=1.0..2.5]` | tests only; not saved |
| `ui_contract` | constants from `XrECore/Editor/EditorUiContract.h`: tile range, defaults and error text, view defaults, theme keys, categories, card menu labels, panels, drawers, dialog ids and titles, kit dialogs, document kinds, popups, menus, viewport menus, the palette's id, the scale range |
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
| `titlebar_info` | keeps `band`, `menus_end`, `buttons_x`; + `tabs`, `interactive`, `caption_h`, `folded`. Valid on every page |
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

## 8. Implementation plan

**Status.** Phases 0, 1 and 2 are built: the kit, the shell, the nine packages, the promotions and deltas of the integration, three rounds of the visual tour (`test_ui_tour.py` at both sizes and scales) with their fixes, and the full QA green. The plan below stays as the record of who owns what.
- Promoted into the kit in phase 2: `SplitterTail` / `SplitterAt` (F's and H's side splitters, E's edges), `EllipsizeText` / `EllipsizeFace` / `TextWidth` / `AuditTruncated` / `TextAt`, `BrowsePath`, `Prose`, `TextBlock` / `BlockLabel`, `MenuItem` with an icon and `danger`, `Kind::GhostDanger`, `Toolbar::Segmented` / `Split` / `Menu(request, on)`, `NavRow`, `TableHeaders`, `BeginCard`, `BeginPage` / `BeginPageCard` / `CardFooter`, `BeginSheet`, `FooterBusy`, `PropInline`, `ScrollTarget`.
- Still local, each used by one package: B's `XUI_Local::CategoryRow` / `FrameText` / `ColorField` / `PickFromList` / `PanelHeader` / `MutedValue` (XrEProps `UIPropsLocal.h`); C's `XUI_LocalButtonFlow`, `XUI_LocalEyeToggle`, `XUI_LocalFilteredList`, `XUI_LocalPercentSelect`; F's `XUI_LocalLibraryList`; G's `XUI_LocalDirtyDot`, `XUI_LocalLevelPicker`, `XUI_LocalProblemsDrawer`. A second user promotes one.
- Proposed and not needed yet: a badge on `Toolbar::Action`, `PropertySection` trailing actions, an icon `Button`, per-chip tone and tooltip, an ellipsizing `Toolbar::Status`, `PropertyText(text, Clip)`, `XUI::BeginTabs` that always draws the selected overline, one shared "keys bound to a command" helper.
- QA of the UI: `tools/qa/test_ui_shell.py` (the frame over MCP), `test_ui_tour.py` (the 35 audit issues, `--before` for the "before" shots), `lint_ui_units.py`, `test_graph_ui.py`, `test_previews.py`, `test_properties.py`, beside every package's suite.

Paths are under `Source/Editors/` unless they start with `tools/`, `docs/` or `rawdata/`. A listed `.cpp` implies its `.h`.

**Build.** CMake uses `CONFIGURE_DEPENDS` globs: flat in XrEUI, XrECore/Editor and Nq, recursive in LevelEditor.
- New files go only into globbed folders (`LevelEditor/Editor/Mcp/` is one). No build file is edited.
- After adding files, run `Build.ps1`, then `zs.py --refresh`.

**Phase 0: foundation.** One owner, merged first, may touch any file.
1. Take the "before" shots with `test_ui_tour.py --before`, at both sizes and both scales.
2. Build the kit from sections 2-4: `XrUIFonts` (from `spectrum.cpp`), `XrUIIcons`, `XrUIKit`, `Layout`, the audit and `-uiscale`. `EditorAssetIcons` moves onto the icon engine.
3. Build the shell and frame (all of section 5), including ELog precedence and the busy frame.
4. Adapters:
   - Every document opens through `Shell::Open` with its old inner UI.
   - Manifest and Releases stay modal until packages G and H.
   - `DrawRenderToolBar` moves verbatim to `LevelEditor/UI/UIViewportToolbar.cpp`.
5. MCP split. The dispatcher leaves `UI_LevelMain.cpp` for `LevelEditor/Editor/Mcp/`:
   - `LevelMcp.cpp`: router, shell, `ui_*`, window, theme, palette, progress, recovery, `project_close`, `preferences`;
   - `LevelMcpScene`, `LevelMcpProps`, `LevelMcpContent`, `LevelMcpGraph`, `LevelMcpPreview`, `LevelMcpMod`: today's handlers, unchanged;
   - also: the section 7 commands and `EditorUiContract.h`.
6. Delete `UI/ObjectList/*`, `UITopBarForm_ButtonList.h`, `spectrum.h`, `UIMinimapEditorForm.*` and `rawdata/level_imgui_default.ini`.
7. Commit the in-tree `ShortcutText.h`, `PropertiesListTypes.cpp` and `UI_CommandPalette.cpp`. Replace the Object List hunk.
8. `test_mod_catalog.py`: drop the default-ini fallback (:87). Phase F then checks that Submit is centred after a resize and after minimize.

Phase 0 is done when:
- the build is green;
- the KeepAliveOnly probe passes (panels keep their nodes and the ini is unchanged);
- every document kind opens at both sizes;
- the current QA passes;
- a `mod_package` shot shows the busy frame.

**Phase 1: nine parallel packages.** Each package owns its own files, and together they cover every file that draws UI.

- **A. Shell, menus, log, palette, MCP core, bridge, docs** - T1, T7, T8, T22; `test_ui_shell`
  - `XrEUI/*`
  - `XrECore/Editor/{GrizzlyMCP, EDX11Utils, device, ui_main, EditorStartup, EditorTitleBar.h, UI_ProgressCenter, UI_CommandPalette, UILogForm, UI_MainCommand, ELog, EditorAssetIcons, EditorUiContract.h}`
  - `LevelEditor/{LevelEditor.cpp, UI/UIMainForm, UI/UIMainMenuForm, UI/UITitleBar, UI/UITopBarForm, Editor/UI_LevelMain, Editor/UI_LevelTools, Editor/Mcp/LevelMcp.cpp}`
  - `Public/ShortcutText.h`, `tools/mcp/{grizzly_mcp.py, AI_SETUP.md}`, `CLAUDE.md`, `docs/MOD_QUICKSTART.md`
  - new: `tools/qa/{test_ui_shell, test_ui_tour, lint_ui_units}.py`
  - Confirm sites: UI_LevelMain 361 and 935-985, UI_MainCommand 233/417/449, UIMainMenuForm 320.
- **B. Details, grid, pickers, Preferences** - T2, T5, T21; Esc stays bound
  - `XrEProps/**`
  - `LevelEditor/UI/{UILPropertiesFrom, UIWorldPropertiesFrom, UIPropertiesModal}`
  - `XrECore/Editor/{EditorPreferences, EditorChooseEvents.h}`
  - `LevelEditor/Editor/{Entry/Light/ELight_props.cpp, Mcp/LevelMcpProps.cpp}`
- **C. Scene panels, viewport** - T2-T4; `screenshot_viewport` works behind a document
  - `LevelEditor/UI/{Outliner/*, UILeftBarForm, Tools/*, UIDOShuffle, UIDOOneColor, UIViewportToolbar}`
  - `XrECore/Editor/{UIRenderForm, UI_ViewportNavigation, EStats, UI_ToolsCustom}`
  - `LevelEditor/Editor/Mcp/LevelMcpScene.cpp`
- **D. Content Browser** - T6
  - `LevelEditor/UI/ContentBrowser/*`
  - `XrECore/Editor/{EThumbnailTexture, EThumbnailVisual}`
  - `LevelEditor/Editor/Mcp/LevelMcpContent.cpp`
  - `tools/qa/{test_asset_icons, test_unicode_names}.py`
- **E. Graph** - T9, T10; titles at least 12 px after Frame All
  - `LevelEditor/UI/QuestGraph/*`
  - `XrECore/Editor/Nq/{NqLayout, NqMcp}`
  - `LevelEditor/Editor/Utils/LevelPreferences` (its splits move to `Layout`)
  - `LevelEditor/Editor/Mcp/LevelMcpGraph.cpp`
  - `docs/NQ_ARCHITECTURE.md`, `docs/nq/NQ_FORMAT.md`
  - `tools/qa/{test_behaviours, test_graph_ui}.py`
- **F. Previews, libraries** - T11, T12, T23, T24
  - `XrECore/Editor/{EditorPdaPreview, EditorPda, EditorPdaInternal.h, UIEditLightAnim, UIImageEditorForm, UISoundEditorForm}`
  - `LevelEditor/UI/Preview/*`
  - `LevelEditor/Editor/Mcp/LevelMcpPreview.cpp`
  - `docs/pda/PDA_FORMAT.md`
  - `tools/qa/{test_pda, test_previews}.py`
- **G. Mod Settings, Characters, World Rules, build dialogs** - T13-T15, T19
  - `XrECore/Editor/{EditorModManifest, EditorCharacters, EditorWorld, EditorGameModes}`
  - `LevelEditor/Editor/{EditorModScene, Mcp/LevelMcpMod.cpp}`
  - `tools/qa/{test_characters, test_world, test_build_contents}.py`
- **H. Releases, Publish, Mod Browser** - T16-T18
  - `XrECore/Editor/{EditorModPublish, EditorCatalogSubmit, EditorCatalog, EditorGithub.h}`
  - `tools/qa/{test_mod_catalog, test_mod_publish}.py`, `tools/qa/github_mock/` (Go)
- **I. Start pages** - T20
  - `XrECore/Editor/EditorProject`
  - `LevelEditor/Editor/Recovery/*`
  - `tools/qa/test_project_browser.py`

**Contracts between packages**
- The contract is this document plus the phase 0 headers (`XUI`, `XUI::Shell`, `EditorUiContract.h`, `UIWorldOutliner::Focus()`, `UIContentBrowser::DrawBody()`/`MakeVisible()`, `UIPropertiesForm`).
- A widget the kit lacks becomes `XUI_Local` in the package; A promotes it.
- Deltas go to `<scratchpad>/ui/deltas/<pkg>.md`.

**Tour steps.** T1-T22 are the `audit_live.json` tour. T23 is the texture preview; T24 the libraries.

**Verification per package.** Short runs in the foreground:
1. `zs.py <owned .cpp>`, then `Build.ps1`.
2. `LevelEditor.exe -project <COPY> -nodlg`, closed with `window_control close`.
3. Run each owned tour step twice: at `ui_scale 1.5` / 1707x912 and at `ui_scale 1.0` / 1280x720. Each run must:
   - open and close over `ui_*`;
   - report no `escaped` entries and an empty `outside`;
   - keep its footers and overflow reachable;
   - resize both ways;
   - match its before shot.
4. `lint_ui_units.py`, then the package's QA.

**Phase 2: integration** (led by A).
1. Promote the `XUI_Local` widgets and apply the deltas.
2. Run `test_ui_tour.py`: one pass/fail shot plus one MCP assertion per issue #1-#35, at both sizes and both scales. It restores the ini files byte for byte.
3. Run the full QA, then diff the startup profile.
4. Commit per package, split by trees.

**Risks and containment**
- Old layouts: `layout_version`, with ids kept behind `###`.
- Hidden Level: keep-alive, the kept `RTSize`, forced passes.
- Screenshots: synchronous, no animation.
- `-nodlg`: the precedence.
- Canvas: node width stays 240.
- Drawer: one host per frame.
- DLLs: `XREUI_API`, one CRT.
- imgui.cpp: `Grizzly Editor:` markers.
- Constants: only in `EditorUiContract.h`.
