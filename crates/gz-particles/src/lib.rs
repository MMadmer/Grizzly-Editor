//! Particle systems as the editor plays them: a port of the engine's particle library
//! (`Source/XrParticles`: the domains, every action, the particle store, the noise) and of the
//! playback of `Source/XrRender/Private` (`CParticleEffect`, `CPEDef`'s animation and collision,
//! `CParticleGroup` with its child effects and timing, `CPSLibrary`'s lookups).
//!
//! Pure: no window, no GPU, no thread and no global state. A [`Library`] is made once from a
//! `particles.xr` ([`gz_fmt_xr::Particles`]); [`Library::create`] makes a [`System`] by name
//! (an effect first, then a group - `model_CreateParticles`). A system is driven by the caller:
//!
//! ```text
//! system.update_parent(transform, velocity, false)   where it is (UpdateParent)
//! system.play() / system.stop(deferred)              the object's Play / Stop
//! system.on_frame(dt_ms, collider)                   the engine's 33 ms steps, up to three
//! system.sprites(&mut out)                           what to draw, one batch per effect
//! ```
//!
//! - **Deterministic.** The engine draws from one process-wide `CRandom` and the CRT's `rand()`;
//!   a system owns both ([`Rng`]), seeded by [`System::with_seed`], and every effect of a group
//!   draws from the group's. The draws happen in the C++ order, so a test can pin a step.
//! - **Output** is plain data: [`Particle`]s for whoever wants them and [`SpriteBatch`]es - the
//!   sprite's centre, half sizes, angle, colour, atlas frame and alignment - that the renderer
//!   turns into quads facing its camera (`FillSprite`). The engine draws a particle effect as
//!   sprites only (`dfSprite`; the mesh form of other engines does not exist in this one).
//! - **Collision** (`dfCollision`) asks the caller's [`Collider`] for the nearest surface along a
//!   particle's last move, the way the editor's `ExecuteCollision` asks `Tools->RayPick`.

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

/// The engine's step (`PS::uDT_STEP`): milliseconds an update covers.
pub const STEP_MS: u32 = 33;
/// The step in seconds (`PS::fDT_STEP`).
pub const STEP: f32 = STEP_MS as f32 / 1000.0;
