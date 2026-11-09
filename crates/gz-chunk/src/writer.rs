use glam::{Quat, Vec2, Vec3, Vec4};

use crate::{COMPRESSED, lzhuf};

/// Builds a byte stream with the engine's `IWriter` vocabulary. Infallible: it writes to memory,
/// and the caller puts the bytes on disk in one go.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
    /// Offsets of the size fields of the chunks that are open.
    open: Vec<usize>,
}

macro_rules! write_le {
    ($($name:ident($t:ty)),* $(,)?) => {$(
        #[inline]
        pub fn $name(&mut self, v: $t) {
            self.buf.extend_from_slice(&v.to_le_bytes());
        }
    )*};
}

impl Writer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        Self { buf: Vec::with_capacity(n), open: Vec::new() }
    }

    #[must_use]
    pub fn tell(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// The stream. Chunks still open are closed first.
    #[must_use]
    pub fn into_bytes(mut self) -> Vec<u8> {
        while !self.open.is_empty() {
            self.close_chunk();
        }
        self.buf
    }

    write_le! {
        w_u8(u8), w_u16(u16), w_u32(u32), w_u64(u64),
        w_i8(i8), w_i16(i16), w_i32(i32), w_i64(i64),
        w_f32(f32), w_f64(f64),
    }

    pub fn w_bool(&mut self, v: bool) {
        self.w_u8(u8::from(v));
    }

    pub fn w(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn w_vec2(&mut self, v: Vec2) {
        self.w_f32(v.x);
        self.w_f32(v.y);
    }

    pub fn w_vec3(&mut self, v: Vec3) {
        self.w_f32(v.x);
        self.w_f32(v.y);
        self.w_f32(v.z);
    }

    pub fn w_vec4(&mut self, v: Vec4) {
        self.w_f32(v.x);
        self.w_f32(v.y);
        self.w_f32(v.z);
        self.w_f32(v.w);
    }

    pub fn w_quat(&mut self, q: Quat) {
        self.w_f32(q.x);
        self.w_f32(q.y);
        self.w_f32(q.z);
        self.w_f32(q.w);
    }

    pub fn w_matrix(&mut self, m: &[[f32; 4]; 4]) {
        for row in m {
            for v in row {
                self.w_f32(*v);
            }
        }
    }

    /// A float stored as `u16` over `[min, max]`, clamped as the engine clamps it.
    pub fn w_float_q16(&mut self, v: f32, min: f32, max: f32) {
        let q = ((v - min) * 65535.0 / (max - min)).clamp(0.0, 65535.0);
        self.w_u16(q as u16);
    }

    pub fn w_float_q8(&mut self, v: f32, min: f32, max: f32) {
        let q = ((v - min) / (max - min) * 255.0).clamp(0.0, 255.0);
        self.w_u8(q as u8);
    }

    /// A zero-terminated string, bytes as given.
    pub fn w_stringz_raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.buf.push(0);
    }

    /// A zero-terminated UTF-8 string: the editor's own files.
    pub fn w_stringz(&mut self, text: &str) {
        self.w_stringz_raw(text.as_bytes());
    }

    /// A zero-terminated cp1251 string: files the game reads.
    pub fn w_stringz_cp1251(&mut self, text: &str) {
        let (bytes, _) = gz_base::text::utf8_to_cp1251(text);
        self.w_stringz_raw(&bytes);
    }

    /// A text line with the engine's `\r\n`.
    pub fn w_line(&mut self, text: &str) {
        self.buf.extend_from_slice(text.as_bytes());
        self.buf.extend_from_slice(b"\r\n");
    }

    /// Starts a chunk. Chunks nest.
    pub fn open_chunk(&mut self, id: u32) {
        self.w_u32(id);
        self.open.push(self.buf.len());
        self.w_u32(0);
    }

    /// Ends the innermost open chunk and fills in its size.
    pub fn close_chunk(&mut self) {
        if let Some(at) = self.open.pop() {
            let size = (self.buf.len() - at - 4) as u32;
            self.buf[at..at + 4].copy_from_slice(&size.to_le_bytes());
        }
    }

    /// A whole chunk from bytes.
    pub fn w_chunk(&mut self, id: u32, body: &[u8]) {
        self.w_u32(id);
        self.w_u32(body.len() as u32);
        self.w(body);
    }

    /// A whole chunk built by `fill`.
    pub fn chunk(&mut self, id: u32, fill: impl FnOnce(&mut Writer)) {
        self.open_chunk(id);
        fill(self);
        self.close_chunk();
    }

    /// A whole chunk, LZHUF-compressed, with the compressed flag on its id.
    pub fn w_chunk_compressed(&mut self, id: u32, body: &[u8]) {
        let packed = lzhuf::compress(body);
        self.w_chunk(id | COMPRESSED, &packed);
    }
}
