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
    /// A model or a texture over one of the game (`source = "game:..."`): its game name. A
    /// texture of the game publishes its description alone - the picture stays the game's.
    pub game: Option<String>,
    /// A game texture's `#` companion (`<stem>#.dds` beside its `.dds` source), published with it.
    pub companion: Option<RelPath>,
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

    /// The project file a model of the game is made of when the project has it: a `game:` model
    /// is read from the project's own `gamedata\meshes` first.
    #[must_use]
    pub fn game_model_file(&self) -> Option<String> {
        (self.kind == MediaKind::Model)
            .then_some(self.game.as_ref())
            .flatten()
            .map(|g| format!("gamedata/meshes/{}.ogf", g.replace('\\', "/").to_lowercase()))
    }

    /// Whether the asset takes this project file in (its source, a file the source names, a
    /// bump picture, a `#` companion, the `.thm` beside its source), so the build never ships
    /// the file as it is. Its source is its own while the asset is the module's (not excluded):
    /// a file has one asset, which says where it ships - a `game:` model's file of the project
    /// too. Another file of the module's `gamedata` is the game's data where it lies: it is the
    /// asset's only when the asset publishes it at that very path, and ships by its own name
    /// when the asset publishes another one (a picture a model wears).
    #[must_use]
    pub fn takes(&self, rel: &str) -> bool {
        let key = rel.replace('\\', "/").to_lowercase();
        let same = |p: &RelPath| p.as_str().to_lowercase() == key;
        if !self.excluded
            && (self.source.as_ref().is_some_and(|s| same(s) || same(&s.with_extension("thm")))
                || self.game_model_file().is_some_and(|g| g == key))
        {
            return true;
        }
        let named = self.uses.iter().any(same) || self.companion.as_ref().is_some_and(same);
        if !named {
            return false;
        }
        if !ships_where_it_lies(&key) {
            return true;
        }
        self.ships()
            && crate::publish::paths_of(self).iter().any(|p| p.replace('\\', "/").to_lowercase() == key)
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
        // a model of the game is read from the project's own `gamedata\meshes` first: a file
        // there is what a `game:` model of that name is made of
        let game_model = key
            .strip_prefix("gamedata/meshes/")
            .and_then(|k| k.strip_suffix(".ogf"))
            .map(|k| k.replace('/', "\\"));
        self.entries
            .iter()
            .filter(|e| {
                e.source.as_ref().is_some_and(same)
                    || e.uses.iter().any(same)
                    || e.companion.as_ref().is_some_and(same)
                    || (e.kind == MediaKind::Model && e.game.is_some() && e.game == game_model)
            })
            .collect()
    }

    /// A file the build never ships as it is: a media asset where it lies, or a file an asset
    /// takes its data from ([`Entry::takes`]).
    #[must_use]
    pub fn is_source(&self, rel: &str) -> bool {
        gz_project::rules::is_media_asset_name(rel) || self.entries.iter().any(|e| e.takes(rel))
    }

    /// Every file of the project some asset takes ([`Entry::takes`]), lower case with `/`: made
    /// once, for whoever asks about many files.
    #[must_use]
    pub fn taken(&self) -> rustc_hash::FxHashSet<String> {
        let mut out = rustc_hash::FxHashSet::default();
        for e in &self.entries {
            let thm = e.source.as_ref().map(|s| s.with_extension("thm"));
            for r in e.source.iter().chain(&e.uses).chain(&e.companion).chain(&thm) {
                if e.takes(r.as_str()) {
                    out.insert(r.as_str().replace('\\', "/").to_lowercase());
                }
            }
            if let Some(g) = e.game_model_file().filter(|g| e.takes(g)) {
                out.insert(g);
            }
        }
        out
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
        game: None,
        companion: None,
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
            e.game = files::game_model(m.source.as_deref()).filter(|_| !e.parts);
            (m.name, if e.parts || e.game.is_some() { None } else { m.source })
        }
        MediaKind::Texture => {
            let t = TextureAsset::from_value(&value).0;
            if let Some(b) = t.bump.as_ref().and_then(|b| b.source.as_deref()) {
                let p = files::beside(&abs, b);
                if let Some(r) = p.is_file().then(|| rel_of(&p)).flatten() {
                    e.uses.push(r);
                }
            }
            // a description of a texture of the game is that texture's: it publishes under its name
            e.game = files::game_texture(t.source.as_deref());
            match &e.game {
                Some(g) => (Some(t.name.clone().unwrap_or_else(|| g.clone())), None),
                None => (t.name, t.source),
            }
        }
        MediaKind::Sound => {
            let s = SoundAsset::from_value(&value).0;
            (s.name, s.source)
        }
    };
    e.name = files::published_name(name.as_deref(), rel.as_str(), module);
    e.key = name_key(&e.name);
    if !e.parts
        && e.game.is_none()
        && let Some(src) = files::source_of(kind, &abs, source.as_deref())
    {
        e.uses.extend(files::dependencies(&src).iter().filter_map(|d| rel_of(d)));
        if kind == MediaKind::Texture {
            e.companion = files::hash_companion(&src).and_then(|c| rel_of(&c));
        }
        e.source = rel_of(&src);
    }
    e
}

/// Whether a project file is one the game reads where it lies: a file of the module's
/// `gamedata`. A build ships it by its own name whatever asset names it as a source.
#[must_use]
pub fn ships_where_it_lies(rel: &str) -> bool {
    rel.replace('\\', "/").to_lowercase().starts_with("gamedata/")
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
