//! Preferences > Viewport, the viewport's group: how it renders (the render scale, Real Time),
//! the graphics adapter and the driver's preference, and its sounds. The view's own switches -
//! the view mode, what is shown, the snapping - are its pills'.

use std::sync::Arc;

use gz_app::{Enabled, PrefGroup, PrefKind, PrefRow, ShellBuilder};
use gz_editor::Editor;
use gz_ui::Icon;

use crate::service::{RENDER_SCALES, SETTINGS};

// the rows name their settings by place in the table: a test holds the places

const SCALE_CHOICES: [&str; 4] =
    [RENDER_SCALES[0].1, RENDER_SCALES[1].1, RENDER_SCALES[2].1, RENDER_SCALES[3].1];
const GPU_CHOICES: [&str; 3] = ["Automatic", "High Performance", "Power Saving"];

fn real_time(ed: &Arc<Editor>) -> bool {
    crate::service::real_time(ed)
}

fn set_real_time(ed: &Arc<Editor>, on: bool) {
    crate::service::set_real_time(ed, on);
    crate::service::save(ed);
}

fn muted(ed: &Arc<Editor>) -> bool {
    gz_app::EditorSettings::of(ed).render_flag("mute_sounds")
}

fn set_muted(ed: &Arc<Editor>, on: bool) {
    crate::service::set_mute_sounds(ed, on);
}

fn always(_: &Arc<Editor>) -> Enabled {
    Enabled::On
}

/// The adapters to cycle through: the system's choice, then each by name.
fn adapter_choices(ed: &Arc<Editor>) -> Vec<String> {
    let names = gz_app::gpu::Gpu::of(ed).adapters().map(|a| a.names).unwrap_or_default();
    std::iter::once(String::new()).chain(names).collect()
}

fn next_adapter(ed: &Arc<Editor>) {
    let settings = gz_app::EditorSettings::of(ed);
    let choices = adapter_choices(ed);
    let now = settings.gpu_adapter();
    let i = choices.iter().position(|c| *c == now).map_or(0, |i| (i + 1) % choices.len().max(1));
    settings.set_gpu_adapter(choices.get(i).map_or("", String::as_str));
    settings.save_soon(ed);
}

fn adapter_status(ed: &Arc<Editor>) -> String {
    let settings = gz_app::EditorSettings::of(ed);
    let active = gz_app::gpu::Gpu::of(ed).adapters().map(|a| a.active).unwrap_or_default();
    let wanted = settings.gpu_adapter();
    let wanted = if wanted.is_empty() { "the system's choice".to_owned() } else { wanted };
    let active = if active.is_empty() { "unknown".to_owned() } else { active };
    format!("Uses {active}; the next start uses {wanted}.")
}

const RENDERING: [PrefRow; 2] = [
    PrefRow {
        label: "Render Scale",
        help: "The size the viewport renders at, against its size on the screen. Below 100 % draws faster and blurrier.",
        kind: PrefKind::Setting { def: SETTINGS[17], unit: "", choices: &SCALE_CHOICES },
    },
    PrefRow {
        label: "Real Time",
        help: "The viewport draws every frame, not only when something it shows changes.",
        kind: PrefKind::Switch { get: real_time, set: set_real_time, default: false },
    },
];

const GRAPHICS: [PrefRow; 2] = [
    PrefRow {
        label: "Graphics Adapter",
        help: "The graphics adapter the editor draws with. Takes effect the next time the editor starts.",
        kind: PrefKind::Button {
            text: "Use the Next Adapter",
            run: next_adapter,
            enabled: always,
            status: adapter_status,
        },
    },
    PrefRow {
        label: "GPU Preference",
        help: "Which GPU Windows gives the editor on a computer with two. Takes effect the next time the editor starts.",
        kind: PrefKind::Setting { def: SETTINGS[18], unit: "", choices: &GPU_CHOICES },
    },
];

const SOUND: [PrefRow; 1] = [PrefRow {
    label: "Mute Sounds",
    help: "The scene's sound sources stay silent in the viewport.",
    kind: PrefKind::Switch { get: muted, set: set_muted, default: false },
}];

/// Plugs the viewport's groups into Preferences > Viewport.
pub(crate) fn install(b: &mut ShellBuilder) {
    b.pref_group(PrefGroup {
        page: "Viewport",
        icon: Icon::Perspective,
        section: "Rendering",
        rows: &RENDERING,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "Viewport",
        icon: Icon::Perspective,
        section: "Graphics",
        rows: &GRAPHICS,
        note: None,
    });
    b.pref_group(PrefGroup {
        page: "Viewport",
        icon: Icon::Perspective,
        section: "Sound",
        rows: &SOUND,
        note: None,
    });
}

#[cfg(test)]
mod tests {
    use crate::service::SETTINGS;

    #[test]
    fn the_rows_edit_the_settings_they_name() {
        assert_eq!(SETTINGS[17].key, "viewport_render_scale");
        assert_eq!(SETTINGS[18].key, "viewport_gpu_preference");
    }
}
