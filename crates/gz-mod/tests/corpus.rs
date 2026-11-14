//! Every `mod.ltx` there is: the projects' and the installed modules' of the game. Each parses,
//! writes back its own bytes, reads as a manifest and passes the manifest's value rules: a shipped
//! module that fails one is a finding, not something to loosen the rule for.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_base::text::{cp1251_to_utf8, utf8_to_cp1251};
use gz_corpus::{check_all, finish};
use gz_mod::{Manifest, ManifestDoc};

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The text the manifest reader takes (UTF-8 when it is, cp1251 otherwise) and the bytes it is
/// written back as.
fn round_trip(bytes: &[u8]) -> Result<(), String> {
    let body = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let (text, utf8) = match std::str::from_utf8(body) {
        Ok(s) => (std::borrow::Cow::Borrowed(s), true),
        Err(_) => (cp1251_to_utf8(body), false),
    };
    let doc = ManifestDoc::parse(&text);
    let written = doc.to_string();
    let out: Vec<u8> = if utf8 { written.into_bytes() } else { utf8_to_cp1251(&written).0.into_owned() };
    if out != body {
        let at = out.iter().zip(body).position(|(a, b)| a != b).unwrap_or(out.len().min(body.len()));
        return Err(format!("differs from byte {at}"));
    }
    let m = Manifest::parse(&doc);
    if m.id.is_empty() {
        return Err("no module id".into());
    }
    // a manifest without a target is the first problem of a new project, not a shipped module's
    let errors: Vec<String> = gz_mod::validate(&doc)
        .iter()
        .filter(|p| p.is_error() && p.code != "mod.target")
        .map(|p| format!("{}:{} {}", p.line.unwrap_or(0), p.code, p.message))
        .collect();
    if !errors.is_empty() {
        return Err(errors.join(" | "));
    }
    // warnings are listed, not failed: a shipped module may keep a key nobody reads
    for p in gz_mod::validate(&doc).iter().filter(|p| !p.is_error() || p.code == "mod.target") {
        println!("not failed {}:{} {}", p.line.unwrap_or(0), p.code, p.message);
    }
    Ok(())
}

fn manifests_below(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> =
        rd.flatten().map(|e| e.path().join("mod.ltx")).filter(|p| p.is_file()).collect();
    out.sort();
    out
}

#[test]
fn every_manifest_writes_back_the_bytes_it_read() {
    let started = Instant::now();
    let mut files: Vec<PathBuf> = Vec::new();
    for p in gz_corpus::projects().unwrap_or_default() {
        files.extend(Some(p.join("mod.ltx")).filter(|f| f.is_file()));
    }
    if let Some(game) = gz_corpus::game_root() {
        files.extend(manifests_below(&game.join("modules")));
        files.extend(manifests_below(&game.join("mods")));
    }
    let failed = check_all(
        &files,
        |p| p.display().to_string(),
        |p| round_trip(&std::fs::read(p).map_err(|e| e.to_string())?),
    );
    finish("mod.ltx", files.len(), started, &failed);
}
