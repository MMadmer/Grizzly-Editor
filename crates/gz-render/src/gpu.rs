//! The device-side state every frame shares: layouts, pipelines, samplers.

use std::borrow::Cow;

use crate::material::EditorMaterial;

pub(crate) const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub(crate) const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

const COMMON: &str = include_str!("shaders/common.wgsl");
const MESH: &str = include_str!("shaders/mesh.wgsl");
const MISC: &str = include_str!("shaders/misc.wgsl");

/// How a mesh draw is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum MeshPipe {
    Opaque,
    Opaque2s,
    Alpha,
    Alpha2s,
    Blend,
    Blend2s,
    Add,
    SelfLit,
    SelfLit2s,
    Terrain,
    Water,
    Sky,
    Particle,
    Unlit,
    Unlit2s,
    Wire,
    AlphaAdd,
    Mul,
    Mul2x,
}

impl MeshPipe {
    const ALL: [MeshPipe; 19] = [
        Self::Opaque,
        Self::Opaque2s,
        Self::Alpha,
        Self::Alpha2s,
        Self::Blend,
        Self::Blend2s,
        Self::Add,
        Self::SelfLit,
        Self::SelfLit2s,
        Self::Terrain,
        Self::Water,
        Self::Sky,
        Self::Particle,
        Self::Unlit,
        Self::Unlit2s,
        Self::Wire,
        Self::AlphaAdd,
        Self::Mul,
        Self::Mul2x,
    ];

    /// The pipeline of a material; every surface's edges in the wireframe view.
    pub fn of(kind: EditorMaterial, two_sided: bool, wireframe: bool) -> Self {
        if wireframe && kind != EditorMaterial::Sky {
            return Self::Wire;
        }
        match (kind, two_sided) {
            (EditorMaterial::Opaque, false) => Self::Opaque,
            (EditorMaterial::Opaque, true) => Self::Opaque2s,
            (EditorMaterial::AlphaTest, false) => Self::Alpha,
            (EditorMaterial::AlphaTest | EditorMaterial::Tree, _) => Self::Alpha2s,
            (EditorMaterial::Blended, false) => Self::Blend,
            (EditorMaterial::Blended, true) => Self::Blend2s,
            (EditorMaterial::Additive, _) => Self::Add,
            (EditorMaterial::SelfLit, false) => Self::SelfLit,
            (EditorMaterial::SelfLit, true) => Self::SelfLit2s,
            (EditorMaterial::Terrain, _) => Self::Terrain,
            (EditorMaterial::Water, _) => Self::Water,
            (EditorMaterial::Sky, _) => Self::Sky,
            // a distortion is never drawn: the packet leaves it out
            (EditorMaterial::Particle | EditorMaterial::Distort, _) => Self::Particle,
            (EditorMaterial::AlphaAdd, _) => Self::AlphaAdd,
            (EditorMaterial::Multiply, _) => Self::Mul,
            (EditorMaterial::Multiply2x, _) => Self::Mul2x,
            (EditorMaterial::UnlitColor | EditorMaterial::SelectionOutline, false) => Self::Unlit,
            (EditorMaterial::UnlitColor | EditorMaterial::SelectionOutline, true) => Self::Unlit2s,
            (EditorMaterial::Wireframe, _) => Self::Wire,
        }
    }

    /// How the draw writes the id target; `None`: it does not.
    pub fn id_pipe(self) -> Option<IdPipe> {
        Some(match self {
            Self::Opaque | Self::SelfLit | Self::Terrain | Self::Unlit | Self::Wire => IdPipe::Opaque,
            Self::Opaque2s | Self::SelfLit2s | Self::Unlit2s => IdPipe::Opaque2s,
            Self::Alpha => IdPipe::Alpha,
            Self::Alpha2s => IdPipe::Alpha2s,
            Self::Blend => IdPipe::Blend,
            Self::Blend2s
            | Self::Add
            | Self::Water
            | Self::Particle
            | Self::AlphaAdd
            | Self::Mul
            | Self::Mul2x => IdPipe::Blend2s,
            Self::Sky => return None,
        })
    }

    pub fn is_wire(self) -> bool {
        self == Self::Wire
    }

    fn spec(self) -> (&'static str, &'static str, Option<wgpu::Face>, Blend, bool) {
        let back = Some(wgpu::Face::Back);
        match self {
            Self::Opaque => ("vs_main", "fs_lit", back, Blend::None, true),
            Self::Opaque2s => ("vs_main", "fs_lit", None, Blend::None, true),
            Self::Alpha => ("vs_main", "fs_alpha", back, Blend::None, true),
            Self::Alpha2s => ("vs_main", "fs_alpha", None, Blend::None, true),
            Self::Blend => ("vs_main", "fs_blend", back, Blend::Alpha, false),
            Self::Blend2s => ("vs_main", "fs_blend", None, Blend::Alpha, false),
            Self::Add => ("vs_main", "fs_add", None, Blend::Add, false),
            Self::SelfLit => ("vs_main", "fs_selflit", back, Blend::None, true),
            Self::SelfLit2s => ("vs_main", "fs_selflit", None, Blend::None, true),
            Self::Terrain => ("vs_main", "fs_terrain", back, Blend::None, true),
            Self::Water => ("vs_main", "fs_water", None, Blend::Alpha, false),
            Self::Sky => ("vs_sky", "fs_sky", None, Blend::None, false),
            Self::Particle => ("vs_main", "fs_particle", None, Blend::Alpha, false),
            Self::Unlit => ("vs_main", "fs_unlit", back, Blend::None, true),
            Self::Unlit2s => ("vs_main", "fs_unlit", None, Blend::None, true),
            Self::Wire => ("vs_main", "fs_wire", None, Blend::None, true),
            Self::AlphaAdd => ("vs_main", "fs_particle", None, Blend::AlphaAdd, false),
            Self::Mul => ("vs_main", "fs_mul", None, Blend::Mul, false),
            Self::Mul2x => ("vs_main", "fs_mul", None, Blend::Mul2x, false),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum IdPipe {
    Opaque,
    Opaque2s,
    Alpha,
    Alpha2s,
    /// Something see-through hides nothing in the id target: it is there only when selected.
    Blend,
    Blend2s,
}

impl IdPipe {
    const ALL: [IdPipe; 6] =
        [Self::Opaque, Self::Opaque2s, Self::Alpha, Self::Alpha2s, Self::Blend, Self::Blend2s];
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Blend {
    None,
    Alpha,
    Add,
    AlphaAdd,
    Mul,
    Mul2x,
}

impl Blend {
    fn state(self) -> Option<wgpu::BlendState> {
        // the target's alpha stays 1: a picture is opaque
        let keep_alpha = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let color = |src_factor, dst_factor| {
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor, dst_factor, operation: wgpu::BlendOperation::Add },
                alpha: keep_alpha,
            })
        };
        use wgpu::BlendFactor as F;
        match self {
            Self::None => None,
            Self::Alpha => color(F::SrcAlpha, F::OneMinusSrcAlpha),
            Self::Add => color(F::One, F::One),
            Self::AlphaAdd => color(F::SrcAlpha, F::One),
            Self::Mul => color(F::Dst, F::Zero),
            Self::Mul2x => color(F::Dst, F::Src),
        }
    }
}

const MESH_ATTRS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x2, 4 => Unorm8x4, 5 => Uint16x4, 6 => Unorm8x4
];
const LINE_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
const LABEL_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];

fn mesh_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 56,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &MESH_ATTRS,
    }
}

fn line_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 16,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &LINE_ATTRS,
    }
}

fn label_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 16,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &LABEL_ATTRS,
    }
}

/// Layouts, pipelines and samplers, made once per device.
pub(crate) struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub samples: u32,
    pub bc: bool,
    pub max_side: u32,
    pub frame_layout: wgpu::BindGroupLayout,
    pub material_layout: wgpu::BindGroupLayout,
    pub id_layout: wgpu::BindGroupLayout,
    mesh: [wgpu::RenderPipeline; 19],
    id: [wgpu::RenderPipeline; 6],
    pub line: wgpu::RenderPipeline,
    pub line_overlay: wgpu::RenderPipeline,
    pub tri: wgpu::RenderPipeline,
    pub tri_overlay: wgpu::RenderPipeline,
    pub outline: wgpu::RenderPipeline,
    pub label: wgpu::RenderPipeline,
    pub sampler: wgpu::Sampler,
    pub roof_dummy: wgpu::TextureView,
}

struct PipeDesc<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vs: &'a str,
    fs: &'a str,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    topology: wgpu::PrimitiveTopology,
    cull: Option<wgpu::Face>,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
    format: wgpu::TextureFormat,
    samples: u32,
}

fn pipeline(device: &wgpu::Device, d: &PipeDesc<'_>) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(d.label),
        layout: Some(d.layout),
        vertex: wgpu::VertexState {
            module: d.module,
            entry_point: Some(d.vs),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: d.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: d.topology,
            strip_index_format: None,
            // the engine's front faces are clockwise
            front_face: wgpu::FrontFace::Cw,
            cull_mode: d.cull,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(d.depth_write),
            depth_compare: Some(d.depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: d.samples, mask: !0, alpha_to_coverage_enabled: false },
        fragment: Some(wgpu::FragmentState {
            module: d.module,
            entry_point: Some(d.fs),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: d.format,
                blend: d.blend,
                write_mask: d.write_mask,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

fn storage() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn texture(sample_type: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn cube() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::Cube,
        multisampled: false,
    }
}

impl Gpu {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let features = device.features();
        let bc = features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let msaa = COLOR_FORMAT.guaranteed_format_features(features).flags.sample_count_supported(4)
            && DEPTH_FORMAT.guaranteed_format_features(features).flags.sample_count_supported(4);
        let samples = if msaa { 4 } else { 1 };
        let max_side = device.limits().max_texture_dimension_2d;

        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gz-render frame"),
            entries: &[
                entry(0, uniform()),
                entry(1, storage()),
                entry(2, storage()),
                entry(3, storage()),
                entry(4, texture(wgpu::TextureSampleType::Float { filterable: false })),
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gz-render material"),
            entries: &[
                entry(0, uniform()),
                entry(1, texture(wgpu::TextureSampleType::Float { filterable: true })),
                entry(2, texture(wgpu::TextureSampleType::Float { filterable: true })),
                entry(3, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                // the same textures as cube maps, for a sky (a white cube for a flat one)
                entry(4, cube()),
                entry(5, cube()),
            ],
        });
        let id_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gz-render id target"),
            entries: &[entry(0, texture(wgpu::TextureSampleType::Uint))],
        });

        let mesh_layout_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gz-render mesh"),
            bind_group_layouts: &[Some(&frame_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let line_layout_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gz-render helpers"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let outline_layout_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gz-render outline"),
            bind_group_layouts: &[Some(&frame_layout), Some(&id_layout)],
            immediate_size: 0,
        });

        let module = |label: &str, body: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{COMMON}\n{body}"))),
            })
        };
        let mesh_module = module("gz-render mesh.wgsl", MESH);
        let misc_module = module("gz-render misc.wgsl", MISC);

        let mesh_buffers = [Some(mesh_layout())];
        let mesh = MeshPipe::ALL.map(|p| {
            let (vs, fs, cull, blend, depth_write) = p.spec();
            pipeline(
                &device,
                &PipeDesc {
                    label: fs,
                    layout: &mesh_layout_pl,
                    module: &mesh_module,
                    vs,
                    fs,
                    buffers: &mesh_buffers,
                    topology: if p.is_wire() {
                        wgpu::PrimitiveTopology::LineList
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    cull,
                    blend: blend.state(),
                    write_mask: wgpu::ColorWrites::ALL,
                    depth_write,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    format: COLOR_FORMAT,
                    samples,
                },
            )
        });
        let id = IdPipe::ALL.map(|p| {
            let (fs, cull) = match p {
                IdPipe::Opaque => ("fs_id", Some(wgpu::Face::Back)),
                IdPipe::Opaque2s => ("fs_id", None),
                IdPipe::Alpha => ("fs_id_alpha", Some(wgpu::Face::Back)),
                IdPipe::Alpha2s => ("fs_id_alpha", None),
                IdPipe::Blend => ("fs_id_blend", Some(wgpu::Face::Back)),
                IdPipe::Blend2s => ("fs_id_blend", None),
            };
            pipeline(
                &device,
                &PipeDesc {
                    label: fs,
                    layout: &mesh_layout_pl,
                    module: &mesh_module,
                    vs: "vs_main",
                    fs,
                    buffers: &mesh_buffers,
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                    depth_write: true,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    format: ID_FORMAT,
                    samples: 1,
                },
            )
        });

        let line_buffers = [Some(line_layout())];
        let helper = |label: &str, topology, tested: bool| {
            pipeline(
                &device,
                &PipeDesc {
                    label,
                    layout: &line_layout_pl,
                    module: &misc_module,
                    vs: "vs_line",
                    fs: "fs_line",
                    buffers: &line_buffers,
                    topology,
                    cull: None,
                    blend: Blend::Alpha.state(),
                    write_mask: wgpu::ColorWrites::ALL,
                    depth_write: false,
                    depth_compare: if tested {
                        wgpu::CompareFunction::GreaterEqual
                    } else {
                        wgpu::CompareFunction::Always
                    },
                    format: COLOR_FORMAT,
                    samples,
                },
            )
        };
        let line = helper("gz-render lines", wgpu::PrimitiveTopology::LineList, true);
        let line_overlay = helper("gz-render overlay lines", wgpu::PrimitiveTopology::LineList, false);
        let tri = helper("gz-render helper faces", wgpu::PrimitiveTopology::TriangleList, true);
        let tri_overlay = helper("gz-render overlay faces", wgpu::PrimitiveTopology::TriangleList, false);

        let outline = pipeline(
            &device,
            &PipeDesc {
                label: "gz-render outline",
                layout: &outline_layout_pl,
                module: &misc_module,
                vs: "vs_full",
                fs: "fs_outline",
                buffers: &[],
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull: None,
                blend: Blend::Alpha.state(),
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::Always,
                format: COLOR_FORMAT,
                samples,
            },
        );
        let label_buffers = [Some(label_layout())];
        let label = pipeline(
            &device,
            &PipeDesc {
                label: "gz-render label anchors",
                layout: &line_layout_pl,
                module: &misc_module,
                vs: "vs_label",
                fs: "fs_label",
                buffers: &label_buffers,
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull: None,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
                depth_write: false,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                format: COLOR_FORMAT,
                samples,
            },
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gz-render surfaces"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare: None,
            anisotropy_clamp: 8,
            border_color: None,
        });
        let roof = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gz-render no roof map"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            roof.as_image_copy(),
            &f32::MAX.to_le_bytes(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            roof.size(),
        );
        let roof_dummy = roof.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            device,
            queue,
            samples,
            bc,
            max_side,
            frame_layout,
            material_layout,
            id_layout,
            mesh,
            id,
            line,
            line_overlay,
            tri,
            tri_overlay,
            outline,
            label,
            sampler,
            roof_dummy,
        }
    }

    pub fn mesh(&self, p: MeshPipe) -> &wgpu::RenderPipeline {
        &self.mesh[p as usize]
    }

    pub fn id(&self, p: IdPipe) -> &wgpu::RenderPipeline {
        &self.id[p as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipes_follow_the_material() {
        assert_eq!(MeshPipe::of(EditorMaterial::Opaque, false, false), MeshPipe::Opaque);
        assert_eq!(MeshPipe::of(EditorMaterial::Opaque, true, false), MeshPipe::Opaque2s);
        assert_eq!(MeshPipe::of(EditorMaterial::Tree, false, false), MeshPipe::Alpha2s);
        assert_eq!(MeshPipe::of(EditorMaterial::Blended, false, true), MeshPipe::Wire);
        assert_eq!(MeshPipe::of(EditorMaterial::Sky, false, true), MeshPipe::Sky);
        assert_eq!(MeshPipe::Sky.id_pipe(), None);
        assert_eq!(MeshPipe::Blend.id_pipe(), Some(IdPipe::Blend));
        assert_eq!(MeshPipe::Alpha.id_pipe(), Some(IdPipe::Alpha));
        for (i, p) in MeshPipe::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i);
        }
        for (i, p) in IdPipe::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i);
        }
    }

    #[test]
    fn the_shaders_parse() {
        // naga's front end, as the device would run it: catches a typo without a GPU
        for body in [MESH, MISC] {
            let src = format!("{COMMON}\n{body}");
            let module = naga_parse(&src);
            assert!(module.is_ok(), "{module:?}");
        }
    }

    fn naga_parse(src: &str) -> Result<(), String> {
        // the wgsl feature of wgpu carries naga; reach it through wgpu's re-export
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};
        let module = wgpu::naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }
}
