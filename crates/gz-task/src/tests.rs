//! Stress tests of the job center: jobs started, cancelled and waited for from many threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::{Cancel, ExecOptions, JobState, Jobs, Runtime};

fn runtime() -> Runtime {
    Runtime::start(ExecOptions { async_threads: 4, cpu_threads: 4, io_threads: 2 }).unwrap()
}

#[test]
fn cancelled_jobs_stop_at_their_next_check_and_every_wait_returns() {
    let rt = Arc::new(runtime());
    let jobs = Jobs::new(rt.exec());
    // steps a job took after its stop was asked for, beyond the one it was in
    let overran = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    // a watcher that never sees a finished job as running
    let watcher = {
        let (jobs, stop) = (jobs.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut ended = std::collections::HashSet::new();
            while !stop.load(Ordering::Relaxed) {
                for info in jobs.snapshot() {
                    if info.running() {
                        assert!(!ended.contains(&info.id), "job {:?} ran again after it ended", info.id);
                        assert!(info.ended_ms.is_none());
                    } else {
                        assert!(info.ended_ms.is_some());
                        ended.insert(info.id);
                    }
                }
            }
        })
    };
    let outcomes: Vec<(JobState, bool)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let (rt, jobs, overran) = (rt.clone(), jobs.clone(), overran.clone());
                s.spawn(move || {
                    (0..6)
                        .map(|i| {
                            let steps = Arc::new(AtomicUsize::new(0));
                            let done = steps.clone();
                            let cancel = i % 2 == 0;
                            // a job to be stopped runs until it is; the others end by themselves
                            let total = if cancel { usize::MAX } else { 200 };
                            let id = jobs
                                .start("test", None, format!("job {t}.{i}"), move |ctx| async move {
                                    for step in 0..total {
                                        ctx.check()?;
                                        done.fetch_add(1, Ordering::SeqCst);
                                        ctx.progress(step, total);
                                        tokio::task::yield_now().await;
                                    }
                                    Ok(serde_json::json!({ "steps": total }))
                                })
                                .unwrap();
                            let mut at_stop = 0;
                            if cancel {
                                std::thread::sleep(Duration::from_millis(1));
                                jobs.cancel(id);
                                at_stop = steps.load(Ordering::SeqCst);
                            }
                            let info = rt.block_on(jobs.wait(id)).unwrap();
                            // the step under way when the stop came may end; no other starts
                            if cancel && steps.load(Ordering::SeqCst) > at_stop + 1 {
                                overran.fetch_add(1, Ordering::SeqCst);
                            }
                            (info.state, cancel)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    stop.store(true, Ordering::Relaxed);
    watcher.join().unwrap();
    assert_eq!(outcomes.len(), 48);
    for (state, cancelled) in outcomes {
        assert_eq!(state, if cancelled { JobState::Cancelled } else { JobState::Done });
    }
    assert_eq!(overran.load(Ordering::SeqCst), 0);
    assert!(!jobs.any_running());
}

#[test]
fn a_group_runs_one_job_at_a_time_however_many_ask() {
    let rt = runtime();
    let jobs = Jobs::new(rt.exec());
    let gate = Cancel::new();
    let started = std::thread::scope(|s| {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (jobs, gate) = (jobs.clone(), gate.clone());
                s.spawn(move || {
                    jobs.start("build", Some("build"), "Build", move |_| async move {
                        gate.cancelled().await;
                        Ok(serde_json::Value::Null)
                    })
                    .ok()
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(started.len(), 1);
    assert!(jobs.running_in("build").is_some());
    gate.cancel();
    let info = rt.block_on(jobs.wait(started[0])).unwrap();
    assert_eq!(info.state, JobState::Done);
    assert!(jobs.running_in("build").is_none());
}

#[test]
fn a_job_that_panics_fails_and_its_waiters_return() {
    let rt = runtime();
    let jobs = Jobs::new(rt.exec());
    let id = jobs
        .start("test", Some("g"), "Broken", |_| async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            panic!("broken job")
        })
        .unwrap();
    let waits: Vec<_> = (0..8)
        .map(|_| {
            rt.exec().spawn({
                let jobs = jobs.clone();
                async move { jobs.wait(id).await }
            })
        })
        .collect();
    rt.block_on(async {
        for w in waits {
            let info = w.await.unwrap().unwrap();
            assert_eq!(info.state, JobState::Failed);
            assert!(info.error.as_deref().is_some_and(|e| e.contains("broken job")));
        }
    });
    // the group is free again
    assert!(jobs.start("test", Some("g"), "Next", |_| async { Ok(serde_json::Value::Null) }).is_ok());
}

#[test]
fn a_request_scope_learns_the_jobs_started_for_it() {
    let rt = runtime();
    let exec = rt.exec();
    let jobs = Jobs::new(exec.clone());
    let quick = |_| async { Ok(serde_json::Value::Null) };
    let scope = crate::JobScope::default();
    let (j, e) = (jobs.clone(), exec.clone());
    let (inside, spawned) = rt.block_on(crate::in_scope(scope.clone(), async move {
        let inside = j.start("a", None, "A", quick).unwrap();
        // a task the command spawns works for the same request
        let j2 = j.clone();
        let spawned = e.spawn(async move { j2.start("b", None, "B", quick).unwrap() }).await.unwrap();
        (inside, spawned)
    }));
    // a job started outside every scope belongs to no request
    let outside = jobs.start("c", None, "C", quick).unwrap();
    assert_eq!(scope.ids(), vec![inside, spawned]);
    assert!(!scope.ids().contains(&outside));
}

#[test]
fn a_closure_run_on_another_thread_for_a_request_keeps_its_scope() {
    let rt = runtime();
    let jobs = Jobs::new(rt.exec());
    let quick = |_| async { Ok(serde_json::Value::Null) };
    let scope = crate::JobScope::default();
    // what the UI thread's queue does: the request's task takes its scope along, the closure
    // runs later on a thread of its own inside it
    let taken = rt.block_on(crate::in_scope(scope.clone(), async { crate::current_scope() }));
    let (j, s) = (jobs.clone(), taken.unwrap());
    let on_thread = std::thread::spawn(move || {
        crate::in_scope_sync(s, || {
            let _enter = rt.exec().handle().enter();
            j.start("a", None, "A", quick).unwrap()
        })
    })
    .join()
    .unwrap();
    assert_eq!(scope.ids(), vec![on_thread]);
    assert!(crate::current_scope().is_none());
}
