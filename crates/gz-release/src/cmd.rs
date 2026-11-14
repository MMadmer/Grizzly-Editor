//! The commands: `mod.package` (the release files, a job the command waits for), `mod.thumbnail`
//! (the Mod Browser's picture) and `release.key` (the author key). Like every command they ask
//! nothing; a key password goes in and never comes out.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use gz_mod::{manifest_answer, open_manifest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::keys::{AuthorKeys, KeyState};
use crate::package::{PackageRequest, PackageResult, Releases};

pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::publish(
            "mod.package",
            "Packages a release: the descriptor, the file index and the zip the game's updater installs from.",
        )
        .example("path=releases/1.2.0"),
        mod_package,
    );
    reg.cmd(
        Spec::write(
            "mod.thumbnail",
            "Makes the Mod Browser's thumbnail.dds from a picture, or removes the one the editor made.",
        )
        .example("from=D:/Art/cover.png"),
        mod_thumbnail,
    );
    reg.area(
        "release",
        "GitHub releases and the in-game Mod Browser: publish, list, edit, delete, the catalog, the keys",
    );
    reg.cmd(
        Spec::write(
            "release.key",
            "The author key that signs releases: its state, a new key, another key file, opened with its password or locked.",
        )
        .example("action=open password=..."),
        release_key,
    );
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

fn wire(path: &str) -> String {
    path.replace('\\', "/")
}

/// The answer of a packaging that ended.
pub fn package_answer(r: &PackageResult) -> Reply {
    if r.cancelled {
        return Err(CmdError::new("cancelled", "the packaging was stopped; nothing was written"));
    }
    if !r.ok {
        return Err(refuse(r.err.clone()));
    }
    let catalog: Vec<Value> = r
        .catalog_block
        .iter()
        .map(|t| json!({ "level": "block", "text": t }))
        .chain(r.catalog_review.iter().map(|t| json!({ "level": "review", "text": t })))
        .collect();
    Ok(json!({
        "descriptor": wire(&r.descriptor),
        "out_dir": wire(&r.out_dir),
        "index": { "name": r.index.name, "size": r.index.size, "sha256": r.index.sha256 },
        "packages": r.packages.iter().map(|p| json!({ "name": p.name, "size": p.size, "sha256": p.sha256 })).collect::<Vec<_>>(),
        "files": r.files,
        "unpacked": r.unpacked,
        "packed": r.packed,
        "requires_game": r.requires_game,
        "baked": r.baked,
        "warnings": r.warnings,
        "signed": r.signed,
        "key_id": r.key_id,
        "left_out": r.left_out,
        // the Mod Browser's content rules: reported, the release is written either way
        "catalog_content": catalog,
    }))
}

// ---- mod.package ------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PackageParams {
    /// The output folder. Left out: `<project>\releases\<version>`; inside the project only a
    /// folder under `releases\` is taken.
    #[serde(default)]
    path: Option<String>,
    /// The lowest game version the release runs on (`1.4.2`). Left out: the linked game's
    /// version; empty: no requirement.
    #[serde(default)]
    requires_game: Option<String>,
    /// The author key's password, when its file has one. Never echoed or logged.
    #[serde(default)]
    password: Option<String>,
    /// Wait for the packaging to end (true) or answer its job at once (false).
    #[serde(default = "yes")]
    wait: bool,
}

const fn yes() -> bool {
    true
}

/// Opens the author key with a password a caller sent; the text is wiped when it goes.
pub async fn unlock_with(editor: &Editor, password: Option<String>) -> Result<(), CmdError> {
    let Some(password) = password.map(Zeroizing::new).filter(|p| !p.is_empty()) else { return Ok(()) };
    let keys = AuthorKeys::of(editor);
    editor.exec.io(move || keys.unlock(&password)).await.map_err(|e| refuse(format!("author key: {e}")))
}

async fn mod_package(ed: Arc<Editor>, p: PackageParams, _origin: Origin) -> Reply {
    ed.project()?;
    unlock_with(&ed, p.password).await?;
    // a command asks nothing: an earlier packaging of the same version is simply replaced
    let req = PackageRequest { out_dir: p.path.unwrap_or_default(), requires_game: p.requires_game };
    let id = Releases::start(&ed, req)?;
    if !p.wait {
        return Ok(json!({ "job": id.0 }));
    }
    ed.jobs.wait(id).await;
    let r = Releases::of(&ed).result(id).ok_or_else(|| CmdError::internal("the packaging left no result"))?;
    package_answer(&r)
}

// ---- mod.thumbnail ----------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ThumbnailParams {
    /// A picture (PNG, JPG, TGA, BMP, DDS; absolute or project-relative): its middle square
    /// becomes `thumbnail.dds` at the module root. A 256 x 256 DXT DDS of the project is named as
    /// it is.
    #[serde(default)]
    from: Option<String>,
    /// Takes the picture off the module; only a file the editor made is deleted.
    #[serde(default)]
    remove: bool,
}

async fn mod_thumbnail(ed: Arc<Editor>, p: ThumbnailParams, origin: Origin) -> Reply {
    let value = match (p.from, p.remove) {
        (Some(_), true) => return Err(CmdError::invalid("mod.thumbnail takes from or remove, not both")),
        (Some(f), false) if !f.trim().is_empty() => f,
        (_, true) => String::new(),
        _ => return Err(CmdError::invalid("mod.thumbnail needs from=<picture> or remove=true")),
    };
    let (project, service) = open_manifest(&ed).await?;
    let snap = service.snapshot();
    let Some(m) = snap.manifest() else { return Err(refuse("the project has no mod.ltx: it is no module")) };
    let (root, current) = (service.root().to_path_buf(), m.thumbnail.clone());
    let thumbnail =
        ed.exec.io(move || crate::browser::apply_thumbnail(&root, &value, &current)).await.map_err(refuse)?;
    service.edit(vec![gz_mod::Edit::optional("module", "thumbnail", &thumbnail)], origin).await?;
    let mut answer = manifest_answer(&ed, &project, &service).await?;
    if let Value::Object(map) = &mut answer {
        map.insert("thumbnail".into(), json!(wire(&thumbnail)));
    }
    Ok(answer)
}

// ---- release.key ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum KeyAction {
    /// Where the key is, whether it is open or locked, its id and public key.
    #[default]
    Status,
    /// A new key file (`path`, optional `password`). Never overwrites a file, never inside a
    /// project.
    Create,
    /// Uses the key file at `path`, and/or opens the key with its `password`.
    Open,
    /// Locks the opened key: its secret is wiped from memory.
    Lock,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KeyParams {
    #[serde(default)]
    action: KeyAction,
    /// The key file. Create: where it goes (left out: Documents\Grizzly Editor\Keys\author.xkey).
    #[serde(default)]
    path: Option<String>,
    /// The key's password. Never echoed or logged.
    #[serde(default)]
    password: Option<String>,
}

/// The author key as it stands.
pub async fn key_state(editor: &Arc<Editor>) -> KeyState {
    let keys = AuthorKeys::of(editor);
    editor.exec.io(move || keys.state()).await
}

async fn release_key(ed: Arc<Editor>, p: KeyParams, _origin: Origin) -> Reply {
    let keys = AuthorKeys::of(&ed);
    let project = ed.project_root();
    let password = p.password.map(Zeroizing::new);
    match p.action {
        KeyAction::Status => {}
        KeyAction::Create => {
            let path = p.path.unwrap_or_default();
            let password = password.unwrap_or_default();
            ed.exec
                .io(move || keys.create(Some(&path), &password, project.as_deref()))
                .await
                .map_err(refuse)?;
        }
        KeyAction::Open => {
            if p.path.is_none() && password.is_none() {
                return Err(CmdError::invalid("release.key action=open needs path, password or both"));
            }
            if let Some(path) = p.path.filter(|s| !s.trim().is_empty()) {
                let k = keys.clone();
                ed.exec.io(move || k.use_file(&path, project.as_deref())).await.map_err(refuse)?;
            }
            if let Some(password) = password {
                ed.exec.io(move || keys.unlock(&password)).await.map_err(refuse)?;
            }
        }
        KeyAction::Lock => keys.lock(),
    }
    Ok(json!({ "key": key_state(&ed).await }))
}
