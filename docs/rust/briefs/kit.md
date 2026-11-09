# Brief: the UI kit's widgets

Port the widgets of the editor's UI kit from C++ / Dear ImGui to Rust + egui 0.37. Read
`docs/rust/CONVENTIONS.md` and `docs/rust/STATE.md` first, then:

1. `docs/UI_DESIGN.md` sections 1, 1a, 2 and 4 (lines 1-322): **the contract** of every widget -
   sizes, colours, behaviour.
2. The kit's core, which you build on and imitate in style:
   `crates/gz-ui/src/{lib,theme,font,text,audit,layout,button}.rs`.
3. For behaviour the design text leaves open, the C++ kit: `Source/Editors/XrEUI/XrUIKit.h` (the
   API), `XrUIKit.cpp`, `XrUIKitToolbar.cpp`, `XrUIKitPanels.cpp`, `XrUIKitDialogs.cpp`. Port
   behaviour and look, not ImGui structure.

You own new modules in `crates/gz-ui/src` and may append `pub mod` / `pub use` lines and rustdoc
bullets to `lib.rs`. Do not change the existing modules. `icons.rs` belongs to another package:
use only `Icon::<Variant>`, `icons::draw(painter, icon, rect, color)`, `icons::draw_in`.

## egui 0.37

It differs from the versions most examples show. Check signatures in
`C:\Users\admin\.cargo\registry\src\index.crates.io-*\egui-0.37.0\src` instead of guessing:
`Panel` (`containers/panel.rs`), `Modal` (`containers/modal.rs`), `Popup`
(`containers/popup.rs`), `Id::unique`, `ui.scope_id()`, `Role` (not `WidgetType`),
`CornerRadius`, `ctx.global_style()`, `ctx.fonts_mut`. Sizes are points; font sizes are em
(`Font::Body` is 12 for a 16 pt line). Zero deprecation warnings.

## Modules

- **`search.rs`** - `SearchField` (magnifier, clear x, Esc clears, hint, minimum width in em)
  and `matches(query, text)`: space-separated terms that must all match, case-insensitive
  including Cyrillic, "ё" read as "е". It is the only text filter of the editor.
- **`menu.rs`** - `menu_item(ui, label, MenuItem { icon, keys, selected, enabled, danger })`:
  keycaps flush right in their column (`button::keycaps`), at least 24 pt after the longest
  label; a `danger` item in the error colour; a helper for a popup body with the theme's padding.
- **`toolbar.rs`** - `Toolbar` (section 4, "Toolbar"). Never wraps. Too wide: labels collapse to
  icons first, then items move into the overflow menu (`»`), rising priority first, rightmost
  first on a tie. It lays out from the last frame's measures (state in `ctx.data` temp storage,
  keyed by the toolbar's id); a folded label unfolds only with 24 pt to spare; a row keeps its
  layout while it still fits. `group(priority)`, `separator()`, `spring()`, `status(text, tone)`,
  `action(id, icon, label, tip, enabled, checked, kind) -> bool`, `split(..)` (an action with a
  chevron menu; in the overflow it is the action plus "<label> Options"), `menu(..)` (off: keeps
  caret and width, its reason as the tip), `search(..)`, `segmented(..)` (radio items under its
  caption in the overflow), `widget(id, width, |ui|)` (hidden in the overflow). Styles: Row
  (36 pt, buttons 28), Panel (32), Overlay (pills on the overlay colour at 92%). Make the
  overflow planning a pure function of measured widths so it can be unit-tested.
- **`grid.rs`** - the property grid (section 4, "Property grid"): `PropertyGrid::new(role)
  .show(ui, |g| ..)` with `section(title, open, badge)`, `row(label, help, flags, depth, |ui|
  value)`, `message(tone, text)`, `trailing(icon, tip)`, flags ReadOnly / Required / Invalid /
  Modified / Mixed / Inline; the label column's ratio per role in the layout key `grid.<role>`
  (default .40), draggable within 6 em .. width - 8 em; below 18 em the label goes above the
  value; `help_footer`. Row 26, section header 28, each level indents 12. Values: text field,
  number drag without steppers with a unit after it, switch, combo, colour swatch + hex,
  `vec3_field`, `pick_field` (leaf name with a start ellipsis, browse, clear ->
  `PickAction`), `path_field`, `chips` (removable), `text_block`. Every value starts at the left
  of its field; differing values read "Multiple values".
- **`list.rs`** - `list(ui, id, size, |l| ..)`, `list_row(stable_id, selected) -> Response`
  (22 pt, stable ids), `nav_row(..)` per `NavRowSpec` (icon, ellipsized label, a status dot or a
  count, one hover action), `row_actions(icons, tips) -> Option<usize>` (row-high squares with
  14 pt glyphs, on hover), `table_headers`.
- **`splitter.rs`** - `splitter` (sizes the pane before the line), `splitter_tail` (the pane
  after it), `splitter_at` (a pane laid out by hand): a 1 pt line with a 6 pt hit area,
  re-clamped every frame, double-click resets, the value kept through `layout::set`. The
  clamping is a pure function.
- **`state.rs`** - `empty_state(ui, icon, title, text, action) -> bool` (fits the height it gets:
  drops the icon first, then the margins), `badge`, `status_dot`, `spinner`, `busy_line`,
  `banner(tone, text, a1, a2) -> Option<usize>`, toasts: `toast(ctx, tone, text)` queues,
  `draw_toasts(ui)` draws them up to 360 pt wide, stacked bottom-right, timed out.
- **`card.rs`** - `card(ui, id, title, tone, attention, |ui|)` (a raised box as tall as what it
  holds, a tone strip while it asks for attention), `page(ui, title, |ui|)` (fills the work
  area), `page_card(ui, id, title, width, |ui|)` + `card_footer(ui, status, tone, actions)
  -> Option<usize>` (status left, buttons right, Enter presses the default).
- **`dialog.rs`** - the dialog system (section 4, "Dialogs" and "Precedence"):
  - `Dialog::new(title, "dlg.<name>", DialogSize::{S, M, L, XL}).show(ui, &mut open, |body|,
    |footer|)`: modal over `palette.dim`, centred, at most W-32 x H-48, a 36 pt title with a close
    cross that takes no keyboard focus, a body that scrolls, a 52 pt footer that stays put
    (status left; buttons right, at least 88 x 28), Enter presses the default, Esc cancels; in a
    destructive dialog both cancel; `footer_busy(stage)`. A running job never resizes it.
  - a sheet: a dialog raised from a dialog, under its parent's title, no wider than it, the
    parent dimmed.
  - a queue of kit dialogs drawn at the end of the frame by `draw_overlays(ui)`:
    `confirm(ctx, asker, ConfirmSpec { id, title, headline, items, total, consequence, action,
    destructive }, then)`, `unsaved_prompt(ctx, asker, id, what, then)` ([Save] [Don't Save]
    [Cancel]), `notice(ctx, asker, tone, title, text)`, `result(ctx, ResultSpec { id, title, tone,
    headline, rows, left_out, warnings, action })`. `then: impl FnOnce(Answer) + Send + 'static`
    runs after the frame that answered. Confirm lists up to 10 items, then "and N more", the
    counts, and for a destructive one "Deleted for good - this cannot be undone." in the warning
    colour; Cancel is its default.
  - who answers - `enum Asker { Ui, Mcp, Unattended }`, given by the caller: `Mcp` - a confirm
    is Yes at once, nothing opens; `Unattended` - No for a confirm, OK for a notice, and the
    function returns the line to log (`~ [dlg suppressed] <title>`); `Ui` - the dialog opens.
  - for MCP: `open_dialogs(ctx) -> Vec<DialogInfo { id, title, rect_px }>` (what the last frame
    drew, topmost last), `modal_on_top(ctx)`, `close_dialog(ctx, id) -> bool` (as Esc).

## Rules

- Colours only from `palette(ctx)`; sizes from `metrics` or named constants in points; type
  only through `Font`. No `Color32::from_rgb` outside `theme.rs`.
- Every interactive widget reports itself with `audit::control(..)`; a text you cut by hand
  reports `audit::truncated` (`text::label` / `text::paint` already do).
- State across frames lives in `ctx.data` temp storage (values there are
  `Clone + Send + Sync + 'static`), never in a static.
- These widgets draw hundreds of times per frame: take `&str`, do not allocate where a borrow
  does, measure text once.
- A small, concrete API, listed at the top of each module and as bullets in `lib.rs`'s rustdoc:
  later authors read only `lib.rs`.
- Tests for pure logic only (search matching, overflow planning, splitter clamping, the pick
  field's ellipsis) - written, not run.
- Compile: `bash tools/rust/cg.sh check -p gz-ui --tests`, zero warnings in your files. An
  error in `icons.rs` is not yours.

## Report

Under 300 words: the modules and their entry points, and exactly what of section 4 is not
implemented or simplified. Then update the `gz-ui` row of `docs/rust/STATE.md`.
