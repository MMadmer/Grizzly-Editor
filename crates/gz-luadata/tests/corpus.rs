//! Every declarative asset there is - quests, behaviour graphs, characters, PDA layouts - from the
//! repository's examples and fixtures and from the projects: each parses, its document is the
//! file byte for byte and holds the reader's value, and the canonical writer's text of that
//! value parses back to it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_corpus::{check_all, finish, walk_ext};
use gz_luadata::{LuaDoc, Value, parse, write_asset, write_value};

const EXTS: [&str; 4] = ["nqasset", "behasset", "npcasset", "pdaasset"];

fn check(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "not UTF-8".to_owned())?;
    let value = parse(text).map_err(|e| format!("line {}: {e}", e.line))?;
    let doc = LuaDoc::parse(text).map_err(|e| format!("the document refuses it: {e}"))?;
    if doc.text().as_bytes() != bytes {
        return Err("the document is not the file".into());
    }
    if doc.value() != value {
        return Err("the document holds another value than the reader".into());
    }
    let again = parse(&format!("return {}", write_value(&value, 0)))
        .map_err(|e| format!("the written value does not parse: {e}"))?;
    if again != value {
        return Err("the written value parses to another value".into());
    }
    let Value::Table(top) = &value else { return Err("the asset is no table".into()) };
    let back = parse(&write_asset("", top)).map_err(|e| format!("the written asset does not parse: {e}"))?;
    if back != value {
        return Err("the written asset parses to another value".into());
    }
    Ok(())
}

#[test]
fn every_asset_reads_as_a_document() {
    let started = Instant::now();
    let repo = gz_corpus::repo();
    let mut files: Vec<PathBuf> = walk_ext(&repo.join("docs").join("nq").join("examples"), &EXTS);
    files.extend(walk_ext(&repo.join("tools").join("qa").join("fixtures"), &EXTS));
    files.extend(walk_ext(&repo.join("gamedata"), &EXTS));
    for p in gz_corpus::projects().unwrap_or_default() {
        files.extend(walk_ext(&p, &EXTS));
    }
    let failed = check_all(&files, |p| p.display().to_string(), |p| check(p));
    finish("declarative assets", files.len(), started, &failed);
}
