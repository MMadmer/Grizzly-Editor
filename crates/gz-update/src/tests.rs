//! The updater end to end, inside one test: a release served by a small HTTP server of the test's
//! own on 127.0.0.1 (ranges, a redirect to storage, a cut connection), an installation of
//! harmless files in a temp folder, and the `update.*` commands that check, download and apply.

#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gz_editor::{Editor, EditorOptions, Flags, Log, Origin, Registry, Value, json};
use gz_task::{ExecOptions, Runtime};
use parking_lot::Mutex;
use serde_json::Map;
use sha2::{Digest, Sha256};

use crate::manifest::{MANIFEST, Manifest, Row, SCHEMA};
use crate::service::install_found;
use crate::version::Version;
use crate::zipdir::tests::archive;
use crate::{Install, Start, install, register};

fn runtime() -> Runtime {
    Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 1, io_threads: 2 }).unwrap()
}

fn editor(rt: &Runtime, home: &Path, qa: bool) -> Arc<Editor> {
    let (log, guard) = Log::start(None, false).unwrap();
    std::mem::forget(guard);
    let mut reg = Registry::new();
    register(&mut reg);
    let mut opts = EditorOptions::default();
    opts.paths.exe_dir = home.to_path_buf();
    opts.paths.home = home.to_path_buf();
    opts.flags = Flags { qa_github: qa, ..Flags::default() };
    Editor::new(opts, rt.exec(), log, reg)
}

fn call(rt: &Runtime, ed: &Arc<Editor>, cmd: &str, args: Value) -> Value {
    rt.block_on(ed.call(cmd, args.as_object().cloned().unwrap_or_default(), Origin::Mcp))
}

#[test]
fn a_development_build_has_no_updater() {
    let rt = runtime();
    let home = tempfile::tempdir().unwrap();
    // a folder without update-manifest.txt beside the program
    let editor = editor(&rt, home.path(), false);
    let svc = install(&editor, Start::default());
    assert!(svc.install.is_none());
    let status = call(&rt, &editor, "update.status", json!({}));
    assert_eq!(status["state"], "off", "{status}");
    assert_eq!(status["text"], "This is a development build: it does not update itself.");
    for cmd in ["update.check", "update.download", "update.apply"] {
        let reply = rt.block_on(editor.call(cmd, Map::new(), Origin::Mcp));
        assert_eq!(reply["code"], "refused", "{cmd}: {reply}");
    }
    assert!(!home.path().join(".update").exists());
}

/// A release on 127.0.0.1: the latest release, a download address that redirects to storage, the
/// archive by range. It counts what was read and cuts one answer when told.
struct Server {
    port: u16,
    reads: Arc<Mutex<Vec<(u64, u64, usize)>>>,
    cut: Arc<Mutex<Option<usize>>>,
    stop: Arc<AtomicBool>,
}

fn respond(
    mut s: TcpStream,
    zip: &[u8],
    release: &str,
    reads: &Mutex<Vec<(u64, u64, usize)>>,
    cut: &Mutex<Option<usize>>,
) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
    assert!(!head.to_ascii_lowercase().contains("authorization:"), "a token was sent: {head}");
    let range = head
        .lines()
        .find_map(|l| l.strip_prefix("range: bytes=").or_else(|| l.strip_prefix("Range: bytes=")));
    let send = |s: &mut TcpStream, status: &str, extra: &str, body: &[u8], limit: Option<usize>| {
        let _ = write!(
            s,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        );
        let n = limit.unwrap_or(body.len()).min(body.len());
        let _ = s.write_all(&body[..n]);
        let _ = s.flush();
        n
    };
    match path.as_str() {
        "/repos/qa/Grizzly-Editor/releases/latest" => {
            send(&mut s, "200 OK", "Content-Type: application/json\r\n", release.as_bytes(), None);
        }
        "/download/archive.zip" => {
            send(&mut s, "302 Found", "Location: /storage/archive.zip\r\n", b"", None);
        }
        "/storage/archive.zip" => {
            let Some(r) = range else {
                send(&mut s, "200 OK", "", zip, None);
                return;
            };
            let (a, b) = r.trim().split_once('-').unwrap();
            let (a, b): (u64, u64) = (a.parse().unwrap(), b.parse().unwrap());
            let body = &zip[a as usize..=b as usize];
            let limit = cut.lock().take();
            let extra = format!("Content-Range: bytes {a}-{b}/{}\r\n", zip.len());
            let sent = send(&mut s, "206 Partial Content", &extra, body, limit);
            reads.lock().push((a, b + 1, sent));
        }
        _ => {
            send(&mut s, "404 Not Found", "", b"", None);
        }
    }
}

fn serve(zip: Vec<u8>, version: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let release = json!({
        "tag_name": version,
        "body": "# What is new\n- A second version.",
        "html_url": format!("http://127.0.0.1:{port}/qa/Grizzly-Editor/releases/tag/{version}"),
        "assets": [{ "id": 7, "name": format!("Grizzly-Editor-{version}.zip"), "size": zip.len(),
                     "browser_download_url": format!("http://127.0.0.1:{port}/download/archive.zip") }]
    })
    .to_string();
    let server =
        Server { port, reads: Arc::default(), cut: Arc::default(), stop: Arc::new(AtomicBool::new(false)) };
    let (reads, cut, stop) = (server.reads.clone(), server.cut.clone(), server.stop.clone());
    let zip = Arc::new(zip);
    std::thread::spawn(move || {
        for s in listener.incoming() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Ok(s) = s else { continue };
            let (zip, release, reads, cut) = (zip.clone(), release.clone(), reads.clone(), cut.clone());
            std::thread::spawn(move || respond(s, &zip, &release, &reads, &cut));
        }
    });
    server
}

fn row(path: &str, data: &[u8]) -> Row {
    Row { sha256: hex::encode(Sha256::digest(data)), size: data.len() as u64, path: path.into() }
}

fn manifest(version: &str, files: &[(&str, &[u8])]) -> Manifest {
    Manifest {
        version: Version::parse(version).unwrap(),
        rows: files.iter().map(|(p, d)| row(p, d)).collect(),
    }
}

fn wait_until(what: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn an_update_is_checked_downloaded_by_its_delta_resumed_and_applied() {
    let rt = runtime();
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let big: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let old_files: [(&str, &[u8]); 3] = [("a.txt", b"the old a"), ("b.bin", &big), ("gone.txt", b"dropped")];
    let new_files: [(&str, &[u8]); 3] =
        [("a.txt", b"the new a, longer"), ("b.bin", &big), ("c/new.txt", b"brand new")];
    for (p, d) in old_files {
        std::fs::write(root.path().join(p), d).unwrap();
    }
    std::fs::write(root.path().join("mine.txt"), b"the author's").unwrap();
    let old = manifest("0.7.0", &old_files);
    std::fs::write(root.path().join(MANIFEST), old.to_text()).unwrap();
    let new = manifest("0.7.1", &new_files);
    let text = new.to_text();
    assert!(text.starts_with(&format!("schema={SCHEMA}\n")));
    let mut entries: Vec<(&str, &[u8])> = new_files.to_vec();
    entries.push((MANIFEST, text.as_bytes()));
    let zip = archive(&entries, b"", false);
    let server = serve(zip.clone(), "0.7.1");
    let base = format!("http://127.0.0.1:{}", server.port);

    let ed = editor(&rt, home.path(), true);
    let inst = Install {
        root: root.path().to_path_buf(),
        repo: "qa/Grizzly-Editor".into(),
        api: base.clone(),
        web: base.clone(),
        qa: true,
        program: root.path().join("editor.bin"),
        manifest: old,
        pace: Duration::from_millis(10),
        move_wait: Duration::from_millis(200),
    };
    install_found(&ed, Start::default(), Ok(inst.clone()));

    let st = call(&rt, &ed, "update.check", json!({}));
    assert_eq!(st["state"], "available", "{st}");
    assert_eq!(st["latest"], "0.7.1");
    assert_eq!(st["download"]["files"], 2);
    assert_eq!(st["download"]["mode"], "ranges");
    // the end of the archive and the manifest's record, nothing else
    assert_eq!(server.reads.lock().len(), 2, "{:?}", server.reads.lock());

    // the first range is cut inside the local header of a.txt's record
    server.reads.lock().clear();
    *server.cut.lock() = Some(10);
    let st = call(&rt, &ed, "update.download", json!({}));
    assert_eq!(st["state"], "ready", "{st}");
    let reads = server.reads.lock().clone();
    assert_eq!(reads.len(), 3, "{reads:?}");
    assert_eq!(reads[0].2, 10);
    assert_eq!(reads[1].0, reads[0].0 + 10, "the cut range resumes at its byte: {reads:?}");
    // b.bin is unchanged and 300 KB between the two records: never asked for
    let total: usize = reads.iter().map(|r| r.2).sum();
    assert!(total < 1000, "{reads:?}");

    let st = call(&rt, &ed, "update.apply", json!({ "restart": false }));
    assert_eq!(st["ok"], true, "{st}");
    assert_eq!(st["from"], "0.7.0");
    assert_eq!(std::fs::read(root.path().join("a.txt")).unwrap(), b"the new a, longer");
    assert_eq!(std::fs::read(root.path().join("c").join("new.txt")).unwrap(), b"brand new");
    assert!(!root.path().join("gone.txt").exists());
    assert_eq!(std::fs::read(root.path().join("mine.txt")).unwrap(), b"the author's");
    assert_eq!(std::fs::read_to_string(root.path().join(MANIFEST)).unwrap(), text);
    assert!(wait_until(|| ed.quit_token().is_cancelled()), "the editor ends after the update");

    // the next start: the new version says where it came from and tidies up
    let mut next = inst;
    next.manifest = new;
    let svc = install_found(&ed, Start::default(), Ok(next));
    assert!(wait_until(|| svc.updated().is_some() && !root.path().join(".update").exists()));
    let st = call(&rt, &ed, "update.status", json!({}));
    assert_eq!(st["updated_from"], "0.7.0", "{st}");
    assert_eq!(st["updated_to"], "0.7.1");
    server.stop.store(true, Ordering::Relaxed);
}
