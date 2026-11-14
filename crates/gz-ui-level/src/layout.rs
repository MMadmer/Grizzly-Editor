//! The Level dock as data: the size classes, the panels, the four side slots around the
//! viewport, the layout each class builds and when a layout counts as customised.
//!
//! Wide (1400 x 640 pt and up): Place (the mode's panel) 260 left, a 340 right column (the
//! Outliner, 40 %, over Details + Level Settings), a 240 strip under the viewport only with the
//! Content Browser and the Output Log. Compact: Place 240, the right column 300, the browser and
//! the log in drawers.
//!
//! What persists goes into the kit's layout store (`level.*` keys), which the editor's
//! settings file keeps: the sizes (the splitters' keys), and per panel its slot and whether it
//! is open, per slot the tab in front, the class the layout was built for and the custom flag.
//! A panel moved out into a window of its own (Preferences > General > Allow Panels Outside the
//! Window) keeps its slot to come back to; whether it is out and where its window stood (pt, the
//! screen's) are `level.float.<panel>` and `level.float.<panel>.x` / `.y` / `.w` / `.h`.

use egui::Context;
use gz_ui::layout as store;

/// The two layouts of the Level tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeClass {
    Wide,
    Compact,
}

impl SizeClass {
    /// The class of a window of this size, pt.
    #[must_use]
    pub fn of(width: f32, height: f32) -> Self {
        if width >= 1400.0 && height >= 640.0 { SizeClass::Wide } else { SizeClass::Compact }
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            SizeClass::Wide => "wide",
            SizeClass::Compact => "compact",
        }
    }

    fn code(self) -> f32 {
        match self {
            SizeClass::Wide => 1.0,
            SizeClass::Compact => 2.0,
        }
    }

    fn from_code(v: f32) -> Option<Self> {
        match v.round() as i32 {
            1 => Some(SizeClass::Wide),
            2 => Some(SizeClass::Compact),
            _ => None,
        }
    }
}

/// The panels the dock holds (the viewport is its centre, not one of them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelId {
    Place,
    Outliner,
    Details,
    LevelSettings,
    ContentBrowser,
    OutputLog,
    /// The build's problems, beside the Output Log.
    Problems,
}

/// How many panels the dock holds.
pub const PANELS: usize = 7;

impl PanelId {
    pub const ALL: [PanelId; PANELS] = [
        PanelId::Place,
        PanelId::Outliner,
        PanelId::Details,
        PanelId::LevelSettings,
        PanelId::ContentBrowser,
        PanelId::OutputLog,
        PanelId::Problems,
    ];

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            PanelId::Place => 0,
            PanelId::Outliner => 1,
            PanelId::Details => 2,
            PanelId::LevelSettings => 3,
            PanelId::ContentBrowser => 4,
            PanelId::OutputLog => 5,
            PanelId::Problems => 6,
        }
    }

    /// The MCP id.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            PanelId::Place => "place",
            PanelId::Outliner => "world_outliner",
            PanelId::Details => "details",
            PanelId::LevelSettings => "level_settings",
            PanelId::ContentBrowser => "content_browser",
            PanelId::OutputLog => "output_log",
            PanelId::Problems => "problems",
        }
    }

    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// The window the contract names.
    #[must_use]
    pub fn window(self) -> &'static str {
        gz_app::contract::panel_window(self.id())
    }

    /// The tab's label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PanelId::Place => "Place",
            PanelId::Outliner => "World Outliner",
            PanelId::Details => "Details",
            PanelId::LevelSettings => "Level Settings",
            PanelId::ContentBrowser => "Content Browser",
            PanelId::OutputLog => "Output Log",
            PanelId::Problems => "Problems",
        }
    }

    /// The minimum size of the panel, pt (UI_DESIGN section 2, "Minimum sizes").
    #[must_use]
    pub fn min_size(self) -> (f32, f32) {
        match self {
            PanelId::Place => (240.0, 200.0),
            PanelId::Outliner => (220.0, 160.0),
            PanelId::Details | PanelId::LevelSettings => (280.0, 200.0),
            PanelId::ContentBrowser => (360.0, 150.0),
            PanelId::OutputLog => (320.0, 110.0),
            // it shares the Compact right column, 300 pt, with Details
            PanelId::Problems => (280.0, 110.0),
        }
    }

    /// It answers its own keys: the scene's bindings stay away while it has the focus.
    #[must_use]
    pub fn owns_keys(self) -> bool {
        matches!(self, PanelId::ContentBrowser | PanelId::OutputLog)
    }

    /// Where the drawer that shows the same object is (the Compact class).
    #[must_use]
    pub fn drawer(self) -> Option<gz_app::Drawer> {
        match self {
            PanelId::ContentBrowser => Some(gz_app::Drawer::Content),
            PanelId::OutputLog => Some(gz_app::Drawer::Log),
            _ => None,
        }
    }
}

/// The places a panel docks into around the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Left,
    RightTop,
    RightBottom,
    /// Under the viewport only.
    Bottom,
}

impl Slot {
    pub const ALL: [Slot; 4] = [Slot::Left, Slot::RightTop, Slot::RightBottom, Slot::Bottom];

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Slot::Left => 0,
            Slot::RightTop => 1,
            Slot::RightBottom => 2,
            Slot::Bottom => 3,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Slot::Left => "Left",
            Slot::RightTop => "Right, Top",
            Slot::RightBottom => "Right, Bottom",
            Slot::Bottom => "Under the Viewport",
        }
    }

    fn code(self) -> f32 {
        (self.index() + 1) as f32
    }

    fn from_code(v: f32) -> Option<Self> {
        Self::ALL.get((v.round() as i64 - 1).clamp(-1, 8) as usize).copied().filter(|_| v >= 0.5)
    }
}

/// The splitters' keys: the sizes of the side slots, pt.
pub const KEY_LEFT: &str = "level.left_w";
pub const KEY_RIGHT: &str = "level.right_w";
pub const KEY_BOTTOM: &str = "level.bottom_h";
pub const KEY_RIGHT_TOP: &str = "level.right_top_h";
const KEY_CLASS: &str = "level.class";
const KEY_CUSTOM: &str = "level.custom";

/// The sizes a class builds with, pt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    /// The Outliner's share of the right column until the author drags it.
    pub right_top_frac: f32,
}

impl Sizes {
    #[must_use]
    pub fn of(class: SizeClass) -> Self {
        match class {
            SizeClass::Wide => Sizes { left: 260.0, right: 340.0, bottom: 240.0, right_top_frac: 0.40 },
            SizeClass::Compact => Sizes { left: 240.0, right: 300.0, bottom: 240.0, right_top_frac: 0.40 },
        }
    }
}

/// The arrangement of the panels.
#[derive(Clone, Debug, PartialEq)]
pub struct DockLayout {
    /// The class the layout was built for.
    pub class: SizeClass,
    /// The author moved a panel or a splitter: crossing a class keeps the layout.
    pub custom: bool,
    /// Where each panel docks (`PanelId::index`); `None` lives in a drawer only.
    pub slot: [Option<Slot>; PANELS],
    pub open: [bool; PANELS],
    /// The tab in front of each slot.
    pub front: [Option<PanelId>; 4],
    /// Out in a window of its own.
    pub floating: [bool; PANELS],
    /// Where its window stood: x, y, w, h in pt of the screen.
    pub float_rect: [Option<[f32; 4]>; PANELS],
}

impl DockLayout {
    /// The layout a class builds, with every panel of the class open; the
    /// open state of a panel the class has no room for stays as it was.
    #[must_use]
    pub fn built(class: SizeClass, before: Option<&DockLayout>) -> Self {
        let wide = class == SizeClass::Wide;
        let strip = if wide { Some(Slot::Bottom) } else { None };
        let slot = [
            Some(Slot::Left),
            Some(Slot::RightTop),
            Some(Slot::RightBottom),
            Some(Slot::RightBottom),
            strip,
            strip,
            // the build's problems: beside the log, or with Details where the log is a drawer
            Some(if wide { Slot::Bottom } else { Slot::RightBottom }),
        ];
        let mut open = [true; PANELS];
        if !wide {
            for p in [PanelId::ContentBrowser, PanelId::OutputLog] {
                open[p.index()] = before.is_some_and(|b| b.open[p.index()]);
            }
        }
        DockLayout {
            class,
            custom: false,
            slot,
            open,
            front: [
                Some(PanelId::Place),
                Some(PanelId::Outliner),
                Some(PanelId::Details),
                if wide { Some(PanelId::ContentBrowser) } else { None },
            ],
            floating: [false; PANELS],
            float_rect: before.map_or([None; PANELS], |b| b.float_rect),
        }
    }

    #[must_use]
    pub fn is_floating(&self, p: PanelId) -> bool {
        self.floating.get(p.index()).copied().unwrap_or(false)
    }

    /// The open panels out in windows of their own.
    #[must_use]
    pub fn floating_panels(&self) -> Vec<PanelId> {
        PanelId::ALL.into_iter().filter(|p| self.is_open(*p) && self.is_floating(*p)).collect()
    }

    /// Moves a panel out into a window of its own (`at`: where, pt of the screen; else where it
    /// stood last).
    pub fn float_panel(&mut self, p: PanelId, at: Option<[f32; 2]>) {
        if let Some(o) = self.open.get_mut(p.index()) {
            *o = true;
        }
        if let Some(f) = self.floating.get_mut(p.index()) {
            *f = true;
        }
        if let (Some([x, y]), Some(r)) = (at, self.float_rect.get_mut(p.index())) {
            let (w, h) = r.map_or_else(|| (p.min_size().0 + 80.0, p.min_size().1 + 200.0), |r| (r[2], r[3]));
            *r = Some([x, y, w, h]);
        }
        self.custom = true;
    }

    /// Brings a panel back into its slot, in front there.
    pub fn dock_panel(&mut self, p: PanelId) {
        if let Some(f) = self.floating.get_mut(p.index()) {
            *f = false;
        }
        self.open_panel(p);
    }

    /// Where a floating panel's window stands now (pt of the screen).
    pub fn set_float_rect(&mut self, p: PanelId, r: [f32; 4]) {
        if let Some(slot) = self.float_rect.get_mut(p.index()) {
            *slot = Some(r);
        }
    }

    #[must_use]
    pub fn slot_of(&self, p: PanelId) -> Option<Slot> {
        self.slot.get(p.index()).copied().flatten()
    }

    #[must_use]
    pub fn is_open(&self, p: PanelId) -> bool {
        self.open.get(p.index()).copied().unwrap_or(false)
    }

    /// The open panels of a slot, in their order.
    #[must_use]
    pub fn panels_in(&self, s: Slot) -> Vec<PanelId> {
        PanelId::ALL
            .into_iter()
            .filter(|p| self.is_open(*p) && !self.is_floating(*p) && self.slot_of(*p) == Some(s))
            .collect()
    }

    /// Whether the slot is drawn: it holds an open panel, and the strip under the viewport only
    /// in the Wide class or in a customised layout.
    #[must_use]
    pub fn slot_shown(&self, s: Slot, class: SizeClass) -> bool {
        if s == Slot::Bottom && class != SizeClass::Wide && !self.custom {
            return false;
        }
        !self.panels_in(s).is_empty()
    }

    /// The panel in front of a slot: the remembered one while it is there, else the first.
    #[must_use]
    pub fn front_of(&self, s: Slot) -> Option<PanelId> {
        let list = self.panels_in(s);
        let want = self.front.get(s.index()).copied().flatten();
        want.filter(|p| list.contains(p)).or_else(|| list.first().copied())
    }

    /// Whether a panel is on screen: open, in a shown slot, and the tab in front.
    #[must_use]
    pub fn visible(&self, p: PanelId, class: SizeClass) -> bool {
        match self.slot_of(p) {
            Some(s) => self.is_open(p) && self.slot_shown(s, class) && self.front_of(s) == Some(p),
            None => false,
        }
    }

    /// Opens a panel and brings its tab to the front of its slot.
    pub fn open_panel(&mut self, p: PanelId) {
        if let Some(o) = self.open.get_mut(p.index()) {
            *o = true;
        }
        if let Some(s) = self.slot_of(p)
            && let Some(f) = self.front.get_mut(s.index())
        {
            *f = Some(p);
        }
    }

    pub fn close_panel(&mut self, p: PanelId) {
        if let Some(o) = self.open.get_mut(p.index()) {
            *o = false;
        }
    }

    /// Moves a panel into another slot, in front there: the layout is the author's now.
    pub fn move_panel(&mut self, p: PanelId, to: Slot) {
        if self.slot_of(p) == Some(to) {
            return;
        }
        if let Some(s) = self.slot.get_mut(p.index()) {
            *s = Some(to);
        }
        self.open_panel(p);
        self.custom = true;
    }

    /// Whether the panels sit where the class builds them.
    #[must_use]
    pub fn placement_is_built(&self) -> bool {
        let built = DockLayout::built(self.class, None);
        built.slot == self.slot && !self.floating.iter().any(|f| *f)
    }

    /// Reads the layout from the store; `None` when none was saved (or it is broken).
    #[must_use]
    pub fn load(ctx: &Context) -> Option<Self> {
        let class = SizeClass::from_code(store::get(ctx, KEY_CLASS, 0.0))?;
        let mut out = DockLayout::built(class, None);
        out.custom = store::get(ctx, KEY_CUSTOM, 0.0) >= 0.5;
        for p in PanelId::ALL {
            let i = p.index();
            if store::has(ctx, &slot_key(p)) {
                out.slot[i] = Slot::from_code(store::get(ctx, &slot_key(p), 0.0));
            }
            if store::has(ctx, &open_key(p)) {
                out.open[i] = store::get(ctx, &open_key(p), 1.0) >= 0.5;
            }
        }
        for s in Slot::ALL {
            if store::has(ctx, &front_key(s)) {
                let v = store::get(ctx, &front_key(s), 0.0).round() as i64;
                out.front[s.index()] = usize::try_from(v - 1).ok().and_then(|i| PanelId::ALL.get(i).copied());
            }
        }
        // panels stay in the window while they may not leave it
        let may_float = gz_app::floating_panels(ctx);
        for p in PanelId::ALL {
            let i = p.index();
            out.floating[i] = may_float && store::get(ctx, &float_key(p), 0.0) >= 0.5;
            let part = |k: &str| {
                let key = format!("{}.{k}", float_key(p));
                store::has(ctx, &key).then(|| store::get(ctx, &key, 0.0))
            };
            if let (Some(x), Some(y), Some(w), Some(h)) = (part("x"), part("y"), part("w"), part("h")) {
                out.float_rect[i] = Some([x, y, w, h]);
            }
        }
        Some(out)
    }

    /// Writes the layout into the store (only what changed is marked dirty there).
    pub fn save(&self, ctx: &Context) {
        set_if(ctx, KEY_CLASS, self.class.code());
        set_if(ctx, KEY_CUSTOM, if self.custom { 1.0 } else { 0.0 });
        for p in PanelId::ALL {
            set_if(ctx, &slot_key(p), self.slot_of(p).map_or(0.0, Slot::code));
            set_if(ctx, &open_key(p), if self.is_open(p) { 1.0 } else { 0.0 });
        }
        for s in Slot::ALL {
            set_if(
                ctx,
                &front_key(s),
                self.front.get(s.index()).copied().flatten().map_or(0.0, |p| (p.index() + 1) as f32),
            );
        }
        for p in PanelId::ALL {
            let i = p.index();
            if self.is_floating(p) || store::has(ctx, &float_key(p)) {
                set_if(ctx, &float_key(p), if self.is_floating(p) { 1.0 } else { 0.0 });
            }
            if let Some(Some(r)) = self.float_rect.get(i) {
                for (k, v) in ["x", "y", "w", "h"].iter().zip(r) {
                    set_if(ctx, &format!("{}.{k}", float_key(p)), v.round());
                }
            }
        }
    }
}

fn set_if(ctx: &Context, key: &str, v: f32) {
    if !store::has(ctx, key) || store::get(ctx, key, v) != v {
        store::set(ctx, key, v);
    }
}

fn slot_key(p: PanelId) -> String {
    format!("level.slot.{}", p.id())
}

fn open_key(p: PanelId) -> String {
    format!("level.open.{}", p.id())
}

fn float_key(p: PanelId) -> String {
    format!("level.float.{}", p.id())
}

fn front_key(s: Slot) -> String {
    format!("level.front.{}", s.index())
}

/// Sizes rounded to 8 pt: a splitter a few points off its place is not a layout of the author's.
#[must_use]
pub fn round8(v: f32) -> i32 {
    (v / 8.0).round() as i32
}

/// Whether a stored size differs from the class's default (rounded to 8 pt): the author
/// dragged a splitter away from where the build put it.
#[must_use]
pub fn size_customised(stored: Option<f32>, default: f32) -> bool {
    stored.is_some_and(|v| round8(v) != round8(default))
}

/// Forgets the sizes: the splitters fall back to the class's defaults.
pub fn reset_sizes(ctx: &Context) {
    for k in [KEY_LEFT, KEY_RIGHT, KEY_BOTTOM, KEY_RIGHT_TOP] {
        store::reset(ctx, k);
    }
}

/// Whether the stored sizes are the author's for this class.
#[must_use]
pub fn sizes_customised(ctx: &Context, class: SizeClass) -> bool {
    let d = Sizes::of(class);
    let stored = |k: &str| store::has(ctx, k).then(|| store::get(ctx, k, 0.0));
    size_customised(stored(KEY_LEFT), d.left)
        || size_customised(stored(KEY_RIGHT), d.right)
        || size_customised(stored(KEY_BOTTOM), d.bottom)
        || store::has(ctx, KEY_RIGHT_TOP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert_eq!(SizeClass::of(1707.0, 912.0), SizeClass::Wide);
        assert_eq!(SizeClass::of(1280.0, 720.0), SizeClass::Compact);
        assert_eq!(SizeClass::of(1400.0, 639.0), SizeClass::Compact);
        assert_eq!(SizeClass::from_code(SizeClass::Compact.code()), Some(SizeClass::Compact));
    }

    #[test]
    fn the_wide_build_has_every_panel() {
        let l = DockLayout::built(SizeClass::Wide, None);
        assert!(PanelId::ALL.iter().all(|p| l.is_open(*p)));
        assert_eq!(l.panels_in(Slot::RightBottom), vec![PanelId::Details, PanelId::LevelSettings]);
        assert_eq!(l.front_of(Slot::Bottom), Some(PanelId::ContentBrowser));
        assert!(l.visible(PanelId::Details, SizeClass::Wide));
        assert!(!l.visible(PanelId::LevelSettings, SizeClass::Wide));
    }

    #[test]
    fn compact_keeps_the_strip_in_drawers() {
        let wide = DockLayout::built(SizeClass::Wide, None);
        let l = DockLayout::built(SizeClass::Compact, Some(&wide));
        assert_eq!(l.slot_of(PanelId::OutputLog), None);
        assert!(l.is_open(PanelId::OutputLog));
        assert!(!l.slot_shown(Slot::Bottom, SizeClass::Compact));
        assert!(l.placement_is_built());
    }

    #[test]
    fn moving_a_panel_customises() {
        let mut l = DockLayout::built(SizeClass::Wide, None);
        l.move_panel(PanelId::OutputLog, Slot::Left);
        assert!(l.custom);
        assert_eq!(l.front_of(Slot::Left), Some(PanelId::OutputLog));
        assert!(!l.placement_is_built());
    }

    #[test]
    fn a_floating_panel_leaves_its_slot_and_comes_back() {
        let mut l = DockLayout::built(SizeClass::Wide, None);
        l.float_panel(PanelId::Details, Some([100.0, 50.0]));
        assert!(l.is_floating(PanelId::Details));
        assert_eq!(l.panels_in(Slot::RightBottom), vec![PanelId::LevelSettings]);
        assert_eq!(l.floating_panels(), vec![PanelId::Details]);
        assert!(!l.placement_is_built());
        l.dock_panel(PanelId::Details);
        assert_eq!(l.front_of(Slot::RightBottom), Some(PanelId::Details));
        assert!(l.float_rect[PanelId::Details.index()].is_some());
    }

    #[test]
    fn sizes_compare_at_8_pt() {
        assert!(!size_customised(Some(263.0), 260.0));
        assert!(size_customised(Some(280.0), 260.0));
        assert!(!size_customised(None, 260.0));
    }

    #[test]
    fn slot_codes_round_trip() {
        for s in Slot::ALL {
            assert_eq!(Slot::from_code(s.code()), Some(s));
        }
        assert_eq!(Slot::from_code(0.0), None);
    }
}
