//! Frames for the kit's tests: a context, a screen, input events, and nothing painted.

use egui::{Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Ui, vec2};

/// A context with the kit's theme and fonts as egui ships them.
pub fn context() -> Context {
    let ctx = Context::default();
    crate::theme::install(&ctx, crate::theme::Accent::default());
    ctx
}

/// Runs one frame of `add` with `events`, at time `t`.
pub fn frame(ctx: &Context, t: f64, events: Vec<Event>, mut add: impl FnMut(&mut Ui)) {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0));
    let mut out = ctx
        .run_ui(RawInput { screen_rect: Some(screen), time: Some(t), events, ..RawInput::default() }, |ui| {
            add(ui)
        });
    // the textures egui made are the painter's, and here nobody paints
    out.textures_delta.clear();
}

/// The events of a click at `at`: the move, the press, the release.
pub fn click_at(at: Pos2) -> Vec<Vec<Event>> {
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    vec![vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]]
}

/// A key press.
pub fn key(k: Key) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

/// Text typed.
pub fn typed(text: &str) -> Event {
    Event::Text(text.to_owned())
}

/// The events of a right click at `at`.
pub fn secondary_click_at(at: Pos2) -> Vec<Vec<Event>> {
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    vec![vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]]
}
