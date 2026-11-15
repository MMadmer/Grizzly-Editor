//! Every media asset of a project: by kind, by path and by the name it publishes, with the
//! files it takes its data from. Made once per state of the files (the service keeps it), or
//! by a walk of a project folder for a build ([`scan`]).

use std::path::Path;

use gz_base::{RelPath, Rev};
use gz_project::asset_kind::MediaKind;

use crate::files;
use crate::kind;
use crate::model::ModelAsset;
use crate::read::name_key;
use crate::sound::SoundAsset;
use crate::texture::TextureAsset;

/// One media asset.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: MediaKind,
    pub path: RelPath,
    /// The name the game knows it by, as written (back slashes).
    pub name: String,
    /// [`name_key`] of `name`: what two names are compared by.
    pub key: String,
    /// Its source, when it is there.
    pub source: Option<RelPath>,
    /// What else it takes its data from: a glTF's buffers and pictures, an OBJ's material
    /// libraries, a texture's bump picture.
    pub uses: Vec<RelPath>,
    /// A model with `kind = "dynamic"`: published as a model of its own.
    pub dynamic: bool,
    /// A model of `parts`: no source.
    pub parts: bool,
    /// Left out of the build by `[exclude]`.
    pub excluded: bool,
    /// Why the file does not read.
    pub error: Option<String>,
}

impl Entry {
    /// Whether the build publishes it: a texture, a sound, a dynamic model - read, not excluded.
    #[must_use]
    pub fn ships(&self) -> bool {
        !self.excluded && self.error.is_none() && (self.kind != MediaKind::Model || self.dynamic)
    }
}

/// The media assets of a project.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// The state of the files it was made from.
    pub stamp: Rev,
    pub module: String,
    /// In path order.
    pub entries: Vec<Entry>,
}

impl Index {
    /// The asset at a path (either slash, any case).
    #[must_use]
    pub fn at(&self, rel: &str) -> Option<&Entry> {
        let key = rel.replace('\\', "/").to_lowercase();
        self.entries.iter().find(|e| e.path.as_str().to_lowercase() == key)
    }

    /// The assets of a kind that publish a name, the excluded ones left out.
    #[must_use]
    pub fn named(&self, kind: MediaKind, name: &str) -> Vec<&Entry> {
        let key = name_key(name);
        self.entries.iter().filter(|e| e.kind == kind && !e.excluded && e.key == key).collect()
    }

    /// The first asset of a kind that publishes a name.
    #[must_use]
    pub fn by_name(&self, kind: MediaKind, name: &str) -> Option<&Entry> {
        self.named(kind, name).into_iter().next()
    }

    /// The names a kind publishes, sorted, each once.
    #[must_use]
    pub fn names(&self, kind: MediaKind) -> Vec<String> {
        let mut out: Vec<String> =
            self.entries.iter().filter(|e| e.kind == kind && !e.excluded).map(|e| e.name.clone()).collect();
        out.sort_by_key(|n| n.to_lowercase());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out
    }

    /// The assets of a kind.
    pub fn of_kind(&self, kind: MediaKind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }

    /// The assets that take their data from a file.
    #[must_use]
    pub fn users_of(&self, rel: &str) -> Vec<&Entry> {
        let key = rel.replace('\\', "/").to_lowercase();
        let same = |p: &RelPath| p.as_str().to_lowercase() == key;
        self.entries
            .iter()
            .filter(|e| e.source.as_ref().is_some_and(same) || e.uses.iter().any(same))
            .collect()
    }

    /// A file the build never ships as it is: a media asset where it lies, or a file an asset
    /// takes its data from.
    #[must_use]
    pub fn is_source(&self, rel: &str) -> bool {
        gz_project::rules::is_media_asset_name(rel) || !self.users_of(rel).is_empty()
    }

    /// The paths the build writes for the assets that ship, relative to the module's root,
    /// without baking anything.
    #[must_use]
    pub fn published_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        for e in self.entries.iter().filter(|e| e.ships()) {
            out.extend(crate::publish::paths_of(e));
        }
        out
    }
}

/// What one asset file says of itself, read: its name, its source and what else it uses.
#[must_use]
pub fn entry(root: &Path, rel: &RelPath, module: &str, excluded: bool) -> Entry {
    let kind = MediaKind::of_file(rel.as_str()).unwrap_or(MediaKind::Model);
    let abs = rel.to_fs(root);
    let mut e = Entry {
        kind,
        path: rel.clone(),
        name: String::new(),
        key: String::new(),
        source: None,
        uses: Vec::new(),
        dynamic: false,
        parts: false,
        excluded,
        error: None,
    };
    let value = std::fs::read(&abs)
        .map_err(|err| format!("{rel} could not be read: {}", gz_base::io_reason(&err)))
        .and_then(|b| kind::load(kind, rel.as_str(), &String::from_utf8_lossy(&b)));
    let value = match value {
        Ok(v) => v,
        Err(why) => {
            e.name = files::published_name(None, rel.as_str(), module);
            e.key = name_key(&e.name);
            e.error = Some(why);
            return e;
        }
    };
    let rel_of = |p: &Path| files::rel_of(root, p);
    let (name, source) = match kind {
        MediaKind::Model => {
            let m = ModelAsset::from_value(&value).0;
            e.dynamic = m.dynamic;
            e.parts = !m.parts.is_empty();
            (m.name, if e.parts { None } else { m.source })
        }
        MediaKind::Texture => {
            let t = TextureAsset::from_value(&value).0;
            if let Some(b) = t.bump.as_ref().and_then(|b| b.source.as_deref()) {
                let p = files::beside(&abs, b);
                if let Some(r) = p.is_file().then(|| rel_of(&p)).flatten() {
                    e.uses.push(r);
                }
            }
            (t.name, t.source)
        }
        MediaKind::Sound => {
            let s = SoundAsset::from_value(&value).0;
            (s.name, s.source)
        }
    };
    e.name = files::published_name(name.as_deref(), rel.as_str(), module);
    e.key = name_key(&e.name);
    if !e.parts
        && let Some(src) = files::source_of(kind, &abs, source.as_deref())
    {
        e.uses.extend(files::dependencies(&src).iter().filter_map(|d| rel_of(d)));
        e.source = rel_of(&src);
    }
    e
}

/// The index of these asset files, read in parallel on the caller's pool.
#[must_use]
pub fn build(root: &Path, files: Vec<(RelPath, bool)>, module: String, stamp: Rev) -> Index {
    use rayon::prelude::*;
    let mut entries: Vec<Entry> =
        files.par_iter().map(|(rel, excluded)| entry(root, rel, &module, *excluded)).collect();
    entries.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    Index { stamp, module, entries }
}

/// The index of a project folder by a walk of it (the editor's scratch, tooling and links left
/// out, `[exclude]` applied): what a build that has no editor's service reads. Blocking.
#[must_use]
pub fn scan(root: &Path) -> Index {
    let (module, rules) = match gz_mod::read_manifest_file(root) {
        Ok(Some(text)) => {
            let m = gz_mod::Manifest::parse(&gz_mod::ManifestDoc::parse(&text));
            (m.id, m.exclude)
        }
        _ => (String::new(), Vec::new()),
    };
    let mut found = Vec::new();
    walk(root, root, &mut found);
    let files = found
        .into_iter()
        .map(|rel| {
            let excluded = gz_mod::outside_build(&rules, rel.as_str());
            (rel, excluded)
        })
        .collect();
    build(root, files, module, Rev::ZERO)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<RelPath>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if gz_project::is_tooling_entry(&name) || (dir == root && gz_project::is_editor_only_entry(&name)) {
            continue;
        }
        let Ok(ty) = e.file_type() else { continue };
        if ty.is_symlink() {
            continue;
        }
        let p = e.path();
        if ty.is_dir() {
            walk(root, &p, out);
        } else if gz_project::rules::is_media_asset_name(&name)
            && let Some(rel) = files::rel_of(root, &p)
        {
            out.push(rel);
        }
    }
}
