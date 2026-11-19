#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::{Origin, RelPath, Severity};
use gz_editor::{Editor, EditorOptions, Log, ProjectSession, Registry};
use gz_luadata::{LuaDataError, LuaDoc, Seg, Value};
use gz_task::{ExecOptions, Runtime};
use serde_json::{Map, json};

use crate::{KindOps, Kinds, read_text};

/// Version 1 called it `title`.
fn v1_to_v2(doc: &mut LuaDoc) -> Result<(), LuaDataError> {
    if let Some(t) = doc.get(&[Seg::key("title")]) {
        doc.set(&[Seg::key("name")], &t)?;
        doc.remove(&[Seg::key("title")])?;
    }
    Ok(())
}

/// Version 2 counted in tens.
fn v2_to_v3(doc: &mut LuaDoc) -> Result<(), LuaDataError> {
    if let Some(Value::Number(n)) = doc.get(&[Seg::key("count")]) {
        doc.set(&[Seg::key("count")], &Value::from(n * 10.0))?;
    }
    Ok(())
}

const STEPS: &[crate::Migration] = &[v1_to_v2, v2_to_v3];

fn test_kind() -> Arc<KindOps> {
    let mut k = KindOps::of(gz_project::AssetKind::Quest);
    k.name = "test asset";
    k.ext = ".testasset";
    k.header = "test";
    k.version = 3;
    k.migrations = STEPS;
    k.known = &[("", &["id", "name", "count", "list"])];
    k.check = |_, f| {
        Box::pin(async move {
            let mut out = Vec::new();
            if f.doc.get(&[Seg::key("id")]).is_none() {
                out.push(f.place_head(gz_base::Problem::error("T001", "the asset has no id")));
            }
            if let Some(Value::Number(n)) = f.doc.get(&[Seg::key("count")])
                && n < 0.0
            {
                out.push(
                    f.place(gz_base::Problem::warning("T002", "a count below zero"), &[Seg::key("count")]),
                );
            }
            out
        })
    };
    Arc::new(k)
}

fn read(text: &str) -> Result<crate::AssetFile, crate::ReadFailure> {
    read_text(&test_kind(), RelPath::new("a/x.testasset").unwrap(), PathBuf::new(), text, &[("nq", "quest")])
}

#[test]
fn versions_old_current_newer_and_missing() {
    // current: read as it is, nothing to note
    let f = read("return {\n  test = 3,\n  id = \"x\",\n  name = \"N\",\n}\n").unwrap();
    assert_eq!(f.read_version, 3);
    assert!(f.notes.is_empty(), "{:?}", f.notes);
    assert!(!f.changed());
    // older: migrated in memory, step by step, comments kept
    let v1 = "-- the author's comment\nreturn {\n  test = 1,\n  id = \"x\",\n  title = \"N\", -- the old name\n  count = 4,\n}\n";
    let f = read(v1).unwrap();
    assert_eq!(f.read_version, 1);
    assert!(f.changed());
    let v = f.doc.value();
    assert_eq!(v.get_number("test", 0.0), 3.0);
    assert_eq!(v.get_string("name", ""), "N");
    assert!(v.get("title").is_none());
    assert_eq!(v.get_number("count", 0.0), 40.0);
    assert!(f.doc.text().starts_with("-- the author's comment\n"));
    assert!(f.notes.iter().any(|n| n.code == "old_version" && n.severity == Severity::Note));
    let f = read("return { test = 2, id = \"x\", count = 1 }").unwrap();
    assert_eq!(f.doc.value().get_number("count", 0.0), 10.0);
    // newer: refused with the sentence, at the header
    let e = read("return {\n  test = 4,\n  id = \"x\",\n}").err().unwrap();
    assert_eq!(e.code, "newer_version");
    assert!(e.problem.message.contains("made by a newer Grizzly Editor"), "{}", e.problem.message);
    assert_eq!((e.problem.line, e.problem.column), (Some(2), Some(3)));
    // missing: read as the current version, an error for the game, the header added on save
    let mut f = read("return {\n  id = \"x\",\n}\n").unwrap();
    assert!(f.headerless);
    let e = f.notes.iter().find(|n| n.code == "E001").unwrap();
    assert_eq!(e.fix.as_deref(), Some("test = 3"));
    assert_eq!(e.line, Some(1));
    f.ensure_header().unwrap();
    assert_eq!(f.doc.text(), "return {\n  test = 3,\n  id = \"x\",\n}\n");
    // another kind's header in this kind's file
    let f = read("return { nq = 1, id = \"x\" }").unwrap();
    assert!(f.notes.iter().any(|n| n.code == "E001" && n.message.contains("'nq' is the header of a quest")));
    // a key nobody knows: kept, noted at its place
    let f = read("return {\n  test = 3,\n  id = \"x\",\n  extra = { 1 },\n}").unwrap();
    let n = f.notes.iter().find(|n| n.code == "unknown_key").unwrap();
    assert_eq!((n.line, n.column, n.message.as_str()), (Some(4), Some(3), "unknown key 'extra' (kept)"));
    // syntax: the reader's sentence at its place
    let e = read("return {\n  id = ,\n}").err().unwrap();
    assert_eq!((e.code, e.problem.line, e.problem.column), ("syntax", Some(2), Some(8)));
    // a byte order mark: kept, and an error, since the game refuses it
    let f = read("\u{feff}return { test = 3, id = \"x\" }").unwrap();
    assert!(f.notes.iter().any(|n| n.code == "bom" && n.is_error()));
}

#[test]
fn saving_writes_only_the_difference_and_adds_a_missing_header() {
    let text = "-- keep me\nreturn {\n  id = \"x\",\n  name = 'N',\n  unknown = true,\n}\n";
    let mut f = read(text).unwrap();
    let mut model = Value::table();
    model.put("id", "x");
    model.put("name", "N");
    // nothing changed: nothing written, the header not added
    assert!(!f.apply(&model, &model.clone()).unwrap());
    assert!(!f.changed());
    let mut new = model.clone();
    new.set("name", "M");
    assert!(f.apply(&model, &new).unwrap());
    assert_eq!(
        f.doc.text(),
        "-- keep me\nreturn {\n  test = 3,\n  id = \"x\",\n  name = 'M',\n  unknown = true,\n}\n"
    );
}

fn editor(root: PathBuf) -> (Runtime, Arc<Editor>) {
    let rt = Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 1, io_threads: 2 }).unwrap();
    let (log, _guard) = Log::start(None, false).unwrap();
    let mut reg = Registry::new();
    crate::register(&mut reg);
    reg.install_help();
    let ed = Editor::new(EditorOptions::default(), rt.exec(), log, reg);
    ed.set_project(Some(Arc::new(ProjectSession::new(root))));
    Kinds::of(&ed).add((*test_kind()).clone());
    (rt, ed)
}

fn args(v: serde_json::Value) -> Map<String, serde_json::Value> {
    v.as_object().cloned().unwrap()
}

#[test]
fn the_commands_check_and_migrate_a_project() {
    let root = std::env::temp_dir().join(format!("gz-asset-cmd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a")).unwrap();
    let v1 = "-- the author's comment\nreturn {\n  test = 1,\n  id = \"x\",\n  title = \"N\", -- the old name\n  count = -1,\n}\n";
    std::fs::write(root.join("a/old.testasset"), v1).unwrap();
    std::fs::write(root.join("a/new.testasset"), "return {\n  test = 3,\n  name = \"N\",\n}\n").unwrap();
    std::fs::write(root.join("a/future.testasset"), "return { test = 9, id = \"f\" }\n").unwrap();
    std::fs::write(root.join("a/notes.txt"), "not an asset").unwrap();
    let (rt, ed) = editor(root.clone());

    let r = rt.block_on(ed.call("asset.check", args(json!({ "path": "a" })), Origin::Mcp));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["files"], 3);
    let problems = r["problems"].as_array().unwrap();
    let find = |code: &str| problems.iter().find(|p| p["code"] == code).cloned().unwrap_or_default();
    assert_eq!(find("T001")["file"], "a/new.testasset");
    assert_eq!(find("T001")["line"], 1);
    assert_eq!(find("newer_version")["file"], "a/future.testasset");
    assert_eq!(find("old_version")["severity"], "note");
    // the old file's count, -1 in version 1, is -10 in version 3: the check reads the current
    // one, and points into the file as it is
    assert_eq!((find("T002")["line"].clone(), find("T002")["column"].clone()), (json!(6), json!(3)));
    assert!(r["counts"]["error"].as_u64().unwrap() >= 2);
    let failed =
        rt.block_on(ed.call("asset.check", args(json!({ "path": "a", "fail_on": "error" })), Origin::Mcp));
    assert_eq!((failed["ok"].clone(), failed["code"].clone()), (json!(false), json!("failed")), "{failed}");
    assert!(failed["problems"].is_array());

    let dry = rt.block_on(ed.call("asset.migrate", args(json!({ "dry_run": true })), Origin::Mcp));
    assert_eq!(dry["files"], json!([{ "file": "a/old.testasset", "from": 1, "to": 3 }]), "{dry}");
    assert_eq!(dry["refused"][0]["file"], "a/future.testasset");
    assert_eq!(std::fs::read_to_string(root.join("a/old.testasset")).unwrap(), v1);
    let done = rt.block_on(ed.call("asset.migrate", args(json!({})), Origin::Mcp));
    assert_eq!(done["written"], 1, "{done}");
    let text = std::fs::read_to_string(root.join("a/old.testasset")).unwrap();
    assert_eq!(
        text,
        "-- the author's comment\nreturn {\n  test = 3,\n  id = \"x\",\n  count = -10,\n  name = \"N\",\n}\n"
    );
    let again = rt.block_on(ed.call("asset.migrate", args(json!({})), Origin::Mcp));
    assert_eq!(again["written"], 0);

    let k = rt.block_on(ed.call("asset.kinds", Map::new(), Origin::Mcp));
    assert!(
        k["kinds"].as_array().unwrap().iter().any(|x| x["extension"] == ".testasset" && x["version"] == 3)
    );
    let f = rt.block_on(ed.call("asset.find", args(json!({ "refs_to": "x" })), Origin::Mcp));
    assert_eq!(f["results"][0]["file"], "a/old.testasset", "{f}");
    assert_eq!(f["results"][0]["path"], "id");
    let f = rt.block_on(ed.call("asset.find", args(json!({ "query": "AUTHOR" })), Origin::Mcp));
    assert_eq!(f["total"], 1, "{f}");
    let h = rt.block_on(ed.call("help", args(json!({ "about": "assets" })), Origin::Mcp));
    assert!(h["text"].as_str().unwrap().contains("Assets are text files"), "{h}");
    drop(ed);
    drop(rt);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_manifest_answers_its_format_by_any_of_its_names() {
    let root = std::env::temp_dir().join(format!("gz-asset-manifest-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (rt, ed) = editor(root.clone());
    let kinds = rt.block_on(ed.call("asset.kinds", Map::new(), Origin::Mcp));
    let manifest = kinds["kinds"].as_array().unwrap().iter().find(|k| k["extension"] == "mod.ltx").unwrap();
    assert!(manifest["reference"].as_str().unwrap().contains("asset.reference kind=manifest"), "{manifest}");
    for word in ["manifest", "module manifest", "MOD.LTX"] {
        let r = rt.block_on(ed.call("asset.reference", args(json!({ "kind": word })), Origin::Mcp));
        assert_eq!(r["ok"], true, "{word}: {r}");
        assert_eq!(r["kind"], "module manifest");
        let text = r["text"].as_str().unwrap();
        for part in ["[module]", "target = default", "[provides_mode]", "[world]", "Example"] {
            assert!(text.contains(part), "{word}: {part}");
        }
        // with no game linked the page still answers, and says what only a game adds
        let (example, modes, note) = (
            text.find("Example").unwrap(),
            text.find("Modes of the linked game").unwrap(),
            text.find("Left as written").unwrap(),
        );
        assert!(example < modes && modes < note, "the modes come after the example, before the closing note");
        assert!(text.contains("No game is linked"), "{text}");
    }
    let bad = rt.block_on(ed.call("asset.reference", args(json!({ "kind": "nothing" })), Origin::Mcp));
    assert_eq!(bad["ok"], false);
    assert!(bad["error"].as_str().unwrap().contains("module manifest"), "{bad}");
    drop(ed);
    drop(rt);
    let _ = std::fs::remove_dir_all(&root);
}

/// What the test's text check says of every file it reads.
fn seen(_: Arc<Editor>, rel: RelPath) -> futures::future::BoxFuture<'static, Vec<gz_base::Problem>> {
    Box::pin(async move { vec![gz_base::Problem::warning("X001", "seen").in_file(rel).at_line(1)] })
}

#[test]
fn a_text_file_that_is_no_asset_is_checked_by_its_check() {
    let root = std::env::temp_dir().join(format!("gz-asset-text-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["scripts", "ui", "part", "design", "temp"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("mod.ltx"), "[module]\nid = t\n[exclude]\ndesign\n").unwrap();
    std::fs::write(root.join("scripts/a.script"), "function f() end\n").unwrap();
    std::fs::write(root.join("ui/b.XML"), "<w/>\n").unwrap();
    std::fs::write(root.join("notes.txt"), "no check reads it").unwrap();
    // a part of an asset: the folder of the asset's name beside it
    std::fs::write(root.join("part.testasset"), "return {\n  test = 3,\n  id = \"p\",\n}\n").unwrap();
    std::fs::write(root.join("part/objects.script"), "return {}\n").unwrap();
    // left out of the build; the editor's scratch
    std::fs::write(root.join("design/c.script"), "x = 1\n").unwrap();
    std::fs::write(root.join("temp/d.script"), "x = 1\n").unwrap();
    let (rt, ed) = editor(root.clone());
    let kinds = Kinds::of(&ed);
    kinds.add_text_check(&["script", "xml", "ltx", "testasset"], seen);
    // the manifest keeps its own check, a kind's file its kind's
    assert!(kinds.text_check("mod.ltx").is_none() && kinds.text_check("part.testasset").is_none());
    assert!(kinds.text_check("other.ltx").is_some() && kinds.text_check("a.Script").is_some());
    assert!(kinds.text_check("notes.txt").is_none());

    let seen_in = |args_: serde_json::Value| -> Vec<String> {
        let r = rt.block_on(ed.call("asset.check", args(args_), Origin::Mcp));
        assert_eq!(r["ok"], true, "{r}");
        let mut files: Vec<String> = r["problems"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["code"] == "X001")
            .map(|p| p["file"].as_str().unwrap().to_owned())
            .collect();
        files.sort();
        files
    };
    assert_eq!(seen_in(json!({})), ["scripts/a.script", "ui/b.XML"]);
    // a file asked for by itself; a folder the build leaves out, asked for, is looked into
    assert_eq!(seen_in(json!({ "path": "scripts/a.script" })), ["scripts/a.script"]);
    assert_eq!(seen_in(json!({ "path": "design" })), ["design/c.script"]);
    // a part of an asset is checked with its asset, and its folder stands for the asset
    assert!(seen_in(json!({ "path": "part/objects.script" })).is_empty());
    let r = rt.block_on(ed.call("asset.check", args(json!({ "path": "part" })), Origin::Mcp));
    assert_eq!(r["files"], 1, "{r}");
    let other = rt.block_on(ed.call("asset.check", args(json!({ "path": "notes.txt" })), Origin::Mcp));
    assert_eq!(other["ok"], false, "{other}");
    drop(ed);
    drop(rt);
    let _ = std::fs::remove_dir_all(&root);
}
