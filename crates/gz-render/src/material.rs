//! The editor's materials and the one table engine shader names map through.

use smol_str::SmolStr;

use crate::color::Color;

/// The small set of materials the editor draws everything with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EditorMaterial {
    Opaque,
    AlphaTest,
    Blended,
    Additive,
    /// Lit by itself: drawn as it is in every view mode.
    SelfLit,
    /// The base texture times a tiled detail texture.
    Terrain,
    /// Trees and LOD billboards: two-sided, alpha-tested, no wind.
    Tree,
    Water,
    /// Drawn behind everything, centred on the camera.
    Sky,
    Particle,
    UnlitColor,
    /// The mesh's edges.
    Wireframe,
    /// Not a surface's: the selection outline pass.
    SelectionOutline,
    /// Its colour times its alpha added (`D3DBLEND_SRCALPHA`, `D3DBLEND_ONE`).
    AlphaAdd,
    /// What is behind times its colour (`D3DBLEND_DESTCOLOR`, `D3DBLEND_ZERO`).
    Multiply,
    /// What is behind times twice its colour (`D3DBLEND_DESTCOLOR`, `D3DBLEND_SRCCOLOR`): grey
    /// leaves it as it is.
    Multiply2x,
    /// A distortion the R1 renderer the editor uses does not draw: nothing.
    Distort,
}

/// When a material is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pass {
    Opaque,
    AlphaTest,
    Sky,
    /// Sorted back to front.
    Blended,
    Overlay,
}

const NAMES: [(EditorMaterial, &str); 17] = [
    (EditorMaterial::Opaque, "opaque"),
    (EditorMaterial::AlphaTest, "alpha_test"),
    (EditorMaterial::Blended, "blended"),
    (EditorMaterial::Additive, "additive"),
    (EditorMaterial::SelfLit, "self_lit"),
    (EditorMaterial::Terrain, "terrain"),
    (EditorMaterial::Tree, "tree"),
    (EditorMaterial::Water, "water"),
    (EditorMaterial::Sky, "sky"),
    (EditorMaterial::Particle, "particle"),
    (EditorMaterial::UnlitColor, "unlit_color"),
    (EditorMaterial::Wireframe, "wireframe"),
    (EditorMaterial::SelectionOutline, "selection_outline"),
    (EditorMaterial::AlphaAdd, "alpha_add"),
    (EditorMaterial::Multiply, "multiply"),
    (EditorMaterial::Multiply2x, "multiply_2x"),
    (EditorMaterial::Distort, "distort"),
];

impl EditorMaterial {
    #[must_use]
    pub fn name(self) -> &'static str {
        NAMES.iter().find(|(m, _)| *m == self).map_or("opaque", |(_, n)| n)
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        NAMES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name.trim())).map(|(m, _)| *m)
    }

    #[must_use]
    pub fn pass(self) -> Pass {
        match self {
            Self::Opaque | Self::SelfLit | Self::Terrain | Self::UnlitColor | Self::Wireframe => Pass::Opaque,
            Self::AlphaTest | Self::Tree => Pass::AlphaTest,
            Self::Sky => Pass::Sky,
            Self::Blended
            | Self::Additive
            | Self::Water
            | Self::Particle
            | Self::AlphaAdd
            | Self::Multiply
            | Self::Multiply2x
            | Self::Distort => Pass::Blended,
            Self::SelectionOutline => Pass::Overlay,
        }
    }

    /// Whether the material is drawn from both sides whatever the surface says.
    #[must_use]
    pub fn two_sided(self) -> bool {
        matches!(
            self,
            Self::Tree
                | Self::Additive
                | Self::Water
                | Self::Particle
                | Self::Sky
                | Self::AlphaAdd
                | Self::Multiply
                | Self::Multiply2x
        )
    }

    /// The alpha under which a pixel is dropped: the cut of an alpha-tested surface, and where a
    /// blended one stops counting for the selection.
    #[must_use]
    pub fn default_alpha_ref(self) -> f32 {
        match self {
            Self::AlphaTest | Self::Tree => 0.5,
            _ => 0.02,
        }
    }
}

/// The mapping shipped with the editor: an engine shader name (a surface's shader in `.object`,
/// a visual's in OGF, a blender of `shaders.xr`) to the material that draws it. The first
/// pattern that matches wins; `*` matches any run of characters, case and slash direction do
/// not matter. Adding a mapping is one line.
pub const DEFAULT_RULES: &[(&str, EditorMaterial)] = &[
    ("editor\\wire*", EditorMaterial::Wireframe),
    ("editor\\*", EditorMaterial::UnlitColor),
    ("sky\\*", EditorMaterial::Sky),
    ("*skydome*", EditorMaterial::Sky),
    ("effects\\sky*", EditorMaterial::Sky),
    ("clouds*", EditorMaterial::Sky),
    ("*water*", EditorMaterial::Water),
    ("terrain\\*", EditorMaterial::Terrain),
    ("*lightplanes*", EditorMaterial::Additive),
    ("effects\\glow*", EditorMaterial::Additive),
    // the particle blender's modes (`Blender_Particle`): set, blend, add, mul, alpha-add; the
    // x-shaders of shaders\r1 add or distort
    ("particles\\alpha_add*", EditorMaterial::AlphaAdd),
    ("particles\\xadd*", EditorMaterial::Additive),
    ("particles\\add*", EditorMaterial::Additive),
    ("particles\\dark*", EditorMaterial::Multiply),
    ("particles\\set*", EditorMaterial::AlphaTest),
    ("particles\\*distort*", EditorMaterial::Distort),
    ("*_xadd*", EditorMaterial::Additive),
    ("particles\\*", EditorMaterial::Particle),
    ("effects\\wallmarkmult*", EditorMaterial::Multiply2x),
    ("effects\\wallmark*", EditorMaterial::Blended),
    ("*selflight*", EditorMaterial::SelfLit),
    ("selflight*", EditorMaterial::SelfLit),
    ("flora\\*", EditorMaterial::Tree),
    ("trees\\*", EditorMaterial::Tree),
    ("details\\*", EditorMaterial::Tree),
    ("def_shaders\\lod*", EditorMaterial::Tree),
    ("*\\lod*", EditorMaterial::Tree),
    ("def_shaders\\def_trans*", EditorMaterial::Blended),
    ("models\\pautina*", EditorMaterial::Blended),
    ("models\\xglass*", EditorMaterial::Blended),
    ("models\\xwindows*", EditorMaterial::Blended),
    ("*glass*", EditorMaterial::Blended),
    ("*_aref*", EditorMaterial::AlphaTest),
    ("*aref*", EditorMaterial::AlphaTest),
    ("def_shaders\\def_vertex*", EditorMaterial::Opaque),
    ("models\\*", EditorMaterial::Opaque),
    ("default", EditorMaterial::Opaque),
];

/// Engine shader names to editor materials. Data: the rules are tried in order, an unknown
/// name gets the default.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialTable {
    rules: Vec<(SmolStr, EditorMaterial)>,
    default: EditorMaterial,
}

impl Default for MaterialTable {
    fn default() -> Self {
        let mut t = Self::new(EditorMaterial::Opaque);
        for (pattern, m) in DEFAULT_RULES {
            t.push(pattern, *m);
        }
        t
    }
}

impl MaterialTable {
    /// A table with no rules.
    #[must_use]
    pub fn new(default: EditorMaterial) -> Self {
        Self { rules: Vec::new(), default }
    }

    /// Adds a rule after the others.
    pub fn push(&mut self, pattern: &str, material: EditorMaterial) {
        self.rules.push((normalize(pattern), material));
    }

    /// Adds a rule before the others: it wins over every rule already there.
    pub fn prepend(&mut self, pattern: &str, material: EditorMaterial) {
        self.rules.insert(0, (normalize(pattern), material));
    }

    #[must_use]
    pub fn default_material(&self) -> EditorMaterial {
        self.default
    }

    #[must_use]
    pub fn rules(&self) -> &[(SmolStr, EditorMaterial)] {
        &self.rules
    }

    /// The material a shader name is drawn with.
    #[must_use]
    pub fn classify(&self, shader: &str) -> EditorMaterial {
        let name = normalize(shader);
        self.rules.iter().find(|(p, _)| glob(p.as_bytes(), name.as_bytes())).map_or(self.default, |(_, m)| *m)
    }

    /// Rules from text, one `pattern = material` a line (`;` and `#` start comments), placed
    /// before the shipped ones; `* = material` sets the default.
    pub fn parse_overrides(&mut self, text: &str) -> Result<(), String> {
        let mut added = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split([';', '#']).next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((pattern, material)) = line.split_once('=') else {
                return Err(format!("line {}: a rule is \"pattern = material\"", n + 1));
            };
            let Some(m) = EditorMaterial::from_name(material) else {
                return Err(format!("line {}: \"{}\" is not an editor material", n + 1, material.trim()));
            };
            if pattern.trim() == "*" {
                self.default = m;
            } else {
                added.push((normalize(pattern.trim()), m));
            }
        }
        added.extend(std::mem::take(&mut self.rules));
        self.rules = added;
        Ok(())
    }
}

fn normalize(name: &str) -> SmolStr {
    let mut s = String::with_capacity(name.len());
    for c in name.trim().chars() {
        s.push(if c == '/' { '\\' } else { c.to_ascii_lowercase() });
    }
    SmolStr::from(s)
}

/// `*` matches any run; everything else matches itself.
fn glob(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(c) if *c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pattern.get(p..).is_some_and(|rest| rest.iter().all(|c| *c == b'*'))
}

/// A material: what draws it and with which textures (names as the game names them, without
/// an extension).
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialDesc {
    pub kind: EditorMaterial,
    pub base: Option<SmolStr>,
    /// The tiled detail texture of [`EditorMaterial::Terrain`].
    pub detail: Option<SmolStr>,
    pub color: Color,
    pub two_sided: bool,
    pub alpha_ref: f32,
    /// How many times the detail texture repeats per repeat of the base.
    pub detail_scale: f32,
}

impl MaterialDesc {
    #[must_use]
    pub fn new(kind: EditorMaterial) -> Self {
        Self {
            kind,
            base: None,
            detail: None,
            color: Color::WHITE,
            two_sided: kind.two_sided(),
            alpha_ref: kind.default_alpha_ref(),
            detail_scale: 16.0,
        }
    }

    /// A material with a base texture.
    #[must_use]
    pub fn textured(kind: EditorMaterial, texture: &str) -> Self {
        Self { base: (!texture.is_empty()).then(|| texture_key(texture)), ..Self::new(kind) }
    }

    /// A flat colour.
    #[must_use]
    pub fn color(kind: EditorMaterial, color: Color) -> Self {
        Self { color, ..Self::new(kind) }
    }

    #[must_use]
    pub fn with_two_sided(mut self, two_sided: bool) -> Self {
        self.two_sided = two_sided || self.kind.two_sided();
        self
    }

    pub(crate) fn key(&self) -> MaterialKey {
        MaterialKey {
            kind: self.kind,
            base: self.base.clone(),
            detail: self.detail.clone(),
            color: self.color,
            two_sided: self.two_sided,
            alpha_ref: self.alpha_ref.to_bits(),
            detail_scale: self.detail_scale.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MaterialKey {
    kind: EditorMaterial,
    base: Option<SmolStr>,
    detail: Option<SmolStr>,
    color: Color,
    two_sided: bool,
    alpha_ref: u32,
    detail_scale: u32,
}

/// A texture name as the caches key it: lower case, back slashes, no picture extension.
#[must_use]
pub(crate) fn texture_key(name: &str) -> SmolStr {
    strip_ext(&normalize(name), &["dds", "tga", "png", "jpg", "jpeg", "bmp"])
}

/// A model name as the caches key it: lower case, back slashes, no `.object` / `.ogf`.
#[must_use]
pub(crate) fn model_key(name: &str) -> SmolStr {
    strip_ext(&normalize(name), &["object", "ogf"])
}

fn strip_ext(n: &str, known: &[&str]) -> SmolStr {
    match n.rsplit_once('.') {
        Some((stem, ext)) if known.contains(&ext) => SmolStr::from(stem),
        _ => SmolStr::from(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_table_classifies() {
        let t = MaterialTable::default();
        let cases = [
            ("default", EditorMaterial::Opaque),
            ("def_shaders\\def_aref", EditorMaterial::AlphaTest),
            ("def_shaders/def_aref_v", EditorMaterial::AlphaTest),
            ("def_shaders\\def_trans", EditorMaterial::Blended),
            ("def_shaders\\def_vertex", EditorMaterial::Opaque),
            ("models\\model", EditorMaterial::Opaque),
            ("models\\pautina", EditorMaterial::Blended),
            ("models\\selflight", EditorMaterial::SelfLit),
            ("Effects\\LightPlanes", EditorMaterial::Additive),
            ("effects\\water", EditorMaterial::Water),
            ("effects\\waterryaska", EditorMaterial::Water),
            ("terrain\\terrain_escape", EditorMaterial::Terrain),
            ("flora\\leaf_wave", EditorMaterial::Tree),
            ("details\\blend", EditorMaterial::Tree),
            ("def_shaders\\lod", EditorMaterial::Tree),
            ("particles\\add", EditorMaterial::Additive),
            ("particles\\xadd", EditorMaterial::Additive),
            ("particles\\alpha_add", EditorMaterial::AlphaAdd),
            ("particles\\dark", EditorMaterial::Multiply),
            ("particles\\set", EditorMaterial::AlphaTest),
            ("particles\\xdistort", EditorMaterial::Distort),
            ("particles\\xblend", EditorMaterial::Particle),
            ("effects\\wallmarkmult", EditorMaterial::Multiply2x),
            ("effects\\wallmarkblend", EditorMaterial::Blended),
            ("particles\\blend", EditorMaterial::Particle),
            ("editor\\wire", EditorMaterial::Wireframe),
            ("editor\\selection", EditorMaterial::UnlitColor),
            ("sky\\sky_5_cube", EditorMaterial::Sky),
            ("something\\nobody\\knows", EditorMaterial::Opaque),
            ("", EditorMaterial::Opaque),
        ];
        for (name, want) in cases {
            assert_eq!(t.classify(name), want, "{name}");
        }
    }

    #[test]
    fn rules_are_data() {
        let mut t = MaterialTable::default();
        t.prepend("models\\model", EditorMaterial::UnlitColor);
        assert_eq!(t.classify("models\\model"), EditorMaterial::UnlitColor);
        assert_eq!(t.classify("models\\model_x"), EditorMaterial::Opaque);
        t.parse_overrides("; a comment\nmy\\*  = self_lit\n* = unlit_color\n").unwrap();
        assert_eq!(t.classify("my\\shader"), EditorMaterial::SelfLit);
        assert_eq!(t.classify("nobody\\knows"), EditorMaterial::UnlitColor);
        assert!(t.parse_overrides("broken line").is_err());
        assert!(t.parse_overrides("a = glossy").is_err());
        for (m, n) in NAMES {
            assert_eq!(EditorMaterial::from_name(n), Some(m));
            assert_eq!(m.name(), n);
        }
    }

    #[test]
    fn globs() {
        assert!(glob(b"a*c", b"abbbc"));
        assert!(glob(b"*", b""));
        assert!(glob(b"a*", b"a"));
        assert!(!glob(b"a*c", b"abbbd"));
        assert!(glob(b"*b*", b"abc"));
        assert!(!glob(b"abc", b"abcd"));
        assert!(glob(b"*_aref*", b"def_shaders\\def_aref_v"));
    }

    #[test]
    fn passes_and_keys() {
        assert_eq!(EditorMaterial::Tree.pass(), Pass::AlphaTest);
        assert_eq!(EditorMaterial::Water.pass(), Pass::Blended);
        assert_eq!(EditorMaterial::Sky.pass(), Pass::Sky);
        assert!(MaterialDesc::new(EditorMaterial::Tree).two_sided);
        assert_eq!(texture_key("Terrain/Terrain_Escape.DDS"), "terrain\\terrain_escape");
        assert_eq!(texture_key("trees\\trees_bark_a"), "trees\\trees_bark_a");
        assert_eq!(texture_key("act\\act_stalker.v2"), "act\\act_stalker.v2");
        assert_eq!(model_key("Dynamics/Box.OGF"), "dynamics\\box");
        assert_eq!(model_key("trees\\tree_01.object"), "trees\\tree_01");
        assert_eq!(MaterialDesc::textured(EditorMaterial::Opaque, "a/b").base.as_deref(), Some("a\\b"));
    }
}
