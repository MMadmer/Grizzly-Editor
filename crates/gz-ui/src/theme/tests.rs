//! Every theme against the contrast it promises (`min`), and the list against its contract.

use egui::Color32;

use super::color::{contrast, distance};
use super::{Palette, THEMES, Theme, Tone, min};

/// A token at an opacity (premultiplied), over what it stands on.
fn over(c: Color32, under: Color32) -> Color32 {
    let a = f32::from(c.a()) / 255.0;
    let ch = |x: u8, u: u8| (f32::from(x) + f32::from(u) * (1.0 - a)).round().min(255.0) as u8;
    Color32::from_rgb(ch(c.r(), under.r()), ch(c.g(), under.g()), ch(c.b(), under.b()))
}

fn holds(fails: &mut Vec<String>, theme: &str, what: &str, fg: Color32, bg: Color32, least: f32) {
    let r = contrast(fg, bg);
    if r + 1e-3 < least {
        fails.push(format!("{theme}: {what} {r:.2} < {least}"));
    }
}

/// Every rule of `min`, for one palette.
fn check(p: &Palette, fails: &mut Vec<String>) {
    let t = p.theme;
    let surfaces = [
        ("base", p.base),
        ("inset", p.inset),
        ("raised", p.raised),
        ("overlay", p.overlay),
        ("chrome", p.chrome),
        ("input", p.input),
        ("canvas", p.canvas),
    ];
    for (n, bg) in surfaces {
        holds(fails, t, &format!("text on {n}"), p.text, bg, min::TEXT);
        holds(fails, t, &format!("secondary on {n}"), p.text_secondary, bg, min::SECONDARY);
        holds(fails, t, &format!("muted on {n}"), p.text_muted, bg, min::MUTED);
    }
    for (n, bg) in &surfaces[..5] {
        holds(fails, t, &format!("disabled on {n}"), p.text_disabled, *bg, min::DISABLED);
        holds(fails, t, &format!("accent on {n}"), p.accent, *bg, min::GRAPHIC);
        holds(fails, t, &format!("accent.bright on {n}"), p.accent_bright, *bg, min::ACCENT_TEXT);
        for tone in [Tone::Info, Tone::Success, Tone::Warning, Tone::Error, Tone::Required] {
            let c = p.tone(tone);
            holds(fails, t, &format!("{tone:?} on {n}"), c, *bg, min::TONE);
            holds(fails, t, &format!("{tone:?} on its tint over {n}"), c, over(p.tint(tone), *bg), min::TONE);
        }
    }
    // the four states stay apart: they are told apart by their colour
    let states = [Tone::Error, Tone::Warning, Tone::Success, Tone::Required].map(|tone| (tone, p.tone(tone)));
    for (i, (a, ca)) in states.iter().enumerate() {
        for (b, cb) in &states[i + 1..] {
            let d = distance(*ca, *cb);
            if d < min::TONES_APART {
                fails.push(format!("{t}: {a:?} and {b:?} only {d:.3} apart"));
            }
        }
    }
    // the levels of text stand in order
    let on_base = |c: Color32| contrast(c, p.base);
    if !(on_base(p.text) > on_base(p.text_secondary)
        && on_base(p.text_secondary) > on_base(p.text_muted)
        && on_base(p.text_muted) > on_base(p.text_disabled))
    {
        fails.push(format!("{t}: the text levels are out of order"));
    }
    for (n, fill) in [
        ("primary", p.primary),
        ("primary.hover", p.primary_hover),
        ("primary.pressed", p.primary_pressed),
        ("danger", p.danger),
        ("danger.hover", p.danger_hover),
        ("danger.pressed", p.danger_pressed),
    ] {
        holds(fails, t, &format!("on_fill on {n}"), p.on_fill, fill, min::ON_FILL);
    }
    holds(fails, t, "on_fill on accent", p.on_fill, p.accent, min::ON_ACCENT);
    for (n, sel) in [("selection", p.selection), ("selection.focused", p.selection_focused)] {
        holds(fails, t, &format!("text on {n}"), p.text, sel, min::SELECTED_TEXT);
        holds(fails, t, &format!("{n} on base"), sel, p.base, min::SELECTION);
        holds(fails, t, &format!("{n} on hover"), sel, p.hover, min::SELECTION);
    }
    holds(fails, t, "border on input", p.border, p.input, min::BORDER);
    holds(fails, t, "border on base", p.border, p.base, min::BORDER);
    holds(fails, t, "border.strong on input", p.border_strong, p.input, min::BORDER_STRONG);
    holds(
        fails,
        t,
        "a checked icon on selection.focused",
        p.accent_bright,
        p.selection_focused,
        min::GRAPHIC,
    );
    holds(fails, t, "chrome on base", p.chrome, p.base, min::DEPTH);
    for (n, bg) in [("inset", p.inset), ("base", p.base)] {
        holds(fails, t, &format!("scroll handle on {n}"), p.scroll_handle, bg, min::BORDER);
    }
    for (i, f) in p.graph_family.iter().enumerate() {
        holds(fails, t, &format!("family {i} on canvas"), *f, p.canvas, min::GRAPHIC);
        holds(fails, t, &format!("family {i} on overlay"), *f, p.overlay, min::GRAPHIC);
    }
    for (n, c) in [("axis x", p.axis_x), ("axis y", p.axis_y), ("axis z", p.axis_z), ("wire", p.wire_base)] {
        holds(fails, t, &format!("{n} on canvas"), c, p.canvas, min::GRAPHIC);
    }
}

#[test]
fn every_theme_reads() {
    let mut fails = Vec::new();
    for theme in THEMES {
        check(&theme.palette(), &mut fails);
    }
    assert!(fails.is_empty(), "{} short:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn the_list_is_whole() {
    let light = THEMES.iter().filter(|t| t.light).count();
    assert!(THEMES.len() >= 16 && light >= 5, "{} themes, {light} light", THEMES.len());
    // dark first, then light; ids unique and kebab-case; light agrees with the palette
    assert!(THEMES.windows(2).all(|w| !(w[0].light && !w[1].light)));
    for (i, t) in THEMES.iter().enumerate() {
        assert!(THEMES[..i].iter().all(|o| o.id != t.id && o.title != t.title), "{} twice", t.id);
        assert!(t.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{}", t.id);
        let p = t.palette();
        assert_eq!((p.theme, p.light), (t.id, t.light));
        assert!(!t.author.is_empty() && t.licence == "MIT");
        assert_eq!(Theme::find(t.id), Some(t));
        assert_eq!(Theme::find(&t.title.to_uppercase()), Some(t));
    }
    assert_eq!(Theme::default_theme().id, "grizzly-dark");
    // a settings file's theme by id or title; one it does not know is the default
    assert_eq!(Theme::from_settings("Solarized Light").id, "solarized-light");
    assert_eq!(Theme::from_settings("pink").id, "grizzly-dark");
}

#[test]
fn grizzly_dark_is_the_contract() {
    // docs/UI_DESIGN.md, section 2
    let p = Theme::default_theme().palette();
    let hex = |c: Color32| format!("{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
    assert_eq!(
        [p.chrome, p.base, p.inset, p.canvas, p.raised, p.overlay, p.text, p.accent, p.accent_bright]
            .map(hex),
        ["17161b", "202025", "1c1b20", "151419", "26252c", "28272e", "f2effb", "7971bd", "a79fed"]
    );
}

#[test]
fn the_notices_name_every_theme() {
    let notices = include_str!("../../../../licenses/THEMES.txt");
    for t in THEMES {
        assert!(notices.contains(t.title), "licenses/THEMES.txt does not name {}", t.title);
        assert!(t.source.is_empty() || notices.contains(t.source), "licenses/THEMES.txt lacks {}", t.source);
    }
}

/// `test.sh -p gz-ui theme_table -- --ignored --nocapture` prints every theme's key ratios on
/// its base.
#[test]
#[ignore = "a table to read, not a check"]
fn theme_table() {
    for t in THEMES {
        let p = t.palette();
        println!(
            "{:22} text {:5.2} sec {:5.2} muted {:5.2} dis {:4.2} acc {:4.2} prim {:4.2} err {:4.2} warn {:4.2}",
            t.id,
            contrast(p.text, p.base),
            contrast(p.text_secondary, p.base),
            contrast(p.text_muted, p.base),
            contrast(p.text_disabled, p.base),
            contrast(p.accent_bright, p.base),
            contrast(p.on_fill, p.primary),
            contrast(p.error, p.base),
            contrast(p.warning, p.base),
        );
    }
}
