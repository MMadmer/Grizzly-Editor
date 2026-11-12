//! The characters' commands (`EditorCharacters::McpList` / `McpGet` / `McpWrite` / `McpDelete`)
//! and what the area hangs on the editor.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Origin, Problem};
use gz_build::{BuildCheck, BuildChecks, BuildWalk};
use gz_editor::{Args, ArgsExt, CmdError, Editor, ProjectSession, Registry, Reply};
use gz_project::{AssetTemplates, FileOps, ProjectHooks, Report, TemplateKind};
use serde_json::{Value, json};

use crate::model::{Character, base_name, load_text, parse, read_file, template, text_in};
use crate::service::{Characters, Checker, Roster};

const AREA: &str = "gz-characters";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "characters_list", characters_list);
    reg.add(AREA, "characters_get", characters_get);
    reg.add(AREA, "characters_write", characters_write);
    reg.add(AREA, "characters_delete", characters_delete);
}

/// What the area hangs on the editor: the `content_create` template of a character, the
/// build's check, and the characters made when a project opens, so that `gz_nq` knows them
/// from the first validation on.
pub fn install(editor: &Editor) {
    AssetTemplates::of(editor).provide(TemplateKind::Character, |cx| Ok(template(cx.stem).into_bytes()));
    BuildChecks::of(editor).add(Arc::new(CharactersCheck));
    ProjectHooks::of(editor).on_open(|ed, _session| {
        let ed = ed.clone();
        ed.exec.clone().spawn(async move {
            // a project without characters costs a look at its file tree
            let _ = Characters::of(&ed).await;
        });
    });
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

// ---- paths the way NqUtil spelled them ------------------------------------------------------

/// `/` as `\`, doubled separators and `\.\` dropped, a trailing separator cut
/// (`NqUtil::NormalizePath`).
fn normalize(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.ends_with('\\') {
            continue;
        }
        out.push(c);
    }
    while let Some(at) = out.find("\\.\\") {
        out.replace_range(at..at + 2, "");
    }
    while out.len() > 3 && out.ends_with('\\') {
        out.pop();
    }
    out
}

fn is_absolute(n: &str) -> bool {
    let b = n.as_bytes();
    (b.len() >= 2 && b.get(1) == Some(&b':')) || n.starts_with("\\\\")
}

/// A project-relative or absolute path as an absolute one (`NqUtil::ProjectPath`).
fn project_path(root: &str, path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("path is empty".into());
    }
    let n = normalize(path);
    if is_absolute(&n) {
        return Ok(n);
    }
    // a project-relative path never escapes the project
    if n.contains("..") {
        return Err("path may not contain '..'".into());
    }
    Ok(format!("{}\\{n}", normalize(root)))
}

/// Inside the project folder, not the folder itself.
fn inside(root: &str, abs: &str) -> bool {
    let root = normalize(root).to_lowercase();
    let n = normalize(abs).to_lowercase();
    n.len() > root.len() + 1 && n.starts_with(&root) && n.as_bytes().get(root.len()) == Some(&b'\\')
}

/// The wire form of a path: project-relative when inside, `/` separated (`NqUtil::ProjectRelative`).
fn project_relative(root: &str, abs: &str) -> String {
    let n = normalize(abs);
    let r = normalize(root);
    let rel = if inside(&r, &n) { n.get(r.len() + 1..).unwrap_or(&n).to_owned() } else { n };
    rel.replace('\\', "/")
}

/// `path`, or `id` looked up among the project's characters (`ResolveTarget`).
fn resolve_target(args: &Args, root: &str, roster: &Roster) -> Result<String, String> {
    if let Some(path) = args.text("path") {
        return project_path(root, path);
    }
    let Some(id) = args.text("id") else {
        return Err("give 'path' (a .npcasset in the project) or 'id' (a character id)".into());
    };
    match roster.get_raw(id) {
        Some(c) => project_path(root, &c.file),
        None => Err(format!("no character with id '{id}' in the project")),
    }
}

// ---- the answer -----------------------------------------------------------------------------

/// One character as the answers describe it (`JsonCharacter`).
#[must_use]
pub fn character_json(c: &Character) -> Value {
    json!({
        "id": c.id,
        "file": c.file.replace('\\', "/"),
        "name": text_in(&c.name, "rus"),
        "community": c.community,
        "visual": c.visual.replace('\\', "/"),
        "base": c.base,
        "abstract": c.is_abstract,
        "supplies": c.supplies.len(),
        "trader": c.trade.is_trader(),
        "squad": c.squad,
    })
}

/// Validates one character on the IO pool.
async fn validated(
    ed: &Editor,
    checker: Checker,
    c: Character,
    mut problems: Vec<String>,
) -> (Character, Vec<String>) {
    ed.exec
        .io(move || {
            checker.validate(&c, &mut problems);
            (c, problems)
        })
        .await
}

/// The folders a write or a delete changed, for the file tree to list again.
fn mark_folders(svc: &Characters, abs: &str) {
    let tree = &svc.project().tree;
    let path = PathBuf::from(abs);
    let mut dir = path.parent();
    for _ in 0..2 {
        let Some(d) = dir else { break };
        tree.mark_fs(d);
        dir = d.parent();
    }
}

// ---- the commands ---------------------------------------------------------------------------

async fn open(ed: &Editor) -> Result<Arc<Characters>, CmdError> {
    ed.project()?;
    let svc = Characters::of(ed).await?;
    // a mod.ltx changed behind the editor moves [exclude], and with it the characters
    let _ = svc.manifest().sync().await;
    Ok(svc)
}

async fn characters_list(ed: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let roster = svc.fresh_roster().await;
    let checker = svc.checker(&ed).await;
    let mut problems = roster.problems.clone();
    let all = roster.clone();
    problems.extend(ed.exec.io(move || checker.validate_all(&all.resolved)).await);
    let module = svc.manifest().snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default();
    let characters: Vec<Value> = roster.resolved.iter().map(character_json).collect();
    Ok(json!({ "module": module, "characters": characters, "problems": problems }))
}

async fn characters_get(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let roster = svc.fresh_roster().await;
    let root = svc.root().to_owned();
    let abs = resolve_target(&args, &root, &roster).map_err(refuse)?;
    let wire = project_relative(&root, &abs);
    let (text, mut c) = ed
        .exec
        .io(move || {
            let text = read_file(Path::new(&abs))?;
            let c = load_text(&text, &wire)?;
            Ok::<_, String>((text, c))
        })
        .await
        .map_err(refuse)?;
    // the answer describes what the character REALLY is, base chain and all; `text` is still
    // the file as written
    if let Some(r) = roster.get(&c.id) {
        c = r.clone();
    }
    let checker = svc.checker(&ed).await;
    let (c, problems) = validated(&ed, checker, c, Vec::new()).await;
    Ok(json!({ "character": character_json(&c), "text": text, "problems": problems }))
}

async fn characters_write(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let Some(text) = args.text("text").map(str::to_owned) else {
        return Err(refuse("missing 'text' (the whole .npcasset body)"));
    };
    let roster = svc.fresh_roster().await;
    let root = svc.root().to_owned();
    // where it goes: an explicit path, an existing character's file, or a new file named after
    // the id the text carries
    let abs = match resolve_target(&args, &root, &roster) {
        Ok(abs) => abs,
        Err(_) => {
            let probe = parse(&text, "character").map_err(refuse)?;
            if probe.id.is_empty() {
                return Err(refuse("the text has no id and no 'path' was given"));
            }
            format!("{}\\characters\\{}.npcasset", normalize(&root), probe.id)
        }
    };
    // a deviation from the C++, which wrote wherever an absolute path pointed
    if !inside(&root, &abs) {
        return Err(refuse(
            "path is outside the project folder - characters are written into the project only",
        ));
    }
    let wire = project_relative(&root, &abs);
    let mut c = parse(&text, base_name(&abs)).map_err(refuse)?;
    let target = PathBuf::from(&abs);
    ed.exec
        .io(move || gz_editor::fsx::write_atomic(&target, text.as_bytes()))
        .await
        .map_err(|_| refuse("cannot create file"))?;
    mark_folders(&svc, &abs);

    // validate what it RESOLVES to, not what the file literally says: a count of 0 is how an
    // inherited row is dropped, and a missing base only shows once the chain is walked
    let roster = svc.fresh_roster().await;
    let own: Vec<String> = roster.problems.iter().filter(|p| p.contains(&wire)).cloned().collect();
    c.file.clone_from(&wire);
    if c.id.is_empty() {
        c.id = crate::model::id_from_name(base_name(&abs));
    }
    if let Some(r) = roster.get(&c.id) {
        c = r.clone();
    }
    let checker = svc.checker(&ed).await;
    let (c, problems) = validated(&ed, checker, c, own).await;
    Ok(json!({ "file": wire, "character": character_json(&c), "problems": problems }))
}

async fn characters_delete(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let svc = open(&ed).await?;
    let roster = svc.fresh_roster().await;
    let root = svc.root().to_owned();
    let abs = resolve_target(&args, &root, &roster).map_err(refuse)?;
    let wire = project_relative(&root, &abs);
    // the rule every disk deletion of the editor follows: gone for good
    let (rep, touched) = {
        let root = root.clone();
        let abs = abs.clone();
        let project = svc.project().clone();
        ed.exec
            .io(move || {
                let no_game = || -> Option<Arc<gz_vfs::GameFiles>> { None };
                let mut ops = FileOps::new(&root, &no_game, project.write_guard(None));
                let mut rep = Report::default();
                ops.delete(&abs, false, &mut rep);
                (rep, ops.touched)
            })
            .await
    };
    for dir in &touched {
        svc.project().tree.mark_fs(Path::new(dir));
    }
    if !rep.ok() {
        return Err(refuse(
            rep.failures.first().map_or_else(|| "delete failed".to_owned(), |f| f.reason.clone()),
        ));
    }
    // the roster without it before the answer: a quest validated next names it as gone
    svc.fresh_roster().await;
    Ok(json!({ "deleted": wire }))
}

// ---- the build ------------------------------------------------------------------------------

/// A coded line as a problem: its code, the rest its message.
fn as_problem(line: &str) -> Problem {
    let (code, message) = line.split_once(' ').unwrap_or((line, ""));
    if code.starts_with('E') { Problem::error(code, message) } else { Problem::warning(code, message) }
}

/// The build makes nothing of a character - the game reads the assets as they are; what is
/// left is saying no to a module whose characters the game could not use (`CheckForBuild`).
/// A character that does not parse stops the build: half a roster ships quests that point at
/// NPCs that do not exist. The refusal is the first error, as the C++ gave it.
struct CharactersCheck;

impl BuildCheck for CharactersCheck {
    fn stage(&self) -> &'static str {
        "Checking the characters"
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        _session: Arc<ProjectSession>,
        _walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            // the roster reads only the files the build ships (`outside_build`)
            let Ok(svc) = Characters::of(&editor).await else { return Vec::new() };
            let roster = svc.fresh_roster().await;
            let all = roster.clone();
            let mut problems = roster.problems.clone();
            problems.extend(
                editor
                    .exec
                    .io(move || {
                        let checker = Checker::unlinked();
                        let shipped: Vec<_> =
                            all.resolved.iter().filter(|c| !c.is_abstract).cloned().collect();
                        checker.validate_all(&shipped)
                    })
                    .await,
            );
            problems.iter().find(|p| p.starts_with('E')).map(|p| vec![as_problem(p)]).unwrap_or_default()
        })
    }
}
