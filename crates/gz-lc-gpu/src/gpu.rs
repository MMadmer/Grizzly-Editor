//! The graphics card's lighter.

use gz_lc_light::{BuildTexture, LcError, Lighter, LitScene, RayModel};

/// Sample points lit by a compute shader on a graphics card.
#[derive(Debug)]
pub struct GpuLighter(());

impl GpuLighter {
    /// The lighter on the best adapter of this machine; the reason when there is none to use.
    pub fn new() -> Result<Self, String> {
        Err("the graphics card's lighter is not part of this build yet.".to_owned())
    }
}

impl Lighter for GpuLighter {
    fn name(&self) -> String {
        "the graphics card".to_owned()
    }

    fn scene<'a>(
        &'a self,
        _: &'a RayModel,
        _: &'a [BuildTexture],
    ) -> gz_lc_light::Result<Box<dyn LitScene + 'a>> {
        Err(LcError::Invalid("the graphics card's lighter is not part of this build yet".to_owned()))
    }
}
