//! Particle systems as the editor plays them: the engine's particle library
//! (the domains, every action, the particle store, the noise) and the
//! playback of the render side (effects, their animation and collision,
//! groups with their child effects and timing, and the library's lookups).
//!
//! Pure: no window, no GPU, no thread and no global state. A [`Library`] is made once from a
//! `particles.xr` ([`gz_fmt_xr::Particles`]); [`Library::create`] makes a [`System`] by name
//! (an effect first, then a group). A system is driven by the caller:
//!
//! ```text
//! system.update_parent(transform, velocity, false)   where it is
//! system.play() / system.stop(deferred)              the object's play / stop
//! system.on_frame(dt_ms, collider)                   the engine's 33 ms steps, up to three
//! system.sprites(&mut out)                           what to draw, one batch per effect
//! ```
//!
//! - **Deterministic.** The engine draws from one process-wide random generator and the C runtime's `rand()`;
//!   a system owns both ([`Rng`]), seeded by [`System::with_seed`], and every effect of a group
//!   draws from the group's. The draws happen in the engine's order, so a test can pin a step.
//! - **Output** is plain data: [`Particle`]s for whoever wants them and [`SpriteBatch`]es - the
//!   sprite's centre, half sizes, angle, colour, atlas frame and alignment - that the renderer
//!   turns into quads facing its camera. The engine draws a particle effect as
//!   sprites only (the sprite flag; the mesh form of other engines does not exist in this one).
//! - **Collision** asks the caller's [`Collider`] for the nearest surface along a
//!   particle's last move, the way the editor's ray pick does.

#![forbid(unsafe_code)]

mod actions;
mod domain;
mod effect;
mod group;
mod noise;
#[cfg(test)]
mod pinned;
mod rand;
mod system;

pub use domain::Domain;
pub use effect::{Align, Cull, Particle, Sprite, SpriteBatch};
pub use noise::Noise;
pub use rand::Rng;
pub use system::{Collider, EffectDef, Frame, GroupDef, GroupEffect, Library, System, flags, group_flags};

/// The engine's step: milliseconds an update covers.
pub const STEP_MS: u32 = 33;
/// The step in seconds.
pub const STEP: f32 = STEP_MS as f32 / 1000.0;
