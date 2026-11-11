//! Type (docs/UI_DESIGN.md, section 2): one Segoe UI face for Latin and Cyrillic, its semibold
//! for emphasis, Cascadia Mono for code.
//!
//! The design names sizes by their line box (Body 16); egui sizes a font by its em. Segoe UI's
//! line is 1.33 em and Cascadia's 1.2 em, hence the sizes below.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily, FontId};

use crate::theme::STRONG_FAMILY;

/// The six type styles. No text is drawn in any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    /// The default.
    Body,
    /// Headers, node titles, Primary labels.
    Strong,
    /// Status bar, table headers, captions, hints, badges, keycaps.
    Small,
    /// Dialog titles, document titles, empty states.
    Title,
    /// The Start page.
    Page,
    /// Log, source, stats.
    Mono,
}

impl Font {
    /// The em size in points.
    #[must_use]
    pub const fn size(self) -> f32 {
        match self {
            Font::Body | Font::Strong => 12.0,
            Font::Small => 10.5,
            Font::Title => 13.5,
            Font::Page => 19.5,
            Font::Mono => 12.5,
        }
    }

    /// The height of a line of this style: what the design calls its size.
    #[must_use]
    pub const fn line(self) -> f32 {
        match self {
            Font::Body | Font::Strong => 16.0,
            Font::Small => 14.0,
            Font::Title => 18.0,
            Font::Page => 26.0,
            Font::Mono => 15.0,
        }
    }

    #[must_use]
    pub fn family(self) -> FontFamily {
        match self {
            Font::Body | Font::Small => FontFamily::Proportional,
            Font::Strong | Font::Title | Font::Page => FontFamily::Name(STRONG_FAMILY.into()),
            Font::Mono => FontFamily::Monospace,
        }
    }

    #[must_use]
    pub fn id(self) -> FontId {
        FontId::new(self.size(), self.family())
    }

    /// The same style at another size: canvases that zoom.
    #[must_use]
    pub fn at(self, size: f32) -> FontId {
        FontId::new(size, self.family())
    }
}

/// The font files the kit reads, as bytes. Reading them is the caller's (a startup stage of its
/// own, off the UI thread); a face that is missing falls back to egui's built-in one.
#[derive(Clone, Default)]
pub struct FontFiles {
    pub body: Option<Vec<u8>>,
    pub strong: Option<Vec<u8>>,
    pub mono: Option<Vec<u8>>,
}

/// Where Windows keeps its fonts.
#[must_use]
pub fn system_font_dir() -> PathBuf {
    Path::new(&std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts")
}

impl FontFiles {
    /// The candidates of each face, best first.
    pub const BODY: &'static [&'static str] = &["segoeui.ttf", "tahoma.ttf", "arial.ttf"];
    pub const STRONG: &'static [&'static str] =
        &["seguisb.ttf", "segoeuib.ttf", "tahomabd.ttf", "arialbd.ttf"];
    pub const MONO: &'static [&'static str] = &["CascadiaMono.ttf", "consola.ttf", "cour.ttf"];

    /// Reads the three faces from a folder. Blocking: three file reads.
    #[must_use]
    pub fn read(dir: &Path) -> Self {
        let first = |names: &[&str]| names.iter().find_map(|n| std::fs::read(dir.join(n)).ok());
        Self { body: first(Self::BODY), strong: first(Self::STRONG), mono: first(Self::MONO) }
    }
}

/// Installs the faces. Call once, before the first frame that should look right.
pub fn install(ctx: &Context, files: FontFiles) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: Option<Vec<u8>>| -> bool {
        match bytes {
            Some(bytes) => {
                fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            None => false,
        }
    };
    let has_body = add("gz-body", files.body);
    let has_strong = add("gz-strong", files.strong);
    let has_mono = add("gz-mono", files.mono);

    if has_body {
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "gz-body".to_owned());
    }
    if has_mono {
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "gz-mono".to_owned());
    }
    // the strong family falls back to the body face, then to whatever Proportional falls back to
    let mut strong: Vec<String> = Vec::new();
    if has_strong {
        strong.push("gz-strong".to_owned());
    }
    strong.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name(STRONG_FAMILY.into()), strong);
    ctx.set_fonts(fonts);
}
