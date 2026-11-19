//! Tokens: every colour and every size of the UI (docs/UI_DESIGN.md, section 2).
//!
//! A surface never writes a colour or a chrome size of its own: it takes the [`Palette`] of its
//! context and the constants of [`metrics`]. Sizes are points - egui's own unit - so DPI enters
//! in one place, the context's pixels per point. The palette is a [`Theme`]'s, one of the dark and
//! light themes of [`THEMES`], each held to the contrast of [`min`]; [`install`] applies one at
//! once.

pub mod color;
#[cfg(test)]
mod tests;
mod themes;

pub use themes::{THEMES, Theme, min};

use std::sync::Arc;

use egui::epaint::Shadow;
use egui::style::{Selection, WidgetVisuals, Widgets};
use egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Id, Margin, Stroke, Style, TextStyle, Vec2, Visuals,
};

/// Sizes of the chrome and of controls, in points.
pub mod metrics {
    pub const CAPTION: f32 = 32.0;
    pub const WINDOW_BUTTON_W: f32 = 46.0;
    pub const TAB_H: f32 = 28.0;
    pub const TAB_MIN_W: f32 = 96.0;
    pub const TAB_MAX_W: f32 = 220.0;
    pub const TOOLBAR: f32 = 36.0;
    pub const TOOLBAR_BUTTON: f32 = 28.0;
    pub const TOOLBAR_GROUP_GAP: f32 = 12.0;
    pub const GLYPH: f32 = 16.0;
    pub const GLYPH_ROW: f32 = 14.0;
    pub const GLYPH_STATUS: f32 = 12.0;
    pub const PANEL_TABS: f32 = 26.0;
    pub const PANEL_TOOLBAR: f32 = 32.0;
    pub const STATUS_BAR: f32 = 24.0;
    /// The height of a control: a button, a field, a combo.
    pub const FRAME: f32 = 26.0;
    pub const ROW: f32 = 22.0;
    pub const PROPERTY_ROW: f32 = 26.0;
    pub const SECTION_HEADER: f32 = 28.0;
    pub const ICON_BUTTON: f32 = 24.0;
    pub const INDENT: f32 = 14.0;
    pub const SCROLLBAR: f32 = 10.0;
    pub const RADIUS: u8 = 4;
    pub const RADIUS_POPUP: u8 = 6;
    pub const RADIUS_DIALOG: u8 = 8;
    pub const FOCUS_RING: f32 = 2.0;
    pub const DIALOG_S: f32 = 400.0;
    pub const DIALOG_M: f32 = 560.0;
    pub const DIALOG_L: f32 = 760.0;
    pub const DIALOG_XL: f32 = 1040.0;
    pub const DIALOG_TITLE: f32 = 36.0;
    pub const DIALOG_FOOTER: f32 = 52.0;
    pub const DIALOG_BUTTON_W: f32 = 88.0;
    pub const DIALOG_BUTTON_H: f32 = 28.0;
    pub const TOOLTIP_W: f32 = 360.0;
    pub const FORM_W: f32 = 760.0;
    /// A settings-like page: a centred column of cards at most this wide.
    pub const PAGE_W: f32 = 760.0;
    /// A card on a page: its corner, its padding and the gap between two.
    pub const CARD_RADIUS: u8 = 6;
    pub const CARD_PAD: f32 = 12.0;
    pub const CARD_GAP: f32 = 12.0;
    /// The narrowest a menu is.
    pub const MENU_MIN_W: f32 = 200.0;
    /// The underline of a toolbar toggle that is on.
    pub const TOGGLE_LINE: f32 = 2.0;
    /// Spacing steps.
    pub const S2: f32 = 2.0;
    pub const S4: f32 = 4.0;
    pub const S6: f32 = 6.0;
    pub const S8: f32 = 8.0;
    pub const S12: f32 = 12.0;
    pub const S16: f32 = 16.0;
    pub const S24: f32 = 24.0;
    pub const S32: f32 = 32.0;
    /// How long the pointer rests before a tooltip shows, in seconds.
    pub const TOOLTIP_DELAY: f32 = 0.45;
}

/// What a colour says about state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Muted,
    Info,
    Success,
    Warning,
    Error,
    /// Something the author still has to do. Never grey.
    Required,
}

/// The colours of the theme, by role.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The theme's id ([`Theme::id`]).
    pub theme: &'static str,
    /// A light theme: dark text on light surfaces.
    pub light: bool,
    /// Caption, status bar, inactive tabs.
    pub chrome: Color32,
    /// Panels, documents, toolbars, the active tab.
    pub base: Color32,
    /// Lists, tables, grids, tiles, the log.
    pub inset: Color32,
    /// Viewport, graph, previews.
    pub canvas: Color32,
    /// Headers, cards, node bodies.
    pub raised: Color32,
    /// Menus, popups, tooltips, dialogs, toasts.
    pub overlay: Color32,
    pub input: Color32,
    pub input_hover: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub button: Color32,
    pub button_hover: Color32,
    pub border_subtle: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    /// Active tool, checkbox fill, toggles that are on. Never a text colour.
    pub accent: Color32,
    /// Focus, check mark, caret, links, the tab overline, active icons.
    pub accent_bright: Color32,
    pub selection: Color32,
    pub selection_focused: Color32,
    pub primary: Color32,
    pub primary_hover: Color32,
    pub primary_pressed: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    pub danger_pressed: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub required: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    /// What dims the work behind a modal, at 60%.
    pub dim: Color32,
    /// The label on a Primary or Danger fill.
    pub on_fill: Color32,
    /// The families of graph nodes: ten hues of one lightness. A family colour says what a node
    /// is, never its state - state is the status tones'.
    pub graph_family: [Color32; 10],
    /// Graph wires: at rest, under the pointer, of the selection, unrelated to the selection.
    pub wire_base: Color32,
    pub wire_hot: Color32,
    pub wire_selected: Color32,
    pub wire_dim: Color32,
    /// Graph pins: an exit (flow), an input, one under the pointer, one a wire may land on.
    pub pin_exec: Color32,
    pub pin_input: Color32,
    pub pin_hot: Color32,
    pub pin_target: Color32,
    /// A comment frame on a graph: its fill and its title strip.
    pub frame_fill: Color32,
    pub frame_title: Color32,
    /// The graph's grid lines.
    pub grid_line: Color32,
    /// The 3D viewport's grid: its lines and every tenth one.
    pub scene_grid: Color32,
    pub scene_grid_major: Color32,
    /// Edges in the 3D viewport's Wireframe mode.
    pub scene_wire: Color32,
    /// What a viewport label casts over the scene, behind the label's own colour.
    pub scene_label_shadow: Color32,
    /// A scroll bar's handle, at rest and under the pointer or dragged.
    pub scroll_handle: Color32,
    pub scroll_handle_hover: Color32,
    /// Elevation: what a menu, a pick list or a tooltip casts.
    pub shadow_popup: Shadow,
    /// What a dialog casts.
    pub shadow_dialog: Shadow,
    /// What a drawer, a toast, the Task card and a graph node cast.
    pub shadow_card: Shadow,
    /// The colours of code text (`code`).
    pub syntax: Syntax,
}

/// The colours of code text, by the kind of token: the theme's own token colours where it
/// publishes them, each reading on inset and base as muted text does (`min::SYNTAX`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syntax {
    /// `local`, `function`, `if`, `nil`; a Markdown emphasis.
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    /// The theme's muted text.
    pub comment: Color32,
    /// A function's name; a key of Lua data or JSON; an LTX section.
    pub function: Color32,
    /// A script's name before its dot; an LTX section's parents.
    pub name: Color32,
    /// An XML tag; a Markdown heading.
    pub tag: Color32,
    /// An XML attribute; an LTX key.
    pub attribute: Color32,
    /// Operators and brackets: the secondary text.
    pub punct: Color32,
    /// A diff's added line.
    pub added: Color32,
    /// A diff's removed line.
    pub removed: Color32,
}

impl Default for Palette {
    /// Grizzly Dark's.
    fn default() -> Self {
        Theme::default_theme().palette()
    }
}

impl Palette {
    /// What a cache of drawn colours keys on: the theme the palette is.
    #[must_use]
    pub fn key(&self) -> u64 {
        use std::hash::{BuildHasher, BuildHasherDefault};
        BuildHasherDefault::<rustc_hash::FxHasher>::default().hash_one(self.theme)
    }

    /// The colour of a node family, by its number (wraps after ten).
    #[must_use]
    pub fn family(&self, n: usize) -> Color32 {
        self.graph_family[n % self.graph_family.len()]
    }

    /// The colour of a tone.
    #[must_use]
    pub fn tone(&self, tone: Tone) -> Color32 {
        match tone {
            Tone::Muted => self.text_muted,
            Tone::Info => self.accent_bright,
            Tone::Success => self.success,
            Tone::Warning => self.warning,
            Tone::Error => self.error,
            Tone::Required => self.required,
        }
    }

    /// A tone as a background wash: the tone at 16%.
    #[must_use]
    pub fn tint(&self, tone: Tone) -> Color32 {
        self.tone(tone).gamma_multiply(0.16)
    }

    /// A colour at an opacity.
    #[must_use]
    pub fn alpha(color: Color32, a: f32) -> Color32 {
        color.gamma_multiply(a.clamp(0.0, 1.0))
    }
}

fn palette_id() -> Id {
    Id::unique("gz-ui.palette")
}

/// The palette of this context.
#[must_use]
pub fn palette(ctx: &Context) -> Arc<Palette> {
    ctx.data(|d| d.get_temp::<Arc<Palette>>(palette_id())).unwrap_or_else(|| {
        let made = Arc::new(Palette::default());
        ctx.data_mut(|d| d.insert_temp(palette_id(), made.clone()));
        made
    })
}

/// The themes as pick list rows: the dark ones, then the light ones, under those headings, each
/// with its swatch.
#[must_use]
pub fn pick_items() -> Vec<crate::pick::PickItem> {
    THEMES
        .iter()
        .map(|t| {
            crate::pick::PickItem::new(t.id, t.title)
                .group(if t.light { "Light" } else { "Dark" })
                .swatch(&t.swatch())
        })
        .collect()
}

/// The theme picker: the chosen theme with its swatch, opening [`pick_items`]. Returns another
/// theme when one is picked.
pub fn theme_combo(ui: &mut egui::Ui, id_salt: &str, current: &Theme) -> Option<&'static Theme> {
    let items = pick_items();
    let shown = current.swatch();
    let list = crate::pick::PickList::new(id_salt, &items).current(current.id).hint("Search themes");
    crate::pick::Combo::new(id_salt, current.title)
        .swatch(&shown)
        .show(ui, list)
        .and_then(|i| THEMES.get(i))
        .filter(|t| t.id != current.id)
}

/// The name of the font family that holds the semibold face.
pub const STRONG_FAMILY: &str = "strong";

fn visuals(p: &Palette) -> Visuals {
    let radius = CornerRadius::same(metrics::RADIUS);
    let widget = |fill: Color32, stroke: Color32, text: Color32| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    };
    // egui's own light or dark visuals underneath: its text rendering differs between the two
    let mut v = if p.light { Visuals::light() } else { Visuals::dark() };
    v.dark_mode = !p.light;
    // an input reads as an input: a 1 pt outline at rest that is not its own fill's colour, a
    // stronger one under the pointer
    v.widgets = Widgets {
        noninteractive: widget(p.base, p.border_subtle, p.text),
        inactive: widget(p.button, p.border, p.text),
        hovered: widget(p.button_hover, p.border_strong, p.text),
        active: widget(p.pressed, p.border_strong, p.text),
        open: widget(p.hover, p.border, p.text),
    };
    // egui paints a scroll bar's handle in the strong fill (and its own colour picker's and slider's
    // parts, which the kit uses once, in the property grid's colour field): the theme's handle colours
    v.widgets.inactive.bg_fill = p.scroll_handle;
    v.widgets.hovered.bg_fill = p.scroll_handle_hover;
    v.widgets.active.bg_fill = p.scroll_handle_hover;
    v.selection = Selection { bg_fill: p.selection_focused, stroke: Stroke::new(1.0, p.text) };
    v.override_text_color = None;
    v.weak_text_color = Some(p.text_muted);
    v.hyperlink_color = p.accent_bright;
    v.faint_bg_color = p.raised;
    v.extreme_bg_color = p.inset;
    v.text_edit_bg_color = Some(p.input);
    v.code_bg_color = p.inset;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.window_corner_radius = CornerRadius::same(metrics::RADIUS_DIALOG);
    v.window_fill = p.overlay;
    // elevation instead of heavy borders: a hairline and the shadow
    v.window_stroke = Stroke::new(1.0, p.border_subtle);
    v.window_shadow = p.shadow_dialog;
    v.popup_shadow = p.shadow_popup;
    v.menu_corner_radius = CornerRadius::same(metrics::RADIUS_POPUP);
    v.panel_fill = p.base;
    v.text_cursor.stroke = Stroke::new(1.5, p.accent_bright);
    v.ime_composition.active_underline_stroke = Stroke::new(1.5, p.text);
    v.ime_composition.inactive_underline_stroke = Stroke::new(1.0, p.text_muted);
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.slider_trailing_fill = true;
    v.disabled_alpha = 0.4;
    v.window_highlight_topmost = false;
    v
}

/// Installs a theme: the palette for the kit and egui's own style for what egui draws itself. It
/// applies on the next frame; nothing restarts.
pub fn install(ctx: &Context, theme: &Theme) {
    let p = Arc::new(theme.palette());
    ctx.data_mut(|d| d.insert_temp(palette_id(), p.clone()));

    let mut style = Style { visuals: visuals(&p), ..(*ctx.global_style()).clone() };
    style.spacing.item_spacing = Vec2::new(8.0, 4.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.window_margin = Margin::same(8);
    style.spacing.menu_margin = Margin::same(4);
    style.spacing.indent = metrics::INDENT;
    style.spacing.interact_size = Vec2::new(metrics::FRAME, metrics::FRAME);
    style.spacing.icon_width = 16.0;
    style.spacing.icon_width_inner = 10.0;
    style.spacing.icon_spacing = 6.0;
    style.spacing.tooltip_width = metrics::TOOLTIP_W;
    style.spacing.scroll.bar_width = metrics::SCROLLBAR;
    style.spacing.scroll.floating = false;
    // the handle in the theme's scroll colours, not in the text colour egui's floating bars take
    style.spacing.scroll.foreground_color = false;
    style.interaction.tooltip_delay = metrics::TOOLTIP_DELAY;
    style.interaction.selectable_labels = false;
    // Activation is synchronous: a menu, a dialog or a tooltip is whole on the frame it appears,
    // so a picture never catches it half way. Only the spinner animates, and it draws itself.
    style.animation_time = 0.0;
    style.text_styles = [
        (TextStyle::Body, FontId::new(crate::font::Font::Body.size(), FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(crate::font::Font::Body.size(), FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(crate::font::Font::Small.size(), FontFamily::Proportional)),
        (
            TextStyle::Heading,
            FontId::new(crate::font::Font::Title.size(), FontFamily::Name(STRONG_FAMILY.into())),
        ),
        (TextStyle::Monospace, FontId::new(crate::font::Font::Mono.size(), FontFamily::Monospace)),
    ]
    .into();
    ctx.set_global_style(style);
}
