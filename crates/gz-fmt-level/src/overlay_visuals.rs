//! `overlay_visuals.ltx`: the registry next to `overlay.xcform` - the visuals a module adds to a
//! level (`[v_N]`) and the boxes that hide base visuals (`[cut_N]`).
//!
//! The editor never rewrites this file: it appends to the text that is there, so hand edits
//! survive. The functions here do the same, with the same text layout and `\r\n` line ends.

use std::fmt::Write as _;

use glam::Vec3;
use smol_str::SmolStr;

use crate::Aabb;
use crate::xcform::same_cut;

/// A `[v_N]` section: one exported visual.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualEntry {
    pub index: i32,
    /// The path the section names, `visuals/<name>.ogf`.
    pub file: SmolStr,
}

/// A `[cut_N]` section: base visuals inside the box are detached.
#[derive(Clone, Debug, PartialEq)]
pub struct CutEntry {
    pub index: i32,
    pub center: Vec3,
    pub half: Vec3,
    /// `overlap = true`: also take visuals that only touch the box.
    pub overlap: bool,
}

/// The entries of an `overlay_visuals.ltx`, as plain data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayVisuals {
    pub visuals: Vec<VisualEntry>,
    pub cuts: Vec<CutEntry>,
}

impl OverlayVisuals {
    /// Reads the sections of the text. Lines it does not know are skipped; a comment starts
    /// with `;`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        enum Section {
            None,
            Visual(usize),
            Cut(usize),
        }
        let mut out = Self::default();
        let mut section = Section::None;
        for raw in text.lines() {
            let line = raw.split(';').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
                let name = name.trim();
                section = if let Some(n) = name.strip_prefix("v_").and_then(|n| n.parse().ok()) {
                    out.visuals.push(VisualEntry { index: n, file: SmolStr::default() });
                    Section::Visual(out.visuals.len() - 1)
                } else if let Some(n) = name.strip_prefix("cut_").and_then(|n| n.parse().ok()) {
                    out.cuts.push(CutEntry {
                        index: n,
                        center: Vec3::ZERO,
                        half: Vec3::ZERO,
                        overlap: false,
                    });
                    Section::Cut(out.cuts.len() - 1)
                } else {
                    Section::None
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            match section {
                Section::Visual(i) if key == "file" => {
                    if let Some(v) = out.visuals.get_mut(i) {
                        v.file = SmolStr::new(value);
                    }
                }
                Section::Cut(i) => {
                    let Some(cut) = out.cuts.get_mut(i) else { continue };
                    if key == "hide" {
                        let mut f = value.split(',').map(|p| p.trim().parse::<f32>().unwrap_or(0.0));
                        let mut next = || f.next().unwrap_or(0.0);
                        cut.center = Vec3::new(next(), next(), next());
                        cut.half = Vec3::new(next(), next(), next());
                    } else if key == "overlap" {
                        cut.overlap = matches!(value, "true" | "on" | "yes" | "1");
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// A scene object's name as a file name. Path separators,
/// wildcards, quotes and spaces become `_`; an empty name is `unnamed`; at most 255 bytes.
#[must_use]
pub fn sanitize_file_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if out.len() + c.len_utf8() > 255 {
            break;
        }
        out.push(if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ') {
            '_'
        } else {
            c
        });
    }
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

/// The file name of an exported visual: the sanitised scene name, with `_2`, `_3`, ... when
/// another object of this export already took it. The name is added to `used`.
pub fn unique_visual_name(scene_name: &str, used: &mut Vec<String>) -> String {
    let base = sanitize_file_name(scene_name);
    let mut safe = base.clone();
    let mut dup = 2u32;
    while used.contains(&safe) {
        // "%.*s_%u" with a precision of 248 bytes
        let mut cut = base.len().min(248);
        while !base.is_char_boundary(cut) {
            cut -= 1;
        }
        safe = format!("{}_{dup}", base.get(..cut).unwrap_or_default());
        dup += 1;
    }
    used.push(safe.clone());
    safe
}

/// Appends `[v_N]` sections to the registry text of a level, the way the visual export does.
#[derive(Clone, Debug)]
pub struct VisualsWriter {
    text: String,
    next_index: usize,
}

impl VisualsWriter {
    /// Starts from the text that is on disk (empty when there is no file). Numbering goes on
    /// after the `[v_` sections it holds.
    #[must_use]
    pub fn new(existing: &str) -> Self {
        Self { text: existing.to_owned(), next_index: existing.matches("[v_").count() }
    }

    /// Registers `visuals/<name>.ogf`. `false` when the text names that file already.
    pub fn add(&mut self, name: &str) -> bool {
        let line = format!("file = visuals/{name}.ogf");
        if self.text.contains(&line) {
            return false;
        }
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push_str("\r\n");
        }
        let _ = write!(self.text, "[v_{}]\r\n{line}\r\n\r\n", self.next_index);
        self.next_index += 1;
        true
    }

    /// The text to write.
    #[must_use]
    pub fn finish(self) -> String {
        self.text
    }
}

/// A cursor with the few `sscanf` conversions the cut export uses.
struct Scan<'a> {
    b: &'a [u8],
    at: usize,
}

impl Scan<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.at).copied()
    }

    /// White space in a format: any amount of it, none included.
    fn space(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace() || c == 0x0b) {
            self.at += 1;
        }
    }

    fn lit(&mut self, text: &[u8]) -> bool {
        if self.b.get(self.at..).is_some_and(|rest| rest.starts_with(text)) {
            self.at += text.len();
            true
        } else {
            false
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        self.at - start
    }

    fn sign(&mut self) {
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
    }

    /// `%d`.
    fn int(&mut self) -> Option<i32> {
        self.space();
        let start = self.at;
        self.sign();
        if self.digits() == 0 {
            return None;
        }
        let text = std::str::from_utf8(self.b.get(start..self.at)?).ok()?;
        // out of range clamps, as strtol does
        Some(text.parse::<i64>().map_or(if text.starts_with('-') { i32::MIN } else { i32::MAX }, |v| {
            v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        }))
    }

    /// `%f`: a decimal number with an optional exponent.
    fn float(&mut self) -> Option<f32> {
        self.space();
        let start = self.at;
        self.sign();
        let whole = self.digits();
        let mut frac = 0;
        if self.peek() == Some(b'.') {
            self.at += 1;
            frac = self.digits();
        }
        if whole + frac == 0 {
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.at;
            self.at += 1;
            self.sign();
            if self.digits() == 0 {
                self.at = mark;
            }
        }
        let text = std::str::from_utf8(self.b.get(start..self.at)?).ok()?;
        text.parse::<f64>().ok().map(|v| v as f32)
    }
}

/// `sscanf(line, " [cut_%d]", &idx)`.
fn scan_cut_index(line: &str) -> Option<i32> {
    let mut s = Scan { b: line.as_bytes(), at: 0 };
    s.space();
    if !s.lit(b"[cut_") {
        return None;
    }
    s.int()
}

/// `sscanf(line, " hide = %f , %f , %f , %f , %f , %f", ...)`, all six.
fn scan_hide(line: &str) -> Option<[f32; 6]> {
    let mut s = Scan { b: line.as_bytes(), at: 0 };
    s.space();
    if !s.lit(b"hide") {
        return None;
    }
    s.space();
    if !s.lit(b"=") {
        return None;
    }
    let mut out = [0.0f32; 6];
    for (i, slot) in out.iter_mut().enumerate() {
        if i > 0 {
            s.space();
            if !s.lit(b",") {
                return None;
            }
        }
        *slot = s.float()?;
    }
    Some(out)
}

/// The comment the cut export writes when `overlap` is off.
const OVERLAP_OFF: &str = "; overlap = true  ; uncomment to also take visuals that only touch the box";

/// Adds `[cut_N]` sections for the boxes the text does not hide yet, the way the cut export
/// does: what is there stays as written, numbering goes on after the highest index, and a box
/// within [`crate::CUT_TOLERANCE`] of a `hide` line is not repeated. Returns the text to write
/// and how many sections were added.
#[must_use]
pub fn append_cuts(existing: &str, boxes: &[Aabb], overlap: bool) -> (String, usize) {
    let mut known: Vec<Aabb> = Vec::new();
    let mut next_cut = 0i32;
    for line in existing.split('\n') {
        if let Some(idx) = scan_cut_index(line)
            && idx >= next_cut
        {
            next_cut = idx.saturating_add(1);
        }
        if let Some([x, y, z, ex, ey, ez]) = scan_hide(line) {
            known.push(Aabb::new(Vec3::new(x - ex, y - ey, z - ez), Vec3::new(x + ex, y + ey, z + ez)));
        }
    }

    let mut text = existing.trim_end_matches(['\n', '\r']).to_owned();
    if !text.is_empty() {
        text.push_str("\r\n\r\n");
    }
    let mut added = 0usize;
    for b in boxes {
        if known.iter().any(|k| same_cut(b, k)) {
            continue;
        }
        let c = b.center();
        let e = b.half_extents();
        let f = |v: f32| f64::from(v);
        let _ = write!(
            text,
            "[cut_{}]\r\nhide    = {:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\r\n{}\r\n",
            next_cut.wrapping_add(added as i32),
            f(c.x),
            f(c.y),
            f(c.z),
            f(e.x),
            f(e.y),
            f(e.z),
            if overlap { "overlap = true" } else { OVERLAP_OFF },
        );
        added += 1;
    }
    (text, added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe_and_unique() {
        assert_eq!(sanitize_file_name("house 1/roof:a*b?"), "house_1_roof_a_b_");
        assert_eq!(sanitize_file_name(""), "unnamed");
        assert_eq!(sanitize_file_name("Дом 2"), "Дом_2");
        let mut used = Vec::new();
        assert_eq!(unique_visual_name("a b", &mut used), "a_b");
        assert_eq!(unique_visual_name("a_b", &mut used), "a_b_2");
        assert_eq!(unique_visual_name("a:b", &mut used), "a_b_3");
        assert_eq!(unique_visual_name("c", &mut used), "c");
    }

    #[test]
    fn visuals_are_appended_with_the_editor_layout() {
        let mut w = VisualsWriter::new("");
        assert!(w.add("house"));
        assert!(w.add("barn"));
        assert!(!w.add("house"));
        let text = w.finish();
        assert_eq!(text, "[v_0]\r\nfile = visuals/house.ogf\r\n\r\n[v_1]\r\nfile = visuals/barn.ogf\r\n\r\n");

        // a hand-edited file without a final line end, with two sections already
        let mut w = VisualsWriter::new("[v_0]\r\nfile = visuals/a.ogf\r\n[v_7]\r\nfile = visuals/b.ogf");
        assert!(w.add("c"));
        assert_eq!(
            w.finish(),
            "[v_0]\r\nfile = visuals/a.ogf\r\n[v_7]\r\nfile = visuals/b.ogf\r\n[v_2]\r\nfile = visuals/c.ogf\r\n\r\n"
        );
    }

    #[test]
    fn cuts_are_appended_once_and_numbered_on() {
        let b = Aabb::new(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 3.0));
        let (text, added) = append_cuts("", &[b], false);
        assert_eq!(added, 1);
        assert_eq!(
            text,
            "[cut_0]\r\nhide    = 0.000000,1.000000,1.000000,1.000000,1.000000,2.000000\r\n\
             ; overlap = true  ; uncomment to also take visuals that only touch the box\r\n"
        );

        // the same box again changes nothing but the blank lines at the end
        let (again, added) = append_cuts(&text, &[b], true);
        assert_eq!(added, 0);
        assert_eq!(again, format!("{}\r\n\r\n", text.trim_end()));

        // a new box goes after the highest index, whatever order the sections are in
        let held = "[cut_5]\r\nhide = 0, 1 ,1,1,1,2\r\noverlap = true\r\n[cut_2]\r\nhide=9,9,9,1,1,1\r\n";
        let other = Aabb::new(Vec3::splat(10.0), Vec3::splat(12.5));
        let (text, added) = append_cuts(held, &[b, other], true);
        assert_eq!(added, 1);
        assert!(text.starts_with("[cut_5]\r\nhide = 0, 1 ,1,1,1,2\r\noverlap = true\r\n[cut_2]\r\nhide=9,9,9,1,1,1\r\n\r\n[cut_6]\r\n"));
        assert!(text.ends_with(
            "hide    = 11.250000,11.250000,11.250000,1.250000,1.250000,1.250000\r\noverlap = true\r\n"
        ));
    }

    #[test]
    fn the_registry_reads_back_as_data() {
        let mut w = VisualsWriter::new("");
        w.add("house");
        let (text, _) = append_cuts(&w.finish(), &[Aabb::new(Vec3::ZERO, Vec3::splat(2.0))], true);
        let (text, _) = append_cuts(&text, &[Aabb::new(Vec3::splat(4.0), Vec3::splat(8.0))], false);
        let data = OverlayVisuals::parse(&text);
        assert_eq!(data.visuals, vec![VisualEntry { index: 0, file: "visuals/house.ogf".into() }]);
        assert_eq!(data.cuts.len(), 2);
        assert_eq!(data.cuts[0], CutEntry { index: 0, center: Vec3::ONE, half: Vec3::ONE, overlap: true });
        assert_eq!(data.cuts[1].index, 1);
        assert_eq!(data.cuts[1].center, Vec3::splat(6.0));
        assert!(!data.cuts[1].overlap);
    }

    #[test]
    fn the_scanner_reads_what_sscanf_reads() {
        assert_eq!(scan_cut_index("  [cut_12]\r"), Some(12));
        assert_eq!(scan_cut_index("[cut_ -3]"), Some(-3));
        assert_eq!(scan_cut_index("[cut_x]"), None);
        assert_eq!(scan_cut_index("; [cut_1]"), None);
        assert_eq!(scan_hide("hide=1,2,3,4,5,6"), Some([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
        assert_eq!(
            scan_hide(" hide = -1.5 , 2e1 , .5 , 4. , +5 , 6\r"),
            Some([-1.5, 20.0, 0.5, 4.0, 5.0, 6.0])
        );
        assert_eq!(scan_hide("hide = 1,2,3,4,5"), None);
        assert_eq!(scan_hide("hides = 1,2,3,4,5,6"), None);
    }
}
