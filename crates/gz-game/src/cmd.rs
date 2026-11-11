//! The game area's commands. The wire is the C++ handlers': `EditorProject::McpGameLink`
//! (`game_link`), `EditorGameModes::McpList` (`game_modes`), `LevelMcpScene.cpp`
//! (`list_spawn_sections`).

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_project::{Project, paths};
use serde_json::{Map, Value, json};

use crate::service::{GameLink, spawn_roster};

const AREA: &str = "gz-game";

/// The `Entities` category of `list_assets`: every section with `$spawn`, in the roster's order
/// (the C++ chooser walked `pSettings`, whose sections are sorted).
pub fn install(editor: &Editor) {
    gz_project::AssetLists::of(editor).provide_async("Entities", |ed: Arc<Editor>| async move {
        spawn_roster(&ed).await.iter().map(|s| s.section.to_string()).collect()
    });
}

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "game_link", game_link);
    reg.add(AREA, "game_modes", game_modes);
    reg.add(AREA, "list_spawn_sections", list_spawn_sections);
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// `{linked, game_root, reason}` of the open project.
async fn link_fields(ed: &Arc<Editor>, project: &Arc<Project>, reason: &str) -> Map<String, Value> {
    let p = project.clone();
    let linked = ed.exec.io(move || p.game_linked()).await;
    let mut m = Map::new();
    m.insert("linked".into(), json!(linked));
    m.insert("game_root".into(), json!(paths::wire(&project.game_root())));
    m.insert("reason".into(), json!(reason));
    m
}

/// Reports the link, or links `path`. What is wrong with a link is a reason, not a failure; a
/// link that cannot be made is.
async fn game_link(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Ok((_, project)) = Project::of(&ed) else {
        let reason = "no project is open";
        let fields = Map::from_iter([
            ("linked".to_owned(), json!(false)),
            ("game_root".to_owned(), json!("")),
            ("reason".to_owned(), json!(reason)),
        ]);
        return Err(refuse(reason).with_data(Value::Object(fields)));
    };
    if let Some(path) = args.text("path").map(paths::to_backslash) {
        let current = project.game_root();
        let other = current.is_empty() || !paths::same_path(&current, &paths::trim_trailing(path.clone()));
        if other && gz_project::ProjectHooks::of(&ed).live_scene(&ed) {
            let reason = "close or clear the current scene before linking a different game".to_owned();
            let fields = link_fields(&ed, &project, &reason).await;
            return Err(refuse(reason).with_data(Value::Object(fields)));
        }
        return match project.link_game(&path).await {
            Ok(()) => Ok(Value::Object(link_fields(&ed, &project, "").await)),
            Err(reason) => {
                let fields = link_fields(&ed, &project, &reason).await;
                Err(refuse(reason).with_data(Value::Object(fields)))
            }
        };
    }
    let root = project.game_root();
    let reason = if root.is_empty() {
        "no game linked".to_owned()
    } else {
        ed.exec.io(move || gz_vfs::looks_like_game(&root).err().unwrap_or_default().to_owned()).await
    };
    Ok(Value::Object(link_fields(&ed, &project, &reason).await))
}

/// `{modes: [{id, caption, key, source}]}`; a scan that could not read everything answers
/// `ok:false` with its reason and whatever it found.
async fn game_modes(ed: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    let scan = match GameLink::of(&ed) {
        Ok(link) => link.modes().await,
        Err(e) => Arc::new(crate::ModeScan { modes: Vec::new(), error: Some(e.to_string()) }),
    };
    let modes: Vec<Value> = scan
        .modes
        .iter()
        .map(|m| json!({ "id": paths::wire(&m.id), "caption": paths::wire(&m.caption), "key": paths::wire(&m.key), "source": paths::wire(&m.source) }))
        .collect();
    match &scan.error {
        None => Ok(json!({ "modes": modes })),
        Some(e) => Err(refuse(paths::wire(e)).with_data(json!({ "modes": modes }))),
    }
}

/// `{sections: [{section, group, class}], shown, total}`: the roster of the linked game (the
/// editor's own without one), `filter` a substring of `"<section> <group>"`, any case.
async fn list_spawn_sections(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let filter = args.str("filter").unwrap_or_default().to_lowercase();
    let limit = args.i64("limit").unwrap_or(200);
    let roster = spawn_roster(&ed).await;
    let (mut total, mut shown) = (0i64, 0i64);
    let mut sections = Vec::new();
    for s in roster.iter() {
        if !filter.is_empty() && !format!("{} {}", s.section, s.group).to_lowercase().contains(&filter) {
            continue;
        }
        total += 1;
        if shown >= limit {
            continue;
        }
        sections.push(json!({ "section": paths::wire(&s.section), "group": paths::wire(&s.group), "class": paths::wire(&s.class) }));
        shown += 1;
    }
    Ok(json!({ "sections": sections, "shown": shown, "total": total }))
}
