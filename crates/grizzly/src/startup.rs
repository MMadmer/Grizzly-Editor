//! The startup profile: one line per stage with its seconds, reads and file operations.
//!
//! A start costs the NUMBER of file touches, not the bytes, so the profile counts operations;
//! the first line is what Windows did before `main` (the loader: a scanner meeting the binary).

use std::time::{Instant, SystemTime};

use gz_log::Log;
use gz_win::ProcessCounters;

struct Stage {
    name: String,
    seconds: f64,
    reads: u64,
    ops: u64,
}

pub struct Startup {
    began: Instant,
    stage_start: Instant,
    stage_counters: Option<ProcessCounters>,
    current: String,
    stages: Vec<Stage>,
    loader: Stage,
}

impl Startup {
    /// Call first thing in `main`: what was counted before it belongs to the loader.
    pub fn begin() -> Self {
        let now = Instant::now();
        let counters = gz_win::process_counters().ok();
        let loader = Stage {
            name: "loader (before main)".to_owned(),
            seconds: counters.map_or(0.0, |c| SystemTime::now().duration_since(c.created).map_or(0.0, |d| d.as_secs_f64())),
            reads: counters.map_or(0, |c| c.read_ops),
            ops: counters.map_or(0, |c| c.file_ops()),
        };
        Self { began: now, stage_start: now, stage_counters: counters, current: "Starting".to_owned(), stages: Vec::new(), loader }
    }

    fn close_stage(&mut self) {
        let now = Instant::now();
        let counters = gz_win::process_counters().ok();
        let (reads, ops) = match (self.stage_counters, counters) {
            (Some(a), Some(b)) => (b.read_ops.saturating_sub(a.read_ops), b.file_ops().saturating_sub(a.file_ops())),
            _ => (0, 0),
        };
        self.stages.push(Stage {
            name: std::mem::take(&mut self.current),
            seconds: now.duration_since(self.stage_start).as_secs_f64(),
            reads,
            ops,
        });
        self.stage_start = now;
        self.stage_counters = counters;
    }

    /// Names the stage that begins now.
    pub fn stage(&mut self, name: &str) {
        self.close_stage();
        self.current = name.to_owned();
    }

    /// Ends the profile and writes it to the log.
    pub fn finish(mut self, log: &Log) {
        self.close_stage();
        let total = self.loader.seconds + self.began.elapsed().as_secs_f64();
        let reads: u64 = self.loader.reads + self.stages.iter().map(|s| s.reads).sum::<u64>();
        let ops: u64 = self.loader.ops + self.stages.iter().map(|s| s.ops).sum::<u64>();
        log.info(format!("startup: {total:.2} s, {reads} file reads, {ops} file operations"));
        for s in std::iter::once(&self.loader).chain(self.stages.iter()) {
            log.info(format!("  {:<30} {:>6.2} s {:>8} reads {:>8} ops", s.name, s.seconds, s.reads, s.ops));
        }
        log.info("file names: UTF-8 (wide system calls; the code page is not involved)");
    }
}
