use std::future::Future;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;

use rayon::prelude::*;
use tokio::sync::oneshot;

use crate::wake::Wake;

/// Thread counts. Zero means "work it out from the machine".
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecOptions {
    pub async_threads: usize,
    pub cpu_threads: usize,
    pub io_threads: usize,
}

/// Owns the threads. Lives in `main`; everything else holds an [`Executors`].
pub struct Runtime {
    rt: tokio::runtime::Runtime,
    exec: Executors,
}

/// The handle work is started through. Clone it freely.
#[derive(Clone)]
pub struct Executors(Arc<Inner>);

struct Inner {
    rt: tokio::runtime::Handle,
    cpu: rayon::ThreadPool,
    io: rayon::ThreadPool,
    wake: Wake,
}

fn pool(name: &'static str, threads: usize) -> io::Result<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(move |i| format!("{name}-{i}"))
        .build()
        .map_err(io::Error::other)
}

impl Runtime {
    pub fn start(opts: ExecOptions) -> io::Result<Self> {
        let cores = std::thread::available_parallelism().map_or(4, usize::from);
        let pick = |asked: usize, auto: usize| if asked == 0 { auto } else { asked };
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(pick(opts.async_threads, (cores / 2).clamp(2, 8)))
            .thread_name("gz-async")
            .enable_all()
            .build()?;
        let exec = Executors(Arc::new(Inner {
            rt: rt.handle().clone(),
            cpu: pool("gz-cpu", pick(opts.cpu_threads, cores))?,
            // Files wait on the disk and on whatever scans them, not on a core: more threads
            // than cores is the point.
            io: pool("gz-io", pick(opts.io_threads, (cores * 2).clamp(8, 32)))?,
            wake: Wake::new(),
        }));
        Ok(Self { rt, exec })
    }

    #[must_use]
    pub fn exec(&self) -> Executors {
        self.exec.clone()
    }

    /// Runs a future to its end on the calling thread. For `main` and for tests.
    pub fn block_on<F: Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }
}

impl Executors {
    /// Starts an async task.
    pub fn spawn<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        // a task spawned for a request works for it too: the jobs it starts are the request's
        match crate::jobs::current_scope() {
            Some(scope) => self.0.rt.spawn(crate::jobs::in_scope(scope, fut)),
            None => self.0.rt.spawn(fut),
        }
    }

    #[must_use]
    pub fn handle(&self) -> &tokio::runtime::Handle {
        &self.0.rt
    }

    #[must_use]
    pub fn wake(&self) -> &Wake {
        &self.0.wake
    }

    /// The CPU pool, for `install` and scoped parallel iterators inside sync code.
    #[must_use]
    pub fn cpu_pool(&self) -> &rayon::ThreadPool {
        &self.0.cpu
    }

    #[must_use]
    pub fn io_pool(&self) -> &rayon::ThreadPool {
        &self.0.io
    }

    /// Computes on the CPU pool and awaits the result. A panic in `f` continues in the awaiter.
    pub async fn cpu<R, F>(&self, f: F) -> R
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        run_on(&self.0.cpu, f).await
    }

    /// Runs blocking file work on the IO pool and awaits the result.
    pub async fn io<R, F>(&self, f: F) -> R
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        run_on(&self.0.io, f).await
    }

    /// Fire and forget on the CPU pool.
    pub fn spawn_cpu(&self, f: impl FnOnce() + Send + 'static) {
        self.0.cpu.spawn(f);
    }

    /// Fire and forget on the IO pool.
    pub fn spawn_io(&self, f: impl FnOnce() + Send + 'static) {
        self.0.io.spawn(f);
    }

    /// Maps over items on the IO pool, in order. Blocks the caller: call it from sync code that
    /// already runs on a pool, never from an async task or the UI thread.
    pub fn par_io<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send,
        R: Send,
        F: Fn(T) -> R + Sync + Send,
    {
        self.0.io.install(|| items.into_par_iter().map(f).collect())
    }

    /// Maps over items on the CPU pool, in order. Blocks the caller like [`Self::par_io`].
    pub fn par_cpu<T, R, F>(&self, items: Vec<T>, f: F) -> Vec<R>
    where
        T: Send,
        R: Send,
        F: Fn(T) -> R + Sync + Send,
    {
        self.0.cpu.install(|| items.into_par_iter().map(f).collect())
    }
}

async fn run_on<R, F>(pool: &rayon::ThreadPool, f: F) -> R
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    pool.spawn(move || {
        // the receiver may be gone (the awaiter was cancelled): nothing to tell then
        let _ = tx.send(catch_unwind(AssertUnwindSafe(f)));
    });
    match rx.await {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => resume_unwind(payload),
        // the pool dropped the closure without running it: only at shutdown
        Err(_) => std::future::pending().await,
    }
}
