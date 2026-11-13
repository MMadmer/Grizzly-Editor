//! Every quest and behaviour graph of the repository and the projects, saved the way the graph
//! tab saves: a model that did not change writes nothing (the file stays byte for byte), and one
//! changed value changes the lines of that value alone.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_corpus::{check_all, finish, walk_ext};
use gz_luadata::{Limits, LuaDoc};
use gz_nq::Catalog;
use gz_nq::asset::{AutoPos, diff_rules, graph_from_value, model_value};
use gz_nq::catalog::SourceFile;

/// The kind catalog the files were written with: the linked game's (or the bundled copy), then a
/// project's own kinds.
fn catalog(project: Option<&Path>) -> Catalog {
    let mut files = Vec::new();
    let core = gz_corpus::game()
        .and_then(Result::ok)
        .and_then(|g| g.read_path("configs/nq/catalog.ltx").ok())
        .or_else(|| std::fs::read(gz_corpus::repo().join("gamedata/configs/nq/catalog.ltx")).ok())
        .unwrap_or_default();
    files.push(SourceFile {
        label: "catalog.ltx".into(),
        text: gz_base::text::decode_guess(&core).into_owned(),
    });
    if let Some(p) = project {
        for k in walk_ext(&p.join("gamedata").join("configs").join("nq").join("kinds"), &["ltx"]) {
            let text =
                std::fs::read(&k).map(|b| gz_base::text::decode_guess(&b).into_owned()).unwrap_or_default();
            files.push(SourceFile { label: k.display().to_string(), text });
        }
    }
    Catalog::assemble(&files, true, None, 1)
}

fn changed_lines(a: &str, b: &str) -> usize {
    let (x, y): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let head = x.iter().zip(&y).take_while(|(p, q)| p == q).count();
    let tail = x.iter().rev().zip(y.iter().rev()).take_while(|(p, q)| p == q).count();
    x.len().max(y.len()).saturating_sub(head + tail)
}

fn check(catalog: &Catalog, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let chunk = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut doc = LuaDoc::parse_with(&text, &chunk, Limits::asset()).map_err(|e| format!("{e:?}"))?;
    let (graph, shape) = graph_from_value(catalog, &doc.value());
    if !shape.is_empty() {
        return Err(format!("the model does not read: {}", shape.join("; ")));
    }
    let base = model_value(catalog, &graph, &AutoPos::default());
    let rules = diff_rules();
    if doc.apply_diff(&base, &base.clone(), &rules).map_err(|e| e.message)? || doc.text() != text {
        return Err("an unchanged model wrote something".into());
    }
    let Some(first) = graph.nodes.front().map(|n| n.id.clone()) else { return Ok(()) };
    let mut edited = graph.clone();
    if let Some(n) = edited.nodes.front_mut() {
        n.comment = "saved by the corpus test".into();
    }
    let new = model_value(catalog, &edited, &AutoPos::default());
    doc.apply_diff(&base, &new, &rules).map_err(|e| e.message)?;
    let n = changed_lines(&text, doc.text());
    if n == 0 || n > 3 {
        return Err(format!("one changed comment changed {n} lines"));
    }
    let back = LuaDoc::parse_with(doc.text(), &chunk, Limits::asset())
        .map_err(|e| format!("the saved text does not read: {e:?}"))?;
    let (again, _) = graph_from_value(catalog, &back.value());
    let node = again.nodes.iter().find(|x| x.id == first).ok_or("the node went")?;
    if node.comment != "saved by the corpus test" || again.nodes.len() != graph.nodes.len() {
        return Err("the saved text says something else than the model".into());
    }
    Ok(())
}

#[test]
fn every_graph_saves_only_what_changed() {
    let started = Instant::now();
    let repo = gz_corpus::repo();
    let exts = ["nqasset", "behasset"];
    let mut sets: Vec<(Option<PathBuf>, Vec<PathBuf>)> = vec![(None, {
        let mut f = walk_ext(&repo.join("docs").join("nq").join("examples"), &exts);
        f.extend(walk_ext(&repo.join("tools").join("qa").join("fixtures"), &exts));
        f
    })];
    for p in gz_corpus::projects().unwrap_or_default() {
        let files = walk_ext(&p, &exts);
        sets.push((Some(p), files));
    }
    let mut failed = Vec::new();
    let mut checked = 0;
    for (project, files) in &sets {
        let cat = catalog(project.as_deref());
        checked += files.len();
        failed.extend(check_all(files, |p| p.display().to_string(), |p| check(&cat, p)));
    }
    finish("graphs", checked, started, &failed);
}
