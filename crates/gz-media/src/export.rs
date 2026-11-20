//! `asset.export`: a media asset's file written back out of the project as it was brought in -
//! its source with what the source names beside it (a glTF's buffers and pictures, an OBJ's
//! material libraries, a texture's bump picture and `#` companion) and a `.thm` that lies beside
//! it, each where it lay beside the source; over a file of the game, the game's file. A model of
//! `parts` has no file: its geometry is the asset's text.

use std::path::{Path, PathBuf};

use gz_base::RelPath;
use gz_project::asset_kind::MediaKind;

use crate::index::Entry;
use crate::resolve;

/// Where one file of an export comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum From {
    /// A file of the project.
    Project(RelPath),
    /// A model of the game, by its name.
    GameModel(String),
    /// A texture of the game, by its name.
    GameTexture(String),
}

/// One file an export writes: its name under the target folder (`\` separators) and its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out {
    pub name: String,
    pub from: From,
}

/// The files of one asset, or why it has none. Pure but for whether a `.thm` lies beside the
/// source (`there`).
pub fn files_of(e: &Entry, there: impl Fn(&RelPath) -> bool) -> Result<Vec<Out>, String> {
    if let Some(why) = &e.error {
        return Err(format!("{}: {why}", e.path));
    }
    if let Some(src) = &e.source {
        let base = src.parent().unwrap_or_else(RelPath::root);
        // a file below the source's folder keeps where it lies beside it, any other its name
        let named = |r: &RelPath| match r.strip_prefix(&base) {
            Some(tail) if !tail.is_root() => tail.to_backslash(),
            _ => r.file_name().to_owned(),
        };
        let mut out = vec![Out { name: named(src), from: From::Project(src.clone()) }];
        let thm = Some(src.with_extension("thm")).filter(|t| t.key() != src.key() && there(t));
        for r in e.uses.iter().chain(&e.companion).chain(&thm) {
            if !out.iter().any(|o| o.from == From::Project(r.clone())) {
                out.push(Out { name: named(r), from: From::Project(r.clone()) });
            }
        }
        return Ok(out);
    }
    if let Some(game) = &e.game {
        let leaf = game.rsplit(['\\', '/']).next().unwrap_or(game);
        return Ok(match e.kind {
            MediaKind::Model => {
                let name = game.strip_suffix(".ogf").unwrap_or(game);
                let leaf = leaf.strip_suffix(".ogf").unwrap_or(leaf);
                vec![Out { name: format!("{leaf}.ogf"), from: From::GameModel(name.to_owned()) }]
            }
            MediaKind::Texture => {
                vec![Out { name: format!("{leaf}.dds"), from: From::GameTexture(game.clone()) }]
            }
            MediaKind::Sound => {
                return Err(format!("{}: a sound of the game has no file of the project", e.path));
            }
        });
    }
    if e.parts {
        return Err(format!("{}: a model of parts has no file - its geometry is the asset's text", e.path));
    }
    Err(format!("{}: the asset names no file", e.path))
}

/// The bytes of one file. Blocking.
pub fn read(
    root: &Path,
    from: &From,
    models: &resolve::GameModel,
    textures: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    match from {
        From::Project(r) => {
            std::fs::read(r.to_fs(root)).map_err(|e| format!("{r}: {}", gz_base::io_reason(&e)))
        }
        From::GameModel(n) => models(n).ok_or_else(|| format!("the linked game has no model {n}")),
        From::GameTexture(n) => textures(n).ok_or_else(|| format!("the linked game has no texture {n}")),
    }
}

/// Where an export's file lands: under `to`, never above it.
#[must_use]
pub fn target(to: &Path, name: &str) -> Option<PathBuf> {
    let mut p = to.to_path_buf();
    for part in name.split(['\\', '/']).filter(|s| !s.is_empty()) {
        if part == "." || part == ".." || part.contains(':') {
            return None;
        }
        p.push(part);
    }
    (p != to).then_some(p)
}
