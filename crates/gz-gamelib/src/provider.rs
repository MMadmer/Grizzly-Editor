//! Models of the game as objects of the scene's library: a reference that ends in `.ogf` is a
//! compiled model by its engine name - the project's own `gamedata\meshes` first, then the
//! linked game's files - read into the model everything downstream uses
//! (`docs/GAME_LIBRARY.md`, 7).

use std::path::Path;
use std::sync::Arc;

use gz_editor::Editor;
use gz_scene::library::{LibraryObject, Provider, ProviderMakers};

use crate::service::GameLibrary;

/// The bytes of a compiled model by its engine name (`dynamics\box\box_wood_01`), from the
/// project's `gamedata\meshes` or the game there is now. Blocking.
pub fn model_bytes(editor: &Editor, project: Option<&Path>, name: &str) -> Option<Vec<u8>> {
    // a name under `meshes`, never a way out of it: no drive, no root, no step up - nothing a
    // scene or a command names is read from outside the project and the game
    let plain = !name.contains(':')
        && name.split(['\\', '/']).all(|part| !part.is_empty() && part != "." && part != "..");
    if !plain {
        return None;
    }
    let rel = format!("{}.ogf", name.replace('\\', "/"));
    if let Some(root) = project {
        let own = root.join("gamedata").join("meshes").join(&rel);
        if own.is_file() {
            return std::fs::read(own).ok();
        }
    }
    let game = GameLibrary::game_root(editor);
    if game.is_empty() {
        return None;
    }
    let files = GameLibrary::of(editor).files_blocking(editor, &game).ok()?;
    files.read(&format!("meshes/{rel}")).ok()
}

fn provider(editor: &Arc<Editor>, project: &Path) -> Provider {
    let (ed, root) = (editor.clone(), project.to_path_buf());
    Arc::new(move |key: &str| {
        if let Some(level) = key.strip_prefix(crate::levels::GEOMETRY_PREFIX) {
            return crate::levels::geometry(&ed, level).map(Arc::new);
        }
        let name = key.strip_suffix(".ogf")?;
        let bytes = model_bytes(&ed, Some(&root), name)?;
        let src = gz_fmt_mesh::read_ogf_bytes(&bytes, key).ok()?;
        let mut made = LibraryObject::provided(key, src.object);
        // FNV-1a over the file: a model whose file changed is another model
        made.stamp = bytes
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
        Some(Arc::new(made))
    })
}

/// Adds the provider to every scene library made from now on.
pub fn install(editor: &Arc<Editor>) {
    ProviderMakers::of(editor).add(Arc::new(|ed, root| Some(provider(ed, root))));
    // a model asset over a model of the game reads it the way the scene does
    gz_media::GameModels::of(editor).provide(Arc::new(|ed, root, name| model_bytes(ed, Some(root), name)));
}
