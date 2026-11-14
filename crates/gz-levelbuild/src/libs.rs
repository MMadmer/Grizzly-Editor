//! The engine libraries a build checks the scene's names against.

use gz_fmt_xr::{GameMaterials, Shaders, ShadersXrlc};

/// The game material flags the build reads: the material library's own.
pub mod mtl_flags {
    pub const BREAKABLE: u32 = gz_fmt_xr::GameMaterial::BREAKABLE;
    pub const CLIMABLE: u32 = gz_fmt_xr::GameMaterial::CLIMABLE;
    pub const DYNAMIC: u32 = gz_fmt_xr::GameMaterial::DYNAMIC;
}

/// The compiler shader flags.
pub mod xrlc_flags {
    pub const COLLISION: u32 = 1 << 0;
    pub const RENDERING: u32 = 1 << 1;
}

/// The three libraries, read once per build.
#[derive(Clone, Debug, Default)]
pub struct Engine {
    pub shaders: Option<Shaders>,
    pub xrlc: Option<ShadersXrlc>,
    pub materials: Option<GameMaterials>,
}

impl Engine {
    /// Reads the libraries from their bytes; one that does not read is absent.
    #[must_use]
    pub fn read(shaders: Option<&[u8]>, xrlc: Option<&[u8]>, materials: Option<&[u8]>) -> Self {
        Self {
            shaders: shaders.and_then(|b| Shaders::read(b).ok()),
            xrlc: xrlc.and_then(|b| ShadersXrlc::read(b).ok()),
            materials: materials.and_then(|b| GameMaterials::read(b).ok()),
        }
    }

    /// `Resources->_FindBlender`: the engine shader exists.
    #[must_use]
    pub fn has_shader(&self, name: &str) -> bool {
        self.shaders.as_ref().is_some_and(|s| s.blender(name).is_some())
    }

    /// `ShaderXRLC.Get(name)`'s flags; `None` for a shader the library lacks.
    #[must_use]
    pub fn xrlc_flags(&self, name: &str) -> Option<u32> {
        let lib = self.xrlc.as_ref()?;
        lib.shaders.iter().find(|s| s.name().eq_ignore_ascii_case(name)).map(|s| s.flags)
    }

    /// `GetMaterialIdx(name)`: the material's index in the library.
    // a name the library lacks gives the index past the end, as the X-Ray SDK's iterator
    // arithmetic does in a release build.
    #[must_use]
    pub fn material_index(&self, name: &str) -> u16 {
        let Some(lib) = &self.materials else { return 0 };
        lib.materials.iter().position(|m| m.name.eq_ignore_ascii_case(name)).unwrap_or(lib.materials.len())
            as u16
    }

    /// The material's id, `None` when the name is unknown.
    #[must_use]
    pub fn material_id(&self, name: &str) -> Option<u32> {
        self.materials.as_ref()?.by_name(name).map(|m| m.id)
    }

    /// The flags of a material by id.
    #[must_use]
    pub fn material_flags(&self, id: u32) -> Option<u32> {
        self.materials.as_ref()?.by_id(id).map(|m| m.flags)
    }
}
