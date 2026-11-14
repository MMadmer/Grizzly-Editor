//! How a preview picture reaches the window: a small WGSL pass inside egui's own render pass.
//!
//! egui blends a texture as premultiplied, so a texture's white with alpha 0 would paint white;
//! a channel view would need a copy per channel. This pass samples the texture as the file
//! stores it - BC blocks included - and shows it blended by its straight alpha, one channel
//! over black, the alpha as grey, or opaque (a rendered picture), with a linear or a nearest
//! sampler. Nothing is computed per frame but a few uniforms.
//!
//! The pipeline is made on a thread of its own the first time a preview asks for it (a pipeline
//! compile is tens of milliseconds): a preview tab asks as it opens, so it is there before its
//! picture is, and no frame waits for it.

use std::sync::Arc;

use egui_wgpu::RenderState;

/// How a picture is drawn into the window's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMode {
    /// Its colours over what is under it, blended by its alpha as stored.
    Blend = 0,
    /// One channel, opaque: red, green or blue over black.
    Red = 1,
    Green = 2,
    Blue = 3,
    /// The alpha as grey, opaque.
    Alpha = 4,
    /// The colours, opaque: a rendered picture whose alpha is no coverage.
    Opaque = 5,
}

impl ImageMode {
    /// The texture preview's channel views: RGBA, R, G, B, A.
    #[must_use]
    pub fn of_channel(channel: usize) -> Self {
        match channel {
            1 => ImageMode::Red,
            2 => ImageMode::Green,
            3 => ImageMode::Blue,
            4 => ImageMode::Alpha,
            _ => ImageMode::Blend,
        }
    }
}

const SHADER: &str = "
struct U { uv0: vec2<f32>, uv1: vec2<f32>, mode: u32, srgb: u32, pad0: u32, pad1: u32 };
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: U;
struct V { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let t = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    var o: V;
    o.pos = vec4<f32>(t.x * 2.0 - 1.0, 1.0 - t.y * 2.0, 0.0, 1.0);
    o.uv = mix(u.uv0, u.uv1, t);
    return o;
}
fn to_target(c: vec3<f32>) -> vec3<f32> {
    if (u.srgb == 0u) { return c; }
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c < vec3<f32>(0.04045));
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, v.uv);
    var o = c;
    switch u.mode {
        case 1u: { o = vec4<f32>(c.r, 0.0, 0.0, 1.0); }
        case 2u: { o = vec4<f32>(0.0, c.g, 0.0, 1.0); }
        case 3u: { o = vec4<f32>(0.0, 0.0, c.b, 1.0); }
        case 4u: { o = vec4<f32>(c.a, c.a, c.a, 1.0); }
        case 5u: { o = vec4<f32>(c.rgb, 1.0); }
        default: {}
    }
    return vec4<f32>(to_target(o.rgb), o.a);
}
";

/// The pass: one pipeline on the window's target, a linear and a nearest sampler. Kept in
/// egui's callback resources, made once per window.
pub struct ImagePass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// The window's target decodes sRGB: what egui writes there is linear.
    srgb: bool,
}

impl ImagePass {
    fn new(gpu: &RenderState) -> Self {
        // made on a thread of its own, with the device the window's frame draws with
        let _drawing = gz_win::drawing_priority();
        let device = &gpu.device;
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gz-ui-previews image"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(
                    2,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gz-ui-previews image"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gz-ui-previews image"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER)),
        });
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gz-ui-previews image"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.target_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |mag: wgpu::FilterMode, label: &str| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: mag,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        };
        Self {
            pipeline,
            layout,
            linear: sampler(wgpu::FilterMode::Linear, "gz-ui-previews linear"),
            nearest: sampler(wgpu::FilterMode::Nearest, "gz-ui-previews nearest"),
            srgb: gpu.target_format.is_srgb(),
        }
    }

    /// The pass of this window once it is made. The first call starts making it on a thread of
    /// its own and answers `None` until it is there; `wake` asks for the frame that draws with it.
    #[must_use]
    pub fn get(gpu: &RenderState, exec: &gz_task::Executors) -> Option<Arc<ImagePass>> {
        if let Some(p) = gpu.renderer.read().callback_resources.get::<Arc<ImagePass>>() {
            return Some(p.clone());
        }
        {
            let mut r = gpu.renderer.write();
            if r.callback_resources.get::<Preparing>().is_some() {
                return None;
            }
            r.callback_resources.insert(Preparing);
        }
        let (state, wake) = (gpu.clone(), exec.wake().clone());
        let _ = exec.thread("gz-ui-previews image pass", move || {
            let pass = Arc::new(ImagePass::new(&state));
            state.renderer.write().callback_resources.insert(pass);
            wake.ping();
        });
        None
    }
}

/// The pass is being made.
struct Preparing;

/// A picture ready to draw: its view bound with a sampler and the draw's own uniforms. Made when
/// the view changes, never per frame; one draw a frame each.
pub struct ImageDraw {
    bind: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
}

impl ImageDraw {
    /// `None` while the pass is being made (see [`ImagePass::get`]).
    #[must_use]
    pub fn new(
        gpu: &RenderState,
        view: &wgpu::TextureView,
        nearest: bool,
        exec: &gz_task::Executors,
    ) -> Option<Arc<Self>> {
        let pass = ImagePass::get(gpu, exec)?;
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gz-ui-previews image"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gz-ui-previews image"),
            layout: &pass.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if nearest {
                        &pass.nearest
                    } else {
                        &pass.linear
                    }),
                },
                wgpu::BindGroupEntry { binding: 2, resource: uniforms.as_entire_binding() },
            ],
        });
        Some(Arc::new(Self { bind, uniforms }))
    }
}

/// The uniforms' bytes: the texture coordinates, the mode, whether the target decodes sRGB.
#[must_use]
pub fn uniform_bytes(uv: [f32; 4], mode: ImageMode, srgb: bool) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (chunk, v) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(uv) {
        chunk.copy_from_slice(&v.to_le_bytes());
    }
    let tail = [mode as u32, u32::from(srgb)];
    for (chunk, v) in bytes.as_chunks_mut::<4>().0.iter_mut().skip(4).zip(tail) {
        chunk.copy_from_slice(&v.to_le_bytes());
    }
    bytes
}

struct ImageCallback {
    draw: Arc<ImageDraw>,
    uv: [f32; 4],
    mode: ImageMode,
}

impl egui_wgpu::CallbackTrait for ImageCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let srgb = res.get::<Arc<ImagePass>>().is_some_and(|p| p.srgb);
        queue.write_buffer(&self.draw.uniforms, 0, &uniform_bytes(self.uv, self.mode, srgb));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        res: &egui_wgpu::CallbackResources,
    ) {
        let Some(p) = res.get::<Arc<ImagePass>>() else { return };
        pass.set_pipeline(&p.pipeline);
        pass.set_bind_group(0, &self.draw.bind, &[]);
        pass.draw(0..4, 0..1);
    }
}

/// Draws a picture whose whole lies at `image` (points), the part inside `clip` only: the
/// texture coordinates of that part are worked out here.
pub fn paint_image(
    painter: &egui::Painter,
    image: egui::Rect,
    clip: egui::Rect,
    uv: [f32; 4],
    mode: ImageMode,
    draw: &Arc<ImageDraw>,
) {
    let shown = image.intersect(clip);
    if shown.width() <= 0.0 || shown.height() <= 0.0 || image.width() <= 0.0 || image.height() <= 0.0 {
        return;
    }
    let fx = |x: f32| uv[0] + (uv[2] - uv[0]) * (x - image.min.x) / image.width();
    let fy = |y: f32| uv[1] + (uv[3] - uv[1]) * (y - image.min.y) / image.height();
    let part = [fx(shown.min.x), fy(shown.min.y), fx(shown.max.x), fy(shown.max.y)];
    painter.add(egui_wgpu::Callback::new_paint_callback(
        shown,
        ImageCallback { draw: draw.clone(), uv: part, mode },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_are_laid_out_as_the_shader_reads_them() {
        let b = uniform_bytes([0.0, 0.25, 0.5, 1.0], ImageMode::Alpha, true);
        assert_eq!(&b[4..8], &0.25f32.to_le_bytes());
        assert_eq!(&b[16..20], &4u32.to_le_bytes());
        assert_eq!(&b[20..24], &1u32.to_le_bytes());
        assert_eq!(ImageMode::of_channel(2), ImageMode::Green);
    }
}
