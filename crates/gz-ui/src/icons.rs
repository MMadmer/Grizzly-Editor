//! The editor's vector icon set (docs/UI_DESIGN.md, section 3).
//!
//! Every glyph is geometry on a 16-unit grid, built once and only scaled when drawn, so it is
//! exact at any size and DPI. [`draw`] takes a square, [`draw_in`] centres one in a rectangle;
//! the colour is the caller's (the state tones live in the kit). [`Icon::name`] is the stable
//! name MCP answers and QA read. [`sheet`] is the developer's view of the whole set.

mod geometry;
mod paint;

use egui::{Color32, Painter, Rect, Ui};

use crate::theme::palette;

/// Every icon of the kit; [`Icon::name`] is the stable name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    None,
    Minimize,
    Maximize,
    Restore,
    Close,
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Caret,
    More,
    Overflow,
    Menu,
    Dot,
    Grip,
    Pin,
    External,
    SidebarLeft,
    SidebarRight,
    PanelBottom,
    New,
    Open,
    Save,
    SaveAll,
    Reload,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Rename,
    Delete,
    Plus,
    Minus,
    Clear,
    Check,
    Search,
    Filter,
    Sort,
    SortAsc,
    SortDesc,
    Settings,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Link,
    Unlink,
    Star,
    StarFilled,
    Bookmark,
    BookmarkFilled,
    BookmarkPrev,
    BookmarkNext,
    History,
    Back,
    Forward,
    Up,
    Home,
    Reveal,
    Info,
    Help,
    Keyboard,
    ExpandAll,
    CollapseAll,
    MoveUp,
    MoveDown,
    Globe,
    Key,
    Shield,
    Tag,
    Picture,
    Terminal,
    Refresh,
    Error,
    Warning,
    Success,
    Required,
    Spinner,
    Dirty,
    BuildCollision,
    BuildAIMap,
    BuildGameGraph,
    BuildAll,
    Play,
    PlayGame,
    PlayClean,
    Stop,
    ReloadConfigs,
    GameFolder,
    BuildMod,
    Package,
    Publish,
    Releases,
    Select,
    Place,
    Move,
    Rotate,
    Scale,
    SnapGrid,
    SnapAngle,
    SnapScale,
    SnapObject,
    FrameAll,
    FrameSelection,
    Perspective,
    Ortho,
    Lit,
    Unlit,
    Wireframe,
    Bones,
    Stats,
    ViewOptions,
    SceneObject,
    Light,
    SoundSource,
    SoundEnv,
    Glow,
    Shape,
    Spawn,
    Way,
    Sector,
    Portal,
    Group,
    Particles,
    DetailObjects,
    AIMap,
    Wallmark,
    FogVolume,
    Folder,
    FolderOpen,
    File,
    FileTexture,
    FileModel,
    FileSound,
    FileScript,
    FileConfig,
    FileXml,
    FileText,
    FileArchive,
    Level,
    Quest,
    Behaviour,
    Character,
    CharacterTemplate,
    PdaInterface,
    Flow,
    Wait,
    Condition,
    Variable,
    SpawnNpc,
    Dialog,
    Item,
    PdaMsg,
    World,
    Relation,
    Squad,
    Order,
    Timer,
    Trigger,
    End,
    Task,
    Role,
    Minimap,
    AutoLayout,
    Validate,
    FindInProject,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Zoom100,
    Person,
    Community,
    Rank,
    Money,
    Kit,
    Area,
    Transition,
    PlayerStart,
    Mode,
    Relations,
    Exclude,
    Aspect,
    Language,
    Tasks,
    Mcp,
    Log,
    Problems,
    Drawer,
}

impl Icon {
    /// Every icon but `None`, in enum order.
    pub const ALL: &'static [Icon] = &[
        Icon::Minimize,
        Icon::Maximize,
        Icon::Restore,
        Icon::Close,
        Icon::ChevronUp,
        Icon::ChevronDown,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Caret,
        Icon::More,
        Icon::Overflow,
        Icon::Menu,
        Icon::Dot,
        Icon::Grip,
        Icon::Pin,
        Icon::External,
        Icon::SidebarLeft,
        Icon::SidebarRight,
        Icon::PanelBottom,
        Icon::New,
        Icon::Open,
        Icon::Save,
        Icon::SaveAll,
        Icon::Reload,
        Icon::Undo,
        Icon::Redo,
        Icon::Cut,
        Icon::Copy,
        Icon::Paste,
        Icon::Duplicate,
        Icon::Rename,
        Icon::Delete,
        Icon::Plus,
        Icon::Minus,
        Icon::Clear,
        Icon::Check,
        Icon::Search,
        Icon::Filter,
        Icon::Sort,
        Icon::SortAsc,
        Icon::SortDesc,
        Icon::Settings,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Lock,
        Icon::Unlock,
        Icon::Link,
        Icon::Unlink,
        Icon::Star,
        Icon::StarFilled,
        Icon::Bookmark,
        Icon::BookmarkFilled,
        Icon::BookmarkPrev,
        Icon::BookmarkNext,
        Icon::History,
        Icon::Back,
        Icon::Forward,
        Icon::Up,
        Icon::Home,
        Icon::Reveal,
        Icon::Info,
        Icon::Help,
        Icon::Keyboard,
        Icon::ExpandAll,
        Icon::CollapseAll,
        Icon::MoveUp,
        Icon::MoveDown,
        Icon::Globe,
        Icon::Key,
        Icon::Shield,
        Icon::Tag,
        Icon::Picture,
        Icon::Terminal,
        Icon::Refresh,
        Icon::Error,
        Icon::Warning,
        Icon::Success,
        Icon::Required,
        Icon::Spinner,
        Icon::Dirty,
        Icon::BuildCollision,
        Icon::BuildAIMap,
        Icon::BuildGameGraph,
        Icon::BuildAll,
        Icon::Play,
        Icon::PlayGame,
        Icon::PlayClean,
        Icon::Stop,
        Icon::ReloadConfigs,
        Icon::GameFolder,
        Icon::BuildMod,
        Icon::Package,
        Icon::Publish,
        Icon::Releases,
        Icon::Select,
        Icon::Place,
        Icon::Move,
        Icon::Rotate,
        Icon::Scale,
        Icon::SnapGrid,
        Icon::SnapAngle,
        Icon::SnapScale,
        Icon::SnapObject,
        Icon::FrameAll,
        Icon::FrameSelection,
        Icon::Perspective,
        Icon::Ortho,
        Icon::Lit,
        Icon::Unlit,
        Icon::Wireframe,
        Icon::Bones,
        Icon::Stats,
        Icon::ViewOptions,
        Icon::SceneObject,
        Icon::Light,
        Icon::SoundSource,
        Icon::SoundEnv,
        Icon::Glow,
        Icon::Shape,
        Icon::Spawn,
        Icon::Way,
        Icon::Sector,
        Icon::Portal,
        Icon::Group,
        Icon::Particles,
        Icon::DetailObjects,
        Icon::AIMap,
        Icon::Wallmark,
        Icon::FogVolume,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::File,
        Icon::FileTexture,
        Icon::FileModel,
        Icon::FileSound,
        Icon::FileScript,
        Icon::FileConfig,
        Icon::FileXml,
        Icon::FileText,
        Icon::FileArchive,
        Icon::Level,
        Icon::Quest,
        Icon::Behaviour,
        Icon::Character,
        Icon::CharacterTemplate,
        Icon::PdaInterface,
        Icon::Flow,
        Icon::Wait,
        Icon::Condition,
        Icon::Variable,
        Icon::SpawnNpc,
        Icon::Dialog,
        Icon::Item,
        Icon::PdaMsg,
        Icon::World,
        Icon::Relation,
        Icon::Squad,
        Icon::Order,
        Icon::Timer,
        Icon::Trigger,
        Icon::End,
        Icon::Task,
        Icon::Role,
        Icon::Minimap,
        Icon::AutoLayout,
        Icon::Validate,
        Icon::FindInProject,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::ZoomFit,
        Icon::Zoom100,
        Icon::Person,
        Icon::Community,
        Icon::Rank,
        Icon::Money,
        Icon::Kit,
        Icon::Area,
        Icon::Transition,
        Icon::PlayerStart,
        Icon::Mode,
        Icon::Relations,
        Icon::Exclude,
        Icon::Aspect,
        Icon::Language,
        Icon::Tasks,
        Icon::Mcp,
        Icon::Log,
        Icon::Problems,
        Icon::Drawer,
    ];

    /// The stable lower-case name ("chevron_down") MCP answers and QA read.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Icon::None => "none",
            Icon::Minimize => "minimize",
            Icon::Maximize => "maximize",
            Icon::Restore => "restore",
            Icon::Close => "close",
            Icon::ChevronUp => "chevron_up",
            Icon::ChevronDown => "chevron_down",
            Icon::ChevronLeft => "chevron_left",
            Icon::ChevronRight => "chevron_right",
            Icon::Caret => "caret",
            Icon::More => "more",
            Icon::Overflow => "overflow",
            Icon::Menu => "menu",
            Icon::Dot => "dot",
            Icon::Grip => "grip",
            Icon::Pin => "pin",
            Icon::External => "external",
            Icon::SidebarLeft => "sidebar_left",
            Icon::SidebarRight => "sidebar_right",
            Icon::PanelBottom => "panel_bottom",
            Icon::New => "new",
            Icon::Open => "open",
            Icon::Save => "save",
            Icon::SaveAll => "save_all",
            Icon::Reload => "reload",
            Icon::Undo => "undo",
            Icon::Redo => "redo",
            Icon::Cut => "cut",
            Icon::Copy => "copy",
            Icon::Paste => "paste",
            Icon::Duplicate => "duplicate",
            Icon::Rename => "rename",
            Icon::Delete => "delete",
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::Clear => "clear",
            Icon::Check => "check",
            Icon::Search => "search",
            Icon::Filter => "filter",
            Icon::Sort => "sort",
            Icon::SortAsc => "sort_asc",
            Icon::SortDesc => "sort_desc",
            Icon::Settings => "settings",
            Icon::Eye => "eye",
            Icon::EyeOff => "eye_off",
            Icon::Lock => "lock",
            Icon::Unlock => "unlock",
            Icon::Link => "link",
            Icon::Unlink => "unlink",
            Icon::Star => "star",
            Icon::StarFilled => "star_filled",
            Icon::Bookmark => "bookmark",
            Icon::BookmarkFilled => "bookmark_filled",
            Icon::BookmarkPrev => "bookmark_prev",
            Icon::BookmarkNext => "bookmark_next",
            Icon::History => "history",
            Icon::Back => "back",
            Icon::Forward => "forward",
            Icon::Up => "up",
            Icon::Home => "home",
            Icon::Reveal => "reveal",
            Icon::Info => "info",
            Icon::Help => "help",
            Icon::Keyboard => "keyboard",
            Icon::ExpandAll => "expand_all",
            Icon::CollapseAll => "collapse_all",
            Icon::MoveUp => "move_up",
            Icon::MoveDown => "move_down",
            Icon::Globe => "globe",
            Icon::Key => "key",
            Icon::Shield => "shield",
            Icon::Tag => "tag",
            Icon::Picture => "picture",
            Icon::Terminal => "terminal",
            Icon::Refresh => "refresh",
            Icon::Error => "error",
            Icon::Warning => "warning",
            Icon::Success => "success",
            Icon::Required => "required",
            Icon::Spinner => "spinner",
            Icon::Dirty => "dirty",
            Icon::BuildCollision => "build_collision",
            Icon::BuildAIMap => "build_ai_map",
            Icon::BuildGameGraph => "build_game_graph",
            Icon::BuildAll => "build_all",
            Icon::Play => "play",
            Icon::PlayGame => "play_game",
            Icon::PlayClean => "play_clean",
            Icon::Stop => "stop",
            Icon::ReloadConfigs => "reload_configs",
            Icon::GameFolder => "game_folder",
            Icon::BuildMod => "build_mod",
            Icon::Package => "package",
            Icon::Publish => "publish",
            Icon::Releases => "releases",
            Icon::Select => "select",
            Icon::Place => "place",
            Icon::Move => "move",
            Icon::Rotate => "rotate",
            Icon::Scale => "scale",
            Icon::SnapGrid => "snap_grid",
            Icon::SnapAngle => "snap_angle",
            Icon::SnapScale => "snap_scale",
            Icon::SnapObject => "snap_object",
            Icon::FrameAll => "frame_all",
            Icon::FrameSelection => "frame_selection",
            Icon::Perspective => "perspective",
            Icon::Ortho => "ortho",
            Icon::Lit => "lit",
            Icon::Unlit => "unlit",
            Icon::Wireframe => "wireframe",
            Icon::Bones => "bones",
            Icon::Stats => "stats",
            Icon::ViewOptions => "view_options",
            Icon::SceneObject => "scene_object",
            Icon::Light => "light",
            Icon::SoundSource => "sound_source",
            Icon::SoundEnv => "sound_env",
            Icon::Glow => "glow",
            Icon::Shape => "shape",
            Icon::Spawn => "spawn",
            Icon::Way => "way",
            Icon::Sector => "sector",
            Icon::Portal => "portal",
            Icon::Group => "group",
            Icon::Particles => "particles",
            Icon::DetailObjects => "detail_objects",
            Icon::AIMap => "ai_map",
            Icon::Wallmark => "wallmark",
            Icon::FogVolume => "fog_volume",
            Icon::Folder => "folder",
            Icon::FolderOpen => "folder_open",
            Icon::File => "file",
            Icon::FileTexture => "file_texture",
            Icon::FileModel => "file_model",
            Icon::FileSound => "file_sound",
            Icon::FileScript => "file_script",
            Icon::FileConfig => "file_config",
            Icon::FileXml => "file_xml",
            Icon::FileText => "file_text",
            Icon::FileArchive => "file_archive",
            Icon::Level => "level",
            Icon::Quest => "quest",
            Icon::Behaviour => "behaviour",
            Icon::Character => "character",
            Icon::CharacterTemplate => "character_template",
            Icon::PdaInterface => "pda_interface",
            Icon::Flow => "flow",
            Icon::Wait => "wait",
            Icon::Condition => "condition",
            Icon::Variable => "variable",
            Icon::SpawnNpc => "spawn_npc",
            Icon::Dialog => "dialog",
            Icon::Item => "item",
            Icon::PdaMsg => "pda_msg",
            Icon::World => "world",
            Icon::Relation => "relation",
            Icon::Squad => "squad",
            Icon::Order => "order",
            Icon::Timer => "timer",
            Icon::Trigger => "trigger",
            Icon::End => "end",
            Icon::Task => "task",
            Icon::Role => "role",
            Icon::Minimap => "minimap",
            Icon::AutoLayout => "auto_layout",
            Icon::Validate => "validate",
            Icon::FindInProject => "find_in_project",
            Icon::ZoomIn => "zoom_in",
            Icon::ZoomOut => "zoom_out",
            Icon::ZoomFit => "zoom_fit",
            Icon::Zoom100 => "zoom_100",
            Icon::Person => "person",
            Icon::Community => "community",
            Icon::Rank => "rank",
            Icon::Money => "money",
            Icon::Kit => "kit",
            Icon::Area => "area",
            Icon::Transition => "transition",
            Icon::PlayerStart => "player_start",
            Icon::Mode => "mode",
            Icon::Relations => "relations",
            Icon::Exclude => "exclude",
            Icon::Aspect => "aspect",
            Icon::Language => "language",
            Icon::Tasks => "tasks",
            Icon::Mcp => "mcp",
            Icon::Log => "log",
            Icon::Problems => "problems",
            Icon::Drawer => "drawer",
        }
    }

    /// The icon an MCP name or a test asks for.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Icon> {
        Icon::ALL.iter().copied().find(|i| i.name() == name)
    }
}

/// Draws an icon into a square: `rect`'s top-left corner and its width. The colour is the
/// caller's; a secondary stroke of a glyph is drawn at 55% of it.
pub fn draw(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    if icon == Icon::None {
        return;
    }
    if let Some(g) = geometry::glyphs().get(icon as usize) {
        paint::draw_glyph(painter, g, rect.min, rect.width(), color);
    }
}

/// Draws an icon centred in the largest square that fits `rect`.
pub fn draw_in(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    draw(painter, icon, paint::square_in(rect), color);
}

/// The developer's sheet of the icon set: every glyph from `first` on (an index into
/// [`Icon::ALL`]) at 16, 24 and 48 pt with its name, the 48 pt one in its grid frame and margin,
/// in the colour a state gives it (Text, Secondary, Checked, Disabled). Draw it in a scroll area.
pub fn sheet(ui: &mut Ui, first: usize) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let colour_id = egui::Id::unique("gz-ui.icons.sheet.colour");
    let mut colour = ctx.data(|d| d.get_temp::<usize>(colour_id)).unwrap_or(0);
    if crate::button::segmented(ui, &mut colour, &["Text", "Secondary", "Checked", "Disabled"], None) {
        ctx.data_mut(|d| d.insert_temp(colour_id, colour));
    }
    let ink = match colour {
        1 => p.text_secondary,
        2 => p.accent_bright,
        3 => p.text_disabled,
        _ => p.text,
    };
    let icons = Icon::ALL.get(first.min(Icon::ALL.len())..).unwrap_or(&[]);
    let font = crate::font::Font::Small.id();
    let sizes = [16.0f32, 24.0, 48.0];
    let gap = 8.0;
    let big = 48.0;
    let name_w = Icon::ALL.iter().map(|i| crate::text::width(&ctx, &font, i.name())).fold(0.0, f32::max);
    let glyphs_w = sizes.iter().sum::<f32>() + gap * 2.0;
    let cell_w = glyphs_w.max(name_w) + gap * 2.0;
    let cell_h = big + gap * 2.0 + crate::font::Font::Small.line();
    let cols = ((ui.available_width() / cell_w) as usize).max(1);
    for row in icons.chunks(cols) {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(cols as f32 * cell_w, cell_h), egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let painter = ui.painter();
        // a checked icon sits on selection.focused
        if colour == 2 {
            painter.rect_filled(rect, 0.0, p.selection_focused);
        }
        for (col, icon) in row.iter().enumerate() {
            let x0 = rect.min.x + col as f32 * cell_w + gap;
            let base = rect.min.y + gap + big;
            let mut x = x0;
            for side in sizes {
                let min = egui::pos2(x.round(), (base - side).round());
                let square = Rect::from_min_size(min, egui::vec2(side, side));
                if side == big {
                    let unit = side / geometry::GRID;
                    painter.rect_stroke(
                        square,
                        0.0,
                        egui::Stroke::new(1.0, p.border.gamma_multiply(0.8)),
                        egui::StrokeKind::Inside,
                    );
                    painter.rect_stroke(
                        square.shrink(unit),
                        0.0,
                        egui::Stroke::new(1.0, p.border.gamma_multiply(0.35)),
                        egui::StrokeKind::Inside,
                    );
                }
                draw(painter, *icon, square, ink);
                x += side + gap;
            }
            painter.text(
                egui::pos2(x0, base + gap * 0.5),
                egui::Align2::LEFT_TOP,
                icon.name(),
                font.clone(),
                p.text_muted,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Icon;
    use super::geometry::{PrimKind, ear_clip, glyphs, signed_area};
    use egui::pos2;

    #[test]
    fn every_icon_has_geometry() {
        let all = glyphs();
        for &icon in Icon::ALL {
            let g = all.get(icon as usize);
            assert!(g.is_some_and(|g| !g.prims.is_empty()), "{} has no geometry", icon.name());
        }
        assert!(all.first().is_some_and(|g| g.prims.is_empty()));
    }

    #[test]
    fn names_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for &icon in Icon::ALL {
            assert!(seen.insert(icon.name()), "{} twice", icon.name());
            assert_eq!(Icon::from_name(icon.name()), Some(icon));
        }
        assert_eq!(Icon::ALL.len(), 189);
        assert!(!Icon::ALL.contains(&Icon::None));
        assert_eq!(Icon::from_name("no_such_icon"), None);
    }

    #[test]
    fn concave_fills_are_triangulated() {
        let all = glyphs();
        for icon in [Icon::StarFilled, Icon::BookmarkFilled] {
            let g = all.get(icon as usize);
            let fill = g.and_then(|g| g.prims.iter().find(|p| p.kind == PrimKind::Fill).map(|p| (g, *p)));
            assert!(
                fill.is_some_and(|(g, p)| g.triangles(&p).len() == (p.count as usize - 2) * 3),
                "{}",
                icon.name()
            );
        }
    }

    #[test]
    fn ear_clipping_covers_the_polygon() {
        // a notched square, clockwise on screen
        let pts = [pos2(0.0, 0.0), pos2(4.0, 0.0), pos2(4.0, 4.0), pos2(2.0, 2.0), pos2(0.0, 4.0)];
        assert!(signed_area(&pts) > 0.0);
        let tris = ear_clip(&pts);
        assert_eq!(tris.len(), 9);
        let area: f32 = tris
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| pts[i as usize]);
                ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
            })
            .sum();
        assert!((area - 12.0).abs() < 1e-4);
    }
}
