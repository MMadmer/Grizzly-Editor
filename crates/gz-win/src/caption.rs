//! The hit test of a window that draws its own caption.
//!
//! An undecorated window's client covers the whole window, so Windows answers `HTCLIENT`
//! everywhere: no drag by the caption, no double-click maximise, no snap layouts, no resize
//! borders. [`install_caption_hit_test`] chains a window procedure in front of the window's own
//! and answers `WM_NCHITTEST` from a [`CaptionShape`] the shell publishes every frame - read with
//! one atomic load, never a lock the UI thread could be holding.
//!
//! The maximise button answers `HTMAXBUTTON`, which is what makes Windows 11 offer its snap
//! layouts over it. A click there is then a non-client click egui never sees: the procedure
//! takes it itself (maximise or restore on release) and keeps [`CaptionHitTest::max_hot`]
//! for the shell to draw the hover.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwap;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWLP_WNDPROC, GetClientRect, GetPropW, GetWindowThreadProcessId, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsIconic,
    IsWindow, IsZoomed, RemovePropW, SW_MAXIMIZE, SW_RESTORE, SetPropW, SetWindowLongPtrW, ShowWindow,
    WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WNDPROC,
};
use windows::core::w;

/// A rectangle in client pixels, `x1` / `y1` exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PxRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl PxRect {
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// What the caption drew this frame, in client pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptionShape {
    /// The caption band; `None` when the frame drew none (nothing drags the window then).
    pub band: Option<PxRect>,
    /// Parts of the band that take the mouse themselves: menus, tabs, window buttons.
    pub interactive: Vec<PxRect>,
    /// The maximise button: `HTMAXBUTTON`.
    pub max_button: Option<PxRect>,
    /// How far in from each edge a restored window resizes; 0 for no resize borders.
    pub border: i32,
}

/// The hit test's answer for a point of the client, `x` / `y` in client pixels.
#[must_use]
pub fn hit_test(shape: &CaptionShape, x: i32, y: i32, width: i32, height: i32, restored: bool) -> u32 {
    let b = shape.border;
    if restored && b > 0 {
        let corner = b * 2;
        let (left, right, top, bottom) = (x < b, x >= width - b, y < b, y >= height - b);
        let (near_left, near_right, near_top, near_bottom) =
            (x < corner, x >= width - corner, y < corner, y >= height - corner);
        if top {
            return if near_left {
                HTTOPLEFT
            } else if near_right {
                HTTOPRIGHT
            } else {
                HTTOP
            };
        }
        if bottom {
            return if near_left {
                HTBOTTOMLEFT
            } else if near_right {
                HTBOTTOMRIGHT
            } else {
                HTBOTTOM
            };
        }
        if left {
            return if near_top {
                HTTOPLEFT
            } else if near_bottom {
                HTBOTTOMLEFT
            } else {
                HTLEFT
            };
        }
        if right {
            return if near_top {
                HTTOPRIGHT
            } else if near_bottom {
                HTBOTTOMRIGHT
            } else {
                HTRIGHT
            };
        }
    }
    if !shape.band.is_some_and(|band| band.contains(x, y)) {
        return HTCLIENT;
    }
    if shape.max_button.is_some_and(|r| r.contains(x, y)) {
        return HTMAXBUTTON;
    }
    if shape.interactive.iter().any(|r| r.contains(x, y)) {
        return HTCLIENT;
    }
    HTCAPTION
}

/// What the shell and the window procedure share.
#[derive(Debug, Default)]
pub struct CaptionHitTest {
    shape: ArcSwap<CaptionShape>,
    max_hot: AtomicBool,
}

impl CaptionHitTest {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Replaces what the hit test answers from. Cheap when nothing changed.
    pub fn publish(&self, shape: CaptionShape) {
        if **self.shape.load() != shape {
            self.shape.store(Arc::new(shape));
        }
    }

    /// The shape the hit test answers from now.
    #[must_use]
    pub fn shape(&self) -> Arc<CaptionShape> {
        self.shape.load_full()
    }

    /// Whether the mouse is over the maximise button - a non-client spot egui cannot see.
    #[must_use]
    pub fn max_hot(&self) -> bool {
        self.max_hot.load(Ordering::Relaxed)
    }
}

struct Hook {
    caption: Arc<CaptionHitTest>,
    prev: isize,
}

fn as_wndproc(value: isize) -> WNDPROC {
    // SAFETY: `value` is what GetWindowLongPtr/SetWindowLongPtr returned for GWLP_WNDPROC: a
    // window procedure (or a handle CallWindowProcW resolves itself), never dereferenced here.
    // An `Option<fn>` has the layout of a nullable pointer, the same size as `isize`.
    unsafe { std::mem::transmute::<isize, WNDPROC>(value) }
}

/// The hook of `hwnd`, borrowed for as long as the caller does not call into another
/// procedure: a nested message may destroy the window and free it.
///
/// # Safety
/// `hwnd` must be a window of the calling thread.
unsafe fn hook_of<'a>(hwnd: HWND) -> Option<&'a Hook> {
    // SAFETY: the property holds the pointer `install_caption_hit_test` boxed, or nothing; it is
    // removed (and the box freed) only in WM_NCDESTROY on this very thread, never while a borrow
    // taken here is alive, because callers drop it before calling into another procedure.
    unsafe {
        let ptr = GetPropW(hwnd, PROP).0.cast::<Hook>();
        ptr.cast_const().as_ref()
    }
}

const PROP: windows::core::PCWSTR = w!("GrizzlyCaptionHook");

fn lparam_point(lparam: LPARAM) -> POINT {
    // the screen point of a WM_NC* message: two signed words
    let v = lparam.0 as u32;
    POINT { x: i32::from((v & 0xFFFF) as u16 as i16), y: i32::from((v >> 16) as u16 as i16) }
}

unsafe extern "system" fn caption_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the system calls this procedure on the window's own thread.
    let Some(hook) = (unsafe { hook_of(hwnd) }) else {
        // the hook is gone (cannot happen while this procedure is installed): nothing to chain to
        // SAFETY: plain default processing of a message of a live window.
        return unsafe { windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    let prev = as_wndproc(hook.prev);
    let caption = hook.caption.clone();
    let chain = |msg: u32, wparam: WPARAM, lparam: LPARAM| {
        // SAFETY: `prev` is the procedure this window had before ours; the hook borrow is not used
        // after this call.
        unsafe { CallWindowProcW(prev, hwnd, msg, wparam, lparam) }
    };
    match msg {
        WM_NCHITTEST => {
            let def = chain(msg, wparam, lparam);
            if def.0 != HTCLIENT as isize {
                return def;
            }
            let mut pt = lparam_point(lparam);
            let mut rc = RECT::default();
            // SAFETY: a live window of this thread; `pt` and `rc` are live locals.
            let (ok, restored) = unsafe {
                let ok = ScreenToClient(hwnd, &mut pt).as_bool() && GetClientRect(hwnd, &mut rc).is_ok();
                (ok, !IsZoomed(hwnd).as_bool() && !IsIconic(hwnd).as_bool())
            };
            if !ok {
                return def;
            }
            let shape = caption.shape.load();
            LRESULT(hit_test(&shape, pt.x, pt.y, rc.right - rc.left, rc.bottom - rc.top, restored) as isize)
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 == HTMAXBUTTON as usize => LRESULT(0),
        WM_NCLBUTTONUP if wparam.0 == HTMAXBUTTON as usize => {
            // SAFETY: a live window of this thread.
            unsafe {
                let zoomed = IsZoomed(hwnd).as_bool();
                let _ = ShowWindow(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE });
            }
            LRESULT(0)
        }
        WM_NCMOUSEMOVE => {
            let hot = wparam.0 == HTMAXBUTTON as usize;
            if hot {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: `track` is a live, filled TRACKMOUSEEVENT for a window of this thread.
                let _ = unsafe { TrackMouseEvent(&mut track) };
            }
            set_hot(hwnd, &caption, hot);
            chain(msg, wparam, lparam)
        }
        WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
            set_hot(hwnd, &caption, false);
            chain(msg, wparam, lparam)
        }
        WM_NCDESTROY => {
            // SAFETY: this thread's window; the property is ours. The box is freed once: the
            // property is gone afterwards, so no later message finds it. The previous procedure
            // goes back first, so nothing calls into ours with a freed hook.
            unsafe {
                let _ = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, hook.prev);
                if let Ok(handle) = RemovePropW(hwnd, PROP)
                    && !handle.0.is_null()
                {
                    drop(Box::from_raw(handle.0.cast::<Hook>()));
                }
            }
            chain(msg, wparam, lparam)
        }
        _ => chain(msg, wparam, lparam),
    }
}

fn set_hot(hwnd: HWND, caption: &CaptionHitTest, hot: bool) {
    if caption.max_hot.swap(hot, Ordering::Relaxed) != hot {
        // a frame shows the hover: the window draws on WM_PAINT
        // SAFETY: a live window of this thread; no rectangle means the whole client.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// Puts the caption hit test in front of the window procedure of `hwnd`, a window of the calling
/// thread. It stays until the window is destroyed. An error when `hwnd` is not a window of this
/// thread or already has the hook.
pub fn install_caption_hit_test(hwnd: isize, caption: Arc<CaptionHitTest>) -> io::Result<()> {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: IsWindow and GetWindowThreadProcessId accept any value; the thread check makes
    // every later call happen on the window's own thread, where it cannot be destroyed between
    // two of our calls.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || GetWindowThreadProcessId(hwnd, None) != GetCurrentThreadId() {
            return Err(io::Error::other("the caption hit test needs a window of the calling thread"));
        }
        if !GetPropW(hwnd, PROP).0.is_null() {
            return Err(io::Error::other("the window already has the caption hit test"));
        }
        let hook = Box::into_raw(Box::new(Hook { caption, prev: 0 }));
        if let Err(e) = SetPropW(hwnd, PROP, Some(HANDLE(hook.cast()))) {
            drop(Box::from_raw(hook));
            return Err(io::Error::other(e));
        }
        let proc_ptr: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = caption_proc;
        // `prev` is written before any message reaches our procedure: both happen on this thread
        // and SetWindowLongPtrW sends nothing
        let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc_ptr as usize as isize);
        if prev == 0 {
            let _ = RemovePropW(hwnd, PROP);
            drop(Box::from_raw(hook));
            return Err(io::Error::last_os_error());
        }
        (*hook).prev = prev;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> CaptionShape {
        CaptionShape {
            band: Some(PxRect { x0: 0, y0: 0, x1: 1000, y1: 32 }),
            interactive: vec![
                PxRect { x0: 30, y0: 0, x1: 400, y1: 32 },
                PxRect { x0: 862, y0: 0, x1: 908, y1: 32 },
                PxRect { x0: 954, y0: 0, x1: 1000, y1: 32 },
            ],
            max_button: Some(PxRect { x0: 908, y0: 0, x1: 954, y1: 32 }),
            border: 6,
        }
    }

    #[test]
    fn band_parts() {
        let s = shape();
        assert_eq!(hit_test(&s, 100, 16, 1000, 700, false), HTCLIENT);
        assert_eq!(hit_test(&s, 600, 16, 1000, 700, false), HTCAPTION);
        assert_eq!(hit_test(&s, 10, 16, 1000, 700, false), HTCAPTION);
        assert_eq!(hit_test(&s, 930, 16, 1000, 700, false), HTMAXBUTTON);
        assert_eq!(hit_test(&s, 977, 16, 1000, 700, false), HTCLIENT);
        assert_eq!(hit_test(&s, 500, 300, 1000, 700, false), HTCLIENT);
    }

    #[test]
    fn borders_only_when_restored() {
        let s = shape();
        assert_eq!(hit_test(&s, 600, 2, 1000, 700, true), HTTOP);
        assert_eq!(hit_test(&s, 2, 2, 1000, 700, true), HTTOPLEFT);
        assert_eq!(hit_test(&s, 2, 300, 1000, 700, true), HTLEFT);
        assert_eq!(hit_test(&s, 998, 698, 1000, 700, true), HTBOTTOMRIGHT);
        assert_eq!(hit_test(&s, 600, 2, 1000, 700, false), HTCAPTION);
        assert_eq!(hit_test(&CaptionShape::default(), 600, 2, 1000, 700, true), HTCLIENT);
    }
}
