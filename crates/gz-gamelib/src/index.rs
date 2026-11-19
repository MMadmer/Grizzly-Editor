//! The library's catalogs read back for searching: every row of every kind, loaded once per
//! state of the library and shared (`asset.find`, `asset.card`, the checks of a scene's values).

use std::path::Path;
use std::sync::Arc;

use gz_editor::{CmdError, Editor};
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::layout::Layout;
use crate::service::GameLibrary;

/// The kinds a search names, with the key a row is called by.
pub const KINDS: [(&str, &str); 11] = [
    ("model", "ref"),
    ("texture", "name"),
    ("sound", "name"),
    ("particle", "name"),
    ("section", "name"),
    ("material", "name"),
    ("compiler", "name"),
    ("shader", "name"),
    ("lightanim", "name"),
    ("sound_env", "name"),
    ("level", "name"),
];

/// The key a row of a kind is named by; `None` for a word that is no kind.
#[must_use]
pub fn name_key(kind: &str) -> Option<&'static str> {
    KINDS.iter().find(|k| k.0 == kind).map(|k| k.1)
}

/// The rows of the library, by kind.
#[derive(Default)]
pub struct Index {
    rows: FxHashMap<&'static str, Vec<Value>>,
    /// Per kind: a row's place by its name, lower case.
    by_name: FxHashMap<&'static str, FxHashMap<String, usize>>,
}

struct Cache {
    /// The library's folder and the time its state was written.
    key: (std::path::PathBuf, Option<std::time::SystemTime>),
    index: Arc<Index>,
}

/// The loaded index, and the turn at loading it: callers that ask at once load it once.
#[derive(Default)]
struct Held(Mutex<Option<Cache>>, tokio::sync::Mutex<()>);

fn read_lua(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    // the game's reader is the fast one and takes what an asset may be; a list of thousands of
    // rows is past that and goes through the reader of the editor's own files
    let v = gz_luadata::parse(&text).ok().or_else(|| {
        gz_luadata::LuaDoc::parse_value(&text, "", gz_luadata::Limits::scene()).ok().map(|(_, v)| v)
    })?;
    Some(gz_luadata::to_json(&v))
}

/// The rows of a list of a file: `key` of the file's table.
fn rows_of(file: &Value, key: &str) -> Vec<Value> {
    match file.get(key) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// Every `.lua` file of a folder, by name.
fn lua_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lua")))
        .collect();
    out.sort();
    out
}

impl Index {
    /// Reads every catalog of a library. Blocking; runs on the rayon pool it is called in.
    #[must_use]
    pub fn load(layout: &Layout) -> Self {
        let mut out = Self::default();
        let folder = |dir: std::path::PathBuf, key: &str| -> Vec<Value> {
            let files = lua_files(&dir);
            let lists: Vec<Vec<Value>> =
                files.par_iter().map(|p| read_lua(p).map(|v| rows_of(&v, key)).unwrap_or_default()).collect();
            lists.into_iter().flatten().collect()
        };
        out.rows.insert("model", folder(layout.root.join("models").join("index"), "models"));
        out.rows.insert("texture", folder(layout.root.join("textures"), "textures"));
        out.rows.insert("sound", folder(layout.root.join("sounds"), "sounds"));
        if let Some(v) = read_lua(&layout.list("particles")) {
            // a group and an effect are both a thing a scene's particle object names
            let mut rows = Vec::new();
            for (key, word) in [("groups", "group"), ("effects", "effect")] {
                for mut r in rows_of(&v, key) {
                    if let Some(m) = r.as_object_mut() {
                        m.insert("is".into(), Value::from(word));
                    }
                    rows.push(r);
                }
            }
            out.rows.insert("particle", rows);
        }
        if let Some(v) = read_lua(&layout.list("sections")) {
            out.rows.insert("section", rows_of(&v, "sections"));
        }
        if let Some(v) = read_lua(&layout.list("materials")) {
            out.rows.insert("material", rows_of(&v, "materials"));
            out.rows.insert("compiler", rows_of(&v, "compilers"));
            out.rows.insert("shader", rows_of(&v, "shaders"));
        }
        if let Some(v) = read_lua(&layout.list("lightanims")) {
            out.rows.insert("lightanim", rows_of(&v, "lightanims"));
        }
        if let Some(v) = read_lua(&layout.list("sound_envs")) {
            out.rows.insert("sound_env", rows_of(&v, "sound_envs"));
        }
        // a level's row is the head of its passport
        let mut levels = Vec::new();
        if let Ok(rd) = std::fs::read_dir(layout.root.join("levels")) {
            let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            for d in dirs {
                if let Some(v) = read_lua(&d.join("passport.lua")) {
                    let mut row = serde_json::Map::new();
                    for key in ["name", "box", "size", "counts", "ai"] {
                        if let Some(x) = v.get(key) {
                            row.insert(key.to_owned(), x.clone());
                        }
                    }
                    levels.push(Value::Object(row));
                }
            }
        }
        out.rows.insert("level", levels);
        for (kind, key) in KINDS {
            let map = out
                .rows
                .get(kind)
                .map(|rows| {
                    rows.iter()
                        .enumerate()
                        .filter_map(|(i, r)| {
                            r.get(key).and_then(Value::as_str).map(|n| (n.to_lowercase(), i))
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.by_name.insert(kind, map);
        }
        out
    }

    /// The index of the library of the game there is now, the library made or brought up to
    /// date first when it has to be.
    pub async fn of(editor: &Arc<Editor>) -> Result<Arc<Self>, CmdError> {
        let layout = GameLibrary::ensure(editor).await?;
        Ok(Self::of_layout(editor, &layout).await)
    }

    /// The index of a library as it is on the disk, without bringing it up to date.
    pub async fn of_layout(editor: &Arc<Editor>, layout: &Layout) -> Arc<Self> {
        let held: Arc<Held> = editor.services.get_or_init(Held::default);
        let stamp = std::fs::metadata(layout.state()).and_then(|m| m.modified()).ok();
        let key = (layout.root.clone(), stamp);
        let cached = |held: &Held| held.0.lock().as_ref().filter(|c| c.key == key).map(|c| c.index.clone());
        if let Some(index) = cached(&held) {
            return index;
        }
        let _loading = held.1.lock().await;
        if let Some(index) = cached(&held) {
            return index;
        }
        let l = layout.clone();
        let index = Arc::new(editor.exec.cpu(move || Self::load(&l)).await);
        *held.0.lock() = Some(Cache { key, index: index.clone() });
        index
    }

    /// Drops the loaded index: the library changed.
    pub fn forget(editor: &Editor) {
        let held: Arc<Held> = editor.services.get_or_init(Held::default);
        *held.0.lock() = None;
    }

    /// The rows of a kind.
    #[must_use]
    pub fn rows(&self, kind: &str) -> &[Value] {
        self.rows.get(kind).map_or(&[], Vec::as_slice)
    }

    /// The row of a kind by its name, any case and either slash.
    #[must_use]
    pub fn row(&self, kind: &str, name: &str) -> Option<&Value> {
        let key = name.trim().to_lowercase().replace('/', "\\");
        let i = *self.by_name.get(kind)?.get(&key)?;
        self.rows.get(kind)?.get(i)
    }

    /// Whether the library holds a thing of a kind by this name.
    #[must_use]
    pub fn has(&self, kind: &str, name: &str) -> bool {
        self.row(kind, name).is_some()
    }

    /// How many rows every kind has.
    #[must_use]
    pub fn counts(&self) -> Value {
        let m: serde_json::Map<String, Value> =
            KINDS.iter().map(|(k, _)| ((*k).to_owned(), Value::from(self.rows(k).len()))).collect();
        Value::Object(m)
    }

    /// The names of a kind nearest to a name that is not there: what a problem offers.
    #[must_use]
    pub fn nearest(&self, kind: &str, name: &str, count: usize) -> Vec<String> {
        let Some(key) = name_key(kind) else { return Vec::new() };
        let want = name.to_lowercase().replace('/', "\\");
        let leaf = want.rsplit('\\').next().unwrap_or(&want).to_owned();
        let mut hits: Vec<(usize, &str)> = self
            .rows(kind)
            .iter()
            .filter_map(|r| r.get(key).and_then(Value::as_str))
            .filter_map(|n| {
                let low = n.to_lowercase();
                let other = low.rsplit('\\').next().unwrap_or(&low);
                // the same leaf elsewhere first, then a leaf that holds it or is held by it
                if other == leaf {
                    Some((0, n))
                } else if !leaf.is_empty() && (other.contains(&leaf) || leaf.contains(other)) {
                    Some((1 + other.len().abs_diff(leaf.len()), n))
                } else {
                    None
                }
            })
            .collect();
        hits.sort();
        hits.into_iter().take(count).map(|(_, n)| n.to_owned()).collect()
    }
}
