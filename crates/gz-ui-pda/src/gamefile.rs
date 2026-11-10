//! What the preview reads of the game's own files, as the C++ preview read them
//! (`EditorPdaPreview.cpp`): plain text scans, not the engine's parsers - the device of
//! `pda.xml` / `pda_16.xml`, a level's map from its `level.ltx`, a value of a plain ltx text.
//! Pure functions over text.

/// C `atof`: the longest number at the start of the text (after white space), 0 without one.
#[must_use]
pub fn atof(s: &str) -> f32 {
    scan_float(s).map_or(0.0, |(v, _)| v)
}

/// C `atoi`: the whole number at the start of the text, 0 without one.
#[must_use]
pub fn atoi(s: &str) -> i64 {
    let t = s.trim_start_matches([' ', '\t', '\r', '\n']);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, t.get(1..).unwrap_or("")),
        Some(b'+') => (false, t.get(1..).unwrap_or("")),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        v = v.saturating_mul(10).saturating_add(i64::from(b - b'0'));
    }
    if neg { -v } else { v }
}

/// A `strtod` prefix: the number and the bytes it took (white space before it included).
#[must_use]
pub fn scan_float(s: &str) -> Option<(f32, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    while b.get(i).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)) {
        i += 1;
    }
    let start = i;
    if matches!(b.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits_from = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i - digits_from;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let frac = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        digits += i - frac;
    }
    if digits == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = j;
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > exp {
            i = j;
        }
    }
    s.get(start..i).and_then(|t| t.parse::<f64>().ok()).map(|v| (v as f32, i))
}

/// `sscanf(s, "%f , %f , ...")`: `n` numbers separated by commas (white space around them
/// allowed). `None` unless all `n` are there.
#[must_use]
pub fn comma_floats(s: &str, n: usize) -> Option<Vec<f32>> {
    let mut out = Vec::with_capacity(n);
    let mut rest = s;
    for k in 0..n {
        if k > 0 {
            let t = rest.trim_start_matches([' ', '\t', '\r', '\n']);
            rest = t.strip_prefix(',')?;
        }
        let (v, used) = scan_float(rest)?;
        out.push(v);
        rest = rest.get(used..).unwrap_or("");
    }
    Some(out)
}

/// `NqUtil::Trim`: spaces, tabs and line ends off both ends.
#[must_use]
pub fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\r', '\n'])
}

/// The text without `<!-- ... -->` (an unclosed comment runs to the end).
#[must_use]
pub fn strip_comments(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut pos = 0;
    while pos < raw.len() {
        let Some(c) = raw.get(pos..).and_then(|r| r.find("<!--")).map(|c| pos + c) else {
            text.push_str(raw.get(pos..).unwrap_or(""));
            break;
        };
        text.push_str(raw.get(pos..c).unwrap_or(""));
        pos = raw.get(c + 4..).and_then(|r| r.find("-->")).map_or(raw.len(), |e| c + 4 + e + 3);
    }
    text
}

/// `TagAttr`: the value of `name="..."` inside a tag, `""` when it is not there; a name that is
/// the tail of a longer one does not count.
#[must_use]
pub fn tag_attr(tag: &str, name: &str) -> String {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(at) = tag.get(from..).and_then(|r| r.find(&key)).map(|i| from + i) {
        if at == 0 || matches!(tag.as_bytes().get(at - 1), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            let v = at + key.len();
            return tag.get(v..).and_then(|r| r.find('"')).and_then(|e| tag.get(v..v + e)).unwrap_or("").to_owned();
        }
        from = at + key.len();
    }
    String::new()
}

fn find_from(text: &str, pat: &str, from: usize) -> Option<usize> {
    text.get(from..).and_then(|r| r.find(pat)).map(|i| from + i)
}

/// The value of `key` in `[section]` of a plain ltx text (`LtxValue`): the first header of that
/// name standing at a line's start (or after white space), the first `key = value` line of it,
/// `;` comments cut; a key it lacks is looked for in its `[child]:parent`, four levels deep.
#[must_use]
pub fn ltx_value(text: &str, section: &str, key: &str) -> Option<String> {
    ltx_value_at(text, section, key, 0)
}

fn ltx_value_at(text: &str, section: &str, key: &str, depth: u32) -> Option<String> {
    let head = format!("[{section}]");
    let bytes = text.as_bytes();
    let mut p = 0;
    while let Some(found) = find_from(text, &head, p) {
        p = found;
        if p > 0 && !matches!(bytes.get(p - 1), Some(b'\n' | b'\r' | b' ' | b'\t')) {
            p += 1;
            continue;
        }
        let line_end = find_from(text, "\n", p);
        let header = text.get(p..line_end.unwrap_or(text.len())).unwrap_or("");
        let mut pos = line_end.map_or(text.len(), |e| e + 1);
        while pos < text.len() {
            let e = find_from(text, "\n", pos).unwrap_or(text.len());
            let mut line = text.get(pos..e).unwrap_or("");
            pos = e + 1;
            if let Some(sc) = line.find(';') {
                line = line.get(..sc).unwrap_or("");
            }
            let line = trim(line);
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                break;
            }
            let Some(eq) = line.find('=') else { continue };
            if trim(line.get(..eq).unwrap_or("")) == key {
                return Some(trim(line.get(eq + 1..).unwrap_or("")).to_owned());
            }
        }
        // [child]:parent
        if let Some(colon) = header.find("]:")
            && depth < 4
        {
            let parent = trim(header.get(colon + 2..).unwrap_or(""));
            if !parent.is_empty() {
                return ltx_value_at(text, parent, key, depth + 1);
            }
        }
        return None;
    }
    None
}

/// Every `key = value` of `[section]` the way [`ltx_value`] reads each of them: for a section
/// whose keys are asked one by one (a font's 256 glyphs) without scanning the text 256 times.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LtxSection {
    /// In file order; the first of a key is the one [`ltx_value`] answers.
    pub values: Vec<(String, String)>,
    /// The `[child]:parent` chain, nearest first, as far as it is followed.
    pub parents: Vec<LtxSection>,
}

impl LtxSection {
    /// The section of `text`, or `None` when no header of it stands where [`ltx_value`] looks.
    #[must_use]
    pub fn read(text: &str, section: &str) -> Option<Self> {
        Self::read_at(text, section, 0)
    }

    fn read_at(text: &str, section: &str, depth: u32) -> Option<Self> {
        let head = format!("[{section}]");
        let bytes = text.as_bytes();
        let mut p = 0;
        while let Some(found) = find_from(text, &head, p) {
            p = found;
            if p > 0 && !matches!(bytes.get(p - 1), Some(b'\n' | b'\r' | b' ' | b'\t')) {
                p += 1;
                continue;
            }
            let line_end = find_from(text, "\n", p);
            let header = text.get(p..line_end.unwrap_or(text.len())).unwrap_or("");
            let mut pos = line_end.map_or(text.len(), |e| e + 1);
            let mut values = Vec::new();
            while pos < text.len() {
                let e = find_from(text, "\n", pos).unwrap_or(text.len());
                let mut line = text.get(pos..e).unwrap_or("");
                pos = e + 1;
                if let Some(sc) = line.find(';') {
                    line = line.get(..sc).unwrap_or("");
                }
                let line = trim(line);
                if line.is_empty() {
                    continue;
                }
                if line.starts_with('[') {
                    break;
                }
                let Some(eq) = line.find('=') else { continue };
                values.push((trim(line.get(..eq).unwrap_or("")).to_owned(), trim(line.get(eq + 1..).unwrap_or("")).to_owned()));
            }
            let mut parents = Vec::new();
            if let Some(colon) = header.find("]:")
                && depth < 4
            {
                let parent = trim(header.get(colon + 2..).unwrap_or(""));
                if !parent.is_empty()
                    && let Some(up) = Self::read_at(text, parent, depth + 1)
                {
                    parents.push(up);
                }
            }
            return Some(Self { values, parents });
        }
        None
    }

    /// The value of `key`, as [`ltx_value`] would answer it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        if let Some((_, v)) = self.values.iter().find(|(k, _)| k == key) {
            return Some(v);
        }
        self.parents.first().and_then(|p| p.get(key))
    }
}

/// A rectangle in the game's 1024 x 768 units (`Frect`: corners).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FRect {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl FRect {
    #[must_use]
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    #[must_use]
    pub fn width(&self) -> f32 {
        self.x2 - self.x1
    }

    #[must_use]
    pub fn height(&self) -> f32 {
        self.y2 - self.y1
    }

    /// The smallest rectangle holding both.
    #[must_use]
    pub fn union(&self, o: &FRect) -> FRect {
        FRect::new(self.x1.min(o.x1), self.y1.min(o.y1), self.x2.max(o.x2), self.y2.max(o.y2))
    }
}

/// What one node of `pda.xml` says (`XmlNode`): its rectangle, the texture of its first
/// `<texture...>` (or `<tex_name...>`) inside it, and that tag's `a`.
#[derive(Clone, Debug, PartialEq)]
pub struct XmlNode {
    pub rect: FRect,
    /// Empty when the node holds no texture tag.
    pub texture: String,
    /// The texture tag's `a`; 255 when it has none or there is no texture tag.
    pub alpha: u32,
}

/// The node `<name ...>` of an XML text (comments already stripped): its rectangle and texture.
/// `tex_name` names the texture tag (`texture_e` of a button), `texture` by default; a texture
/// tag counts only before the node's closing tag.
#[must_use]
pub fn xml_node(text: &str, name: &str, tex_name: Option<&str>) -> Option<XmlNode> {
    let open = format!("<{name}");
    let bytes = text.as_bytes();
    let mut p = text.find(&open);
    // a whole tag name, not the head of a longer one
    while let Some(at) = p {
        if matches!(bytes.get(at + open.len()), Some(b' ' | b'>' | b'\t' | b'\r' | b'\n')) {
            break;
        }
        p = find_from(text, &open, at + 1);
    }
    let p = p?;
    let gt = find_from(text, ">", p)?;
    let tag = text.get(p..gt).unwrap_or("");
    let x = atof(&tag_attr(tag, "x"));
    let y = atof(&tag_attr(tag, "y"));
    let rect = FRect::new(x, y, x + atof(&tag_attr(tag, "width")), y + atof(&tag_attr(tag, "height")));
    let tex_open = format!("<{}", tex_name.unwrap_or("texture"));
    let close_tag = find_from(text, &format!("</{name}"), gt);
    let mut texture = String::new();
    let mut alpha = 255;
    if let Some(t) = find_from(text, &tex_open, gt)
        && close_tag.is_none_or(|c| t < c)
        && let Some(tgt) = find_from(text, ">", t)
        && let Some(tend) = find_from(text, "<", tgt + 1)
    {
        texture = trim(text.get(tgt + 1..tend).unwrap_or("")).to_owned();
        let a = tag_attr(text.get(t..tgt).unwrap_or(""), "a");
        alpha = if a.is_empty() { 255 } else { u32::try_from(atoi(&a)).unwrap_or(0) };
    }
    Some(XmlNode { rect, texture, alpha })
}

/// One aspect of the device: the PDA's art, its screen (where the noise lies and the layout is
/// drawn) and the close button, in the game's 1024 x 768 units.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceSide {
    pub bg: FRect,
    pub noise: FRect,
    pub close: FRect,
    pub bg_tex: String,
    pub noise_tex: String,
    pub close_tex: String,
    pub noise_alpha: u32,
}

/// The device on both aspects: `[0]` is 4:3 (`pda.xml`), `[1]` 16:9 (`pda_16.xml`).
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub sides: [DeviceSide; 2],
}

impl Default for Device {
    /// Dead Air's own values: what a game without the files would show.
    fn default() -> Self {
        let side = |bg: FRect, noise: FRect, close: FRect| DeviceSide {
            bg,
            noise,
            close,
            bg_tex: "ui_inGame2_pda_texture".to_owned(),
            noise_tex: "ui_pda2_noice".to_owned(),
            close_tex: "ui_inGame2_pda_offline_e".to_owned(),
            noise_alpha: 70,
        };
        Self {
            sides: [
                side(FRect::new(0.0, 0.0, 1024.0, 768.0), FRect::new(25.0, 33.0, 998.0, 716.0), FRect::new(973.0, 26.0, 1006.0, 60.0)),
                side(FRect::new(102.0, 0.0, 921.0, 768.0), FRect::new(122.0, 33.0, 900.0, 716.0), FRect::new(880.0, 26.0, 906.0, 60.0)),
            ],
        }
    }
}

impl Device {
    /// Lays an aspect's XML over the defaults (`Device()`): `wide` is `pda_16.xml`.
    pub fn apply_xml(&mut self, wide: bool, raw: &str) {
        let Some(side) = self.sides.get_mut(usize::from(wide)) else { return };
        let xml = strip_comments(raw);
        if let Some(n) = xml_node(&xml, "background_static", None) {
            side.bg = n.rect;
            if !n.texture.is_empty() {
                side.bg_tex = n.texture;
            }
        }
        if let Some(n) = xml_node(&xml, "noice_static", None) {
            side.noise = n.rect;
            if !n.texture.is_empty() {
                side.noise_tex = n.texture;
            }
            side.noise_alpha = n.alpha;
        }
        if let Some(n) = xml_node(&xml, "close_button", Some("texture_e")) {
            side.close = n.rect;
            if !n.texture.is_empty() {
                side.close_tex = n.texture;
            }
        }
    }

    #[must_use]
    pub fn side(&self, wide: bool) -> &DeviceSide {
        let [standard, wide_side] = &self.sides;
        if wide { wide_side } else { standard }
    }

    /// The device itself (`DeviceRect`): its art, the screen and the close button. Fit fits this,
    /// not the game's whole screen - on 16:9 the art covers only the middle of it.
    #[must_use]
    pub fn rect(&self, wide: bool) -> FRect {
        let d = self.side(wide);
        let r = d.bg.union(&d.noise).union(&d.close);
        if r.width() < 1.0 || r.height() < 1.0 { FRect::new(0.0, 0.0, 1024.0, 768.0) } else { r }
    }
}

/// A level's map (`SLevelMap`): its texture and the world rectangle it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelMap {
    /// Lower case, textures-relative.
    pub tex: String,
    /// x1, z1 (bottom), x2, z2 (top) in world units.
    pub bound: [f32; 4],
}

/// `[level_map] texture` and `bound_rect` of a `level.ltx` (`LevelMap`); `None` without either,
/// or for a rectangle that is not one.
#[must_use]
pub fn parse_level_map(ltx: &str) -> Option<LevelMap> {
    let tex = ltx_value(ltx, "level_map", "texture")?;
    let rect = ltx_value(ltx, "level_map", "bound_rect")?;
    // `100.0f` was written by C programmers
    let r = rect.replace('f', " ");
    let v = comma_floats(&r, 4)?;
    let (a, b, c, d) = (*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?);
    (c > a && d > b).then(|| LevelMap { tex: trim(&tex).to_ascii_lowercase(), bound: [a, b, c, d] })
}
