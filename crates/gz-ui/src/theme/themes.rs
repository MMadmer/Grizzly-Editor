//! The themes of the editor: Grizzly's own and the popular published palettes, each mapped onto
//! every role of [`Palette`] (docs/UI_DESIGN.md, section 2, "Themes").
//!
//! A published theme is its palette as its authors publish it ([`Source`]: the backgrounds, the
//! three text levels, the accent and eight hues) and one mapping, [`derive`], that turns it into
//! every token: the depths by mixing the background toward the text, the states from the hues, the
//! node families as ten hues of one lightness. Where a source colour falls short of the contrast
//! a role needs ([`min`]), the mapping moves it toward the theme's extreme until it meets it; the
//! hue stays. Grizzly Dark and Grizzly Graphite are written out by hand.
//!
//! Who made each palette, under which licence and where it is published: [`Theme`]'s credit
//! fields, and `licenses/THEMES.txt`, which the third-party notices carry.

use egui::Color32;
use egui::epaint::Shadow;

use super::color::{contrast, distance, luminance, mix, oklch_fit, reach, rgb, to_oklch};
use super::{Palette, Syntax};

/// The contrast every theme holds (WCAG 2 ratios), what [`derive`] reaches and the tests check.
pub mod min {
    /// Text on base, inset, raised, overlay, chrome and input.
    pub const TEXT: f32 = 7.0;
    /// Secondary text on the same.
    pub const SECONDARY: f32 = 5.5;
    /// Muted text (hints, captions, the quiet half of a row) on base, inset, raised, overlay and
    /// chrome: still text, so AA.
    pub const MUTED: f32 = 4.5;
    /// Disabled text: legible, never mistaken for live text (always below muted).
    pub const DISABLED: f32 = 1.8;
    /// A status tone as text, on the backgrounds and on its own tint.
    pub const TONE: f32 = 4.5;
    /// The accent as a foreground (links, Info, the focus ring, check marks) on the backgrounds.
    pub const ACCENT_TEXT: f32 = 4.5;
    /// The label on a Primary or Danger fill, at rest, hovered and pressed.
    pub const ON_FILL: f32 = 4.5;
    /// The check mark or a switch's knob on the accent fill.
    pub const ON_ACCENT: f32 = 3.0;
    /// Text on the selection.
    pub const SELECTED_TEXT: f32 = 4.5;
    /// An input's outline at rest against its fill and against the surface it stands on.
    pub const BORDER: f32 = 1.25;
    /// The outline under the pointer.
    pub const BORDER_STRONG: f32 = 1.7;
    /// A graphic on what it stands on: a node family on the canvas or the overlay, an axis, a
    /// wire at rest, the focus ring, a checked icon on the focused selection.
    pub const GRAPHIC: f32 = 3.0;
    /// The chrome against the panels: a step one sees.
    pub const DEPTH: f32 = 1.06;
    /// The selection against the panels and against a row under the pointer.
    pub const SELECTION: f32 = 1.15;
    /// How far apart error, warning, success and required are in OKLab, at the least.
    pub const TONES_APART: f32 = 0.05;
    /// A token of code text on inset and base.
    pub const SYNTAX: f32 = 4.5;
}

/// A colour of a published palette, with the opacity its source gives it (over the theme's base).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Col {
    rgb: u32,
    a: u8,
}

const fn c(rgb: u32) -> Col {
    Col { rgb, a: 255 }
}

/// A colour its source writes as `rrggbbaa`.
const fn ca(rgba: u32) -> Col {
    Col { rgb: rgba >> 8, a: rgba as u8 }
}

impl Col {
    fn over(self, under: Color32) -> Color32 {
        mix(under, rgb(self.rgb), f32::from(self.a) / 255.0)
    }
}

/// A published palette, by what the mapping reads of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    light: bool,
    /// The deepest background the theme uses (a title bar, an activity bar, a terminal).
    crust: Col,
    /// Sidebars and panels: darker than the editor in a dark theme, greyer in a light one.
    sunken: Col,
    /// The editor's background.
    base: Col,
    text: Col,
    subtext: Col,
    /// The comment colour: the theme's muted text.
    comment: Col,
    accent: Col,
    /// red, orange, yellow, green, cyan, blue, purple, pink.
    hues: [Col; 8],
}

const fn dark(bg: [Col; 3], fg: [Col; 3], accent: Col, hues: [Col; 8]) -> Source {
    Source {
        light: false,
        crust: bg[0],
        sunken: bg[1],
        base: bg[2],
        text: fg[0],
        subtext: fg[1],
        comment: fg[2],
        accent,
        hues,
    }
}

const fn light(bg: [Col; 3], fg: [Col; 3], accent: Col, hues: [Col; 8]) -> Source {
    Source { light: true, ..dark(bg, fg, accent, hues) }
}

const fn hues(h: [u32; 8]) -> [Col; 8] {
    [c(h[0]), c(h[1]), c(h[2]), c(h[3]), c(h[4]), c(h[5]), c(h[6]), c(h[7])]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// Grizzly's own dark palette, written out by hand, with its violet or graphite accent.
    Grizzly {
        graphite: bool,
    },
    Derived(Source),
}

/// A theme: the id the settings and the commands use, the name the author reads, and where its
/// colours come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// `[ui] theme`, `app.prefs key=theme value=<id>`.
    pub id: &'static str,
    pub title: &'static str,
    pub light: bool,
    /// Who made the palette.
    pub author: &'static str,
    /// Its licence (SPDX).
    pub licence: &'static str,
    /// Where it is published; empty for the editor's own.
    pub source: &'static str,
    look: Look,
}

const GRIZZLY: (&str, &str, &str) = ("Grizzly Editor", "MIT", "");
const ONE_DARK: (&str, &str, &str) = ("Binaryify", "MIT", "https://github.com/Binaryify/OneDark-Pro");
const ONE_LIGHT: (&str, &str, &str) =
    ("Mahmoud Ali, after GitHub's Atom One Light", "MIT", "https://github.com/akamud/vscode-theme-onelight");
const DRACULA: (&str, &str, &str) = ("Dracula Theme", "MIT", "https://github.com/dracula/visual-studio-code");
const ALUCARD: (&str, &str, &str) = ("Dracula Theme", "MIT", "https://github.com/dracula/cursor");
const GITHUB: (&str, &str, &str) =
    ("Primer (GitHub)", "MIT", "https://github.com/primer/github-vscode-theme");
const MONOKAI: (&str, &str, &str) = (
    "Microsoft Corporation, after Colorsublime",
    "MIT",
    "https://github.com/microsoft/vscode/tree/main/extensions/theme-monokai",
);
const NIGHT_OWL: (&str, &str, &str) =
    ("Sarah Drasner", "MIT", "https://github.com/sdras/night-owl-vscode-theme");
const TOKYO: (&str, &str, &str) = ("Enkia", "MIT", "https://github.com/enkia/tokyo-night-vscode-theme");
const AYU: (&str, &str, &str) =
    ("Ike Kurghinyan, Konstantin Pschera", "MIT", "https://github.com/ayu-theme/ayu-colors");
const CATPPUCCIN: (&str, &str, &str) = ("Catppuccin", "MIT", "https://github.com/catppuccin/palette");
const NORD: (&str, &str, &str) = ("Sven Greb", "MIT", "https://github.com/nordtheme/nord");
const GRUVBOX: (&str, &str, &str) = ("Pavel Pertsev", "MIT", "https://github.com/morhetz/gruvbox");
const SOLARIZED: (&str, &str, &str) = ("Ethan Schoonover", "MIT", "https://github.com/altercation/solarized");
const ROSE_PINE: (&str, &str, &str) = ("Rosé Pine", "MIT", "https://github.com/rose-pine/neovim");
const EVERFOREST: (&str, &str, &str) = ("sainnhe", "MIT", "https://github.com/sainnhe/everforest");
const KANAGAWA: (&str, &str, &str) = ("Tommaso Laurenzi", "MIT", "https://github.com/rebelot/kanagawa.nvim");

const fn theme(
    id: &'static str,
    title: &'static str,
    credit: (&'static str, &'static str, &'static str),
    look: Look,
) -> Theme {
    let light = match look {
        Look::Grizzly { .. } => false,
        Look::Derived(s) => s.light,
    };
    Theme { id, title, light, author: credit.0, licence: credit.1, source: credit.2, look }
}

const fn derived(
    id: &'static str,
    title: &'static str,
    credit: (&'static str, &'static str, &'static str),
    s: Source,
) -> Theme {
    theme(id, title, credit, Look::Derived(s))
}

/// Every theme: the dark ones, Grizzly Dark (the default) first, then the light ones.
pub const THEMES: &[Theme] = &[
    theme("grizzly-dark", "Grizzly Dark", GRIZZLY, Look::Grizzly { graphite: false }),
    theme("grizzly-graphite", "Grizzly Graphite", GRIZZLY, Look::Grizzly { graphite: true }),
    // Binaryify/OneDark-Pro themes/OneDark-Pro.json: sideBar, editor, foregrounds, the badge, tokens
    derived(
        "one-dark-pro",
        "One Dark Pro",
        ONE_DARK,
        dark(
            [c(0x21252b), c(0x21252b), c(0x282c34)],
            [c(0xabb2bf), c(0x9da5b4), c(0x7f848e)],
            c(0x4d78cc),
            hues([0xe06c75, 0xd19a66, 0xe5c07b, 0x98c379, 0x56b6c2, 0x61afef, 0xc678dd, 0xc678dd]),
        ),
    ),
    // dracula/visual-studio-code src/dracula.yml and the spec: BGDarker, BGDark, Background,
    // Foreground, Comment; Dracula has no blue (the spec's AnsiBlue is Purple)
    derived(
        "dracula",
        "Dracula",
        DRACULA,
        dark(
            [c(0x191a21), c(0x21222c), c(0x282a36)],
            [c(0xf8f8f2), c(0x6272a4), c(0x6272a4)],
            c(0xbd93f9),
            hues([0xff5555, 0xffb86c, 0xf1fa8c, 0x50fa7b, 0x8be9fd, 0xbd93f9, 0xbd93f9, 0xff79c6]),
        ),
    ),
    // primer/github-vscode-theme on @primer/primitives 7.10: canvas.inset, canvas.default,
    // fg.default, gray[3], fg.muted, accent.emphasis, the token scale steps
    derived(
        "github-dark",
        "GitHub Dark",
        GITHUB,
        dark(
            [c(0x010409), c(0x010409), c(0x0d1117)],
            [c(0xe6edf3), c(0x8b949e), c(0x7d8590)],
            c(0x1f6feb),
            hues([0xff7b72, 0xffa657, 0xe3b341, 0x7ee787, 0x39c5cf, 0x79c0ff, 0xd2a8ff, 0xff9bce]),
        ),
    ),
    // microsoft/vscode extensions/theme-monokai: sideBar, editor, foregrounds, comment, tokens
    derived(
        "monokai",
        "Monokai",
        MONOKAI,
        dark(
            [c(0x1e1f1c), c(0x1e1f1c), c(0x272822)],
            [c(0xf8f8f2), c(0xccccc7), c(0x88846f)],
            c(0xf92672),
            hues([0xf44747, 0xfd971f, 0xe6db74, 0xa6e22e, 0x66d9ef, 0x819aff, 0xae81ff, 0xf92672]),
        ),
    ),
    // sdras/night-owl-vscode-theme themes/Night Owl-color-theme.json
    derived(
        "night-owl",
        "Night Owl",
        NIGHT_OWL,
        dark(
            [c(0x01111d), c(0x011627), c(0x011627)],
            [c(0xd6deeb), c(0x89a4bb), c(0x637777)],
            c(0x7e57c2),
            hues([0xef5350, 0xf78c6c, 0xecc48d, 0x22da6e, 0x7fdbca, 0x82aaff, 0xc792ea, 0xff5874]),
        ),
    ),
    // enkia/tokyo-night-vscode-theme: the colour themes and the README's palette
    derived(
        "tokyo-night",
        "Tokyo Night",
        TOKYO,
        dark(
            [c(0x16161e), c(0x16161e), c(0x1a1b26)],
            [c(0xa9b1d6), c(0x787c99), c(0x51597d)],
            c(0x3d59a1),
            hues([0xf7768e, 0xff9e64, 0xe0af68, 0x9ece6a, 0x7dcfff, 0x7aa2f7, 0xbb9af7, 0xbb9af7]),
        ),
    ),
    derived(
        "tokyo-night-storm",
        "Tokyo Night Storm",
        TOKYO,
        dark(
            [c(0x1f2335), c(0x1f2335), c(0x24283b)],
            [c(0xa9b1d6), c(0x8089b3), c(0x5f6996)],
            c(0x3d59a1),
            hues([0xf7768e, 0xff9e64, 0xe0af68, 0x9ece6a, 0x7dcfff, 0x7aa2f7, 0xbb9af7, 0xbb9af7]),
        ),
    ),
    // ayu-theme/ayu-colors themes/*.yaml and vscode-ayu: surfaces, the translucent foregrounds
    derived(
        "ayu-dark",
        "Ayu Dark",
        AYU,
        dark(
            [c(0x010102), c(0x0d1017), c(0x10141c)],
            [ca(0xfcf8ecbd), ca(0xc4d6fe6b), c(0x5a6673)],
            c(0xe6b450),
            hues([0xf07178, 0xff8f40, 0xffb454, 0xaad94c, 0x95e6cb, 0x59c2ff, 0xd2a6ff, 0xf29668]),
        ),
    ),
    derived(
        "ayu-mirage",
        "Ayu Mirage",
        AYU,
        dark(
            [c(0x181c26), c(0x1f2430), c(0x242936)],
            [ca(0xfefaecc4), ca(0xd3e3fc73), c(0x6e7c8f)],
            c(0xffcc66),
            hues([0xf28779, 0xffa659, 0xffcd66, 0xd5ff80, 0x95e6cb, 0x73d0ff, 0xdfbfff, 0xf29e74]),
        ),
    ),
    // catppuccin/palette palette.json: crust, mantle, base, text, subtext0, overlay2, mauve, and
    // red peach yellow green teal blue mauve pink
    derived(
        "catppuccin-mocha",
        "Catppuccin Mocha",
        CATPPUCCIN,
        dark(
            [c(0x11111b), c(0x181825), c(0x1e1e2e)],
            [c(0xcdd6f4), c(0xa6adc8), c(0x9399b2)],
            c(0xcba6f7),
            hues([0xf38ba8, 0xfab387, 0xf9e2af, 0xa6e3a1, 0x94e2d5, 0x89b4fa, 0xcba6f7, 0xf5c2e7]),
        ),
    ),
    derived(
        "catppuccin-macchiato",
        "Catppuccin Macchiato",
        CATPPUCCIN,
        dark(
            [c(0x181926), c(0x1e2030), c(0x24273a)],
            [c(0xcad3f5), c(0xa5adcb), c(0x939ab7)],
            c(0xc6a0f6),
            hues([0xed8796, 0xf5a97f, 0xeed49f, 0xa6da95, 0x8bd5ca, 0x8aadf4, 0xc6a0f6, 0xf5bde6]),
        ),
    ),
    derived(
        "catppuccin-frappe",
        "Catppuccin Frappé",
        CATPPUCCIN,
        dark(
            [c(0x232634), c(0x292c3c), c(0x303446)],
            [c(0xc6d0f5), c(0xa5adce), c(0x949cbb)],
            c(0xca9ee6),
            hues([0xe78284, 0xef9f76, 0xe5c890, 0xa6d189, 0x81c8be, 0x8caaee, 0xca9ee6, 0xf4b8e4]),
        ),
    ),
    // nordtheme/nord: nord0 base, nord4 text, nord8 frost, the aurora; the secondary text is nord4
    // at 75% (its VS Code theme writes 90%, too close to the text to tell apart) and the comment
    // colour is its VS Code theme's
    derived(
        "nord",
        "Nord",
        NORD,
        dark(
            [c(0x2e3440), c(0x2e3440), c(0x2e3440)],
            [c(0xd8dee9), ca(0xd8dee9bf), c(0x616e88)],
            c(0x88c0d0),
            hues([0xbf616a, 0xd08770, 0xebcb8b, 0xa3be8c, 0x8fbcbb, 0x81a1c1, 0xb48ead, 0xb48ead]),
        ),
    ),
    // morhetz/gruvbox colors/gruvbox.vim: bg0_h, bg0, fg1, fg4, gray, the bright row
    derived(
        "gruvbox-dark",
        "Gruvbox Dark",
        GRUVBOX,
        dark(
            [c(0x1d2021), c(0x1d2021), c(0x282828)],
            [c(0xebdbb2), c(0xa89984), c(0x928374)],
            c(0x83a598),
            hues([0xfb4934, 0xfe8019, 0xfabd2f, 0xb8bb26, 0x8ec07c, 0x83a598, 0xd3869b, 0xd3869b]),
        ),
    ),
    // altercation/solarized: base03, base0 (body), base00, base01 (comments), the accents
    derived(
        "solarized-dark",
        "Solarized Dark",
        SOLARIZED,
        dark(
            [c(0x002b36), c(0x002b36), c(0x002b36)],
            [c(0x839496), c(0x657b83), c(0x586e75)],
            c(0x268bd2),
            hues([0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682]),
        ),
    ),
    // rose-pine/neovim lua/rose-pine/palette.lua: base, text, subtle, muted, rose, and love, rose,
    // gold, leaf, foam, pine, iris, rose (Rosé Pine has no orange: rose is its warm hue)
    derived(
        "rose-pine",
        "Rosé Pine",
        ROSE_PINE,
        dark(
            [c(0x191724), c(0x191724), c(0x191724)],
            [c(0xe0def4), c(0x908caa), c(0x6e6a86)],
            c(0xebbcba),
            hues([0xeb6f92, 0xebbcba, 0xf6c177, 0x95b1ac, 0x9ccfd8, 0x31748f, 0xc4a7e7, 0xebbcba]),
        ),
    ),
    derived(
        "rose-pine-moon",
        "Rosé Pine Moon",
        ROSE_PINE,
        dark(
            [c(0x232136), c(0x232136), c(0x232136)],
            [c(0xe0def4), c(0x908caa), c(0x6e6a86)],
            c(0xea9a97),
            hues([0xeb6f92, 0xea9a97, 0xf6c177, 0x95b1ac, 0x9ccfd8, 0x3e8fb0, 0xc4a7e7, 0xea9a97]),
        ),
    ),
    // sainnhe/everforest palette.md, medium: bg_dim, bg0, fg, grey2, grey1, the accents
    derived(
        "everforest-dark",
        "Everforest Dark",
        EVERFOREST,
        dark(
            [c(0x232a2e), c(0x232a2e), c(0x2d353b)],
            [c(0xd3c6aa), c(0x9da9a0), c(0x859289)],
            c(0xa7c080),
            hues([0xe67e80, 0xe69875, 0xdbbc7f, 0xa7c080, 0x83c092, 0x7fbbb3, 0xd699b6, 0xd699b6]),
        ),
    ),
    // rebelot/kanagawa.nvim lua/kanagawa/colors.lua and themes.lua: the ui and syntax colours
    derived(
        "kanagawa-wave",
        "Kanagawa Wave",
        KANAGAWA,
        dark(
            [c(0x16161d), c(0x181820), c(0x1f1f28)],
            [c(0xdcd7ba), c(0xc8c093), c(0x727169)],
            c(0x7e9cd8),
            hues([0xe46876, 0xffa066, 0xe6c384, 0x98bb6c, 0x7aa89f, 0x7e9cd8, 0x957fb8, 0xd27e99]),
        ),
    ),
    derived(
        "kanagawa-dragon",
        "Kanagawa Dragon",
        KANAGAWA,
        dark(
            [c(0x0d0c0c), c(0x12120f), c(0x181616)],
            [c(0xc5c9c5), c(0xa6a69c), c(0x737c73)],
            c(0x8ba4b0),
            hues([0xc4746e, 0xb6927b, 0xc4b28a, 0x8a9a7b, 0x8ea4a2, 0x8ba4b0, 0x8992a7, 0xa292a3]),
        ),
    ),
    // Grizzly's own light palette: violet greys, the Grizzly accent
    derived(
        "grizzly-light",
        "Grizzly Light",
        GRIZZLY,
        light(
            [c(0xe2e1e8), c(0xebeaf0), c(0xf8f7fb)],
            [c(0x1c1a22), c(0x4a4655), c(0x6b6777)],
            c(0x6a62ae),
            hues([0xc22f4a, 0xb4560f, 0x8a6500, 0x2c7a35, 0x157a80, 0x2e6bd1, 0x7650bd, 0xad3f7c]),
        ),
    ),
    derived(
        "github-light",
        "GitHub Light",
        GITHUB,
        light(
            [c(0xf6f8fa), c(0xf6f8fa), c(0xffffff)],
            [c(0x1f2328), c(0x656d76), c(0x6e7781)],
            c(0x0969da),
            hues([0xcf222e, 0x953800, 0x9a6700, 0x116329, 0x1b7c83, 0x0550ae, 0x8250df, 0xbf3989]),
        ),
    ),
    // akamud/vscode-theme-onelight themes/OneLight.json and Atom's one-light-syntax hues
    derived(
        "one-light",
        "Atom One Light",
        ONE_LIGHT,
        light(
            [c(0xeaeaeb), c(0xeaeaeb), c(0xfafafa)],
            [c(0x383a42), c(0x696c77), c(0xa0a1a7)],
            c(0x526fff),
            hues([0xe45649, 0x986801, 0xc18401, 0x50a14f, 0x0184bc, 0x4078f2, 0xa626a4, 0xa626a4]),
        ),
    ),
    derived(
        "catppuccin-latte",
        "Catppuccin Latte",
        CATPPUCCIN,
        light(
            [c(0xdce0e8), c(0xe6e9ef), c(0xeff1f5)],
            [c(0x4c4f69), c(0x6c6f85), c(0x7c7f93)],
            c(0x8839ef),
            hues([0xd20f39, 0xfe640b, 0xdf8e1d, 0x40a02b, 0x179299, 0x1e66f5, 0x8839ef, 0xea76cb]),
        ),
    ),
    // the README's light rebase: base3, base2, base00 (body), base0, base1 (comments)
    derived(
        "solarized-light",
        "Solarized Light",
        SOLARIZED,
        light(
            [c(0xeee8d5), c(0xeee8d5), c(0xfdf6e3)],
            [c(0x657b83), c(0x839496), c(0x93a1a1)],
            c(0x268bd2),
            hues([0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682]),
        ),
    ),
    // the light branch of gruvbox.vim: light0_soft, light0, dark1, dark4, gray, the faded row
    derived(
        "gruvbox-light",
        "Gruvbox Light",
        GRUVBOX,
        light(
            [c(0xf2e5bc), c(0xf2e5bc), c(0xfbf1c7)],
            [c(0x3c3836), c(0x7c6f64), c(0x928374)],
            c(0x076678),
            hues([0x9d0006, 0xaf3a03, 0xb57614, 0x79740e, 0x427b58, 0x076678, 0x8f3f71, 0x8f3f71]),
        ),
    ),
    derived(
        "ayu-light",
        "Ayu Light",
        AYU,
        light(
            [c(0xebeef0), c(0xf8f9fa), c(0xfcfcfc)],
            [ca(0x020a12a3), ca(0x021a3c7a), c(0xadaeb1)],
            c(0xf29718),
            hues([0xf07171, 0xfa8532, 0xeba400, 0x86b300, 0x4cbf99, 0x22a4e6, 0xa37acc, 0xf2a191]),
        ),
    ),
    // themes/Night Owl-Light-color-theme.json ("Light Owl"): panels, editor, foreground, the
    // line numbers' colour as secondary text, comment, the button, the terminal colours
    derived(
        "night-owl-light",
        "Night Owl Light",
        NIGHT_OWL,
        light(
            [c(0xf0f0f0), c(0xf0f0f0), c(0xfbfbfb)],
            [c(0x403f53), c(0x90a7b2), c(0x989fb1)],
            c(0x2aa298),
            hues([0xde3d3b, 0xc96765, 0xe0af02, 0x08916a, 0x2aa298, 0x4876d6, 0x994cc3, 0xd6438a]),
        ),
    ),
    derived(
        "tokyo-night-light",
        "Tokyo Night Light",
        TOKYO,
        light(
            [c(0xd6d8df), c(0xd6d8df), c(0xe6e7ed)],
            [c(0x343b59), c(0x707280), c(0x888b94)],
            c(0x2959aa),
            hues([0x8c4351, 0x965027, 0x8f5e15, 0x385f0d, 0x006c86, 0x2959aa, 0x65359d, 0x7b43ba]),
        ),
    ),
    // palette.lua, Dawn: overlay, highlight_low, base, text, subtle, muted, rose, and the hues
    derived(
        "rose-pine-dawn",
        "Rosé Pine Dawn",
        ROSE_PINE,
        light(
            [c(0xf2e9e1), c(0xf4ede8), c(0xfaf4ed)],
            [c(0x464261), c(0x797593), c(0x9893a5)],
            c(0xd7827e),
            hues([0xb4637a, 0xd7827e, 0xea9d34, 0x6d8f89, 0x56949f, 0x286983, 0x907aa9, 0xd7827e]),
        ),
    ),
    // palette.md, light medium: bg4, bg_dim, bg0, fg, grey2, grey1, the accents
    derived(
        "everforest-light",
        "Everforest Light",
        EVERFOREST,
        light(
            [c(0xe0dcc7), c(0xefebd4), c(0xfdf6e3)],
            [c(0x5c6a72), c(0x829181), c(0x939f91)],
            c(0x93b259),
            hues([0xf85552, 0xf57d26, 0xdfa000, 0x8da101, 0x35a77c, 0x3a94c5, 0xdf69ba, 0xdf69ba]),
        ),
    ),
    derived(
        "kanagawa-lotus",
        "Kanagawa Lotus",
        KANAGAWA,
        light(
            [c(0xd5cea3), c(0xdcd5ac), c(0xf2ecbc)],
            [c(0x545464), c(0x716e61), c(0x8a8980)],
            c(0x4d699b),
            hues([0xc84053, 0xcc6d00, 0xde9800, 0x6f894e, 0x597b75, 0x4d699b, 0x624c83, 0xb35b79]),
        ),
    ),
    // dracula/cursor dev/src/alucard.yml: Dracula's light variant
    derived(
        "alucard",
        "Alucard",
        ALUCARD,
        light(
            [c(0xbcbab3), c(0xceccc0), c(0xfffbeb)],
            [c(0x1f1f1f), c(0x6c664b), c(0x6c664b)],
            c(0x644ac9),
            hues([0xcb3a2a, 0xa34d14, 0x846e15, 0x14710a, 0x036a96, 0x644ac9, 0x644ac9, 0xa3144d]),
        ),
    ),
];

impl Theme {
    /// The default theme: Grizzly Dark.
    #[must_use]
    pub fn default_theme() -> &'static Theme {
        &THEMES[0]
    }

    /// A theme by its id or its title, any case.
    #[must_use]
    pub fn find(text: &str) -> Option<&'static Theme> {
        let t = text.trim();
        THEMES.iter().find(|th| th.id.eq_ignore_ascii_case(t)).or_else(|| {
            let lower = t.to_lowercase();
            THEMES.iter().find(|th| th.title.to_lowercase() == lower)
        })
    }

    /// The theme a settings file names; one it does not know is the default.
    #[must_use]
    pub fn from_settings(text: &str) -> &'static Theme {
        Self::find(text).unwrap_or_else(Self::default_theme)
    }

    /// Every colour of the theme, by role.
    #[must_use]
    pub fn palette(&self) -> Palette {
        match self.look {
            Look::Grizzly { graphite } => grizzly(self.id, graphite),
            Look::Derived(s) => derive(self.id, &s),
        }
    }

    /// A few colours that say what the theme looks like: its chrome, its base, its text, its
    /// accent. Read off the published palette, so it costs nothing to draw a list of them.
    #[must_use]
    pub fn swatch(&self) -> [Color32; 4] {
        match self.look {
            Look::Grizzly { graphite } => {
                [rgb(0x17161b), rgb(0x202025), rgb(0xf2effb), rgb(if graphite { 0x727782 } else { 0x7971bd })]
            }
            Look::Derived(s) => {
                let base = rgb(s.base.rgb);
                [s.sunken.over(base), base, s.text.over(base), s.accent.over(base)]
            }
        }
    }
}

/// Grizzly Dark, written out by hand; Graphite changes only the accent family.
fn grizzly(id: &'static str, graphite: bool) -> Palette {
    let (accent_c, bright, selection, selection_focused, primary, primary_hover, primary_pressed) =
        if graphite {
            (
                rgb(0x727782),
                rgb(0xa1a7b3),
                rgb(0x454951),
                rgb(0x50545d),
                rgb(0x60656f),
                rgb(0x70757f),
                rgb(0x545862),
            )
        } else {
            (
                rgb(0x7971bd),
                rgb(0xa79fed),
                rgb(0x464150),
                rgb(0x554f72),
                rgb(0x6a62ae),
                rgb(0x726ab6),
                rgb(0x5c5599),
            )
        };
    Palette {
        theme: id,
        light: false,
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
        text_muted: rgb(0x938f9e),
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
        error: rgb(0xf5838c),
        warning: rgb(0xe5c07b),
        success: rgb(0x98c379),
        required: rgb(0xd29c69),
        axis_x: rgb(0xe06c75),
        axis_y: rgb(0x98c379),
        axis_z: rgb(0x61afef),
        dim: Color32::from_rgba_unmultiplied(0x0c, 0x0b, 0x0e, 153),
        on_fill: Color32::WHITE,
        // OKLCH lightness 0.72, chroma 0.09, hues 36 degrees apart: equal weight on the canvas
        graph_family: [
            rgb(0xd08a8c),
            rgb(0xcf9776),
            rgb(0xbfa56a),
            rgb(0xa3b06e),
            rgb(0x80b685),
            rgb(0x68b5a4),
            rgb(0x68aec0),
            rgb(0x7ea3d3),
            rgb(0x9a98d6),
            rgb(0xb98fc5),
        ],
        wire_base: rgb(0x8e8a99),
        wire_hot: rgb(0xf2effb),
        wire_selected: bright,
        wire_dim: rgb(0x3d3947),
        pin_exec: rgb(0xbdb8c9),
        pin_input: rgb(0x8e8a99),
        pin_hot: rgb(0xf2effb),
        pin_target: bright,
        frame_fill: Color32::from_rgba_unmultiplied(0x9a, 0x98, 0xd6, 18),
        frame_title: Color32::from_rgba_unmultiplied(0x9a, 0x98, 0xd6, 46),
        grid_line: rgb(0x1f1e25),
        scene_grid: rgb(0x909090),
        scene_grid_major: rgb(0xb4b4b4),
        scene_wire: rgb(0xd2d2d2),
        scene_label_shadow: Color32::BLACK,
        scroll_handle: rgb(0x524d5f),
        scroll_handle_hover: rgb(0x6c677a),
        shadow_popup: Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(110) },
        shadow_dialog: Shadow { offset: [0, 10], blur: 32, spread: 2, color: Color32::from_black_alpha(140) },
        shadow_card: Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(90) },
        // One Dark's token colours, which the status tones above come from
        syntax: Syntax {
            keyword: rgb(0xc678dd),
            string: rgb(0x98c379),
            number: rgb(0xd19a66),
            comment: rgb(0x938f9e),
            function: rgb(0x61afef),
            name: rgb(0xe5c07b),
            tag: rgb(0xe06c75),
            attribute: rgb(0xd19a66),
            punct: rgb(0xbdb8c9),
            added: rgb(0x98c379),
            removed: rgb(0xe06c75),
        },
    }
}

/// A tone that reads as text on every one of `on` and on its own tint (the tone at 16% over each
/// of them): its OKLCH lightness moved toward the theme's extreme (`lighter` in a dark theme) as
/// far as that takes, its hue and as much of its chroma as sRGB holds kept, so the tones stay
/// apart.
fn tone(c: Color32, lighter: bool, on: &[Color32]) -> Color32 {
    let reads = |x: Color32| {
        on.iter()
            .all(|b| contrast(x, *b) >= min::TONE + 0.05 && contrast(x, mix(*b, x, 0.16)) >= min::TONE + 0.05)
    };
    let [l0, chroma, hue] = to_oklch(c);
    let mut x = c;
    for i in 1..=100 {
        if reads(x) {
            break;
        }
        let l = if lighter { l0 + (1.0 - l0) * i as f32 / 100.0 } else { l0 * (1.0 - i as f32 / 100.0) };
        x = oklch_fit(l, chroma, hue);
    }
    x
}

/// Every token of a published palette (the module's documentation says how).
fn derive(id: &'static str, s: &Source) -> Palette {
    let (white, black) = (Color32::WHITE, Color32::BLACK);
    let paper = rgb(s.base.rgb);
    let text0 = s.text.over(paper);
    let sunken0 = s.sunken.over(paper);
    let crust0 = s.crust.over(paper);
    // the depths: a dark theme rises toward its text, a light one has white content on grey
    // panels and greyer chrome
    struct Depth {
        chrome: Color32,
        base: Color32,
        inset: Color32,
        canvas: Color32,
        raised: Color32,
        overlay: Color32,
        input: Color32,
        input_hover: Color32,
        hover: Color32,
        pressed: Color32,
        button: Color32,
        button_hover: Color32,
        border_subtle: Color32,
        border: Color32,
        border_strong: Color32,
    }
    let d = if s.light {
        let sunken =
            if luminance(sunken0) < luminance(paper) * 0.96 { sunken0 } else { mix(paper, text0, 0.05) };
        let crust =
            if luminance(crust0) < luminance(sunken) * 0.97 { crust0 } else { mix(sunken, text0, 0.05) };
        // the chrome and the canvas stand a visible step below the content, the panels between
        let chrome = reach(sunken, text0, &[paper], 1.14);
        let base = mix(paper, chrome, 0.5);
        let step = |t: f32| mix(base, text0, t);
        let input = mix(paper, white, 0.7);
        let button = mix(paper, white, 0.5);
        Depth {
            chrome,
            base,
            inset: paper,
            canvas: reach(crust, text0, &[paper], 1.18),
            raised: mix(paper, white, 0.4),
            overlay: mix(paper, white, 0.6),
            input,
            input_hover: mix(input, text0, 0.04),
            hover: step(0.06),
            pressed: step(0.11),
            button,
            button_hover: mix(button, text0, 0.05),
            border_subtle: step(0.09),
            border: step(0.2),
            border_strong: step(0.34),
        }
    } else {
        let base = paper;
        let step = |t: f32| mix(base, text0, t);
        let chrome =
            if luminance(sunken0) < luminance(base) * 0.9 { sunken0 } else { mix(base, black, 0.22) };
        let canvas =
            if luminance(crust0) < luminance(chrome) * 0.92 { crust0 } else { mix(chrome, black, 0.2) };
        // a visible step below the panels; a base that is nearly black already leaves black itself
        let (chrome, canvas) = (reach(chrome, black, &[base], 1.1), reach(canvas, black, &[base], 1.15));
        Depth {
            chrome,
            base,
            inset: mix(base, chrome, 0.5),
            canvas,
            raised: step(0.03),
            overlay: step(0.04),
            input: step(0.05),
            input_hover: step(0.075),
            hover: step(0.065),
            pressed: step(0.105),
            button: step(0.095),
            button_hover: step(0.14),
            border_subtle: step(0.055),
            border: step(0.14),
            border_strong: step(0.24),
        }
    };
    let toward = if s.light { black } else { white };
    // what text stands on: the surfaces, the canvas (a graph's labels and empty state)
    let surfaces = [d.base, d.inset, d.raised, d.overlay, d.chrome, d.input, d.canvas];
    // what a tone or a link stands on: the panels, lists, cards, popups and the status bar
    let backgrounds = [d.base, d.inset, d.raised, d.overlay, d.chrome];
    let text = reach(text0, toward, &surfaces, min::TEXT);
    let text_secondary = reach(s.subtext.over(paper), text, &surfaces, min::SECONDARY);
    let text_muted = reach(s.comment.over(paper), text, &surfaces, min::MUTED);
    let text_disabled =
        reach(mix(d.base, text, if s.light { 0.36 } else { 0.3 }), text, &surfaces[..5], min::DISABLED);
    // an input's outline reads against its fill and the surface it stands on
    let border = reach(d.border, text, &[d.input, d.base], min::BORDER);
    let border_strong = reach(d.border_strong, text, &[d.input], min::BORDER_STRONG);

    let [red, orange, yellow, green, cyan, blue, purple, _pink] = s.hues.map(|h| h.over(paper));
    let accent0 = s.accent.over(paper);
    // the accent as a foreground is the Info tone too: it reads on its own tint
    let accent_bright = tone(reach(accent0, toward, &backgrounds, min::ACCENT_TEXT), !s.light, &backgrounds);
    // the fill (a check box, a switch, a progress bar) stands out from the surfaces
    let accent_fill = reach(accent0, toward, &backgrounds, min::GRAPHIC);
    // the label on a fill: white, or in a dark theme its darkest colour where that reads better
    // on its accent (a pastel accent); a light theme's fills are deep enough for white
    let on_fill = if s.light || contrast(white, accent_fill) >= contrast(d.canvas, accent_fill) {
        white
    } else {
        d.canvas
    };
    let away = if on_fill == white { black } else { white };
    let accent = reach(accent_fill, away, &[on_fill], min::ON_ACCENT);
    let primary = reach(accent_fill, away, &[on_fill], min::ON_FILL);
    let danger = reach(if s.light { red } else { mix(red, d.base, 0.35) }, away, &[on_fill], min::ON_FILL);
    // the selection is seen against the panels and against a row under the pointer
    let sel_hue = mix(accent0, text_muted, 0.5);
    let selection =
        reach(mix(d.base, sel_hue, if s.light { 0.22 } else { 0.3 }), d.base, &[text], min::SELECTED_TEXT);
    let selection = reach(selection, text, &[d.base, d.hover], min::SELECTION);
    let selection_focused =
        reach(mix(d.base, accent0, if s.light { 0.32 } else { 0.45 }), d.base, &[text], min::SELECTED_TEXT);
    // a checked icon is accent.bright on the focused selection
    let selection_focused = reach(selection_focused, d.base, &[accent_bright], min::GRAPHIC);

    let error = tone(red, !s.light, &backgrounds);
    let warning = tone(yellow, !s.light, &backgrounds);
    let success = tone(green, !s.light, &backgrounds);
    // required is the theme's orange, or where that comes out too close to the error or the
    // warning, the hue between them that stands furthest from both
    let required = {
        let own = tone(orange, !s.light, &backgrounds);
        let apart = |c: Color32| distance(c, error).min(distance(c, warning));
        if apart(own) >= min::TONES_APART {
            own
        } else {
            let ([_, ce, he], [_, cw, hw], [lo, co, _]) = (to_oklch(red), to_oklch(yellow), to_oklch(orange));
            // the shorter way round the circle from the red's hue to the yellow's
            let span = (hw - he + 540.0) % 360.0 - 180.0;
            [0.35, 0.5, 0.65]
                .map(|k| tone(oklch_fit(lo, co.max((ce + cw) * 0.5), he + span * k), !s.light, &backgrounds))
                .into_iter()
                .chain([own])
                .max_by(|a, b| apart(*a).total_cmp(&apart(*b)))
                .unwrap_or(own)
        }
    };
    // the depth axis is blue; a theme whose blue is its purple takes its cyan
    let z = if s.hues[5] == s.hues[6] { cyan } else { blue };
    let axis = |c: Color32| reach(c, toward, &[d.canvas], min::GRAPHIC);

    // ten hues of one lightness, from the theme's red round the circle, at the lightness and
    // chroma its own hues have on average
    let lch: Vec<[f32; 3]> =
        [red, orange, yellow, green, cyan, blue, purple].iter().map(|c| to_oklch(*c)).collect();
    let mean_l = lch.iter().map(|x| x[0]).sum::<f32>() / lch.len() as f32;
    let mean_c = lch.iter().map(|x| x[1]).sum::<f32>() / lch.len() as f32;
    let l = if s.light { mean_l.clamp(0.48, 0.58) } else { mean_l.clamp(0.68, 0.8) };
    let chroma = mean_c.clamp(0.07, 0.13);
    let h0 = lch[0][2];
    let graph_family: [Color32; 10] = std::array::from_fn(|i| {
        reach(
            oklch_fit(l, chroma, h0 + 36.0 * i as f32),
            toward,
            &[d.overlay, d.raised, d.canvas],
            min::GRAPHIC,
        )
    });
    // code: the theme's own hues, moved toward its text until they read on the code's surfaces
    let code = |c: Color32| reach(c, toward, &[d.inset, d.base], min::SYNTAX);
    let syntax = Syntax {
        keyword: code(purple),
        string: code(green),
        number: code(orange),
        comment: text_muted,
        function: code(blue),
        name: code(yellow),
        tag: code(red),
        attribute: code(orange),
        punct: text_secondary,
        added: code(green),
        removed: code(red),
    };
    let frame_c = graph_family[8];
    let frame_alpha = |a: u8| Color32::from_rgba_unmultiplied(frame_c.r(), frame_c.g(), frame_c.b(), a);
    let shadow_alpha: [u8; 3] = if s.light { [40, 64, 30] } else { [110, 140, 90] };
    let dim_c = if s.light { mix(text, black, 0.4) } else { mix(d.canvas, black, 0.5) };

    Palette {
        theme: id,
        light: s.light,
        chrome: d.chrome,
        base: d.base,
        inset: d.inset,
        canvas: d.canvas,
        raised: d.raised,
        overlay: d.overlay,
        input: d.input,
        input_hover: d.input_hover,
        hover: d.hover,
        pressed: d.pressed,
        button: d.button,
        button_hover: d.button_hover,
        border_subtle: d.border_subtle,
        border,
        border_strong,
        text,
        text_secondary,
        text_muted,
        text_disabled,
        accent,
        accent_bright,
        selection,
        selection_focused,
        primary,
        primary_hover: mix(primary, away, 0.1),
        primary_pressed: mix(primary, away, 0.2),
        danger,
        danger_hover: mix(danger, away, 0.1),
        danger_pressed: mix(danger, away, 0.2),
        error,
        warning,
        success,
        required,
        axis_x: axis(red),
        axis_y: axis(green),
        axis_z: axis(z),
        dim: Color32::from_rgba_unmultiplied(dim_c.r(), dim_c.g(), dim_c.b(), if s.light { 97 } else { 153 }),
        on_fill,
        graph_family,
        wire_base: reach(text_muted, toward, &[d.canvas], min::GRAPHIC),
        wire_hot: text,
        wire_selected: accent_bright,
        wire_dim: border,
        pin_exec: text_secondary,
        pin_input: text_muted,
        pin_hot: text,
        pin_target: accent_bright,
        frame_fill: frame_alpha(if s.light { 24 } else { 18 }),
        frame_title: frame_alpha(if s.light { 60 } else { 46 }),
        grid_line: mix(d.canvas, text, if s.light { 0.07 } else { 0.045 }),
        scene_grid: mix(d.canvas, text, if s.light { 0.22 } else { 0.5 }),
        scene_grid_major: mix(d.canvas, text, if s.light { 0.4 } else { 0.66 }),
        scene_wire: mix(d.canvas, text, if s.light { 0.65 } else { 0.82 }),
        scene_label_shadow: Color32::BLACK,
        // a scroll bar is quiet: the strong border at rest, a step toward the text under the pointer
        scroll_handle: border_strong,
        scroll_handle_hover: mix(border_strong, text, 0.25),
        shadow_popup: Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(shadow_alpha[0]),
        },
        shadow_dialog: Shadow {
            offset: [0, 10],
            blur: 32,
            spread: 2,
            color: Color32::from_black_alpha(shadow_alpha[1]),
        },
        shadow_card: Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(shadow_alpha[2]),
        },
        syntax,
    }
}
