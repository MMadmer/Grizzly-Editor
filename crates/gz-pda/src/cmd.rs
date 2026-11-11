//! The PDA area's commands. The wire is the C++ handlers' (`EditorPda.cpp`, `McpList` ..
//! `McpValidate`): `pda_list`, `pda_get`, `pda_write`, `pda_delete`, `pda_use`, `pda_validate`.
//! `pda_preview` is the preview tab's (it forwards to [`crate::Preview`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, ProjectSession, Registry, Reply};
use gz_project::{FileOps, Project, Report};
use serde_json::{Map, Value, json};

use crate::layout::{Layout, PdaProblem};
use crate::read::{ReadError, base_name, extension, is_pda_path, template};
use crate::service::{PdaService, project_relative, read_text, same_path};

const AREA: &str = "gz-pda";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "pda_list", pda_list);
    reg.add(AREA, "pda_get", pda_get);
    reg.add(AREA, "pda_write", pda_write);
    reg.add(AREA, "pda_delete", pda_delete);
    reg.add(AREA, "pda_use", pda_use);
    reg.add(AREA, "pda_validate", pda_validate);
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A string argument the way `ArgString` read it: present means given, even empty; a bare
/// number or flag is its text.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A number as `%g` printed it, as a JSON number.
fn num_g(v: f32) -> Value {
    let text = gz_luadata::format_g(f64::from(v), 6);
    match text.parse::<f64>() {
        Ok(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => json!(n as i64),
        Ok(n) => json!(n),
        Err(_) => json!(0),
    }
}

/// A path for the wire: `/` separators (`JsonPath`).
fn wire(path: &str) -> String {
    path.replace('\\', "/")
}

/// The outline of a layout (`JsonOutline`).
fn outline(l: &Layout) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(l.id));
    o.insert("title".into(), json!(l.title));
    o.insert("start".into(), json!(l.start));
    o.insert("canvas".into(), json!({ "w": num_g(l.canvas_w), "h": num_g(l.canvas_h) }));
    o.insert("device".into(), json!({ "noise": l.noise, "close": l.close }));
    o.insert("keys".into(), json!({ "map": l.key_map, "tasks": l.key_tasks, "contacts": l.key_contacts }));
    let pages: Vec<Value> = l
        .pages
        .iter()
        .map(|p| {
            let widgets = l.items.iter().filter(|it| it.page == p.id && it.parent == 0).count();
            let mut po = Map::new();
            po.insert("id".into(), json!(p.id));
            po.insert("title".into(), json!(p.title));
            if !p.stock.is_empty() {
                po.insert("stock".into(), json!(p.stock));
            }
            po.insert("widgets".into(), json!(widgets));
            Value::Object(po)
        })
        .collect();
    o.insert("pages".into(), Value::Array(pages));
    let vars: Map<String, Value> = l.vars.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    o.insert("vars".into(), Value::Object(vars));
    // the addressable items: what show:/hide:/toggle: and xms.pda name
    let ids: Vec<Value> = l
        .items
        .iter()
        .filter(|it| !it.id.is_empty())
        .map(|it| {
            let mut io = Map::new();
            io.insert("id".into(), json!(it.id));
            io.insert("kind".into(), json!(it.kind));
            io.insert("layer".into(), json!(it.layer));
            if !it.page.is_empty() {
                io.insert("page".into(), json!(it.page));
            }
            io.insert("visible".into(), json!(it.visible));
            Value::Object(io)
        })
        .collect();
    o.insert("ids".into(), Value::Array(ids));
    o.insert("items".into(), json!(l.items.len()));
    Value::Object(o)
}

/// The problems of a layout and the textures W320 names (`JsonProblems`).
fn problems_json(problems: &[PdaProblem], missing: &[String]) -> Value {
    let mut out: Vec<Value> =
        problems.iter().map(|p| json!({ "code": p.code, "path": p.path, "message": p.message })).collect();
    for t in missing {
        out.push(json!({ "code": "W320", "path": "", "message": format!("texture '{t}' is neither in the linked game nor in the project") }));
    }
    Value::Array(out)
}

struct Open {
    session: Arc<ProjectSession>,
    project: Arc<Project>,
    service: Arc<PdaService>,
}

fn open(ed: &Arc<Editor>) -> Result<Open, CmdError> {
    let (session, project) = Project::of(ed)?;
    let service = PdaService::of_session(ed, &session);
    Ok(Open { session, project, service })
}

/// The file a command is about: `path`, or the layout the module uses (`ResolvePath`). The
/// absolute path with `\` and the project-relative one with `/`.
async fn resolve(ed: &Arc<Editor>, o: &Open, args: &Args) -> Result<(String, String), CmdError> {
    let mut path = arg(args, "path").unwrap_or_default();
    if path.is_empty() {
        path = o.service.used_layout(ed).await;
        if path.is_empty() {
            return Err(refuse("missing 'path' (and the module uses no PDA layout yet)"));
        }
    }
    let abs =
        gz_project::ops::resolve_in_project(o.project.root(), &path).map_err(refuse)?.replace('/', "\\");
    if !is_pda_path(&abs) {
        return Err(refuse("a PDA layout is a .pdaasset file"));
    }
    let rel = project_relative(o.project.root(), &abs);
    Ok((abs, rel))
}

async fn read_file(ed: &Arc<Editor>, abs: &str) -> Result<String, String> {
    let p = PathBuf::from(abs);
    ed.exec.io(move || read_text(&p)).await
}

async fn file_exists(ed: &Arc<Editor>, abs: &str) -> bool {
    let p = PathBuf::from(abs);
    ed.exec.io(move || p.is_file()).await
}

/// The project tree re-lists the folder of a file that was written or deleted.
fn touch(o: &Open, abs: &str) {
    if let Some(dir) = Path::new(abs).parent() {
        o.project.tree.mark_fs(dir);
    }
}

async fn pda_list(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let o = open(&ed)?;
    let files = o.service.list(&ed).await?;
    let used = o.service.used_layout(&ed).await;
    let lang = arg(&args, "lang").unwrap_or_else(|| "rus".to_owned());
    let reader = o.service.reader_source(&ed).await;
    let mut layouts = Vec::with_capacity(files.len());
    for rel_bs in &files {
        let abs = o.session.root.join(rel_bs).to_string_lossy().into_owned();
        let rel = wire(rel_bs);
        let read = match read_file(&ed, &abs).await {
            Ok(text) => o.service.evaluate(&ed, Arc::from(text), &base_name(&abs), &lang).await,
            Err(e) => Err(ReadError { message: e, line: 0 }),
        };
        let mut e = Map::new();
        e.insert("path".into(), json!(rel));
        e.insert("used".into(), json!(same_path(&rel, &used)));
        match read {
            Err(err) => {
                e.insert("error".into(), json!(err.with_line()));
            }
            Ok(l) => {
                e.insert("id".into(), json!(l.id));
                e.insert("title".into(), json!(l.title));
                e.insert("pages".into(), json!(l.pages.len()));
                e.insert("items".into(), json!(l.items.len()));
                e.insert("errors".into(), json!(l.errors));
                e.insert("warnings".into(), json!(l.warnings));
            }
        }
        layouts.push(Value::Object(e));
    }
    Ok(json!({ "used": wire(&used), "reader": wire(&reader), "layouts": layouts }))
}

async fn pda_get(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let o = open(&ed)?;
    let (abs, rel) = resolve(&ed, &o, &args).await?;
    let text = read_file(&ed, &abs).await.map_err(refuse)?;
    let lang = arg(&args, "lang").unwrap_or_else(|| "rus".to_owned());
    let read = o.service.evaluate(&ed, Arc::from(text.as_str()), &base_name(&abs), &lang).await;
    let used = o.service.used_layout(&ed).await;
    let mut out = Map::new();
    out.insert("path".into(), json!(rel));
    out.insert("used".into(), json!(same_path(&rel, &used)));
    out.insert("text".into(), json!(text));
    let l = match read {
        Err(e) => {
            out.insert("parse_error".into(), json!(e.message));
            out.insert("line".into(), json!(e.line));
            return Ok(Value::Object(out));
        }
        Ok(l) => l,
    };
    let missing = o.service.missing_textures(&ed, &o.session, &l).await;
    out.insert("layout".into(), outline(&l));
    out.insert("errors".into(), json!(l.errors));
    out.insert("warnings".into(), json!(l.warnings as usize + missing.len()));
    out.insert("problems".into(), problems_json(&l.problems, &missing));
    out.insert("reader".into(), json!(wire(&o.service.reader_source(&ed).await)));
    Ok(Value::Object(out))
}

async fn pda_write(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let o = open(&ed)?;
    let mut text = arg(&args, "text").unwrap_or_default();
    let mut path = arg(&args, "path").unwrap_or_default();
    let id = arg(&args, "id").unwrap_or_default();
    let title = arg(&args, "title").unwrap_or_default();
    if path.is_empty() {
        if id.is_empty() {
            return Err(refuse("missing 'path' (or 'id' for a new layout under pda\\)"));
        }
        path = format!("pda\\{id}.pdaasset");
    }
    let mut abs =
        gz_project::ops::resolve_in_project(o.project.root(), &path).map_err(refuse)?.replace('/', "\\");
    // a bare name gets the family's extension, the way the browser's rename does
    if extension(&abs).is_empty() {
        abs.push_str(".pdaasset");
    }
    if !is_pda_path(&abs) {
        return Err(refuse("a PDA layout is a .pdaasset file"));
    }
    let existed = file_exists(&ed, &abs).await;
    // no text: a new layout from the template (never over an existing file)
    if text.is_empty() {
        if existed {
            return Err(refuse("missing 'text' (the whole .pdaasset body)"));
        }
        text = template(if id.is_empty() { base_name(&abs) } else { id.clone() }.as_str(), &title);
    }
    let read = o.service.evaluate(&ed, Arc::from(text.as_str()), &base_name(&abs), "rus").await;
    let l = match read {
        Ok(l) => l,
        Err(e) => return Err(refuse(e.with_line()).with_data(json!({ "line": e.line, "written": false }))),
    };
    let target = PathBuf::from(&abs);
    let bytes = text.into_bytes();
    ed.exec
        .io(move || {
            if let Some(dir) = target.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            gz_editor::fsx::write_atomic(&target, &bytes)
        })
        .await
        .map_err(|_| refuse("cannot create file"))?;
    touch(&o, &abs);
    let rel = project_relative(o.project.root(), &abs);
    // the first layout of a module is the one it uses, unless the caller says otherwise
    let used_before = o.service.used_layout(&ed).await;
    let mut use_changed = false;
    if args.bool("use").unwrap_or(used_before.is_empty()) {
        use_changed = o.service.use_layout(&ed, &rel, origin).await.is_ok();
    }
    let missing = o.service.missing_textures(&ed, &o.session, &l).await;
    let used = o.service.used_layout(&ed).await;
    let mut out = Map::new();
    out.insert("path".into(), json!(rel));
    out.insert("created".into(), json!(!existed));
    out.insert("used".into(), json!(same_path(&rel, &used)));
    out.insert("layout".into(), outline(&l));
    out.insert("errors".into(), json!(l.errors));
    out.insert("warnings".into(), json!(l.warnings as usize + missing.len()));
    out.insert("problems".into(), problems_json(&l.problems, &missing));
    out.insert("use_changed".into(), json!(use_changed));
    Ok(Value::Object(out))
}

async fn pda_delete(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let o = open(&ed)?;
    let (abs, rel) = resolve(&ed, &o, &args).await?;
    if !file_exists(&ed, &abs).await {
        return Err(refuse(format!("{rel} does not exist")));
    }
    let root = o.project.root().to_owned();
    let victim = abs.clone();
    let project = o.project.clone();
    let (gone, touched) = ed
        .exec
        .io(move || {
            let game = || project.game_files().ok();
            let mut ops = FileOps::new(&root, &game);
            let mut rep = Report::default();
            ops.delete(&victim, false, &mut rep);
            (!Path::new(&victim).is_file(), ops.touched)
        })
        .await;
    for dir in &touched {
        o.project.tree.mark_fs(Path::new(dir));
    }
    if !gone {
        return Err(refuse(format!("{rel} could not be deleted")));
    }
    touch(&o, &abs);
    // the module does not keep naming a file that is gone
    let mut cleared = false;
    if same_path(&rel, &o.service.used_layout(&ed).await) {
        cleared = o.service.use_layout(&ed, "", origin).await.is_ok();
    }
    Ok(json!({ "deleted": rel, "use_cleared": cleared }))
}

async fn pda_use(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let o = open(&ed)?;
    let path = arg(&args, "path").unwrap_or_default();
    o.service.use_layout(&ed, &path, origin).await.map_err(refuse)?;
    Ok(json!({ "used": wire(&o.service.used_layout(&ed).await) }))
}

async fn pda_validate(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let o = open(&ed)?;
    let lang = arg(&args, "lang").unwrap_or_else(|| "rus".to_owned());
    let mut label = "text".to_owned();
    // a text to check before writing it, or a file
    let text = match arg(&args, "text").filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => {
            let (abs, rel) = resolve(&ed, &o, &args).await?;
            let t = read_file(&ed, &abs).await.map_err(refuse)?;
            label = rel;
            t
        }
    };
    let l = match o.service.evaluate(&ed, Arc::from(text), &base_name(&label), &lang).await {
        Ok(l) => l,
        Err(e) => return Ok(json!({ "valid": false, "parse_error": e.message, "line": e.line })),
    };
    let missing = o.service.missing_textures(&ed, &o.session, &l).await;
    let mut out = Map::new();
    out.insert("valid".into(), json!(l.ok));
    out.insert("errors".into(), json!(l.errors));
    out.insert("warnings".into(), json!(l.warnings as usize + missing.len()));
    out.insert("problems".into(), problems_json(&l.problems, &missing));
    out.insert("layout".into(), outline(&l));
    Ok(Value::Object(out))
}
