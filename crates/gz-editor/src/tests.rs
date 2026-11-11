//! Stress tests of the frame clock and of saving as it is edited.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use gz_base::{Origin, Rev};
use gz_store::{Apply, CmdError, State, Store, StoreOptions};
use gz_task::{ExecOptions, Runtime, Wake};

use crate::{AutoSave, FrameClock, FrameStamp};

fn runtime() -> Runtime {
    Runtime::start(ExecOptions { async_threads: 4, cpu_threads: 2, io_threads: 4 }).unwrap()
}

#[test]
fn frame_held_requests_are_answered_once_and_never_before_their_frame() {
    let rt = runtime();
    let wake = Wake::new();
    let asked = Arc::new(AtomicUsize::new(0));
    let a = asked.clone();
    wake.set(move || {
        a.fetch_add(1, Ordering::Relaxed);
    });
    let clock = Arc::new(FrameClock::new(wake));
    clock.publish(FrameStamp { frame: 1, ..FrameStamp::default() });
    let presented = Arc::new(AtomicU64::new(1));
    let stop = Arc::new(AtomicBool::new(false));
    // the window: presents a frame every few hundred microseconds while anybody waits
    let window = {
        let (clock, presented, stop) = (clock.clone(), presented.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_micros(300));
                let next = presented.fetch_add(1, Ordering::SeqCst) + 1;
                clock.publish(FrameStamp { frame: next, ..FrameStamp::default() });
            }
        })
    };
    let answered = Arc::new(AtomicUsize::new(0));
    // 32 connections, 20 requests each, for 1..4 frames
    let tasks: Vec<_> = (0..32u64)
        .map(|c| {
            let (clock, presented, answered) = (clock.clone(), presented.clone(), answered.clone());
            rt.exec().spawn(async move {
                for i in 0..20u64 {
                    let frames = 1 + (c + i) % 4;
                    let before = clock.current().frame;
                    let stamp = clock.after(frames).await.unwrap();
                    // the frame it waited for was presented after the request was made
                    assert!(stamp.frame >= before + frames, "{} < {before} + {frames}", stamp.frame);
                    assert!(stamp.frame <= presented.load(Ordering::SeqCst));
                    answered.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    rt.block_on(async {
        for t in tasks {
            t.await.unwrap();
        }
    });
    stop.store(true, Ordering::Relaxed);
    window.join().unwrap();
    assert_eq!(answered.load(Ordering::SeqCst), 32 * 20);
    // every request asked the window for its frames
    assert!(asked.load(Ordering::Relaxed) >= 32 * 20);
}

/// A document that is one number, rendered as its text.
#[derive(Clone, Default)]
struct Doc(u64);

impl State for Doc {
    type Cmd = u64;
    type Reply = ();
    fn apply(&mut self, v: u64, cx: &mut Apply) -> Result<(), CmdError> {
        self.0 = v;
        cx.content_changed();
        Ok(())
    }
}

#[test]
fn a_burst_of_edits_ends_on_disk_as_the_last_one_and_nothing_follows_the_flush() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let file: PathBuf = dir.path().join("doc.txt");
    let store = Store::spawn("doc", Doc::default(), &rt.exec(), StoreOptions::default());
    let writes = Arc::new(AtomicUsize::new(0));
    let (f, w) = (file.clone(), writes.clone());
    let saver = AutoSave::spawn(
        &rt.exec(),
        &store,
        move |d: &Doc| {
            w.fetch_add(1, Ordering::SeqCst);
            Some((f.clone(), d.0.to_string().into_bytes()))
        },
        |e| panic!("{e}"),
    );
    // eight writers, 100 edits each; each flushes its own edit like a command that answers
    let tasks: Vec<_> = (0..8u64)
        .map(|t| {
            let (store, saver) = (store.clone(), saver.clone());
            rt.exec().spawn(async move {
                for i in 0..100u64 {
                    store.send(t * 1000 + i + 1, Origin::Mcp).await.unwrap();
                    let rev = store.rev();
                    saver.flush(rev).await.unwrap();
                }
            })
        })
        .collect();
    rt.block_on(async {
        for t in tasks {
            t.await.unwrap();
        }
    });
    let last = store.snapshot();
    assert_eq!(last.rev, Rev(800));
    rt.block_on(saver.flush(last.rev)).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), last.0.to_string());
    // overtaken states were skipped, and after the flush nothing is rendered or written
    let rendered = writes.load(Ordering::SeqCst);
    assert!(rendered <= 800);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(writes.load(Ordering::SeqCst), rendered);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), last.0.to_string());
}

#[test]
fn an_edit_made_before_the_saver_runs_is_saved_and_flushed() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("doc.txt");
    for round in 0..50u64 {
        let store = Store::spawn("doc", Doc::default(), &rt.exec(), StoreOptions::default());
        let f = file.clone();
        let saver = AutoSave::spawn(
            &rt.exec(),
            &store,
            move |d: &Doc| Some((f.clone(), d.0.to_string().into_bytes())),
            |_| {},
        );
        // sent at once: the saver's task may not have run yet
        rt.block_on(async {
            store.send(round + 1, Origin::Ui).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), saver.flush(store.rev())).await.unwrap().unwrap();
        });
        assert_eq!(std::fs::read_to_string(&file).unwrap(), (round + 1).to_string());
    }
}
