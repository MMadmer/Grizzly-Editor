//! Quests and behaviour graphs as kinds of `gz-asset`: what `asset.check` and
//! `asset.reference` ask of them, and the keys they know.

use std::fmt::Write as _;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps};
use gz_base::Problem;
use gz_editor::Editor;
use gz_luadata::Seg;
use gz_project::AssetKind;

use crate::asset;
use crate::catalog::Catalog;
use crate::service::NqService;
use crate::validate::{NqProblem, Severity};

/// The format reference: quests, and behaviour graphs in its section 10.
const NQ_FORMAT: &str = include_str!("../../../docs/nq/NQ_FORMAT.md");

const TOP: &[&str] =
    &["nq", "beh", "id", "title", "activation", "roles", "vars", "tasks", "nodes", "editor", "canvas"];
const NODE: &[&str] = &[
    "id",
    "kind",
    "once",
    "params",
    "cond",
    "on_enter",
    "on_exit",
    "out",
    "pos",
    "comment",
    "collapsed",
    "disabled",
];
const TASK: &[&str] = &["title", "descr", "type", "target", "icon", "auto_complete", "objectives"];
const ROLE: &[&str] = &["title", "many", "optional", "default"];
const KNOWN: &[(&str, &[&str])] = &[("", TOP), ("nodes.*", NODE), ("tasks.*", TASK), ("roles.*", ROLE)];

/// The kinds of the graph area.
#[must_use]
pub fn kinds() -> [KindOps; 2] {
    let make = |kind: AssetKind| {
        let mut k = KindOps::of(kind);
        k.known = KNOWN;
        k.reference_doc = if kind == AssetKind::Behaviour {
            "docs/nq/NQ_FORMAT.md section 10"
        } else {
            "docs/nq/NQ_FORMAT.md"
        };
        k.check = check;
        k.diff_rules = asset::diff_rules;
        k
    };
    let take: gz_asset::TakeTheirsFn = |ed, rel, index| Box::pin(take_theirs(ed, rel, index));
    let mut quest = make(AssetKind::Quest);
    quest.take_theirs = Some(take);
    quest.reference = |ed| Box::pin(reference(ed, false));
    let mut beh = make(AssetKind::Behaviour);
    beh.take_theirs = Some(take);
    beh.reference = |ed| Box::pin(reference(ed, true));
    [quest, beh]
}

/// Where a problem of the validator stands in the file: its node, then what its slot names
/// (`param:npc`, `enter:2`, `cond:1`, `out:next`, `task:t/objective:o`).
#[must_use]
pub fn problem_path(node: &str, slot: &str) -> Vec<Seg> {
    let mut p = Vec::new();
    if !node.is_empty() {
        p.push(Seg::key("nodes"));
        p.push(Seg::item("id", node));
    }
    let (head, rest) = slot.split_once(':').unwrap_or((slot, ""));
    let index = |r: &str| r.split(['/', ':']).next().and_then(|n| n.parse::<usize>().ok());
    match head {
        "" => {}
        "param" if node.is_empty() => p.extend(rest.split('/').filter(|s| !s.is_empty()).map(Seg::key)),
        "param" => {
            p.push(Seg::key("params"));
            p.extend(rest.split('/').filter(|s| !s.is_empty()).map(Seg::key));
        }
        "task" => {
            let (task, more) = rest.split_once('/').unwrap_or((rest, ""));
            p.push(Seg::key("tasks"));
            p.push(Seg::key(task));
            if let Some(o) = more.strip_prefix("objective:") {
                p.push(Seg::key("objectives"));
                p.push(Seg::item("id", o));
            }
        }
        "enter" | "exit" | "cond" => {
            p.push(Seg::key(match head {
                "enter" => "on_enter",
                "exit" => "on_exit",
                _ => "cond",
            }));
            if let Some(i) = index(rest) {
                p.push(Seg::Index(i));
            }
        }
        "out" => {
            p.push(Seg::key("out"));
            if !rest.is_empty() {
                p.push(Seg::key(rest));
            }
        }
        other => p.push(Seg::key(other)),
    }
    p
}

/// A validator's problem at its place in the file.
#[must_use]
pub fn positioned(p: &NqProblem, file: &AssetFile) -> Problem {
    let base = match p.severity {
        Severity::Error => Problem::error(p.code.clone(), p.message.clone()),
        Severity::Warning => Problem::warning(p.code.clone(), p.message.clone()),
    };
    let mut base = base;
    if !p.node.is_empty() {
        base = base.about(p.node.as_str());
    }
    if !p.slot.is_empty() {
        base = base.in_field(p.slot.as_str());
    }
    file.place(base, &problem_path(&p.node, &p.slot))
}

/// A shape problem of the reader (`node 'x': params must be a table`) at the node it names.
fn shape_problem(message: &str, file: &AssetFile) -> Problem {
    let path = message
        .strip_prefix("node '")
        .and_then(|r| r.split_once('\''))
        .map(|(id, rest)| {
            let mut p = vec![Seg::key("nodes"), Seg::item("id", id)];
            if let Some(field) = rest.strip_prefix(": ").and_then(|r| r.split([' ', '.', '#', ':']).next()) {
                p.push(Seg::key(field));
            }
            p
        })
        .unwrap_or_default();
    file.place(Problem::error("E003", message), &path)
}

/// Whether the behaviour graph `id` comes back to itself through two or more of the graphs it
/// runs (the direct case is the graph's own check).
fn runs_itself(id: &str, graphs: &[(String, Vec<String>)]) -> bool {
    let Some(start) = graphs.iter().position(|g| g.0 == id) else { return false };
    let mut seen = vec![false; graphs.len()];
    let mut stack: Vec<usize> = Vec::new();
    for c in &graphs[start].1 {
        for (k, other) in graphs.iter().enumerate() {
            if other.0 == *c && k != start && !seen[k] {
                seen[k] = true;
                stack.push(k);
            }
        }
    }
    while let Some(at) = stack.pop() {
        for c in &graphs[at].1 {
            if c == id {
                return true;
            }
            for (k, other) in graphs.iter().enumerate() {
                if other.0 == *c && !seen[k] {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
    }
    false
}

fn check(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let Ok(svc) = NqService::of(&ed) else { return Vec::new() };
        let catalog = svc.catalog(&ed).await;
        let env = svc.env(&ed, false).await;
        let (mut graph, shape) = asset::graph_from_value(&catalog, &file.doc.value());
        if file.headerless {
            graph.nq = i32::try_from(file.ops.version).unwrap_or(1);
        }
        let mut out: Vec<Problem> = shape.iter().map(|m| shape_problem(m, &file)).collect();
        // the header is the read path's (gz-asset)
        out.extend(
            env.validate(&graph, file.path.as_str())
                .iter()
                .filter(|p| p.code != "E001")
                .map(|p| positioned(p, &file)),
        );
        if graph.is_behaviour() {
            let graphs: Vec<(String, Vec<String>)> = env
                .index
                .entries
                .iter()
                .filter(|e| e.readable && e.graph.is_behaviour())
                .map(|e| {
                    let calls = e
                        .graph
                        .nodes
                        .iter()
                        .filter(|n| n.kind == "behaviour.run")
                        .map(|n| n.params.get_string("asset", ""))
                        .filter(|a| !a.is_empty() && !a.contains('.'))
                        .collect();
                    (e.graph.id.clone(), calls)
                })
                .collect();
            if runs_itself(&graph.id, &graphs) {
                out.push(file.place(
                    Problem::error(
                        "E113",
                        format!("behaviour graph '{}' runs itself through the graphs it calls", graph.id),
                    ),
                    &[Seg::key("id")],
                ));
            }
        }
        out
    })
}

/// The node kinds of the linked game's catalog a document of this kind may use, as text.
#[must_use]
pub fn catalog_text(catalog: &Catalog, behaviour: bool) -> String {
    let mut out = String::new();
    let doc = if behaviour { "behaviour graph" } else { "quest" };
    let _ = writeln!(out, "\n## The node kinds of the linked game ({doc}s)\n");
    if catalog.kinds().is_empty() {
        let why = if catalog.error.is_empty() { "no catalog was read" } else { catalog.error.as_str() };
        let _ = writeln!(out, "No kind catalog: {why}.");
        return out;
    }
    let _ = writeln!(
        out,
        "From {} ({} kinds). `params`: name, type, required or the default; `pins`: the outputs `out` names.\n",
        if catalog.source.is_empty() { "the catalog" } else { catalog.source.as_str() },
        catalog.kinds().len()
    );
    let mut group = String::new();
    for k in catalog.kinds().iter().filter(|k| if behaviour { k.in_behaviour } else { k.in_quest }) {
        if k.group != group {
            group.clone_from(&k.group);
            let _ = writeln!(out, "### {group}\n");
        }
        let title = if k.title.is_empty() { String::new() } else { format!(" - {}", k.title) };
        let _ = writeln!(out, "- `{}`{title}", k.id);
        if !k.desc.is_empty() {
            let _ = writeln!(out, "  {}", k.desc.replace('\n', " "));
        }
        let param = |p: &crate::catalog::Param| {
            let mut s = format!("{}: {}", p.name, p.ty);
            if !p.enums.is_empty() {
                s.push_str(&format!(" ({})", p.enums.join("|")));
            }
            if p.required {
                s.push_str(", required");
            } else if let Some(d) = &p.default {
                s.push_str(&format!(", default {d}"));
            }
            if let (Some(a), Some(b)) = (p.min, p.max) {
                s.push_str(&format!(", {a}..{b}"));
            }
            s
        };
        if !k.params.is_empty() {
            let _ = writeln!(out, "  params: {}", k.params.iter().map(param).collect::<Vec<_>>().join("; "));
        }
        if !k.rows.is_empty() {
            let _ = writeln!(out, "  each row: {}", k.rows.iter().map(param).collect::<Vec<_>>().join("; "));
        }
        if k.pins_from_cases {
            let _ = writeln!(out, "  pins: one per entry of params.cases");
        } else if !k.pins.is_empty() {
            let _ = writeln!(out, "  pins: {}", k.pins.join(", "));
        }
        let mut uses = Vec::new();
        if k.in_quest {
            uses.push("quests");
        }
        if k.in_behaviour {
            uses.push("behaviour graphs");
        }
        let _ = writeln!(out, "  used in: {}", uses.join(" and "));
    }
    out
}

async fn take_theirs(ed: Arc<Editor>, rel: gz_base::RelPath, index: Option<usize>) -> Result<usize, String> {
    let svc = NqService::of(&ed).map_err(|e| e.message)?;
    let doc = svc.get_doc(rel.as_str()).ok_or_else(|| format!("{rel} is not open"))?;
    svc.take_theirs(&ed, &doc, index).await?;
    Ok(doc.disk().conflicts.len())
}

async fn reference(ed: Arc<Editor>, behaviour: bool) -> String {
    let mut text = NQ_FORMAT.to_owned();
    if let Ok(svc) = NqService::of(&ed) {
        let catalog = svc.catalog(&ed).await;
        text.push_str(&catalog_text(&catalog, behaviour));
    } else {
        text.push_str("\nOpen a project to see the node kinds of its linked game.\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_luadata::path::show;

    #[test]
    fn slots_name_their_place() {
        let s = |node: &str, slot: &str| show(&problem_path(node, slot));
        assert_eq!(s("talk", "param:npc"), "nodes[id=talk].params.npc");
        assert_eq!(s("talk", "enter:2"), "nodes[id=talk].on_enter[2]");
        assert_eq!(s("talk", "cond:0"), "nodes[id=talk].cond[0]");
        assert_eq!(s("talk", "out:next"), "nodes[id=talk].out.next");
        assert_eq!(s("talk", "comment"), "nodes[id=talk].comment");
        assert_eq!(s("", "param:vars/agreed"), "vars.agreed");
        assert_eq!(s("", "task:kill/objective:o1"), "tasks.kill.objectives[id=o1]");
        assert_eq!(s("", ""), "");
        let graphs = vec![("a".to_owned(), vec!["b".to_owned()]), ("b".to_owned(), vec!["a".to_owned()])];
        assert!(runs_itself("a", &graphs));
        assert!(!runs_itself("a", &[("a".to_owned(), vec!["c".to_owned()])]));
    }
}
