//! The manifest's commands: `mod.set` (what a text edit of `mod.ltx` cannot do in one step -
//! the rules checked, the answer with the problems) and `mod.exclude` (the content browser's
//! Exclude from Build). `mod.info`, which also says what a build would ship, is `gz-build`'s.
//! An agent may equally edit `mod.ltx` itself and run `asset.check`.

use std::collections::BTreeMap;
use std::sync::Arc;

use gz_base::{Origin, Problem, RelPath};
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use gz_project::{EntryKind, FileTree, Project, ProjectHooks};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::doc::entry_line;
use crate::edit::Edit;
use crate::model::{Command, GlobalVar, Manifest, ProvidesMode};
use crate::rules::{
    normalize_github_repo, valid_global_name, validate, validate_github_repo, validate_id, validate_mode_id,
    validate_version, validate_website,
};
use crate::service::{ArgCx, ManifestArgs, ManifestFields, ManifestService, ManifestState, command_check};
use crate::text::{display_value, split_list, trim};
use crate::{commands, io};

/// The room the Mod Settings form gives the description.
const DESCRIPTION_ROOM: usize = 4096;

pub fn register(reg: &mut Registry) {
    reg.area("mod", "The module: its manifest, what a build ships, building and packaging it");
    reg.cmd(
        Spec::write(
            "mod.set",
            "Changes keys of mod.ltx by section, checked by the manifest's rules; answers the manifest and its problems.",
        )
        .example(r#"set={"module":{"version":"1.2.0"},"update":{"github":"owner/repo"}}"#),
        mod_set,
    );
    reg.cmd(
        Spec::write(
            "mod.exclude",
            "Leaves a file or folder of the project out of the build, or takes it back in ([exclude] of mod.ltx).",
        )
        .example("path=docs on=true"),
        mod_exclude,
    );
}

/// Hangs the area on the editor: a project that opens or is created without `mod.ltx` gets
/// its scaffold.
pub fn install(editor: &Editor) {
    ProjectHooks::of(editor).on_scaffold(|root, name| {
        io::scaffold(root, name).map(|_| ()).map_err(|e| format!("mod.ltx could not be written: {e}"))
    });
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// The open project's manifest service, the file read again: a command answers what the file
/// says now.
pub async fn open_manifest(ed: &Arc<Editor>) -> Result<(Arc<Project>, Arc<ManifestService>), CmdError> {
    let (session, project) = Project::of(ed)?;
    let service = ManifestService::of_session(ed, &session).await?;
    service.sync().await?;
    Ok((project, service))
}

fn has_dir(tree: &FileTree, name: &str) -> bool {
    RelPath::new(name)
        .ok()
        .and_then(|rel| tree.entry(&rel).map(|e| e.kind))
        .is_some_and(|k| matches!(k, EntryKind::Dir | EntryKind::Link))
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

/// The manifest as data: what `mod.info` and `mod.set` answer under `manifest`. `world` comes
/// from the area that owns it.
#[must_use]
pub fn manifest_json(editor: &Editor, state: &ManifestState, m: &Manifest, tree: &FileTree) -> Value {
    let world = ManifestFields::of(editor).get("world").map_or(Value::Null, |f| f(state));
    let modes: Vec<Value> =
        m.provides_modes.iter().map(|p| json!({ "id": p.id, "title": p.title })).collect();
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
        "pda_layout": m.pda_layout.replace('\\', "/"),
        "has_dirs": Value::Object(dirs),
    })
}

/// The manifest's own problems and its commands', placed in `mod.ltx`.
pub async fn manifest_problems(ed: &Arc<Editor>, m: &Manifest) -> (Vec<String>, Vec<Problem>) {
    let Ok(session) = ed.project() else { return (Vec::new(), Vec::new()) };
    let mut problems = validate(m);
    let (functions, commands) = command_check(ed, &session, m).await;
    problems.extend(commands);
    let file = RelPath::new(crate::model::FILE).ok();
    for p in &mut problems {
        if p.file.is_none() {
            p.file.clone_from(&file);
        }
    }
    (functions, problems)
}

/// `{manifest, problems, functions}` of the manifest as the store holds it now.
pub async fn manifest_answer(ed: &Arc<Editor>, project: &Project, service: &ManifestService) -> Reply {
    let snap = service.snapshot();
    let Some(m) = snap.manifest() else { return Err(refuse("the project has no mod.ltx: it is no module")) };
    project.tree.refresh().await;
    let tree = project.tree.snapshot();
    let manifest = manifest_json(ed, &snap, m, &tree);
    let (functions, problems) = manifest_problems(ed, m).await;
    Ok(json!({ "manifest": manifest, "problems": problems, "functions": functions }))
}

// ---- mod.set ----------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetParams {
    /// What to change, by section of mod.ltx. `module`: id, name, version, author, description,
    /// disable_game_updates, website, mode, target, tags. `update`: github. `order`: after,
    /// before. `requires`, `conflicts`, `exclude`: the whole list. `vars`: {name: default}.
    /// `provides_mode`: {id, title} (an empty id drops it). `pda`: {layout}. `commands`:
    /// {name: {function, help, args, cheat}}, null removes one. An empty text removes a key.
    set: Map<String, Value>,
}

/// A value as the text `mod.ltx` holds: a number or a flag as written, a list joined by `sep`,
/// `null` as empty.
fn text_of(v: &Value, sep: &str) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(items) => items.iter().map(|i| text_of(i, sep)).collect::<Vec<_>>().join(sep),
        Value::Object(_) => v.to_string(),
    }
}

/// A list: an array, or a text with one entry per line or comma separated.
fn list_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => {
            items.iter().map(|i| trim(&text_of(i, ",")).to_owned()).filter(|s| !s.is_empty()).collect()
        }
        other => {
            let t = text_of(other, ",");
            split_list(&t, if t.contains('\n') { '\n' } else { ',' })
        }
    }
}

fn flag_of(v: &Value) -> Result<bool, CmdError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Null => Ok(false),
        Value::Number(n) => Ok(n.as_f64().is_some_and(|x| x != 0.0)),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "" | "false" | "0" | "no" | "off" => Ok(false),
            other => Err(refuse(format!("'{other}' is no flag: true or false expected"))),
        },
        _ => Err(refuse("a flag is true or false")),
    }
}

fn section_keys<'a>(section: &str, v: &'a Value) -> Result<&'a Map<String, Value>, CmdError> {
    v.as_object()
        .ok_or_else(|| refuse(format!("set.{section} takes its keys as an object: {{\"key\": value}}")))
}

fn unknown_key(section: &str, key: &str, known: &[&str]) -> CmdError {
    refuse(format!(
        "mod.set does not set [{section}] {key} (it sets {}): edit mod.ltx for anything else",
        known.join(", ")
    ))
}

/// `name = default` rows from an object, a list of rows or a text.
fn parse_vars(v: &Value) -> Result<Vec<GlobalVar>, CmdError> {
    let rows: Vec<(String, String)> = match v {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), text_of(v, ","))).collect(),
        other => list_of(other)
            .into_iter()
            .map(|row| {
                let (name, value) = row.split_once('=').unwrap_or((row.as_str(), ""));
                (trim(name).to_owned(), trim(value).to_owned())
            })
            .collect(),
    };
    let mut vars: Vec<GlobalVar> = Vec::new();
    for (name, value) in rows {
        let name = trim(&name).to_owned();
        if name.is_empty() {
            continue;
        }
        if !valid_global_name(&name) {
            return Err(refuse(format!("a global variable is named by letters, digits and _: '{name}'")));
        }
        if vars.iter().any(|g| g.name == name) {
            return Err(refuse(format!("the global variable '{name}' is declared twice")));
        }
        vars.push(GlobalVar { name, value: trim(&value).to_owned() });
    }
    Ok(vars)
}

const MODULE_KEYS: [&str; 10] = [
    "id",
    "name",
    "version",
    "author",
    "description",
    "disable_game_updates",
    "website",
    "mode",
    "target",
    "tags",
];

/// The edits of the `module` section; `tags` goes through the Mod Browser's hook.
async fn module_edits(
    ed: &Arc<Editor>,
    service: &ManifestService,
    cur: &Manifest,
    keys: &Map<String, Value>,
    edits: &mut Vec<Edit>,
    id: &mut String,
    mode: &mut String,
) -> Result<bool, CmdError> {
    let mut target_given = false;
    for (key, v) in keys {
        let text = text_of(v, ", ");
        match key.as_str() {
            "id" => {
                if !validate_id(&text) {
                    return Err(refuse("the module id takes a-z, 0-9, _ . and - only"));
                }
                edits.push(Edit::set("module", "id", text.as_str()));
                id.clone_from(&text);
            }
            "name" => edits.push(Edit::set("module", "name", display_value(&text, false))),
            "version" => {
                if !validate_version(&text) {
                    return Err(refuse("the version is 1 to 4 numbers separated by dots, e.g. 1.2.0"));
                }
                edits.push(Edit::set("module", "version", text));
            }
            "author" => edits.push(if text.is_empty() {
                Edit::remove("module", "author")
            } else {
                Edit::set("module", "author", display_value(&text, false))
            }),
            "description" => {
                let d = trim(&text);
                if d.len() >= DESCRIPTION_ROOM {
                    return Err(refuse("the description is too long: 4095 bytes at most"));
                }
                // the one text that routinely holds ';' and line breaks: always quoted
                edits.push(if d.is_empty() {
                    Edit::remove("module", "description")
                } else {
                    Edit::set("module", "description", display_value(d, true))
                });
            }
            "disable_game_updates" => edits.push(if flag_of(v)? {
                Edit::set("module", "disable_game_updates", "true")
            } else {
                Edit::remove("module", "disable_game_updates")
            }),
            "website" => {
                let w = trim(&text);
                if !validate_website(w) {
                    return Err(refuse(
                        "the website is a https link to ap-pro.ru or moddb.com, nothing else",
                    ));
                }
                edits.push(Edit::optional("module", "website", w));
            }
            "mode" => {
                if !validate_mode_id(&text) {
                    return Err(refuse("a mode id takes a-z, 0-9, _ . and - only, or is empty"));
                }
                edits.push(Edit::optional("module", "mode", &text));
                mode.clone_from(&text);
            }
            "target" => {
                if !["default", "existing", "new", "any"].iter().any(|t| t.eq_ignore_ascii_case(&text)) {
                    return Err(refuse("target is default, existing, new or any"));
                }
                edits.push(Edit::set("module", "target", text));
                target_given = true;
            }
            "tags" => {
                let Some(hook) = ManifestArgs::of(ed).get("tags") else {
                    return Err(refuse("tags cannot be set: this editor has no Mod Browser support"));
                };
                let cx = ArgCx {
                    editor: ed.clone(),
                    root: service.root().to_path_buf(),
                    value: list_of(v).join(", "),
                    manifest: cur.clone(),
                };
                edits.extend(hook(cx).await.map_err(refuse)?);
            }
            "thumbnail" => return Err(refuse("the picture is set with mod.thumbnail from=<picture>")),
            other => return Err(unknown_key("module", other, &MODULE_KEYS)),
        }
    }
    Ok(target_given)
}

fn command_edits(cur: &Manifest, v: &Value, edits: &mut Vec<Edit>) -> Result<(), CmdError> {
    let map = section_keys("commands", v)?;
    for (name, spec) in map {
        let name = trim(name).to_owned();
        let same = |c: &&Command| c.name.eq_ignore_ascii_case(&name);
        if spec.is_null() {
            if !cur.commands.iter().any(|c| same(&c)) {
                return Err(refuse(format!("there is no command '{name}'")));
            }
            edits.push(Edit::RemoveCommand(name));
            continue;
        }
        if !commands::valid_command_name(&name) {
            return Err(refuse(format!("a command name is a-z, 0-9 and _ only, 64 at most: '{name}'")));
        }
        let fields = spec.as_object().ok_or_else(|| {
            refuse(format!("set.commands.{name} takes {{function, help, args, cheat}}, or null to remove it"))
        })?;
        let mut c = cur.commands.iter().find(same).cloned().unwrap_or_default();
        c.name.clone_from(&name);
        for (key, value) in fields {
            match key.as_str() {
                "function" => c.function = trim(&text_of(value, ",")).to_owned(),
                "help" => c.help = trim(&text_of(value, ",")).to_owned(),
                "args" => c.args = trim(&text_of(value, ",")).to_owned(),
                "cheat" => c.cheat = flag_of(value)?,
                other => {
                    return Err(unknown_key(
                        &format!("command.{name}"),
                        other,
                        &["function", "help", "args", "cheat"],
                    ));
                }
            }
        }
        if !commands::valid_command_function(&c.function) {
            return Err(refuse(format!(
                "a command calls <script>.<function> of the project's gamedata\\scripts, e.g. my_mod.status - got '{}'",
                c.function
            )));
        }
        c.declared = true;
        edits.push(Edit::SetCommand(c));
    }
    Ok(())
}

fn provides_mode_edit(cur: &Manifest, v: &Value) -> Result<Edit, CmdError> {
    // only the first declared mode is edited; an empty id or null drops it
    let mut modes = cur.provides_modes.clone();
    let (id, title) = match v {
        Value::Null => (String::new(), None),
        Value::Object(map) => {
            for k in map.keys() {
                if k != "id" && k != "title" {
                    return Err(unknown_key("provides_mode", k, &["id", "title"]));
                }
            }
            let id = map.get("id").map(|v| text_of(v, ","));
            let title = map.get("title").map(|v| text_of(v, ","));
            (id.unwrap_or_else(|| modes.first().map(|m| m.id.clone()).unwrap_or_default()), title)
        }
        other => (text_of(other, ","), None),
    };
    if !validate_mode_id(&id) {
        return Err(refuse("a mode id takes a-z, 0-9, _ . and - only, or is empty"));
    }
    if id.is_empty() {
        if !modes.is_empty() {
            modes.remove(0);
        }
    } else {
        if modes.is_empty() {
            modes.push(ProvidesMode::default());
        }
        if let Some(first) = modes.first_mut() {
            first.id = id;
            if let Some(t) = title {
                first.title = t;
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for m in &modes {
        lines.push(entry_line("id", &m.id, 11));
        if !m.title.is_empty() {
            lines.push(entry_line("title", &m.title, 11));
        }
    }
    lines.extend(cur.provides_extra.iter().cloned());
    Ok(Edit::lines("provides_mode", lines))
}

async fn mod_set(ed: Arc<Editor>, p: SetParams, origin: Origin) -> Reply {
    let (project, service) = open_manifest(&ed).await?;
    // a project without a manifest gets its scaffold, then the edit
    service.ensure(project.name()).await?;
    let snap = service.snapshot();
    let Some(cur) = snap.manifest() else {
        return Err(refuse("the project has no mod.ltx: it is no module"));
    };
    let cur = cur.clone();
    let mut edits: Vec<Edit> = Vec::new();
    let (mut id, mut mode) = (cur.id.clone(), cur.mode.clone());
    let mut target_given = false;
    let mut modes_touched = false;

    // the order of the sections is fixed, whatever order the request named them in
    let sections: BTreeMap<&str, &Value> = p.set.iter().map(|(k, v)| (k.as_str(), v)).collect();
    for (section, v) in &sections {
        match *section {
            "module" => {
                let keys = section_keys("module", v)?;
                target_given |=
                    module_edits(&ed, &service, &cur, keys, &mut edits, &mut id, &mut mode).await?;
            }
            "update" => {
                for (key, value) in section_keys("update", v)? {
                    if key != "github" {
                        return Err(unknown_key("update", key, &["github"]));
                    }
                    let repo = normalize_github_repo(&text_of(value, ","));
                    if !validate_github_repo(&repo) {
                        return Err(refuse("github is owner/repo, e.g. Madmer/agroprom-story"));
                    }
                    edits.push(Edit::optional("update", "github", &repo));
                }
            }
            "order" => {
                for (key, value) in section_keys("order", v)? {
                    if key != "after" && key != "before" {
                        return Err(unknown_key("order", key, &["after", "before"]));
                    }
                    edits.push(Edit::optional("order", key, &list_of(value).join(", ")));
                }
            }
            "requires" | "conflicts" | "exclude" => edits.push(Edit::lines(section, list_of(v))),
            "vars" => {
                let vars = parse_vars(v)?;
                edits.push(Edit::lines(
                    "vars",
                    vars.iter().map(|g| entry_line(&g.name, &g.value, 15)).collect(),
                ));
            }
            "provides_mode" => {
                edits.push(provides_mode_edit(&cur, v)?);
                modes_touched = true;
            }
            "pda" => {
                for (key, value) in section_keys("pda", v)? {
                    if key != "layout" {
                        return Err(unknown_key("pda", key, &["layout"]));
                    }
                    let Some(hook) = ManifestArgs::of(&ed).get("pda.layout") else {
                        return Err(refuse("the PDA layout cannot be set: this editor has no PDA support"));
                    };
                    let cx = ArgCx {
                        editor: ed.clone(),
                        root: service.root().to_path_buf(),
                        value: text_of(value, ","),
                        manifest: cur.clone(),
                    };
                    edits.extend(hook(cx).await.map_err(refuse)?);
                }
            }
            "commands" => command_edits(&cur, v, &mut edits)?,
            other => {
                return Err(refuse(format!(
                    "mod.set does not set [{other}]: module, update, order, requires, conflicts, exclude, vars, provides_mode, pda and commands are its sections (world.set, screens.set and the files themselves change the rest)"
                )));
            }
        }
    }

    // The targeting choice. "default" cannot be inferred - a module with no mode either targets
    // the ordinary game on purpose or was never decided - so it is stated explicitly; picking a
    // mode states it implicitly. A classic module (`mode = *`) keeps `target = any`.
    let decides_target = modes_touched || sections.get("module").and_then(|m| m.get("mode")).is_some();
    if !target_given && decides_target {
        let mut after = snap.doc().cloned().unwrap_or_default();
        for e in &edits {
            crate::edit::apply(&mut after, e.clone())?;
        }
        let after = Manifest::parse(&after);
        if !after.provides_modes.is_empty() {
            edits.push(Edit::set("module", "target", "new"));
        } else if after.mode != "*" && !after.mode.is_empty() {
            edits.push(Edit::set("module", "target", "existing"));
        }
    }

    if !validate_id(&id) {
        return Err(refuse("the module id is missing or invalid: a-z, 0-9, _ . and - only"));
    }
    let changed = service.edit(edits, origin).await?;
    let mut answer = manifest_answer(&ed, &project, &service).await?;
    if let Value::Object(map) = &mut answer {
        map.insert("changed".into(), json!(changed));
    }
    Ok(answer)
}

// ---- mod.exclude ------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExcludeParams {
    /// A file or folder of the project, or a pattern (`*.psd`, `docs/*.md`).
    path: String,
    /// True leaves it out of the build, false takes it back in.
    #[serde(default = "yes")]
    on: bool,
}

const fn yes() -> bool {
    true
}

async fn mod_exclude(ed: Arc<Editor>, p: ExcludeParams, origin: Origin) -> Reply {
    let (_project, service) = open_manifest(&ed).await?;
    if !service.snapshot().exists() {
        return Err(refuse("the project has no mod.ltx: it is no module"));
    }
    let path = trim(&p.path).replace("\\\\", "\\");
    if path.is_empty() {
        return Err(CmdError::invalid(
            "mod.exclude needs path: a file or folder of the project, or a pattern",
        ));
    }
    let changed = service.edit(vec![Edit::SetExcluded { rel: path, excluded: p.on }], origin).await?;
    let snap = service.snapshot();
    let exclude = snap.manifest().map(|m| m.exclude.clone()).unwrap_or_default();
    Ok(json!({ "exclude": exclude, "changed": changed }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn vars_rows() {
        let vars = parse_vars(&json!("kills = 0\nmet = \"1\"\n\n")).unwrap();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[1].value, "\"1\"");
        assert_eq!(parse_vars(&json!("a=1, b=2")).unwrap().len(), 2);
        assert_eq!(parse_vars(&json!({"kills": 0, "met": "yes"})).unwrap()[0].value, "0");
        assert_eq!(
            parse_vars(&json!("a-b = 1")).unwrap_err().message,
            "a global variable is named by letters, digits and _: 'a-b'"
        );
        assert_eq!(
            parse_vars(&json!(["a=1", "a=2"])).unwrap_err().message,
            "the global variable 'a' is declared twice"
        );
    }

    #[test]
    fn values_as_text_and_lists() {
        assert_eq!(text_of(&json!(2), ","), "2");
        assert_eq!(text_of(&json!(null), ","), "");
        assert_eq!(list_of(&json!(["a", " b ", ""])), ["a", "b"]);
        assert_eq!(list_of(&json!("a, b")), ["a", "b"]);
        assert_eq!(list_of(&json!("docs\n*.psd")), ["docs", "*.psd"]);
        assert!(flag_of(&json!("yes")).unwrap());
        assert!(flag_of(&json!("maybe")).is_err());
    }

    #[test]
    fn provides_mode_first_entry() {
        let mut cur = Manifest::default();
        let e = provides_mode_edit(&cur, &json!({"id": "night", "title": "Night"})).unwrap();
        assert_eq!(
            e,
            Edit::lines(
                "provides_mode",
                vec![entry_line("id", "night", 11), entry_line("title", "Night", 11)]
            )
        );
        cur.provides_modes.push(ProvidesMode { id: "night".into(), title: "Night".into() });
        assert_eq!(provides_mode_edit(&cur, &Value::Null).unwrap(), Edit::lines("provides_mode", Vec::new()));
        assert!(provides_mode_edit(&cur, &json!({"name": "x"})).is_err());
    }
}
