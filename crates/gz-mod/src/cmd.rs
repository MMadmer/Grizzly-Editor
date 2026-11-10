//! The manifest's commands. The wire is the C++ handlers' (`EditorModManifest.cpp`, reached
//! from `LevelMcpMod.cpp`): `mod_manifest`, `mod_set_manifest`, `mod_exclude`, `mod_commands`.

use std::sync::Arc;

use gz_base::{Origin, RelPath};
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_project::{EntryKind, FileTree, Project, ProjectHooks};
use serde_json::{Map, Value, json};

use crate::doc::entry_line;
use crate::edit::Edit;
use crate::model::{Command, GlobalVar, Manifest};
use crate::rules::{normalize_github_repo, valid_global_name, validate_github_repo, validate_id, validate_mode_id, validate_version, validate_website};
use crate::service::{ArgCx, ManifestArgs, ManifestFields, ManifestService, ManifestState, command_check};
use crate::text::{display_value, split_list, trim};
use crate::{commands, io};

const AREA: &str = "gz-mod";

/// The room the Mod Settings form gives the description.
const DESCRIPTION_ROOM: usize = 4096;

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_manifest", mod_manifest);
    reg.add(AREA, "mod_set_manifest", mod_set_manifest);
    reg.add(AREA, "mod_exclude", mod_exclude);
    reg.add(AREA, "mod_commands", mod_commands);
}

/// Hangs the area on the editor: a project that opens or is created without `mod.ltx` gets
/// its scaffold.
pub fn install(editor: &Editor) {
    ProjectHooks::of(editor).on_scaffold(|root, name| io::scaffold(root, name).map(|_| ()).map_err(|e| format!("mod.ltx could not be written: {e}")));
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A string argument the way `GetArg` read it: present means given, even empty; a bare number
/// or flag is its text.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

async fn open(ed: &Arc<Editor>) -> Result<(Arc<Project>, Arc<ManifestService>), CmdError> {
    let (session, project) = Project::of(ed)?;
    let service = ManifestService::of_session(ed, &session).await?;
    Ok((project, service))
}

fn has_dir(tree: &FileTree, name: &str) -> bool {
    RelPath::new(name).ok().and_then(|rel| tree.entry(&rel).map(|e| e.kind)).is_some_and(|k| matches!(k, EntryKind::Dir | EntryKind::Link))
}

fn commands_json(commands: &[Command]) -> Value {
    Value::Array(
        commands
            .iter()
            .map(|c| {
                let mut o = Map::new();
                o.insert("name".into(), json!(c.name));
                o.insert("function".into(), json!(c.function));
                o.insert("help".into(), json!(c.help));
                o.insert("args".into(), json!(c.args));
                o.insert("cheat".into(), json!(c.cheat));
                if !c.declared {
                    o.insert("declared".into(), json!(false));
                }
                Value::Object(o)
            })
            .collect(),
    )
}

/// The `mod_manifest` answer, fields in the C++ order. `world` comes from the area that owns it.
#[must_use]
pub fn manifest_json(editor: &Editor, state: &ManifestState, m: &Manifest, tree: &FileTree) -> Value {
    let world = ManifestFields::of(editor).get("world").map_or(Value::Null, |f| f(state));
    let modes: Vec<Value> = m.provides_modes.iter().map(|p| json!({ "id": p.id, "title": p.title })).collect();
    let vars: Vec<Value> = m.vars.iter().map(|v| json!({ "name": v.name, "value": v.value })).collect();
    let mut dirs = Map::new();
    for d in ["gamedata", "patch", "spawn", "scripts", "levels"] {
        dirs.insert(d.into(), json!(has_dir(tree, d)));
    }
    json!({
        "id": m.id,
        "name": m.name,
        "version": m.version,
        "author": m.author,
        "description": m.description,
        "disable_game_updates": m.disable_game_updates,
        "website": m.website,
        "github": m.update_github,
        "api": m.api,
        "target": m.target,
        "mode": m.mode,
        "provides_mode": modes,
        "requires": m.requires,
        "after": m.after,
        "before": m.before,
        "conflicts": m.conflicts,
        "exclude": m.exclude,
        "thumbnail": m.thumbnail.replace('\\', "/"),
        "tags": m.tags,
        "world": world,
        "vars": vars,
        "commands": commands_json(&m.commands),
        "pda_layout": m.pda_layout,
        "has_dirs": Value::Object(dirs),
    })
}

async fn manifest_reply(ed: &Arc<Editor>, project: &Project, service: &ManifestService) -> Reply {
    let snap = service.snapshot();
    let Some(m) = snap.manifest() else { return Err(refuse("mod.ltx not found in the project")) };
    project.tree.refresh().await;
    let tree = project.tree.snapshot();
    Ok(manifest_json(ed, &snap, m, &tree))
}

async fn mod_manifest(ed: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    let (project, service) = open(&ed).await?;
    service.sync().await?;
    manifest_reply(&ed, &project, &service).await
}

/// `name = default` rows, one per line or comma separated on one line.
fn parse_vars(text: &str) -> Result<Vec<GlobalVar>, CmdError> {
    let sep = if text.contains('\n') { '\n' } else { ',' };
    let mut vars: Vec<GlobalVar> = Vec::new();
    for row in split_list(text, sep) {
        let (name, value) = row.split_once('=').unwrap_or((row.as_str(), ""));
        let (name, value) = (trim(name), trim(value));
        if name.is_empty() {
            continue;
        }
        if !valid_global_name(name) {
            return Err(refuse(format!("a global variable is named by letters, digits and _: '{name}'")));
        }
        if vars.iter().any(|v| v.name == name) {
            return Err(refuse(format!("the global variable '{name}' is declared twice")));
        }
        vars.push(GlobalVar { name: name.to_owned(), value: value.to_owned() });
    }
    Ok(vars)
}

async fn mod_set_manifest(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let (project, service) = open(&ed).await?;
    // a pre-XMS project: scaffold the manifest, then edit it
    service.ensure(project.name()).await?;
    let snap = service.snapshot();
    let Some(cur) = snap.manifest() else { return Err(refuse("mod.ltx not found in the project")) };
    let cur = cur.clone();
    let mut edits: Vec<Edit> = Vec::new();

    let mut id = cur.id.clone();
    if let Some(v) = arg(&args, "id") {
        if !validate_id(&v) {
            return Err(refuse("invalid id (allowed: a-z 0-9 _ . -)"));
        }
        edits.push(Edit::set("module", "id", v.as_str()));
        id = v;
    }
    if let Some(v) = arg(&args, "name") {
        edits.push(Edit::set("module", "name", display_value(&v, false)));
    }
    if let Some(v) = arg(&args, "version") {
        if !validate_version(&v) {
            return Err(refuse("invalid version (1-4 numbers separated by dots, e.g. 1.2.0)"));
        }
        edits.push(Edit::set("module", "version", v));
    }
    if let Some(v) = arg(&args, "author") {
        edits.push(if v.is_empty() { Edit::remove("module", "author") } else { Edit::set("module", "author", display_value(&v, false)) });
    }
    if let Some(v) = arg(&args, "description") {
        let d = trim(&v);
        if d.len() >= DESCRIPTION_ROOM {
            return Err(refuse("description is too long (4095 bytes at most)"));
        }
        // the one text that routinely holds ';' and line breaks: always quoted
        edits.push(if d.is_empty() { Edit::remove("module", "description") } else { Edit::set("module", "description", display_value(d, true)) });
    }
    if let Some(off) = args.bool("disable_game_updates") {
        edits.push(if off { Edit::set("module", "disable_game_updates", "true") } else { Edit::remove("module", "disable_game_updates") });
    }
    if let Some(v) = arg(&args, "website") {
        let w = trim(&v);
        if !validate_website(w) {
            return Err(refuse("invalid website (only https://ap-pro.ru or https://moddb.com links are allowed)"));
        }
        edits.push(Edit::optional("module", "website", w));
    }
    if let Some(v) = arg(&args, "github") {
        let repo = normalize_github_repo(&v);
        if !validate_github_repo(&repo) {
            return Err(refuse("invalid github (owner/repo, e.g. Madmer/agroprom-story)"));
        }
        edits.push(Edit::optional("update", "github", &repo));
    }
    let mut mode = cur.mode.clone();
    if let Some(v) = arg(&args, "mode") {
        if !validate_mode_id(&v) {
            return Err(refuse("invalid mode id (allowed: a-z 0-9 _ . - or empty)"));
        }
        edits.push(Edit::optional("module", "mode", &v));
        mode = v;
    }
    // provides_mode edits touch only the first declared mode; an empty id drops it
    let mut modes = cur.provides_modes.clone();
    let mut modes_changed = false;
    if let Some(v) = arg(&args, "provides_mode_id") {
        if !validate_mode_id(&v) {
            return Err(refuse("invalid provides_mode_id (allowed: a-z 0-9 _ . - or empty)"));
        }
        if !v.is_empty() {
            if modes.is_empty() {
                modes.push(crate::model::ProvidesMode::default());
            }
            if let Some(first) = modes.first_mut() {
                first.id = v;
            }
        } else if !modes.is_empty() {
            modes.remove(0);
        }
        modes_changed = true;
    }
    if let Some(v) = arg(&args, "provides_mode_title")
        && let Some(first) = modes.first_mut()
    {
        first.title = v;
        modes_changed = true;
    }
    if modes_changed {
        let mut lines: Vec<String> = Vec::new();
        for m in &modes {
            lines.push(entry_line("id", &m.id, 11));
            if !m.title.is_empty() {
                lines.push(entry_line("title", &m.title, 11));
            }
        }
        lines.extend(cur.provides_extra.iter().cloned());
        edits.push(Edit::lines("provides_mode", lines));
    }
    // The targeting choice itself. "default" cannot be inferred - a module with no mode set
    // either targets the ordinary game on purpose or was never decided, and only the caller
    // knows which - so it is stated explicitly; picking a mode states it implicitly.
    if let Some(v) = arg(&args, "target") {
        if !["default", "existing", "new"].iter().any(|t| t.eq_ignore_ascii_case(&v)) {
            return Err(refuse("invalid target (allowed: default, existing, new)"));
        }
        edits.push(Edit::set("module", "target", v));
    } else if !modes.is_empty() {
        edits.push(Edit::set("module", "target", "new"));
    } else if !mode.is_empty() {
        edits.push(Edit::set("module", "target", "existing"));
    }
    if let Some(v) = arg(&args, "requires") {
        edits.push(Edit::lines("requires", split_list(&v, ',')));
    }
    for key in ["after", "before"] {
        if let Some(v) = arg(&args, key) {
            edits.push(Edit::optional("order", key, &split_list(&v, ',').join(", ")));
        }
    }
    if let Some(v) = arg(&args, "conflicts") {
        edits.push(Edit::lines("conflicts", split_list(&v, ',')));
    }
    // the whole list at once: one entry per line, or comma separated on one line
    if let Some(v) = arg(&args, "exclude") {
        edits.push(Edit::lines("exclude", split_list(&v, if v.contains('\n') { '\n' } else { ',' })));
    }
    if let Some(v) = arg(&args, "vars") {
        let vars = parse_vars(&v)?;
        edits.push(Edit::lines("vars", vars.iter().map(|g| entry_line(&g.name, &g.value, 15)).collect()));
    }
    // the Mod Browser's tags and picture belong to the catalog area
    for key in ["tags", "thumbnail"] {
        let Some(value) = arg(&args, key) else { continue };
        let Some(hook) = ManifestArgs::of(&ed).get(key) else {
            return Err(refuse(format!("'{key}' cannot be set: this editor has no Mod Browser support")));
        };
        let cx = ArgCx { editor: ed.clone(), root: service.root().to_path_buf(), value, manifest: cur.clone() };
        edits.extend(hook(cx).await.map_err(refuse)?);
    }

    if !validate_id(&id) {
        return Err(refuse("module id missing or invalid (allowed: a-z 0-9 _ . -)"));
    }
    service.edit(edits, origin).await?;
    manifest_reply(&ed, &project, &service).await
}

async fn mod_exclude(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let (_project, service) = open(&ed).await?;
    let Some(path) = args.text("path") else { return Err(refuse("missing 'path' (project-relative, e.g. docs or design/story.md)")) };
    // a value that still carries its escaped separators
    let path = path.replace("\\\\", "\\");
    // true unless told otherwise - the menu item it stands for excludes
    let excluded = args.bool("excluded").unwrap_or(true);
    service.sync().await?;
    if !service.snapshot().exists() {
        return Err(refuse("mod.ltx not found in the project"));
    }
    service.edit(vec![Edit::SetExcluded { rel: path, excluded }], origin).await?;
    let snap = service.snapshot();
    let exclude = snap.manifest().map(|m| m.exclude.clone()).unwrap_or_default();
    Ok(json!({ "exclude": exclude }))
}

async fn mod_commands(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let (session, project) = Project::of(&ed)?;
    let service = ManifestService::of_session(&ed, &session).await?;
    service.ensure(project.name()).await?;
    let name = arg(&args, "name").map(|n| trim(&n).to_owned()).unwrap_or_default();
    if !name.is_empty() {
        let snap = service.snapshot();
        let Some(m) = snap.manifest() else { return Err(refuse("mod.ltx not found in the project")) };
        let same = |c: &&Command| c.name.eq_ignore_ascii_case(&name);
        let edit = if args.flag("remove") {
            // every entry of that name: a second line and a section without one go with it
            if !m.commands.iter().any(|c| same(&c)) {
                return Err(refuse(format!("there is no command '{name}'")));
            }
            Edit::RemoveCommand(name.clone())
        } else {
            if !commands::valid_command_name(&name) {
                return Err(refuse(format!("a command name is a-z, 0-9 and _ only, 64 at most: '{name}'")));
            }
            let mut c = m.commands.iter().find(same).cloned().unwrap_or_default();
            c.name.clone_from(&name);
            if let Some(v) = arg(&args, "function") {
                c.function = trim(&v).to_owned();
            }
            if let Some(v) = arg(&args, "help") {
                c.help = trim(&v).to_owned();
            }
            if let Some(v) = arg(&args, "args") {
                c.args = trim(&v).to_owned();
            }
            if args.has("cheat") {
                c.cheat = args.flag("cheat");
            }
            if !commands::valid_command_function(&c.function) {
                return Err(refuse(format!(
                    "function: <script>.<function> of the project's gamedata\\scripts, e.g. my_mod.status - got '{}'",
                    c.function
                )));
            }
            c.declared = true;
            Edit::SetCommand(c)
        };
        service.edit(vec![edit], origin).await?;
    }
    let snap = service.snapshot();
    let Some(m) = snap.manifest() else { return Err(refuse("mod.ltx not found in the project")) };
    let (functions, problems) = command_check(&ed, &session, m).await;
    // "E404 commands: '...'" -> {code, text}
    let problems: Vec<Value> = problems.iter().map(|p| json!({ "code": p.code, "text": p.message })).collect();
    Ok(json!({ "commands": commands_json(&m.commands), "functions": functions, "problems": problems }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn vars_rows() {
        let vars = parse_vars("kills = 0\nmet = \"1\"\n\n").unwrap();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[1].value, "\"1\"");
        assert_eq!(parse_vars("a=1, b=2").unwrap().len(), 2);
        assert_eq!(parse_vars("a-b = 1").unwrap_err().message, "a global variable is named by letters, digits and _: 'a-b'");
        assert_eq!(parse_vars("a=1,a=2").unwrap_err().message, "the global variable 'a' is declared twice");
    }

    #[test]
    fn string_arguments() {
        let args: Args = serde_json::from_str(r#"{"name":"","version":2,"flag":true,"none":null}"#).unwrap();
        assert_eq!(arg(&args, "name").as_deref(), Some(""));
        assert_eq!(arg(&args, "version").as_deref(), Some("2"));
        assert_eq!(arg(&args, "none"), None);
        assert_eq!(arg(&args, "missing"), None);
    }
}
