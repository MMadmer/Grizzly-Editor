//! The game's spawn and graph containers.
//!
//! ```text
//! SpawnFile::parse(blob)  -> header, spawn graph (per vertex the spawn and the update packet),
//!                            artefact spawn points, patrol paths, the embedded game graph
//! spawn_file.to_bytes()   -> the same file, byte for byte
//! GameGraph / CrossTable  -> `game.graph` and `level.gct`, standalone or embedded
//! SpawnIndex::build(&spawn_file) -> levels, every entity with its level, every patrol name
//! peek_packet(bytes)      -> the abstract header and the vertices of a spawn packet
//! ```
//!
//! Entity packets stay bytes here ([`Bytes`]: a range of the file, never copied);
//! `gz-entities` decodes them. Hostile input is an error, never a panic: every count is checked
//! against the bytes that are left before anything is reserved for it.

mod bytes;
mod graph;
mod index;
mod patrol;
mod peek;
mod spawn;

pub use bytes::Bytes;
pub use graph::{
    CROSS_TABLE_HEADER_SIZE, CrossTable, GameGraph, GraphEdge, GraphLevel, GraphVertex, Guid, LevelPoint,
};
pub use index::{IndexEntity, IndexLevel, SpawnIndex};
pub use patrol::{PatrolLink, PatrolPath, PatrolPoint};
pub use peek::{PacketHead, peek_packet};
pub use spawn::{SpawnEdge, SpawnFile, SpawnHeader, SpawnVertex, XRAI_VERSION_PRIQUEL};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpawnError {
    #[error("{0}")]
    Chunk(#[from] gz_chunk::ChunkError),
    #[error("the {what} ends {wanted} bytes early")]
    Short { what: &'static str, wanted: usize },
    #[error("chunk {0} is missing")]
    Missing(&'static str),
    #[error("the {what} is damaged: {why}")]
    Damaged { what: &'static str, why: String },
}

pub type Result<T, E = SpawnError> = std::result::Result<T, E>;

pub(crate) fn damaged(what: &'static str, why: impl Into<String>) -> SpawnError {
    SpawnError::Damaged { what, why: why.into() }
}
