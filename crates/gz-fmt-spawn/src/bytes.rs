use std::fmt;
use std::ops::{Deref, Range};
use std::sync::Arc;

/// A range of a shared buffer: a packet or a table of the file it was read from, never copied.
#[derive(Clone)]
pub struct Bytes {
    buf: Arc<[u8]>,
    start: usize,
    end: usize,
}

impl Bytes {
    #[must_use]
    pub fn new(buf: Arc<[u8]>) -> Self {
        let end = buf.len();
        Self { buf, start: 0, end }
    }

    #[must_use]
    pub fn from_vec(v: Vec<u8>) -> Self {
        Self::new(Arc::from(v))
    }

    /// `range` of `buf`; an empty range when it does not fit.
    #[must_use]
    pub fn slice_of(buf: &Arc<[u8]>, range: Range<usize>) -> Self {
        if range.start <= range.end && range.end <= buf.len() {
            Self { buf: buf.clone(), start: range.start, end: range.end }
        } else {
            Self { buf: buf.clone(), start: 0, end: 0 }
        }
    }

    /// The range `part` covers in `buf`, when `part` is a slice of it.
    #[must_use]
    pub(crate) fn of_part(buf: &Arc<[u8]>, part: &[u8]) -> Self {
        let base = buf.as_ptr() as usize;
        let at = part.as_ptr() as usize;
        match at.checked_sub(base) {
            Some(start) if start + part.len() <= buf.len() => Self::slice_of(buf, start..start + part.len()),
            _ => Self::from_vec(part.to_vec()),
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.buf.get(self.start..self.end).unwrap_or(&[])
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Bytes {}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({} bytes)", self.len())
    }
}

impl Default for Bytes {
    fn default() -> Self {
        Self::from_vec(Vec::new())
    }
}
