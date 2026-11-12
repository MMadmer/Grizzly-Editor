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
use egui_wgpu::{RenderState, RendererOptions, SurfaceConfig, WgpuConfiguration, WgpuSetupCreateNew};
use egui_winit::winit;
use gz_editor::{Editor, FrameStamp, SmolStr};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowId};

/// What wakes the event loop from another thread.
#[derive(Clone, Copy, Debug)]
pub enum UserEvent {
    Wake,
}

/// What one frame tells the window.
#[derive(Clone, Debug, Default)]
pub struct FrameReport {
    /// The document id of the tab the frame drew.
    pub active_tab: SmolStr,
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
        }
    }
}

/// How other threads reach the window: pictures of it, and the GPU it draws with.
#[derive(Default)]
pub struct WindowLink {
    captures: Mutex<Vec<egui::ScreenshotCallback>>,
    gpu: Mutex<Option<RenderState>>,
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
    shown: bool,
}

struct App {
    editor: Arc<Editor>,
    link: Arc<WindowLink>,
    opts: WindowOptions,
    shell: Box<dyn Shell>,
    live: Option<Live>,
    /// A frame is due.
    dirty: bool,
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
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<Live, String> {
        let start_minimized = gz_win::started_minimized();
        let mut attrs = Window::default_attributes()
            .with_title(&self.opts.title)
            .with_inner_size(LogicalSize::new(self.opts.size[0], self.opts.size[1]))
            .with_min_inner_size(LogicalSize::new(self.opts.min_size[0], self.opts.min_size[1]))
            .with_decorations(self.opts.decorations)
            .with_maximized(self.opts.maximized && !start_minimized)
            // shown after the first frame is painted: no white flash
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

        let ctx = egui::Context::default();
        let repaint_at = self.repaint_at.clone();
        let wake = self.editor.exec.wake().clone();
        ctx.set_request_repaint_callback(move |info| {
            *repaint_at.lock() = Some(Instant::now() + info.delay);
            wake.ping();
        });

        // BC textures go to the GPU as the files hold them when the adapter samples them
        let mut setup = WgpuSetupCreateNew::from_display_handle(event_loop.owned_display_handle());
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut desc = base(adapter);
            desc.required_features |= adapter.features() & egui_wgpu::wgpu::Features::TEXTURE_COMPRESSION_BC;
            desc
        });
        // View > Render > GPU: the adapter of that name, when it can draw into the window
        if !self.opts.gpu_adapter.is_empty() {
            let wanted = self.opts.gpu_adapter.clone();
            let log = self.editor.log.clone();
            setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
                if let Some(a) = crate::gpu::pick_adapter(adapters, &wanted, surface) {
                    return Ok(a.clone());
                }
                log.warn(format!("the GPU '{wanted}' is not there: the device is made on another one"));
                adapters
                    .iter()
                    .find(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
                    .cloned()
                    .ok_or_else(|| "no graphics adapter draws into the window".to_owned())
            }));
        }
        let config = WgpuConfiguration {
            surface: SurfaceConfig::LOW_LATENCY,
            wgpu_setup: setup.into(),
            ..Default::default()
        };
        let rt = self.editor.exec.handle().clone();
        let mut painter = rt.block_on(Painter::new(ctx.clone(), config, false, RendererOptions::default()));
        rt.block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))
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
                // one card reports itself once per backend
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
        if start_minimized {
            window.set_minimized(true);
        }
        Ok(Live {
            window,
            ctx,
            input,
            shared,
            info: ViewportInfo::default(),
            size_px: [size.width.max(1), size.height.max(1)],
            shown: false,
        })
    }

    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = self.live.as_mut() else { return };
        let began = Instant::now();
        self.dirty = false;
        *self.repaint_at.lock() = None;

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
        let mut output = live.ctx.run_ui(raw, |ui| report = shell.frame(ui));

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
            let mut waited = sh.painter.paint_and_update_textures(
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
                // on a poll; no later frame may come to do it
                let polled = Instant::now();
                let _ = gpu.device.poll(egui_wgpu::wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(2)),
                });
                waited += polled.elapsed().as_secs_f32();
            }
            waited
        };

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

        if !live.shown {
            live.shown = true;
            if !gz_win::started_minimized() {
                live.window.set_visible(true);
            }
        }

        self.frame += 1;
        let total = began.elapsed().as_secs_f32() * 1000.0;
        let wait_ms = (waited * 1000.0).min(total);
        self.editor.frames.publish(FrameStamp {
            frame: self.frame,
            active_tab: report.active_tab,
            scale: output.pixels_per_point,
            width: live.size_px[0],
            height: live.size_px[1],
            work_ms: total - wait_ms,
            wait_ms,
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
            let rt = editor.exec.handle().clone();
            if let Err(e) = rt.block_on(sh.painter.set_window(id, Some(window.clone()))) {
                editor.log.warn(format!("a panel's window could not be drawn into: {e}"));
                continue;
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

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.dirty = true;
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }
        match self.create(event_loop) {
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: UserEvent) {
        // Leaving has to be asked for while a message is being handled: asked from
        // `about_to_wait`, the loop goes to sleep first and leaves at the next message.
        if self.editor.quit_token().is_cancelled() {
            event_loop.exit();
            return;
        }
        self.dirty = true;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.quit_token().is_cancelled() {
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        let due = *self.repaint_at.lock();
        if due.is_some_and(|at| at <= now) {
            self.dirty = true;
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
        // 4. the device handle given out
        step("the shared device", &mut || *self.link.gpu.lock() = None);
        if let Some(live) = self.live.take() {
            let Live { window, ctx, input, shared, .. } = live;
            // 5. the panels' own windows: their surfaces first
            let rt = self.editor.exec.handle().clone();
            step("the panels' windows", &mut || {
                if let Ok(mut sh) = shared.try_borrow_mut() {
                    let ids: Vec<ViewportId> = sh.extra.keys().copied().collect();
                    for id in ids {
                        let _ = rt.block_on(sh.painter.set_window(id, None));
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
pub fn run(editor: Arc<Editor>, opts: WindowOptions, shell: Box<dyn Shell>) -> Result<(), String> {
    // this thread draws and presents: it works in bursts and gets its core before the pools,
    // which load every core during a build
    let _ = gz_win::raise_this_thread_priority();
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| format!("the event loop did not start: {e}"))?;
    let proxy = event_loop.create_proxy();
    editor.exec.wake().set(move || {
        let _ = proxy.send_event(UserEvent::Wake);
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
        dirty: true,
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
