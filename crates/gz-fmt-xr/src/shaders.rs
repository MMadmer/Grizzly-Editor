//! `shaders.xr`
//! and `shaders_xrlc.xr`.

use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::{Encoding, Result, Text, invalid, numbered};

/// `WaveForm`: a function and its four arguments (20 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaveForm {
    pub function: u32,
    pub args: [f32; 4],
}

impl WaveForm {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { function: r.r_u32()?, args: [r.r_f32()?, r.r_f32()?, r.r_f32()?, r.r_f32()?] })
    }
}

/// A texture coordinate transform.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixDef {
    pub name: SmolStr,
    pub mode: u32,
    pub tcm: u32,
    pub scale_u: WaveForm,
    pub scale_v: WaveForm,
    pub rotate: WaveForm,
    pub scroll_u: WaveForm,
    pub scroll_v: WaveForm,
}

/// One blender: its description and the rest of its chunk, the property stream its class reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Blender {
    /// The class id: eight characters.
    pub class_id: [u8; 8],
    pub name: SmolStr,
    pub computer: SmolStr,
    pub time: u32,
    pub version: u16,
    /// Everything after the 176-byte description.
    pub props: Vec<u8>,
}

/// What geometry a blender's class draws: the game compiles a shader for the vertices its class
/// expects, and geometry of another kind is drawn with nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draws {
    /// A level's static geometry, lit by lightmaps or by its vertices.
    Level,
    /// A model: an object with bones, or one placed by a matrix.
    Model,
    /// Anything else: effects, details, trees, particles, the editor's own.
    Other,
}

impl Draws {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Model => "model",
            Self::Other => "other",
        }
    }
}

/// A value of a blender's property stream.
#[derive(Clone, Debug, PartialEq)]
pub enum BlenderProp {
    Text(String),
    Integer(i32),
    Float(f32),
    Bool(bool),
    /// The selected item of a list.
    Token(u32),
}

impl Blender {
    /// The class id as text (`"BLENDER"`, `"LmBmmD  "`, ...), trailing blanks kept.
    #[must_use]
    pub fn class_name(&self) -> String {
        // CLASS_ID is a u64 whose first character is its highest byte
        let mut b = self.class_id;
        b.reverse();
        b.iter().map(|c| char::from(*c)).collect()
    }

    /// What geometry the class draws, by its id: the lightmapped and the vertex-lit classes of a
    /// level (`LM`, `LM_AREF`, `LmEbB`, `BmmD`, `V`, `V_AREF`), the classes of a model (`MODEL`,
    /// `MODELEbB`).
    #[must_use]
    pub fn draws(&self) -> Draws {
        match self.class_name().trim_end() {
            "LM" | "LM_AREF" | "LmEbB" | "LmBmmD" | "BmmD" | "BmmDold" | "V" | "V_AREF" => Draws::Level,
            "MODEL" | "MODELEbB" => Draws::Model,
            _ => Draws::Other,
        }
    }

    /// The named properties of the blender, in the stream's order (the markers between them left
    /// out). The stream is the game's own: a property of a kind this reader does not know ends
    /// the list there.
    #[must_use]
    pub fn properties(&self) -> Vec<(String, BlenderProp)> {
        // the kinds of a property: a marker has no value, a name of a matrix, a constant or a
        // texture is 64 bytes, a number comes with its limits, a list with its items
        const MARKER: u32 = 0;
        const MATRIX: u32 = 1;
        const CONSTANT: u32 = 2;
        const TEXTURE: u32 = 3;
        const INTEGER: u32 = 4;
        const FLOAT: u32 = 5;
        const BOOL: u32 = 6;
        const TOKEN: u32 = 7;
        const NAME: usize = 64;
        const TOKEN_ITEM: usize = 4 + NAME;
        let mut out = Vec::new();
        let mut r = Reader::new(&self.props);
        while !r.eof() {
            let Ok(kind) = r.r_u32() else { break };
            let Ok(name) = r.r_stringz_raw() else { break };
            let name = String::from_utf8_lossy(name).into_owned();
            let value = match kind {
                MARKER => continue,
                MATRIX | CONSTANT | TEXTURE => match r.take(NAME) {
                    Ok(b) => {
                        let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
                        BlenderProp::Text(String::from_utf8_lossy(&b[..end]).into_owned())
                    }
                    Err(_) => break,
                },
                INTEGER => match (r.r_i32(), r.advance(8)) {
                    (Ok(v), Ok(())) => BlenderProp::Integer(v),
                    _ => break,
                },
                FLOAT => match (r.r_f32(), r.advance(8)) {
                    (Ok(v), Ok(())) => BlenderProp::Float(v),
                    _ => break,
                },
                BOOL => match r.r_u32() {
                    Ok(v) => BlenderProp::Bool(v != 0),
                    Err(_) => break,
                },
                TOKEN => match (r.r_u32(), r.r_u32()) {
                    (Ok(selected), Ok(count)) => {
                        if r.advance((count as usize).saturating_mul(TOKEN_ITEM)).is_err() {
                            break;
                        }
                        BlenderProp::Token(selected)
                    }
                    _ => break,
                },
                _ => break,
            };
            out.push((name, value));
        }
        out
    }

    /// Whether the blender cuts by its texture's alpha (`test`) and whether it blends by it
    /// (`blend`): a model's "Use alpha-channel", a level blender's "Alpha ref" above zero and
    /// "Alpha-blend".
    #[must_use]
    pub fn alpha(&self) -> (bool, bool) {
        let props = self.properties();
        let flag = |name: &str| {
            props.iter().any(|(n, v)| n.eq_ignore_ascii_case(name) && *v == BlenderProp::Bool(true))
        };
        let aref = props.iter().find_map(|(n, v)| match v {
            BlenderProp::Integer(x) if n.eq_ignore_ascii_case("Alpha ref") => Some(*x),
            _ => None,
        });
        let blend = flag("Alpha-blend");
        match self.class_name().trim_end() {
            // a model's alpha channel is a cut at its reference
            "MODEL" => (flag("Use alpha-channel"), false),
            "MODELEbB" => (false, blend),
            _ => (aref.is_some_and(|x| x > 0) && !blend, blend),
        }
    }
}

/// `shaders.xr`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shaders {
    /// Name and the R, G, B, A waves.
    pub constants: Vec<(SmolStr, [WaveForm; 4])>,
    pub matrices: Vec<MatrixDef>,
    pub blenders: Vec<Blender>,
    pub encoding: Encoding,
}

const DESC_SIZE: usize = 176;

fn fixed_str(bytes: &[u8], text: &mut Text) -> Result<SmolStr> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut with_nul = bytes[..end].to_vec();
    with_nul.push(0);
    text.r_str(&mut Reader::new(&with_nul))
}

impl Shaders {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let mut text = Text::default();
        let mut out = Self::default();
        if let Some(c) = r.open_chunk(0)? {
            let mut cr = c.reader();
            while !cr.eof() {
                let name = text.r_str(&mut cr)?;
                out.constants.push((
                    name,
                    [
                        WaveForm::read(&mut cr)?,
                        WaveForm::read(&mut cr)?,
                        WaveForm::read(&mut cr)?,
                        WaveForm::read(&mut cr)?,
                    ],
                ));
            }
        }
        if let Some(c) = r.open_chunk(1)? {
            let mut cr = c.reader();
            while !cr.eof() {
                let name = text.r_str(&mut cr)?;
                out.matrices.push(MatrixDef {
                    name,
                    mode: cr.r_u32()?,
                    tcm: cr.r_u32()?,
                    scale_u: WaveForm::read(&mut cr)?,
                    scale_v: WaveForm::read(&mut cr)?,
                    rotate: WaveForm::read(&mut cr)?,
                    scroll_u: WaveForm::read(&mut cr)?,
                    scroll_v: WaveForm::read(&mut cr)?,
                });
            }
        }
        if let Some(c) = r.open_chunk(2)? {
            for b in numbered(&c.reader())? {
                let body = b.bytes();
                if body.len() < DESC_SIZE {
                    return Err(invalid("a blender's description is cut short"));
                }
                let mut d = Reader::new(body);
                let mut class_id = [0u8; 8];
                class_id.copy_from_slice(d.take(8)?);
                let name = fixed_str(d.take(128)?, &mut text)?;
                let computer = fixed_str(d.take(32)?, &mut text)?;
                let time = d.r_u32()?;
                let version = d.r_u16()?;
                out.blenders.push(Blender {
                    class_id,
                    name,
                    computer,
                    time,
                    version,
                    props: body[DESC_SIZE..].to_vec(),
                });
            }
        }
        out.encoding = text.encoding();
        Ok(out)
    }

    #[must_use]
    pub fn blender(&self, name: &str) -> Option<&Blender> {
        self.blenders.iter().find(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// The shader a level's static geometry is drawn with in place of `name`, when `name` is a
    /// shader for models: a lightmapped blender of the library that treats the texture's alpha
    /// the same way - the plain one (`default` when the library has it), the one that cuts, the
    /// one that blends. `None` for a shader that is no model's, or when the library has no
    /// such blender.
    ///
    /// A compiled model placed as a static object is level geometry, lit by lightmaps; its
    /// parts still name the shader the model was made for, which the game compiles for a
    /// model's vertices - and draws nothing with.
    #[must_use]
    pub fn level_twin(&self, name: &str) -> Option<&Blender> {
        let b = self.blender(name)?;
        if b.draws() != Draws::Model {
            return None;
        }
        let want = b.alpha();
        let class = if want == (false, false) { "LM" } else { "LM_AREF" };
        let of_class =
            || self.blenders.iter().filter(move |x| x.class_name().trim_end() == class && x.alpha() == want);
        of_class().find(|x| x.name.eq_ignore_ascii_case("default")).or_else(|| of_class().next())
    }
}

/// One record of `shaders_xrlc.xr` (144 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderXrlc {
    /// The name field as stored: 128 bytes, zero-terminated, what follows kept.
    pub name_raw: [u8; 128],
    pub flags: u32,
    pub vert_translucency: f32,
    pub vert_ambient: f32,
    pub lm_density: f32,
}

impl ShaderXrlc {
    pub const COLLISION: u32 = 1 << 0;
    pub const RENDERING: u32 = 1 << 1;
    pub const OPTIMIZE_UV: u32 = 1 << 2;
    pub const LIGHT_VERTEX: u32 = 1 << 3;
    pub const LIGHT_CAST_SHADOW: u32 = 1 << 4;
    pub const LIGHT_SHARP: u32 = 1 << 5;

    #[must_use]
    pub fn name(&self) -> String {
        let end = self.name_raw.iter().position(|&b| b == 0).unwrap_or(128);
        gz_base::text::decode_guess(&self.name_raw[..end]).into_owned()
    }
}

/// `shaders_xrlc.xr`: a bare array of records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadersXrlc {
    pub shaders: Vec<ShaderXrlc>,
}

impl ShadersXrlc {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        if !bytes.len().is_multiple_of(144) {
            return Err(invalid("shaders_xrlc.xr is not a whole number of shaders"));
        }
        let mut r = Reader::new(bytes);
        let mut shaders = Vec::with_capacity(bytes.len() / 144);
        while !r.eof() {
            let mut name_raw = [0u8; 128];
            name_raw.copy_from_slice(r.take(128)?);
            shaders.push(ShaderXrlc {
                name_raw,
                flags: r.r_u32()?,
                vert_translucency: r.r_f32()?,
                vert_ambient: r.r_f32()?,
                lm_density: r.r_f32()?,
            });
        }
        Ok(Self { shaders })
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(self.shaders.len() * 144);
        for s in &self.shaders {
            w.w(&s.name_raw);
            w.w_u32(s.flags);
            w.w_f32(s.vert_translucency);
            w.w_f32(s.vert_ambient);
            w.w_f32(s.lm_density);
        }
        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blenders_constants_and_matrices_are_read() {
        let mut w = Writer::new();
        w.chunk(0, |w| {
            w.w_stringz("c0");
            for _ in 0..4 {
                w.w_u32(1);
                (0..4).for_each(|_| w.w_f32(0.5));
            }
        });
        w.chunk(1, |w| {
            w.w_stringz("m0");
            w.w_u32(2);
            w.w_u32(3);
            for _ in 0..5 {
                w.w_u32(0);
                (0..4).for_each(|_| w.w_f32(0.0));
            }
        });
        w.chunk(2, |w| {
            w.chunk(0, |w| {
                w.w(b"REDNELB ");
                let mut name = [0u8; 128];
                name[..9].copy_from_slice(b"def_shade");
                w.w(&name);
                w.w(&[0u8; 32]);
                w.w_u32(7);
                w.w_u16(3);
                w.w(&[0, 0]);
                w.w(b"props");
            });
        });
        let s = Shaders::read(&w.into_bytes()).unwrap();
        assert_eq!(s.constants[0].1[3].args, [0.5; 4]);
        assert_eq!((s.matrices[0].mode, s.matrices[0].tcm), (2, 3));
        let b = s.blender("DEF_SHADE").unwrap();
        assert_eq!(
            (b.version, b.time, b.props.as_slice(), b.class_name().as_str()),
            (3, 7, &b"props"[..], " BLENDER")
        );
    }

    fn blender(class: &[u8; 8], name: &str, props: &[(u32, &str, &[u8])]) -> Blender {
        let mut class_id = *class;
        class_id.reverse();
        let mut stream = Vec::new();
        for (kind, key, value) in props {
            stream.extend_from_slice(&kind.to_le_bytes());
            stream.extend_from_slice(key.as_bytes());
            stream.push(0);
            stream.extend_from_slice(value);
        }
        Blender {
            class_id,
            name: name.into(),
            computer: SmolStr::default(),
            time: 0,
            version: 0,
            props: stream,
        }
    }

    fn int(v: i32) -> Vec<u8> {
        [v.to_le_bytes(), 0i32.to_le_bytes(), 255i32.to_le_bytes()].concat()
    }

    #[test]
    fn a_models_shader_has_a_twin_for_level_geometry_that_treats_alpha_the_same_way() {
        let (yes, no) = (1u32.to_le_bytes(), 0u32.to_le_bytes());
        let mut texture = [0u8; 64];
        texture[..6].copy_from_slice(b"$base0");
        let lib = Shaders {
            blenders: vec![
                blender(b"LM      ", "default_hm", &[(0, "General", &[]), (4, "Priority", &int(1))]),
                blender(
                    b"LM      ",
                    "default",
                    &[(3, "Name", &texture), (7, "Tessellation", &[no, no].concat())],
                ),
                blender(
                    b"LM_AREF ",
                    "def_shaders\\def_trans",
                    &[(4, "Alpha ref", &int(0)), (6, "Alpha-blend", &yes)],
                ),
                blender(
                    b"LM_AREF ",
                    "def_shaders\\def_aref",
                    &[(4, "Alpha ref", &int(33)), (6, "Alpha-blend", &no)],
                ),
                blender(
                    b"MODEL   ",
                    "models\\model",
                    &[(6, "Use alpha-channel", &no), (4, "Alpha ref", &int(32))],
                ),
                blender(
                    b"MODEL   ",
                    "models\\model_aref",
                    &[(6, "Use alpha-channel", &yes), (4, "Alpha ref", &int(128))],
                ),
                blender(b"MODELEbB", "models\\window", &[(6, "Alpha-Blend", &yes)]),
                blender(b"S_SET   ", "effects\\flare", &[]),
            ],
            ..Shaders::default()
        };
        let twin = |name: &str| lib.level_twin(name).map(|b| b.name.to_string());
        assert_eq!(twin("models\\model").as_deref(), Some("default"));
        assert_eq!(twin("MODELS\\MODEL_AREF").as_deref(), Some("def_shaders\\def_aref"));
        assert_eq!(twin("models\\window").as_deref(), Some("def_shaders\\def_trans"));
        // a shader of the level, an effect and a name nobody has stay what they are
        assert_eq!((twin("default"), twin("effects\\flare"), twin("nobody")), (None, None, None));
        let b = lib.blender("default").unwrap();
        assert_eq!((b.draws(), lib.blender("models\\model").unwrap().draws()), (Draws::Level, Draws::Model));
        assert_eq!(
            b.properties(),
            [
                ("Name".to_owned(), BlenderProp::Text("$base0".into())),
                ("Tessellation".to_owned(), BlenderProp::Token(0))
            ]
        );
        // a stream that ends in the middle of a value is what was read before it
        let cut = blender(b"MODEL   ", "cut", &[(6, "Use alpha-channel", &yes), (4, "Alpha ref", &[1, 2])]);
        assert_eq!(cut.properties().len(), 1);
    }

    #[test]
    fn xrlc_shaders_round_trip() {
        let mut name_raw = [0xCDu8; 128];
        name_raw[..8].copy_from_slice(b"default\0");
        let lib = ShadersXrlc {
            shaders: vec![ShaderXrlc {
                name_raw,
                flags: 0x37,
                vert_translucency: 0.5,
                vert_ambient: 0.0,
                lm_density: 1.0,
            }],
        };
        let b = lib.write();
        assert_eq!(b.len(), 144);
        let back = ShadersXrlc::read(&b).unwrap();
        assert_eq!(back, lib);
        assert_eq!(back.shaders[0].name(), "default");
        assert!(ShadersXrlc::read(&b[..100]).is_err());
    }
}
