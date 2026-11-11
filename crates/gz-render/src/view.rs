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
    /// The edges of every mesh, no hidden lines removed (`D3DFILL_WIREFRAME`).
    Wireframe,
    /// The scene's lights and the weather of [`crate::Environment`], per pixel, the way the game
    /// lights the level (`EGameLighting`). Without an environment it is `Lit`.
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
        /// [`ViewMode::Lit`] by the list's directional lights the way the C++ editor's
        /// fixed-function emulation lights (`EDevice_PushFFConstants`, at most four, no
        /// ambient), and a surface no light reaches as it is (`calc_ffp_lighting` returns 1);
        /// without it the studio light.
        const SCENE_LIGHTS = 1 << 4;
    }
}

/// The floor grid (`CDrawUtilities::UpdateGrid`): `cells` each way from the origin, a thick line
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

/// The View menu's render switches (`psDeviceFlags`, `dwShadeMode`): how surfaces are drawn
/// besides the view mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewStyle {
    /// `D3DSHADE_FLAT`: one normal per face.
    pub flat: bool,
    /// `rsRenderTextures`: off draws every surface white.
    pub textures: bool,
    /// `rsFilterLinear`: off samples the nearest texel.
    pub linear: bool,
    /// `rsEdgedFaces`: the edges drawn over the surfaces.
    pub edged: bool,
    /// `rsFog`: the weather's fog with [`ViewStyle::environment`], the editor's own otherwise.
    pub fog: bool,
    /// `rsEnvironment`: the weather's sky and fog colour round the scene in every view mode.
    pub environment: bool,
}

impl Default for ViewStyle {
    fn default() -> Self {
        Self { flat: false, textures: true, linear: true, edged: false, fog: false, environment: false }
    }
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
        // the picture is lit as the view is
        Self { flags: ViewFlags::PREVIEW | (self.flags & ViewFlags::SCENE_LIGHTS), ..*self }
    }

    pub(crate) fn shows(&self, flag: ViewFlags) -> bool {
        !self.flags.contains(ViewFlags::PREVIEW) && self.flags.contains(flag)
    }
}
