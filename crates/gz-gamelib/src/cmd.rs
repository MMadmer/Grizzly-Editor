//! The library's commands (`docs/GAME_LIBRARY.md`, 10): `game.library`, `asset.find`,
//! `asset.card`, `game.read`, `game.search`.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::index::{Index, KINDS, name_key};
use crate::service::GameLibrary;
use crate::sync::Options;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LibraryParams {
    /// `status` (left out): the library's folder, state and counts, answered at once. `sync`:
    /// brings the library up to date with the game and waits for it.
    #[serde(default)]
    action: Option<String>,
    /// With sync: every description is made again, whatever the library holds.
    #[serde(default)]
    rebuild: bool,
    /// With sync: only these kinds - models, textures, sounds, lists, levels.
    #[serde(default)]
    kinds: Option<Vec<String>>,
    /// With sync: answer the job at once instead of waiting for it (`job.wait` follows it).
    #[serde(default)]
    wait: Option<bool>,
}

async fn game_library(ed: Arc<Editor>, p: LibraryParams, _o: Origin) -> Reply {
    match p.action.as_deref().map(str::trim).unwrap_or("status") {
        "" | "status" => {
            let mut out = GameLibrary::status(&ed);
            // the counts of a library that is there, without bringing it up to date
            if out.get("state").and_then(Value::as_str) == Some("ready")
                && let Some(layout) = GameLibrary::layout(&ed)
            {
                let index = Index::of_layout(&ed, &layout).await;
                if let Some(m) = out.as_object_mut() {
                    m.insert("counts".into(), index.counts());
                }
            }
            Ok(out)
        }
        "sync" => {
            if let Some(kinds) = &p.kinds {
                const KNOWN: [&str; 5] = ["models", "textures", "sounds", "lists", "levels"];
                if let Some(bad) = kinds.iter().find(|k| !KNOWN.contains(&k.as_str())) {
                    return Err(CmdError::invalid(format!(
                        "kinds takes models, textures, sounds, lists and levels, not '{bad}'"
                    )));
                }
            }
            let opts = Options { rebuild: p.rebuild, kinds: p.kinds, only: None };
            if p.wait == Some(false) {
                let id = GameLibrary::start(&ed, opts)?;
                return Ok(json!({ "job": id.0, "state": "syncing" }));
            }
            let report = GameLibrary::sync(&ed, opts).await?;
            let mut out = GameLibrary::status(&ed);
            if let Some(m) = out.as_object_mut() {
                m.insert("sync".into(), report);
            }
            Ok(out)
        }
        other => Err(CmdError::invalid(format!("action is status or sync, not '{other}'"))),
    }
}

/// The most rows one `asset.find` answers.
const MAX_ROWS: usize = 500;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindParams {
    /// What to look for: model, texture, sound, particle, section, material, compiler, shader,
    /// lightanim, sound_env or level.
    kind: String,
    /// Only rows whose name holds this text (any case); several words must all be there.
    #[serde(default)]
    query: Option<String>,
    /// Models and sections with a model: the longest side lies between these metres,
    /// `[min, max]`.
    #[serde(default)]
    size: Option<[f32; 2]>,
    /// Models: only those that stop a body (true) or let it through (false).
    #[serde(default)]
    collides: Option<bool>,
    /// Models: only this physics - none, rigid or jointed.
    #[serde(default)]
    physics: Option<String>,
    /// Sections: only this kind, by the word a row's `kind` says (npc, monster, anomaly, item,
    /// weapon, zone, restrictor, ...). A word no section has is refused with the ones there are.
    #[serde(default)]
    class: Option<String>,
    /// `name` (left out) or `size` (models: the largest first).
    #[serde(default)]
    sort: Option<String>,
    /// At most this many rows (50; 500 at most); `total` counts them all.
    #[serde(default)]
    limit: Option<usize>,
    /// Rows to pass over first.
    #[serde(default)]
    offset: Option<usize>,
}

/// The longest side of what a row is: a model's own size, a section's by the model it wears.
fn longest_side(index: &Index, row: &Value) -> Option<f64> {
    let size = match row.get("size") {
        Some(size) => size,
        None => index.row("model", row.get("visual")?.as_str()?)?.get("size")?,
    };
    size.as_array()?
        .iter()
        .filter_map(Value::as_f64)
        .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
}

/// A filter's word must be one the rows have under `key`: an empty answer to a word that does
/// not exist reads as "the game has none".
fn known_word(rows: &[Value], key: &str, word: &str, what: &str) -> Result<(), CmdError> {
    let mut words: Vec<&str> = rows.iter().filter_map(|r| r.get(key).and_then(Value::as_str)).collect();
    words.sort_unstable();
    words.dedup();
    if words.contains(&word) {
        return Ok(());
    }
    Err(CmdError::invalid(format!("{what} is one of {}, not '{word}'", words.join(", "))))
}

async fn asset_find(ed: Arc<Editor>, p: FindParams, _o: Origin) -> Reply {
    let kind = p.kind.trim().to_ascii_lowercase();
    let Some(key) = name_key(&kind) else {
        let all: Vec<&str> = KINDS.iter().map(|k| k.0).collect();
        return Err(CmdError::invalid(format!("kind is one of {}, not '{}'", all.join(", "), p.kind)));
    };
    let by_size = match p.sort.as_deref().map(str::trim) {
        None | Some("" | "name") => false,
        Some("size") => true,
        Some(other) => return Err(CmdError::invalid(format!("sort is name or size, not '{other}'"))),
    };
    let index = Index::of(&ed).await?;
    if let Some(class) = p.class.as_deref().map(str::trim) {
        if kind != "section" {
            return Err(CmdError::invalid("class is a filter of sections: kind=section"));
        }
        known_word(index.rows(&kind), "kind", class, "class")?;
    }
    if let Some(physics) = p.physics.as_deref().map(str::trim) {
        if kind != "model" {
            return Err(CmdError::invalid("physics is a filter of models: kind=model"));
        }
        known_word(index.rows(&kind), "physics", physics, "physics")?;
    }
    let words: Vec<String> = p
        .query
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .map(|w| w.to_lowercase().replace('/', "\\"))
        .collect();
    let mut hits: Vec<&Value> = index
        .rows(&kind)
        .iter()
        .filter(|r| {
            let name = r.get(key).and_then(Value::as_str).unwrap_or("").to_lowercase();
            // a section is also found by what the player reads
            let title = r.get("title").and_then(Value::as_str).unwrap_or("").to_lowercase();
            words.iter().all(|w| name.contains(w.as_str()) || title.contains(w.as_str()))
        })
        .filter(|r| match p.size {
            Some([min, max]) => {
                longest_side(&index, r).is_some_and(|s| s >= f64::from(min) && s <= f64::from(max))
            }
            None => true,
        })
        .filter(|r| p.collides.is_none_or(|c| r.get("collides").and_then(Value::as_bool) == Some(c)))
        .filter(|r| {
            p.physics.as_deref().is_none_or(|ph| r.get("physics").and_then(Value::as_str) == Some(ph.trim()))
        })
        .filter(|r| {
            p.class.as_deref().is_none_or(|c| r.get("kind").and_then(Value::as_str) == Some(c.trim()))
        })
        .collect();
    if by_size {
        hits.sort_by_cached_key(|r| {
            std::cmp::Reverse((longest_side(&index, r).unwrap_or(0.0) * 1000.0) as i64)
        });
    } else {
        hits.sort_by(|a, b| {
            let n = |r: &Value| r.get(key).and_then(Value::as_str).unwrap_or("").to_lowercase();
            n(a).cmp(&n(b))
        });
    }
    let total = hits.len();
    let limit = p.limit.unwrap_or(50).min(MAX_ROWS);
    let mut rows: Vec<Value> = hits.into_iter().skip(p.offset.unwrap_or(0)).take(limit).cloned().collect();
    // a section wears a model: how large what it spawns is, from the model's own card
    if kind == "section" {
        for row in &mut rows {
            let size = row
                .get("visual")
                .and_then(Value::as_str)
                .and_then(|visual| index.row("model", visual))
                .and_then(|model| model.get("size"))
                .cloned();
            if let (Some(size), Some(m)) = (size, row.as_object_mut()) {
                m.insert("size".into(), size);
            }
        }
    }
    Ok(json!({ "kind": kind, "total": total, "rows": rows }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CardParams {
    /// The asset: a model (`dynamics\box\box_wood_01.ogf`, a library object, a model asset of
    /// the project), a texture, a sound, a particle effect, a spawn section, a game material,
    /// a light animation, a sound environment or a level, by the name the game or a scene uses.
    #[serde(rename = "ref")]
    reference: String,
    /// The kind, when a name is one of several (a texture and a sound of one name): as
    /// `asset.find` names them.
    #[serde(default)]
    kind: Option<String>,
}

fn read_lua(path: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(gz_luadata::to_json(&gz_luadata::parse(&text).ok()?))
}

/// The card of a model the scene's library holds or makes: a library object, a model asset of
/// the project, a compiled model placed by its name.
async fn scene_model_card(ed: &Arc<Editor>, reference: &str) -> Option<Value> {
    let svc = gz_scene::SceneService::of(ed).ok()?;
    let lib = svc.library.clone();
    let r = reference.to_owned();
    ed.exec
        .io(move || {
            // a model the scene does not hold is read into a library of the card's own
            let lo = match lib.cached(&r) {
                Some(lo) => lo,
                None => lib.fork().load(&r).ok()?,
            };
            let uses = lib.uses(&lo);
            let (_, facts) = crate::models::facts_json(&lo, &uses, &gz_scene::facts::Limits::default());
            let source = if lo.file.as_os_str().is_empty() { "made" } else { "library" };
            Some(json!({ "kind": "model", "ref": r, "source": source, "facts": facts }))
        })
        .await
}

async fn asset_card(ed: Arc<Editor>, p: CardParams, _o: Origin) -> Reply {
    let name = p.reference.trim().to_owned();
    if name.is_empty() {
        return Err(CmdError::invalid("ref names no asset"));
    }
    let wanted = match p.kind.as_deref().map(|k| k.trim().to_ascii_lowercase()) {
        Some(k) if name_key(&k).is_none() => {
            let all: Vec<&str> = KINDS.iter().map(|k| k.0).collect();
            return Err(CmdError::invalid(format!("kind is one of {}, not '{k}'", all.join(", "))));
        }
        other => other,
    };
    let index = Index::of(&ed).await?;
    let layout = GameLibrary::layout(&ed).ok_or_else(|| CmdError::new("no_game", "no game is linked"))?;
    // a compiled model of the game: its whole card
    let low = name.to_lowercase().replace('/', "\\");
    let model_ref = if low.ends_with(".ogf") { low.clone() } else { format!("{low}.ogf") };
    if wanted.as_deref().is_none_or(|k| k == "model")
        && index.has("model", &model_ref)
        && let Some(card) = read_lua(&layout.model(model_ref.trim_end_matches(".ogf")))
    {
        return Ok(json!({ "kind": "model", "ref": model_ref, "source": "game", "card": card }));
    }
    for (kind, key) in KINDS {
        if kind == "model" || wanted.as_deref().is_some_and(|k| k != kind) {
            continue;
        }
        if kind == "level" {
            if index.has("level", &low)
                && let Some(passport) = read_lua(&layout.level(&low).join("passport.lua"))
            {
                return Ok(json!({ "kind": "level", "name": low, "source": "game", "card": passport }));
            }
            continue;
        }
        if let Some(row) = index.row(kind, &name) {
            return Ok(json!({ "kind": kind, key: row.get(key), "source": "game", "card": row }));
        }
    }
    // what the scene's library holds or makes
    if wanted.as_deref().is_none_or(|k| k == "model")
        && let Some(card) = scene_model_card(&ed, &name).await
    {
        return Ok(card);
    }
    let near: Vec<String> = match &wanted {
        Some(k) => index.nearest(k, &name, 5),
        None => ["model", "texture", "sound", "section", "particle"]
            .iter()
            .flat_map(|k| index.nearest(k, &name, 2))
            .collect(),
    };
    Err(CmdError::new("missing", format!("nothing is called '{name}'"))
        .with_data(json!({ "nearest": near, "hint": "asset.find kind=<kind> query=<text> searches by a part of the name" })))
}

/// The most lines one `game.read` answers.
const MAX_LINES: usize = 2000;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadParams {
    /// The file under the game's `gamedata`: `configs\scripts\agroprom\smart\x.ltx`,
    /// `scripts\xr_logic.script`. A folder lists what it holds.
    path: String,
    /// The first line to answer, 1-based (1).
    #[serde(default)]
    from: Option<usize>,
    /// At most this many lines (400; 2000 at most).
    #[serde(default)]
    lines: Option<usize>,
}

fn game_path(p: &str) -> String {
    p.trim().trim_matches(['\\', '/']).replace('\\', "/").to_lowercase()
}

async fn game_read(ed: Arc<Editor>, p: ReadParams, _o: Origin) -> Reply {
    let (_, files) = GameLibrary::files(&ed).await?;
    let path = game_path(&p.path);
    if files.is_dir(&path) || path.is_empty() {
        let mut folders = Vec::new();
        let mut list = Vec::new();
        for l in files.list(&path) {
            match l {
                gz_vfs::Listed::Dir(name) => folders.push(name.to_owned()),
                gz_vfs::Listed::File(f) => list.push(json!({ "name": f.name(), "size": f.size })),
            }
        }
        return Ok(json!({ "path": path.replace('/', "\\"), "folders": folders, "files": list }));
    }
    let Some(f) = files.get(&path) else {
        return Err(CmdError::new("missing", format!("the game has no file {}", path.replace('/', "\\"))));
    };
    if f.size > 16 * 1024 * 1024 {
        return Err(CmdError::invalid(format!("{} is {} bytes: too large to read as text", p.path, f.size)));
    }
    let (fs, key) = (files.clone(), path.clone());
    let bytes =
        ed.exec.io(move || fs.read(&key)).await.map_err(|e| CmdError::new("failed", e.to_string()))?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(CmdError::invalid(format!(
            "{} is no text file: asset.card answers what a model, a texture or a sound is",
            p.path
        )));
    }
    let text = gz_base::text::decode_guess(&bytes);
    let all: Vec<&str> = text.lines().collect();
    let from = p.from.unwrap_or(1).max(1);
    let count = p.lines.unwrap_or(400).min(MAX_LINES);
    let shown: Vec<&str> = all.iter().skip(from - 1).take(count).copied().collect();
    Ok(json!({
        "path": path.replace('/', "\\"),
        "lines": all.len(),
        "from": from,
        "to": from + shown.len().saturating_sub(1),
        "text": shown.join("\n"),
    }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchParams {
    /// The text to find (any case).
    text: String,
    /// Where: a folder under the game's `gamedata` (`configs`, `scripts`, `configs\scripts\agroprom`).
    /// Left out: `configs` and `scripts`.
    #[serde(default)]
    r#in: Option<String>,
    /// At most this many matching lines (100; 1000 at most); `total` counts them all.
    #[serde(default)]
    limit: Option<usize>,
}

async fn game_search(ed: Arc<Editor>, p: SearchParams, _o: Origin) -> Reply {
    let needle = p.text.to_lowercase();
    if needle.trim().is_empty() {
        return Err(CmdError::invalid("text is empty"));
    }
    let (_, files) = GameLibrary::files(&ed).await?;
    let roots: Vec<String> = match p.r#in.as_deref().map(game_path).filter(|s| !s.is_empty()) {
        Some(r) => vec![format!("{r}/")],
        None => vec!["configs/".to_owned(), "scripts/".to_owned()],
    };
    let limit = p.limit.unwrap_or(100).min(1000);
    let found = ed
        .exec
        .cpu(move || {
            use rayon::prelude::*;
            const TEXT: [&str; 6] = [".ltx", ".script", ".xml", ".lua", ".txt", ".ini"];
            let list: Vec<gz_vfs::FileRef<'_>> = files
                .iter()
                .filter(|f| roots.iter().any(|r| f.path.starts_with(r.as_str())))
                .filter(|f| TEXT.iter().any(|e| f.path.ends_with(e)) && f.size < 8 * 1024 * 1024)
                .collect();
            let mut list = list;
            list.sort_by(|a, b| a.path.cmp(b.path));
            let lines_of = |f: &gz_vfs::FileRef<'_>| -> Vec<(usize, String)> {
                let Ok(bytes) = files.read_ref(*f) else { return Vec::new() };
                gz_base::text::decode_guess(&bytes)
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.to_lowercase().contains(&needle))
                    .map(|(i, l)| (i + 1, l.trim().chars().take(240).collect()))
                    .collect()
            };
            // counted first, on every core, nothing kept: a text half the game's lines hold
            // would be all of them in memory
            let counts: Vec<usize> = list.par_iter().map(|f| lines_of(f).len()).collect();
            let total: usize = counts.iter().sum();
            // then the lines of the first files, by path, up to what is answered
            let mut hits: Vec<(String, usize, String)> = Vec::new();
            for (f, _) in list.iter().zip(&counts).filter(|(_, n)| **n > 0) {
                if hits.len() >= limit {
                    break;
                }
                let file = f.path.replace('/', "\\");
                hits.extend(lines_of(f).into_iter().map(|(line, text)| (file.clone(), line, text)));
            }
            hits.truncate(limit);
            (list.len(), total, hits)
        })
        .await;
    let (searched, total, hits) = found;
    let rows: Vec<Value> = hits
        .into_iter()
        .map(|(file, line, text)| json!({ "file": file, "line": line, "text": text }))
        .collect();
    Ok(json!({ "total": total, "files_searched": searched, "matches": rows }))
}

pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::write(
            "game.library",
            "The game as the editor's library: where its descriptions lie and what state they are in; action=sync brings them up to date with the game.",
        )
        .example("action=sync"),
        game_library,
    );
    reg.cmd(
        Spec::read(
            "game.read",
            "Reads a text file of the linked game - a config, a script, an XML - by lines, or lists a folder of it.",
        )
        .example("path=configs\\scripts\\agroprom\\smart\\agr_smart_terrain_4_4.ltx"),
        game_read,
    );
    reg.cmd(
        Spec::read(
            "game.search",
            "Finds a text in the linked game's configs and scripts: the files and lines that hold it.",
        )
        .example("text=agr_smart_terrain_4_4 in=configs"),
        game_search,
    );
    reg.cmd(
        Spec::read(
            "asset.find",
            "Searches the game's assets by kind, name and what they are: models by size and physics, textures, sounds, particle effects, spawn sections, materials, light animations, sound environments, levels.",
        )
        .example("kind=model query=box size=[0.4,1.5] physics=rigid"),
        asset_find,
    );
    reg.cmd(
        Spec::read(
            "asset.card",
            "Everything known of one asset: a model's box, pivot, collision, bones, the faces a thing can stand on; a texture's size and colour; a sound's length and distances; a level's passport.",
        )
        .example("ref=dynamics\\box\\box_wood_01.ogf"),
        asset_card,
    );
}
