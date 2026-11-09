//! The window and its frame loop.
//!
//! The editor draws on demand: a frame runs when the system asks for one, when egui wants a
//! repaint, or when anything pings the editor's `Wake` - a store published, a query arrived, a
//! request is waiting for a picture. Frames run straight from the event loop rather than through
//! a redraw request, so a minimised or covered editor still runs its UI pass and can be
//! pictured: the pass draws into a capture texture when no surface frame is to be had.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{ViewportId, ViewportInfo};
use egui_wgpu::winit::Painter;
use egui_wgpu::{RenderState, RendererOptions, SurfaceConfig, WgpuConfiguration, WgpuSetup};
use egui_winit::winit;
use gz_editor::{Editor, FrameStamp, SmolStr};
use parking_lot::Mutex;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self { title: "Grizzly Editor".to_owned(), size: [1440.0, 900.0], min_size: [960.0, 600.0], maximized: false, decorations: true }
    }
}

/// How other threads reach the window: pictures of it, and the GPU it draws with.
#[derive(Default)]
pub struct WindowLink {
    captures: Mutex<Vec<egui::ScreenshotCallback>>,
    gpu: Mutex<Option<RenderState>>,
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
}

struct Live {
    window: Arc<Window>,
    ctx: egui::Context,
    input: egui_winit::State,
    painter: Painter,
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
}

impl App {
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<Live, String> {
        let start_minimized = gz_win::started_minimized();
        let attrs = Window::default_attributes()
            .with_title(&self.opts.title)
            .with_inner_size(LogicalSize::new(self.opts.size[0], self.opts.size[1]))
            .with_min_inner_size(LogicalSize::new(self.opts.min_size[0], self.opts.min_size[1]))
            .with_decorations(self.opts.decorations)
            .with_maximized(self.opts.maximized && !start_minimized)
            // shown after the first frame is painted: no white flash
            .with_visible(false);
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| format!("the window could not be created: {e}"))?);

        let ctx = egui::Context::default();
        let repaint_at = self.repaint_at.clone();
        let wake = self.editor.exec.wake().clone();
        ctx.set_request_repaint_callback(move |info| {
            *repaint_at.lock() = Some(Instant::now() + info.delay);
            wake.ping();
        });

        let config = WgpuConfiguration {
            surface: SurfaceConfig::LOW_LATENCY,
            wgpu_setup: WgpuSetup::from_display_handle(event_loop.owned_display_handle()),
            ..Default::default()
        };
        let rt = self.editor.exec.handle().clone();
        let mut painter = rt.block_on(Painter::new(ctx.clone(), config, false, RendererOptions::default()));
        rt.block_on(painter.set_window(ViewportId::ROOT, Some(window.clone())))
            .map_err(|e| format!("the graphics device could not be created: {e}"))?;
        let gpu = painter.render_state().ok_or_else(|| "the graphics device could not be created".to_owned())?;

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

        let size = window.inner_size();
        if start_minimized {
            window.set_minimized(true);
        }
        Ok(Live {
            window,
            ctx,
            input,
            painter,
            info: ViewportInfo::default(),
            size_px: [size.width.max(1), size.height.max(1)],
            shown: false,
        })
    }

    fn run_frame(&mut self) {
        let Some(live) = self.live.as_mut() else { return };
        self.dirty = false;
        *self.repaint_at.lock() = None;

        let mut raw = live.input.take_egui_input(&live.window);
        egui_winit::update_viewport_info(&mut live.info, &live.ctx, &live.window, self.frame == 0);
        raw.viewports.insert(ViewportId::ROOT, live.info.clone());

        let shell = &mut self.shell;
        let mut report = FrameReport::default();
        let mut output = live.ctx.run_ui(raw, |ui| report = shell.frame(ui));

        live.input.handle_platform_output(&live.window, output.platform_output);

        let mut actions = Vec::new();
        if let Some(root) = output.viewport_output.remove(&ViewportId::ROOT) {
            egui_winit::process_viewport_commands(&live.ctx, &mut live.info, root.commands, &live.window, &mut actions);
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
        live.painter.paint_and_update_textures(
            ViewportId::ROOT,
            output.pixels_per_point,
            clear,
            &primitives,
            &mut output.textures_delta,
            captures,
            &live.window,
        );
        if capturing && let Some(gpu) = live.painter.render_state() {
            // the picture is handed over when the read-back buffer is mapped, which happens on a
            // poll; no later frame may come to do it
            let _ = gpu.device.poll(egui_wgpu::wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(2)) });
        }

        if !live.shown {
            live.shown = true;
            if !gz_win::started_minimized() {
                live.window.set_visible(true);
            }
        }

        self.frame += 1;
        self.editor.frames.publish(FrameStamp {
            frame: self.frame,
            active_tab: report.active_tab,
            scale: output.pixels_per_point,
            width: live.size_px[0],
            height: live.size_px[1],
        });
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(live) = self.live.as_mut() else { return };
        let response = live.input.on_window_event(&live.window, &event);
        if response.repaint {
            self.dirty = true;
        }
        match event {
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) {
                    live.size_px = [size.width, size.height];
                    live.painter.on_window_resized(ViewportId::ROOT, w, h);
                }
                self.dirty = true;
            }
            WindowEvent::RedrawRequested => self.run_frame(),
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
            self.run_frame();
        }
        match *self.repaint_at.lock() {
            Some(at) if at > Instant::now() => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            Some(_) => event_loop.set_control_flow(ControlFlow::Poll),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        *self.link.gpu.lock() = None;
        self.live = None;
    }
}

/// Opens the window and runs the editor's frame loop on the calling thread - the main thread -
/// until the window closes or the editor is asked to quit.
pub fn run(editor: Arc<Editor>, opts: WindowOptions, shell: Box<dyn Shell>) -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().map_err(|e| format!("the event loop did not start: {e}"))?;
    let proxy = event_loop.create_proxy();
    editor.exec.wake().set(move || {
        let _ = proxy.send_event(UserEvent::Wake);
    });
    let link = editor.services.get_or_init(WindowLink::default);
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
    };
    let ran = event_loop.run_app(&mut app);
    // nothing may ping a loop that is gone
    editor.exec.wake().set(|| {});
    if let Some(e) = app.failed {
        return Err(e);
    }
    ran.map_err(|e| format!("the event loop stopped: {e}"))
}
