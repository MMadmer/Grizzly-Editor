//! The io half of the graph documents. A document is read through the one read path of the
//! assets (`gz-asset`): it keeps the file's lossless document and the model's value as it was
//! read, and a save writes only the difference into the author's text - comments, layout and
//! keys the editor does not know stay where they are. It follows its file through the
//! project's watcher: an outside change reloads a document without unsaved edits, and rebases
//! the edits of one that has them; a file that stops reading leaves the last good state up and
//! says why until it reads again.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use gz_asset::{AssetFile, Kinds};
use gz_base::{Origin, Problem, RelPath};
use gz_editor::Editor;
use gz_luadata::{Conflict, LuaDoc, Value, set_at};
use gz_project::{ChangeKind, Filter};
use rustc_hash::FxHashSet;
use smol_str::SmolStr;

use crate::asset::{self, AutoPos, Graph};
use crate::catalog::Catalog;
use crate::doc::DocOp;
use crate::layout;
use crate::service::{DocHandle, DocPath, NqService};

/// What a document's file says that the document does not show.
#[derive(Clone, Debug, Default)]
pub struct DiskState {
    /// The file on disk does not read: the reader's sentence with its line and column. The
    /// document keeps its last good state.
    pub error: Option<String>,
    /// The file is gone.
    pub missing: bool,
    /// Values both the file and the unsaved edits changed: the document holds its own.
    pub conflicts: Vec<Conflict>,
    /// Moves with every change of this state.
    pub rev: u64,
}

/// The file as it is on disk and the model as it was read from it.
pub(crate) struct OnDisk {
    pub asset: AssetFile,
    /// The model's value of the file (no position the layout chose).
    pub base: Value,
    /// The positions the layout gave nodes the file has none for.
    pub auto: AutoPos,
}

/// A problem as one sentence, with its place.
fn sentence(p: &Problem) -> String {
    match (p.line, p.column) {
        (Some(l), Some(c)) => format!("{} (line {l}:{c})", p.message),
        (Some(l), None) => format!("{} (line {l})", p.message),
        _ => p.message.clone(),
    }
}

/// The positions of the nodes `unplaced` names, as the model holds them now.
fn auto_of(graph: &Graph, unplaced: &[SmolStr]) -> AutoPos {
    graph.nodes.iter().filter(|n| unplaced.contains(&n.id)).map(|n| (n.id.clone(), n.pos)).collect()
}

/// The model's value for a save: a node the file does not hold yet goes right after the node
/// that links to it (else last), so the file grows where the graph grew.
pub(crate) fn save_value(catalog: &Catalog, graph: &Graph, auto: &AutoPos, base: &Value) -> Value {
    let mut v = asset::model_value(catalog, graph, auto);
    let known: FxHashSet<String> = base
        .get("nodes")
        .map(|n| {
            n.arr().iter().filter_map(|x| x.get("id").and_then(Value::as_str).map(str::to_owned)).collect()
        })
        .unwrap_or_default();
    let Some(Value::Table(nodes)) = gz_luadata::path::get_at_mut(&mut v, &[gz_luadata::Seg::key("nodes")])
    else {
        return v;
    };
    let id = |n: &Value| n.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
    let links =
        |from: &str, to: &str| graph.nodes.iter().find(|n| n.id == from).is_some_and(|n| n.has_target(to));
    let all = std::mem::take(&mut nodes.arr);
    let (old, fresh): (Vec<Value>, Vec<Value>) = all.into_iter().partition(|n| known.contains(&id(n)));
    let mut order = old;
    let mut added: FxHashSet<String> = FxHashSet::default();
    for n in fresh {
        let nid = id(&n);
        let linker = order.iter().position(|x| links(&id(x), &nid));
        match linker {
            Some(i) => {
                let lid = id(&order[i]);
                let mut j = i + 1;
                while j < order.len() && added.contains(&id(&order[j])) && links(&lid, &id(&order[j])) {
                    j += 1;
                }
                order.insert(j, n);
            }
            None => order.push(n),
        }
        added.insert(nid);
    }
    nodes.arr = order;
    v
}

impl NqService {
    /// Reads a document's file: the read file, the graph, the reader's shape problems.
    pub(crate) async fn read_file(
        &self,
        editor: &Editor,
        p: &DocPath,
    ) -> Result<(AssetFile, Graph, Vec<String>), String> {
        let ops = Kinds::of(editor)
            .of_file(&p.wire)
            .ok_or_else(|| format!("{} is no quest or behaviour graph", p.wire))?;
        let asset = if p.inside {
            let rel = RelPath::new(&p.wire).map_err(|e| e.to_string())?;
            gz_asset::read(editor, &rel).await.map_err(|f| sentence(&f.problem))?
        } else {
            let abs = p.abs.clone();
            let text = self
                .exec
                .io(move || std::fs::read(&abs))
                .await
                .map_err(|_| format!("file does not exist: {}", p.wire))?;
            let text = String::from_utf8(text).map_err(|_| format!("{} is not UTF-8 text", p.wire))?;
            let name =
                p.abs.file_name().map_or_else(|| "graph".to_owned(), |n| n.to_string_lossy().into_owned());
            let rel = RelPath::new(&name).map_err(|e| e.to_string())?;
            let others = gz_asset::other_headers(editor, &ops);
            gz_asset::read_text(&ops, rel, p.abs.clone(), &text, &others).map_err(|f| sentence(&f.problem))?
        };
        let catalog = self.catalog(editor).await;
        let (mut graph, shape) = asset::graph_from_value(&catalog, &asset.doc.value());
        if asset.headerless {
            // the header the next save writes
            graph.nq = i32::try_from(ops.version).unwrap_or(1);
        }
        Ok((asset, graph, shape))
    }

    /// Loads a graph read from its file into a document: the document shows it, and the file,
    /// the model's value of it and the positions the layout chose are kept for the next save.
    async fn load_into(
        &self,
        editor: &Editor,
        doc: &DocHandle,
        asset: AssetFile,
        graph: Graph,
        shape: Vec<String>,
    ) -> Result<(), String> {
        let catalog = self.catalog(editor).await;
        let unplaced = asset::unplaced(&graph);
        let base = asset::model_value(&catalog, &graph, &AutoPos::default());
        let time = file_time(&asset.abs);
        self.send(editor, doc, DocOp::Load { graph, shape, file_time: time }, Origin::Mcp)
            .await
            .map_err(|e| e.message)?;
        let auto = auto_of(&doc.store.snapshot().graph, &unplaced);
        *doc.on_disk.lock() = Some(OnDisk { asset, base, auto });
        doc.set_disk(|d| {
            d.error = None;
            d.missing = false;
            d.conflicts.clear();
        });
        Ok(())
    }

    /// Opens a document, or returns the one already open. A missing file is an error: a new
    /// one is a file like any other (`file.create`, or written by hand).
    pub async fn open(&self, editor: &Arc<Editor>, path: &str) -> Result<Arc<DocHandle>, String> {
        let p = self.resolve(path, false)?;
        if let Some(d) = self.find_doc(&p.key) {
            return Ok(d);
        }
        let abs = p.abs.clone();
        if !self.exec.io(move || abs.is_file()).await {
            return Err(format!("file does not exist: {}", p.wire));
        }
        let (asset, graph, shape) = self.read_file(editor, &p).await?;
        let handle = Arc::new(DocHandle::new(&self.exec, p.clone()));
        self.load_into(editor, &handle, asset, graph, shape).await?;
        let handle = {
            let mut docs = self.docs.lock();
            if let Some(d) = docs.iter().find(|d| d.path().key == p.key).cloned() {
                return Ok(d);
            }
            docs.push(handle.clone());
            handle
        };
        self.docs_rev.bump();
        self.follow(editor, &handle);
        Ok(handle)
    }

    /// Reads the file of an open document again; its edits and history go.
    pub async fn reload(&self, editor: &Editor, doc: &DocHandle) -> Result<(), String> {
        let _io = doc.io.lock().await;
        let p = doc.path();
        let abs = p.abs.clone();
        if !self.exec.io(move || abs.is_file()).await {
            return Err("file does not exist".into());
        }
        let (asset, graph, shape) = self.read_file(editor, &p).await?;
        self.load_into(editor, doc, asset, graph, shape).await
    }

    /// Writes the document: the difference between the model as it was read and as it is now,
    /// into the file's own text. A change of the file the watcher has not delivered yet is
    /// taken in first, so nothing of it is written over.
    pub async fn save(&self, editor: &Editor, doc: &DocHandle) -> Result<(), String> {
        let p = doc.path();
        if !p.inside {
            return Err("the document is outside the project folder - it cannot be saved from here".into());
        }
        let _io = doc.io.lock().await;
        let abs = p.abs.clone();
        let now = self.exec.io(move || std::fs::read(abs)).await.ok().and_then(|b| String::from_utf8(b).ok());
        if let Some(text) = now {
            let stale = doc.on_disk.lock().as_ref().is_some_and(|o| o.asset.disk_text() != text);
            if stale {
                self.take_text(editor, doc, &text).await;
            }
        }
        let catalog = self.catalog(editor).await;
        let snap = doc.store.snapshot();
        let Some(mut on) = doc.on_disk.lock().take() else {
            return Err("the document was never read".into());
        };
        let new = save_value(&catalog, &snap.graph, &on.auto, &on.base);
        let kept = on.asset.clone();
        let written = match on.asset.apply(&on.base, &new) {
            Ok(_) => gz_asset::write(editor, &mut on.asset).await,
            Err(e) => Err(e),
        };
        match written {
            Ok(_) => {
                on.base = new;
                let time = file_time(&on.asset.abs);
                *doc.on_disk.lock() = Some(on);
                doc.set_disk(|d| {
                    d.conflicts.clear();
                    d.missing = false;
                });
                self.send(editor, doc, DocOp::Saved { file_time: time }, Origin::Mcp)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.message)
            }
            Err(e) => {
                on.asset = kept;
                *doc.on_disk.lock() = Some(on);
                Err(e.message)
            }
        }
    }

    /// Every dirty document written before the project closes, each one logged. Answers how
    /// many were written.
    pub async fn save_dirty(&self, editor: &Editor) -> usize {
        let mut saved = 0;
        for d in self.docs().into_iter().filter(|d| d.is_dirty()) {
            let path = d.path().wire;
            match self.save(editor, &d).await {
                Ok(()) => {
                    saved += 1;
                    editor.log.info(format!("* [nq] unsaved graph written before it was closed: {path}"));
                }
                Err(e) => editor
                    .log
                    .warn(format!("! [nq] {path} could NOT be saved ({e}) - the unsaved changes are lost")),
            }
        }
        saved
    }

    /// A graph of a path: the open document's, else read from the disk.
    pub async fn read_graph(&self, editor: &Editor, path: &str) -> Result<(Arc<Graph>, DocPath), String> {
        let p = self.resolve(path, false)?;
        if let Some(d) = self.find_doc(&p.key) {
            return Ok((Arc::new(d.store.snapshot().graph.clone()), p));
        }
        let (_, graph, shape) = self.read_file(editor, &p).await?;
        if !shape.is_empty() {
            return Err(shape.join("\n"));
        }
        Ok((Arc::new(graph), p))
    }

    /// Follows the document's file: every change of it from outside the editor reaches
    /// [`Self::on_disk_change`]. A rename follows it anew; a closed document ends it.
    pub(crate) fn follow(&self, editor: &Arc<Editor>, doc: &Arc<DocHandle>) {
        let p = doc.path();
        if !p.inside {
            return;
        }
        let Ok(rel) = RelPath::new(&p.wire) else { return };
        let mut rx = self.project.tree.changes().subscribe(Filter::Path(rel));
        let generation = doc.follow.fetch_add(1, Ordering::AcqRel) + 1;
        let (ed, weak) = (editor.clone(), Arc::downgrade(doc));
        editor.exec.spawn(async move {
            while let Some(c) = rx.recv().await {
                let Some(doc) = weak.upgrade() else { break };
                if doc.follow.load(Ordering::Acquire) != generation {
                    break;
                }
                let Ok(svc) = NqService::of(&ed) else { break };
                if svc.find_doc(&doc.path().key).is_none_or(|d| !Arc::ptr_eq(&d, &doc)) {
                    break;
                }
                svc.on_disk_change(&ed, &doc, c.kind).await;
                ed.exec.wake().ping();
            }
        });
    }

    /// The file of an open document changed outside the editor.
    pub(crate) async fn on_disk_change(&self, editor: &Editor, doc: &DocHandle, kind: ChangeKind) {
        let _io = doc.io.lock().await;
        let abs = doc.path().abs;
        let read = self.exec.io(move || std::fs::read(abs)).await;
        let Ok(bytes) = read else {
            if kind == ChangeKind::Removed || kind == ChangeKind::Rescan {
                doc.set_disk(|d| d.missing = true);
            }
            return;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            doc.set_disk(|d| d.error = Some("the file is no longer UTF-8 text".into()));
            return;
        };
        let same = doc.on_disk.lock().as_ref().is_some_and(|o| o.asset.disk_text() == text);
        if same {
            doc.set_disk(|d| {
                d.missing = false;
                d.error = None;
            });
            return;
        }
        self.take_text(editor, doc, &text).await;
    }

    /// Takes in the text the file holds now: a document without unsaved edits shows it; one
    /// with them keeps them, applied to it, and lists what both sides changed.
    async fn take_text(&self, editor: &Editor, doc: &DocHandle, text: &str) {
        let p = doc.path();
        let Some(ops) = Kinds::of(editor).of_file(&p.wire) else { return };
        let Ok(rel) = RelPath::new(&p.wire) else { return };
        let others = gz_asset::other_headers(editor, &ops);
        let theirs = match gz_asset::read_text(&ops, rel, p.abs.clone(), text, &others) {
            Ok(f) => f,
            Err(f) => {
                let why = sentence(&f.problem);
                doc.set_disk(|d| d.error = Some(why));
                return;
            }
        };
        let catalog = self.catalog(editor).await;
        let (mut tgraph, tshape) = asset::graph_from_value(&catalog, &theirs.doc.value());
        if theirs.headerless {
            tgraph.nq = i32::try_from(ops.version).unwrap_or(1);
        }
        if !doc.is_dirty() {
            if let Err(e) = self.load_into(editor, doc, theirs, tgraph, tshape).await {
                doc.set_disk(|d| d.error = Some(e));
            }
            return;
        }
        let Some(mut on) = doc.on_disk.lock().take() else { return };
        let snap = doc.store.snapshot();
        let mine = save_value(&catalog, &snap.graph, &on.auto, &on.base);
        let rules = asset::diff_rules();
        let name = asset::base_name(&p.wire);
        match LuaDoc::rebase_with(&on.base, &mine, text, &rules, &name, ops.limits) {
            Ok((merged, conflicts)) => {
                let (graph, auto) = place_like(&catalog, &merged.value(), &snap.graph);
                on.base = asset::model_value(&catalog, &tgraph, &AutoPos::default());
                on.asset = theirs;
                on.auto = auto;
                *doc.on_disk.lock() = Some(on);
                let _ = self.send(editor, doc, DocOp::Replace(graph), Origin::Mcp).await;
                doc.set_disk(|d| {
                    d.error = None;
                    d.missing = false;
                    d.conflicts = conflicts;
                });
            }
            Err(e) => {
                *doc.on_disk.lock() = Some(on);
                let why = if e.line > 0 {
                    format!("{} (line {}:{})", e.message, e.line, e.column)
                } else {
                    e.message
                };
                doc.set_disk(|d| d.error = Some(why));
            }
        }
    }

    /// Takes the file's value of one conflict: the document gives up its own for it.
    pub async fn take_theirs(
        &self,
        editor: &Editor,
        doc: &DocHandle,
        index: Option<usize>,
    ) -> Result<(), String> {
        let _io = doc.io.lock().await;
        let all = doc.disk().conflicts;
        let chosen = match index {
            Some(i) => vec![all.get(i).cloned().ok_or_else(|| "no such conflict".to_owned())?],
            None if all.is_empty() => return Err("no value changed on disk too".to_owned()),
            None => all,
        };
        let catalog = self.catalog(editor).await;
        let snap = doc.store.snapshot();
        let (mine, auto) = {
            let on = doc.on_disk.lock();
            let on = on.as_ref().ok_or_else(|| "the document was never read".to_owned())?;
            (save_value(&catalog, &snap.graph, &on.auto, &on.base), on.auto.clone())
        };
        let mut mine = mine;
        for c in &chosen {
            set_at(&mut mine, &c.path, c.theirs.clone().unwrap_or(Value::Nil));
        }
        let (graph, fresh) = place_like(&catalog, &mine, &snap.graph);
        if let Some(on) = doc.on_disk.lock().as_mut() {
            on.auto = auto.into_iter().chain(fresh).collect();
        }
        self.send(editor, doc, DocOp::Replace(graph), Origin::Mcp).await.map_err(|e| e.message)?;
        doc.set_disk(|d| match index {
            Some(i) if i < d.conflicts.len() => {
                d.conflicts.remove(i);
            }
            Some(_) => {}
            None => d.conflicts.clear(),
        });
        Ok(())
    }
}

/// The graph of a value, its nodes without a position placed where the document shows them
/// (else by the layout), with the positions so chosen.
fn place_like(catalog: &Catalog, v: &Value, shown: &Graph) -> (Graph, AutoPos) {
    let (mut g, _) = asset::graph_from_value(catalog, v);
    let unplaced = asset::unplaced(&g);
    for n in g.nodes.iter_mut().filter(|n| !n.has_pos) {
        if let Some(s) = shown.nodes.iter().find(|s| s.id == n.id) {
            n.pos = s.pos;
            n.has_pos = true;
        }
    }
    layout::ensure_positions(catalog, &mut g);
    let auto = auto_of(&g, &unplaced);
    (g, auto)
}

fn file_time(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}
