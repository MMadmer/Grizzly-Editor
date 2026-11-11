//! The `quest_*` commands (`NqMcp::Handle`, and the graph tab's `McpEdit` / `McpFind` /
//! `McpBookmarks` / `McpHistory` / `McpMinimap`). The wire is the C++ one: the same names, the
//! same forgiving arguments, the same fields in the same order, the same error texts.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_project::{AssetTemplates, ProjectHooks, TemplateKind};
use serde_json::{Map, Value, json};

use crate::asset::{self, Graph};
use crate::check;
use crate::doc::{self, CanvasOp, DocOp, ViewOp, zoom_of};
use crate::layout;
use crate::pickers::{PickerEntry, PickerType};
use crate::references;
use crate::service::{DocHandle, NqDirty, NqService};
use crate::text::{self, contains_no_case};
use crate::validate::{self, NqProblem};

const AREA: &str = "gz-nq";
const NEED_PATH: &str =
    "missing 'path' (project-relative, e.g. quests/wolf_debt.nqasset or npc/ambush.behasset)";

fn refused(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A string argument the way `NqUtil::ArgString` read it: present means given, even empty; a
/// bare number or flag is its text.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn need_path(args: &Args) -> Result<String, CmdError> {
    arg(args, "path").filter(|p| !p.is_empty()).ok_or_else(|| refused(NEED_PATH))
}

fn service(ed: &Editor) -> Result<Arc<NqService>, CmdError> {
    NqService::of(ed)
}

fn r1(v: f32) -> Value {
    json!((f64::from(v) * 10.0).round() / 10.0)
}

/// `path`, `id`, `title`, `nodes`, `errors`, `warnings`, `problems` [, `outline`] [, `lua`]
/// (`AppendSummary`).
fn summary(
    out: &mut Map<String, Value>,
    catalog: &crate::Catalog,
    wire: &str,
    g: &Graph,
    problems: &[NqProblem],
    with_outline: bool,
    with_lua: bool,
) {
    out.insert("path".into(), json!(wire));
    out.insert("id".into(), json!(g.id));
    out.insert("title".into(), json!(text::preview(&g.title)));
    out.insert("nodes".into(), json!(g.nodes.len()));
    out.insert("errors".into(), json!(validate::count_errors(problems)));
    out.insert("warnings".into(), json!(validate::count_warnings(problems)));
    out.insert("problems".into(), validate::to_json(problems));
    if with_outline {
        let mut ol = asset::outline(catalog, g);
        ol.push_str(&validate::to_text(problems));
        out.insert("outline".into(), json!(ol));
    }
    if with_lua {
        out.insert("lua".into(), json!(asset::write(catalog, g)));
    }
}

/// `open`, `dirty` [, `undo`, `redo`] (`AppendDocState`).
fn doc_state(out: &mut Map<String, Value>, d: Option<&DocHandle>) {
    out.insert("open".into(), json!(d.is_some()));
    out.insert("dirty".into(), json!(d.is_some_and(DocHandle::is_dirty)));
    if let Some(d) = d {
        let h = d.store.history();
        out.insert("undo".into(), json!(h.undo.len()));
        out.insert("redo".into(), json!(h.redo.len()));
    }
}

async fn respond_doc(
    ed: &Editor,
    svc: &NqService,
    d: &DocHandle,
    with_outline: bool,
    with_lua: bool,
) -> Map<String, Value> {
    let catalog = svc.catalog(ed).await;
    let problems = svc.problems(ed, d).await;
    let snap = d.store.snapshot();
    let mut out = Map::new();
    summary(&mut out, &catalog, &d.path().wire, &snap.graph, &problems, with_outline, with_lua);
    doc_state(&mut out, Some(d));
    out
}

/// A document that exists on disk opens on demand: MCP works without a canvas.
async fn open_for_edit(ed: &Editor, svc: &NqService, path: &str) -> Result<Arc<DocHandle>, CmdError> {
    svc.open(ed, path).await.map_err(refused)
}

/// Lua table text may come with or without `return` (`AsChunk`).
fn as_chunk(text: &str) -> String {
    let t = text::trim(text);
    if t.starts_with('{') { format!("return {t}") } else { text.to_owned() }
}

// ---- the commands -----------------------------------------------------------------------------

async fn quest_catalog(ed: Arc<Editor>, _args: Args, _o: Origin) -> Reply {
    let svc = service(&ed)?;
    svc.catalog(&ed).await.to_json().map_err(refused)
}

async fn quest_list(ed: Arc<Editor>, _args: Args, _o: Origin) -> Reply {
    let svc = service(&ed)?;
    svc.refresh_tree().await;
    let index = svc.index(&ed, true).await;
    let env = svc.env(&ed, true).await;
    let mut quests = Vec::with_capacity(index.entries.len());
    for e in &index.entries {
        let mut o = Map::new();
        o.insert("path".into(), json!(e.rel));
        if !e.usable() {
            o.insert("readable".into(), json!(false));
            o.insert("error".into(), json!(e.error));
            o.insert("errors".into(), json!(1));
            o.insert("warnings".into(), json!(0));
            quests.push(Value::Object(o));
            continue;
        }
        let d = svc.get_doc(&e.rel);
        let (g, problems): (Graph, Arc<Vec<NqProblem>>) = match &d {
            Some(d) => (d.store.snapshot().graph.clone(), svc.problems(&ed, d).await),
            None => ((*e.graph).clone(), Arc::new(env.validate(&e.graph, &e.rel))),
        };
        o.insert("readable".into(), json!(true));
        o.insert("id".into(), json!(g.id));
        // a behaviour graph is listed beside the quests but is not one
        o.insert("kind".into(), json!(g.kind.key()));
        o.insert("title".into(), json!(text::preview(&g.title)));
        o.insert("nodes".into(), json!(g.nodes.len()));
        o.insert("errors".into(), json!(validate::count_errors(&problems)));
        o.insert("warnings".into(), json!(validate::count_warnings(&problems)));
        o.insert("open".into(), json!(d.is_some()));
        o.insert("dirty".into(), json!(d.as_ref().is_some_and(|d| d.is_dirty())));
        quests.push(Value::Object(o));
    }
    Ok(json!({ "count": quests.len(), "quests": quests }))
}

async fn quest_new(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    let p = svc.resolve(&path, true).map_err(refused)?;
    let abs = p.abs.clone();
    if ed.exec.io(move || abs.exists()).await {
        return Err(refused("file already exists"));
    }
    // the extension says what is being made: .behasset is a behaviour graph
    let text = asset::template_for(&p.wire);
    svc.write_file(&p, text).await.map_err(refused)?;
    let d = svc.open(&ed, &p.wire).await.map_err(refused)?;
    Ok(Value::Object(respond_doc(&ed, &svc, &d, true, true).await))
}

async fn quest_open(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    let d = open_for_edit(&ed, &svc, &path).await?;
    Ok(Value::Object(respond_doc(&ed, &svc, &d, true, false).await))
}

async fn quest_close(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    match svc.close(&path, args.flag("discard")) {
        Ok(()) => Ok(Value::Null),
        Err(e) if e == "unsaved" => Err(CmdError::new("unsaved", "unsaved")),
        Err(e) => Err(refused(e)),
    }
}

/// The summary of a graph that is not open: read from the disk and validated.
async fn closed_summary(
    ed: &Editor,
    svc: &NqService,
    path: &str,
    with_lua: bool,
) -> Result<Map<String, Value>, CmdError> {
    let (g, p) = svc.read_graph(ed, path).await.map_err(refused)?;
    let env = svc.env(ed, true).await;
    let problems = env.validate(&g, &p.wire);
    let mut out = Map::new();
    summary(&mut out, &env.catalog, &p.wire, &g, &problems, true, with_lua);
    doc_state(&mut out, None);
    Ok(out)
}

async fn quest_get(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    if let Some(d) = svc.get_doc(&path) {
        return Ok(Value::Object(respond_doc(&ed, &svc, &d, true, true).await));
    }
    svc.resolve(&path, false).map_err(refused)?;
    Ok(Value::Object(closed_summary(&ed, &svc, &path, true).await?))
}

async fn quest_write(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let Some(lua) = arg(&args, "lua").filter(|l| !l.is_empty()) else {
        return Err(refused("missing 'lua' (the whole file text: return { nq = 1, ... })"));
    };
    let svc = service(&ed)?;
    let p = svc.resolve(&path, true).map_err(refused)?;
    let chunk = as_chunk(&lua);
    let catalog = svc.catalog(&ed).await;
    let name = asset::base_name(&p.wire);
    let v = asset::parse_value(&chunk, &name).map_err(|e| refused(e.with_position()))?;
    let (g, shape) = asset::graph_from_value(&catalog, &v);
    if let Some(d) = svc.get_doc(&p.wire) {
        // open document: replaced in memory with an undo step, saved by quest_save. The reader
        // DROPS what it cannot type: taking the text anyway would put a document into the tab
        // that no longer says what the caller wrote
        if !shape.is_empty() {
            return Err(refused(shape.join("; ")));
        }
        svc.send(&ed, &d, DocOp::Replace(g), Origin::Mcp).await?;
        let mut out = Map::new();
        out.insert("saved".into(), json!(false));
        out.extend(respond_doc(&ed, &svc, &d, true, false).await);
        return Ok(Value::Object(out));
    }
    // closed (or new) file: the canonical text goes to disk when it parses; problems are
    // reported, only the build refuses them
    if !shape.is_empty() {
        return Err(refused(format!("nothing written - the editor cannot represent: {}", shape.join("; "))));
    }
    svc.write_file(&p, asset::write(&catalog, &g)).await.map_err(|e| refused(format!("{e}: {}", p.wire)))?;
    let env = svc.env(&ed, true).await;
    let problems = env.validate(&g, &p.wire);
    let mut out = Map::new();
    out.insert("saved".into(), json!(true));
    summary(&mut out, &catalog, &p.wire, &g, &problems, true, false);
    doc_state(&mut out, None);
    Ok(Value::Object(out))
}

async fn undo_redo(ed: Arc<Editor>, args: Args, undo: bool) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    let d = svc.get_doc(&path).ok_or_else(|| refused("not open"))?;
    let done = if undo { d.store.undo().await } else { d.store.redo().await };
    if done.is_none() {
        return Err(refused(if undo { "nothing to undo" } else { "nothing to redo" }));
    }
    Ok(Value::Object(respond_doc(&ed, &svc, &d, true, false).await))
}

async fn quest_undo(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    undo_redo(ed, args, true).await
}

async fn quest_redo(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    undo_redo(ed, args, false).await
}

async fn quest_apply(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let path = need_path(&args)?;
    let Some(ops) = arg(&args, "ops").filter(|s| !s.is_empty()) else {
        return Err(refused(
            "missing 'ops' (Lua list of operations: { { op = \"add_node\", node = { ... } }, ... })",
        ));
    };
    let svc = service(&ed)?;
    let d = open_for_edit(&ed, &svc, &path).await?;
    let v =
        asset::parse_value(&as_chunk(&ops), "ops").map_err(|e| refused(format!("ops: {}", e.with_line())))?;
    let reply = svc.send(&ed, &d, DocOp::ApplyOps(v), o).await?;
    let mut out = Map::new();
    out.insert("applied".into(), json!(reply.count));
    out.extend(respond_doc(&ed, &svc, &d, true, false).await);
    Ok(Value::Object(out))
}

async fn quest_validate(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    if let Some(d) = svc.get_doc(&path) {
        return Ok(Value::Object(respond_doc(&ed, &svc, &d, true, false).await));
    }
    svc.resolve(&path, false).map_err(refused)?;
    Ok(Value::Object(closed_summary(&ed, &svc, &path, false).await?))
}

async fn quest_save(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    let d = svc.get_doc(&path).ok_or_else(|| refused("not open"))?;
    svc.save(&ed, &d, args.flag("force")).await.map_err(refused)?;
    let problems = svc.problems(&ed, &d).await;
    Ok(json!({
        "path": d.path().wire,
        "errors": validate::count_errors(&problems),
        "warnings": validate::count_warnings(&problems),
    }))
}

async fn quest_reload(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    let d = svc.get_doc(&path).ok_or_else(|| refused("not open"))?;
    svc.reload(&ed, &d).await.map_err(refused)?;
    Ok(Value::Object(respond_doc(&ed, &svc, &d, true, false).await))
}

async fn quest_layout(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let path = need_path(&args)?;
    let svc = service(&ed)?;
    let d = open_for_edit(&ed, &svc, &path).await?;
    let reply =
        svc.send(&ed, &d, DocOp::Layout { only_missing: !args.bool("all").unwrap_or(true) }, o).await?;
    let catalog = svc.catalog(&ed).await;
    let snap = d.store.snapshot();
    let positions: Vec<Value> = snap
        .graph
        .nodes
        .iter()
        .map(|n| {
            // the box the canvas draws: content-sized, the same one the layout made room for
            let s = layout::node_size(&catalog, n);
            json!({ "id": n.id, "x": n.pos[0], "y": n.pos[1], "w": s[0], "h": s[1] })
        })
        .collect();
    let mut out = Map::new();
    out.insert("moved".into(), json!(reply.count));
    out.insert("positions".into(), Value::Array(positions));
    doc_state(&mut out, Some(&d));
    Ok(Value::Object(out))
}

async fn quest_check_all(ed: Arc<Editor>, _args: Args, _o: Origin) -> Reply {
    let svc = service(&ed)?;
    let r = check::run(&ed, &svc).await;
    Ok(json!({
        "passed": r.errors == 0,
        "errors": r.errors,
        "warnings": r.warnings,
        "error": r.error(),
        "log": r.log,
    }))
}

async fn quest_project_find(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let Some(query) = arg(&args, "query") else { return Err(refused("missing 'query'")) };
    let svc = service(&ed)?;
    let index = svc.index(&ed, true).await;
    let catalog = svc.catalog(&ed).await;
    let found = crate::index::find_nodes(&index, &catalog, &query);
    let limit = usize::try_from(args.i64("limit").unwrap_or(200).clamp(1, 1000)).unwrap_or(200);
    let count = limit.min(found.results.len());
    let results: Vec<Value> = found
        .results
        .iter()
        .take(count)
        .map(|(path, h, source)| json!({ "path": path, "node": h.node, "kind": h.kind, "title": h.title, "match": h.matched, "source": source }))
        .collect();
    Ok(json!({
        "query": query,
        "count": found.results.len(),
        "returned": count,
        "truncated": count < found.results.len(),
        "complete": found.complete,
        "generation": found.generation,
        "fingerprint": crate::index::fingerprint_text(found.fingerprint),
        "results": results,
        "diagnostics": found.diagnostics,
    }))
}

async fn quest_references(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let Some(ty) = arg(&args, "type").filter(|s| !s.is_empty()) else {
        return Err(refused("missing 'type'"));
    };
    let Some(value) = arg(&args, "value").filter(|s| !s.is_empty()) else {
        return Err(refused("missing 'value'"));
    };
    let svc = service(&ed)?;
    let scope = match arg(&args, "path").filter(|s| !s.is_empty()) {
        Some(p) => Some(svc.resolve(&p, false).map_err(refused)?.wire),
        None => None,
    };
    let index = svc.index(&ed, true).await;
    let catalog = svc.catalog(&ed).await;
    let r = references::find(&index, &catalog, &ty, &value, scope.as_deref()).map_err(refused)?;
    let limit = usize::try_from(args.i64("limit").unwrap_or(500).clamp(1, 2000)).unwrap_or(500);
    let count = limit.min(r.references.len());
    Ok(json!({
        "type": ty,
        "value": value,
        "path": scope,
        "count": r.references.len(),
        "returned": count,
        "truncated": count < r.references.len(),
        "complete": r.complete,
        "generation": r.generation,
        "fingerprint": crate::index::fingerprint_text(r.fingerprint),
        "catalog_generation": r.catalog_generation,
        "references": r.references.iter().take(count).map(references::Reference::to_json).collect::<Vec<_>>(),
        "diagnostics": r.diagnostics.iter().map(references::Diagnostic::to_json).collect::<Vec<_>>(),
    }))
}

async fn quest_rename_task(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    if !args.flag("ack_runtime_identity") {
        return Err(refused(
            "task rename changes runtime/save identity; pass ack_runtime_identity=true explicitly",
        ));
    }
    let path = need_path(&args)?;
    let Some(from) = arg(&args, "from").filter(|s| !s.is_empty()) else {
        return Err(refused("missing 'from'"));
    };
    let Some(to) = arg(&args, "to").filter(|s| !s.is_empty()) else { return Err(refused("missing 'to'")) };
    let svc = service(&ed)?;
    let d = open_for_edit(&ed, &svc, &path).await?;
    let wire = d.path().wire;
    let reply =
        svc.send(&ed, &d, DocOp::RenameTask { from: from.clone(), to: to.clone(), path: wire }, o).await?;
    let mut out = Map::new();
    out.insert("from".into(), json!(from));
    out.insert("to".into(), json!(to));
    out.insert("references_updated".into(), json!(reply.count));
    out.insert("runtime_identity_changed".into(), json!(true));
    out.extend(respond_doc(&ed, &svc, &d, true, false).await);
    Ok(Value::Object(out))
}

async fn quest_lookup(ed: Arc<Editor>, args: Args, _o: Origin) -> Reply {
    let ty = arg(&args, "type").unwrap_or_default();
    let query = arg(&args, "query").unwrap_or_default();
    let path = arg(&args, "path").unwrap_or_default();
    let limit = usize::try_from(args.i64("limit").unwrap_or(50)).ok().filter(|&l| l > 0).unwrap_or(50);
    if ty.is_empty() {
        return Err(refused(
            "missing 'type' (item_section, squad_section, level, smart, story_id, profile, community, info, spot_type, task_id, var_name, global_name, ref_name, node_id, quest_id, behaviour_id, role, way_path, character_id, level_changer)",
        ));
    }
    let svc = service(&ed)?;
    let picker = PickerType::from_name(&ty);
    let mut doc_entries: Vec<PickerEntry> = Vec::new();
    if picker.is_none() {
        let entry = |id: String, name: String, extra: String| PickerEntry { id, name, extra };
        match ty.as_str() {
            "quest_id" | "behaviour_id" => {
                // one project index, two kinds of document: each call names its own
                let want = ty == "behaviour_id";
                let index = svc.index(&ed, true).await;
                for e in index.entries.iter().filter(|e| e.behaviour() == want) {
                    let (id, title) = if e.readable {
                        (e.graph.id.clone(), text::preview(&e.graph.title))
                    } else {
                        (String::new(), String::new())
                    };
                    doc_entries.push(entry(id, title, e.rel.clone()));
                }
            }
            "way_path" => doc_entries = svc.way_paths(),
            "global_name" => {
                for (name, value) in svc.global_vars(&ed).await {
                    doc_entries.push(entry(name, value, "mod.ltx [vars]".into()));
                }
            }
            "character_id" => {
                // a template is listed and marked: it explains itself instead of being missing
                for b in svc.characters().1.iter() {
                    let extra =
                        if b.template { "template - cannot be spawned".to_owned() } else { b.file.clone() };
                    doc_entries.push(entry(b.id.clone(), b.name.clone(), extra));
                }
            }
            "role" | "task_id" | "var_name" | "ref_name" | "node_id" => {
                if path.is_empty() {
                    return Err(refused("document-scoped type needs 'path'"));
                }
                let (g, _) = svc.read_graph(&ed, &path).await.map_err(refused)?;
                match ty.as_str() {
                    "role" => {
                        for r in &g.roles {
                            doc_entries.push(entry(
                                r.name.clone(),
                                text::preview(&r.title),
                                (if r.many { "group" } else { "one" }).into(),
                            ));
                        }
                    }
                    "task_id" => {
                        for t in &g.tasks {
                            doc_entries.push(entry(t.id.clone(), text::preview(&t.title), String::new()));
                        }
                    }
                    "var_name" => {
                        for v in &g.vars {
                            doc_entries.push(entry(
                                v.name.clone(),
                                text::value_summary(&v.value, 48),
                                String::new(),
                            ));
                        }
                    }
                    "node_id" => {
                        for n in &g.nodes {
                            doc_entries.push(entry(n.id.to_string(), n.kind.to_string(), String::new()));
                        }
                    }
                    _ => {
                        // the category the picker shows a human: which ref belongs in an item slot
                        let catalog = svc.catalog(&ed).await;
                        for (r, kind) in validate::declared_refs(&g, &catalog) {
                            doc_entries.push(entry(r, String::new(), kind.to_owned()));
                        }
                    }
                }
            }
            _ => return Err(refused("unknown lookup type")),
        }
    }
    let pickers = svc.pickers(&ed).await;
    let (found, total, available): (Vec<&PickerEntry>, usize, bool) = match picker {
        None => {
            let found = doc_entries
                .iter()
                .filter(|e| {
                    query.is_empty() || contains_no_case(&e.id, &query) || contains_no_case(&e.name, &query)
                })
                .take(limit)
                .collect();
            (found, doc_entries.len(), true)
        }
        Some(t) => (pickers.search(t, &query, limit), pickers.entries(t).len(), pickers.available()),
    };
    let entries: Vec<Value> = found
        .iter()
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
    Ok(json!({ "type": ty, "available": available, "total": total, "entries": entries }))
}

// ---- the graph tab's operations ---------------------------------------------------------------

/// `zoom_level`, `zoom`, `center`, `selected`, `slot` (`AppendViewState`).
fn view_state(out: &mut Map<String, Value>, v: &doc::View) {
    out.insert("zoom_level".into(), json!(v.zoom));
    out.insert("zoom".into(), json!((f64::from(zoom_of(v.zoom)) * 1000.0).round() / 1000.0));
    out.insert("center".into(), json!([r1(v.center[0]), r1(v.center[1])]));
    out.insert("selected".into(), json!(v.selection));
    out.insert("slot".into(), json!(v.slot));
}

/// The tab's document (`OpenMcpWindow`): opened on demand.
async fn tab_doc(ed: &Editor, svc: &NqService, args: &Args) -> Result<Arc<DocHandle>, CmdError> {
    let Some(path) = arg(args, "path").filter(|p| !p.is_empty()) else {
        return Err(refused("path is required"));
    };
    svc.open(ed, &path).await.map_err(refused)
}

fn split_ids(list: &str) -> Vec<String> {
    list.split(',').map(text::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// The handler of `quest_edit`, for the UI area that brings the tab up first.
pub async fn quest_edit(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    const OPS: &str = "connect, disconnect, bypass, delete, duplicate, connect_nearest, select_all";
    let svc = service(&ed)?;
    let d = tab_doc(&ed, &svc, &args).await?;
    let Some(op) = arg(&args, "op").filter(|s| !s.is_empty()) else {
        return Err(refused(format!("missing 'op': {OPS}")));
    };
    // `nodes` overrides the selection for the ops that take one: a caller never selects first
    if let Some(nodes) = arg(&args, "nodes").filter(|s| !s.is_empty()) {
        let snap = d.store.snapshot();
        let ids = split_ids(&nodes);
        if let Some(missing) = ids.iter().find(|id| snap.graph.find_node(id).is_none()) {
            return Err(refused(format!("no node '{missing}' in this graph")));
        }
        svc.send(&ed, &d, DocOp::View(ViewOp::Select(ids)), o).await?;
    }
    let from = arg(&args, "from").unwrap_or_default();
    let pin = arg(&args, "pin").unwrap_or_default();
    let to = arg(&args, "to").unwrap_or_default();
    match op.as_str() {
        "connect" => {
            if from.is_empty() || pin.is_empty() || to.is_empty() {
                return Err(refused("connect needs 'from', 'pin' and 'to'"));
            }
            let catalog = svc.catalog(&ed).await;
            doc::can_link(&catalog, &d.store.snapshot().graph, &from, &to).map_err(refused)?;
            svc.send(&ed, &d, DocOp::Connect { from, pin, to }, o).await?;
        }
        "disconnect" => {
            // no 'to' drops every target of the pin, no 'pin' every pin
            if from.is_empty() {
                return Err(refused("disconnect needs 'from' (plus 'pin' and 'to' to name one link)"));
            }
            svc.send(&ed, &d, DocOp::Disconnect { from, pin, to }, o).await?;
        }
        "bypass" => {
            svc.send(&ed, &d, DocOp::Canvas(CanvasOp::Bypass), o).await?;
        }
        "delete" => {
            svc.send(&ed, &d, DocOp::Canvas(CanvasOp::Delete), o).await?;
        }
        "duplicate" => {
            svc.send(&ed, &d, DocOp::Canvas(CanvasOp::Duplicate), o).await?;
        }
        "connect_nearest" => {
            svc.send(&ed, &d, DocOp::Canvas(CanvasOp::ConnectNearest), o).await?;
        }
        "select_all" => {
            svc.send(&ed, &d, DocOp::Canvas(CanvasOp::SelectAll), o).await?;
        }
        other => return Err(refused(format!("unknown op '{other}': {OPS}"))),
    }
    let catalog = svc.catalog(&ed).await;
    let snap = d.store.snapshot();
    let p = d.path();
    // the outline is how a caller sees what the edit did, without a screenshot
    // oracle: the C++ answered this command's path as the absolute file, not project-relative
    Ok(json!({
        "path": p.abs.to_string_lossy().replace('\\', "/"),
        "op": op,
        "selected": snap.view.selection,
        "status": snap.view.status,
        "outline": asset::outline(&catalog, &snap.graph),
    }))
}

/// The handler of `quest_find`, for the UI area that brings the tab up first.
pub async fn quest_find(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = tab_doc(&ed, &svc, &args).await?;
    let query = arg(&args, "query");
    let action = arg(&args, "action")
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| (if query.is_some() { "set" } else { "get" }).to_owned());
    if query.as_ref().is_some_and(|q| q.len() >= 256) {
        return Err(refused("query is too long (max 255 UTF-8 bytes)"));
    }
    let catalog = svc.catalog(&ed).await;
    let results_of = |d: &DocHandle| {
        let snap = d.store.snapshot();
        let r = doc::find_results(&catalog, &snap.graph, &snap.view.find_query);
        let i = snap.find_index(&r);
        (r, i)
    };
    match action.as_str() {
        "set" => {
            svc.send(&ed, &d, DocOp::View(ViewOp::FindQuery(query.clone().unwrap_or_default())), o).await?;
            if args.flag("select") {
                svc.send(&ed, &d, DocOp::View(ViewOp::FindGo(0)), o).await?;
            }
        }
        "next" | "previous" => {
            if let Some(q) = &query {
                svc.send(&ed, &d, DocOp::View(ViewOp::FindQuery(q.clone())), o).await?;
            }
            let (_, i) = results_of(&d);
            let to = if action == "previous" { i - 1 } else { i + 1 };
            svc.send(&ed, &d, DocOp::View(ViewOp::FindGo(to)), o).await?;
        }
        "clear" => {
            svc.send(&ed, &d, DocOp::View(ViewOp::FindClear), o).await?;
        }
        "get" => {
            if let Some(q) = &query {
                svc.send(&ed, &d, DocOp::View(ViewOp::FindQuery(q.clone())), o).await?;
            }
        }
        _ => return Err(refused("action must be get, set, next, previous or clear")),
    }
    let (results, index) = results_of(&d);
    let snap = d.store.snapshot();
    let limit = usize::try_from(args.i64("limit").unwrap_or(100).clamp(1, 500)).unwrap_or(100);
    let count = limit.min(results.len());
    let mut out = Map::new();
    out.insert("path".into(), json!(d.path().wire));
    out.insert("query".into(), json!(snap.view.find_query));
    out.insert("count".into(), json!(results.len()));
    out.insert("index".into(), json!(index));
    out.insert(
        "current".into(),
        usize::try_from(index).ok().and_then(|i| results.get(i)).map_or(Value::Null, |r| json!(r.node)),
    );
    out.insert(
        "results".into(),
        Value::Array(
            results
                .iter()
                .take(count)
                .map(|r| json!({ "node": r.node, "kind": r.kind, "title": r.title, "match": r.matched }))
                .collect(),
        ),
    );
    out.insert("truncated".into(), json!(count < results.len()));
    out.insert("drawer".into(), json!(snap.view.drawer));
    view_state(&mut out, &snap.view);
    Ok(Value::Object(out))
}

/// The handler of `quest_bookmarks`, for the UI area that brings the tab up first.
pub async fn quest_bookmarks(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = tab_doc(&ed, &svc, &args).await?;
    let action = arg(&args, "action").filter(|a| !a.is_empty()).unwrap_or_else(|| "get".to_owned());
    let snap = d.store.snapshot();
    let mut node = arg(&args, "node").unwrap_or_default();
    if node.is_empty()
        && let [one] = snap.view.selection.as_slice()
    {
        node = one.to_string();
    }
    match action.as_str() {
        "add" | "remove" | "toggle" => {
            if node.is_empty() {
                return Err(refused("node is required unless exactly one node is selected"));
            }
            if action != "remove" && snap.graph.find_node(&node).is_none() {
                return Err(refused("node not found"));
            }
            let marked = snap.view.bookmarks.iter().any(|b| *b == node);
            let on = action == "add" || (action == "toggle" && !marked);
            svc.send(&ed, &d, DocOp::View(ViewOp::Bookmark { node, on }), o).await?;
        }
        "next" | "previous" => {
            svc.send(
                &ed,
                &d,
                DocOp::View(ViewOp::BookmarkJump(if action == "previous" { -1 } else { 1 })),
                o,
            )
            .await?;
        }
        "clear" => {
            svc.send(&ed, &d, DocOp::View(ViewOp::BookmarksClear), o).await?;
        }
        "get" => {}
        _ => return Err(refused("action must be get, add, remove, toggle, next, previous or clear")),
    }
    let snap = d.store.snapshot();
    let marks: Vec<&str> = snap
        .view
        .bookmarks
        .iter()
        .filter(|b| snap.graph.find_node(b).is_some())
        .map(|b| b.as_str())
        .collect();
    let mut out = Map::new();
    out.insert("path".into(), json!(d.path().wire));
    out.insert("bookmarks".into(), json!(marks));
    out.insert("count".into(), json!(marks.len()));
    view_state(&mut out, &snap.view);
    Ok(Value::Object(out))
}

fn history_state(s: &doc::ViewState) -> Value {
    json!({ "center": [r1(s.center[0]), r1(s.center[1])], "zoom_level": s.zoom, "selected": s.selection, "slot": s.slot })
}

/// The handler of `quest_history`, for the UI area that brings the tab up first.
pub async fn quest_history(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = tab_doc(&ed, &svc, &args).await?;
    let action = arg(&args, "action").filter(|a| !a.is_empty()).unwrap_or_else(|| "get".to_owned());
    let op = match action.as_str() {
        "back" => Some(ViewOp::HistoryBack),
        "forward" => Some(ViewOp::HistoryForward),
        "clear" => Some(ViewOp::HistoryClear),
        "get" => None,
        _ => return Err(refused("action must be get, back, forward or clear")),
    };
    let moved = match op {
        Some(op) => svc.send(&ed, &d, DocOp::View(op), o).await?.moved,
        None => false,
    };
    let snap = d.store.snapshot();
    let h = d.store.history();
    let v = &snap.view;
    Ok(json!({
        "path": d.path().wire,
        "moved": moved,
        "back_count": v.back.len(),
        "forward_count": v.forward.len(),
        "back": v.back.iter().map(history_state).collect::<Vec<_>>(),
        "forward": v.forward.iter().map(history_state).collect::<Vec<_>>(),
        "current": history_state(&v.current()),
        // the document's own history, newest first: what quest_undo / quest_redo take
        "undo_steps": h.undo,
        "redo_steps": h.redo,
    }))
}

/// The handler of `quest_minimap`, for the UI area that brings the tab up first.
pub async fn quest_minimap(ed: Arc<Editor>, args: Args, o: Origin) -> Reply {
    let svc = service(&ed)?;
    let d = tab_doc(&ed, &svc, &args).await?;
    let action = arg(&args, "action").filter(|a| !a.is_empty()).unwrap_or_else(|| "get".to_owned());
    let visible = d.store.snapshot().view.minimap;
    let set = match action.as_str() {
        "show" => Some(true),
        "hide" => Some(false),
        "toggle" => Some(!visible),
        "get" => None,
        _ => return Err(refused("action must be get, show, hide or toggle")),
    };
    if let Some(on) = set {
        svc.send(&ed, &d, DocOp::View(ViewOp::Minimap(on)), o).await?;
    }
    let catalog = svc.catalog(&ed).await;
    let snap = d.store.snapshot();
    let v = &snap.view;
    let bounds = |b: [f32; 4]| json!([r1(b[0]), r1(b[1]), r1(b[2]), r1(b[3])]);
    let viewport = v.canvas.filter(|c| c[0] > 0.0 && c[1] > 0.0).map(|c| {
        let s = zoom_of(v.zoom) * v.dpi;
        [
            v.center[0] - c[0] / (2.0 * s),
            v.center[1] - c[1] / (2.0 * s),
            v.center[0] + c[0] / (2.0 * s),
            v.center[1] + c[1] / (2.0 * s),
        ]
    });
    let mut out = Map::new();
    out.insert("path".into(), json!(d.path().wire));
    out.insert("visible".into(), json!(v.minimap));
    // what the last frame drew: the toggle asks, a small graph or a small canvas shows none
    out.insert("shown".into(), json!(v.minimap_shown));
    out.insert("min_nodes".into(), json!(doc::MINIMAP_MIN_NODES));
    out.insert("available".into(), json!(snap.graph.nodes.len() >= doc::MINIMAP_MIN_NODES));
    out.insert(
        "graph_bounds".into(),
        layout::graph_bounds(&catalog, &snap.graph).map_or(Value::Null, bounds),
    );
    out.insert("viewport_bounds".into(), viewport.map_or(Value::Null, bounds));
    view_state(&mut out, v);
    Ok(Value::Object(out))
}

/// Every `quest_*` command of the bridge except `quest_view`, the graph tab's own.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "quest_catalog", quest_catalog);
    reg.add(AREA, "quest_list", quest_list);
    reg.add(AREA, "quest_new", quest_new);
    reg.add(AREA, "quest_open", quest_open);
    reg.add(AREA, "quest_close", quest_close);
    reg.add(AREA, "quest_get", quest_get);
    reg.add(AREA, "quest_write", quest_write);
    reg.add(AREA, "quest_undo", quest_undo);
    reg.add(AREA, "quest_redo", quest_redo);
    reg.add(AREA, "quest_apply", quest_apply);
    reg.add(AREA, "quest_validate", quest_validate);
    reg.add(AREA, "quest_save", quest_save);
    reg.add(AREA, "quest_reload", quest_reload);
    reg.add(AREA, "quest_layout", quest_layout);
    reg.add(AREA, "quest_lookup", quest_lookup);
    reg.add(AREA, "quest_check_all", quest_check_all);
    reg.add(AREA, "quest_project_find", quest_project_find);
    reg.add(AREA, "quest_references", quest_references);
    reg.add(AREA, "quest_rename_task", quest_rename_task);
    reg.add(AREA, "quest_edit", quest_edit);
    reg.add(AREA, "quest_find", quest_find);
    reg.add(AREA, "quest_bookmarks", quest_bookmarks);
    reg.add(AREA, "quest_history", quest_history);
    reg.add(AREA, "quest_minimap", quest_minimap);
}

/// What the area hangs on the editor: the `content_create` templates of a quest and a
/// behaviour graph, open documents following a rename, the build's check of the graphs, and the
/// unsaved documents.
pub fn install(editor: &Editor) {
    let templates = AssetTemplates::of(editor);
    templates.provide(TemplateKind::Quest, |cx| {
        Ok(asset::quest_template(&text::id_from_name(cx.stem)).into_bytes())
    });
    templates.provide(TemplateKind::Behaviour, |cx| {
        Ok(asset::behaviour_template(&text::id_from_name(cx.stem)).into_bytes())
    });
    ProjectHooks::of(editor).on_renamed(|ed, from, to| {
        if let Some(session) = ed.project_opt()
            && let Some(svc) = NqService::get(&session)
        {
            svc.retarget(from, to);
        }
    });
    // the graphs die with the project, the work in them does not (`NqDocs::SaveDirty` before
    // the tabs go): whatever is still unsaved when nothing settled it is written first
    ProjectHooks::of(editor).on_before_close(|ed, session| async move {
        if let Some(svc) = NqService::get(&session) {
            let saved = svc.save_dirty(&ed).await;
            if saved > 0 {
                ed.log.info(format!("* [nq] {saved} unsaved graph(s) saved before leaving the project"));
            }
        }
    });
    gz_build::BuildChecks::of(editor).add(Arc::new(check::GraphCheck));
    editor.add_dirty_source(Arc::new(NqDirty));
}
