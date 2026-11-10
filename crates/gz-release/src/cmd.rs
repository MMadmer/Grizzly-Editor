//! The commands. The wire is frozen by the C++ handlers reached from `LevelMcpMod.cpp`:
//! `mod_package` (`EditorMod::McpPackage` / `AnswerPackage`) answers once its job has ended,
//! while every other command goes on meanwhile; the author key actions of `mod_catalog`
//! (`EditorCatalogSubmit::McpCatalog`: `status`, `key_create`, `key_use`, `key_unlock`) with the
//! `key` object of its status.
//!
//! `mod_catalog` as a whole belongs to the Mod Browser area (`gz-catalog`), which registers it
//! after this crate - the later handler wins - and calls [`key_action`] and [`key_state`] for
//! the key. Until it does, the handler here answers the key actions and refuses the others.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::keys::{AuthorKeys, KeyState};
use crate::package::{PackageRequest, PackageResult, Releases};

const AREA: &str = "gz-release";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_package", mod_package);
    reg.add(AREA, "mod_catalog", mod_catalog);
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A string argument the way `GetArg` read it: present means given, even empty.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A secret argument taken out of the request, so that the only copy is wiped when it goes.
fn take_secret(args: &mut Args, key: &str) -> Option<Zeroizing<String>> {
    match args.remove(key)? {
        Value::String(s) => Some(Zeroizing::new(s)),
        Value::Number(n) => Some(Zeroizing::new(n.to_string())),
        Value::Bool(b) => Some(Zeroizing::new(b.to_string())),
        _ => None,
    }
}

fn wire(path: &str) -> String {
    path.replace('\\', "/")
}

/// The answer of a packaging that was held until it ended (`AnswerPackage`).
fn answer(r: Option<Arc<PackageResult>>) -> Reply {
    let Some(r) = r else { return Err(refuse("")) };
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

async fn mod_package(ed: Arc<Editor>, mut args: Args, _origin: Origin) -> Reply {
    if args.flag("answer_only") {
        return answer(Releases::of(&ed).last());
    }
    ed.project()?;
    let out_dir = arg(&args, "out_dir").unwrap_or_default();
    // absent = the linked game's version, present but empty = no requirement
    let requires_game = arg(&args, "requires_game");

    // the author key's password, when the key file has one; never echoed or logged
    if let Some(password) = take_secret(&mut args, "password").filter(|p| !p.is_empty()) {
        let keys = AuthorKeys::of(&ed);
        ed.exec.io(move || keys.unlock(&password)).await.map_err(|e| refuse(format!("author key: {e}")))?;
    }
    // MCP is the programmatic path: it asks nothing, an earlier packaging of the same version is
    // simply replaced
    let id = Releases::start(&ed, PackageRequest { out_dir, requires_game })?;
    ed.jobs.wait(id).await;
    answer(Releases::of(&ed).result(id))
}

/// The author key as it stands: the `key` object of `mod_catalog`.
pub async fn key_state(editor: &Arc<Editor>) -> KeyState {
    let keys = AuthorKeys::of(editor);
    editor.exec.io(move || keys.state()).await
}

/// One of the author key's actions of `mod_catalog`: `key_create` (`path`, `password`),
/// `key_use` (`path`), `key_unlock` (`password`). `None` for any other action. The password is
/// taken out of `args` and wiped.
pub async fn key_action(editor: &Arc<Editor>, action: &str, args: &mut Args) -> Option<Result<(), CmdError>> {
    let keys = AuthorKeys::of(editor);
    let project = editor.project_root();
    let done = match action {
        "key_create" => {
            let path = arg(args, "path").unwrap_or_default();
            let password = take_secret(args, "password").unwrap_or_default();
            editor.exec.io(move || keys.create(Some(&path), &password, project.as_deref())).await
        }
        "key_use" => {
            let Some(path) = args.text("path").map(str::to_owned) else { return Some(Err(refuse("key_use needs 'path'"))) };
            editor.exec.io(move || keys.use_file(&path, project.as_deref())).await
        }
        "key_unlock" => {
            let Some(password) = take_secret(args, "password") else { return Some(Err(refuse("key_unlock needs 'password'"))) };
            editor.exec.io(move || keys.unlock(&password)).await
        }
        _ => return None,
    };
    Some(done.map_err(refuse))
}

async fn mod_catalog(ed: Arc<Editor>, mut args: Args, _origin: Origin) -> Reply {
    ed.project()?;
    let action = args.text("action").unwrap_or("status").to_owned();
    if action != "status" {
        match key_action(&ed, &action, &mut args).await {
            Some(done) => done?,
            None => return Err(refuse("unknown action: status, key_create, key_use or key_unlock expected (the rest of Submit to Mod Browser is not in this editor yet)")),
        }
    }
    Ok(json!({ "key": key_state(&ed).await }))
}
