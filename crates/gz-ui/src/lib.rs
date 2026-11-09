//! The editor's UI kit: the only way to draw UI (docs/UI_DESIGN.md).
//!
//! A surface draws with what is here and nothing else: colours come from the [`Palette`], sizes
//! from [`theme::metrics`], text from the six [`Font`] styles, pictures from the [`Icon`] set.
//! Sizes are points - egui's unit - so DPI enters in one place.
//!
//! - [`theme`] - tokens: [`Palette`], [`Tone`], [`Accent`], `metrics`, [`theme::install`]
//! - [`font`] - [`Font`], [`font::FontFiles`], [`font::install`]
//! - [`icons`] - [`Icon`], `icons::draw`
//! - [`text`] - measured and ellipsized text: [`Clip`], `text::label`, `text::paint`, `text::prose`
//! - [`button`] - [`Button`], [`Kind`], `icon_button`, `segmented`, `keycaps`
//! - [`audit`] - what a frame cut short or let escape, for `ui_audit`
//! - [`layout`] - sizes the author dragged, by key
//! - [`units`] - [`units::em`] (16 pt), [`units::button_width`] for rows laid out by hand
//! - [`search`] - [`SearchField`] (magnifier, clear, Esc clears, Ctrl+F focuses) and
//!   [`search::matches()`], the one text filter (terms, case, Cyrillic, "ё" as "е")
//! - [`menu`] - [`menu_item`] with [`MenuItem`] (icon, keycaps flush right, danger),
//!   `popup_frame`, `popup_body`, `menu_popup`, `show_popup` / `take_popup_request` (by name)
//! - [`toolbar`] - [`Toolbar::begin`] .. `end`: `group`, `separator`, `spring`, `status`,
//!   `action`, `split`, `menu`, `search`, `segmented`, `widget`; [`ToolbarStyle`]; labels fold,
//!   then items overflow into `»`; [`toolbar::plan`] is the pure planning
//! - [`grid`] - [`PropertyGrid::new(role)`](PropertyGrid::new)`.show(ui, |g| ..)`: `g.section`,
//!   `g.row(label, help, PropFlags, depth, |ui| value)`, `g.trailing`, `g.message`,
//!   `g.was_reset`, `g.block`; `help_footer`; values `text_field`, `number` / `integer`, `switch`,
//!   `combo`, `color`, `vec3_field`, `pick_field` -> [`PickAction`], `path_field` ->
//!   [`PathAction`], `chips`, `text_block`, `read_only`
//! - [`list`] - [`list::list`]`(ui, id, size, |l| ..)` with `l.row(stable_id, selected, |ui| ..)`
//!   and `l.row_actions(icons, tips)`; [`nav_row`] per [`NavRowSpec`]; `table_headers` per
//!   [`list::Column`]
//! - [`splitter`] - `splitter`, `splitter_tail`, `splitter_at` -> [`splitter::Split`]; the
//!   clamping as pure `clamp_head` / `clamp_tail` / `clamp_at`
//! - [`state`] - `empty_state`, `badge`, `status_dot`, `spinner`, `busy_line`, `banner`,
//!   `toast` (queues) + `draw_toasts` (bottom-right, timed out), `tone_icon`
//! - [`card`] - `card` (a raised box, a tone strip for attention), `page` (the work area),
//!   `page_card` + `card_footer` per [`CardAction`] (Enter presses the default)
//! - [`dialog`] - [`Dialog::new(title, "dlg.<name>", size)`](Dialog::new)`.show(ui, &mut open,
//!   |body|, |footer|)` with [`dialog::Footer`] (`status`, `busy`, `button`, `button_set`),
//!   `.sheet()`; the queued kit questions `confirm` ([`ConfirmSpec`]), `unsaved_prompt`,
//!   `notice`, `result` ([`ResultSpec`]) answered per [`Asker`] with an [`Answer`], drawn by
//!   [`dialog::draw_overlays`] (call it last every frame); for MCP `open_dialogs`
//!   ([`DialogInfo`]), `modal_on_top`, `close_dialog`

pub mod audit;
pub mod button;
pub mod font;
pub mod icons;
pub mod layout;
pub mod text;
pub mod theme;

pub mod card;
pub mod dialog;
pub mod grid;
pub mod list;
pub mod menu;
pub mod search;
pub mod splitter;
pub mod state;
pub mod toolbar;
pub mod units;

pub use card::CardAction;
pub use dialog::{Answer, Asker, ConfirmSpec, Dialog, DialogInfo, DialogSize, ResultSpec};
pub use grid::{PathAction, PickAction, PropFlags, PropertyGrid};
pub use list::{NavRowSpec, nav_row};
pub use menu::{MenuItem, menu_item};
pub use search::SearchField;
pub use toolbar::{Toolbar, ToolbarStyle};
pub use button::{Button, Kind, icon_button, icon_button_sized, keycaps, keycaps_width, segmented};
pub use font::Font;
pub use icons::Icon;
pub use text::Clip;
pub use theme::{Accent, Palette, Tone, metrics, palette};
