//! Textures on the window's device for the previews: made once, off the frame, and shown through
//! the window's egui renderer.

/// A side of a block-compressed format is whole 4 x 4 blocks; an uncompressed one is pixels.
fn physical(format: wgpu::TextureFormat, w: u32, h: u32) -> (u32, u32) {
    let (bw, bh) = format.block_dimensions();
    (w.div_ceil(bw) * bw, h.div_ceil(bh) * bh)
}

/// The row bytes and the rows of a level of `w` x `h` texels.
#[must_use]
pub fn level_layout(format: wgpu::TextureFormat, w: u32, h: u32) -> (u32, u32) {
    let (bw, bh) = format.block_dimensions();
    let unit = format.block_copy_size(None).unwrap_or(4);
    (w.div_ceil(bw).max(1) * unit, h.div_ceil(bh).max(1))
}

/// Makes a 2D texture with these levels (largest first) and writes them. Callable from any
/// thread: the device and queue are shared.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    levels: &[Vec<u8>],
) -> Result<wgpu::Texture, String> {
    let max = device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(format!(
            "a texture of {width} x {height} is larger than the graphics device takes ({max} a side)"
        ));
    }
    if levels.is_empty() {
        return Err("the texture holds no picture".to_owned());
    }
    if format.is_compressed() && !device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        return Err("the graphics device does not sample block-compressed textures".to_owned());
    }
    let mips = u32::try_from(levels.len()).unwrap_or(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, data) in levels.iter().enumerate() {
        let mip = u32::try_from(mip).unwrap_or(0);
        let (w, h) = ((width >> mip).max(1), (height >> mip).max(1));
        let (row, rows) = level_layout(format, w, h);
        if data.len() < (row as usize) * (rows as usize) {
            return Err(format!("level {mip} of the texture is cut short"));
        }
        let (pw, ph) = physical(format, w, h);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        );
    }
    Ok(texture)
}

/// One level of a texture as a view.
#[must_use]
pub fn mip_view(texture: &wgpu::Texture, mip: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: mip,
        mip_level_count: Some(1),
        ..Default::default()
    })
}
