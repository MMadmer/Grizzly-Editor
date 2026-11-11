//! The renderer: the render thread, the targets the UI shows, read-backs and offscreen pictures.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gz_fmt_tex::Image;
use gz_task::Executors;
use parking_lot::{Condvar, Mutex};

use crate::assets::{AssetGpu, Assets, Budgets, upload_texture_to};
use crate::color::Color;
use crate::frame::{FrameResources, GPU_WAIT, map_read};
use crate::gpu::Gpu;
use crate::material::MaterialTable;
use crate::packet::{FramePacket, PacketStats, ScreenLabel};
use crate::targets::{Targets, target_extent};
use crate::texture::TextureData;
use crate::{RenderError, Result};

/// The last finished picture, for the UI to show: the view is the top-left `size` of the
/// texture; `uv` is the corner of it in texture coordinates.
#[derive(Clone, Debug)]
pub struct TargetHandle {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// The render size.
    pub size: [u32; 2],
    /// The texture's size: the size rounded up to 64 px buckets.
    pub extent: [u32; 2],
    /// Changes when the texture is made again: register the new view with the UI then.
    pub serial: u64,
    /// The frame this picture is.
    pub frame: u64,
}

impl TargetHandle {
    #[must_use]
    pub fn uv(&self) -> [f32; 2] {
        [
            self.size[0] as f32 / self.extent[0].max(1) as f32,
            self.size[1] as f32 / self.extent[1].max(1) as f32,
        ]
    }
}

/// The frame clock: what the last finished frame drew.
#[derive(Clone, Debug, Default)]
pub struct FrameInfo {
    /// Finished frames so far.
    pub frame: u64,
    /// The serial [`Renderer::submit`] gave the packet the frame drew.
    pub packet: u64,
    pub size: [u32; 2],
    /// The packet's labels with the visibility of the depth-tested ones settled.
    pub labels: Arc<[ScreenLabel]>,
    pub stats: PacketStats,
    /// The graphics device failed the frame.
    pub error: Option<String>,
}

struct Capture {
    after: u64,
    reply: mpsc::Sender<Result<Image>>,
}

#[derive(Default)]
struct Mail {
    packet: Option<Arc<FramePacket>>,
    serial: u64,
    redraw: bool,
    captures: Vec<Capture>,
    quit: bool,
}

#[derive(Default)]
struct Done {
    info: FrameInfo,
    target: Option<TargetHandle>,
}

struct Shared {
    gpu: Gpu,
    assets: Arc<Assets>,
    exec: Executors,
    mail: Mutex<Mail>,
    mail_cv: Condvar,
    done: Mutex<Done>,
    done_cv: Condvar,
    offscreen: Mutex<Vec<(Targets, FrameResources)>>,
    target_serial: AtomicU64,
}

/// The editor's renderer. Owns the render thread; dropping it stops the thread.
pub struct Renderer {
    shared: Arc<Shared>,
    /// Taken by [`Renderer::shutdown`]: the thread is joined once, by whoever stops it first.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Renderer {
    /// Takes the device and the queue the window draws with (shared, both `Send + Sync`), makes
    /// the pipelines and starts the render thread. `exec`'s CPU pool records large passes and
    /// decodes assets, its IO pool reads them, its `Wake` is pinged after every frame.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, exec: Executors) -> Result<Self> {
        let gpu = Gpu::new(device, queue);
        let checker = TextureData::checker(64, 8, Color::rgb(200, 200, 200), Color::rgb(140, 140, 140));
        let white = upload_texture_to(
            &gpu.device,
            &gpu.queue,
            gpu.bc,
            gpu.max_side,
            &TextureData::solid(Color::WHITE),
            "gz-render white",
        )?;
        let checker =
            upload_texture_to(&gpu.device, &gpu.queue, gpu.bc, gpu.max_side, &checker, "gz-render checker")?;
        let white_cube = upload_texture_to(
            &gpu.device,
            &gpu.queue,
            gpu.bc,
            gpu.max_side,
            &TextureData::solid_cube(Color::WHITE),
            "gz-render white cube",
        )?;
        let asset_gpu = AssetGpu {
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            material_layout: gpu.material_layout.clone(),
            sampler: gpu.sampler.clone(),
            white,
            checker,
            white_cube,
            bc: gpu.bc,
            max_side: gpu.max_side,
        };
        let assets = Assets::with_gpu(asset_gpu, exec.clone(), MaterialTable::default(), Budgets::default());
        let shared = Arc::new(Shared {
            gpu,
            assets: assets.clone(),
            exec,
            mail: Mutex::new(Mail::default()),
            mail_cv: Condvar::new(),
            done: Mutex::new(Done::default()),
            done_cv: Condvar::new(),
            offscreen: Mutex::new(Vec::new()),
            target_serial: AtomicU64::new(0),
        });
        let weak: Weak<Shared> = Arc::downgrade(&shared);
        assets.set_on_change(Arc::new(move || {
            if let Some(s) = weak.upgrade() {
                s.mail.lock().redraw = true;
                s.mail_cv.notify_all();
            }
        }));
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("gz-render".into())
            .spawn(move || render_loop(&worker))
            .map_err(|e| RenderError::Device(format!("the render thread did not start: {e}")))?;
        Ok(Self { shared, thread: Mutex::new(Some(thread)) })
    }

    /// The meshes, textures and materials frames draw with.
    #[must_use]
    pub fn assets(&self) -> &Arc<Assets> {
        &self.shared.assets
    }

    /// Hands a packet to the render thread; an older one it has not started is dropped. Never
    /// blocks. The serial names the packet in [`FrameInfo::packet`].
    pub fn submit(&self, packet: Arc<FramePacket>) -> u64 {
        let serial = {
            let mut m = self.shared.mail.lock();
            m.serial += 1;
            m.packet = Some(packet);
            m.serial
        };
        self.shared.mail_cv.notify_all();
        serial
    }

    /// The last finished picture; `None` before the first frame.
    #[must_use]
    pub fn target(&self) -> Option<TargetHandle> {
        self.shared.done.lock().target.clone()
    }

    /// What the last finished frame drew.
    #[must_use]
    pub fn finished(&self) -> FrameInfo {
        self.shared.done.lock().info.clone()
    }

    /// Waits for the frame that drew packet `serial` or a later one. Blocks: for a job, never
    /// the UI thread.
    pub fn wait_frame(&self, serial: u64, timeout: Duration) -> Result<FrameInfo> {
        let deadline = Instant::now() + timeout;
        let mut done = self.shared.done.lock();
        while done.info.packet < serial {
            if self.stopped() {
                return Err(RenderError::Stopped);
            }
            if self.shared.done_cv.wait_until(&mut done, deadline).timed_out() {
                return Err(RenderError::Timeout);
            }
        }
        Ok(done.info.clone())
    }

    /// A copy of the picture of the frame that drew packet `serial` or a later one, cropped to
    /// the render size (`screenshot_viewport`). Blocks: for a job, never the UI thread.
    pub fn capture(&self, serial: u64, timeout: Duration) -> Result<Image> {
        let (tx, rx) = mpsc::channel();
        self.shared.mail.lock().captures.push(Capture { after: serial, reply: tx });
        self.shared.mail_cv.notify_all();
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(RenderError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RenderError::Stopped),
        }
    }

    /// Draws a packet into targets of its own and reads the picture back, `size` pixels (the
    /// packet's view size, for its projection): thumbnails, the model preview, `preview.png`.
    /// Runs on the calling thread and blocks until the GPU is done: call it from a job.
    pub fn render_offscreen(&self, packet: &FramePacket, size: [u32; 2]) -> Result<Image> {
        let gpu = &self.shared.gpu;
        check_size(size, gpu.max_side)?;
        let extent = target_extent(None, size).map(|s| s.min(gpu.max_side));
        let slot = {
            let mut pool = self.shared.offscreen.lock();
            pool.iter().position(|(t, _)| t.extent == extent).map(|i| pool.swap_remove(i))
        };
        let (targets, mut res) =
            slot.unwrap_or_else(|| (Targets::new(gpu, extent, 0), FrameResources::new(gpu)));
        let rec = res.record(gpu, &self.shared.assets, packet, &targets, size, self.shared.exec.cpu_pool());
        let index = gpu.queue.submit(rec.buffers);
        let waited =
            gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: Some(GPU_WAIT) });
        let picture = match waited {
            Ok(_) => read_picture(gpu, &targets, size),
            Err(e) => Err(RenderError::Device(e.to_string())),
        };
        let mut pool = self.shared.offscreen.lock();
        if pool.len() < 4 {
            pool.push((targets, res));
        }
        picture
    }

    /// Samples a pixel of the picture: 4 with MSAA, 1 without.
    #[must_use]
    pub fn samples(&self) -> u32 {
        self.shared.gpu.samples
    }

    /// Whether BC1-3 textures go to the GPU as they are.
    #[must_use]
    pub fn block_compressed_textures(&self) -> bool {
        self.shared.gpu.bc
    }
}

impl Renderer {
    /// Stops the render thread and waits for it: what it owns on the GPU (its frame resources,
    /// its targets) is dropped there, on its own thread, before this returns. The window calls it
    /// for every renderer while the event loop is still alive; other handles of this renderer
    /// then answer [`RenderError::Stopped`]. Calling it again does nothing.
    pub fn shutdown(&self) {
        let captures = {
            let mut m = self.shared.mail.lock();
            m.quit = true;
            m.packet = None;
            std::mem::take(&mut m.captures)
        };
        // a capture nobody will answer is told so at once
        drop(captures);
        self.shared.mail_cv.notify_all();
        let thread = self.thread.lock().take();
        if let Some(t) = thread {
            let _ = t.join();
        }
        self.shared.done_cv.notify_all();
    }

    fn stopped(&self) -> bool {
        self.thread.lock().as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn check_size(size: [u32; 2], max: u32) -> Result<()> {
    if size[0] == 0 || size[1] == 0 || size[0] > max || size[1] > max {
        return Err(RenderError::Size { width: size[0], height: size[1], max });
    }
    Ok(())
}

/// The picture's top-left `size`, row by row.
fn read_picture(gpu: &Gpu, t: &Targets, size: [u32; 2]) -> Result<Image> {
    let (w, h) = (size[0].min(t.extent[0]), size[1].min(t.extent[1]));
    let row = w * 4;
    let padded = wgpu::util::align_to(row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gz-render read-back"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gz-render read-back") });
    enc.copy_texture_to_buffer(
        t.color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let raw = map_read(gpu, &buffer.slice(..))?;
    buffer.unmap();
    let mut rgba = Vec::with_capacity((row * h) as usize);
    for r in raw.chunks_exact(padded as usize).take(h as usize) {
        rgba.extend_from_slice(r.get(..row as usize).unwrap_or(&[]));
    }
    Ok(Image::new(w, h, rgba)?)
}

/// The render thread: the newest packet, or the last one again when an asset it waited for
/// arrived; pictures for whoever asked for one.
fn render_loop(s: &Shared) {
    let gpu = &s.gpu;
    let mut res = FrameResources::new(gpu);
    let mut targets: Option<Targets> = None;
    let mut last: Option<(Arc<FramePacket>, u64)> = None;
    let mut last_pending = false;
    let mut frames = 0u64;
    loop {
        let job = {
            let mut m = s.mail.lock();
            loop {
                if m.quit {
                    return;
                }
                let drawn = last.as_ref().map_or(0, |(_, serial)| *serial);
                if let Some(p) = m.packet.take() {
                    m.redraw = false;
                    break Some((p, m.serial));
                }
                if m.redraw
                    && last_pending
                    && let Some(l) = &last
                {
                    m.redraw = false;
                    break Some(l.clone());
                }
                m.redraw = false;
                if targets.is_some() && m.captures.iter().any(|c| c.after <= drawn) {
                    break None;
                }
                s.mail_cv.wait(&mut m);
            }
        };

        if let Some((packet, serial)) = job {
            let size = packet.view.size.map(|v| v.clamp(1, gpu.max_side));
            let extent = target_extent(targets.as_ref().map(|t| t.extent), size).map(|v| v.min(gpu.max_side));
            if targets.as_ref().is_none_or(|t| t.extent != extent) {
                targets = Some(Targets::new(gpu, extent, s.target_serial.fetch_add(1, Ordering::AcqRel) + 1));
            }
            let Some(t) = targets.as_ref() else { continue };
            let rec = res.record(gpu, &s.assets, &packet, t, size, s.exec.cpu_pool());
            let index = gpu.queue.submit(rec.buffers);
            let error = gpu
                .device
                .poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: Some(GPU_WAIT) })
                .err()
                .map(|e| e.to_string());
            let mut labels: Vec<ScreenLabel> = packet.labels().to_vec();
            if rec.queries > 0 && error.is_none() {
                let seen = res.label_visibility(gpu, rec.queries);
                for l in &mut labels {
                    if let Some(q) = l.query {
                        l.visible = seen.get(q as usize).copied().unwrap_or(true);
                    }
                }
            }
            frames += 1;
            last_pending = rec.pending || packet.waiting_on_assets();
            {
                let mut d = s.done.lock();
                d.info = FrameInfo {
                    frame: frames,
                    packet: serial,
                    size,
                    labels: labels.into(),
                    stats: packet.stats,
                    error,
                };
                d.target = Some(TargetHandle {
                    texture: t.color.clone(),
                    view: t.color_view.clone(),
                    size,
                    extent: t.extent,
                    serial: t.serial,
                    frame: frames,
                });
            }
            s.done_cv.notify_all();
            s.exec.wake().ping();
            last = Some((packet, serial));
        }

        // pictures of what is now on the target
        let drawn = last.as_ref().map_or(0, |(_, serial)| *serial);
        let ready: Vec<Capture> = {
            let mut m = s.mail.lock();
            let (now, later): (Vec<Capture>, Vec<Capture>) =
                m.captures.drain(..).partition(|c| c.after <= drawn);
            m.captures = later;
            now
        };
        if let (false, Some(t), Some((p, _))) = (ready.is_empty(), targets.as_ref(), last.as_ref()) {
            let size = p.view.size.map(|v| v.clamp(1, gpu.max_side));
            let picture = read_picture(gpu, t, size);
            for c in ready {
                let _ = c.reply.send(picture.clone());
            }
        }
    }
}
