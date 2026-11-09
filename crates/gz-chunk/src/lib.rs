//! The engine's binary streams.
//!
//! Almost every binary file of X-Ray is a tree of chunks: `u32 id`, `u32 size`, `size` bytes,
//! where bit 31 of the id says the body is LZHUF-compressed. [`Reader`] walks a byte slice
//! without copying; [`Writer`] builds one. Reading never panics: a file cut short is an error.

pub mod lzhuf;
pub mod lzo;
mod reader;
mod writer;

pub use reader::{Chunk, ChunkIter, Reader};
pub use writer::Writer;

/// Bit 31 of a chunk id: the body is compressed with [`lzhuf`].
pub const COMPRESSED: u32 = 1 << 31;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChunkError {
    #[error("the data ends {wanted} bytes early at offset {at}")]
    Eof { at: usize, wanted: usize },
    #[error("chunk {0:#x} is missing")]
    Missing(u32),
    #[error("a string has no terminator")]
    Unterminated,
    #[error("compressed data is damaged: {0}")]
    Compression(&'static str),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = ChunkError> = std::result::Result<T, E>;
