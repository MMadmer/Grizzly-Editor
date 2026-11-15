//! Buffers, buffer views and accessors.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use base64::Engine;

use super::doc::Doc;
use crate::{Bad, MAX_ELEMENTS, MAX_FILE_BYTES};

const I8: u32 = 5120;
const U8: u32 = 5121;
const I16: u32 = 5122;
const U16: u32 = 5123;
const U32: u32 = 5125;
const F32: u32 = 5126;

/// The bytes of every buffer, checked against the lengths the document gives them.
pub(crate) struct Data<'a> {
    doc: &'a Doc,
    buffers: Vec<Cow<'a, [u8]>>,
    names: Vec<String>,
}

/// A `data:` URI: its media type and its bytes.
pub(crate) fn data_uri(uri: &str) -> Option<Result<(String, Vec<u8>), Bad>> {
    let rest = uri.strip_prefix("data:")?;
    let Some((head, body)) = rest.split_once(',') else {
        return Some(Err("a data URI has no `,` before its data".to_owned()));
    };
    let (mime, base64) = match head.strip_suffix(";base64") {
        Some(m) => (m, true),
        None => (head, false),
    };
    let mime = mime.split(';').next().unwrap_or("").to_owned();
    if !base64 {
        return Some(Ok((mime, percent_decode(body))));
    }
    let body: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    Some(
        base64::engine::general_purpose::STANDARD_PAD_INDIFFERENT
            .decode(body.as_bytes())
            .map(|b| (mime, b))
            .map_err(|_| "a data URI holds base64 that does not decode".to_owned()),
    )
}

/// `%20` and the like as bytes.
pub(crate) fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) =
                (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// A relative URI as a path beside the document; `None` for a URI of another scheme, and for
/// one that names a place of the machine or the network ([`crate::beside`]).
pub(crate) fn file_of(dir: &Path, uri: &str) -> Option<PathBuf> {
    let scheme = uri.split_once(':').map(|(s, _)| s);
    // a drive letter is no scheme
    if scheme
        .is_some_and(|s| s.len() > 1 && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)))
    {
        return None;
    }
    crate::beside(dir, String::from_utf8_lossy(&percent_decode(uri)).as_ref())
}

fn too_short(name: &str, have: usize, need: usize) -> Bad {
    format!(
        "the buffer `{name}` is {} bytes short of what the document reads from it ({need} bytes; it holds {have})",
        need - have
    )
}

impl<'a> Data<'a> {
    /// Loads every buffer: the GLB binary chunk, a data URI or a file beside the document.
    pub fn load(doc: &'a Doc, dir: &Path, bin: Option<&'a [u8]>) -> Result<Self, Bad> {
        let mut buffers = Vec::with_capacity(doc.buffers.len());
        let mut names = Vec::with_capacity(doc.buffers.len());
        for (i, b) in doc.buffers.iter().enumerate() {
            if b.byte_length as u64 > MAX_FILE_BYTES {
                return Err(format!(
                    "buffer {i} says it is {} bytes long; the most this editor reads is {MAX_FILE_BYTES}",
                    b.byte_length
                ));
            }
            let (name, bytes): (String, Cow<'a, [u8]>) = match &b.uri {
                None => match bin {
                    Some(bin) if i == 0 => ("the binary chunk".to_owned(), Cow::Borrowed(bin)),
                    _ => {
                        return Err(format!(
                            "buffer {i} names no file, and the document has no binary chunk for it"
                        ));
                    }
                },
                Some(uri) => match data_uri(uri) {
                    Some(decoded) => (format!("{i} (a data URI)"), Cow::Owned(decoded?.1)),
                    None => {
                        let Some(path) = file_of(dir, uri) else {
                            return Err(format!("the buffer `{uri}` is no file beside the document"));
                        };
                        let bytes = match crate::read_file(&path) {
                            Ok(b) => b,
                            Err(crate::MeshError::Io { reason, .. }) => {
                                return Err(format!("the buffer `{uri}` could not be read: {reason}"));
                            }
                            Err(crate::MeshError::Invalid { message, .. }) => {
                                return Err(format!("the buffer `{uri}`: {message}"));
                            }
                        };
                        (uri.clone(), Cow::Owned(bytes))
                    }
                },
            };
            if bytes.len() < b.byte_length {
                return Err(too_short(&name, bytes.len(), b.byte_length));
            }
            let bytes = match bytes {
                Cow::Borrowed(s) => Cow::Borrowed(&s[..b.byte_length]),
                Cow::Owned(mut v) => {
                    v.truncate(b.byte_length);
                    Cow::Owned(v)
                }
            };
            buffers.push(bytes);
            names.push(name);
        }
        Ok(Self { doc, buffers, names })
    }

    /// The bytes of a buffer view.
    pub fn view(&self, v: usize) -> Result<&[u8], Bad> {
        let view =
            self.doc.buffer_views.get(v).ok_or_else(|| format!("buffer view {v} is not in the document"))?;
        let buf = self.buffers.get(view.buffer).ok_or_else(|| {
            format!("buffer view {v} names buffer {}, which the document lacks", view.buffer)
        })?;
        let end = view
            .byte_offset
            .checked_add(view.byte_length)
            .ok_or_else(|| format!("buffer view {v} has a length no file can hold"))?;
        if end > buf.len() {
            return Err(too_short(&self.names[view.buffer], buf.len(), end));
        }
        Ok(&buf[view.byte_offset..end])
    }

    /// An accessor as floats, `normalized` integers brought to [0, 1] or [-1, 1]: the values and
    /// the components of an element. `kinds` are the element types the caller takes.
    pub fn floats(&self, a: usize, what: &str, kinds: &[&str]) -> Result<(Vec<f32>, usize), Bad> {
        self.decode(a, what, kinds, |v, ct, normalized| {
            if !normalized {
                return Ok(v as f32);
            }
            Ok(match ct {
                I8 => (v / 127.0).max(-1.0),
                U8 => v / 255.0,
                I16 => (v / 32767.0).max(-1.0),
                U16 => v / 65535.0,
                _ => {
                    return Err(format!(
                        "accessor {a} ({what}) is normalized, which its component type cannot be"
                    ));
                }
            } as f32)
        })
    }

    /// An accessor of unsigned integers (indices, joints).
    pub fn uints(&self, a: usize, what: &str, kinds: &[&str]) -> Result<(Vec<u32>, usize), Bad> {
        self.decode(a, what, kinds, |v, ct, _| match ct {
            U8 | U16 | U32 => Ok(v as u32),
            _ => Err(format!("accessor {a} ({what}) should hold unsigned integers")),
        })
    }

    /// The element count of an accessor.
    pub fn count(&self, a: usize) -> usize {
        self.doc.accessors.get(a).map_or(0, |x| x.count)
    }

    fn decode<T: Copy + Default>(
        &self,
        a: usize,
        what: &str,
        kinds: &[&str],
        conv: impl Fn(f64, u32, bool) -> Result<T, Bad>,
    ) -> Result<(Vec<T>, usize), Bad> {
        let acc = self
            .doc
            .accessors
            .get(a)
            .ok_or_else(|| format!("{what} names accessor {a}, which the document lacks"))?;
        if !kinds.contains(&acc.kind.as_str()) {
            return Err(format!(
                "accessor {a} ({what}) is a {}, where a {} belongs",
                acc.kind,
                kinds.join(" or ")
            ));
        }
        let (n, rows) = match acc.kind.as_str() {
            "SCALAR" => (1, 1),
            "VEC2" => (2, 2),
            "VEC3" => (3, 3),
            "VEC4" => (4, 4),
            "MAT2" => (4, 2),
            "MAT3" => (9, 3),
            "MAT4" => (16, 4),
            other => return Err(format!("accessor {a} has the unknown type `{other}`")),
        };
        let csize = comp_size(acc.component_type)
            .ok_or_else(|| format!("accessor {a} has the unknown component type {}", acc.component_type))?;
        if acc.count > MAX_ELEMENTS || acc.count * n > MAX_ELEMENTS * 4 {
            return Err(format!(
                "accessor {a} holds {} elements of {n} components; the most this editor reads is {MAX_ELEMENTS} elements, {} components",
                acc.count,
                MAX_ELEMENTS * 4
            ));
        }
        // a matrix column starts on four bytes
        let col = if n > rows { (rows * csize).div_ceil(4) * 4 } else { rows * csize };
        let elem = col * (n / rows);
        let at = |c: usize| (c / rows) * col + (c % rows) * csize;
        let mut out = vec![T::default(); acc.count * n];

        if let Some(v) = acc.buffer_view {
            let bytes = self.view(v)?;
            let stride = self.doc.buffer_views.get(v).and_then(|x| x.byte_stride).unwrap_or(elem);
            if stride < elem {
                return Err(format!(
                    "buffer view {v} has a stride of {stride} bytes, less than an element of accessor {a} ({elem})"
                ));
            }
            if acc.count > 0 {
                let need = (acc.count - 1)
                    .checked_mul(stride)
                    .and_then(|x| x.checked_add(elem))
                    .and_then(|x| x.checked_add(acc.byte_offset))
                    .unwrap_or(usize::MAX);
                if need > bytes.len() {
                    return Err(format!(
                        "accessor {a} ({what}) reads {need} bytes of buffer view {v}, which holds {}: {} bytes short",
                        bytes.len(),
                        need.saturating_sub(bytes.len())
                    ));
                }
            }
            for i in 0..acc.count {
                let base = acc.byte_offset + i * stride;
                for c in 0..n {
                    let raw = comp(bytes, base + at(c), acc.component_type).unwrap_or(0.0);
                    out[i * n + c] = conv(raw, acc.component_type, acc.normalized)?;
                }
            }
        }

        if let Some(sp) = &acc.sparse {
            if sp.count > acc.count {
                return Err(format!(
                    "accessor {a} replaces {} elements sparsely, more than its {}",
                    sp.count, acc.count
                ));
            }
            let ict = sp.indices.component_type;
            let isize = match ict {
                U8 => 1,
                U16 => 2,
                U32 => 4,
                _ => {
                    return Err(format!(
                        "the sparse indices of accessor {a} have component type {ict}, which no index has"
                    ));
                }
            };
            let ib = self.view(sp.indices.buffer_view)?;
            let vb = self.view(sp.values.buffer_view)?;
            let fits = |bytes: &[u8], offset: usize, size: usize| {
                sp.count
                    .checked_mul(size)
                    .and_then(|x| x.checked_add(offset))
                    .is_some_and(|end| end <= bytes.len())
            };
            if !fits(ib, sp.indices.byte_offset, isize) || !fits(vb, sp.values.byte_offset, elem) {
                return Err(format!("the sparse data of accessor {a} runs past its buffer views"));
            }
            for s in 0..sp.count {
                let idx = comp(ib, sp.indices.byte_offset + s * isize, ict).unwrap_or(0.0) as usize;
                if idx >= acc.count {
                    return Err(format!(
                        "accessor {a} replaces element {idx} sparsely, but it holds {}",
                        acc.count
                    ));
                }
                let base = sp.values.byte_offset + s * elem;
                for c in 0..n {
                    let raw = comp(vb, base + at(c), acc.component_type).unwrap_or(0.0);
                    out[idx * n + c] = conv(raw, acc.component_type, acc.normalized)?;
                }
            }
        }
        Ok((out, n))
    }
}

fn comp_size(ct: u32) -> Option<usize> {
    match ct {
        I8 | U8 => Some(1),
        I16 | U16 => Some(2),
        U32 | F32 => Some(4),
        _ => None,
    }
}

/// One component at a byte offset.
fn comp(b: &[u8], at: usize, ct: u32) -> Option<f64> {
    let get = |n: usize| b.get(at..at.checked_add(n)?);
    Some(match ct {
        I8 => f64::from(i8::from_le_bytes(get(1)?.try_into().ok()?)),
        U8 => f64::from(get(1)?[0]),
        I16 => f64::from(i16::from_le_bytes(get(2)?.try_into().ok()?)),
        U16 => f64::from(u16::from_le_bytes(get(2)?.try_into().ok()?)),
        U32 => f64::from(u32::from_le_bytes(get(4)?.try_into().ok()?)),
        F32 => f64::from(f32::from_le_bytes(get(4)?.try_into().ok()?)),
        _ => return None,
    })
}
