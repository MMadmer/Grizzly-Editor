//! Tokens: every colour and every size of the UI (docs/UI_DESIGN.md, section 2).
//!
//! A surface never writes a colour or a chrome size of its own: it takes the [`Palette`] of its
//! context and the constants of [`metrics`]. Sizes are points - egui's own unit - so DPI enters
//! in one place, the context's pixels per point.

use std::sync::Arc;

use egui::style::{Selection, WidgetVisuals, Widgets};
use egui::{Color32, Context, CornerRadius, FontFamily, FontId, Id, Margin, Stroke, Style, TextStyle, Vec2, Visuals};

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
    pub const FRAME: f32 = 24.0;
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

/// The accent family. Everything else of the theme is the same in both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Violet,
    Graphite,
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

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The colours of the theme, by role.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub accent_family: Accent,
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
}

impl Palette {
    #[must_use]
    pub fn new(accent: Accent) -> Self {
        let (accent_c, bright, selection, selection_focused, primary, primary_hover, primary_pressed) = match accent {
            Accent::Violet => (rgb(0x7971bd), rgb(0xa79fed), rgb(0x464150), rgb(0x554f72), rgb(0x6a62ae), rgb(0x7971bd), rgb(0x5c5599)),
            Accent::Graphite => (rgb(0x727782), rgb(0x9399a5), rgb(0x454951), rgb(0x50545d), rgb(0x60656f), rgb(0x727782), rgb(0x545862)),
        };
        Self {
            accent_family: accent,
            chrome: rgb(0x17161b),
            base: rgb(0x202025),
            inset: rgb(0x1c1b20),
            canvas: rgb(0x151419),
            raised: rgb(0x26252c),
            overlay: rgb(0x28272e),
            input: rgb(0x2a2931),
            input_hover: rgb(0x302e38),
            hover: rgb(0x2e2c35),
            pressed: rgb(0x36343f),
            button: rgb(0x34323d),
            button_hover: rgb(0x3d3a47),
            border_subtle: rgb(0x2c2a33),
            border: rgb(0x3d3947),
            border_strong: rgb(0x524d5f),
            text: rgb(0xf2effb),
            text_secondary: rgb(0xbdb8c9),
            text_muted: rgb(0x8e8a99),
            text_disabled: rgb(0x5f5b69),
            accent: accent_c,
            accent_bright: bright,
            selection,
            selection_focused,
            primary,
            primary_hover,
            primary_pressed,
            danger: rgb(0x8a3b43),
            danger_hover: rgb(0xa3464f),
            danger_pressed: rgb(0x74323a),
            error: rgb(0xe06c75),
            warning: rgb(0xe5c07b),
            success: rgb(0x98c379),
            required: rgb(0xd19a66),
            axis_x: rgb(0xe06c75),
            axis_y: rgb(0x98c379),
            axis_z: rgb(0x61afef),
            dim: Color32::from_rgba_unmultiplied(0x0c, 0x0b, 0x0e, 153),
            on_fill: Color32::WHITE,
        }
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
        let made = Arc::new(Palette::new(Accent::default()));
        ctx.data_mut(|d| d.insert_temp(palette_id(), made.clone()));
        made
    })
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
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.widgets = Widgets {
        noninteractive: widget(p.base, p.border_subtle, p.text),
        inactive: widget(p.button, Color32::TRANSPARENT, p.text),
        hovered: widget(p.button_hover, p.border_strong, p.text),
        active: widget(p.pressed, p.border_strong, p.text),
        open: widget(p.hover, p.border, p.text),
    };
    v.selection = Selection { bg_fill: p.selection_focused, stroke: Stroke::new(1.0, p.text), ..v.selection };
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
    v.window_stroke = Stroke::new(1.0, p.border);
    v.menu_corner_radius = CornerRadius::same(metrics::RADIUS_POPUP);
    v.panel_fill = p.base;
    v.text_cursor.stroke = Stroke::new(1.5, p.accent_bright);
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.slider_trailing_fill = true;
    v.disabled_alpha = 0.4;
    v.window_highlight_topmost = false;
    v
}

/// Installs the theme: the palette for the kit and egui's own style for what egui draws itself.
pub fn install(ctx: &Context, accent: Accent) {
    let p = Arc::new(Palette::new(accent));
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
    style.interaction.tooltip_delay = metrics::TOOLTIP_DELAY;
    style.interaction.selectable_labels = false;
    style.text_styles = [
        (TextStyle::Body, FontId::new(crate::font::Font::Body.size(), FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(crate::font::Font::Body.size(), FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(crate::font::Font::Small.size(), FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(crate::font::Font::Title.size(), FontFamily::Name(STRONG_FAMILY.into()))),
        (TextStyle::Monospace, FontId::new(crate::font::Font::Mono.size(), FontFamily::Monospace)),
    ]
    .into();
    ctx.set_global_style(style);
}
