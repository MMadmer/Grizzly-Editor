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

pub mod audit;
pub mod button;
pub mod font;
pub mod icons;
pub mod layout;
pub mod text;
pub mod theme;

pub use button::{Button, Kind, icon_button, icon_button_sized, keycaps, keycaps_width, segmented};
pub use font::Font;
pub use icons::Icon;
pub use text::Clip;
pub use theme::{Accent, Palette, Tone, metrics, palette};
