//! What a frame is drawn for: the camera, the size, the view mode and what is shown.

use crate::camera::Camera;
use crate::color::Color;

/// How surfaces are shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewMode {
    /// "Studio": a headlight along the view, a key from above and a hemisphere ambient.
    #[default]
    Lit,
    /// Textures as they are.
    Unlit,
    /// The edges of every mesh, no hidden lines removed.
    Wireframe,
    /// The scene's lights and the weather of [`crate::Environment`], per pixel, the way the game
    /// lights the level. Without an environment it is `Lit`.
    GameLighting,
}

impl ViewMode {
    pub(crate) fn code(self) -> u32 {
        match self {
            Self::Lit => 0,
            Self::Unlit => 1,
            Self::Wireframe => 2,
            Self::GameLighting => 3,
        }
    }
}

bitflags::bitflags! {
    /// What a frame shows besides the scene's surfaces.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ViewFlags: u32 {
        /// The floor grid.
        const GRID = 1 << 0;
        /// Helper shapes, lines, labels and overlays.
        const HELPERS = 1 << 1;
        /// The selection outline.
        const SELECTION = 1 << 2;
        /// The scene once more as a picture (`preview.png`, thumbnails): no grid, selection,
        /// gizmo or helpers whatever the other flags say, and no instance flagged
        /// [`crate::InstanceFlags::HIDDEN_IN_PREVIEW`].
        const PREVIEW = 1 << 3;
    }
}

/// The floor grid: `cells` each way from the origin, a thick line
/// every `subdiv` cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub cells: u32,
    pub cell_size: f32,
    pub subdiv: u32,
    pub color: Color,
    pub thick_color: Color,
}

impl Default for Grid {
    /// The preferences' defaults: 100 cells of 1 m, a thick line every 10.
    fn default() -> Self {
        Self {
            cells: 100,
            cell_size: 1.0,
            subdiv: 10,
            color: Color::from_argb(0xff90_9090),
            thick_color: Color::from_argb(0xffb4_b4b4),
        }
    }
}

/// What a frame shows besides its view mode.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewStyle {
    /// Fog in a view without the weather: white, from a tenth of the far plane to nearly all of
    /// it. Game lighting always fogs with the weather's own.
    pub fog: bool,
}

/// A frame's view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub camera: Camera,
    /// The render size in pixels.
    pub size: [u32; 2],
    pub mode: ViewMode,
    pub flags: ViewFlags,
    /// What is behind the scene (alpha ignored: a picture is always opaque).
    pub background: Color,
    pub grid: Grid,
    pub selection_color: Color,
    /// The colour of edges in [`ViewMode::Wireframe`].
    pub wire_color: Color,
    pub style: ViewStyle,
}

impl View {
    /// The level viewport's defaults: lit, grid, helpers and selection on, the editor's grey.
    #[must_use]
    pub fn new(camera: Camera, size: [u32; 2]) -> Self {
        Self {
            camera,
            size,
            mode: ViewMode::Lit,
            flags: ViewFlags::GRID | ViewFlags::HELPERS | ViewFlags::SELECTION,
            // DEFAULT_CLEARCOLOR
            background: Color::from_argb(0xff55_5555),
            grid: Grid::default(),
            selection_color: Color::rgb(255, 176, 48),
            wire_color: Color::rgb(210, 210, 210),
            style: ViewStyle::default(),
        }
    }

    /// The same view as a picture: [`ViewFlags::PREVIEW`] alone.
    #[must_use]
    pub fn as_preview(&self) -> Self {
        Self { flags: ViewFlags::PREVIEW, ..*self }
    }

    pub(crate) fn shows(&self, flag: ViewFlags) -> bool {
        !self.flags.contains(ViewFlags::PREVIEW) && self.flags.contains(flag)
    }
}
