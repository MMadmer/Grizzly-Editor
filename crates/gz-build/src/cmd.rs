//! `mod.info` - the manifest as data, its problems and what a build would ship and leave out -
//! and `mod.build`: the build as a job the command waits for (`wait=false` answers the job,
//! `job.*` follows it), while every other command goes on meanwhile.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use gz_mod::{manifest_answer, open_manifest};
use gz_project::Project;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::job::{BuildKind, BuildRequest, BuildResult, Builds};
use crate::walk::{BuildWalk, WalkScope};

pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::read(
            "mod.info",
            "The module's manifest as data, its problems, and what a build would ship and leave out.",
        )
        .example("files=true"),
        mod_info,
    );
    reg.cmd(
        Spec::write(
            "mod.build",
            "Builds the module into the linked game, a folder (modules\\<id>) or a flat gamedata_<id>; answers when the build ended.",
        )
        .example("target=folder path=D:/Builds"),
        mod_build,
    );
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

// ---- mod.info ---------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InfoParams {
    /// Also list every file a build would ship (project-relative); the count is always there.
    #[serde(default)]
    files: bool,
}

async fn mod_info(ed: Arc<Editor>, p: InfoParams, _origin: Origin) -> Reply {
    let (project, service) = open_manifest(&ed).await?;
    let mut answer = manifest_answer(&ed, &project, &service).await?;
    let snap = service.snapshot();
    let rules = snap.manifest().map(|m| m.exclude.clone()).unwrap_or_default();
    let tree = project.tree.snapshot();
    let exec = ed.exec.clone();
    let walk = ed.exec.io(move || BuildWalk::scan(&tree, &rules, WalkScope::Module, &exec)).await;
    let mut build = json!({
        "ships": walk.files.len(),
        "bytes": walk.files.iter().map(|f| f.size).sum::<u64>(),
        "left_out": walk.left_out_texts(),
    });
    if p.files
        && let Value::Object(map) = &mut build
    {
        let files: Vec<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
        map.insert("files".into(), json!(files));
    }
    if let Value::Object(map) = &mut answer {
        map.insert("build".into(), build);
    }
    Ok(answer)
}

// ---- mod.build --------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BuildTarget {
    /// Into the linked game: `<game>\modules\<id>`.
    #[default]
    Game,
    /// Into a folder: `<path>\modules\<id>`.
    Folder,
    /// A flat, JSGME-style `<path>\gamedata_<id>`.
    Flat,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BuildParams {
    /// Where the module goes.
    #[serde(default)]
    target: BuildTarget,
    /// The folder for `folder` and `flat` (absolute). Left out with `folder`: the folder the
    /// window last built to.
    #[serde(default)]
    path: Option<String>,
    /// The files in the target this build does not write go (true), or the build refuses and
    /// names them (false).
    #[serde(default = "yes")]
    clean: bool,
    /// Wait for the build to end (true) or answer its job at once (false; `job.*` follows it).
    #[serde(default = "yes")]
    wait: bool,
}

const fn yes() -> bool {
    true
}

/// How the window builds when a person asks: the clean-target question and the result dialog
/// are the window's. The surface that owns them installs it (`editor.services`).
pub type WindowBuild = Arc<dyn Fn(&Arc<Editor>) -> bool + Send + Sync>;

/// The window's Build Into Game, when there is a window.
#[derive(Default)]
pub struct WindowBuilds(Mutex<Option<WindowBuild>>);

impl WindowBuilds {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// `f` starts Build Into Game the way the window does and answers whether it could.
    pub fn provide(&self, f: impl Fn(&Arc<Editor>) -> bool + Send + Sync + 'static) {
        *self.0.lock() = Some(Arc::new(f));
    }

    fn get(&self) -> Option<WindowBuild> {
        self.0.lock().clone()
    }
}

/// The answer of a build that ended.
pub fn build_answer(r: &BuildResult) -> Reply {
    if r.cancelled {
        return Err(CmdError::new("cancelled", "the build was stopped; the target stays as it was"));
    }
    if !r.ok {
        let mut e = refuse(r.err.clone());
        if r.stale_count > 0 {
            e = e.with_data(json!({ "stale": r.stale_names, "stale_count": r.stale_count, "path": r.path.replace('\\', "/") }));
        }
        return Err(e);
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

async fn mod_build(ed: Arc<Editor>, p: BuildParams, origin: Origin) -> Reply {
    let (_session, project) = Project::of(&ed)?;
    // a person in the window: the window asks what it needs and shows the result
    if origin == Origin::Ui
        && p.target == BuildTarget::Game
        && let Some(window) = WindowBuilds::of(&ed).get()
        && window(&ed)
    {
        return Ok(json!({ "started": true }));
    }
    let (kind, target) = match p.target {
        BuildTarget::Game => {
            let game = project.game_root();
            if game.is_empty() {
                return Err(refuse("no game is linked: link one (game.link) or build to a folder"));
            }
            (BuildKind::IntoGame, game)
        }
        BuildTarget::Folder | BuildTarget::Flat => {
            let path = p
                .path
                .filter(|t| !t.trim().is_empty())
                .or_else(|| {
                    (p.target == BuildTarget::Folder)
                        .then(|| project.export_target())
                        .filter(|t| !t.is_empty())
                })
                .ok_or_else(|| {
                    CmdError::invalid("a folder or flat build needs path: the folder it goes to")
                })?;
            (if p.target == BuildTarget::Flat { BuildKind::Flat } else { BuildKind::ToFolder }, path)
        }
    };
    let id = Builds::start(&ed, BuildRequest { kind, target, clean: p.clean, remember_target: false })?;
    if !p.wait {
        return Ok(json!({ "job": id.0 }));
    }
    ed.jobs.wait(id).await;
    let r = Builds::of(&ed).result(id).ok_or_else(|| CmdError::internal("the build left no result"))?;
    build_answer(&r)
}
