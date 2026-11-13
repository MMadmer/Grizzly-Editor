//! The game area's commands: `game.link` (the project's own game or the editor's default),
//! `game.info` (which game, its version and abilities, how it was found) and `game.lookup` (every
//! list the game offers: spawn sections, modes, and the lists other areas serve through
//! [`Lookups`] - the quest editor's pickers).

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, JsonSchema, Registry, Reply, Spec};
use gz_project::{GameSource, LinkScope, Machine, Project, paths};
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::service::{GameLink, spawn_roster};

/// The `Entities` category of the library listing: every section with `$spawn`, in the roster's
/// order; the project file as a kind of asset; and the default game, looked for once at start
/// off the UI thread.
pub fn install(editor: &Arc<Editor>) {
    gz_project::AssetLists::of(editor).provide_async("Entities", |ed: Arc<Editor>| async move {
        spawn_roster(&ed).await.iter().map(|s| s.section.to_string()).collect()
    });
    // the project file is a kind of asset: asset.check and asset.reference know it
    gz_asset::Kinds::of(editor).add(crate::project_kind::kind());
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let machine = Machine::of(&ed);
        if let Err(e) = machine.load().await {
            ed.log.warn(e.message);
        }
        match machine.ensure_default_game().await {
            Ok(root) => ed.log.info(format!("Default game: {root} ({})", machine.snapshot().found().words())),
            Err(why) => ed.log.info(format!("Default game: {why}")),
        }
    });
}

pub fn register(reg: &mut Registry) {
    reg.area("game", "The game the editor reads: which one, what it is, the lists it offers");
    reg.cmd(
        Spec::write(
            "game.link",
            "Links a game: the open project's own on this machine, or the editor's default.",
        )
        .example("path=D:/Games/Dead Air Refined scope=default"),
        game_link,
    );
    reg.cmd(
        Spec::read(
            "game.info",
            "Says which game the editor reads, its version and abilities, and how it was found.",
        ),
        game_info,
    );
    reg.cmd(
        Spec::read(
            "game.lookup",
            "Searches one of the lists the game offers: spawn sections, modes, the pickers' lists.",
        )
        .example("type=item_section query=medkit limit=10"),
        game_lookup,
    );
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// Which game a link sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Scope {
    /// The open project's own game, on this machine.
    Project,
    /// The editor's default game, for every project that names none.
    Default,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LinkParams {
    /// The game's folder (it holds fsgame.ltx). Left out with scope=project: the project goes
    /// back to the default game.
    #[serde(default)]
    path: Option<String>,
    /// Whose game it is; left out: the open project's, or the default without a project.
    #[serde(default)]
    scope: Option<Scope>,
    /// Look for the default game again, whatever it is now.
    #[serde(default)]
    find: bool,
}

async fn game_link(ed: Arc<Editor>, p: LinkParams, _origin: Origin) -> Reply {
    let machine = Machine::of(&ed);
    let project = Project::of(&ed).ok().map(|(_, p)| p);
    if p.find {
        machine.find_again().await.map_err(refuse)?;
        return info(&ed, false).await;
    }
    let scope = p.scope.unwrap_or(if project.is_some() { Scope::Project } else { Scope::Default });
    let path = p.path.map(|s| paths::to_backslash(s.trim())).filter(|s| !s.is_empty());
    match (scope, path) {
        (Scope::Default, None) => {
            return Err(CmdError::invalid("game.link scope=default needs the game's path (or find=true)"));
        }
        (Scope::Default, Some(path)) => match &project {
            Some(project) => project.link_game(&path, LinkScope::Default).await.map_err(refuse)?,
            None => {
                machine.set_default_game(&path, gz_project::Found::Preference).await.map_err(refuse)?;
            }
        },
        (Scope::Project, path) => {
            let Some(project) = &project else { return Err(CmdError::no_project()) };
            let current = project.game_root();
            let next = path.clone().unwrap_or_else(|| machine.default_game());
            let other = current.is_empty() || !paths::same_path(&current, &paths::trim_trailing(next));
            if other && gz_project::ProjectHooks::of(&ed).live_scene(&ed) {
                return Err(refuse("close or clear the current scene before linking a different game"));
            }
            match path {
                Some(path) => project.link_game(&path, LinkScope::Project).await.map_err(refuse)?,
                None => project.use_default_game().await.map_err(refuse)?,
            }
        }
    }
    info(&ed, false).await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InfoParams {
    /// Also every place a game was looked for, each with why it is or is not one.
    #[serde(default)]
    candidates: bool,
}

async fn game_info(ed: Arc<Editor>, p: InfoParams, _origin: Origin) -> Reply {
    info(&ed, p.candidates).await
}

/// `{root, source, valid, reason, version, abilities, default, default_found[, candidates]}`.
async fn info(ed: &Arc<Editor>, candidates: bool) -> Reply {
    let machine = Machine::of(ed);
    let snap = machine.snapshot();
    let (root, source) = match Project::of(ed) {
        Ok((_, p)) => p.game_source(),
        Err(_) => (snap.game_root(), GameSource::Default(snap.found())),
    };
    let probe = root.clone();
    let (reason, version, abilities) = ed
        .exec
        .io(move || {
            let reason = gz_vfs::looks_like_game(&probe).err().unwrap_or_default().to_owned();
            let exe = std::fs::read(Path::new(&probe).join(gz_project::games::ENGINE)).unwrap_or_default();
            (reason, crate::version::exe_version(&exe), crate::version::engine_abilities(&exe))
        })
        .await;
    let mut out = Map::new();
    out.insert("root".into(), json!(paths::wire(&root)));
    out.insert("source".into(), json!(source.key()));
    out.insert("valid".into(), json!(reason.is_empty()));
    out.insert("reason".into(), json!(reason));
    out.insert("version".into(), json!(version.unwrap_or_default()));
    out.insert("abilities".into(), json!(abilities));
    out.insert("default".into(), json!(paths::wire(&snap.game_root())));
    out.insert("default_found".into(), json!(snap.found().key()));
    if candidates {
        let found: Vec<Value> = machine
            .candidates()
            .await
            .into_iter()
            .map(|c| json!({ "root": paths::wire(&c.root), "found": c.found.key(), "valid": c.valid(), "reason": c.problem.unwrap_or_default() }))
            .collect();
        out.insert("candidates".into(), Value::Array(found));
    }
    Ok(Value::Object(out))
}

// ---- the lists the game offers --------------------------------------------------------------------

/// One entry of a list: the id, its caption (may be empty) and where it comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupEntry {
    pub id: String,
    pub name: String,
    pub extra: String,
}

/// What a list provider is asked.
#[derive(Clone, Debug, Default)]
pub struct LookupQuery {
    /// The list's name (`item_section`, `community`, ...).
    pub ty: String,
    /// Only entries that match, as the provider ranks them.
    pub query: String,
    /// The document a document-scoped list is about (a quest's tasks).
    pub path: Option<String>,
    /// How many entries are wanted at most.
    pub want: usize,
}

/// What a list provider answers.
#[derive(Clone, Debug, Default)]
pub struct LookupAnswer {
    /// The list could be read (a game is known, the document reads).
    pub available: bool,
    /// How many entries match.
    pub total: usize,
    pub entries: Vec<LookupEntry>,
}

type LookupFuture = Pin<Box<dyn Future<Output = Result<LookupAnswer, CmdError>> + Send>>;
type LookupFn = Arc<dyn Fn(Arc<Editor>, LookupQuery) -> LookupFuture + Send + Sync>;

/// The lists other areas serve through `game.lookup`, by name. An `Editor` service.
#[derive(Default)]
pub struct Lookups {
    lists: RwLock<Vec<(Vec<&'static str>, LookupFn)>>,
}

impl Lookups {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Lookups> {
        editor.services.get_or_init(Lookups::default)
    }

    /// Serves the lists `types` with `f`.
    pub fn provide<F>(
        &self,
        types: impl IntoIterator<Item = &'static str>,
        f: impl Fn(Arc<Editor>, LookupQuery) -> F + Send + Sync + 'static,
    ) where
        F: Future<Output = Result<LookupAnswer, CmdError>> + Send + 'static,
    {
        let types = types.into_iter().collect();
        self.lists.write().push((types, Arc::new(move |ed, q| Box::pin(f(ed, q)))));
    }

    fn get(&self, ty: &str) -> Option<LookupFn> {
        self.lists.read().iter().rev().find(|(types, _)| types.contains(&ty)).map(|(_, f)| f.clone())
    }

    /// Every list's name, the game's own first.
    #[must_use]
    pub fn types(&self) -> Vec<&'static str> {
        let mut out = vec!["spawn_section", "mode"];
        for (types, _) in self.lists.read().iter() {
            for t in types {
                if !out.contains(t) {
                    out.push(t);
                }
            }
        }
        out
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LookupParams {
    /// The list: spawn_section, mode, or a picker's list (item_section, squad_section, level,
    /// smart, story_id, profile, community, info, spot_type, character_id, quest_id, ...);
    /// `help about=game.lookup` and a call without it name them all.
    #[serde(rename = "type", default)]
    ty: Option<String>,
    /// Only entries that hold this text, ranked by the list.
    #[serde(default)]
    query: Option<String>,
    /// The document a document-scoped list is about (task_id, var_name, ref_name, node_id, role).
    #[serde(default)]
    path: Option<String>,
    /// At most this many (50 when left out).
    #[serde(default)]
    limit: Option<usize>,
    /// Skip this many first.
    #[serde(default)]
    offset: Option<usize>,
}

fn contains_no_case(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

async fn game_lookup(ed: Arc<Editor>, p: LookupParams, _origin: Origin) -> Reply {
    let lookups = Lookups::of(&ed);
    let Some(ty) = p.ty.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) else {
        return Err(CmdError::invalid(format!("game.lookup needs a type: {}", lookups.types().join(", "))));
    };
    let query = p.query.unwrap_or_default();
    let (offset, limit) = (p.offset.unwrap_or(0), p.limit.unwrap_or(50).max(1));
    let answer = match ty.as_str() {
        "spawn_section" => {
            let roster = spawn_roster(&ed).await;
            let all: Vec<LookupEntry> = roster
                .iter()
                .filter(|s| {
                    query.is_empty() || contains_no_case(&format!("{} {}", s.section, s.group), &query)
                })
                .map(|s| LookupEntry {
                    id: s.section.to_string(),
                    name: s.group.to_string(),
                    extra: s.class.to_string(),
                })
                .collect();
            LookupAnswer { available: true, total: all.len(), entries: all }
        }
        "mode" => {
            let scan = match GameLink::of(&ed) {
                Ok(link) => link.modes().await,
                Err(e) => Arc::new(crate::ModeScan { modes: Vec::new(), error: Some(e.to_string()) }),
            };
            let all: Vec<LookupEntry> = scan
                .modes
                .iter()
                .filter(|m| {
                    query.is_empty()
                        || contains_no_case(&m.id, &query)
                        || contains_no_case(&m.caption, &query)
                })
                .map(|m| LookupEntry { id: m.id.clone(), name: m.caption.clone(), extra: m.source.clone() })
                .collect();
            LookupAnswer { available: scan.error.is_none(), total: all.len(), entries: all }
        }
        other => {
            let Some(provider) = lookups.get(other) else {
                return Err(CmdError::invalid(format!(
                    "game.lookup has no list '{other}': {}",
                    lookups.types().join(", ")
                )));
            };
            provider(ed.clone(), LookupQuery { ty: ty.clone(), query, path: p.path, want: offset + limit })
                .await?
        }
    };
    let entries: Vec<Value> = answer
        .entries
        .iter()
        .skip(offset)
        .take(limit)
        .map(|e| {
            let mut o = Map::new();
            o.insert("id".into(), json!(e.id));
            if !e.name.is_empty() {
                o.insert("name".into(), json!(e.name));
            }
            if !e.extra.is_empty() {
                o.insert("extra".into(), json!(e.extra));
            }
            Value::Object(o)
        })
        .collect();
    Ok(json!({ "type": ty, "available": answer.available, "total": answer.total, "entries": entries }))
}
