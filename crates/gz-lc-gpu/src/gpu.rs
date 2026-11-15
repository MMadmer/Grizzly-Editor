//! The graphics card's lighter: a device of its own, the kernel's two pipelines, a scene per ray
//! model, batches of samples cut to the card's pace.
//!
//! ```text
//! GpuLighter::new      the best adapter (a discrete card before an integrated one; a software one
//!                      is none), its device with the adapter's own limits, the pipelines - once
//! Lighter::scene       the tree and the triangles packed on the caller's pool and uploaded
//! LitScene::light      the samples in Morton order, cut into batches of about 100 ms, three in
//!                      flight; a batch is two passes (a ray a thread, then a sample a thread),
//!                      read back through a mapped buffer; Stop checked between batches; the first
//!                      call of a scene also lights up to 512 samples on the CPU beside the card and
//!                      refuses a card that answers otherwise
//! ```
//!
//! Every error of the device - a validation error, no memory, a lost device, a wait past its
//! limit - is an `Err` with a sentence, and `AutoLighter` lights that batch on the CPU.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use glam::Vec3;
use gz_lc_light::{
    BuildTexture, ColorC, CpuLighter, Ctl, Falloff, LcError, LightJob, LightSet, Lighter, LitScene, RayModel,
    Sample,
};
use parking_lot::Mutex;
use rayon::prelude::*;
use wgpu::util::DeviceExt as _;

use crate::pack::{self, GpuSample, PackedLights, Params};

/// How long the device's creation may take.
const OPEN_LIMIT: Duration = Duration::from_secs(30);
/// How long a batch may take on the card before it is given up (Windows resets the card after two
/// seconds of one task; a batch aims at a tenth of that).
const BATCH_LIMIT: Duration = Duration::from_secs(6);
/// What a dispatch aims at.
const TARGET: Duration = Duration::from_millis(100);
/// The first batch of a call, before the card's pace is known.
const FIRST_BATCH: usize = 16_384;
/// The smallest batch the pace makes.
const MIN_BATCH: usize = 16_384;
/// How often a batch in flight is asked after.
const POLL_EVERY: Duration = Duration::from_micros(250);
/// The most threads of one dispatch: 65535 groups of 64.
const MAX_BATCH: usize = 65_535 * 64;
const GROUP: usize = 64;
/// The CPU lights this many samples of a scene's first batch to check the card's answers.
const CHECK_SAMPLES: usize = 512;
/// The self-check lights its samples in pieces of this many.
const CHECK_PIECE: usize = 32;
/// How far apart the card's and the CPU's light of a sample may be.
const TOLERANCE: f32 = 0.002;
/// The most bytes a batch's rays' answers take.
const THROUGH_BYTES: u64 = 128 << 20;
/// Bytes a sample's light takes in the card's answer.
const OUT_BYTES: u64 = 20;

#[cfg(test)]
pub(crate) const NO_ADAPTER_VAR: &str = "GZ_LC_GPU_TEST_NO_ADAPTER";

fn err(what: impl std::fmt::Display) -> LcError {
    LcError::Invalid(what.to_string())
}

/// The device, the queue and the kernel.
struct Card {
    _instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    /// The pass a ray a thread and the pass a sample a thread.
    trace: wgpu::ComputePipeline,
    add: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The most groups of a dispatch along one axis.
    max_groups: u32,
    /// The largest buffer a binding may hold and the largest buffer.
    max_binding: u64,
    max_buffer: u64,
    /// Errors no error scope caught, and the device's loss.
    errors: Arc<Mutex<Vec<String>>>,
    lost: Arc<AtomicBool>,
}

/// Sample points lit by a compute shader on a graphics card.
pub struct GpuLighter {
    card: Card,
    /// Batch buffers a call has given back: making and dropping them cost a call more than
    /// lighting a small batch does.
    pool: Mutex<Vec<Buffers>>,
    alpha_limit: u64,
    lit: AtomicU64,
    notes: Mutex<Vec<String>>,
}

impl std::fmt::Debug for GpuLighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuLighter").field("card", &self.card.name).finish()
    }
}

impl GpuLighter {
    /// The lighter on the best adapter of this machine; the reason when there is none to use.
    pub fn new() -> Result<Self, String> {
        #[cfg(test)]
        if std::env::var_os(NO_ADAPTER_VAR).is_some() {
            return Err(format!("no graphics adapter ({NO_ADAPTER_VAR} is set)."));
        }
        let card = open_bounded()?;
        let alpha_limit = card.max_binding;
        Ok(Self {
            card,
            pool: Mutex::new(Vec::new()),
            alpha_limit,
            lit: AtomicU64::new(0),
            notes: Mutex::new(Vec::new()),
        })
    }

    /// The same lighter with a smaller alpha buffer (tests).
    #[cfg(test)]
    pub(crate) fn with_alpha_limit(mut self, bytes: u64) -> Self {
        self.alpha_limit = bytes;
        self
    }

    fn note(&self, line: String) {
        self.notes.lock().push(line);
    }

    /// A storage buffer of `bytes`, refused when the card cannot bind it.
    fn storage(&self, label: &str, bytes: &[u8]) -> gz_lc_light::Result<wgpu::Buffer> {
        let size = bytes.len() as u64;
        if size > self.card.max_binding || size > self.card.max_buffer {
            return Err(err(format!(
                "the {label} take {} MiB, more than the graphics card binds ({} MiB)",
                size >> 20,
                self.card.max_binding.min(self.card.max_buffer) >> 20
            )));
        }
        Ok(self.card.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        }))
    }

    /// Whether the device has failed outside an error scope.
    fn device_failed(&self) -> gz_lc_light::Result<()> {
        if self.card.lost.load(Ordering::Relaxed) {
            return Err(err("the graphics card was lost"));
        }
        if let Some(e) = self.card.errors.lock().first() {
            return Err(err(format!("the graphics card failed: {e}")));
        }
        Ok(())
    }
}

/// The error scopes of a stretch of work on one thread, popped in reverse.
struct Scopes(Vec<wgpu::ErrorScopeGuard>);

impl Scopes {
    fn push(device: &wgpu::Device) -> Self {
        Self(
            [wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Internal]
                .into_iter()
                .map(|f| device.push_error_scope(f))
                .collect(),
        )
    }

    fn pop(mut self) -> gz_lc_light::Result<()> {
        let mut first = None;
        while let Some(g) = self.0.pop() {
            if let Some(e) = futures::executor::block_on(g.pop())
                && first.is_none()
            {
                first = Some(e.to_string());
            }
        }
        match first {
            Some(e) => Err(err(format!("the graphics card refused the work: {e}"))),
            None => Ok(()),
        }
    }
}

/// [`open`] with a limit: a driver that hangs costs a worker of the pool, never the caller.
fn open_bounded() -> Result<Card, String> {
    // a pool of one thread would wait for the creation it is to run
    if rayon::current_thread_index().is_some() && rayon::current_num_threads() < 2 {
        return open();
    }
    let (tx, rx) = mpsc::channel();
    rayon::spawn(move || {
        let _ = tx.send(open());
    });
    rx.recv_timeout(OPEN_LIMIT).unwrap_or_else(|_| {
        Err(format!("the graphics card did not answer within {} s.", OPEN_LIMIT.as_secs()))
    })
}

/// How much an adapter is preferred: lower first.
fn rank(info: &wgpu::AdapterInfo) -> Option<(u8, u8)> {
    let kind = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        // a software adapter is no graphics card
        wgpu::DeviceType::Cpu => return None,
    };
    let backend = match info.backend {
        wgpu::Backend::Vulkan => 0,
        wgpu::Backend::Dx12 => 1,
        _ => 2,
    };
    Some((kind, backend))
}

fn open() -> Result<Card, String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc);
    let mut adapters: Vec<(wgpu::Adapter, (u8, u8))> = futures::executor::block_on(
        instance.enumerate_adapters(wgpu::Backends::DX12 | wgpu::Backends::VULKAN),
    )
    .into_iter()
    .filter_map(|a| rank(&a.get_info()).map(|r| (a, r)))
    .collect();
    adapters.sort_by_key(|(_, r)| *r);
    // the best adapter's reason is the one worth reading
    let mut why: Option<String> = None;
    for (adapter, _) in adapters {
        match open_on(instance.clone(), &adapter) {
            Ok(card) => return Ok(card),
            Err(e) => {
                why.get_or_insert(e);
            }
        }
    }
    Err(why.unwrap_or_else(|| "no graphics adapter.".to_owned()))
}

fn open_on(instance: wgpu::Instance, adapter: &wgpu::Adapter) -> Result<Card, String> {
    let info = adapter.get_info();
    let limits = adapter.limits();
    if limits.max_storage_buffers_per_shader_stage < 9 {
        return Err(format!("{} binds too few storage buffers.", info.name));
    }
    let (device, queue) = futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gz-lc-gpu"),
        required_features: wgpu::Features::empty(),
        required_limits: limits.clone(),
        ..Default::default()
    }))
    .map_err(|e| format!("{} gave no device: {e}", info.name))?;
    let errors = Arc::new(Mutex::new(Vec::new()));
    let lost = Arc::new(AtomicBool::new(false));
    {
        let errors = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| errors.lock().push(e.to_string())));
        let lost = lost.clone();
        device.set_device_lost_callback(move |_, _| lost.store(true, Ordering::Relaxed));
    }
    let scopes = Scopes::push(&device);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gz-lc-gpu light"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("light.wgsl"))),
    });
    let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    entries.extend((1..8).map(|b| storage(b, true)));
    entries.push(storage(8, false));
    entries.push(storage(9, false));
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gz-lc-gpu light"),
        entries: &entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gz-lc-gpu light"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gz-lc-gpu light"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    };
    let (trace, add) = (pipeline("trace_main"), pipeline("add_main"));
    warm_up(&device, &queue, &layout, [&trace, &add])?;
    scopes.pop().map_err(|e| format!("{}: {e}", info.name))?;
    Ok(Card {
        _instance: instance,
        max_binding: limits.max_storage_buffer_binding_size,
        max_buffer: limits.max_buffer_size,
        device,
        queue,
        name: info.name,
        trace,
        add,
        layout,
        max_groups: limits.max_compute_workgroups_per_dimension,
        errors,
        lost,
    })
}

/// One dispatch of nothing: the driver finishes the kernel now, not in a build's first batch.
fn warm_up(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    pipelines: [&wgpu::ComputePipeline; 2],
) -> Result<(), String> {
    use wgpu::BufferUsages as U;
    let zeros = [0u8; 256];
    let buffers: Vec<wgpu::Buffer> = (0..10)
        .map(|i| {
            let usage = if i == 0 { U::UNIFORM } else { U::STORAGE };
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gz-lc-gpu warm-up"),
                contents: &zeros,
                usage,
            })
        })
        .collect();
    let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
        .collect();
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gz-lc-gpu warm-up"),
        layout,
        entries: &entries,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gz-lc-gpu warm-up") });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gz-lc-gpu warm-up"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &bind, &[]);
        for p in pipelines {
            pass.set_pipeline(p);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
    let index = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: Some(BATCH_LIMIT) })
        .map(|_| ())
        .map_err(|e| format!("the graphics card did not run the lighter's kernel: {e}"))
}

impl Lighter for GpuLighter {
    fn name(&self) -> String {
        self.card.name.clone()
    }

    fn scene<'a>(
        &'a self,
        model: &'a RayModel,
        textures: &'a [BuildTexture],
    ) -> gz_lc_light::Result<Box<dyn LitScene + 'a>> {
        self.device_failed()?;
        let packed = pack::scene(model, textures, self.alpha_limit);
        // the kernel's last slot is where a push past the end lands: a walk may keep one node fewer
        if packed.stack >= crate::bvh::STACK {
            return Err(err(format!(
                "a walk of the lighter's tree may keep {} nodes, more than its stack holds",
                packed.stack
            )));
        }
        if packed.halved > 0 {
            self.note(format!(
                "The alpha of the transparent faces' textures was made smaller {} times to fit the graphics card.",
                packed.halved
            ));
        }
        let scopes = Scopes::push(&self.card.device);
        let made = (|| {
            Ok::<_, LcError>((
                self.storage("tree's nodes", bytemuck::cast_slice(&packed.nodes))?,
                self.storage("triangles", bytemuck::cast_slice(&packed.tris))?,
                self.storage("transparent faces", bytemuck::cast_slice(&packed.transp))?,
                self.storage("alpha of the textures", bytemuck::cast_slice(&packed.alpha))?,
            ))
        })();
        scopes.pop()?;
        let (nodes, tris, transp, alpha) = made?;
        // the buffers reach the card with the next submission: the scene is ready when they have
        self.card
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(self.card.queue.submit([])),
                timeout: Some(BATCH_LIMIT),
            })
            .map_err(|e| err(format!("the scene did not reach the graphics card: {e}")))?;
        Ok(Box::new(GpuScene {
            owner: self,
            model,
            textures,
            nodes,
            tris,
            transp,
            alpha,
            jobs: Mutex::new(None),
            state: AtomicU8::new(UNCHECKED),
        }))
    }

    fn samples(&self) -> u64 {
        self.lit.load(Ordering::Relaxed)
    }

    fn notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.lock())
    }
}

const UNCHECKED: u8 = 0;
const CHECKED: u8 = 1;
const FAILED: u8 = 2;

/// A job's lights on the card.
struct JobBuffers {
    lights: wgpu::Buffer,
    grid: wgpu::Buffer,
    params: Params,
}

struct GpuScene<'a> {
    owner: &'a GpuLighter,
    model: &'a RayModel,
    textures: &'a [BuildTexture],
    nodes: wgpu::Buffer,
    tris: wgpu::Buffer,
    transp: wgpu::Buffer,
    alpha: wgpu::Buffer,
    /// The last job's lights: a caller lights batch after batch with one set.
    jobs: Mutex<Option<(LightSet, Falloff, Arc<JobBuffers>)>>,
    /// Whether the card's answers were checked against the CPU's.
    state: AtomicU8,
}

/// The buffers of one batch: its block of parameters, its samples, the card's answer and the
/// mapped copy of it.
struct Buffers {
    params: wgpu::Buffer,
    samples: wgpu::Buffer,
    out: wgpu::Buffer,
    read: wgpu::Buffer,
    through: wgpu::Buffer,
}

/// The pool keeps this many batches' buffers.
const POOLED: usize = 6;

/// A batch in flight's buffers, bound for one scene and job.
struct Slot {
    b: Buffers,
    bind: wgpu::BindGroup,
}

struct InFlight {
    slot: usize,
    /// Where the batch's samples are in the Morton order.
    from: usize,
    len: usize,
    submitted: Instant,
    mapped: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

/// The samples' order along a Morton curve over their box: neighbours in the order are
/// neighbours in space, so the rays of a group walk the same nodes.
fn morton_order(samples: &[Sample]) -> Vec<u32> {
    let (lo, hi) = samples
        .par_iter()
        .map(|s| (Vec3::from(s.p), Vec3::from(s.p)))
        .reduce(|| (Vec3::splat(f32::MAX), Vec3::splat(-f32::MAX)), |a, b| (a.0.min(b.0), a.1.max(b.1)));
    let scale = Vec3::splat(1023.0) / (hi - lo).max(Vec3::splat(1e-6));
    let spread = |x: u32| {
        let mut x = x & 0x3ff;
        x = (x | (x << 16)) & 0x0300_00ff;
        x = (x | (x << 8)) & 0x0300_f00f;
        x = (x | (x << 4)) & 0x030c_30c3;
        (x | (x << 2)) & 0x0924_9249
    };
    let mut keyed: Vec<(u32, u32)> = samples
        .par_iter()
        .enumerate()
        .map(|(i, s)| {
            let q = ((Vec3::from(s.p) - lo) * scale).clamp(Vec3::ZERO, Vec3::splat(1023.0));
            let q = if q.is_finite() { q } else { Vec3::ZERO };
            (spread(q.x as u32) | (spread(q.y as u32) << 1) | (spread(q.z as u32) << 2), i as u32)
        })
        .collect();
    keyed.par_sort_unstable();
    keyed.into_iter().map(|(_, i)| i).collect()
}

fn apart(a: &ColorC, b: &ColorC) -> f32 {
    (a.rgb - b.rgb).abs().max_element().max((a.hemi - b.hemi).abs()).max((a.sun - b.sun).abs())
}

impl GpuScene<'_> {
    fn card(&self) -> &Card {
        &self.owner.card
    }

    fn job(&self, job: &LightJob<'_>) -> gz_lc_light::Result<Arc<JobBuffers>> {
        let mut cached = self.jobs.lock();
        if let Some((set, falloff, bufs)) = cached.as_ref()
            && *falloff == job.falloff
            && set == job.lights
        {
            return Ok(bufs.clone());
        }
        let PackedLights { lights, grid, params } =
            pack::lights(job.lights, job.falloff, self.model.opacity(), self.model.cull());
        let bufs = Arc::new(JobBuffers {
            lights: self.owner.storage("lights", bytemuck::cast_slice(&lights))?,
            grid: self.owner.storage("grid of the lights", bytemuck::cast_slice(&grid))?,
            params,
        });
        *cached = Some((job.lights.clone(), job.falloff, bufs.clone()));
        Ok(bufs)
    }

    /// Buffers for a batch of `cap` samples: the pool's smallest that holds it, or new ones of a
    /// size the next calls can take too.
    fn buffers(&self, cap: usize, rays: usize) -> gz_lc_light::Result<Buffers> {
        let samples_size = (cap * std::mem::size_of::<GpuSample>()) as u64;
        let out_size = cap as u64 * OUT_BYTES;
        let through_size = (cap * rays * 4) as u64;
        {
            let mut pool = self.owner.pool.lock();
            let fits = |b: &Buffers| {
                b.samples.size() >= samples_size
                    && b.out.size() >= out_size
                    && b.through.size() >= through_size
            };
            if let Some(k) =
                (0..pool.len()).filter(|&k| fits(&pool[k])).min_by_key(|&k| pool[k].samples.size())
            {
                return Ok(pool.swap_remove(k));
            }
        }
        let limit = self.card().max_binding.min(self.card().max_buffer);
        if samples_size > limit || out_size > limit || through_size > limit {
            return Err(err("a batch of samples is larger than the graphics card binds"));
        }
        let up = |size: u64| size.next_power_of_two().min(limit);
        let (samples_size, out_size, through_size) = (up(samples_size), up(out_size), up(through_size));
        let device = &self.card().device;
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        use wgpu::BufferUsages as U;
        Ok(Buffers {
            params: buffer("light params", std::mem::size_of::<Params>() as u64, U::UNIFORM | U::COPY_DST),
            samples: buffer("light samples", samples_size, U::STORAGE | U::COPY_DST),
            out: buffer("light out", out_size, U::STORAGE | U::COPY_SRC),
            read: buffer("light read", out_size, U::MAP_READ | U::COPY_DST),
            through: buffer("light through", through_size, U::STORAGE),
        })
    }

    fn give_back(&self, slots: Vec<Slot>) {
        let mut pool = self.owner.pool.lock();
        pool.extend(slots.into_iter().map(|s| s.b));
        // the largest stay
        pool.sort_by_key(|b| std::cmp::Reverse(b.samples.size()));
        pool.truncate(POOLED);
    }

    fn slot(&self, cap: usize, bufs: &JobBuffers) -> gz_lc_light::Result<Slot> {
        let device = &self.card().device;
        let b = self.buffers(cap, bufs.params.g[1].max(1) as usize)?;
        let entries = [
            &b.params,
            &self.nodes,
            &self.tris,
            &self.transp,
            &self.alpha,
            &bufs.lights,
            &bufs.grid,
            &b.samples,
            &b.out,
            &b.through,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
        .collect::<Vec<_>>();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gz-lc-gpu light"),
            layout: &self.card().layout,
            entries: &entries,
        });
        Ok(Slot { b, bind })
    }

    fn submit(&self, slot: &Slot, params: &Params, batch: &[GpuSample]) -> InFlightMap {
        let card = self.card();
        let rays = params.g[1].max(1);
        let mut p = *params;
        p.a[0] = batch.len() as u32;
        card.queue.write_buffer(&slot.b.params, 0, bytemuck::bytes_of(&p));
        card.queue.write_buffer(&slot.b.samples, 0, bytemuck::cast_slice(batch));
        let mut encoder = card
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gz-lc-gpu light") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gz-lc-gpu light"),
                timestamp_writes: None,
            });
            let groups = batch.len().div_ceil(GROUP) as u32;
            pass.set_bind_group(0, &slot.bind, &[]);
            pass.set_pipeline(&card.trace);
            pass.dispatch_workgroups(rays, groups, 1);
        }
        {
            // a pass of its own: the rays' answers are written before they are added up
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gz-lc-gpu add"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &slot.bind, &[]);
            pass.set_pipeline(&card.add);
            pass.dispatch_workgroups(batch.len().div_ceil(GROUP) as u32, 1, 1);
        }
        let bytes = batch.len() as u64 * OUT_BYTES;
        encoder.copy_buffer_to_buffer(&slot.b.out, 0, &slot.b.read, 0, bytes);
        card.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        slot.b.read.slice(..bytes).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        rx
    }

    /// Waits for a batch and writes its light where its samples came from.
    fn finish(
        &self,
        slot: &Slot,
        f: &InFlight,
        order: &[u32],
        out: &mut [ColorC],
    ) -> gz_lc_light::Result<()> {
        let card = self.card();
        // the device is asked often rather than waited on: a driver's wait wakes on the system's
        // timer, some 15 ms late, which costs more than a short batch takes
        let began = Instant::now();
        loop {
            card.device
                .poll(wgpu::PollType::Poll)
                .map_err(|e| err(format!("the graphics card failed: {e}")))?;
            match f.mapped.try_recv() {
                Ok(Ok(())) => break,
                Ok(Err(e)) => return Err(err(format!("the graphics card's answer could not be read: {e}"))),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(err("the graphics card's answer did not come"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if began.elapsed() > BATCH_LIMIT {
                return Err(err(format!(
                    "the graphics card did not finish a batch of {} samples within {} s",
                    f.len,
                    BATCH_LIMIT.as_secs()
                )));
            }
            std::thread::sleep(POLL_EVERY);
        }
        let bytes = f.len as u64 * OUT_BYTES;
        {
            let view = slot
                .b
                .read
                .slice(..bytes)
                .get_mapped_range()
                .map_err(|e| err(format!("the graphics card's answer could not be read: {e}")))?;
            let floats: Vec<f32> = view.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
            for (k, light) in floats.as_chunks::<5>().0.iter().enumerate() {
                let Some(&i) = order.get(f.from + k) else { continue };
                if let Some(o) = out.get_mut(i as usize) {
                    *o = ColorC {
                        rgb: Vec3::new(light[0], light[1], light[2]),
                        hemi: light[3],
                        sun: light[4],
                        tmp: 0.0,
                    };
                }
            }
        }
        slot.b.read.unmap();
        self.owner.device_failed()
    }

    /// Every sample lit on the card, in batches cut to its pace.
    fn run(
        &self,
        bufs: &JobBuffers,
        samples: &[Sample],
        out: &mut [ColorC],
        ctl: &Ctl<'_>,
    ) -> gz_lc_light::Result<()> {
        let n = samples.len();
        let order = if n > GROUP { morton_order(samples) } else { (0..n as u32).collect() };
        let rays = bufs.params.g[1].max(1);
        if rays > self.card().max_groups {
            return Err(err(format!(
                "a sample has {rays} lights, more than a dispatch of the graphics card takes"
            )));
        }
        // the rays' answers of a batch stay within a buffer of `THROUGH_BYTES`
        let cap = n.min(MAX_BATCH).min((THROUGH_BYTES / (u64::from(rays) * 4)).max(GROUP as u64) as usize);
        let slots = vec![self.slot(cap, bufs)?, self.slot(cap, bufs)?, self.slot(cap, bufs)?];
        let mut flying: VecDeque<InFlight> = VecDeque::new();
        let mut next = 0usize;
        let mut size = FIRST_BATCH.min(cap);
        let mut last_done: Option<Instant> = None;
        let mut batch: Vec<GpuSample> = Vec::with_capacity(cap);
        while next < n || !flying.is_empty() {
            while flying.len() < slots.len() && next < n {
                ctl.check()?;
                let free = (0..slots.len()).find(|s| flying.iter().all(|f| f.slot != *s)).unwrap_or(0);
                let len = size.min(n - next);
                batch.clear();
                batch.extend(order[next..next + len].iter().map(|&i| GpuSample::of(&samples[i as usize])));
                let mapped = self.submit(&slots[free], &bufs.params, &batch);
                flying.push_back(InFlight { slot: free, from: next, len, submitted: Instant::now(), mapped });
                next += len;
            }
            let Some(f) = flying.pop_front() else { break };
            self.finish(&slots[f.slot], &f, &order, out)?;
            let done = Instant::now();
            let began = last_done.map_or(f.submitted, |l| l.max(f.submitted));
            last_done = Some(done);
            // the pace: a batch of the size asked for, timed while the card was busy with it alone
            if f.len == size {
                let busy = done.duration_since(began).as_secs_f64().max(1e-4);
                let want = (f.len as f64 * TARGET.as_secs_f64() / busy) as usize;
                size = want.clamp(size / 2, size * 4).clamp(MIN_BATCH.min(cap), cap);
            }
        }
        self.give_back(slots);
        ctl.check()?;
        Ok(())
    }

    fn settle(&self, checked: gz_lc_light::Result<()>) -> gz_lc_light::Result<()> {
        let state = match &checked {
            Ok(()) => CHECKED,
            Err(LcError::Cancelled) => UNCHECKED,
            Err(_) => FAILED,
        };
        self.state.store(state, Ordering::Relaxed);
        checked
    }
}

/// The samples the self-check lights on the CPU: up to [`CHECK_SAMPLES`], spread over the batch.
fn picked(n: usize) -> Vec<usize> {
    let take = n.min(CHECK_SAMPLES);
    let step = n / take.max(1);
    (0..take).map(|k| k * step).collect()
}

/// The CPU's light of the picked samples.
fn cpu_light(
    model: &RayModel,
    textures: &[BuildTexture],
    job: &LightJob<'_>,
    samples: &[Sample],
    picked: &[usize],
    ctl: &Ctl<'_>,
) -> gz_lc_light::Result<Vec<ColorC>> {
    let sub: Vec<Sample> = picked.iter().filter_map(|&i| samples.get(i).copied()).collect();
    let mut want = vec![ColorC::default(); sub.len()];
    let cpu = CpuLighter::default();
    let scene = cpu.scene(model, textures)?;
    // in small pieces: every thread of the pool takes some, and a Stop is heard between them
    sub.par_chunks(CHECK_PIECE)
        .zip(want.par_chunks_mut(CHECK_PIECE))
        .try_for_each(|(s, w)| scene.light(job, s, w, ctl))?;
    Ok(want)
}

/// The card's answers against the CPU's: refused when fewer than 99 % agree within [`TOLERANCE`].
fn compare(picked: &[usize], want: &[ColorC], out: &[ColorC]) -> gz_lc_light::Result<()> {
    let take = picked.len();
    let agree = picked
        .iter()
        .zip(want)
        .filter(|&(&i, w)| out.get(i).is_some_and(|o| apart(o, w) <= TOLERANCE))
        .count();
    if agree * 100 < take * 99 {
        return Err(err(format!(
            "the graphics card's answers differ from the CPU's: {agree} of {take} samples agree within {TOLERANCE}"
        )));
    }
    Ok(())
}

/// The self-check of a batch the card lit.
#[cfg(test)]
pub(crate) fn check_against_cpu(
    model: &RayModel,
    textures: &[BuildTexture],
    job: &LightJob<'_>,
    samples: &[Sample],
    out: &[ColorC],
    ctl: &Ctl<'_>,
) -> gz_lc_light::Result<()> {
    let picked = picked(samples.len().min(out.len()));
    compare(&picked, &cpu_light(model, textures, job, samples, &picked, ctl)?, out)
}

type InFlightMap = mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>;

impl LitScene for GpuScene<'_> {
    fn light(
        &self,
        job: &LightJob<'_>,
        samples: &[Sample],
        out: &mut [ColorC],
        ctl: &Ctl<'_>,
    ) -> gz_lc_light::Result<()> {
        if samples.len() != out.len() {
            return Err(err(format!(
                "A batch of {} samples was given room for {} lights.",
                samples.len(),
                out.len()
            )));
        }
        ctl.check()?;
        if self.state.load(Ordering::Relaxed) == FAILED {
            return Err(err("the graphics card's answers differ from the CPU's"));
        }
        if samples.is_empty() {
            return Ok(());
        }
        self.owner.device_failed()?;
        // the first batch of a scene is also lit on the CPU, beside the card's work
        let check = (self.state.load(Ordering::Relaxed) == UNCHECKED).then(|| picked(samples.len()));
        let mut want = None;
        let mut ran = Ok(());
        rayon::in_place_scope(|s| {
            if let Some(picked) = &check {
                let want = &mut want;
                s.spawn(move |_| {
                    *want = Some(cpu_light(self.model, self.textures, job, samples, picked, ctl))
                });
            }
            // the error scopes are this thread's: the card's work stays on it
            let scopes = Scopes::push(&self.card().device);
            let lit = self.job(job).and_then(|bufs| self.run(&bufs, samples, out, ctl));
            ran = lit.and(scopes.pop());
        });
        ran?;
        if let (Some(picked), Some(want)) = (&check, want) {
            self.settle(want.and_then(|want| compare(picked, &want, out)))?;
        }
        self.owner.lit.fetch_add(samples.len() as u64, Ordering::Relaxed);
        Ok(())
    }
}
