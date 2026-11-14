//! View > Render > GPU: the adapter the
//! device is made on - a preference read when the window opens, so a change takes a restart -
//! and the driver's hint, which Windows keeps per executable in
//! `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` and applies when the process starts.
//! The hint is read and written through `reg.exe` on the IO pool: never from a frame.
//!
//! The sounds' mute switch lives here too: View > Mute Sounds (kept in the settings) and a mute
//! for a while (a library's preview) together decide it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use gz_editor::Editor;
use parking_lot::Mutex;

use crate::settings::EditorSettings;

const GPU_PREF_KEY: &str = r"HKCU\Software\Microsoft\DirectX\UserGpuPreferences";

/// The driver's hint (`EGpuPreference`): the value Windows stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverHint {
    Auto,
    PowerSaving,
    Performance,
}

impl DriverHint {
    pub const ALL: [DriverHint; 3] = [DriverHint::Auto, DriverHint::Performance, DriverHint::PowerSaving];

    /// The View menu's row.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            DriverHint::Auto => "Driver Hint: Automatic",
            DriverHint::Performance => "Driver Hint: High Performance",
            DriverHint::PowerSaving => "Driver Hint: Power Saving",
        }
    }

    /// The MCP name.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            DriverHint::Auto => "auto",
            DriverHint::Performance => "performance",
            DriverHint::PowerSaving => "power_saving",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.key().eq_ignore_ascii_case(text))
    }

    /// `GetGpuPreference`: what the stored value says.
    #[must_use]
    pub fn of_value(value: &str) -> Self {
        if value.contains("GpuPreference=2") {
            DriverHint::Performance
        } else if value.contains("GpuPreference=1") {
            DriverHint::PowerSaving
        } else {
            DriverHint::Auto
        }
    }

    fn code(self) -> u8 {
        match self {
            DriverHint::Auto => 0,
            DriverHint::PowerSaving => 1,
            DriverHint::Performance => 2,
        }
    }

    fn from_code(v: u8) -> Self {
        match v {
            1 => DriverHint::PowerSaving,
            2 => DriverHint::Performance,
            _ => DriverHint::Auto,
        }
    }
}

/// What the window made its device on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapters {
    /// Every adapter the runtime reports, by name.
    pub names: Vec<String>,
    /// The one in use.
    pub active: String,
}

/// The GPU's state: an `Editor` service.
#[derive(Default)]
pub struct Gpu {
    adapters: Mutex<Option<Adapters>>,
    /// `DriverHint` as a code; read once, after that what was set.
    hint: AtomicU8,
    hint_read: AtomicBool,
    hint_reading: AtomicBool,
    /// A mute held for a while, over what View > Mute Sounds says.
    mute_held: AtomicBool,
    /// What the player reads: muted now.
    muted: Arc<AtomicBool>,
}

impl Gpu {
    pub fn of(editor: &Editor) -> Arc<Gpu> {
        editor.services.get_or_init(|| {
            let made = Gpu::default();
            made.muted.store(EditorSettings::of(editor).render_flag("mute_sounds"), Ordering::Release);
            made
        })
    }

    /// The window says what it made its device on.
    pub(crate) fn set_adapters(&self, a: Adapters) {
        *self.adapters.lock() = Some(a);
    }

    /// The adapters and the active one; `None` before the window made its device.
    #[must_use]
    pub fn adapters(&self) -> Option<Adapters> {
        self.adapters.lock().clone()
    }

    /// The driver's hint as last read or set; reads it the first time (off the UI thread), and
    /// answers `Auto` until the read is done.
    pub fn hint(&self, editor: &Arc<Editor>) -> DriverHint {
        if !self.hint_read.load(Ordering::Acquire) && !self.hint_reading.swap(true, Ordering::AcqRel) {
            let ed = editor.clone();
            editor.exec.spawn_io(move || {
                let hint = read_hint();
                let gpu = Gpu::of(&ed);
                gpu.hint.store(hint.code(), Ordering::Release);
                gpu.hint_read.store(true, Ordering::Release);
                ed.exec.wake().ping();
            });
        }
        DriverHint::from_code(self.hint.load(Ordering::Acquire))
    }

    /// Whether the hint has been read.
    #[must_use]
    pub fn hint_known(&self) -> bool {
        self.hint_read.load(Ordering::Acquire)
    }

    /// `SetGpuPreference`: written on the IO pool; Windows applies it at the next start.
    pub fn set_hint(&self, editor: &Arc<Editor>, hint: DriverHint) {
        self.hint.store(hint.code(), Ordering::Release);
        self.hint_read.store(true, Ordering::Release);
        let log = editor.log.clone();
        editor.exec.spawn_io(move || {
            if let Err(e) = write_hint(hint) {
                log.warn(format!("the driver's GPU hint could not be written: {e}"));
            }
        });
    }

    /// The mute the player obeys, shared with it.
    #[must_use]
    pub fn mute_flag(&self) -> Arc<AtomicBool> {
        self.muted.clone()
    }

    /// On holds the sounds muted until a call with off, which goes back to what View > Mute Sounds
    /// says.
    pub fn mute_held(&self, editor: &Editor, on: bool) {
        self.mute_held.store(on, Ordering::Release);
        self.refresh_mute(editor);
    }

    /// View > Mute Sounds, kept in the settings.
    pub fn set_mute_sounds(&self, editor: &Editor, on: bool) {
        let settings = EditorSettings::of(editor);
        settings.set_render_flag("mute_sounds", on);
        settings.save_soon(editor);
        self.refresh_mute(editor);
    }

    fn refresh_mute(&self, editor: &Editor) {
        let kept = EditorSettings::of(editor).render_flag("mute_sounds");
        self.muted.store(kept || self.mute_held.load(Ordering::Acquire), Ordering::Release);
    }
}

/// The value name: the executable's full path, as `GetModuleFileName` gives it.
fn exe_value() -> Result<String, String> {
    std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

fn reg() -> std::process::Command {
    let mut c = std::process::Command::new("reg.exe");
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up
        c.creation_flags(0x0800_0000);
    }
    c
}

fn read_hint() -> DriverHint {
    let Ok(value) = exe_value() else { return DriverHint::Auto };
    let Ok(out) = reg().args(["query", GPU_PREF_KEY, "/v", &value]).output() else { return DriverHint::Auto };
    if !out.status.success() {
        return DriverHint::Auto;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|l| l.contains("REG_SZ"))
        .map_or(DriverHint::Auto, |l| DriverHint::of_value(l.rsplit("REG_SZ").next().unwrap_or("")))
}

fn write_hint(hint: DriverHint) -> Result<(), String> {
    let value = exe_value()?;
    let out = if hint == DriverHint::Auto {
        // no value at all == "let Windows decide"
        reg().args(["delete", GPU_PREF_KEY, "/v", &value, "/f"]).output()
    } else {
        let data = format!("GpuPreference={};", hint.code());
        reg().args(["add", GPU_PREF_KEY, "/v", &value, "/t", "REG_SZ", "/d", &data, "/f"]).output()
    }
    .map_err(|e| e.to_string())?;
    // deleting a value that is not there is fine
    if out.status.success() || hint == DriverHint::Auto {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Picks the adapter named in the preference among the runtime's, one that can draw into the
/// window; `None` when none of that name can.
#[must_use]
pub fn pick_adapter<'a>(
    adapters: &'a [wgpu::Adapter],
    wanted: &str,
    surface: Option<&wgpu::Surface<'_>>,
) -> Option<&'a wgpu::Adapter> {
    adapters.iter().find(|a| a.get_info().name == wanted && surface.is_none_or(|s| a.is_surface_supported(s)))
}

#[cfg(test)]
mod tests {
    use super::DriverHint;

    #[test]
    fn hints_read_like_get_gpu_preference() {
        assert_eq!(DriverHint::of_value("GpuPreference=2;"), DriverHint::Performance);
        assert_eq!(DriverHint::of_value("   GpuPreference=1;"), DriverHint::PowerSaving);
        assert_eq!(DriverHint::of_value("SwapEffectUpgradeEnable=1;"), DriverHint::Auto);
        assert_eq!(DriverHint::parse("Performance"), Some(DriverHint::Performance));
        assert_eq!(DriverHint::parse("x"), None);
    }
}

/// Libraries > Reload Textures (`CommandReloadTextures`, `EDevice->ReloadTextures`): a
/// generation every texture cache of the window compares with the one it loaded at - the
/// viewport's renderer, the previews - and reads its textures again when it moved.
#[derive(Default)]
pub struct TextureReload(std::sync::atomic::AtomicU64);

impl TextureReload {
    /// Asks every cache to read its textures again.
    pub fn bump(editor: &Editor) {
        editor.services.get_or_init(TextureReload::default).0.fetch_add(1, Ordering::AcqRel);
        editor.exec.wake().ping();
    }

    #[must_use]
    pub fn generation(editor: &Editor) -> u64 {
        editor.services.get::<TextureReload>().map_or(0, |r| r.0.load(Ordering::Acquire))
    }
}
