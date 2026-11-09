#pragma once
#include <functional>
#include "imgui.h"
#include "XrUIIcons.h"

// The editor's UI kit (docs/UI_DESIGN.md, section 4): one widget per job, sized in pt, coloured by
// the theme's roles. Sizes are logical points: px = pt x Dpi(), and Dpi() changes in one place only
// (SetDpi). Widgets that can clip say so to the audit (truncated / escaped) and describe themselves
// (controls), which is what MCP ui_audit reads.
//
// xrCore defines FLT_MIN as -FLT_MAX: ImGui's SetNextItemWidth(-FLT_MIN) "fill the row" idiom asks for FLT_MAX here.
// Pass GetContentRegionAvail().x instead (lint_ui_units flags it).
namespace XUI
{
using cstr = const char*;
using Strings = xr_vector<xr_string>;

// ---------------------------------------------------------------------------------------------------
// Scale
// ---------------------------------------------------------------------------------------------------
constexpr float FontBaseSize = 16.f;

XREUI_API float Dpi();
XREUI_API float Pt(float pt);
// n x the current font size: widths that follow text
XREUI_API float Em(float n = 1.f);
// The only writer of the scale: restyles in pt, rescales fonts, keeps the docked side panels and the
// floating windows at their pt size and recentres the dialogs. Inside a frame it waits for the next.
XREUI_API void SetDpi(float scale);
// The scale a window on a monitor of that DPI gets: -uiscale <f> on the command line wins.
XREUI_API float ScaleForDpi(unsigned dpi);

// ---------------------------------------------------------------------------------------------------
// Who is asking (the precedence of section 4)
// ---------------------------------------------------------------------------------------------------
// -nodlg: questions get today's logged answers
XREUI_API bool Unattended();
// an MCP request is executing: the caller has decided, nothing may prompt
XREUI_API bool FromMcp();

enum class Answer { Yes, No, Cancel };

// Marks the MCP request the dispatcher executes. `unsaved` answers an UnsavedPrompt reached under it
// (the command's save= / discard=); without one the prompt answers Cancel and McpUnsaved() tells
// the command to reply {error:"unsaved", dirty}.
struct XREUI_API McpScope
{
	explicit McpScope(Answer unsaved = Answer::Cancel);
	~McpScope();
	McpScope(const McpScope&) = delete;
	McpScope& operator=(const McpScope&) = delete;
private:
	bool m_Previous;
	Answer m_PreviousUnsaved;
};
// true when an UnsavedPrompt inside the current McpScope was refused; `what` gets its list
XREUI_API bool McpUnsaved(Strings* what = nullptr);

// ---------------------------------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------------------------------
enum class Font { Body, Strong, Small, Title, Page, Mono };

struct XREUI_API FontScope
{
	explicit FontScope(Font font);
	~FontScope();
	FontScope(const FontScope&) = delete;
	FontScope& operator=(const FontScope&) = delete;
};
// The face and its base size (before the DPI scale), for canvases that push their own sizes.
XREUI_API ImFont* GetFont(Font font);
XREUI_API float FontSize(Font font);
// Reads the faces (Segoe UI, Cascadia Mono; the embedded Source Sans only where Segoe is missing).
// The startup calls it as a stage of its own; a frame drawn before that loads them itself.
XREUI_API bool LoadFonts();

// ---------------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------------
enum class Tone { Muted, Info, Success, Warning, Error, Required };
// GhostDanger: a Ghost with error-coloured text - the quiet button that opens a destructive question ("Delete...");
// the Danger fill stays on the one button inside the question that does it
enum class Kind { Secondary, Primary, Danger, Ghost, GhostDanger };
enum class Clip { End, Middle, Start };

XREUI_API ImU32 ToneColor(Tone tone, float alpha = 1.f);
// One line cut to `w` px (-1: the rest of the row) with an ellipsis where `clip` says; the whole text
// is the tooltip. True when it was cut.
XREUI_API bool TextEllipsis(cstr text, float w = -1.f, Clip clip = Clip::End);
// Prose in a tone with its icon, wrapped at 36 em or the row.
XREUI_API void StatusText(Tone tone, cstr text);
// For rows and canvases that paint by hand: `text` in a type style at `pos` (px), one line cut to max_w px (negative: not
// cut) with an ellipsis where `clip` says; true when it was cut. No item and no tooltip: the caller's hover shows it whole.
XREUI_API bool TextAt(ImDrawList* draw_list, Font font, const ImVec2& pos, ImU32 col, cstr text, float max_w = -1.f, Clip clip = Clip::End);
// the px width of one line of `text` in a type style
XREUI_API float TextWidth(Font font, cstr text);
// `text` cut to `w` px in the current font with an ellipsis where `clip` says; false (out untouched) when it fits
XREUI_API bool EllipsizeText(cstr text, float w, Clip clip, xr_string& out);
// the same in a face at a px size of the caller's (a canvas that scales its text with the zoom)
XREUI_API bool EllipsizeFace(ImFont* face, float px, cstr text, float w, Clip clip, xr_string& out);
// Text painted by hand that was cut: ui_audit lists it as truncated (the kit's own widgets do this themselves). `full`
// is the whole text, `shown` what was drawn, [min, max] where (px).
XREUI_API void AuditTruncated(cstr full, cstr shown, const ImVec2& min, const ImVec2& max);
// Secondary prose, wrapped at 36 em or where the column ends first (StatusText without a tone).
XREUI_API void Prose(cstr text);
// The last item's tooltip after 0.45 s, disabled items included; `keys` are shown as keycaps.
XREUI_API void Tip(cstr text, cstr keys = nullptr);
// Forbids hover while set (a screenshot being taken): no tooltip, no hover fill, the legacy panels' included.
XREUI_API void SuppressHover(bool suppress);
// "Ctrl+Shift+S" as keycaps from `pos` (px), `h` px tall (0: a Small line and 4 pt); the width they take. A null draw
// list only measures.
XREUI_API float Keycaps(ImDrawList* draw_list, const ImVec2& pos, cstr keys, float h = 0.f, bool enabled = true);
// ImGui::MenuItem with the shortcut drawn as keycaps
XREUI_API bool MenuItem(cstr label, cstr keys = nullptr, bool selected = false, bool enabled = true);
XREUI_API bool MenuItem(cstr label, cstr keys, bool* selected, bool enabled = true);
// ... with an icon before the label; `danger` gives the entry that leads to a deletion the error colour
XREUI_API bool MenuItem(cstr label, Icon icon, cstr keys = nullptr, bool enabled = true, bool danger = false);

// ---------------------------------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------------------------------
// `why` is the tooltip of a disabled button. `label_set` lists every label this button can show, so
// a label that changes with the state keeps the button's width.
XREUI_API bool Button(cstr label, Kind kind = Kind::Secondary, bool on = true, cstr why = nullptr,
	const cstr* label_set = nullptr, int n = 0);
XREUI_API bool IconButton(cstr id, Icon icon, cstr tip, bool checked = false, bool on = true);
// The main part returns true; the chevron opens the popup `id` below it (BeginPopup(id) is the
// caller's, right after).
XREUI_API bool SplitButton(cstr id, Icon icon, cstr label, cstr tip, Kind kind = Kind::Secondary);
// Becomes a combo when the row is too narrow for its segments. True on a change.
XREUI_API bool Segmented(cstr id, int* cur, const cstr* labels, int n, const Icon* icons = nullptr);

// ---------------------------------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------------------------------
// A magnifier and a clear cross; Esc clears, Ctrl+F focuses the first one of the focused window.
XREUI_API bool SearchField(cstr id, char* buf, size_t n, cstr hint = "Search", float min_em = 8.f);
// Every space-separated term of `query` occurs in `text`, ignoring case (Latin and Cyrillic, with
// yo read as ye). An empty query matches everything.
XREUI_API bool Match(cstr query, cstr text);
// A path shown with a start ellipsis until it is edited, plus Browse (a native dialog). True on change.
XREUI_API bool PathField(cstr id, char* buf, size_t n, bool folder);
// The native file or folder picker PathField's Browse opens, starting where `buf` points; true when one was picked. Never
// over MCP or under -nodlg.
XREUI_API bool BrowsePath(char* buf, size_t n, bool folder);
// Files brought in by a drag: from Explorer (the window is registered once) or as tiles of the content browser, whose
// payload carries a project-relative path. FileDropZone is true on the frame they land in [min, max], with their paths;
// of zones that overlap the one asked first has them, so ask the inner ones first.
inline constexpr const char* kAssetPayload = "XRAY_ASSET";
XREUI_API void FileDropInstall(void* hwnd);
XREUI_API void FileDropMessage(void* hdrop);	// WM_DROPFILES
XREUI_API void FileDropRemove();
XREUI_API bool FileDropActive();
XREUI_API bool FileDragging();
XREUI_API bool FileDropZone(cstr id, const ImVec2& min, const ImVec2& max, Strings& out);
// A drop at a point of the window, as Explorer would make it (MCP).
XREUI_API void FileDropSimulate(const Strings& files, float x, float y);
enum PickResult { PickNone = 0, PickBrowse = 1, PickClear = 2 };
// The leaf of `value` with a start ellipsis; a click or the chevron is PickBrowse, the cross PickClear.
XREUI_API int PickField(cstr id, cstr value, bool clearable);
// X / Y / Z drags with axis tags, stacked under 15 em.
XREUI_API bool Vec3Field(cstr id, float v[3], float speed);
// Wrapping chips; returns the index of the one removed, or -1.
XREUI_API int Chips(cstr id, const Strings& items, bool removable);
// A field of several lines with its label ABOVE it, as wide as the column. True on a change.
XREUI_API bool TextBlock(cstr label, cstr help, cstr id, char* buf, size_t n, int lines);
// The label of such a block that holds no text field (a picture, a set of checkboxes); the block follows under it.
XREUI_API void BlockLabel(cstr label, cstr help);

// ---------------------------------------------------------------------------------------------------
// Toolbar. Never wraps: labels collapse to icons first (lowest priority, rightmost first), then items
// move into the overflow menu. Layout uses the last frame's measures, so an item keeps its place. A folded label
// unfolds again only with 24 pt to spare, and a row keeps its layout while it still fits, so a value that changes
// width does not fold and unfold its neighbours.
// ---------------------------------------------------------------------------------------------------
enum ToolbarStyle { ToolbarRow = 0, ToolbarPanel = 1, ToolbarOverlay = 2 };

struct XREUI_API Toolbar
{
	explicit Toolbar(cstr id, int style = ToolbarRow);
	~Toolbar();
	Toolbar(const Toolbar&) = delete;
	Toolbar& operator=(const Toolbar&) = delete;

	// starts a group; groups of a lower priority give way first
	void Group(int priority = 0);
	void Separator();
	// takes the free space: what follows is right-aligned
	void Spring();
	void Status(cstr text, Tone tone = Tone::Muted);
	bool Action(cstr id, Icon icon, cstr label, cstr tip, bool on = true, bool checked = false, Kind kind = Kind::Ghost);
	// an action with a chevron that opens its menu: true when the action is clicked; *menu_open true while the menu
	// is up - draw its items, then ImGui::EndPopup()
	bool Split(cstr id, Icon icon, cstr label, cstr tip, bool* menu_open, bool on = true, Kind kind = Kind::Primary);
	// true while its popup is open: draw the items, then ImGui::EndPopup(). `request` is the name ui_popup puts it up by.
	// Off, it is drawn disabled with `tip` as the reason, and never opens.
	bool Menu(cstr id, Icon icon, cstr label, cstr tip, cstr request = nullptr, bool on = true);
	bool Search(char* buf, size_t n, cstr hint, float min_em = 10.f);
	// a Segmented of its natural width; in the overflow menu its choices become radio items under `caption`. True on a
	// change (a pick from the overflow arrives the frame after it). *cur outside 0..n-1 marks none.
	bool Segmented(cstr id, cstr caption, int* cur, const cstr* labels, int n, cstr tip = nullptr, bool on = true);
	// reserves w_pt; true when shown - draw one widget at the cursor, its width is set. Hidden in the overflow.
	bool Widget(cstr id, float w_pt);

	void* m_State;
};

// ---------------------------------------------------------------------------------------------------
// Property grid: a label column (ratio per role, draggable, stored as grid.<role>) and a value column.
// PropertyRow returns true when the row is visible; the caller then draws one value widget, whose
// width is already set. PropInline: the value is one frame square (a switch); where labels go above values it stays
// on the label's line, before the label.
// ---------------------------------------------------------------------------------------------------
enum PropFlags { PropReadOnly = 1, PropRequired = 2, PropInvalid = 4, PropModified = 8, PropMixed = 16, PropInline = 32 };

XREUI_API bool BeginPropertyGrid(cstr role, float label_frac = .40f);
XREUI_API void EndPropertyGrid();
// true when open; EndPropertySection() closes an open one
XREUI_API bool PropertySection(cstr title, bool open = true, cstr badge = nullptr);
XREUI_API void EndPropertySection();
XREUI_API bool PropertyRow(cstr label, cstr help = nullptr, int flags = 0, int depth = 0);
// read-only value: selectable secondary text
XREUI_API void PropertyText(cstr text);
// an icon at the right end of the row (Reset, Browse ...); true on click. The value leaves room for
// as many as the row had the frame before - and always for one on a PropModified row, its Reset.
XREUI_API bool PropertyTrailing(cstr id, Icon icon, cstr tip);
// a line under the row's value
XREUI_API void PropertyMessage(Tone tone, cstr text);
// the hovered row's help, at the bottom of the panel
XREUI_API void HelpFooter();

// ---------------------------------------------------------------------------------------------------
// Sections and lists
// ---------------------------------------------------------------------------------------------------
// a full-row header that toggles on click; true when open, then EndSection()
XREUI_API bool Section(cstr title, bool open = true, cstr right = nullptr);
// An icon button at the right end of the Section or PropertySection header just drawn; true when pressed.
XREUI_API bool HeaderAction(cstr id, Icon icon, cstr tip);
XREUI_API void EndSection();
// size in pt: 0 takes the rest, negative leaves that much. EndList() always follows.
XREUI_API bool BeginList(cstr id, ImVec2 size_pt);
// a 22 pt row; true on click. The row's content follows at the cursor.
XREUI_API bool ListRow(cstr stable_id, bool selected);
XREUI_API void EndList();
// icon buttons at the right of the current row while it is hovered; the index clicked, or -1
XREUI_API int RowActions(const Icon* icons, const cstr* tips, int n);

// A 22 pt row of a nav, a settings page list or a sources pane, inside BeginList or in any scrolling pane: its icon, its
// label cut with an ellipsis, at the right a status dot or a count, and while it is hovered one action icon. The row
// draws itself (nothing follows on its line). True on a click of the row; *action_clicked on a click of the action.
struct NavRowSpec
{
	Icon icon = Icon::None;
	cstr tip = nullptr;			// the row's tooltip
	bool dot = false;			// a status dot in `tone` at the right
	Tone tone = Tone::Muted;
	cstr count = nullptr;		// a count at the right, Small and muted
	bool dim = false;			// the label in disabled text (nothing here matches a search)
	Icon action = Icon::None;	// an icon at the right while the row is hovered
	cstr action_tip = nullptr;
};
XREUI_API bool NavRow(cstr stable_id, cstr label, bool selected, const NavRowSpec& spec, bool* action_clicked = nullptr);

// The header row of the current table in Small secondary text, from the names TableSetupColumn gave; a column whose bit
// is set in `centred` has its name centred (over a column of checkboxes). A sortable table sorts from these as usual.
XREUI_API void TableHeaders(u32 centred = 0);

// A 1 pt line with a 6 pt grip between two panes, called before them: *size_pt is the size of the
// first one (along x when `vertical`), clamped every frame to leave min_b to the second. Double-click
// resets to `def`. The value lives in Layout under `key`.
XREUI_API bool Splitter(cstr key, bool vertical, float* size_pt, float min_a, float min_b, float def);
// The same line for a pane AFTER it that keeps its pt size while the one before takes every resize (a document's side
// or bottom panel, section 1.2): *tail_pt is the second pane's size, `def_tail` its default.
XREUI_API bool SplitterTail(cstr key, bool vertical, float* tail_pt, float min_head, float min_tail, float def_tail);
// The same line where the caller puts it, for panes laid out by hand: `line` (px) is where it starts, `length` (px) how
// long it runs. It sizes a pane of `key` (pt, clamped to lo..hi) that grows as the line moves right or down when `sign`
// is +1 (the pane before it) and shrinks when -1 (the pane after it: a side panel, a drawer). True while it changes.
XREUI_API bool SplitterAt(cstr key, bool vertical, const ImVec2& line, float length, float sign, float lo, float hi, float def);

// Sizes of splits and grids, in pt, kept in the .ini ([GrizzlyLayout][Data]).
namespace Layout
{
XREUI_API float Get(cstr key, float def);
XREUI_API void Set(cstr key, float value);
XREUI_API bool Has(cstr key);
XREUI_API void ResetAll();
XREUI_API void Keys(Strings& out);
}

// ---------------------------------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------------------------------
// centred icon, title, text and an optional action; true when the action is clicked
XREUI_API bool EmptyState(Icon icon, cstr title, cstr text = nullptr, cstr action = nullptr);
XREUI_API void Badge(cstr text, Tone tone = Tone::Muted);
XREUI_API void StatusDot(Tone tone, cstr tip);
// the only thing that animates
XREUI_API void Spinner();
XREUI_API void BusyLine(cstr text);
XREUI_API void Toast(Tone tone, cstr text);
// a full-width message with up to two actions; 1 or 2 when one is clicked
XREUI_API int Banner(Tone tone, cstr text, cstr action1 = nullptr, cstr action2 = nullptr);

// ---------------------------------------------------------------------------------------------------
// Cards and pages
// ---------------------------------------------------------------------------------------------------
// A card: a raised, rounded box as wide as the column and as tall as what it holds, around one part of a surface. Its
// title on top (Strong; in `tone` when `tone_title`), and a 3 pt strip in `tone` down its left edge while it asks for
// `attention`. EndCard() follows.
XREUI_API void BeginCard(cstr id, cstr title, Tone tone = Tone::Muted, bool attention = false, bool tone_title = false);
XREUI_API void EndCard();

// A page over the whole work area under the caption (the Start page): Begin() of the window `title_id`. EndPage()
// always follows.
XREUI_API bool BeginPage(cstr title_id);
XREUI_API void EndPage();
// A card centred on a page, a little above the middle: width_pt wide (less when the page is narrower), its title on top.
// The body follows at the cursor, then CardFooter (optional) and EndPageCard. A page that is not modal asks its
// questions this way.
XREUI_API void BeginPageCard(cstr id, cstr title, float width_pt);
struct CardAction
{
	cstr label;
	Kind kind;
	bool is_default;	// Enter presses it (never a Danger one)
	bool on;
	cstr why;			// the tooltip while it is off
};
// A line, then the status on the left and the buttons on the right, as a dialog's footer has them; Enter presses the
// default. The index clicked, or -1.
XREUI_API int CardFooter(cstr status, Tone tone, const CardAction* actions, int n);
XREUI_API void EndPageCard();

// ---------------------------------------------------------------------------------------------------
// Dialogs. "Title###dlg.<name>", modal, pinned to the editor's window, centred on open and whenever
// the work area changes (clamped instead once moved), at most H-48 tall with a scrolling body and a
// footer that stays put. Enter presses the default, Esc cancels; in a destructive dialog both cancel.
// ---------------------------------------------------------------------------------------------------
enum class DialogSize { S, M, L, XL };

// *open false closes it; null keeps it open while it is drawn. EndDialog() follows a true return.
XREUI_API bool BeginDialog(cstr title_id, bool* open, DialogSize size, bool resizable = false);
// A dialog of the dialog being drawn, as a sheet: under its parent's title, no wider than it, the parent dimmed. Called
// inside the parent (before its EndDialog); shown while *open, whatever -nodlg says - a question the author, or MCP,
// explicitly put up. EndDialog() follows a true return.
XREUI_API bool BeginSheet(cstr title_id, bool* open, DialogSize size = DialogSize::M);
XREUI_API void BeginDialogBody();
// ends the body; the buttons that follow are right-aligned
XREUI_API bool BeginDialogFooter(cstr status = nullptr, Tone tone = Tone::Muted);
// Right after BeginDialogFooter while a job runs: the spinner and the job's stage on the left, in what the buttons
// leave. The job's stop is a labelled footer button of its own, "Stop <job>".
XREUI_API void FooterBusy(cstr stage);
XREUI_API void EndDialog();
XREUI_API bool DialogButton(cstr label, Kind kind, bool is_default = false, bool on = true, cstr why = nullptr);

struct ConfirmSpec
{
	cstr id = nullptr;			// dlg.<id>
	cstr title = nullptr;
	cstr headline = nullptr;
	Strings items;				// what goes: up to 10 shown, then "and N more"
	u32 total = 0;				// the count of all of them (items.size() when 0)
	cstr consequence = nullptr;	// destructive default: "Deleted for good - this cannot be undone."
	cstr action = nullptr;		// the button that does it
	bool destructive = false;
};
// MCP: answers Yes at once. -nodlg: No, logged. Otherwise a dialog (a sheet inside the dialog that
// raised it) and `then` runs on a later frame.
XREUI_API void Confirm(const ConfirmSpec& spec, std::function<void(Answer)> then);
// [Save] (Save All for more than one) [Don't Save] [Cancel]
XREUI_API void UnsavedPrompt(cstr id, const Strings& what, std::function<void(Answer)> then);
XREUI_API void Notice(Tone tone, cstr title, cstr text);
// the shared result of build, export, package, publish and bake; UI runs only
XREUI_API void Result(cstr id, cstr title, Tone tone, cstr headline, const xr_vector<std::pair<xr_string, xr_string>>& rows,
	const Strings& left_out, const Strings& warnings, cstr action = nullptr, std::function<void()> on = {});
// the kit dialogs open now (their dlg.<name>), and a close as if Esc was pressed
XREUI_API void OpenDialogs(Strings& out);
XREUI_API bool CloseDialog(cstr name);

// ---------------------------------------------------------------------------------------------------
// Panels and surfaces
// ---------------------------------------------------------------------------------------------------
// Begin() of a panel window; EndPanel() always follows. Applies a scroll asked for over MCP.
XREUI_API bool BeginPanel(cstr title_id, bool* open, int flags = 0);
XREUI_API void EndPanel();

enum SurfaceKind { SurfacePanel = 0, SurfaceDocument, SurfaceDialog, SurfaceDrawer, SurfacePopup };
enum SurfaceOp { SurfaceOpen = 0, SurfaceClose, SurfaceFocus, SurfaceToggle };
// id: the MCP name; label: the window name BeginPanel is given
struct SurfaceDesc
{
	cstr id;
	cstr label;
	int kind;
	std::function<void(int op)> control;
	std::function<bool()> is_open;
};
XREUI_API void RegisterSurface(const SurfaceDesc& desc);
XREUI_API void Surfaces(Strings& ids);
XREUI_API bool SurfaceControl(cstr id, int op);
XREUI_API bool SurfaceIsOpen(cstr id);
// its label (the window name) and SurfaceKind; false when no surface has that id
XREUI_API bool SurfaceInfo(cstr id, xr_string* label, int* kind);
// top | bottom | <pt> | item:<name>, applied when the surface draws next
XREUI_API void RequestScroll(cstr surface, cstr target);
// A row or node a host draws itself (a tree row, a canvas node) that an MCP "item:<name>" scroll may name: call it
// right after the item. While the panel being drawn waits for `name`, every scrolling window around the item brings it
// into view - one scrolled out of view too.
XREUI_API void ScrollTarget(cstr name);
// The name an "item:<name>" scroll waits for in the panel being drawn, or nullptr: a tree opens the folders on its way.
XREUI_API cstr ScrollTargetWanted();
// opens the popup of that name when its owner draws next (toolbar_overflow, ...)
XREUI_API void ShowPopup(cstr name);

XREUI_API xr_string ShortcutLabel(const xr_shortcut& shortcut);

// ---------------------------------------------------------------------------------------------------
// Audit: what the last complete frame truncated, let escape and offered
// ---------------------------------------------------------------------------------------------------
struct AuditEntry
{
	xr_string window;
	xr_string kind;			// truncated | escaped | control
	xr_string label;
	xr_string detail;		// truncated: what was shown; escaped: by how much
	xr_string why;
	xr_string help;
	bool enabled = true;
	ImVec4 rect;			// x, y, w, h in px of the main viewport
};
XREUI_API void AuditSnapshot(xr_vector<AuditEntry>& out, cstr window = nullptr);
// {"truncated":[...],"escaped":[...],"controls":[...]}
XREUI_API void AuditJson(xr_string& out, cstr window = nullptr);

// Draws the kit's own queued dialogs and toasts; XrUIManager calls it once at the end of a frame.
XREUI_API void DrawOverlays();
// how far above the corner the toasts stand (px): something else lives there for now
XREUI_API void SetToastInset(float px);
// a modal is up: what floats beside the work waits behind it
XREUI_API bool ModalUp();
// the window being drawn stays above the others
XREUI_API void KeepOnTop();
}
