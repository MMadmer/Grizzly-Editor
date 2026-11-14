//! The window and its frame loop.
//!
//! The editor draws on demand: a frame runs when the system asks for one, when egui wants a
//! repaint, or when anything pings the editor's `Wake` - a store published, a query arrived, a
//! request is waiting for a picture. Frames run straight from the event loop rather than through
//! a redraw request, so a minimised or covered editor still runs its UI pass and can be
//! pictured: the pass draws into a capture texture when no surface frame is to be had.
//!
//! With panels outside the window allowed (`WindowOptions::floating`), egui's immediate
//! viewports get native windows of their own: the root frame runs every viewport's pass (a
//! panel shows its window from inside the Level tab's pass), each with the input of its window,
//! painted on a surface of its own by the one painter. A window is made after the pass that first
//! asked for it - the pass before it paints nothing - and goes when no pass asks for it any more.
//!
//! # Leaving
//!
//! A GPU object dropped while the process ends - from a thread-local's destructor, a `static`, a
//! thread nobody joined - calls into the driver while Windows is unloading it, and the driver can
//! hang there for good, taking every later attempt of the machine to make a device with it. So
//! nothing that owns a GPU object lives in a thread-local, a `static` or a detached thread (egui
//! keeps its immediate-viewport renderer in a thread-local: that one holds the painter only
//! weakly), and `App::exiting` takes everything down on the UI thread while the event loop is
//! still alive, in this order:
//!
//! 1. the shell writes what it must (session, settings);
//! 2. the render threads stop: every [`GpuOwners`] hook runs, newest first - a renderer's
//!    `shutdown` joins its thread, which drops its frame resources and targets on itself;
//! 3. the shell is dropped: the documents with their renderers, textures and passes;
//! 4. the device handle other threads were given ([`WindowLink::gpu`]) goes;
//! 5. the panels' own windows: their surfaces, then the windows;
//! 6. the painter - the root surface, egui's renderer and with them the device and the queue -,
//!    then the root window.
//!
//! Only then does the loop end. A panic on the way out does not break the order: each step runs
//! under `catch_unwind`, and while the teardown runs a panic of the UI thread only logs (the
//! crash hook would end the process half way otherwise).

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{ImmediateViewport, ViewportId, ViewportIdSet, ViewportInfo};
use egui_wgpu::winit::Painter;
use egui_wgpu::{
    RenderState, RendererOptions, SurfaceConfig, WgpuConfiguration, WgpuSetupCreateNew, WgpuSetupExisting,
};
use egui_winit::winit;
use futures::FutureExt;
use gz_editor::{Editor, FrameStamp, SmolStr};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowId};

/// What wakes the event loop from another thread.
#[derive(Clone, Copy, Debug)]
pub enum UserEvent {
    /// A frame now: something it shows changed.
    Wake,
    /// egui asked for a frame later (`request_repaint_after`): the loop learns when, and draws
    /// nothing for it now.
    Later,
}

/// The start-up profile the window carries on from `main`: its stages up to the first frame on
/// the screen. Stages may be named from the thread that makes the device.
pub trait StartProfile: Send {
    /// Names the stage that begins now.
    fn stage(&mut self, name: &str);
    /// Notes how long after the process was created `what` happened ("window shown").
    fn milestone(&mut self, what: &str);
    /// Ends the profile and writes it to the log.
    fn finish(&mut self);
}

type Profile = Arc<Mutex<Option<Box<dyn StartProfile>>>>;

fn profile_stage(p: &Profile, name: &str) {
    if let Some(x) = p.lock().as_mut() {
        x.stage(name);
    }
}

/// The device being made off the UI thread, with the window it is for.
struct Pending {
    window: Arc<Window>,
    ctx: egui::Context,
    rx: std::sync::mpsc::Receiver<Result<WgpuSetupExisting, String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The graphics instance, the adapter and the device, made on the device thread (the window's
/// surface cannot be made there: its handle is the UI thread's). One backend is asked for -
/// Direct3D 12 - and Vulkan only when that fails: probing every backend costs a start more than
/// the device itself. The stages go into the start-up profile.
fn make_device(
    rt: &tokio::runtime::Handle,
    opts: &WindowOptions,
    display: winit::event_loop::OwnedDisplayHandle,
    log: &gz_log::Log,
    profile: &Profile,
) -> Result<WgpuSetupExisting, String> {
    use egui_wgpu::wgpu;
    let mut last = String::new();
    for backend in [wgpu::Backends::DX12, wgpu::Backends::VULKAN] {
        profile_stage(profile, "Making the graphics instance");
        let mut setup = WgpuSetupCreateNew::from_display_handle(display.clone());
        setup.instance_descriptor.backends = backend;
        let base = setup.device_descriptor.clone();
        let power = setup.power_preference;
        let instance = rt.block_on(egui_wgpu::WgpuSetup::CreateNew(setup).new_instance());
        profile_stage(profile, "Picking the adapter");
        // View > Render > GPU: the adapter of that name; the system's choice otherwise
        let adapter = if opts.gpu_adapter.is_empty() {
            None
        } else {
            let adapters = rt.block_on(instance.enumerate_adapters(backend));
            let found = crate::gpu::pick_adapter(&adapters, &opts.gpu_adapter, None).cloned();
            if found.is_none() {
                log.warn(format!(
                    "the GPU '{}' is not there: the device is made on another one",
                    opts.gpu_adapter
                ));
            }
            found
        };
        let adapter = match adapter {
            Some(a) => Ok(a),
            None => rt.block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })),
        };
        let adapter = match adapter {
            Ok(a) => a,
            Err(e) => {
                last = e.to_string();
                log.warn(format!("no graphics adapter with {backend:?}: {last}"));
                continue;
            }
        };
        profile_stage(profile, "Making the device");
        // BC textures go to the GPU as the files hold them when the adapter samples them
        let mut desc = base(&adapter);
        desc.required_features |= adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
        match rt.block_on(adapter.request_device(&desc)) {
            Ok((device, queue)) => return Ok(WgpuSetupExisting { instance, adapter, device, queue }),
            Err(e) => {
                last = e.to_string();
                log.warn(format!("the graphics device could not be made with {backend:?}: {last}"));
            }
        }
    }
    Err(format!("the graphics device could not be created: {last}"))
}

/// The window's handle for the few Win32 calls `gz-win` makes on it.
fn hwnd_of(window: &Window) -> Option<isize> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// What one frame tells the window.
#[derive(Clone, Debug, Default)]
pub struct FrameReport {
    /// The document id of the tab the frame drew.
    pub active_tab: SmolStr,
    /// Its longest phase, for a stall: `ui pass (level)`, `command: gz_app::cmds::ui_tab`.
    pub during: String,
    /// The first frame of the start: making the font atlas and the first pipelines is no stall.
    pub first: bool,
}

/// The thing that draws: the shell with its tabs.
pub trait Shell: 'static {
    /// Called once, when the GPU and the egui context exist, before the first frame.
    fn init(&mut self, ctx: &egui::Context, gpu: &RenderState);

    /// Draws one frame.
    fn frame(&mut self, ui: &mut egui::Ui) -> FrameReport;

    /// The window was asked to close. `true` closes it now; `false` means the shell took the
    /// request (a question about unsaved work is up) and will ask the editor to quit itself.
    fn close_requested(&mut self) -> bool;

    /// The window is going away: what must be written is written now.
    fn exiting(&mut self) {}
}

#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: String,
    /// The first size, in points.
    pub size: [f32; 2],
    pub min_size: [f32; 2],
    pub maximized: bool,
    /// The system's title bar and border. Off when the shell draws its own caption.
    pub decorations: bool,
    /// Where the window's outer top-left corner goes, in points; `None` lets the system pick.
    pub position: Option<[f32; 2]>,
    /// The window's icon: square straight RGBA and its side.
    pub icon: Option<(Vec<u8>, u32)>,
    /// The adapter to make the device on, by name; empty is the system's default.
    pub gpu_adapter: String,
    /// egui's viewports become native windows (panels outside the window).
    pub floating: bool,
    /// The theme's background: what the window shows before its device exists.
    pub background: [u8; 3],
    /// `-minimized`: the window starts minimised, as it does for a launcher that asks for it
    /// (`gz_win::started_minimized`).
    pub start_minimized: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "Grizzly Editor".to_owned(),
            size: [1440.0, 900.0],
            min_size: [960.0, 600.0],
            maximized: false,
            decorations: true,
            position: None,
            icon: None,
            gpu_adapter: String::new(),
            floating: false,
            background: [0x20, 0x20, 0x25],
            start_minimized: false,
        }
    }
}

/// How the window's surfaces present: waiting for the monitor's refresh (Preferences > General >
/// VSync, on by default) or not, with the low-latency frame queue either way.
fn surface_config(editor: &Editor) -> SurfaceConfig {
    let vsync = crate::settings::EditorSettings::of(editor).pref("vsync").is_none_or(|v| v != 0.0);
    SurfaceConfig {
        present_mode: if vsync {
            egui_wgpu::wgpu::PresentMode::AutoVsync
        } else {
            egui_wgpu::wgpu::PresentMode::AutoNoVsync
        },
        ..SurfaceConfig::LOW_LATENCY
    }
}

/// The gate a renderer of this editor is made with: the window's, or one of its own for an editor
/// without a window (nothing configures a surface there).
#[must_use]
pub fn gpu_gate(editor: &Editor) -> gz_task::Gate {
    editor.services.get::<WindowLink>().map(|l| l.gpu_gate()).unwrap_or_default()
}

/// How other threads reach the window: pictures of it, and the GPU it draws with.
#[derive(Default)]
pub struct WindowLink {
    captures: Mutex<Vec<egui::ScreenshotCallback>>,
    gpu: Mutex<Option<RenderState>>,
    /// Closed while a surface is configured; every thread that submits to the device passes it.
    gate: gz_task::Gate,
    caption: Arc<gz_win::CaptionHitTest>,
    /// The titles of the windows of the editor's own the picture does not hold.
    outside: Mutex<Vec<String>>,
}

impl WindowLink {
    /// Asks for a picture of the next frame. `done` is called with it on a GPU callback thread.
    pub fn capture(&self, done: impl FnOnce(Arc<egui::ColorImage>) + Send + 'static) {
        self.captures.lock().push(egui::ScreenshotCallback::new(done));
    }

    /// The device and queue the window draws with, once it exists. Shared with whatever renders
    /// off the UI thread.
    #[must_use]
    pub fn gpu(&self) -> Option<RenderState> {
        self.gpu.lock().clone()
    }

    /// The gate of the window's device: a renderer on that device is made with it, so that no
    /// submission of its thread crosses a surface's configuration (the graphics runtime refuses
    /// one that does, and the window would keep a surface of the old size).
    #[must_use]
    pub fn gpu_gate(&self) -> gz_task::Gate {
        self.gate.clone()
    }

    /// What the window's hit test answers from: the shell publishes the caption every frame.
    #[must_use]
    pub fn caption(&self) -> &Arc<gz_win::CaptionHitTest> {
        &self.caption
    }

    /// The visible windows of the editor besides the main one (a panel dragged out), by title,
    /// as the last frame left them.
    #[must_use]
    pub fn outside(&self) -> Vec<String> {
        self.outside.lock().clone()
    }
}

/// What owns a GPU object outside the window - a renderer a service keeps, a cache of
/// textures: each registers how it lets go, and the window runs those hooks, newest first, on
/// the UI thread while the event loop is still alive (the module's "Leaving").
#[derive(Default)]
pub struct GpuOwners {
    hooks: Mutex<Vec<ExitHook>>,
}

/// Who registered it, and how it lets go.
type ExitHook = (&'static str, Box<dyn FnOnce() + Send>);

impl GpuOwners {
    pub fn of(editor: &Editor) -> Arc<GpuOwners> {
        editor.services.get_or_init(GpuOwners::default)
    }

    /// `release` lets the owner's GPU objects go (stops and joins a render thread). It runs once.
    pub fn on_exit(&self, who: &'static str, release: impl FnOnce() + Send + 'static) {
        self.hooks.lock().push((who, Box::new(release)));
    }

    fn take(&self) -> Vec<ExitHook> {
        std::mem::take(&mut *self.hooks.lock())
    }
}

/// The shell put in place of the real one while the window goes: it keeps nothing.
struct Gone;

impl Shell for Gone {
    fn init(&mut self, _: &egui::Context, _: &RenderState) {}

    fn frame(&mut self, _: &mut egui::Ui) -> FrameReport {
        FrameReport::default()
    }

    fn close_requested(&mut self) -> bool {
        true
    }
}

/// A native window of an egui viewport other than the root.
struct Extra {
    window: Arc<Window>,
    input: egui_winit::State,
    info: ViewportInfo,
}

/// What the root frame and the immediate viewports' passes share on the UI thread.
struct Shared {
    painter: Painter,
    extra: FxHashMap<ViewportId, Extra>,
    /// The textures of passes no window painted: the next paint applies them first.
    textures: egui::TexturesDelta,
    clear: [f32; 4],
}

struct Live {
    window: Arc<Window>,
    ctx: egui::Context,
    input: egui_winit::State,
    shared: Rc<RefCell<Shared>>,
    info: ViewportInfo,
    /// The last size that was not zero: what a minimised window is pictured at.
    size_px: [u32; 2],
}

/// The thread that polls the device so a captured frame's read-back is mapped. It is made with the
/// window and never in a frame: making a thread is a system call, and on a busy machine the frame
/// that asked for one stood in it for a tenth of a second.
struct Polls {
    ask: Option<std::sync::mpsc::Sender<egui_wgpu::wgpu::Device>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Polls {
    fn new() -> Self {
        let (ask, asked) = std::sync::mpsc::channel::<egui_wgpu::wgpu::Device>();
        let thread = std::thread::Builder::new()
            .name("gz-readback".into())
            .spawn(move || {
                while let Ok(device) = asked.recv() {
                    let _ = device.poll(egui_wgpu::wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_secs(2)),
                    });
                }
            })
            .ok();
        Self { ask: Some(ask), thread }
    }

    /// A captured frame was submitted: the poll that hands its picture over is the thread's.
    fn poll(&self, device: egui_wgpu::wgpu::Device) {
        if let Some(ask) = &self.ask {
            let _ = ask.send(device);
        }
    }

    /// Ends the thread once the poll it is in is over (2 s at most).
    fn end(&mut self) {
        self.ask = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct App {
    editor: Arc<Editor>,
    link: Arc<WindowLink>,
    opts: WindowOptions,
    shell: Box<dyn Shell>,
    live: Option<Live>,
    /// The window is up and its device is being made.
    pending: Option<Pending>,
    profile: Profile,
    /// The screenshots' read-backs, polled off the UI thread; ended before the device goes.
    polls: Polls,
    /// A frame is due.
    dirty: bool,
    /// Frames left whose paints may configure a surface again after Preferences > General >
    /// VSync changed: every surface does it at its own next paint, a panel's window included.
    reconfiguring: u8,
    /// What made the next frame due, for `perf.frames`' `ui_frame_causes`: a ping of the
    /// editor's wake from another thread, an event of the window, egui's own request.
    woken: bool,
    /// A wake is on its way to the loop and was not taken yet: one message stands for every ping
    /// since. A job that pings a thousand times a second would otherwise fill the loop's queue,
    /// and the loop empties its queue before it draws - the frame the pings ask for came a
    /// second late.
    wake_pending: Arc<AtomicBool>,
    /// The same for the message that says a later frame was asked for.
    later_pending: Arc<AtomicBool>,
    input: bool,
    asked: bool,
    /// Tells the loop from any thread that egui asked for a later frame.
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    /// When egui asked to be drawn again.
    repaint_at: Arc<Mutex<Option<Instant>>>,
    frame: u64,
    failed: Option<String>,
    /// The teardown runs: a panic of the UI thread only logs meanwhile.
    leaving: Arc<AtomicBool>,
}

/// One pass of an immediate viewport (a panel's own window): its window's input, the pass, the
/// paint. Without its window yet, the pass still runs - egui wants it - and only its textures
/// are kept for the next paint.
fn render_immediate(ctx: &egui::Context, shared: &Weak<RefCell<Shared>>, vp: ImmediateViewport<'_>) {
    let ImmediateViewport { ids, builder, mut viewport_ui_cb } = vp;
    // the window is going: the pass runs, nothing is painted
    let Some(shared) = shared.upgrade() else {
        let raw = egui::RawInput { viewport_id: ids.this, ..Default::default() };
        let _ = ctx.run_ui(raw, |ui| viewport_ui_cb(ui));
        return;
    };
    let shared = &shared;
    let raw = {
        let Ok(mut sh) = shared.try_borrow_mut() else { return };
        match sh.extra.get_mut(&ids.this) {
            Some(x) => {
                egui_winit::update_viewport_info(&mut x.info, ctx, &x.window, false);
                let mut raw = x.input.take_egui_input(&x.window);
                raw.viewports.insert(ids.this, x.info.clone());
                x.info.events.clear();
                raw
            }
            None => {
                let mut raw = egui::RawInput { viewport_id: ids.this, ..Default::default() };
                raw.screen_rect = builder.inner_size.map(|s| egui::Rect::from_min_size(egui::Pos2::ZERO, s));
                raw.viewports.insert(ids.this, ViewportInfo::default());
                raw
            }
        }
    };
    let output = ctx.run_ui(raw, |ui| viewport_ui_cb(ui));
    let Ok(mut sh) = shared.try_borrow_mut() else { return };
    let Shared { painter, extra, textures, clear } = &mut *sh;
    match extra.get_mut(&ids.this) {
        Some(x) => {
            x.input.handle_platform_output(&x.window, output.platform_output);
            let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
            let mut delta = std::mem::take(textures);
            delta.append(output.textures_delta);
            painter.paint_and_update_textures(
                ids.this,
                output.pixels_per_point,
                *clear,
                &primitives,
                &mut delta,
                Vec::new(),
                &x.window,
            );
        }
        None => textures.append(output.textures_delta),
    }
}

impl App {
    /// The window, on the screen at once in the theme's background, and the thread that makes its
    /// graphics device: the event loop goes on meanwhile and the first frame follows the device.
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<Pending, String> {
        profile_stage(&self.profile, "Creating the window");
        let start_minimized = self.opts.start_minimized || gz_win::started_minimized();
        let mut attrs = Window::default_attributes()
            .with_title(&self.opts.title)
            .with_inner_size(LogicalSize::new(self.opts.size[0], self.opts.size[1]))
            .with_min_inner_size(LogicalSize::new(self.opts.min_size[0], self.opts.min_size[1]))
            .with_decorations(self.opts.decorations)
            .with_maximized(self.opts.maximized && !start_minimized)
            // shown once it is painted in the background colour below: no white flash
            .with_visible(false)
            // QA finds the window by its class
            .with_class_name("Grizzly Editor");
        if let Some([x, y]) = self.opts.position {
            attrs = attrs.with_position(LogicalPosition::new(x, y));
        }
        if let Some((rgba, side)) = self.opts.icon.clone()
            && let Ok(icon) = winit::window::Icon::from_rgba(rgba, side, side)
        {
            attrs = attrs.with_window_icon(Some(icon));
        }
        let window = Arc::new(
            event_loop.create_window(attrs).map_err(|e| format!("the window could not be created: {e}"))?,
        );
        // an undecorated window answers "client" everywhere: the caption's own hit test
        if !self.opts.decorations {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::Win32(h) = handle.as_raw()
                && let Err(e) = gz_win::install_caption_hit_test(h.hwnd.get(), self.link.caption.clone())
            {
                self.editor.log.warn(format!("the caption hit test could not be installed: {e}"));
            }
        }

        // on the screen now, in the theme's background, long before the device exists
        profile_stage(&self.profile, "Showing the window");
        // a minimised start (a QA harness) stays out of sight, as it always did: an iconic window
        // reports its taskbar button's size, and nothing would lay out
        if !start_minimized {
            window.set_visible(true);
            if let Some(h) = hwnd_of(&window) {
                gz_win::fill_client(h, self.opts.background);
            }
        }
        // the profile says which of the two happened: a start-up budget is about a shown window
        if let Some(p) = self.profile.lock().as_mut() {
            p.milestone(if start_minimized { "window minimised" } else { "window shown" });
        }

        let ctx = egui::Context::default();
        let repaint_at = self.repaint_at.clone();
        let wake = self.editor.exec.wake().clone();
        let proxy = self.proxy.clone();
        let later_pending = self.later_pending.clone();
        ctx.set_request_repaint_callback(move |info| {
            let when = Instant::now() + info.delay;
            let sooner = {
                // the soonest frame asked for wins until a frame answers it
                let mut at = repaint_at.lock();
                let sooner = at.is_none_or(|a| when < a);
                if sooner {
                    *at = Some(when);
                }
                sooner
            };
            // A frame asked for later is not drawn now: a 100 ms request answered at once (the
            // status bar's job chip) drew every frame the window could while a job ran
            if info.delay.is_zero() {
                wake.ping();
            } else if sooner && !later_pending.swap(true, Ordering::AcqRel) {
                // the loop only has to learn of a time sooner than the one it waits for, and one
                // message says so: a thread that asks a thousand times a second must not fill the
                // loop's queue
                let _ = proxy.send_event(UserEvent::Later);
            }
        });

        let (tx, rx) = std::sync::mpsc::channel();
        let (opts, log, profile) = (self.opts.clone(), self.editor.log.clone(), self.profile.clone());
        let display = event_loop.owned_display_handle();
        let rt = self.editor.exec.handle().clone();
        let wake = self.editor.exec.wake().clone();
        let thread = std::thread::Builder::new()
            .name("grizzly-device".to_owned())
            .spawn(move || {
                let made = make_device(&rt, &opts, display, &log, &profile);
                let _ = tx.send(made);
                wake.ping();
            })
            .map_err(|e| format!("the thread that makes the graphics device did not start: {e}"))?;
        Ok(Pending { window, ctx, rx, thread: Some(thread) })
    }

    /// The second half of [`App::create`], on the UI thread once the device is there.
    fn finish_create(&mut self, pending: Pending, device: WgpuSetupExisting) -> Result<Live, String> {
        let Pending { window, ctx, .. } = pending;
        // the window's surface and egui's renderer on the device that is there: nothing in it
        // waits on the GPU, and the one block_on of the UI thread is this, before the first frame
        profile_stage(&self.profile, "Making the surface");
        let config = WgpuConfiguration {
            surface: surface_config(&self.editor),
            wgpu_setup: egui_wgpu::WgpuSetup::Existing(device),
            ..Default::default()
        };
        let rt = self.editor.exec.handle().clone();
        let (c, w) = (ctx.clone(), window.clone());
        let painter = rt
            .block_on(async move {
                let mut painter = Painter::new(c, config, false, RendererOptions::default()).await;
                painter.set_window(ViewportId::ROOT, Some(w)).await.map(|()| painter)
            })
            .map_err(|e| format!("the graphics device could not be created: {e}"))?;
        let gpu =
            painter.render_state().ok_or_else(|| "the graphics device could not be created".to_owned())?;
        // wgpu's default handler panics: a device error is the log's, not the process's end
        let log = self.editor.log.clone();
        gpu.device.on_uncaptured_error(Arc::new(move |e: egui_wgpu::wgpu::Error| {
            log.error(format!("the graphics device reported an error: {e}"))
        }));
        crate::gpu::Gpu::of(&self.editor).set_adapters(crate::gpu::Adapters {
            names: {
                let mut names: Vec<String> = Vec::new();
                for a in &gpu.available_adapters {
                    let n = a.get_info().name;
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                names
            },
            active: gpu.adapter.get_info().name,
        });

        let input = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            painter.max_texture_side(),
        );
        self.shell.init(&ctx, &gpu);
        *self.link.gpu.lock() = Some(gpu);

        let clear = ctx.global_style().visuals.panel_fill.to_normalized_gamma_f32();
        let shared = Rc::new(RefCell::new(Shared {
            painter,
            extra: FxHashMap::default(),
            textures: egui::TexturesDelta::default(),
            clear,
        }));
        if self.opts.floating {
            ctx.set_embed_viewports(false);
            // egui keeps this renderer in a thread-local: it holds the painter weakly, so the
            // painter never goes with the thread's end
            let s = Rc::downgrade(&shared);
            egui::Context::set_immediate_viewport_renderer(move |ctx, vp| render_immediate(ctx, &s, vp));
        }

        let size = window.inner_size();
        profile_stage(&self.profile, "The first frame");
        Ok(Live {
            window,
            ctx,
            input,
            shared,
            info: ViewportInfo::default(),
            size_px: [size.width.max(1), size.height.max(1)],
        })
    }

    /// The device, once its thread is done: the window's state goes on from there.
    fn poll_device(&mut self, event_loop: &ActiveEventLoop) {
        let Some(p) = self.pending.as_mut() else { return };
        let made = match p.rx.try_recv() {
            Ok(m) => m,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("the graphics device thread ended".to_owned())
            }
        };
        let Some(mut pending) = self.pending.take() else { return };
        if let Some(t) = pending.thread.take() {
            let _ = t.join();
        }
        match made.and_then(|device| self.finish_create(pending, device)) {
            Ok(live) => {
                self.live = Some(live);
                self.dirty = true;
            }
            Err(e) => {
                self.failed = Some(e);
                event_loop.exit();
            }
        }
    }

    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = self.live.as_mut() else { return };
        let began = Instant::now();
        // what the UI thread itself spent on the processor: a stall with little of it was a wait
        let cpu_began = gz_win::thread_cpu_seconds();
        let stalls = crate::stalls::Stalls::of(&self.editor);
        stalls.pass_begins();
        // before anything is read: a request held for frames counts only frames begun after it
        self.editor.frames.begin();
        self.dirty = false;
        *self.repaint_at.lock() = None;

        // Preferences > General > VSync: every surface is configured again at its next paint, which
        // no other thread's submission may cross - this frame's paints and the next two frames'
        // hold the gate (a panel's window paints during the pass), and let it go before the
        // panels' windows are synced, which close it themselves
        let surface = surface_config(&self.editor);
        {
            let mut sh = live.shared.borrow_mut();
            if sh.painter.surface_config() != surface {
                sh.painter.set_surface_config(surface);
                self.reconfiguring = 3;
            }
        }
        let alone = (self.reconfiguring > 0).then(|| self.link.gate.close());
        self.reconfiguring = self.reconfiguring.saturating_sub(1);
        let woken = std::mem::take(&mut self.woken);
        let input = std::mem::take(&mut self.input);
        let asked = std::mem::take(&mut self.asked);
        let mut raw = live.input.take_egui_input(&live.window);
        // A minimized editor is drawn the way it looks restored: an undecorated window reports the
        // size of its taskbar button while it is iconic
        if live.window.is_minimized() == Some(true) {
            let ppp = live.window.scale_factor() as f32 * live.ctx.zoom_factor();
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(live.size_px[0] as f32, live.size_px[1] as f32) / ppp.max(0.01),
            ));
        }
        egui_winit::update_viewport_info(&mut live.info, &live.ctx, &live.window, self.frame == 0);
        raw.viewports.insert(ViewportId::ROOT, live.info.clone());
        if let Ok(sh) = live.shared.try_borrow() {
            for (id, x) in &sh.extra {
                raw.viewports.insert(*id, x.info.clone());
            }
        }
        live.shared.borrow_mut().clear = live.ctx.global_style().visuals.panel_fill.to_normalized_gamma_f32();

        let shell = &mut self.shell;
        let mut report = FrameReport::default();
        let mut causes = Vec::new();
        let pass = Instant::now();
        let mut output = live.ctx.run_ui(raw, |ui| {
            causes = ui.ctx().repaint_causes();
            report = shell.frame(ui);
        });
        let pass_ms = pass.elapsed().as_secs_f32() * 1000.0;
        let pinger = self.editor.exec.wake().take_last();
        let cause = frame_cause(woken.then_some(pinger), input, asked, &causes);

        live.input.handle_platform_output(&live.window, output.platform_output);

        let mut actions = Vec::new();
        if let Some(root) = output.viewport_output.remove(&ViewportId::ROOT) {
            egui_winit::process_viewport_commands(
                &live.ctx,
                &mut live.info,
                root.commands,
                &live.window,
                &mut actions,
            );
        }
        let mut captures: Vec<egui::ScreenshotCallback> = self.link.captures.lock().drain(..).collect();
        for action in actions {
            if let egui_winit::ActionRequested::Screenshot(callback) = action {
                captures.push(callback);
            }
        }
        let capturing = !captures.is_empty();

        let primitives = live.ctx.tessellate(output.shapes, output.pixels_per_point);
        let clear = live.ctx.global_style().visuals.panel_fill.to_normalized_gamma_f32();
        // the waits for the swap chain's image, the submission and the present
        let waited = {
            let mut sh = live.shared.borrow_mut();
            let mut delta = std::mem::take(&mut sh.textures);
            delta.append(std::mem::take(&mut output.textures_delta));
            let waited = sh.painter.paint_and_update_textures(
                ViewportId::ROOT,
                output.pixels_per_point,
                clear,
                &primitives,
                &mut delta,
                captures,
                &live.window,
            );
            if capturing && let Some(gpu) = sh.painter.render_state() {
                // the picture is handed over when the read-back buffer is mapped, which happens
                // on a poll; no later frame may come to do it, and the UI thread waits on no GPU
                // read-back: a worker polls
                self.polls.poll(gpu.device.clone());
            }
            waited
        };

        drop(alone);
        // the viewports of the panels outside: windows made for the new, commands for the
        // others, gone for those no pass asked for
        if self.opts.floating {
            Self::sync_extra(&self.editor, live, event_loop, output.viewport_output);
            let outside: Vec<String> = live
                .shared
                .try_borrow()
                .map(|sh| {
                    sh.extra
                        .values()
                        .filter(|x| x.window.is_visible() != Some(false))
                        .map(|x| x.window.title())
                        .collect()
                })
                .unwrap_or_default();
            *self.link.outside.lock() = outside;
        }

        self.frame += 1;
        // the first frame is on the screen: the start-up profile ends here
        if self.frame == 1
            && let Some(mut p) = self.profile.lock().take()
        {
            p.milestone("first frame");
            p.finish();
        }
        let total = began.elapsed().as_secs_f32() * 1000.0;
        let wait_ms = (waited * 1000.0).min(total);
        // the frame's own work held the UI thread: a stall, unless it is the start's first
        let work_ms = total - wait_ms;
        if !report.first && self.frame > 1 {
            let during = if work_ms - pass_ms > pass_ms {
                format!("painting the frame ({:.0} ms; the UI pass {pass_ms:.0} ms)", work_ms - pass_ms)
            } else {
                report.during.clone()
            };
            if work_ms > crate::stalls::STALL_MS {
                let cpu_ms = (gz_win::thread_cpu_seconds() - cpu_began) * 1000.0;
                let mut during = format!("{during}; {cpu_ms:.0} ms of it on the processor");
                // GRIZZLY_DEBUG_STALLS: where the thread stood when the pass had run for the limit
                if let Some(stack) = stalls.pass_ends() {
                    during.push_str("; it stood in ");
                    during.push_str(&stack.join(" <- "));
                }
                stalls.note(&self.editor, work_ms, &report.active_tab, &during);
            }
        }
        let _ = stalls.pass_ends();
        self.editor.frames.publish(FrameStamp {
            frame: self.frame,
            active_tab: report.active_tab,
            scale: output.pixels_per_point,
            width: live.size_px[0],
            height: live.size_px[1],
            work_ms: total - wait_ms,
            wait_ms,
            cause,
        });
    }

    fn sync_extra(
        editor: &Editor,
        live: &mut Live,
        event_loop: &ActiveEventLoop,
        outputs: std::collections::BTreeMap<ViewportId, egui::ViewportOutput>,
    ) {
        let Ok(mut sh) = live.shared.try_borrow_mut() else { return };
        let mut alive = ViewportIdSet::default();
        alive.insert(ViewportId::ROOT);
        for (id, vo) in outputs {
            if id == ViewportId::ROOT {
                continue;
            }
            alive.insert(id);
            if let Some(x) = sh.extra.get_mut(&id) {
                let mut actions = Vec::new();
                egui_winit::process_viewport_commands(
                    &live.ctx,
                    &mut x.info,
                    vo.commands,
                    &x.window,
                    &mut actions,
                );
                continue;
            }
            let window = match egui_winit::create_window(&live.ctx, event_loop, &vo.builder) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    editor.log.warn(format!("a panel's window could not be made: {e}"));
                    continue;
                }
            };
            // the device exists: making a surface awaits nothing, so the future ends on its first
            // poll and the UI thread blocks on nothing
            let gate = gpu_gate(editor);
            let made = {
                let _alone = gate.close();
                sh.painter.set_window(id, Some(window.clone())).now_or_never()
            };
            match made {
                Some(Ok(())) => {}
                Some(Err(e)) => {
                    editor.log.warn(format!("a panel's window could not be drawn into: {e}"));
                    continue;
                }
                None => {
                    editor.log.warn("a panel's window could not be drawn into: its surface was not ready");
                    continue;
                }
            }
            let input = egui_winit::State::new(
                live.ctx.clone(),
                id,
                &window,
                Some(window.scale_factor() as f32),
                window.theme(),
                sh.painter.max_texture_side(),
            );
            let mut info = ViewportInfo::default();
            egui_winit::update_viewport_info(&mut info, &live.ctx, &window, true);
            sh.extra.insert(id, Extra { window, input, info });
            // its first paint is the next frame's
            live.ctx.request_repaint();
        }
        sh.extra.retain(|id, _| alive.contains(id));
        sh.painter.gc_viewports(&alive);
    }
}

/// What a frame was drawn for: egui's own request, named by the line that asked
/// (`repaint gz-app/src/chrome.rs:1476`), a ping of the editor's wake (a store, a job, a request
/// held for frames), an event of the window - in that order when several came together.
fn frame_cause(
    woken: Option<Option<&'static std::panic::Location<'static>>>,
    input: bool,
    asked: bool,
    causes: &[egui::RepaintCause],
) -> SmolStr {
    // the path below `crates/`, or the file's name
    let short = |file: &str, line: u32| {
        let file = file.replace('\\', "/");
        let short = match file.split_once("crates/") {
            Some((_, rest)) => rest,
            None => file.rsplit('/').next().unwrap_or(&file),
        };
        format!("{short}:{line}")
    };
    if asked && let Some(c) = causes.first() {
        return SmolStr::new(format!("repaint {}", short(c.file, c.line)));
    }
    match woken {
        Some(Some(at)) => SmolStr::new(format!("wake {}", short(at.file(), at.line()))),
        Some(None) => SmolStr::new_static("wake"),
        None if asked => SmolStr::new_static("repaint"),
        None if input => SmolStr::new_static("input"),
        None => SmolStr::new_static("other"),
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.dirty = true;
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() || self.pending.is_some() {
            return;
        }
        match self.create(event_loop) {
            Ok(pending) => self.pending = Some(pending),
            Err(e) => {
                self.failed = Some(e);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        // Leaving has to be asked for while a message is being handled: asked from
        // `about_to_wait`, the loop goes to sleep first and leaves at the next message.
        if self.editor.quit_token().is_cancelled() {
            event_loop.exit();
            return;
        }
        // `Later`: `about_to_wait` sets the loop's wait from the time egui asked for
        if matches!(event, UserEvent::Later) {
            self.later_pending.store(false, Ordering::Release);
        }
        if matches!(event, UserEvent::Wake) {
            // cleared before the frame is marked due: a ping from here on sends a message again
            self.wake_pending.store(false, Ordering::Release);
            self.dirty = true;
            self.woken = true;
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // a frame times itself; every other event is timed here for the stall monitor
        if matches!(event, WindowEvent::RedrawRequested) {
            self.handle_window_event(event_loop, window_id, event);
            return;
        }
        let kind = event_name(&event);
        let began = Instant::now();
        let cpu_began = gz_win::thread_cpu_seconds();
        let stalls = crate::stalls::Stalls::of(&self.editor);
        stalls.pass_begins();
        self.handle_window_event(event_loop, window_id, event);
        let ms = began.elapsed().as_secs_f32() * 1000.0;
        if self.frame > 1 {
            let tab = self.editor.frames.current().active_tab;
            if ms > crate::stalls::STALL_MS {
                let cpu_ms = (gz_win::thread_cpu_seconds() - cpu_began) * 1000.0;
                let mut during = format!("event: {kind}; {cpu_ms:.0} ms of it on the processor");
                if let Some(stack) = stalls.pass_ends() {
                    during.push_str("; it stood in ");
                    during.push_str(&stack.join(" <- "));
                }
                stalls.note(&self.editor, ms, &tab, &during);
            }
        }
        let _ = stalls.pass_ends();
    }

    /// The mouse's own motion (raw input), as egui's `MouseMoved`: what the viewport's camera turns
    /// by while it holds the cursor in place, where the cursor's place says nothing.
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && let Some(live) = self.live.as_mut()
            && live.input.on_mouse_motion(delta)
        {
            self.dirty = true;
            self.input = true;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.quit_token().is_cancelled() {
            event_loop.exit();
            return;
        }
        self.poll_device(event_loop);
        let now = Instant::now();
        let due = *self.repaint_at.lock();
        if due.is_some_and(|at| at <= now) {
            self.dirty = true;
            self.asked = true;
        }
        if self.dirty {
            self.run_frame(event_loop);
        }
        match *self.repaint_at.lock() {
            Some(at) if at > Instant::now() => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            Some(_) => event_loop.set_control_flow(ControlFlow::Poll),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.leaving.store(true, Ordering::SeqCst);
        let log = self.editor.log.clone();
        let step = |what: &str, f: &mut dyn FnMut()| {
            if catch_unwind(AssertUnwindSafe(f)).is_err() {
                log.error(format!("leaving: {what} failed; the rest goes on"));
            }
        };
        // 1. what must be written
        step("the shell's last writes", &mut || self.shell.exiting());
        // 2. the render threads, newest owner first
        for (who, release) in GpuOwners::of(&self.editor).take().into_iter().rev() {
            let mut release = Some(release);
            step(who, &mut || {
                if let Some(r) = release.take() {
                    r();
                }
            });
        }
        // 3. the documents and everything they draw with
        step("the shell", &mut || drop(std::mem::replace(&mut self.shell, Box::new(Gone))));
        // 4. the device handle given out; the read-backs still polling end first (2 s at most),
        // and a device still being made is let go on this thread
        self.polls.end();
        if let Some(mut p) = self.pending.take() {
            if let Some(t) = p.thread.take() {
                let _ = t.join();
            }
            step("the device being made", &mut || drop(p.rx.try_recv()));
        }
        step("the shared device", &mut || *self.link.gpu.lock() = None);
        if let Some(live) = self.live.take() {
            let Live { window, ctx, input, shared, .. } = live;
            // 5. the panels' own windows: their surfaces first
            step("the panels' windows", &mut || {
                if let Ok(mut sh) = shared.try_borrow_mut() {
                    let ids: Vec<ViewportId> = sh.extra.keys().copied().collect();
                    for id in ids {
                        // clearing a surface awaits nothing
                        let _ = sh.painter.set_window(id, None).now_or_never();
                    }
                    sh.extra.clear();
                    let mut root = ViewportIdSet::default();
                    root.insert(ViewportId::ROOT);
                    sh.painter.gc_viewports(&root);
                }
            });
            if self.opts.floating {
                step("the panels' renderer", &mut || {
                    egui::Context::set_immediate_viewport_renderer(|ctx, vp| {
                        let ImmediateViewport { ids, mut viewport_ui_cb, .. } = vp;
                        let raw = egui::RawInput { viewport_id: ids.this, ..Default::default() };
                        let _ = ctx.run_ui(raw, |ui| viewport_ui_cb(ui));
                    });
                });
            }
            // 6. the painter (root surface, renderer, device and queue), then the window
            let mut shared = Some(shared);
            step("the painter", &mut || drop(shared.take()));
            let mut rest = Some((input, ctx));
            step("the input", &mut || drop(rest.take()));
            let mut window = Some(window);
            step("the window", &mut || drop(window.take()));
        }
        self.leaving.store(false, Ordering::SeqCst);
    }
}

impl App {
    fn handle_window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // the device is still being made: the window keeps the theme's background
        if let Some(p) = &self.pending {
            match event {
                WindowEvent::RedrawRequested | WindowEvent::Resized(_) => {
                    if let Some(h) = hwnd_of(&p.window) {
                        gz_win::fill_client(h, self.opts.background);
                    }
                }
                WindowEvent::CloseRequested => event_loop.exit(),
                _ => {}
            }
            return;
        }
        let Some(live) = self.live.as_mut() else { return };
        if window_id != live.window.id() {
            // a panel's own window: its input, its size, its close (the panel docks again)
            if let Ok(mut sh) = live.shared.try_borrow_mut() {
                let Shared { painter, extra, .. } = &mut *sh;
                if let Some((id, x)) = extra.iter_mut().find(|(_, x)| x.window.id() == window_id) {
                    let _ = x.input.on_window_event(&x.window, &event);
                    match &event {
                        WindowEvent::Resized(size) => {
                            if let (Some(w), Some(h)) = (
                                std::num::NonZeroU32::new(size.width),
                                std::num::NonZeroU32::new(size.height),
                            ) {
                                let _alone = self.link.gate.close();
                                painter.on_window_resized(*id, w, h);
                            }
                        }
                        WindowEvent::CloseRequested => x.info.events.push(egui::ViewportEvent::Close),
                        _ => {}
                    }
                }
            }
            self.dirty = true;
            return;
        }
        let response = live.input.on_window_event(&live.window, &event);
        if response.repaint {
            self.dirty = true;
            self.input = true;
        }
        match event {
            WindowEvent::Resized(size) => {
                // iconic, the window keeps the size it is pictured at
                if live.window.is_minimized() == Some(true) {
                    self.dirty = true;
                    return;
                }
                if let (Some(w), Some(h)) =
                    (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height))
                {
                    live.size_px = [size.width, size.height];
                    let _alone = self.link.gate.close();
                    live.shared.borrow_mut().painter.on_window_resized(ViewportId::ROOT, w, h);
                }
                self.dirty = true;
            }
            WindowEvent::RedrawRequested => self.run_frame(event_loop),
            WindowEvent::CloseRequested => {
                if self.shell.close_requested() {
                    event_loop.exit();
                } else {
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }
}

/// What a window event is called in a stall: its variant's name, `resized`, `keyboard input`.
fn event_name(event: &WindowEvent) -> String {
    let debug = format!("{event:?}");
    let head = debug.split(['(', ' ', '{']).next().unwrap_or("event");
    let mut out = String::with_capacity(head.len() + 4);
    for (i, c) in head.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// While the window goes a panic of the UI thread only logs, so every step of the teardown runs;
/// any other panic goes to the hook that was there.
fn install_leaving_guard(editor: &Editor, leaving: Arc<AtomicBool>) {
    let previous = std::panic::take_hook();
    let ui_thread = std::thread::current().id();
    let log = editor.log.clone();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == ui_thread && leaving.load(Ordering::SeqCst) {
            let at = info
                .location()
                .map_or_else(|| "unknown location".to_owned(), |l| format!("{}:{}", l.file(), l.line()));
            log.error(format!("internal error while leaving at {at}"));
            return;
        }
        previous(info);
    }));
}

/// Opens the window and runs the editor's frame loop on the calling thread - the main thread -
/// until the window closes or the editor is asked to quit.
pub fn run(
    editor: Arc<Editor>,
    opts: WindowOptions,
    shell: Box<dyn Shell>,
    profile: Option<Box<dyn StartProfile>>,
) -> Result<(), String> {
    // this thread draws and presents: it works in bursts and gets its core before the pools,
    // which load every core during a build
    let _ = gz_win::raise_this_thread_priority();
    // a window behind others or on another desktop must not slow the pools: Windows' power
    // throttling cost a level compile beside it 11 - 17 %
    if !gz_win::keep_full_speed() {
        editor.log.warn("the editor could not keep its full speed while its window is in the background");
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| format!("the event loop did not start: {e}"))?;
    let proxy = event_loop.create_proxy();
    let wake_proxy = proxy.clone();
    let wake_pending = Arc::new(AtomicBool::new(false));
    let pending = wake_pending.clone();
    editor.exec.wake().set(move || {
        // one message at a time: the loop clears the flag as it takes the message
        if !pending.swap(true, Ordering::AcqRel) {
            let _ = wake_proxy.send_event(UserEvent::Wake);
        }
    });
    let link = editor.services.get_or_init(WindowLink::default);
    let leaving = Arc::new(AtomicBool::new(false));
    install_leaving_guard(&editor, leaving.clone());
    let mut app = App {
        editor: editor.clone(),
        link,
        opts,
        shell,
        live: None,
        pending: None,
        profile: Arc::new(Mutex::new(profile)),
        polls: Polls::new(),
        dirty: true,
        reconfiguring: 0,
        woken: false,
        wake_pending,
        later_pending: Arc::new(AtomicBool::new(false)),
        input: false,
        asked: false,
        proxy,
        repaint_at: Arc::new(Mutex::new(None)),
        frame: 0,
        failed: None,
        leaving,
    };
    let ran = event_loop.run_app(&mut app);
    // nothing may ping a loop that is gone
    editor.exec.wake().set(|| {});
    if let Some(e) = app.failed {
        return Err(e);
    }
    ran.map_err(|e| format!("the event loop stopped: {e}"))
}
