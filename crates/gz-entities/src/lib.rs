//! The game's server entities without the game DLL.
//!
//! The game's own tools get their entity factory from the game DLL: for a spawn section it makes a
//! `CSE_*` object that reads and writes itself as a network packet and lists its editable
//! properties. This crate is that factory as data and code: the class table ([`class`]), the
//! constructors that read a section's defaults, `STATE_*` / `UPDATE_*` of every class in the
//! game's layout, Dead Air's script-class fields, and the property description of each class.
//!
//! ```text
//! Entity::create(section, settings)              a new entity, as the factory makes it
//! Entity::from_spawn_packet(bytes, settings)     a packet of a scene or all.spawn
//! Entity::from_packets(spawn, update, settings)  both packets of an all.spawn vertex
//! Entity::from_scene(values, settings)           the numbered values of an LTX scene section
//! entity.to_spawn_packet()  entity.update_packet()  entity.to_scene()
//! entity.properties(ctx)  entity.set(path, value, ctx)
//! entity.class_name()  section()  name()  position()  visual_name()  shapes()  story_id() ...
//! ```
//!
//! **Nothing is lost.** An entity of a class the table does not know, of a newer version, or
//! whose state does not read back to the very bytes it came from is kept as [`Opaque`]: it
//! writes back byte for byte and shows its header read-only. A decoded entity keeps the bytes
//! it was read from and writes them back unchanged until it is edited.

pub mod class;
mod create;
mod display;
mod entity;
pub mod model;
mod named;
pub mod packet;
mod props;
pub mod script;
pub mod settings;
mod state;
mod update;

use smol_str::SmolStr;

pub use class::{Class, ClassInfo, ScriptKind, class_by_clsid, class_of_section, class_table};
pub use display::{title_case, word_of};
pub use entity::{
    Entity, Header, LevelChangerDestination, M_SPAWN_OBJECT_ASPLAYER, M_SPAWN_OBJECT_LOCAL, M_SPAWN_VERSION,
    Opaque, OpaqueReason, SPAWN_VERSION,
};
pub use model::{Object, Shape};
pub use named::{HEADER, Named, Placement, diff as named_diff, overlay as named_overlay};
pub use packet::{IniSource, IniValue, NetReader, NetWriter, PacketError, TextEncoding};
pub use props::{ChooseKind, CoverDescription, ListKind, Prop, PropContext, PropKind, PropValue, SetEffect};
pub use settings::{MapSettings, Settings};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EntityError {
    #[error("{0}")]
    Packet(#[from] PacketError),
    #[error("the packet is not a spawn packet (type {0})")]
    NotSpawn(u16),
    #[error("section [{0}] does not name a known server class")]
    UnknownClass(SmolStr),
    #[error("section [{section}] has no '{key}', which its class requires")]
    MissingKey { section: SmolStr, key: SmolStr },
    #[error("section [{section}] has '{key} = {value}', which its class cannot use")]
    BadValue { section: SmolStr, key: SmolStr, value: SmolStr },
    #[error("the packet holds {0}, which this version of the game no longer reads")]
    Unsupported(&'static str),
    #[error("the entity has no property '{0}'")]
    NoProperty(String),
    #[error("property '{0}' is read-only")]
    ReadOnly(String),
    #[error("property '{path}' takes {expected}")]
    WrongType { path: String, expected: &'static str },
    #[error("the entity is kept as it was read ({0}) and cannot be changed")]
    Opaque(&'static str),
}

pub type Result<T, E = EntityError> = std::result::Result<T, E>;
