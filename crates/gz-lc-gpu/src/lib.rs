//! The level compiler's light on a graphics card, and the choice of a build's lighter.
//!
//! ```text
//! lighter(LightChoice) -> Chosen     what `light=auto|gpu|cpu|off` lights a build with
//! GpuLighter                         sample points lit by a compute shader (`gz_lc_light::Lighter`)
//! ```
//!
//! The graphics card's lighter is a plain compute shader over a tree it builds itself: any DX12 or
//! Vulkan adapter, no ray tracing hardware, no experimental feature of the graphics runtime. It
//! has an instance and a device of its own, never the window's. `auto` wraps it in
//! `gz_lc_light::AutoLighter`: whatever the card fails at is lit on the CPU and the build goes on.

mod gpu;

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
    Ok(match choice {
        LightChoice::Cpu => Chosen { lighter: Box::new(CpuLighter::default()), note: None },
        LightChoice::Off => Chosen { lighter: Box::new(NullLighter::default()), note: None },
        LightChoice::Gpu => Chosen { lighter: Box::new(GpuLighter::new()?), note: None },
        LightChoice::Auto => match GpuLighter::new() {
            Ok(gpu) => Chosen { lighter: Box::new(AutoLighter::new(Box::new(gpu))), note: None },
            Err(why) => Chosen {
                lighter: Box::new(CpuLighter::default()),
                note: Some(format!("The light is baked on the CPU: {why}")),
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_and_no_light_need_no_graphics_card() {
        assert_eq!(lighter(LightChoice::Cpu).map(|c| c.lighter.name()), Ok("the CPU".to_owned()));
        assert_eq!(lighter(LightChoice::Off).map(|c| c.lighter.name()), Ok("no light".to_owned()));
        // auto always lights: on a card, or on the CPU with the reason
        let auto = lighter(LightChoice::Auto);
        assert!(auto.is_ok());
    }
}
