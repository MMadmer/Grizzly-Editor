//! The build's command. The wire is the C++ `EditorMod::McpExport` (reached from
//! `LevelMcpMod.cpp`): `mod_export` answers once its job has ended (`FollowsJob`), while every
//! other command goes on meanwhile.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_project::Project;
use serde_json::json;

use crate::job::{BuildKind, BuildRequest, BuildResult, Builds};

const AREA: &str = "gz-build";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_export", mod_export);
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// The answer of a build that was held until it ended.
fn answer(r: Option<Arc<BuildResult>>) -> Reply {
    let Some(r) = r else { return Err(refuse("")) };
    if !r.ok {
        return Err(refuse(r.err.clone()));
    }
    // an agent gets the same warnings the dialog shows: a build that shipped a quest naming
    // something that does not exist is not a clean build
    Ok(json!({
        "files": r.files,
        "path": r.path.replace('\\', "/"),
        "baked": r.bake_note,
        "warnings": r.warnings,
        "left_out": r.left_out,
    }))
}

async fn mod_export(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    if args.flag("answer_only") {
        return answer(Builds::of(&ed).last());
    }
    let (_session, project) = Project::of(&ed)?;
    // no target = deploy to the linked game: <game>\modules\<id> is where the game actually
    // reads modules from, so it is the default, not an option
    let target = match args.text("target") {
        Some(t) => t.to_owned(),
        None => {
            let game = project.game_root();
            if game.is_empty() {
                return Err(refuse("no 'target' and no linked game (see game_link)"));
            }
            game
        }
    };
    let kind = if args.flag("flat") { BuildKind::Flat } else { BuildKind::ToFolder };
    // MCP is the programmatic path: the caller already decided, and it must never stall on a
    // question
    let id = Builds::start(&ed, BuildRequest { kind, target, clean: true, remember_target: false })?;
    ed.jobs.wait(id).await;
    answer(Builds::of(&ed).result(id))
}
