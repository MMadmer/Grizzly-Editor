//! Preferences: the `preferences` singleton tab (docs/UI_DESIGN.md, section 6, "Preferences").
//!
//! A search, a nav list with an icon per page, and the page: a column of cards at most 760 pt wide,
//! one per section, each a property grid (a value at most 16 em). A change applies at once through
//! the shell's `EditorSettings` and the file is written when the field being typed in or dragged
//! lets go, when the tab is left and when it closes.
//!
//! [`PREFS`] is every preference of the window and the level editor - its settings key, its type,
//! its official label, help and unit. Every row is live: the shell's settings keep its key
//! (`the_shells_keys_are_live`). An area adds a group of its own to a page - an existing one or
//! one it brings (Game, Library, Graph, Updates) - with `ShellBuilder::pref_group`; the page shows
//! it as a card of its own.

use std::sync::Arc;

use egui::{Align, Color32, Layout, Rect, ScrollArea, Ui, UiBuilder, Vec2, pos2};
use gz_app::settings::{INTERFACE_SCALES, PrefType, pref_def};
use gz_app::{
    DocCx, Document, EditorSettings, PrefGroup, PrefGroups, PrefKind as RowKind, PrefRow, ShellBuilder,
};
use gz_editor::{Editor, SmolStr};
use gz_ui::grid::{Grid, color, combo, format_number, help_footer, integer, number, read_only, switch};
use gz_ui::list::{NavRowSpec, list, nav_row};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{Accent, Font, Icon, PropFlags, PropertyGrid, SearchField, palette};
use parking_lot::Mutex;

use crate::keyboard::KeyboardPage;

/// How a preference is edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefKind {
    /// The editor's colours: the accent family.
    Theme,
    /// The UI's scale: Auto (the monitor's) or a percentage.
    Scale,
    Float,
    Whole,
    Switch,
    /// Degrees.
    Angle,
    /// `0xAARRGGBB`.
    Color,
    /// A bit of `object_flags`.
    Flag(u32),
}

/// A preference: where it sits (`Page\Section\Label`), its settings key, its type, its help and
/// its unit, and the default and range used while the settings file does not keep it.
#[derive(Clone, Copy, Debug)]
pub struct Pref {
    pub path: &'static str,
    pub key: &'static str,
    pub kind: PrefKind,
    pub help: &'static str,
    pub unit: &'static str,
    pub default: f64,
    pub lo: f64,
    pub hi: f64,
}

const fn pref(
    path: &'static str,
    key: &'static str,
    kind: PrefKind,
    help: &'static str,
    unit: &'static str,
) -> Pref {
    Pref { path, key, kind, help, unit, default: 0.0, lo: 0.0, hi: 0.0 }
}

// one row of the const table of preferences, in the order the table reads
#[allow(clippy::too_many_arguments)]
const fn own(
    path: &'static str,
    key: &'static str,
    kind: PrefKind,
    help: &'static str,
    unit: &'static str,
    default: f64,
    lo: f64,
    hi: f64,
) -> Pref {
    Pref { path, key, kind, help, unit, default, lo, hi }
}

const fn flag(path: &'static str, bit: u32, help: &'static str) -> Pref {
    Pref {
        path,
        key: OBJECT_FLAGS,
        kind: PrefKind::Flag(bit),
        help,
        unit: "",
        default: 0.0,
        lo: 0.0,
        hi: 0.0,
    }
}

/// The objects' display and loading switches, one bit each (`EditorPreferences.h`, `epo*`).
pub const OBJECT_FLAGS: &str = "object_flags";

/// Every preference, in the order the pages list them.
pub const PREFS: &[Pref] = &[
    pref(
        "Interface\\Interface Scale",
        "scale",
        PrefKind::Scale,
        "How large the editor draws everything. Auto follows the monitor's scale.",
        "",
    ),
    pref(
        "Interface\\Accent",
        "accent",
        PrefKind::Theme,
        "The colour of what is active, selected or the one action of a surface.",
        "",
    ),
    pref(
        "General\\Allow Panels Outside the Window",
        "floating_panels",
        PrefKind::Switch,
        "A panel can be dragged out of the editor into a window of its own. Takes effect after a restart.",
        "",
    ),
    pref(
        "General\\Frame Rate Limit",
        "active_idle_fps",
        PrefKind::Whole,
        "The most frames a second the editor draws while its window is in front.",
        "fps",
    ),
    pref(
        "General\\Recent Files",
        "scene_recent_count",
        PrefKind::Whole,
        "How many scenes File > Recent Scene remembers.",
        "",
    ),
    pref(
        "General\\Undo Steps",
        "scene_undo_level",
        PrefKind::Whole,
        "How many scene changes Undo can take back.",
        "",
    ),
    pref(
        "Viewport\\Camera\\Move Sensitivity",
        "cam_sens_move",
        PrefKind::Float,
        "How far the camera moves for a mouse movement.",
        "",
    ),
    pref(
        "Viewport\\Camera\\Rotate Sensitivity",
        "cam_sens_rot",
        PrefKind::Float,
        "How far the camera turns for a mouse movement.",
        "",
    ),
    pref(
        "Viewport\\Camera\\Fly Speed",
        "cam_fly_speed",
        PrefKind::Float,
        "The camera's speed in fly mode.",
        "m/s",
    ),
    pref(
        "Viewport\\Camera\\Fly Altitude",
        "cam_fly_alt",
        PrefKind::Float,
        "The height above the ground the camera keeps in fly mode.",
        "m",
    ),
    pref("Viewport\\Field of View", "view_fov", PrefKind::Angle, "The camera's vertical field of view.", ""),
    pref(
        "Viewport\\Near Plane",
        "view_np",
        PrefKind::Float,
        "Nothing closer to the camera than this is drawn.",
        "m",
    ),
    pref(
        "Viewport\\Far Plane",
        "view_fp",
        PrefKind::Float,
        "Nothing farther from the camera than this is drawn.",
        "m",
    ),
    pref(
        "Viewport\\Render Radius",
        "render_radius",
        PrefKind::Float,
        "Scene objects farther than this from the camera are not drawn.",
        "m",
    ),
    pref(
        "Viewport\\Background Color",
        "scene_clear_color",
        PrefKind::Color,
        "The colour behind everything the viewport draws.",
        "",
    ),
    pref("Viewport\\Fog\\Color", "fog_color", PrefKind::Color, "The colour distant things fade into.", ""),
    pref(
        "Viewport\\Fog\\Density",
        "fog_fogness",
        PrefKind::Float,
        "How quickly distant things fade into the fog.",
        "",
    ),
    pref("Viewport\\Grid\\Cell Size", "grid_cell_size", PrefKind::Float, "The side of a grid cell.", "m"),
    pref(
        "Viewport\\Grid\\Cell Count",
        "grid_cell_count",
        PrefKind::Whole,
        "How many cells the grid has along each side.",
        "",
    ),
    pref(
        "Tools\\Gizmo\\Move Sensitivity",
        "tools_sens_move",
        PrefKind::Float,
        "How far an object moves for a mouse movement.",
        "",
    ),
    pref(
        "Tools\\Gizmo\\Rotate Sensitivity",
        "tools_sens_rot",
        PrefKind::Float,
        "How far an object turns for a mouse movement.",
        "",
    ),
    pref(
        "Tools\\Gizmo\\Scale Sensitivity",
        "tools_sens_scale",
        PrefKind::Float,
        "How much an object grows for a mouse movement.",
        "",
    ),
    pref(
        "Tools\\Gizmo\\Show Move Axes",
        "tools_show_move_axis",
        PrefKind::Switch,
        "Draws the move gizmo's axes on the object being moved.",
        "",
    ),
    pref(
        "Tools\\Snap\\Grid Step",
        "snap_grid_step",
        PrefKind::Float,
        "How far a move goes per step while Grid Snap is on. The viewport toolbar and the status bar show this step.",
        "m",
    ),
    pref(
        "Tools\\Snap\\Angle Step",
        "snap_angle_step",
        PrefKind::Angle,
        "How far a rotation turns per step while Angle Snap is on. The viewport toolbar and the status bar show this step.",
        "",
    ),
    pref(
        "Tools\\Snap\\Scale Step",
        "snap_scale_step",
        PrefKind::Float,
        "How much scaling changes per step while Scale Snap is on. The viewport toolbar shows this step.",
        "",
    ),
    pref(
        "Tools\\Snap\\Surface Search Distance",
        "snap_moveto",
        PrefKind::Float,
        "How far above and below an object the move to a surface looks for the surface it lands on.",
        "m",
    ),
    pref(
        "Tools\\Box Selection\\Limited Depth",
        "bp_lim_depth",
        PrefKind::Switch,
        "A box selects only as deep as the farthest scene object under its four corners.",
        "",
    ),
    pref(
        "Tools\\Box Selection\\Back Face Culling",
        "bp_cull",
        PrefKind::Switch,
        "Faces turned away from the camera are not picked by a box that selects faces. A box of the level editor selects whole objects, so nothing here reads it.",
        "",
    ),
    pref(
        "Tools\\Box Selection\\Depth Tolerance",
        "bp_depth_tolerance",
        PrefKind::Float,
        "How far past that nearest object a box still selects.",
        "m",
    ),
    flag("Tools\\Groups\\Select Whole Group", 1 << 11, "Selecting a group selects every object in it too."),
    flag(
        "Objects\\Library\\Unload Unused Objects",
        1 << 8,
        "A library object leaves memory when no scene object uses it any more.",
    ),
    flag(
        "Objects\\Loading\\Load Render Buffers at Once",
        1 << 9,
        "Meshes build their render buffers as they load, not when they are first drawn.",
    ),
    flag(
        "Objects\\Loading\\Load Collision Models at Once",
        1 << 10,
        "Meshes build their collision models as they load, not when they are first needed.",
    ),
    flag("Objects\\Display\\Show Pivot", 1 << 0, "Draws the pivot of a selected object."),
    flag("Objects\\Display\\Show Animation Path", 1 << 1, "Draws the path an animated object follows."),
    flag(
        "Objects\\Display\\Use LODs",
        1 << 7,
        "An object that has a LOD draws it when it is small on screen.",
    ),
    flag("Objects\\Skeleton\\Show Joints", 1 << 2, "Draws the joints of a skeleton."),
    flag("Objects\\Skeleton\\Show Bone Axes", 1 << 3, "Draws the axes of every bone."),
    flag("Objects\\Skeleton\\Show Bone Names", 1 << 4, "Draws the name of every bone."),
    flag("Objects\\Skeleton\\Show Bone Shapes", 1 << 5, "Draws the collision shape of every bone."),
    own(
        "Recovery\\Enabled",
        "recovery_enabled",
        PrefKind::Switch,
        "Keeps snapshots of the open scene while it has unsaved changes, and offers the last one after the editor did not close cleanly.",
        "",
        1.0,
        0.0,
        1.0,
    ),
    own(
        "Recovery\\Interval",
        "recovery_interval_minutes",
        PrefKind::Whole,
        "Time between two snapshots of the open scene.",
        "min",
        10.0,
        1.0,
        120.0,
    ),
    own(
        "Recovery\\Delay After Input",
        "recovery_interaction_delay_seconds",
        PrefKind::Whole,
        "How long a snapshot waits after the last click or key, so it never interrupts work.",
        "s",
        15.0,
        0.0,
        120.0,
    ),
    own(
        "Recovery\\Snapshots Kept",
        "recovery_retention",
        PrefKind::Whole,
        "How many snapshots of a scene are kept; the oldest go first.",
        "",
        10.0,
        2.0,
        64.0,
    ),
];

/// The Keyboard page: the shortcuts, not a page of the grid.
pub const KEYBOARD: &str = "Keyboard";

/// The pages in the nav's order, with their icons (General is Mod Settings' General too). An
/// area's group may bring a page of its own after these.
pub const PAGES: [(&str, Icon); 7] = [
    ("General", Icon::Info),
    ("Interface", Icon::Aspect),
    ("Viewport", Icon::Perspective),
    ("Tools", Icon::Move),
    ("Objects", Icon::SceneObject),
    ("Recovery", Icon::History),
    (KEYBOARD, Icon::Keyboard),
];

const THEMES: [&str; 2] = ["Grizzly Purple", "Graphite"];
const SCALES: [&str; 6] = ["Auto", "100%", "125%", "150%", "175%", "200%"];

impl Pref {
    /// The page, the section (empty for a row of the page itself) and the label.
    #[must_use]
    pub fn parts(&self) -> (&'static str, &'static str, &'static str) {
        let mut it = self.path.split('\\');
        let page = it.next().unwrap_or("");
        let rest: Vec<&'static str> = it.collect();
        match rest.as_slice() {
            [label] => (page, "", label),
            [section, label] => (page, section, label),
            _ => (page, "", self.path),
        }
    }

    /// The settings key that holds the value.
    #[must_use]
    pub fn settings_key(&self) -> &'static str {
        self.key
    }

    /// The settings file keeps this preference (the theme always).
    #[must_use]
    pub fn live(&self) -> bool {
        matches!(self.kind, PrefKind::Theme | PrefKind::Scale) || pref_def(self.key).is_some()
    }

    /// Its default and range: the shell's where it defines the key, else this table's.
    #[must_use]
    pub fn bounds(&self) -> (f64, f64, f64) {
        match (self.kind, pref_def(self.key)) {
            (PrefKind::Flag(bit), def) => {
                (if def.is_some_and(|d| (d.default as u32) & bit != 0) { 1.0 } else { 0.0 }, 0.0, 1.0)
            }
            (_, Some(d)) => (d.default, d.lo, d.hi),
            (_, None) => (self.default, self.lo, self.hi),
        }
    }

    /// What a search looks at: the label and where it sits.
    #[must_use]
    pub fn matches(&self, search: &str) -> bool {
        gz_ui::search::matches(search, &self.path.replace('\\', " "))
    }
}

/// The value a preference has now: its settings value, or its default while the settings do not
/// keep it.
fn value_of(settings: &EditorSettings, p: &Pref) -> f64 {
    match p.kind {
        PrefKind::Theme => f64::from(u8::from(settings.accent() == Accent::Graphite)),
        // the index into SCALES: 0 Auto
        PrefKind::Scale => settings.interface_scale().map_or(0.0, |s| {
            INTERFACE_SCALES
                .iter()
                .position(|v| (*v as f32 / 100.0 - s).abs() < 0.01)
                .map_or(0.0, |i| (i + 1) as f64)
        }),
        PrefKind::Flag(bit) => match settings.pref(OBJECT_FLAGS) {
            Some(v) => f64::from(u8::from((v as u32) & bit != 0)),
            None => p.bounds().0,
        },
        _ => settings.pref(p.key).unwrap_or(p.bounds().0),
    }
}

/// Applies a new value at once; the file is written later.
fn set_value(settings: &EditorSettings, ctx: &egui::Context, p: &Pref, v: f64) {
    match p.kind {
        PrefKind::Theme => {
            let accent = if v >= 1.0 { Accent::Graphite } else { Accent::Violet };
            settings.set_accent(accent);
            gz_ui::theme::install(ctx, accent);
        }
        PrefKind::Scale => {
            let i = v.round() as usize;
            settings.set_interface_scale(
                i.checked_sub(1).and_then(|k| INTERFACE_SCALES.get(k)).map(|s| *s as f32 / 100.0),
            );
            ctx.request_repaint();
        }
        PrefKind::Flag(bit) => {
            let cur = settings.pref(OBJECT_FLAGS).unwrap_or(0.0) as u32;
            let next = if v != 0.0 { cur | bit } else { cur & !bit };
            settings.set_pref(OBJECT_FLAGS, f64::from(next));
        }
        _ => {
            let (_, lo, hi) = p.bounds();
            let v = if hi > lo { v.clamp(lo, hi) } else { v };
            let v = if pref_def(p.key).is_some_and(|d| d.ty == PrefType::U32) { v.round() } else { v };
            settings.set_pref(p.key, v);
        }
    }
}

/// A value as a read-only row shows it.
fn value_text(p: &Pref, v: f64) -> String {
    match p.kind {
        PrefKind::Switch | PrefKind::Flag(_) => (if v != 0.0 { "On" } else { "Off" }).to_owned(),
        PrefKind::Theme => THEMES.get(v as usize).copied().unwrap_or("").to_owned(),
        PrefKind::Scale => SCALES.get(v as usize).copied().unwrap_or("").to_owned(),
        PrefKind::Color => format!("#{:06x}", (v as u32) & 0x00ff_ffff),
        PrefKind::Angle => format!("{}\u{b0}", format_number(v, 3)),
        _ if p.unit.is_empty() => format_number(v, 3),
        _ => format!("{} {}", format_number(v, 3), p.unit),
    }
}

/// What `preferences action=show` asked for, for the tab to take on its next frame.
#[derive(Default)]
pub struct PrefsNav {
    pending: Mutex<Option<(String, String)>>,
}

impl PrefsNav {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<PrefsNav> {
        editor.services.get_or_init(PrefsNav::default)
    }

    /// `page|search`, as the shell's command hands it to the factory.
    pub fn ask(&self, arg: &str) {
        let (page, search) = arg.split_once('|').unwrap_or((arg, ""));
        *self.pending.lock() = Some((page.to_owned(), search.to_owned()));
    }

    fn take(&self) -> Option<(String, String)> {
        self.pending.lock().take()
    }
}

/// A page by its name, ignoring case.
#[must_use]
pub fn page_named(name: &str) -> Option<&'static str> {
    PAGES.iter().map(|(n, _)| *n).find(|n| n.eq_ignore_ascii_case(name))
}

/// The Preferences tab.
pub struct PreferencesDoc {
    editor: Arc<Editor>,
    settings: Arc<EditorSettings>,
    page: String,
    search: String,
    save_pending: bool,
    keyboard: KeyboardPage,
}

impl PreferencesDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        let settings = EditorSettings::of(editor);
        let page = page_named(&settings.page.lock()).unwrap_or("General").to_owned();
        Self {
            editor: editor.clone(),
            settings,
            page,
            search: String::new(),
            save_pending: false,
            keyboard: KeyboardPage::default(),
        }
    }

    fn set_page(&mut self, page: &str) {
        page.clone_into(&mut self.page);
        *self.settings.page.lock() = SmolStr::new(page);
    }

    fn save(&mut self) {
        self.save_pending = false;
        self.settings.save_soon(&self.editor);
    }

    /// The nav: the search, then a row per page; while searching, a page without a match stays
    /// in its place, dimmed, and one with matches counts them.
    fn draw_nav(&mut self, ui: &mut Ui, width: f32) {
        if SearchField::new(&mut self.search)
            .hint("Search settings")
            .width(width)
            .id_salt("prefs_search")
            .show(ui)
            .changed()
            && !self.search.is_empty()
        {
            self.keyboard.reload(&self.editor);
        }
        ui.add_space(4.0);
        let searching = !self.search.is_empty();
        let pages = self.pages();
        let counts: Vec<usize> = pages
            .iter()
            .map(|(name, _)| {
                if *name == KEYBOARD {
                    self.keyboard.matches(&self.search)
                } else {
                    PREFS.iter().filter(|p| p.parts().0 == *name && p.matches(&self.search)).count()
                }
            })
            .collect();
        let mut picked = None;
        list(ui, "prefs_nav", Vec2::new(width, 0.0), |l| {
            for ((name, icon), count) in pages.iter().zip(&counts) {
                let count_text = count.to_string();
                let spec = NavRowSpec {
                    icon: *icon,
                    tip: if searching { "Clears the search and opens the page" } else { "" },
                    count: (searching && *count > 0).then_some(count_text.as_str()),
                    dim: searching && *count == 0,
                    ..NavRowSpec::default()
                };
                if nav_row(l.ui(), name, name, !searching && self.page == *name, &spec).clicked {
                    picked = Some(*name);
                }
            }
        });
        if let Some(name) = picked {
            self.set_page(name);
            self.search.clear();
            if name == KEYBOARD {
                self.keyboard.reload(&self.editor);
            }
        }
    }

    fn draw_row(&mut self, g: &mut Grid<'_>, p: &Pref, depth: u32, ctx: &egui::Context) {
        let (_, _, label) = p.parts();
        let live = p.live();
        let v = value_of(&self.settings, p);
        let (default, _, _) = p.bounds();
        let switch_like = matches!(p.kind, PrefKind::Switch | PrefKind::Flag(_));
        let mut flags = if switch_like { PropFlags::INLINE } else { PropFlags::NONE };
        if !live {
            flags = flags | PropFlags::READ_ONLY;
        } else if (v - default).abs() > 1e-9 {
            flags = flags | PropFlags::MODIFIED;
        }
        let settings = self.settings.clone();
        let changed = g.row(label, p.help, flags, depth, |ui| {
            ui.set_max_width(em(16.0).min(ui.available_width()));
            if !live {
                read_only(ui, &value_text(p, v), Clip::End);
                return None;
            }
            match p.kind {
                PrefKind::Theme => {
                    let mut i = v as usize;
                    combo(ui, "prefs.theme", &mut i, &THEMES).then_some(i as f64)
                }
                PrefKind::Scale => {
                    let mut i = v as usize;
                    combo(ui, "prefs.scale", &mut i, &SCALES).then_some(i as f64)
                }
                PrefKind::Switch | PrefKind::Flag(_) => {
                    let mut b = v != 0.0;
                    switch(ui, &mut b).changed().then_some(f64::from(u8::from(b)))
                }
                PrefKind::Whole => {
                    let mut i = v.round() as i32;
                    integer(ui, &mut i, 0.25, p.unit).changed().then_some(f64::from(i))
                }
                PrefKind::Angle => {
                    let mut f = v as f32;
                    number(ui, &mut f, 0.1, "\u{b0}").changed().then_some(f64::from(f))
                }
                PrefKind::Float => {
                    let mut f = v as f32;
                    number(ui, &mut f, 0.01, p.unit).changed().then_some(f64::from(f))
                }
                PrefKind::Color => {
                    let c = v as u32;
                    let mut c32 = Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
                    color(ui, &mut c32).changed().then(|| {
                        let [r, gr, b, _] = c32.to_srgba_unmultiplied();
                        f64::from(
                            (c & 0xff00_0000) | (u32::from(r) << 16) | (u32::from(gr) << 8) | u32::from(b),
                        )
                    })
                }
            }
        });
        if let Some(Some(next)) = changed {
            set_value(&settings, ctx, p, next);
            self.save_pending = true;
        }
        if live && g.was_reset() {
            set_value(&settings, ctx, p, default);
            self.save_pending = true;
        }
    }

    /// The groups areas added to Preferences.
    fn groups(&self) -> Vec<PrefGroup> {
        self.editor.services.get::<PrefGroups>().map(|g| g.0.clone()).unwrap_or_default()
    }

    /// The pages of the nav: the window's own, then those the areas' groups bring.
    fn pages(&self) -> Vec<(&'static str, Icon)> {
        let mut out: Vec<(&'static str, Icon)> = PAGES.to_vec();
        for g in self.groups() {
            if !out.iter().any(|(n, _)| *n == g.page) {
                // before Keyboard, which stays last
                let at = out.len() - 1;
                out.insert(at, (g.page, g.icon));
            }
        }
        out
    }

    /// The rows of one page as cards: its own rows, then a card per section, then the groups the
    /// areas added to it, each in the order given.
    fn draw_page_cards(&mut self, ui: &mut Ui, page: &str, ctx: &egui::Context) {
        let rows: Vec<&'static Pref> = PREFS
            .iter()
            .filter(|p| p.parts().0 == page && (self.search.is_empty() || p.matches(&self.search)))
            .collect();
        let own: Vec<&'static Pref> = rows.iter().filter(|p| p.parts().1.is_empty()).copied().collect();
        if !own.is_empty() {
            gz_ui::card::card(ui, &format!("prefs.{page}"), "", gz_ui::Tone::Muted, false, |ui| {
                PropertyGrid::new("preferences").show(ui, |g| {
                    for p in &own {
                        self.draw_row(g, p, 0, ctx);
                    }
                });
            });
            ui.add_space(gz_ui::metrics::CARD_GAP - 4.0);
        }
        let mut sections: Vec<&'static str> = Vec::new();
        for p in &rows {
            let s = p.parts().1;
            if !s.is_empty() && !sections.contains(&s) {
                sections.push(s);
            }
        }
        for section in sections {
            gz_ui::card::card(
                ui,
                &format!("prefs.{page}.{section}"),
                section,
                gz_ui::Tone::Muted,
                false,
                |ui| {
                    PropertyGrid::new("preferences").show(ui, |g| {
                        for p in rows.iter().filter(|p| p.parts().1 == section) {
                            self.draw_row(g, p, 0, ctx);
                        }
                    });
                },
            );
            ui.add_space(gz_ui::metrics::CARD_GAP - 4.0);
        }
        for group in self.groups().into_iter().filter(|g| g.page == page) {
            let shown: Vec<&PrefRow> = group
                .rows
                .iter()
                .filter(|r| {
                    self.search.is_empty()
                        || gz_ui::search::matches(
                            &self.search,
                            &format!("{} {} {}", group.page, group.section, r.label),
                        )
                })
                .collect();
            if shown.is_empty() {
                continue;
            }
            let note = group.note.and_then(|f| f(&self.editor));
            gz_ui::card::card(
                ui,
                &format!("prefs.{page}.{}", group.section),
                group.section,
                gz_ui::Tone::Muted,
                false,
                |ui| {
                    PropertyGrid::new("preferences").show(ui, |g| match &note {
                        // the rows mean nothing here: one sentence says why
                        Some((label, text)) => {
                            g.row(label, text, PropFlags::READ_ONLY, 0, |ui| {
                                gz_ui::grid::read_only(ui, text, Clip::End);
                            });
                        }
                        None => {
                            for r in shown {
                                self.draw_group_row(g, r, ctx);
                            }
                        }
                    });
                },
            );
            ui.add_space(gz_ui::metrics::CARD_GAP - 4.0);
        }
    }

    /// A row an area added: a setting of the settings file edited as its type says, a switch the
    /// area keeps, a value to read, or a button with what it last found.
    fn draw_group_row(&mut self, g: &mut Grid<'_>, r: &PrefRow, ctx: &egui::Context) {
        let editor = self.editor.clone();
        let (d, unit, choices) = match r.kind {
            RowKind::Setting { def, unit, choices } => (def, unit, choices),
            RowKind::Switch { get, set, default } => {
                let mut on = get(&editor);
                let mut flags = PropFlags::INLINE;
                if on != default {
                    flags = flags | PropFlags::MODIFIED;
                }
                if g.row(r.label, r.help, flags, 0, |ui| switch(ui, &mut on).changed()) == Some(true) {
                    set(&editor, on);
                    ctx.request_repaint();
                }
                if g.was_reset() {
                    set(&editor, default);
                }
                return;
            }
            RowKind::ReadOnly { value } => {
                let v = value(&editor);
                g.row(r.label, r.help, PropFlags::READ_ONLY, 0, |ui| {
                    gz_ui::grid::read_only(ui, &v, Clip::End)
                });
                return;
            }
            RowKind::Button { text: label, run, enabled, status } => {
                let on = enabled(&editor);
                let sentence = status(&editor);
                g.row(r.label, r.help, PropFlags::NONE, 0, |ui| {
                    ui.horizontal(|ui| {
                        if gz_ui::Button::new(label).enabled(on.on(), on.why()).show(ui).clicked() {
                            run(&editor);
                        }
                        ui.add_space(8.0);
                        text::secondary(ui, &sentence);
                    });
                });
                return;
            }
        };
        let settings = self.settings.clone();
        let v = settings.pref(d.key).unwrap_or(d.default);
        let switch_like = d.ty == PrefType::Bool;
        let mut flags = if switch_like { PropFlags::INLINE } else { PropFlags::NONE };
        if (v - d.default).abs() > 1e-9 {
            flags = flags | PropFlags::MODIFIED;
        }
        let changed = g.row(r.label, r.help, flags, 0, |ui| {
            ui.set_max_width(em(16.0).min(ui.available_width()));
            if !choices.is_empty() {
                let mut i = v.round().max(0.0) as usize;
                return combo(ui, d.key, &mut i, choices).then_some(i as f64);
            }
            match d.ty {
                PrefType::Bool => {
                    let mut b = v != 0.0;
                    switch(ui, &mut b).changed().then_some(f64::from(u8::from(b)))
                }
                PrefType::U32 => {
                    let mut i = v.round() as i32;
                    integer(ui, &mut i, 0.25, unit).changed().then_some(f64::from(i))
                }
                PrefType::Angle => {
                    let mut f = v as f32;
                    number(ui, &mut f, 0.1, "\u{b0}").changed().then_some(f64::from(f))
                }
                PrefType::Float => {
                    let mut f = v as f32;
                    number(ui, &mut f, 0.01, unit).changed().then_some(f64::from(f))
                }
                PrefType::Color => {
                    let c = v as u32;
                    let mut c32 = Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
                    color(ui, &mut c32).changed().then(|| {
                        let [rr, gg, bb, _] = c32.to_srgba_unmultiplied();
                        f64::from(
                            (c & 0xff00_0000) | (u32::from(rr) << 16) | (u32::from(gg) << 8) | u32::from(bb),
                        )
                    })
                }
            }
        });
        let next = match changed {
            Some(Some(n)) => Some(n),
            _ if g.was_reset() => Some(d.default),
            _ => None,
        };
        if let Some(n) = next {
            let n = if d.hi > d.lo { n.clamp(d.lo, d.hi) } else { n };
            settings.set_pref(d.key, n);
            self.save_pending = true;
            ctx.request_repaint();
        }
    }

    /// The page: its title, the rows that scroll, the help of the row under the mouse.
    fn draw_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let searching = !self.search.is_empty();
        let keyboard = !searching && self.page == KEYBOARD;
        // a page is a column of cards, 760 pt at most
        let width = ui.available_width().clamp(1.0, gz_ui::metrics::PAGE_W);
        let title = if searching { "Search Results" } else { self.page.as_str() };
        let r = Rect::from_min_size(ui.cursor().min, Vec2::new(width, Font::Title.line()));
        let mut t = ui.new_child(UiBuilder::new().max_rect(r));
        text::label(&mut t, title, Font::Title, p.text, Clip::End);
        ui.allocate_rect(r, egui::Sense::hover());
        ui.add_space(6.0);
        let footer_h = if keyboard { 0.0 } else { 8.0 * 2.0 + Font::Small.line() * 2.0 + 2.0 + 4.0 };
        let height = (ui.available_height() - footer_h).max(80.0);
        let area = Rect::from_min_size(ui.cursor().min, Vec2::new(width, height));
        let mut rows = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-app.prefs.rows")
                .max_rect(area)
                .layout(Layout::top_down(Align::Min)),
        );
        ScrollArea::vertical()
            .id_salt(("gz-ui-app.prefs.scroll", self.page.as_str(), searching))
            .auto_shrink([false, false])
            .show(&mut rows, |ui| {
                if keyboard {
                    let ed = self.editor.clone();
                    self.keyboard.draw(ui, &ed, "");
                } else if searching {
                    let pages: Vec<&'static str> = PAGES
                        .iter()
                        .map(|(n, _)| *n)
                        .filter(|n| {
                            *n != KEYBOARD
                                && PREFS.iter().any(|p| p.parts().0 == *n && p.matches(&self.search))
                        })
                        .collect();
                    let keys = self.keyboard.matches(&self.search);
                    if pages.is_empty() && keys == 0 {
                        gz_ui::state::empty_state(
                            ui,
                            Icon::Search,
                            "No settings match",
                            "No setting or command holds every word of the search.",
                            None,
                        );
                    }
                    for page in pages {
                        let p = palette(&ctx);
                        text::label(ui, page, Font::Strong, p.text_secondary, Clip::End);
                        ui.add_space(4.0);
                        self.draw_page_cards(ui, page, &ctx);
                    }
                    if keys > 0 {
                        ui.add_space(8.0);
                        if gz_ui::section::Section::new("prefs.keyboard_results", KEYBOARD).show(ui).open {
                            let (ed, search) = (self.editor.clone(), self.search.clone());
                            self.keyboard.draw(ui, &ed, &search);
                        }
                    }
                } else {
                    let page = self.page.clone();
                    self.draw_page_cards(ui, &page, &ctx);
                }
            });
        ui.allocate_rect(area, egui::Sense::hover());
        if !keyboard {
            ui.add_space(4.0);
            let fr = Rect::from_min_size(ui.cursor().min, Vec2::new(width, footer_h - 4.0));
            let mut f = ui.new_child(UiBuilder::new().max_rect(fr).layout(Layout::top_down(Align::Min)));
            help_footer(&mut f);
            ui.allocate_rect(fr, egui::Sense::hover());
        }
    }
}

impl Document for PreferencesDoc {
    fn id(&self) -> &str {
        "preferences"
    }
    fn kind(&self) -> &'static str {
        "preferences"
    }
    fn label(&self) -> String {
        "Preferences".to_owned()
    }
    fn icon(&self) -> Icon {
        Icon::Settings
    }
    fn tooltip(&self) -> Option<String> {
        Some("The editor's settings. A change takes effect at once and is saved.".to_owned())
    }

    fn toolbar_status(&self) -> Option<String> {
        Some("A change takes effect at once and is saved".to_owned())
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        // preferences action=show: a page and a search
        if let Some((page, search)) = PrefsNav::of(cx.editor).take() {
            if let Some(page) = page_named(&page) {
                self.set_page(page);
            }
            self.search = search;
            self.keyboard.reload(&self.editor);
        }
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let nav_w = 200.0;
        let gap = 16.0;
        let nav = Rect::from_min_max(full.min, pos2((full.min.x + nav_w).min(full.max.x), full.max.y));
        let mut left = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-app.prefs.nav")
                .max_rect(nav)
                .layout(Layout::top_down(Align::Min)),
        );
        self.draw_nav(&mut left, nav_w);
        let page = Rect::from_min_max(pos2((nav.max.x + gap).min(full.max.x), full.min.y), full.max);
        let mut right = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-app.prefs.page")
                .max_rect(page)
                .layout(Layout::top_down(Align::Min)),
        );
        self.draw_page(&mut right);
        ui.allocate_rect(full, egui::Sense::hover());
        // a shortcut being recorded takes the keys of the next frame, before any action does
        if self.keyboard.recording() {
            cx.requests.capture_keys();
        }

        // written once the field being typed in or dragged lets go, not on every step
        if self.save_pending && !ctx.egui_wants_keyboard_input() && ctx.dragged_id().is_none() {
            self.save();
        }
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        self.keyboard.reload(&self.editor);
    }

    fn on_deactivate(&mut self, _cx: &mut DocCx<'_>) {
        if self.save_pending {
            self.save();
        }
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        self.save();
    }

    fn mcp_state(&self) -> serde_json::Value {
        serde_json::json!({ "page": self.page, "search": self.search })
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.document("preferences", |editor, req| {
        if let Some(arg) = req.arg.as_deref() {
            PrefsNav::of(editor).ask(arg);
        }
        Ok(Box::new(PreferencesDoc::new(editor)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_names_its_page() {
        for p in PREFS {
            let (page, _, label) = p.parts();
            assert!(PAGES.iter().any(|(n, _)| *n == page), "{}", p.path);
            assert!(!label.is_empty());
            assert!(!p.help.is_empty(), "{}", p.path);
        }
    }

    #[test]
    fn the_shells_keys_are_live() {
        // every preference the shell's settings define has a row
        for d in &gz_app::settings::PREFS {
            assert!(PREFS.iter().any(|p| p.key == d.key), "{}", d.key);
        }
        let fov = PREFS.iter().find(|p| p.key == "view_fov");
        assert!(fov.is_some_and(|p| p.live() && p.parts() == ("Viewport", "", "Field of View")));
        // and every row is kept by the settings file: none waits for a part of the editor
        let dead: Vec<&str> = PREFS.iter().filter(|p| !p.live()).map(|p| p.path).collect();
        assert!(dead.is_empty(), "{dead:?}");
    }

    #[test]
    fn values_read_as_the_rows_show_them() {
        let p = pref("General\\Undo Steps", "scene_undo_level", PrefKind::Whole, "x", "");
        assert_eq!(value_text(&p, 125.0), "125");
        let p = pref("Viewport\\Fog\\Color", "fog_color", PrefKind::Color, "x", "");
        assert_eq!(value_text(&p, f64::from(0x0055_5555u32)), "#555555");
        let p = flag("Objects\\Display\\Show Pivot", 1, "x");
        assert_eq!(value_text(&p, 0.0), "Off");
    }

    #[test]
    fn show_takes_a_page_and_a_search() {
        let nav = PrefsNav::default();
        nav.ask("keyboard|undo");
        assert_eq!(nav.take(), Some(("keyboard".to_owned(), "undo".to_owned())));
        assert_eq!(page_named("keyboard"), Some("Keyboard"));
        assert_eq!(page_named("nope"), None);
    }
}
