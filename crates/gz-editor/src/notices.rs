use std::collections::VecDeque;

use gz_log::Log;
use gz_task::Wake;
use parking_lot::Mutex;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Note,
    Warning,
    Error,
}

/// A message for the author raised away from the UI thread: a command posted from a button
/// failed, a job has something to say. The window shows it; headless it is only logged.
#[derive(Clone, Debug, Serialize)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

pub struct Notices {
    queue: Mutex<VecDeque<Notice>>,
    log: Log,
    wake: Wake,
}

impl Notices {
    #[must_use]
    pub fn new(log: Log, wake: Wake) -> Self {
        Self { queue: Mutex::new(VecDeque::new()), log, wake }
    }

    pub fn push(&self, kind: NoticeKind, text: impl Into<String>) {
        let text = text.into();
        match kind {
            NoticeKind::Note => self.log.info(&text),
            NoticeKind::Warning => self.log.warn(&text),
            NoticeKind::Error => self.log.error(&text),
        }
        let mut queue = self.queue.lock();
        queue.push_back(Notice { kind, text });
        // nobody draining (headless): keep the tail only
        while queue.len() > 64 {
            queue.pop_front();
        }
        drop(queue);
        self.wake.ping();
    }

    pub fn error(&self, text: impl Into<String>) {
        self.push(NoticeKind::Error, text);
    }

    pub fn note(&self, text: impl Into<String>) {
        self.push(NoticeKind::Note, text);
    }

    /// Takes what queued up. Called by the window once a frame.
    #[must_use]
    pub fn drain(&self) -> Vec<Notice> {
        self.queue.lock().drain(..).collect()
    }
}
