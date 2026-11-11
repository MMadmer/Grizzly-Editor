//! Every declarative asset there is - quests, behaviour graphs, characters, PDA layouts - from the
//! repository's examples and fixtures and from the projects: each parses, and the writer's text
//! parses to the same value. Byte equality is the typed models' (`gz-nq`, `gz-characters`): the
//! order of a file's keys is theirs, a bare value keeps the reader's.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_corpus::{check_all, finish, walk_ext};
use gz_luadata::{Value, parse, write_asset, write_value};

const EXTS: [&str; 4] = ["nqasset", "behasset", "npcasset", "pdaasset"];

/// The comment header and the table: what `NqLua::Write` is given.
fn split_head(text: &str) -> Option<(&str, usize)> {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("return {") {
            return Some((&text[..at], at));
        }
        at += line.len();
    }
    None
}

/// Parses, writes, parses again; `Ok(true)` when the whole file is what the writer writes.
fn check(path: &Path, rewritten: &AtomicUsize) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "not UTF-8".to_owned())?;
    let value = parse(text).map_err(|e| format!("line {}: {e}", e.line))?;
    let again = parse(&format!("return {}", write_value(&value, 0)))
        .map_err(|e| format!("the written value does not parse: {e}"))?;
    if again != value {
        return Err("the written value parses to another value".into());
    }
    let Value::Table(top) = &value else { return Err("the asset is no table".into()) };
    let Some((head, _)) = split_head(text) else { return Ok(false) };
    let whole = write_asset(head, top);
    let back = parse(&whole).map_err(|e| format!("the written asset does not parse: {e}"))?;
    if back != value {
        return Err("the written asset parses to another value".into());
    }
    let same = whole.as_bytes() == bytes;
    if same {
        rewritten.fetch_add(1, Ordering::Relaxed);
    }
    Ok(same)
}

#[test]
fn every_asset_parses_and_writes_back() {
    let started = Instant::now();
    let repo = gz_corpus::repo();
    let mut files: Vec<PathBuf> = walk_ext(&repo.join("docs").join("nq").join("examples"), &EXTS);
    files.extend(walk_ext(&repo.join("tools").join("qa").join("fixtures"), &EXTS));
    files.extend(walk_ext(&repo.join("gamedata"), &EXTS));
    for p in gz_corpus::projects().unwrap_or_default() {
        files.extend(walk_ext(&p, &EXTS));
    }
    let rewritten = AtomicUsize::new(0);
    let failed = check_all(&files, |p| p.display().to_string(), |p| check(p, &rewritten).map(|_| ()));
    finish("declarative assets", files.len(), started, &failed);
}
