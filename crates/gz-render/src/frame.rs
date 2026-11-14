//! Recording a packet: the uploads into buffers that only grow, then the passes - in parallel
//! chunks on the CPU pool when there are many draws - as command buffers in submission order.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use rayon::prelude::*;

use crate::assets::{Assets, GpuMesh};
use crate::gpu::{Gpu, IdPipe, MeshPipe};
use crate::packet::{Draw, FramePacket, MAX_LABEL_QUERIES};
use crate::targets::Targets;
use crate::{RenderError, Result};

/// Draws a pass records before the next chunk gets an encoder of its own.
const CHUNK: usize = 1024;
/// How long the recorder waits for the GPU before it gives up on a frame.
pub(crate) const GPU_WAIT: Duration = Duration::from_secs(10);

/// A buffer that is made again only when it is too small.
struct GrowBuffer {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl GrowBuffer {
    fn new(gpu: &Gpu, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let capacity = 4096;
        Self { label, usage, buffer: create(gpu, label, usage, capacity), capacity }
    }

    /// Puts `data` at the start; `true` when the buffer was made again (bind groups go stale).
    fn write(&mut self, gpu: &Gpu, data: &[u8]) -> bool {
        let need = (data.len() as u64).max(256).next_multiple_of(4);
        let grown = need > self.capacity;
        if grown {
            self.capacity = need.next_power_of_two();
            self.buffer = create(gpu, self.label, self.usage, self.capacity);
        }
        if !data.is_empty() {
            gpu.queue.write_buffer(&self.buffer, 0, data);
        }
        grown
    }
}

fn create(gpu: &Gpu, label: &str, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

/// Where a read-back buffer of the label queries is: free, written by a submitted frame and being
/// mapped, mapped and ready to read, or failed to map.
const READ_IDLE: u8 = 0;
const READ_MAPPING: u8 = 1;
const READ_READY: u8 = 2;
const READ_FAILED: u8 = 3;

/// One of the two buffers the label queries are copied into. A frame writes the free one and
/// maps it once submitted; the render thread reads it when the map has come back, a frame or so
/// later - it never waits for the GPU to get there.
struct Readback {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// How many queries the frame that wrote it ran.
    count: u32,
}

struct Queries {
    set: wgpu::QuerySet,
    capacity: u32,
    resolve: wgpu::Buffer,
    readback: [Readback; 2],
}

impl Queries {
    fn free_slot(&self) -> Option<usize> {
        self.readback.iter().position(|r| r.state.load(Ordering::Acquire) == READ_IDLE)
    }
}

/// What one recorder keeps between frames: the render thread has one, an offscreen picture
/// another.
pub(crate) struct FrameResources {
    frame_ubo: wgpu::Buffer,
    instances: GrowBuffer,
    bones: GrowBuffer,
    lights: GrowBuffer,
    helpers: GrowBuffer,
    labels: GrowBuffer,
    dynamic: GrowBuffer,
    queries: Option<Queries>,
    roof: Option<(u64, wgpu::TextureView)>,
    bind: Option<wgpu::BindGroup>,
    meshes: Vec<Option<Arc<GpuMesh>>>,
    materials: Vec<Arc<wgpu::BindGroup>>,
}

/// A recorded frame.
pub(crate) struct Recorded {
    pub buffers: Vec<wgpu::CommandBuffer>,
    /// A texture it drew was still loading.
    pub pending: bool,
    /// Label occlusion queries the frame copies into a read-back buffer, and which one.
    pub queries: u32,
    pub label_slot: Option<usize>,
}

impl FrameResources {
    pub fn new(gpu: &Gpu) -> Self {
        let storage = wgpu::BufferUsages::STORAGE;
        Self {
            frame_ubo: create(
                gpu,
                "gz-render frame",
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                size_of::<crate::packet::FrameUniform>() as u64,
            ),
            instances: GrowBuffer::new(gpu, "gz-render instances", storage),
            bones: GrowBuffer::new(gpu, "gz-render poses", storage),
            lights: GrowBuffer::new(gpu, "gz-render lights", storage),
            helpers: GrowBuffer::new(gpu, "gz-render helpers", wgpu::BufferUsages::VERTEX),
            labels: GrowBuffer::new(gpu, "gz-render label anchors", wgpu::BufferUsages::VERTEX),
            dynamic: GrowBuffer::new(gpu, "gz-render sprites", wgpu::BufferUsages::VERTEX),
            queries: None,
            roof: None,
            bind: None,
            meshes: Vec::new(),
            materials: Vec::new(),
        }
    }

    /// Uploads `packet` and records its passes into `targets`, the view `size` in their corner.
    pub fn record(
        &mut self,
        gpu: &Gpu,
        assets: &Assets,
        packet: &FramePacket,
        targets: &Targets,
        size: [u32; 2],
        pool: &rayon::ThreadPool,
    ) -> Recorded {
        let mut frame = packet.frame;
        frame.viewport =
            [size[0] as f32, size[1] as f32, 1.0 / size[0].max(1) as f32, 1.0 / size[1].max(1) as f32];
        gpu.queue.write_buffer(&self.frame_ubo, 0, bytemuck::bytes_of(&frame));
        let mut stale = self.bind.is_none();
        stale |= self.instances.write(gpu, bytemuck::cast_slice(&packet.instances));
        stale |= self.bones.write(gpu, bytemuck::cast_slice(&packet.bones));
        stale |= self.lights.write(gpu, bytemuck::cast_slice(&packet.lights));
        stale |= self.upload_roof(gpu, packet);

        // helper vertices back to back: lines, overlay lines, faces, overlay faces
        let h = &packet.helpers;
        let counts =
            [h.lines.len(), h.lines_overlay.len(), h.tris.len(), h.tris_overlay.len()].map(|c| c as u32);
        let mut bytes: Vec<u8> = Vec::with_capacity(
            (counts.iter().sum::<u32>() as usize) * size_of::<crate::helpers::LineVertex>(),
        );
        for part in [&h.lines, &h.lines_overlay, &h.tris, &h.tris_overlay] {
            bytes.extend_from_slice(bytemuck::cast_slice(part));
        }
        self.helpers.write(gpu, &bytes);
        self.dynamic.write(gpu, bytemuck::cast_slice(&packet.dynamic));
        let mut helper_at = [0u32; 4];
        for i in 1..4 {
            helper_at[i] = helper_at[i - 1] + counts[i - 1];
        }
        let mut queries = packet.label_points.len().min(MAX_LABEL_QUERIES) as u32;
        self.labels
            .write(gpu, bytemuck::cast_slice(packet.label_points.get(..queries as usize).unwrap_or(&[])));
        if queries > 0 {
            self.ensure_queries(gpu, queries);
        }
        // both read-backs still on their way: this frame asks nothing, its labels keep what the
        // last answer said
        let label_slot = self.queries.as_ref().filter(|_| queries > 0).and_then(Queries::free_slot);
        if label_slot.is_none() {
            queries = 0;
        }

        if stale {
            let roof = self.roof.as_ref().map_or(&gpu.roof_dummy, |(_, v)| v);
            self.bind = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gz-render frame"),
                layout: &gpu.frame_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.frame_ubo.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.instances.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.bones.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: self.lights.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(roof) },
                ],
            }));
        }

        assets.gpu_meshes(&packet.meshes, &mut self.meshes);
        let pending = assets.bind_groups(&packet.materials, &mut self.materials);
        let placeholder_handle = assets.placeholder();
        let placeholder = packet
            .meshes
            .iter()
            .position(|m| *m == placeholder_handle)
            .and_then(|i| self.meshes.get(i).cloned().flatten());
        let fallback_bind = assets.fallback_bind_group();

        let (Some(bind), Some(placeholder), Some(fallback_bind)) =
            (self.bind.as_ref(), placeholder, fallback_bind)
        else {
            // nothing to draw with: the frame is the background alone
            let mut enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gz-render clear") });
            drop(begin(&mut enc, targets, size, Clear::Both(packet.clear), true, None));
            return Recorded { buffers: vec![enc.finish()], pending, queries: 0, label_slot: None };
        };
        let readback = match (self.queries.as_ref(), label_slot) {
            (Some(q), Some(i)) => q.readback.get(i).map(|r| &r.buffer),
            _ => None,
        };
        let ctx = Ctx {
            gpu,
            targets,
            size,
            bind,
            meshes: &self.meshes,
            materials: &self.materials,
            placeholder: &placeholder,
            fallback_bind: &fallback_bind,
            helpers: &self.helpers.buffer,
            dynamic: &self.dynamic.buffer,
            helper_at,
            helper_counts: counts,
            labels: &self.labels.buffer,
            queries: self.queries.as_ref().filter(|_| queries > 0),
            readback,
            query_count: queries,
            clear: packet.clear,
            outline: packet.id_pass,
        };

        // the passes, in submission order
        let mut passes: Vec<Job<'_>> = Vec::new();
        if packet.id_pass {
            let ids = packet.draws.get(..packet.blended.end).unwrap_or(&[]);
            passes.push(Job::Ids(ids));
        }
        // opaque and alpha-tested lie next to each other
        let solid = packet.draws.get(packet.opaque.start..packet.alpha.end).unwrap_or(&[]);
        if solid.is_empty() {
            passes.push(Job::Meshes { draws: &[], first: true });
        }
        for (i, chunk) in solid.chunks(CHUNK).enumerate() {
            passes.push(Job::Meshes { draws: chunk, first: i == 0 });
        }
        passes.push(Job::Middle(packet.draws.get(packet.sky.clone()).unwrap_or(&[])));
        for chunk in packet.draws.get(packet.blended.clone()).unwrap_or(&[]).chunks(CHUNK) {
            passes.push(Job::Meshes { draws: chunk, first: false });
        }
        passes.push(Job::Last);

        let many = packet.draws.len() > CHUNK;
        let buffers: Vec<wgpu::CommandBuffer> = if many {
            pool.install(|| passes.par_iter().map(|j| record_job(&ctx, j)).collect())
        } else {
            let mut enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gz-render frame") });
            for j in &passes {
                encode_job(&ctx, j, &mut enc);
            }
            vec![enc.finish()]
        };
        let queries = ctx.query_count * u32::from(ctx.queries.is_some());
        Recorded { buffers, pending, queries, label_slot: label_slot.filter(|_| queries > 0) }
    }

    fn upload_roof(&mut self, gpu: &Gpu, packet: &FramePacket) -> bool {
        let Some(r) = &packet.roof else {
            return self.roof.take().is_some();
        };
        if self.roof.as_ref().is_some_and(|(rev, _)| *rev == r.revision) {
            return false;
        }
        let cells = r.cells.min(gpu.max_side);
        let Some(heights) = r.heights.get(..(cells as usize * cells as usize)) else { return false };
        let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gz-render roof map"),
            size: wgpu::Extent3d { width: cells, height: cells, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            tex.as_image_copy(),
            bytemuck::cast_slice(heights),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(cells * 4),
                rows_per_image: Some(cells),
            },
            tex.size(),
        );
        self.roof = Some((r.revision, tex.create_view(&wgpu::TextureViewDescriptor::default())));
        true
    }

    fn ensure_queries(&mut self, gpu: &Gpu, count: u32) {
        if self.queries.as_ref().is_some_and(|q| q.capacity >= count) {
            return;
        }
        let capacity = count.next_power_of_two().max(64);
        let bytes = u64::from(capacity) * 8;
        let readback = || Readback {
            buffer: create(
                gpu,
                "gz-render label read-back",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                bytes,
            ),
            state: Arc::new(AtomicU8::new(READ_IDLE)),
            count: 0,
        };
        self.queries = Some(Queries {
            set: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gz-render labels"),
                ty: wgpu::QueryType::Occlusion,
                count: capacity,
            }),
            capacity,
            resolve: create(
                gpu,
                "gz-render label queries",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                bytes,
            ),
            readback: [readback(), readback()],
        });
    }

    /// After the frame that wrote read-back `slot` was submitted: maps it without waiting; the
    /// device's next poll that finds the frame done makes it ready ([`Self::take_labels`]).
    pub fn read_labels(&mut self, slot: usize, count: u32) {
        let Some(r) = self.queries.as_mut().and_then(|q| q.readback.get_mut(slot)) else { return };
        r.count = count;
        r.state.store(READ_MAPPING, Ordering::Release);
        let state = r.state.clone();
        r.buffer.slice(..u64::from(count) * 8).map_async(wgpu::MapMode::Read, move |res| {
            state.store(if res.is_ok() { READ_READY } else { READ_FAILED }, Ordering::Release);
        });
    }

    /// A read-back is still on its way: the render thread polls the device for it.
    pub fn labels_in_flight(&self) -> bool {
        self.queries
            .as_ref()
            .is_some_and(|q| q.readback.iter().any(|r| r.state.load(Ordering::Acquire) == READ_MAPPING))
    }

    /// The answers of a read-back that came back since the last call: its slot and which
    /// depth-tested labels had a pixel pass the depth test. A map that failed frees its slot and
    /// answers nothing.
    pub fn take_labels(&mut self) -> Option<(usize, Vec<bool>)> {
        let q = self.queries.as_mut()?;
        for (i, r) in q.readback.iter_mut().enumerate() {
            match r.state.load(Ordering::Acquire) {
                READ_READY => {
                    let bytes = u64::from(r.count) * 8;
                    let seen: Option<Vec<bool>> =
                        r.buffer.slice(..bytes).get_mapped_range().ok().map(|view| {
                            view.as_chunks::<8>().0.iter().map(|c| c.iter().any(|b| *b != 0)).collect()
                        });
                    r.buffer.unmap();
                    r.state.store(READ_IDLE, Ordering::Release);
                    if let Some(seen) = seen {
                        return Some((i, seen));
                    }
                }
                READ_FAILED => r.state.store(READ_IDLE, Ordering::Release),
                _ => {}
            }
        }
        None
    }
}

/// Maps a buffer range for reading and copies it out.
pub(crate) fn map_read(gpu: &Gpu, slice: &wgpu::BufferSlice<'_>) -> Result<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(GPU_WAIT) })
        .map_err(|e| RenderError::Device(e.to_string()))?;
    match rx.recv_timeout(GPU_WAIT) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(RenderError::Device(e.to_string())),
        Err(_) => return Err(RenderError::Timeout),
    }
    let view = slice.get_mapped_range().map_err(|e| RenderError::Device(e.to_string()))?;
    Ok(view.to_vec())
}

struct Ctx<'a> {
    gpu: &'a Gpu,
    targets: &'a Targets,
    size: [u32; 2],
    bind: &'a wgpu::BindGroup,
    meshes: &'a [Option<Arc<GpuMesh>>],
    materials: &'a [Arc<wgpu::BindGroup>],
    placeholder: &'a Arc<GpuMesh>,
    fallback_bind: &'a Arc<wgpu::BindGroup>,
    helpers: &'a wgpu::Buffer,
    dynamic: &'a wgpu::Buffer,
    helper_at: [u32; 4],
    helper_counts: [u32; 4],
    labels: &'a wgpu::Buffer,
    queries: Option<&'a Queries>,
    /// The read-back buffer this frame's label queries are copied into.
    readback: Option<&'a wgpu::Buffer>,
    query_count: u32,
    clear: [f64; 4],
    outline: bool,
}

enum Job<'a> {
    Ids(&'a [Draw]),
    Meshes {
        draws: &'a [Draw],
        first: bool,
    },
    /// The depth-tested helper lines and the sky.
    Middle(&'a [Draw]),
    /// Helper faces, the outline, the label queries, the overlays; resolves the picture.
    Last,
}

#[derive(Clone, Copy)]
enum Clear {
    Both([f64; 4]),
    None,
}

fn record_job(ctx: &Ctx<'_>, job: &Job<'_>) -> wgpu::CommandBuffer {
    let mut enc = ctx
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gz-render pass") });
    encode_job(ctx, job, &mut enc);
    enc.finish()
}

fn encode_job(ctx: &Ctx<'_>, job: &Job<'_>, enc: &mut wgpu::CommandEncoder) {
    match job {
        Job::Ids(draws) => {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gz-render ids"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ctx.targets.id_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &ctx.targets.id_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            view_rect(&mut pass, ctx.targets, ctx.size);
            frame_state(ctx, &mut pass);
            draw_meshes(ctx, &mut pass, draws, true);
        }
        Job::Meshes { draws, first } => {
            let clear = if *first { Clear::Both(ctx.clear) } else { Clear::None };
            let mut pass = begin(enc, ctx.targets, ctx.size, clear, false, None);
            frame_state(ctx, &mut pass);
            draw_meshes(ctx, &mut pass, draws, false);
        }
        Job::Middle(sky) => {
            let mut pass = begin(enc, ctx.targets, ctx.size, Clear::None, false, None);
            frame_state(ctx, &mut pass);
            draw_helpers(ctx, &mut pass, &ctx.gpu.line, 0);
            draw_meshes(ctx, &mut pass, sky, false);
        }
        Job::Last => {
            {
                let mut pass =
                    begin(enc, ctx.targets, ctx.size, Clear::None, true, ctx.queries.map(|q| &q.set));
                frame_state(ctx, &mut pass);
                draw_helpers(ctx, &mut pass, &ctx.gpu.tri, 2);
                if ctx.outline {
                    pass.set_pipeline(&ctx.gpu.outline);
                    pass.set_bind_group(1, &ctx.targets.id_bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                if ctx.queries.is_some() && ctx.query_count > 0 {
                    pass.set_pipeline(&ctx.gpu.label);
                    pass.set_vertex_buffer(0, ctx.labels.slice(..));
                    for i in 0..ctx.query_count {
                        pass.begin_occlusion_query(i);
                        pass.draw(0..6, i..i + 1);
                        pass.end_occlusion_query();
                    }
                }
                draw_helpers(ctx, &mut pass, &ctx.gpu.line_overlay, 1);
                draw_helpers(ctx, &mut pass, &ctx.gpu.tri_overlay, 3);
            }
            if let (Some(q), Some(readback)) = (ctx.queries, ctx.readback)
                && ctx.query_count > 0
            {
                enc.resolve_query_set(&q.set, 0..ctx.query_count, &q.resolve, 0);
                enc.copy_buffer_to_buffer(&q.resolve, 0, readback, 0, u64::from(ctx.query_count) * 8);
            }
        }
    }
}

fn begin<'e>(
    enc: &'e mut wgpu::CommandEncoder,
    t: &'e Targets,
    size: [u32; 2],
    clear: Clear,
    last: bool,
    queries: Option<&'e wgpu::QuerySet>,
) -> wgpu::RenderPass<'e> {
    let (color_load, depth_load) = match clear {
        Clear::Both(c) => (
            wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: c[3] }),
            wgpu::LoadOp::Clear(0.0),
        ),
        Clear::None => (wgpu::LoadOp::Load, wgpu::LoadOp::Load),
    };
    let (view, resolve_target, store) = match &t.msaa_view {
        Some(msaa) => (
            msaa,
            last.then_some(&t.color_view),
            if last { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
        ),
        None => (&t.color_view, None, wgpu::StoreOp::Store),
    };
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("gz-render scene"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target,
            ops: wgpu::Operations { load: color_load, store },
            depth_slice: None,
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &t.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: depth_load,
                store: if last { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: queries,
        multiview_mask: None,
    });
    view_rect(&mut pass, t, size);
    pass
}

/// The view is the targets' top-left corner.
fn view_rect(pass: &mut wgpu::RenderPass<'_>, t: &Targets, size: [u32; 2]) {
    let (w, h) = (size[0].min(t.extent[0]).max(1), size[1].min(t.extent[1]).max(1));
    pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
    pass.set_scissor_rect(0, 0, w, h);
}

fn frame_state(ctx: &Ctx<'_>, pass: &mut wgpu::RenderPass<'_>) {
    pass.set_bind_group(0, ctx.bind, &[]);
}

fn draw_helpers(
    ctx: &Ctx<'_>,
    pass: &mut wgpu::RenderPass<'_>,
    pipeline: &wgpu::RenderPipeline,
    part: usize,
) {
    let (Some(&at), Some(&count)) = (ctx.helper_at.get(part), ctx.helper_counts.get(part)) else { return };
    if count == 0 {
        return;
    }
    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, ctx.helpers.slice(..));
    pass.draw(at..at + count, 0..1);
}

/// The draws of a run; `ids` writes the id target instead of the picture.
fn draw_meshes(ctx: &Ctx<'_>, pass: &mut wgpu::RenderPass<'_>, draws: &[Draw], ids: bool) {
    let mut last_pipe: Option<(MeshPipe, Option<IdPipe>)> = None;
    let mut last_material = usize::MAX;
    let mut last_mesh: (usize, bool) = (0, false);
    for d in draws {
        if let Some(range) = &d.dynamic {
            // a sprite batch: its own vertices, no index buffer
            let pipe = if ids {
                let Some(p) = d.pipe.id_pipe() else { continue };
                (d.pipe, Some(p))
            } else {
                (d.pipe, None)
            };
            if range.is_empty() || d.instances.is_empty() {
                continue;
            }
            if last_pipe != Some(pipe) {
                match pipe.1 {
                    Some(p) => pass.set_pipeline(ctx.gpu.id(p)),
                    None => pass.set_pipeline(ctx.gpu.mesh(pipe.0)),
                }
                last_pipe = Some(pipe);
            }
            if d.material as usize != last_material {
                let bg = ctx.materials.get(d.material as usize).unwrap_or(ctx.fallback_bind);
                pass.set_bind_group(1, bg.as_ref(), &[]);
                last_material = d.material as usize;
            }
            pass.set_vertex_buffer(0, ctx.dynamic.slice(..));
            last_mesh = (usize::MAX, false);
            pass.draw(range.clone(), d.instances.clone());
            continue;
        }
        let (mesh, sub) = match ctx.meshes.get(d.mesh as usize) {
            Some(Some(m)) => (m, d.submesh as usize),
            _ => (ctx.placeholder, 0),
        };
        let Some(info) = mesh.info.submeshes.get(sub) else { continue };
        let pipe = if ids {
            let Some(p) = d.pipe.id_pipe() else { continue };
            (d.pipe, Some(p))
        } else {
            (d.pipe, None)
        };
        let wire = !ids && d.pipe.is_wire();
        let range = if wire { info.wire.clone() } else { info.indices.clone() };
        if range.is_empty() || d.instances.is_empty() {
            continue;
        }
        if last_pipe != Some(pipe) {
            match pipe.1 {
                Some(p) => pass.set_pipeline(ctx.gpu.id(p)),
                None => pass.set_pipeline(ctx.gpu.mesh(pipe.0)),
            }
            last_pipe = Some(pipe);
        }
        if d.material as usize != last_material {
            let bg = ctx.materials.get(d.material as usize).unwrap_or(ctx.fallback_bind);
            pass.set_bind_group(1, bg.as_ref(), &[]);
            last_material = d.material as usize;
        }
        let key = (Arc::as_ptr(mesh) as usize, wire);
        if key != last_mesh {
            if mesh.vertices.is_empty() {
                continue;
            }
            pass.set_vertex_buffer(0, mesh.buffer.slice(mesh.vertices.clone()));
            pass.set_index_buffer(
                mesh.buffer.slice(if wire { mesh.wire.clone() } else { mesh.indices.clone() }),
                wgpu::IndexFormat::Uint32,
            );
            last_mesh = key;
        }
        pass.draw_indexed(range, 0, d.instances.clone());
    }
}
