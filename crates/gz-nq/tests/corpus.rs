//! Every quest and behaviour graph of the repository and the projects, read into the typed model
//! and written as `NqLua::Write` writes it. A file the editor wrote (its generated header first)
//! comes back byte for byte; any other file (the hand-written examples) reaches the canonical text
//! in one write, which a second write keeps.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_corpus::{check_all, finish, walk_ext};
use gz_nq::Catalog;
use gz_nq::asset::{parse, write};
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

fn check(catalog: &Catalog, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let chunk = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let graph = parse(catalog, &text, &chunk).map_err(|e| format!("{e:?}"))?;
    let once = write(catalog, &graph);
    // the editor's header: the file line, then the outline's first line
    let mut head = text.lines();
    let editor_wrote = matches!(head.next(), Some(l) if l.starts_with("-- nq: ") || l.starts_with("-- beh: "))
        && matches!(head.next(), Some(l) if l.starts_with("-- quest ") || l.starts_with("-- behaviour "));
    if editor_wrote && once != text {
        let line = once.lines().zip(text.lines()).position(|(a, b)| a != b).unwrap_or(0);
        return Err(format!(
            "line {}: wrote {:?}, read {:?}",
            line + 1,
            once.lines().nth(line).unwrap_or(""),
            text.lines().nth(line).unwrap_or("")
        ));
    }
    let again =
        parse(catalog, &once, &chunk).map_err(|e| format!("the written text does not read: {e:?}"))?;
    if write(catalog, &again) != once {
        return Err("a second write changes the text".into());
    }
    Ok(())
}

#[test]
fn every_graph_reads_and_writes_back() {
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
        assert!(cat.loaded(), "no kind catalog");
        failed.extend(check_all(files, |p| p.display().to_string(), |p| check(&cat, p)));
        checked += files.len();
    }
    finish("graphs", checked, started, &failed);
}
