//! The game's UI XML and string tables (cp1251 and UTF-8): every file reads with its includes to
//! a document with a root, and the string tables of each language build.

use std::time::Instant;

use gz_corpus::{check_all, finish};
use gz_fmt_xml::strings::{Mode, StringTable, is_table_file, order};
use gz_fmt_xml::{Document, decode, preprocess};

#[test]
fn every_xml_of_the_game_parses() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let entries: Vec<_> =
        game.with_ext(&["xml"]).into_iter().filter(|e| e.key.starts_with("configs/")).collect();
    let warned = std::sync::atomic::AtomicUsize::new(0);
    let failed = check_all(
        &entries,
        |e| e.key.clone(),
        |e| {
            let bytes = game.read(e).map_err(|e| e.to_string())?;
            let text = decode(&bytes);
            // `#include "ui\..."` names a path below configs, as `CXml::Load` resolves it
            let include =
                |name: &str| game.read_path(&format!("configs/{name}")).ok().map(|b| decode(&b).into_owned());
            let (text, mut warnings) = preprocess(&text, &include);
            let doc = Document::parse(&text);
            warnings.extend(doc.warnings().iter().cloned());
            // a file without a root holds no markup: a fragment another file includes (an item
            // list of a character description) or a stub of comments
            if doc.root().is_none() && text.split('<').skip(1).any(|t| !t.starts_with("!--")) {
                return Err("markup, but no root element".into());
            }
            if !warnings.is_empty() {
                warned.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(())
        },
    );
    println!("{} files read past a defect", warned.load(std::sync::atomic::Ordering::Relaxed));
    finish("game xml", entries.len(), started, &failed);
}

#[test]
fn the_string_tables_build_in_both_modes() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let mut checked = 0;
    let mut failed = Vec::new();
    for lang in ["rus", "eng"] {
        let mut names: Vec<String> = game
            .files()
            .iter()
            .map(|e| e.key.replace('/', "\\"))
            .filter(|k| is_table_file(k, lang))
            .collect();
        if names.is_empty() {
            continue;
        }
        order(&mut names);
        let texts: Vec<(String, String)> = names
            .iter()
            .filter_map(|n| Some((n.clone(), decode(&game.read_path(n).ok()?).into_owned())))
            .collect();
        let files: Vec<(&str, &str)> = texts.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        for mode in [Mode::Editor, Mode::Game] {
            checked += files.len();
            let (table, _) = StringTable::build(&files, mode);
            if table.len() < 1000 {
                failed.push(format!("{lang} {mode:?}: only {} strings", table.len()));
            }
            // a string of the Russian main menu's table (ui_st_mm.xml), cp1251 decoded; Dead
            // Air's English tables are partial
            let master = table.get("gd_master").unwrap_or_default();
            if lang == "rus" && master != "Мастер" {
                failed.push(format!("{lang} {mode:?}: gd_master is {master:?}"));
            }
        }
    }
    finish("string tables", checked, started, &failed);
}
