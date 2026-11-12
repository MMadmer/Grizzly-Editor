use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gz_base::Rev;
use gz_task::{ExecOptions, Runtime};

use super::{Answer, Query};

fn runtime() -> Runtime {
    Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 4, io_threads: 2 }).unwrap()
}

/// A computation that takes long enough for every other caller to arrive while it runs.
fn slow(count: &Arc<AtomicUsize>, value: u64) -> impl FnOnce() -> u64 + Send + 'static {
    let count = count.clone();
    move || {
        count.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(40));
        value
    }
}

#[test]
fn many_threads_asking_one_key_compute_once_per_revision() {
    let rt = Arc::new(runtime());
    let q: Query<&'static str, u64> = Query::new(rt.exec());
    let count = Arc::new(AtomicUsize::new(0));
    for (rev, value) in [(Rev(1), 10), (Rev(2), 20)] {
        let values: Vec<Arc<u64>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let (rt, q, count) = (rt.clone(), q.clone(), count.clone());
                    s.spawn(move || rt.block_on(q.fresh(&"k", rev, slow(&count, value))))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(values.iter().all(|v| **v == value));
        // one value, shared by every caller
        assert!(values.iter().all(|v| Arc::ptr_eq(v, &values[0])));
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn reads_that_do_not_wait_share_the_computation_of_those_that_do() {
    let rt = Arc::new(runtime());
    let q: Query<u32, u64> = Query::new(rt.exec());
    let count = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        for i in 0..12 {
            let (rt, q, count) = (rt.clone(), q.clone(), count.clone());
            s.spawn(move || {
                if i % 2 == 0 {
                    assert_eq!(*rt.block_on(q.fresh(&7, Rev(5), slow(&count, 3))), 3);
                } else {
                    // the UI's read: never waits, and starts at most the one computation
                    loop {
                        if let Answer::Fresh(v) = q.get(&7, Rev(5), slow(&count, 3)) {
                            assert_eq!(*v, 3);
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            });
        }
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // keys are apart
    assert_eq!(*rt.block_on(q.fresh(&8, Rev(5), slow(&count, 4))), 4);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn a_moved_revision_answers_the_old_value_until_the_new_one_is_there() {
    let rt = runtime();
    let q: Query<(), u64> = Query::new(rt.exec());
    assert_eq!(*rt.block_on(q.fresh(&(), Rev(1), || 1)), 1);
    let gate = Arc::new(std::sync::Barrier::new(2));
    let g = gate.clone();
    let first = q.get(&(), Rev(2), move || {
        g.wait();
        2
    });
    assert!(matches!(first, Answer::Stale(v) if *v == 1));
    gate.wait();
    let fresh = rt.block_on(q.fresh(&(), Rev(2), unreachable_value));
    assert_eq!(*fresh, 2);
}

/// What a computation that must never run would answer.
fn unreachable_value() -> u64 {
    u64::MAX
}

#[test]
fn a_panicking_computation_leaves_the_key_usable() {
    let rt = Arc::new(runtime());
    let q: Query<u8, u64> = Query::new(rt.exec());
    let waiting = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        let (rt1, q1) = (rt.clone(), q.clone());
        let panicked = s.spawn(move || {
            catch_unwind(AssertUnwindSafe(|| {
                rt1.block_on(q1.fresh(&1, Rev(1), || {
                    std::thread::sleep(Duration::from_millis(40));
                    panic!("broken input")
                }))
            }))
            .is_err()
        });
        std::thread::sleep(Duration::from_millis(10));
        // a caller that waited for the broken computation computes for itself
        let (rt2, q2, w) = (rt.clone(), q.clone(), waiting.clone());
        let other = s.spawn(move || {
            *rt2.block_on(q2.fresh(&1, Rev(1), move || {
                w.fetch_add(1, Ordering::SeqCst);
                9
            }))
        });
        assert!(panicked.join().unwrap());
        assert_eq!(other.join().unwrap(), 9);
    });
    assert_eq!(waiting.load(Ordering::SeqCst), 1);
    assert_eq!(q.peek_at(&1, Rev(1)).map(|v| *v), Some(9));
}

#[test]
fn a_read_does_not_compute_a_failed_revision_again() {
    let rt = runtime();
    let q: Query<u8, u64> = Query::new(rt.exec());
    let tries = Arc::new(AtomicUsize::new(0));
    let broken = |tries: &Arc<AtomicUsize>| {
        let t = tries.clone();
        move || -> u64 {
            t.fetch_add(1, Ordering::SeqCst);
            panic!("broken input")
        }
    };
    // a frame's read starts it once; the panic is remembered for that revision
    assert!(matches!(q.get(&1, Rev(1), broken(&tries)), Answer::Pending));
    for _ in 0..200 {
        if tries.load(Ordering::SeqCst) == 1 && !q.inner.slots.lock().get(&1).is_some_and(|s| s.running) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // every later frame's read of the same revision starts nothing
    for _ in 0..20 {
        assert!(matches!(q.get(&1, Rev(1), broken(&tries)), Answer::Pending));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(tries.load(Ordering::SeqCst), 1);
    // the revision moves: computed again
    let ok = Arc::new(AtomicUsize::new(0));
    let o = ok.clone();
    q.get(&1, Rev(2), move || {
        o.fetch_add(1, Ordering::SeqCst);
        7
    });
    for _ in 0..200 {
        if q.peek_at(&1, Rev(2)).is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(q.peek_at(&1, Rev(2)).map(|v| *v), Some(7));
    assert_eq!(ok.load(Ordering::SeqCst), 1);
}
