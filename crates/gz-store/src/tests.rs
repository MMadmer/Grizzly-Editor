//! Stress tests of the store's synchronisation: many writers and readers at once. What they
//! assert is deterministic; only the interleaving is not.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gz_base::{Origin, Rev};
use gz_task::{ExecOptions, Runtime};

use crate::{Apply, CmdError, State, Store, StoreOptions, imbl};

/// A log of numbers and their sum: a state is one a command produced exactly when the sum
/// matches the log.
#[derive(Clone, Default)]
struct Tally {
    log: imbl::Vector<u32>,
    sum: u64,
}

enum Cmd {
    Add(u32),
    /// An edit of a gesture: one undo step per key in a row.
    AddMerged(u32, &'static str),
    /// Changes the content in a way Undo does not take back.
    Note(u32),
    Read,
    Save,
    Fail,
    Panic,
}

impl State for Tally {
    type Cmd = Cmd;
    type Reply = u64;

    fn apply(&mut self, cmd: Cmd, cx: &mut Apply) -> Result<u64, CmdError> {
        match cmd {
            Cmd::Add(v) => {
                // the copy is half edited when a reader could look: it must never see this
                self.log.push_back(v);
                self.sum += u64::from(v);
                cx.undoable("Add");
            }
            Cmd::AddMerged(v, key) => {
                self.log.push_back(v);
                self.sum += u64::from(v);
                cx.undoable_merged("Drag", key);
            }
            Cmd::Note(v) => {
                self.log.push_back(v);
                self.sum += u64::from(v);
                cx.content_changed();
            }
            Cmd::Read => cx.unchanged(),
            Cmd::Save => {
                cx.unchanged();
                cx.mark_saved();
            }
            Cmd::Fail => {
                self.log.push_back(1);
                return Err(CmdError::invalid("refused"));
            }
            Cmd::Panic => {
                self.log.push_back(1);
                panic!("a broken command");
            }
        }
        Ok(self.sum)
    }
}

fn consistent(t: &Tally) -> bool {
    t.log.iter().map(|v| u64::from(*v)).sum::<u64>() == t.sum
}

fn runtime() -> Runtime {
    Runtime::start(ExecOptions { async_threads: 4, cpu_threads: 2, io_threads: 2 }).unwrap()
}

fn store(rt: &Runtime, limit: usize) -> Store<Tally> {
    Store::spawn("tally", Tally::default(), &rt.exec(), StoreOptions { history_limit: limit })
}

/// Readers on threads of their own, spinning on snapshots until `stop`: each checks that the
/// revisions it sees only grow and that every state is whole. Returns how many it saw.
fn readers(s: &Store<Tally>, n: usize, stop: &Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<usize>> {
    (0..n)
        .map(|_| {
            let (s, stop) = (s.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut last = Rev::ZERO;
                let mut seen = 0;
                // at least one look: on a busy machine the writers may be done before a reader
                // thread first runs
                loop {
                    let snap = s.snapshot();
                    assert!(snap.rev >= last, "a revision went back: {:?} after {:?}", snap.rev, last);
                    assert!(consistent(&snap.state), "a reader saw a state no command produced");
                    last = snap.rev;
                    seen += 1;
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                }
                seen
            })
        })
        .collect()
}

#[test]
fn readers_see_growing_revisions_and_whole_states() {
    let rt = runtime();
    let s = store(&rt, 10_000);
    let stop = Arc::new(AtomicBool::new(false));
    let watching = readers(&s, 4, &stop);
    let writers: Vec<_> = (0..8u32)
        .map(|w| {
            let s = s.clone();
            rt.exec().spawn(async move {
                for i in 0..200u32 {
                    let cmd = match i % 4 {
                        0 => Cmd::Add(w * 1000 + i),
                        1 => Cmd::Note(i),
                        2 => Cmd::Fail,
                        _ => Cmd::Read,
                    };
                    let _ = s.send(cmd, Origin::Mcp).await;
                }
            })
        })
        .collect();
    rt.block_on(async {
        for w in writers {
            w.await.unwrap();
        }
    });
    stop.store(true, Ordering::Relaxed);
    for r in watching {
        assert!(r.join().unwrap() > 0);
    }
    let snap = s.snapshot();
    // 8 writers x 50 adds and 50 notes; refusals and reads publish nothing
    assert_eq!(snap.log.len(), 800);
    assert_eq!(snap.rev, Rev(800));
    assert!(consistent(&snap.state));
    assert_eq!(s.history().undo.len(), 400);
}

#[test]
fn undo_and_redo_amid_edits_neither_lose_nor_double_a_step() {
    let rt = runtime();
    let s = store(&rt, 10_000);
    let stop = Arc::new(AtomicBool::new(false));
    let watching = readers(&s, 2, &stop);
    let undone = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = (0..6u32)
        .map(|w| {
            let (s, undone) = (s.clone(), undone.clone());
            rt.exec().spawn(async move {
                for i in 0..150u32 {
                    match (w + i) % 5 {
                        0..=2 => {
                            s.send(Cmd::Add(i + 1), Origin::Ui).await.unwrap();
                        }
                        3 => {
                            if s.undo().await.is_some() {
                                undone.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        _ => {
                            s.redo().await;
                        }
                    }
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
    for r in watching {
        r.join().unwrap();
    }
    // every edit is one step: the state's log and the undo stack agree
    let snap = s.snapshot();
    assert!(consistent(&snap.state));
    let steps = s.history().undo.len();
    // what the last undos left to redo
    let left = s.history().redo.len();
    assert_eq!(steps, snap.log.len());
    assert!(undone.load(Ordering::SeqCst) > 0);
    // undoing them all reaches the empty start, redoing them all the same state again
    let (back, forth) = rt.block_on(async {
        let mut back = 0;
        while s.undo().await.is_some() {
            back += 1;
        }
        let empty = s.snapshot();
        assert!(empty.log.is_empty() && empty.sum == 0);
        assert!(!s.is_dirty());
        let mut forth = 0;
        while s.redo().await.is_some() {
            forth += 1;
        }
        (back, forth)
    });
    // redo brings back every step undone here, and the ones left before
    assert_eq!((back, forth), (steps, steps + left));
    let end = s.snapshot();
    assert_eq!(end.log.len(), snap.log.len() + left);
    assert!(end.log.iter().zip(snap.log.iter()).all(|(a, b)| a == b));
    assert!(consistent(&end.state));
}

#[test]
fn merged_steps_seal_and_saved_marks_hold_under_load() {
    let rt = runtime();
    let s = store(&rt, 10_000);
    // a flood of reads and notes beside the gesture: neither makes an undo step
    let flood: Vec<_> = (0..4)
        .map(|_| {
            let s = s.clone();
            rt.exec().spawn(async move {
                for i in 0..300u32 {
                    let cmd = if i % 3 == 0 { Cmd::Note(1) } else { Cmd::Read };
                    s.send(cmd, Origin::Mcp).await.unwrap();
                }
            })
        })
        .collect();
    rt.block_on(async {
        for i in 0..50 {
            s.send(Cmd::AddMerged(i, "drag"), Origin::Ui).await.unwrap();
        }
        s.seal();
        for i in 0..50 {
            s.send(Cmd::AddMerged(1000 + i, "drag"), Origin::Ui).await.unwrap();
        }
        for f in flood {
            f.await.unwrap();
        }
    });
    assert_eq!(s.history().undo, ["Drag", "Drag"]);
    // the saved mark follows the edit serial: Undo makes the document dirty, Redo clean again
    let saved_at = rt.block_on(async {
        s.send(Cmd::Save, Origin::Ui).await.unwrap();
        assert!(!s.is_dirty());
        let at = s.snapshot().edit;
        s.undo().await.unwrap();
        assert!(s.is_dirty());
        s.redo().await.unwrap();
        at
    });
    assert_eq!(s.snapshot().edit, saved_at);
    assert!(!s.is_dirty());
    // one undo step takes the whole second gesture back and leaves the first
    rt.block_on(s.undo()).unwrap();
    let log = s.snapshot().log.clone();
    assert!(log.iter().all(|v| *v < 1000));
    assert_eq!(log.iter().filter(|v| **v != 1).count(), 49);
    assert!(consistent(&s.snapshot().state));
}

#[test]
fn a_command_that_panics_leaves_the_last_good_snapshot_and_a_live_store() {
    let rt = runtime();
    let s = store(&rt, 64);
    let stop = Arc::new(AtomicBool::new(false));
    let watching = readers(&s, 2, &stop);
    let tasks: Vec<_> = (0..4u32)
        .map(|w| {
            let s = s.clone();
            rt.exec().spawn(async move {
                let mut ok = 0;
                for i in 0..100u32 {
                    if (w + i) % 4 == 0 {
                        let before = s.rev();
                        let e = s.send(Cmd::Panic, Origin::Mcp).await.err().map(|e| e.code);
                        assert_eq!(e.as_deref(), Some("internal"));
                        assert!(s.rev() >= before);
                    } else {
                        s.send(Cmd::Add(1), Origin::Mcp).await.unwrap();
                        ok += 1;
                    }
                }
                ok
            })
        })
        .collect();
    let ok: usize = rt.block_on(async {
        let mut n = 0;
        for t in tasks {
            n += t.await.unwrap();
        }
        n
    });
    stop.store(true, Ordering::Relaxed);
    for r in watching {
        r.join().unwrap();
    }
    let snap = s.snapshot();
    assert_eq!(snap.log.len(), ok);
    assert!(consistent(&snap.state));
    // the history keeps its limit
    assert_eq!(s.history().undo.len(), 64);
    assert_eq!(rt.block_on(s.send(Cmd::Add(5), Origin::Ui)).unwrap(), ok as u64 + 5);
}
