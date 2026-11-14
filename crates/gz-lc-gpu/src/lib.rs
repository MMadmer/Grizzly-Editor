//! The level compiler's light on a graphics card, and the choice of a build's lighter.
//!
//! ```text
//! lighter(LightChoice) -> Chosen     what `light=auto|gpu|cpu|off` lights a build with
//! GpuLighter                         sample points lit by a compute shader (`gz_lc_light::Lighter`)
//! bvh.rs                             the lighter's own tree: binned SAH, collapsed 4-wide, 8-bit planes
//! pack.rs                            the scene and a job's lights as the kernel reads them
//! light.wgsl                         `light_point` and `RayModel::trace`: a ray a thread, then a
//!                                    sample a thread adds its lights up
//! ```
//!
//! The graphics card's lighter is a plain compute shader over a tree it builds itself: any DX12 or
//! Vulkan adapter, no ray tracing hardware, no experimental feature of the graphics runtime. It
//! has an instance and a device of its own, never the window's. `auto` wraps it in
//! `gz_lc_light::AutoLighter`: whatever the card fails at is lit on the CPU and the build goes on.

mod bvh;
mod gpu;
mod pack;
#[cfg(test)]
mod tests;

pub use gpu::GpuLighter;

use gz_lc_light::{AutoLighter, CpuLighter, LightChoice, Lighter, NullLighter};

/// The lighter of a build, and what the log should say of the choice.
pub struct Chosen {
    pub lighter: Box<dyn Lighter>,
    /// Why `auto` lights on the CPU, when it does.
    pub note: Option<String>,
}

/// The lighter `choice` names. `gpu` without a usable graphics card is refused with the reason;
/// `auto` lights on the CPU then and says why in [`Chosen::note`].
pub fn lighter(choice: LightChoice) -> Result<Chosen, String> {
    choose(choice, GpuLighter::new)
}

fn choose(choice: LightChoice, card: impl FnOnce() -> Result<GpuLighter, String>) -> Result<Chosen, String> {
    Ok(match choice {
        LightChoice::Cpu => Chosen { lighter: Box::new(CpuLighter::default()), note: None },
        LightChoice::Off => Chosen { lighter: Box::new(NullLighter::default()), note: None },
        LightChoice::Gpu => Chosen { lighter: Box::new(card()?), note: None },
        LightChoice::Auto => match card() {
            Ok(gpu) => Chosen { lighter: Box::new(AutoLighter::new(Box::new(gpu))), note: None },
            Err(why) => Chosen {
                lighter: Box::new(CpuLighter::default()),
                note: Some(format!("The light is baked on the CPU: {why}")),
            },
        },
    })
}
