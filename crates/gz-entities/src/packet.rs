//! `NET_Packet`: the byte stream server entities are written to and read from.
//!
//! A packet has two faces. The binary one is what the game reads. The second one exists only
//! in the editor's LTX scenes: every typed write is mirrored into numbered keys of an ini
//! section (`000001 = ...`), and a read takes the next key instead of bytes. [`NetWriter`]
//! produces both at once, [`NetReader`] reads either.

use glam::Vec3;
use smol_str::SmolStr;

/// `M_UPDATE`: the type word of an update packet.
pub const M_UPDATE: u16 = 0;
/// `M_SPAWN`: the type word of a spawn packet.
pub const M_SPAWN: u16 = 1;
/// `NET_PacketSizeLimit`: the engine's packet buffer. A longer packet cannot be loaded by it.
pub const NET_PACKET_SIZE_LIMIT: usize = 16 * 1024;

const PI_MUL_2: f32 = 6.283_185_5;
const EPS_S: f32 = 0.000_000_1;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    #[error("the packet ends {wanted} bytes early at offset {at}")]
    Eof { at: usize, wanted: usize },
    #[error("a string in the packet has no terminator")]
    Unterminated,
    #[error("the scene section has no value {0:06}")]
    IniMissing(u32),
    #[error("raw bytes cannot be read from a scene section")]
    IniRaw,
}

pub type PacketResult<T> = Result<T, PacketError>;

/// How text is put into bytes. The game reads cp1251; the C++ editor writes the bytes its
/// UTF-8 process holds. A packet that was read keeps the encoding it came with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextEncoding {
    #[default]
    Cp1251,
    Utf8,
}

/// What the strings of a packet looked like, gathered while reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    /// A string with characters above ASCII was valid UTF-8.
    pub utf8: bool,
    /// A string was not valid UTF-8 and was taken as cp1251.
    pub cp1251: bool,
}

impl TextStats {
    /// The encoding that writes these strings back to the bytes they came from.
    #[must_use]
    pub fn encoding(self) -> TextEncoding {
        if self.utf8 && !self.cp1251 { TextEncoding::Utf8 } else { TextEncoding::Cp1251 }
    }
}

/// The numbered values of a scene section: `value(1)` is the key `000001`.
pub trait IniSource {
    fn value(&self, index: u32) -> Option<&str>;
}

impl<S: AsRef<str>> IniSource for &[S] {
    fn value(&self, index: u32) -> Option<&str> {
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.get(i).map(AsRef::as_ref)
    }
}

impl<S: AsRef<str>> IniSource for Vec<S> {
    fn value(&self, index: u32) -> Option<&str> {
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.get(i).map(AsRef::as_ref)
    }
}

/// One mirrored write of the ini face, typed as the engine's `IIniFileStream` types it.
#[derive(Clone, Debug, PartialEq)]
pub enum IniValue {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    S8(i8),
    S16(i16),
    S32(i32),
    S64(i64),
    Float(f32),
    Vec3(Vec3),
    Vec4([f32; 4]),
    Str(String),
    /// A value kept as the scene section had it.
    Raw(String),
}

impl IniValue {
    /// The key of the `index`-th value (from 1), as `SIniFileStream::gen_name` makes it.
    #[must_use]
    pub fn key(index: u32) -> String {
        format!("{index:06}")
    }

    /// The text `CInifile::w_*` writes for this value.
    #[must_use]
    pub fn to_ltx(&self) -> String {
        match self {
            Self::U8(v) => v.to_string(),
            Self::U16(v) => v.to_string(),
            // oracle: `w_u32` prints with "%d", so the upper half comes out negative.
            Self::U32(v) => (*v as i32).to_string(),
            Self::U64(v) => v.to_string(),
            Self::S8(v) => v.to_string(),
            Self::S16(v) => v.to_string(),
            Self::S32(v) => v.to_string(),
            Self::S64(v) => v.to_string(),
            Self::Float(v) => fmt_f(*v),
            Self::Vec3(v) => format!("{},{},{}", fmt_f(v.x), fmt_f(v.y), fmt_f(v.z)),
            Self::Vec4(v) => format!("{},{},{},{}", fmt_f(v[0]), fmt_f(v[1]), fmt_f(v[2]), fmt_f(v[3])),
            Self::Str(s) => format!("\"{s}\""),
            Self::Raw(s) => s.clone(),
        }
    }
}

/// C's `%f` for a float.
fn fmt_f(v: f32) -> String {
    if v.is_nan() {
        // the CRT prints the sign of a NaN
        return if v.is_sign_negative() { "-nan(ind)".to_owned() } else { "nan".to_owned() };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf".to_owned() } else { "inf".to_owned() };
    }
    format!("{:.6}", f64::from(v))
}

/// C's `atoi`: leading blanks, a sign, digits; the result saturates as the CRT's does.
#[must_use]
pub fn atoi(text: &str) -> i32 {
    let s = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut acc: i64 = 0;
    for b in digits.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        acc = acc * 10 + i64::from(b - b'0');
        if acc > i64::from(u32::MAX) {
            break;
        }
    }
    let v = if neg { -acc } else { acc };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn atoi64(text: &str) -> i64 {
    let s = text.trim_start();
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+'))))
        .map_or(s.len(), |(i, _)| i);
    s[..end].parse::<i128>().map_or(0, |v| v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

fn atou64(text: &str) -> u64 {
    let s = text.trim_start();
    let s = s.strip_prefix('+').unwrap_or(s);
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse::<u128>().map_or(0, |v| v.min(u128::from(u64::MAX)) as u64)
}

/// C's `atof`: the longest prefix that is a number; 0 when there is none.
#[must_use]
pub fn atof(text: &str) -> f64 {
    let s = text.trim_start();
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let digits_from = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if i == digits_from || (i == digits_from + 1 && b.get(digits_from) == Some(&b'.')) {
        return 0.0;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'-' | b'+')) {
            j += 1;
        }
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            i = j;
        }
    }
    s[..i].parse::<f64>().unwrap_or(0.0)
}

/// `sscanf("%f,%f,...")`: the numbers that parse, in order; the rest stay 0.
fn scan_floats<const N: usize>(text: &str) -> [f32; N] {
    let mut out = [0.0f32; N];
    let mut parts = text.split(',');
    for slot in &mut out {
        let Some(part) = parts.next() else { break };
        let t = part.trim_start();
        let numeric = t.as_bytes().first().is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'));
        if !numeric {
            break;
        }
        *slot = atof(t) as f32;
    }
    out
}

/// `CInifile::r_string_wb`: the value without the quotes around it.
fn unquote(text: &str) -> &str {
    let t = text.strip_suffix('"').unwrap_or(text);
    t.strip_prefix('"').unwrap_or(t)
}

/// Bytes of a packet string as text: UTF-8 when valid, cp1251 otherwise.
fn decode(bytes: &[u8], stats: &mut TextStats) -> SmolStr {
    match std::str::from_utf8(bytes) {
        Ok(s) => {
            if !s.is_ascii() {
                stats.utf8 = true;
            }
            SmolStr::new(s)
        }
        Err(_) => {
            stats.cp1251 = true;
            SmolStr::new(gz_base::text::cp1251_to_utf8(bytes))
        }
    }
}

enum Src<'a> {
    Bin { data: &'a [u8], pos: usize },
    Ini { values: &'a dyn IniSource, counter: u32 },
}

/// Reads a packet: bytes, or the numbered values of a scene section.
pub struct NetReader<'a> {
    src: Src<'a>,
    text: TextStats,
}

macro_rules! read_num {
    ($($name:ident -> $t:ty, $ini:expr;)*) => {$(
        pub fn $name(&mut self) -> PacketResult<$t> {
            if self.is_ini() {
                let parse: fn(&str) -> $t = $ini;
                return Ok(parse(self.next_ini()?));
            }
            const N: usize = std::mem::size_of::<$t>();
            let bytes = self.take(N)?;
            let mut raw = [0u8; N];
            raw.copy_from_slice(bytes);
            Ok(<$t>::from_le_bytes(raw))
        }
    )*};
}

impl<'a> NetReader<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { src: Src::Bin { data, pos: 0 }, text: TextStats::default() }
    }

    /// A reader over the numbered values of a scene section.
    #[must_use]
    pub fn ini(values: &'a dyn IniSource) -> Self {
        Self { src: Src::Ini { values, counter: 0 }, text: TextStats::default() }
    }

    /// `inistream != NULL`.
    #[must_use]
    pub fn is_ini(&self) -> bool {
        matches!(self.src, Src::Ini { .. })
    }

    #[must_use]
    pub fn text_stats(&self) -> TextStats {
        self.text
    }

    /// The read position: a byte offset, or the number of values taken.
    #[must_use]
    pub fn r_tell(&self) -> usize {
        match &self.src {
            Src::Bin { pos, .. } => *pos,
            Src::Ini { counter, .. } => *counter as usize,
        }
    }

    /// Bytes left. For a scene section: 1 while a next value exists, 0 after.
    #[must_use]
    pub fn r_elapsed(&self) -> usize {
        match &self.src {
            Src::Bin { data, pos } => data.len().saturating_sub(*pos),
            Src::Ini { values, counter } => usize::from(values.value(counter + 1).is_some()),
        }
    }

    #[must_use]
    pub fn r_eof(&self) -> bool {
        self.r_elapsed() == 0
    }

    pub fn r_seek(&mut self, to: usize) -> PacketResult<()> {
        match &mut self.src {
            Src::Bin { data, pos } => {
                if to > data.len() {
                    return Err(PacketError::Eof { at: data.len(), wanted: to - data.len() });
                }
                *pos = to;
                Ok(())
            }
            Src::Ini { counter, .. } => {
                *counter = u32::try_from(to).unwrap_or(u32::MAX);
                Ok(())
            }
        }
    }

    pub fn r_advance(&mut self, n: usize) -> PacketResult<()> {
        self.take(n).map(|_| ())
    }

    /// The bytes that are left, without moving. Empty for a scene section.
    #[must_use]
    pub fn rest(&self) -> &'a [u8] {
        match &self.src {
            Src::Bin { data, pos } => data.get(*pos..).unwrap_or(&[]),
            Src::Ini { .. } => &[],
        }
    }

    /// `r(p, count)`: raw bytes. An error for a scene section, where the engine asserts.
    pub fn take(&mut self, n: usize) -> PacketResult<&'a [u8]> {
        match &mut self.src {
            Src::Bin { data, pos } => {
                let end = pos.checked_add(n).filter(|&e| e <= data.len());
                match end {
                    Some(end) => {
                        let out = data.get(*pos..end).unwrap_or(&[]);
                        *pos = end;
                        Ok(out)
                    }
                    None => Err(PacketError::Eof { at: *pos, wanted: n - data.len().saturating_sub(*pos).min(n) }),
                }
            }
            Src::Ini { .. } => Err(PacketError::IniRaw),
        }
    }

    fn next_ini(&mut self) -> PacketResult<&'a str> {
        match &mut self.src {
            Src::Ini { values, counter } => {
                let index = counter.saturating_add(1);
                let text = values.value(index).ok_or(PacketError::IniMissing(index))?;
                *counter = index;
                Ok(text)
            }
            Src::Bin { .. } => Err(PacketError::IniRaw),
        }
    }

    read_num! {
        r_u8 -> u8, |t| atoi(t) as u8;
        r_u16 -> u16, |t| atoi(t) as u16;
        r_u32 -> u32, |t| atoi(t) as u32;
        r_u64 -> u64, atou64;
        r_s8 -> i8, |t| atoi(t) as i8;
        r_s16 -> i16, |t| atoi(t) as i16;
        r_s32 -> i32, atoi;
        r_s64 -> i64, atoi64;
        r_float -> f32, |t| atof(t) as f32;
    }

    /// `r_begin`: the type word a packet starts with.
    pub fn r_begin(&mut self) -> PacketResult<u16> {
        self.r_u16()
    }

    pub fn r_vec3(&mut self) -> PacketResult<Vec3> {
        if self.is_ini() {
            let v: [f32; 3] = scan_floats(self.next_ini()?);
            return Ok(Vec3::from_array(v));
        }
        Ok(Vec3::new(self.r_float()?, self.r_float()?, self.r_float()?))
    }

    pub fn r_vec4(&mut self) -> PacketResult<[f32; 4]> {
        if self.is_ini() {
            return Ok(scan_floats(self.next_ini()?));
        }
        Ok([self.r_float()?, self.r_float()?, self.r_float()?, self.r_float()?])
    }

    /// The bytes of a zero-terminated string. Binary packets only.
    pub fn r_stringz_raw(&mut self) -> PacketResult<&'a [u8]> {
        match &mut self.src {
            Src::Bin { data, pos } => {
                let rest = data.get(*pos..).unwrap_or(&[]);
                match rest.iter().position(|&b| b == 0) {
                    Some(n) => {
                        *pos += n + 1;
                        Ok(rest.get(..n).unwrap_or(&[]))
                    }
                    None => Err(PacketError::Unterminated),
                }
            }
            Src::Ini { .. } => Err(PacketError::IniRaw),
        }
    }

    pub fn r_stringz(&mut self) -> PacketResult<SmolStr> {
        if self.is_ini() {
            return Ok(SmolStr::new(unquote(self.next_ini()?)));
        }
        let raw = self.r_stringz_raw()?;
        Ok(decode(raw, &mut self.text))
    }

    pub fn skip_stringz(&mut self) -> PacketResult<()> {
        if self.is_ini() {
            return self.next_ini().map(|_| ());
        }
        self.r_stringz_raw().map(|_| ())
    }

    /// A float stored as `u16` over `[min, max]`.
    pub fn r_float_q16(&mut self, min: f32, max: f32) -> PacketResult<f32> {
        let v = self.r_u16()?;
        Ok((f32::from(v) * (max - min)) / 65535.0 + min)
    }

    /// A float stored as `u8` over `[min, max]`.
    pub fn r_float_q8(&mut self, min: f32, max: f32) -> PacketResult<f32> {
        let v = self.r_u8()?;
        Ok(dequantize_q8(v, min, max))
    }

    pub fn r_angle16(&mut self) -> PacketResult<f32> {
        self.r_float_q16(0.0, PI_MUL_2)
    }

    pub fn r_angle8(&mut self) -> PacketResult<f32> {
        self.r_float_q8(0.0, PI_MUL_2)
    }

    /// A unit vector packed into 16 bits.
    pub fn r_dir(&mut self) -> PacketResult<Vec3> {
        Ok(pv_decompress(self.r_u16()?))
    }

    /// A packed direction and its length.
    pub fn r_sdir(&mut self) -> PacketResult<Vec3> {
        let t = self.r_u16()?;
        let s = self.r_float()?;
        Ok(pv_decompress(t) * s)
    }

    /// The four rows `i`, `j`, `k`, `c` of an `Fmatrix`, three floats each.
    pub fn r_matrix(&mut self) -> PacketResult<[Vec3; 4]> {
        Ok([self.r_vec3()?, self.r_vec3()?, self.r_vec3()?, self.r_vec3()?])
    }

    pub fn r_client_id(&mut self) -> PacketResult<u32> {
        self.r_u32()
    }

    /// A count that is followed by `count` items of at least `item_size` bytes each: an error
    /// when the packet cannot hold them, so nothing is reserved for a count that lies.
    pub(crate) fn checked_count(&self, count: usize, item_size: usize) -> PacketResult<usize> {
        if let Src::Bin { data, pos } = &self.src {
            let left = data.len().saturating_sub(*pos);
            let need = count.saturating_mul(item_size);
            if need > left {
                return Err(PacketError::Eof { at: *pos, wanted: need - left });
            }
        }
        Ok(count)
    }
}

/// `r_float_q8`'s arithmetic.
#[must_use]
pub fn dequantize_q8(v: u8, min: f32, max: f32) -> f32 {
    (f32::from(v) / 255.0001) * (max - min) + min
}

/// `w_float_q8`'s arithmetic.
#[must_use]
pub fn quantize_q8(a: f32, min: f32, max: f32) -> u8 {
    let q = (a - min) / (max - min);
    (q * 255.0 + 0.5).floor() as i32 as u8
}

/// `w_float_q16`'s arithmetic.
#[must_use]
pub fn quantize_q16(a: f32, min: f32, max: f32) -> u16 {
    let q = (a - min) / (max - min);
    (q * 65535.0 + 0.5).floor() as i32 as u16
}

/// `angle_normalize`: into `[0, 2π]`.
#[must_use]
pub fn angle_normalize(a: f32) -> f32 {
    if (0.0..=PI_MUL_2).contains(&a) {
        return a;
    }
    let div = a / PI_MUL_2;
    let rnd = if div > 0.0 { div.floor() } else { div.ceil() } as i32;
    let mut frac = div - rnd as f32;
    if frac < 0.0 {
        frac += 1.0;
    }
    frac * PI_MUL_2
}

const PV_XSIGN: u16 = 0x8000;
const PV_YSIGN: u16 = 0x4000;
const PV_ZSIGN: u16 = 0x2000;
const PV_TOP: u16 = 0x1f80;
const PV_BOTTOM: u16 = 0x007f;

/// `pvCompress`: a direction (any length) into 16 bits.
#[must_use]
pub fn pv_compress(vec: Vec3) -> u16 {
    // the game's guard; the oracle divides by zero here
    if !vec.is_finite() || (vec.x.abs() < EPS_S && vec.y.abs() < EPS_S && vec.z.abs() < EPS_S) {
        return 0;
    }
    let mut out = 0u16;
    if vec.x.is_sign_negative() {
        out |= PV_XSIGN;
    }
    if vec.y.is_sign_negative() {
        out |= PV_YSIGN;
    }
    if vec.z.is_sign_negative() {
        out |= PV_ZSIGN;
    }
    let (x, y, z) = (vec.x.abs(), vec.y.abs(), vec.z.abs());
    let w = 126.0 / (x + y + z);
    let mut xbits = (x * w).floor() as i32;
    let mut ybits = (y * w).floor() as i32;
    if xbits >= 64 {
        xbits = 127 - xbits;
        ybits = 127 - ybits;
    }
    out |= (xbits << 7) as u16;
    out |= ybits as u16;
    out
}

/// `pvDecompress`: the unit vector 16 bits stand for.
#[must_use]
pub fn pv_decompress(packed: u16) -> Vec3 {
    let mut xbits = i32::from((packed & PV_TOP) >> 7);
    let mut ybits = i32::from(packed & PV_BOTTOM);
    if xbits + ybits >= 127 {
        xbits = 127 - xbits;
        ybits = 127 - ybits;
    }
    // `pvUVAdjustment`, computed in place of the engine's table
    let (x, y, z) = (xbits as f32, ybits as f32, (126 - xbits - ybits) as f32);
    let adj = 1.0 / (y * y + z * z + x * x).sqrt();
    let mut v = Vec3::new(adj * x, adj * y, adj * z);
    if packed & PV_XSIGN != 0 {
        v.x = -v.x.abs();
    }
    if packed & PV_YSIGN != 0 {
        v.y = -v.y.abs();
    }
    if packed & PV_ZSIGN != 0 {
        v.z = -v.z.abs();
    }
    v
}

/// Builds a packet. Writing cannot fail; what the ini face cannot hold (raw bytes) is
/// remembered and reported by [`NetWriter::ini_values`].
#[derive(Clone, Debug, Default)]
pub struct NetWriter {
    buf: Vec<u8>,
    ini: Option<Vec<IniValue>>,
    ini_raw: bool,
    encoding: TextEncoding,
}

macro_rules! write_num {
    ($($name:ident($t:ty) => $variant:ident;)*) => {$(
        pub fn $name(&mut self, v: $t) {
            self.buf.extend_from_slice(&v.to_le_bytes());
            if let Some(ini) = &mut self.ini {
                ini.push(IniValue::$variant(v));
            }
        }
    )*};
}

impl NetWriter {
    #[must_use]
    pub fn new(encoding: TextEncoding) -> Self {
        Self { buf: Vec::with_capacity(256), ini: None, ini_raw: false, encoding }
    }

    /// A writer that also mirrors every typed write for a scene section.
    #[must_use]
    pub fn with_ini(encoding: TextEncoding) -> Self {
        Self { buf: Vec::with_capacity(256), ini: Some(Vec::with_capacity(64)), ini_raw: false, encoding }
    }

    #[must_use]
    pub fn is_ini(&self) -> bool {
        self.ini.is_some()
    }

    #[must_use]
    pub fn w_tell(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// The mirrored values, or `None` when the writer had no ini face or raw bytes were
    /// written, which a scene section cannot hold.
    #[must_use]
    pub fn ini_values(self) -> Option<Vec<IniValue>> {
        if self.ini_raw { None } else { self.ini }
    }

    /// `w_begin`: starts the packet over with its type word.
    pub fn w_begin(&mut self, kind: u16) {
        self.buf.clear();
        self.w_u16(kind);
    }

    write_num! {
        w_u8(u8) => U8;
        w_u16(u16) => U16;
        w_u32(u32) => U32;
        w_u64(u64) => U64;
        w_s8(i8) => S8;
        w_s16(i16) => S16;
        w_s32(i32) => S32;
        w_s64(i64) => S64;
    }

    pub fn w_float(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
        if let Some(ini) = &mut self.ini {
            ini.push(IniValue::Float(v));
        }
    }

    /// `w(p, count)`: raw bytes, which have no ini form.
    pub fn w(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.buf.extend_from_slice(bytes);
        if self.ini.is_some() {
            self.ini_raw = true;
        }
    }

    /// `w_seek`: overwrites bytes that are already there. Not mirrored, as in the engine.
    pub fn w_seek(&mut self, at: usize, bytes: &[u8]) {
        if let Some(slot) = at.checked_add(bytes.len()).and_then(|end| self.buf.get_mut(at..end)) {
            slot.copy_from_slice(bytes);
        }
    }

    pub fn w_vec3(&mut self, v: Vec3) {
        for c in v.to_array() {
            self.buf.extend_from_slice(&c.to_le_bytes());
        }
        if let Some(ini) = &mut self.ini {
            ini.push(IniValue::Vec3(v));
        }
    }

    pub fn w_vec4(&mut self, v: [f32; 4]) {
        for c in v {
            self.buf.extend_from_slice(&c.to_le_bytes());
        }
        if let Some(ini) = &mut self.ini {
            ini.push(IniValue::Vec4(v));
        }
    }

    pub fn w_stringz(&mut self, text: &str) {
        match self.encoding {
            TextEncoding::Utf8 => self.buf.extend_from_slice(text.as_bytes()),
            TextEncoding::Cp1251 => self.buf.extend_from_slice(&gz_base::text::utf8_to_cp1251(text).0),
        }
        self.buf.push(0);
        if let Some(ini) = &mut self.ini {
            ini.push(IniValue::Str(text.to_owned()));
        }
    }

    /// A float stored as `u16` over `[min, max]`.
    pub fn w_float_q16(&mut self, a: f32, min: f32, max: f32) {
        self.w_u16(quantize_q16(a, min, max));
    }

    /// A float stored as `u8` over `[min, max]`.
    pub fn w_float_q8(&mut self, a: f32, min: f32, max: f32) {
        self.w_u8(quantize_q8(a, min, max));
    }

    pub fn w_angle16(&mut self, a: f32) {
        self.w_float_q16(angle_normalize(a), 0.0, PI_MUL_2);
    }

    pub fn w_angle8(&mut self, a: f32) {
        self.w_float_q8(angle_normalize(a), 0.0, PI_MUL_2);
    }

    pub fn w_dir(&mut self, d: Vec3) {
        self.w_u16(pv_compress(d));
    }

    /// A packed direction and its length.
    pub fn w_sdir(&mut self, d: Vec3) {
        let mag = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        if mag > EPS_S {
            self.w_dir(Vec3::new(d.x / mag, d.y / mag, d.z / mag));
            self.w_float(mag);
        } else {
            self.w_dir(Vec3::new(0.0, 0.0, 1.0));
            self.w_float(0.0);
        }
    }

    pub fn w_matrix(&mut self, rows: &[Vec3; 4]) {
        for row in rows {
            self.w_vec3(*row);
        }
    }

    pub fn w_client_id(&mut self, id: u32) {
        self.w_u32(id);
    }

    /// `w_chunk_open8`: a block whose length byte is filled in by [`Self::w_chunk_close8`].
    pub fn w_chunk_open8(&mut self) -> usize {
        let at = self.w_tell();
        self.w_u8(0);
        if self.ini.is_some() {
            self.ini_raw = true;
        }
        at
    }

    pub fn w_chunk_close8(&mut self, at: usize) {
        let size = self.w_tell().saturating_sub(at).saturating_sub(1);
        self.w_seek(at, &[size as u8]);
    }

    /// `w_chunk_open16`: a block whose length word is filled in by [`Self::w_chunk_close16`].
    pub fn w_chunk_open16(&mut self) -> usize {
        let at = self.w_tell();
        self.w_u16(0);
        if self.ini.is_some() {
            self.ini_raw = true;
        }
        at
    }

    pub fn w_chunk_close16(&mut self, at: usize) {
        let size = self.w_tell().saturating_sub(at).saturating_sub(2);
        self.w_seek(at, &(size as u16).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q8_follows_the_engine_formulas() {
        // w: u8(floor(q * 255 + 0.5)) with q = (a - min) / (max - min)
        assert_eq!(quantize_q8(1.0, 0.0, 1.0), 255);
        assert_eq!(quantize_q8(0.0, 0.0, 1.0), 0);
        assert_eq!(quantize_q8(0.5, 0.0, 1.0), 128); // 127.5 + 0.5
        assert_eq!(quantize_q8(0.25, -1.0, 1.0), 159); // 0.625 * 255 = 159.375, + 0.5 -> 159
        // r: v / 255.0001 * (max - min) + min
        let one = dequantize_q8(255, 0.0, 1.0);
        assert!(one < 1.0 && one > 0.999_999);
        assert_eq!(dequantize_q8(0, -1.0, 1.0), -1.0);
        // what was read writes back to the byte it was
        for b in 0..=255u8 {
            assert_eq!(quantize_q8(dequantize_q8(b, 0.0, 1.0), 0.0, 1.0), b);
        }
    }

    #[test]
    fn q16_follows_the_engine_formulas() {
        assert_eq!(quantize_q16(1.0, 0.0, 1.0), 65535);
        assert_eq!(quantize_q16(0.5, 0.0, 1.0), 32768); // 32767.5 + 0.5
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w.w_float_q16(0.25, 0.0, 1.0); // 16383.75 + 0.5 -> 16384
        assert_eq!(w.bytes(), &16384u16.to_le_bytes());
        let mut r = NetReader::new(w.bytes());
        let back = r.r_float_q16(0.0, 1.0).unwrap();
        assert!((back - 16384.0 / 65535.0).abs() < 1e-7);
    }

    #[test]
    fn angles_are_normalised_before_they_are_packed() {
        assert_eq!(angle_normalize(1.0), 1.0);
        assert!((angle_normalize(-std::f32::consts::FRAC_PI_2) - 3.0 * std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((angle_normalize(7.0) - (7.0 - PI_MUL_2)).abs() < 1e-5);
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w.w_angle8(std::f32::consts::PI); // q = 0.5 -> 128
        w.w_angle16(std::f32::consts::PI);
        assert_eq!(w.bytes()[0], 128);
        assert_eq!(u16::from_le_bytes([w.bytes()[1], w.bytes()[2]]), 32768);
    }

    #[test]
    fn directions_pack_as_the_engine_packs_them() {
        // +Z: xbits = 0, ybits = 0, no sign bits
        assert_eq!(pv_compress(Vec3::Z), 0);
        // +X: w = 126, xbits = 126 >= 64 -> 1, ybits = 127
        assert_eq!(pv_compress(Vec3::X), (1 << 7) | 127);
        // +Y: xbits = 0, ybits = 126
        assert_eq!(pv_compress(Vec3::Y), 126);
        // -Y sets the y sign bit
        assert_eq!(pv_compress(Vec3::NEG_Y), 0x4000 | 126);
        for v in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::NEG_Y, Vec3::new(1.0, 1.0, 1.0).normalize()] {
            let back = pv_decompress(pv_compress(v));
            assert!((back - v).length() < 0.02, "{v:?} -> {back:?}");
        }
        // a zero vector is the game's 0, not a division by zero
        assert_eq!(pv_compress(Vec3::ZERO), 0);
    }

    #[test]
    fn sdir_keeps_the_length_apart() {
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w.w_sdir(Vec3::new(0.0, 0.0, 3.0));
        w.w_sdir(Vec3::ZERO);
        let mut r = NetReader::new(w.bytes());
        assert_eq!(r.r_sdir().unwrap(), Vec3::new(0.0, 0.0, 3.0));
        assert_eq!(r.r_sdir().unwrap(), Vec3::ZERO);
        assert!(r.r_eof());
    }

    #[test]
    fn a_short_packet_is_an_error() {
        let mut r = NetReader::new(&[1, 2, 3]);
        assert!(matches!(r.r_u32(), Err(PacketError::Eof { .. })));
        assert_eq!(r.r_tell(), 0);
        assert!(matches!(NetReader::new(b"abc").r_stringz(), Err(PacketError::Unterminated)));
        assert!(NetReader::new(&[0; 4]).checked_count(3, 2).is_err());
        assert!(NetReader::new(&[0; 6]).checked_count(3, 2).is_ok());
    }

    #[test]
    fn chunks_fill_in_their_sizes() {
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        let a = w.w_chunk_open8();
        w.w_u32(7);
        w.w_chunk_close8(a);
        let b = w.w_chunk_open16();
        w.w_stringz("ab");
        w.w_chunk_close16(b);
        assert_eq!(w.bytes(), &[4, 7, 0, 0, 0, 3, 0, b'a', b'b', 0]);
    }

    #[test]
    fn text_keeps_the_encoding_it_came_with() {
        let cyr = "Зона";
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w.w_stringz(cyr);
        assert_eq!(w.bytes(), &[0xC7, 0xEE, 0xED, 0xE0, 0]);
        let mut r = NetReader::new(w.bytes());
        assert_eq!(r.r_stringz().unwrap(), cyr);
        assert_eq!(r.text_stats().encoding(), TextEncoding::Cp1251);

        let mut w = NetWriter::new(TextEncoding::Utf8);
        w.w_stringz(cyr);
        let mut r = NetReader::new(w.bytes());
        assert_eq!(r.r_stringz().unwrap(), cyr);
        assert_eq!(r.text_stats().encoding(), TextEncoding::Utf8);
    }

    #[test]
    fn the_ini_face_mirrors_typed_writes() {
        let mut w = NetWriter::with_ini(TextEncoding::Cp1251);
        w.w_begin(M_SPAWN);
        w.w_stringz("stalker");
        w.w_u32(u32::MAX);
        w.w_float(1.5);
        w.w_vec3(Vec3::new(1.0, 2.0, 3.5));
        w.w_u8(200);
        let values = w.ini_values().unwrap();
        let text: Vec<String> = values.iter().map(IniValue::to_ltx).collect();
        assert_eq!(text, ["1", "\"stalker\"", "-1", "1.500000", "1.000000,2.000000,3.500000", "200"]);
        assert_eq!(IniValue::key(3), "000003");

        let mut r = NetReader::ini(&text);
        assert_eq!(r.r_begin().unwrap(), M_SPAWN);
        assert_eq!(r.r_stringz().unwrap(), "stalker");
        assert_eq!(r.r_u32().unwrap(), u32::MAX);
        assert_eq!(r.r_float().unwrap(), 1.5);
        assert_eq!(r.r_vec3().unwrap(), Vec3::new(1.0, 2.0, 3.5));
        assert_eq!(r.r_u8().unwrap(), 200);
        assert!(r.r_eof());
        assert!(matches!(r.r_u8(), Err(PacketError::IniMissing(7))));
    }

    #[test]
    fn raw_bytes_have_no_ini_form() {
        let mut w = NetWriter::with_ini(TextEncoding::Cp1251);
        w.w_u8(1);
        w.w(&[1, 2]);
        assert!(w.ini_values().is_none());
        let values = vec!["1"];
        let mut r = NetReader::ini(&values);
        assert!(matches!(r.take(1), Err(PacketError::IniRaw)));
    }

    #[test]
    fn c_number_parsing() {
        assert_eq!(atoi("  42abc"), 42);
        assert_eq!(atoi("-7"), -7);
        assert_eq!(atoi("x"), 0);
        assert_eq!(atoi("99999999999"), i32::MAX);
        assert_eq!(atof("1.5e2x"), 150.0);
        assert_eq!(atof(".5"), 0.5);
        assert_eq!(atof("-"), 0.0);
        assert_eq!(atof("abc"), 0.0);
    }
}
