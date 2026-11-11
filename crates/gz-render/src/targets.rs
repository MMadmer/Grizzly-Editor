//! The textures a frame is drawn into. They grow in 64 px buckets and the view is their
//! top-left corner, so dragging a splitter does not make them again.

use crate::gpu::{COLOR_FORMAT, DEPTH_FORMAT, Gpu, ID_FORMAT};

/// The step the targets grow by.
pub const BUCKET: u32 = 64;

/// A side rounded up to whole buckets.
#[must_use]
pub fn bucket(side: u32) -> u32 {
    side.max(1).div_ceil(BUCKET) * BUCKET
}

/// The size the targets should have for a view of `wanted` pixels: unchanged while the view
/// fits and has not shrunk two buckets below them; otherwise every side that has to change does.
#[must_use]
pub fn target_extent(current: Option<[u32; 2]>, wanted: [u32; 2]) -> [u32; 2] {
    let want = wanted.map(bucket);
    let Some(cur) = current else { return want };
    let off = |w: u32, c: u32| w > c || w + 2 * BUCKET <= c;
    if off(want[0], cur[0]) || off(want[1], cur[1]) {
        [
            if off(want[0], cur[0]) { want[0] } else { cur[0] },
            if off(want[1], cur[1]) { want[1] } else { cur[1] },
        ]
    } else {
        cur
    }
}

pub(crate) struct Targets {
    pub extent: [u32; 2],
    /// Changes whenever the targets are made again.
    pub serial: u64,
    /// The finished picture: what the UI shows and a read-back copies.
    pub color: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    /// The multisampled picture, resolved into `color` by the last pass.
    pub msaa_view: Option<wgpu::TextureView>,
    pub depth_view: wgpu::TextureView,
    pub id_view: wgpu::TextureView,
    pub id_depth_view: wgpu::TextureView,
    pub id_bind: wgpu::BindGroup,
}

fn make(
    gpu: &Gpu,
    label: &str,
    extent: [u32; 2],
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: extent[0], height: extent[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

impl Targets {
    pub fn new(gpu: &Gpu, extent: [u32; 2], serial: u64) -> Self {
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let color = make(
            gpu,
            "gz-render picture",
            extent,
            COLOR_FORMAT,
            1,
            attach | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let color_view = view(&color);
        let msaa_view = (gpu.samples > 1).then(|| {
            view(&make(gpu, "gz-render picture (multisampled)", extent, COLOR_FORMAT, gpu.samples, attach))
        });
        let depth_view = view(&make(gpu, "gz-render depth", extent, DEPTH_FORMAT, gpu.samples, attach));
        let id_view = view(&make(
            gpu,
            "gz-render ids",
            extent,
            ID_FORMAT,
            1,
            attach | wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        let id_depth_view = view(&make(gpu, "gz-render id depth", extent, DEPTH_FORMAT, 1, attach));
        let id_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gz-render ids"),
            layout: &gpu.id_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&id_view),
            }],
        });
        Self { extent, serial, color, color_view, msaa_view, depth_view, id_view, id_depth_view, id_bind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(bucket(0), 64);
        assert_eq!(bucket(1), 64);
        assert_eq!(bucket(64), 64);
        assert_eq!(bucket(65), 128);
        assert_eq!(bucket(1919), 1920);
    }

    #[test]
    fn targets_grow_at_once_and_shrink_late() {
        assert_eq!(target_extent(None, [800, 600]), [832, 640]);
        // inside the bucket: kept
        assert_eq!(target_extent(Some([832, 640]), [810, 590]), [832, 640]);
        // one bucket smaller: kept
        assert_eq!(target_extent(Some([832, 640]), [760, 570]), [832, 640]);
        // outgrown on one side: that side grows
        assert_eq!(target_extent(Some([832, 640]), [900, 600]), [960, 640]);
        // two buckets below: that side shrinks
        assert_eq!(target_extent(Some([832, 640]), [700, 600]), [704, 640]);
        assert_eq!(target_extent(Some([832, 640]), [700, 500]), [704, 512]);
    }
}
