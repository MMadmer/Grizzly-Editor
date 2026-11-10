//! Recovery snapshots (`EditorRecovery`): the scene's undo stream written now and then into
//! `<project>\backup\recovery`, with a manifest that names the snapshots kept, and a session
//! marker that tells the next start whether the last session ended cleanly. The file layout and
//! the manifest are the C++ editor's, so either editor offers the other's snapshot.
//!
//! Every function here touches files and blocks: call it on the IO pool.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::io;

pub const MANIFEST_VERSION: u32 = 1;
pub const MAX_ENTRIES: usize = 64;
pub const RETRY_DELAY: Duration = Duration::from_secs(30);

/// The Preferences of recovery (`[recovery]` of the level preferences).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryPrefs {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub interaction_delay_seconds: u32,
    pub retention: u32,
}

impl Default for RecoveryPrefs {
    fn default() -> Self {
        Self { enabled: true, interval_minutes: 10, interaction_delay_seconds: 15, retention: 10 }
    }
}

impl RecoveryPrefs {
    #[must_use]
    pub fn interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.interval_minutes.clamp(1, 120)) * 60)
    }
}

/// `SRecoveryEntry`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub snapshot: String,
    pub source: String,
    pub saved_at: u64,
    pub snapshot_size: u64,
    pub snapshot_hash: u64,
    pub source_mtime: u64,
    pub source_size: u64,
}

/// `HashFile`: FNV-1a over the bytes.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 14_695_981_039_346_656_037u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(1_099_511_628_211);
    }
    h
}

/// `IsSafeLeaf`: a bare file name.
#[must_use]
pub fn is_safe_leaf(name: &str) -> bool {
    !name.is_empty() && !name.contains(['\\', '/', ':']) && name != "." && name != ".."
}

/// `BuildManifest`.
#[must_use]
pub fn build_manifest(entries: &[Entry]) -> String {
    let mut out = format!("[recovery]\r\nversion=1\r\ncount={}\r\n", entries.len());
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!(
            "\r\n[entry_{i}]\r\nsnapshot={}\r\nsource={}\r\nsaved_at={}\r\nsnapshot_size={}\r\nsnapshot_hash={}\r\nsource_mtime={}\r\nsource_size={}\r\n",
            e.snapshot, e.source, e.saved_at, e.snapshot_size, e.snapshot_hash, e.source_mtime, e.source_size
        ));
    }
    out
}

/// `ReadManifest` (the `GetPrivateProfile*` reads: a missing value is 0 or empty).
#[must_use]
pub fn read_manifest(text: &str) -> Vec<Entry> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            sections.push((name.to_ascii_lowercase(), Vec::new()));
        } else if let Some((k, v)) = l.split_once('=')
            && let Some(s) = sections.last_mut()
        {
            s.1.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let get = |s: &str, k: &str| -> String {
        sections.iter().find(|(n, _)| n == s).and_then(|(_, kv)| kv.iter().find(|(kk, _)| kk == k)).map(|(_, v)| v.clone()).unwrap_or_default()
    };
    let num = |s: &str, k: &str| get(s, k).parse::<u64>().unwrap_or(0);
    if num("recovery", "version") != u64::from(MANIFEST_VERSION) {
        return Vec::new();
    }
    let count = usize::try_from(num("recovery", "count")).unwrap_or(0).min(MAX_ENTRIES);
    (0..count)
        .filter_map(|i| {
            let s = format!("entry_{i}");
            let snapshot = get(&s, "snapshot");
            is_safe_leaf(&snapshot).then(|| Entry {
                snapshot,
                source: get(&s, "source"),
                saved_at: num(&s, "saved_at"),
                snapshot_size: num(&s, "snapshot_size"),
                snapshot_hash: num(&s, "snapshot_hash"),
                source_mtime: num(&s, "source_mtime"),
                source_size: num(&s, "source_size"),
            })
        })
        .collect()
}

/// The time as a Windows `FILETIME` (100 ns since 1601), as the manifest stores it.
#[must_use]
pub fn file_time_now() -> u64 {
    file_time(SystemTime::now())
}

fn file_time(t: SystemTime) -> u64 {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    t.duration_since(UNIX_EPOCH).map_or(0, |d| EPOCH_DIFF + (d.as_nanos() / 100) as u64)
}

/// `GetFileMetadata`: write time and size.
#[must_use]
pub fn file_meta(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().map(file_time).unwrap_or(0), m.len()))
}

/// `EOperationState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpState {
    Idle,
    Available,
    SnapshotCreated,
    RestorePending,
    Restored,
    DiscardPending,
    Discarded,
    Error,
}

struct Inner {
    bound: Option<PathBuf>,
    owns: bool,
    shared: bool,
    offer: bool,
    available: bool,
    entry: Option<Entry>,
    op: OpState,
    op_id: u64,
    next_op: u64,
    last_error: String,
    startup_error: String,
    sequence: u32,
    next_save: Instant,
    last_input: Instant,
    preserve: bool,
    claim: Option<File>,
}

/// One project's recovery session.
pub struct Recovery {
    inner: Mutex<Inner>,
    pub prefs: Mutex<RecoveryPrefs>,
}

impl Default for Recovery {
    fn default() -> Self {
        Self::new()
    }
}

/// What `recovery` answers besides `ok`.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub state: &'static str,
    pub operation_id: u64,
    pub pending: bool,
    pub enabled: bool,
    pub owns_session: bool,
    pub shared_session: bool,
    pub dirty: bool,
    pub can_snapshot: bool,
    pub can_restore: bool,
    pub can_discard: bool,
    pub snapshot_available: bool,
    pub source_newer: bool,
    pub saved_at: u64,
    pub source: String,
    pub snapshot: String,
    pub last_error: String,
}

impl Status {
    /// The fields in the C++ order.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "state": self.state,
            "operation_id": self.operation_id,
            "pending": self.pending,
            "enabled": self.enabled,
            "owns_session": self.owns_session,
            "shared_session": self.shared_session,
            "dirty": self.dirty,
            "can_snapshot": self.can_snapshot,
            "can_restore": self.can_restore,
            "can_discard": self.can_discard,
            "snapshot_available": self.snapshot_available,
            "source_newer": self.source_newer,
            "saved_at": self.saved_at,
            "source": self.source,
            "snapshot": self.snapshot,
            "last_error": self.last_error,
        })
    }
}

fn open_claim(path: &Path) -> std::io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ: another process can look, not claim
        o.share_mode(0x1);
    }
    o.open(path)
}

impl Recovery {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                bound: None,
                owns: false,
                shared: false,
                offer: false,
                available: false,
                entry: None,
                op: OpState::Idle,
                op_id: 0,
                next_op: 0,
                last_error: String::new(),
                startup_error: String::new(),
                sequence: 0,
                next_save: Instant::now(),
                last_input: Instant::now(),
                preserve: false,
                claim: None,
            }),
            prefs: Mutex::new(RecoveryPrefs::default()),
        }
    }

    /// `<project>\backup\recovery`.
    #[must_use]
    pub fn dir_of(root: &Path) -> PathBuf {
        root.join("backup").join("recovery")
    }

    /// `BindProject` / `TryAcquireSessionOwner`: claims the project's session; a marker left by
    /// the last session means it did not end cleanly, and its newest valid snapshot is offered.
    pub fn bind(&self, root: &Path) {
        let mut s = self.inner.lock();
        if s.bound.as_deref() == Some(root) && !s.shared {
            return;
        }
        let dir = Self::dir_of(root);
        s.bound = Some(root.to_path_buf());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            s.shared = true;
            s.startup_error = format!("cannot create recovery folder '{}' ({e})", dir.display());
            return;
        }
        let Ok(claim) = open_claim(&dir.join("session.claim")) else {
            s.shared = true;
            return;
        };
        s.claim = Some(claim);
        let marker = dir.join("session.lock");
        let unclean = marker.is_file();
        let text = format!("[session]\r\nversion=1\r\npid={}\r\nstarted_at={}\r\n", std::process::id(), file_time_now());
        if gz_editor::fsx::write_atomic(&marker, text.as_bytes()).is_err() {
            s.claim = None;
            s.shared = true;
            return;
        }
        s.owns = true;
        s.shared = false;
        remove_temporary(&dir);
        if unclean {
            match select_startup(&dir) {
                Some(e) => {
                    s.entry = Some(e);
                    s.offer = true;
                    s.available = true;
                    s.op = OpState::Available;
                }
                None => {
                    s.op = OpState::Error;
                    s.last_error = "the previous session ended unexpectedly, but no valid recovery snapshot was found".into();
                }
            }
        }
        s.next_save = Instant::now();
        s.last_input = Instant::now();
    }

    /// The author touched the editor: the autosave waits for the interaction delay.
    pub fn note_input(&self) {
        self.inner.lock().last_input = Instant::now();
    }

    /// `CanReplaceScene`.
    pub fn can_replace_scene(&self) -> Result<(), String> {
        if self.inner.lock().offer {
            return Err("restore or discard the pending crash recovery before replacing the scene".into());
        }
        Ok(())
    }

    /// `CanSerializeScene` without the editor's busy states.
    pub fn can_snapshot(&self, game_linked: bool) -> Result<(), String> {
        let s = self.inner.lock();
        if s.bound.is_none() || s.shared || !s.owns {
            return Err(if s.shared { "recovery is disabled because another live session owns this project" } else { "recovery session is not available" }.into());
        }
        if s.offer {
            return Err("resolve the startup recovery offer before creating another snapshot".into());
        }
        if !game_linked {
            return Err("the project has no valid game link".into());
        }
        Ok(())
    }

    /// Whether the autosave is due: unsaved work, the interval passed, the author idle.
    pub fn due(&self, dirty: bool) -> bool {
        let prefs = *self.prefs.lock();
        let mut s = self.inner.lock();
        let now = Instant::now();
        if !prefs.enabled || s.shared {
            return false;
        }
        if !dirty {
            s.next_save = now + prefs.interval();
            return false;
        }
        let idle = now.duration_since(s.last_input) >= Duration::from_secs(u64::from(prefs.interaction_delay_seconds.min(120)));
        now >= s.next_save && idle
    }

    /// A failed autosave tries again later.
    pub fn retry_later(&self) {
        self.inner.lock().next_save = Instant::now() + RETRY_DELAY;
    }

    /// `SaveNow`: writes `stream` as a snapshot of `source`.
    pub fn save_now(&self, stream: &[u8], source: &str) -> Result<(), String> {
        let root = self.inner.lock().bound.clone().ok_or("recovery session is not available")?;
        let dir = Self::dir_of(&root);
        if stream.is_empty() {
            return Err("recovery serializer produced an empty snapshot".into());
        }
        if let Err(why) = io::scene_kind(stream) {
            return Err(format!("recovery snapshot validation failed: {why}"));
        }
        let saved_at = file_time_now();
        let seq = {
            let mut s = self.inner.lock();
            s.sequence += 1;
            s.sequence
        };
        let name = format!("snapshot-{saved_at:016x}-{:08x}-{seq:04}.recovery", std::process::id());
        let path = dir.join(&name);
        gz_editor::fsx::write_atomic(&path, stream).map_err(|e| format!("cannot write recovery file '{}' ({e})", path.display()))?;
        let (source_mtime, source_size) = file_meta(Path::new(source)).unwrap_or((0, 0));
        let fresh = Entry {
            snapshot: name.clone(),
            source: source.to_owned(),
            saved_at,
            snapshot_size: stream.len() as u64,
            snapshot_hash: fnv1a(stream),
            source_mtime,
            source_size,
        };
        let manifest = dir.join("manifest.ltx");
        let previous = std::fs::read_to_string(&manifest).map(|t| read_manifest(&t)).unwrap_or_default();
        let mut kept = vec![fresh.clone()];
        for e in previous {
            if kept.len() >= MAX_ENTRIES || e.snapshot.eq_ignore_ascii_case(&name) {
                continue;
            }
            kept.push(e);
        }
        let retention = self.prefs.lock().retention.clamp(2, MAX_ENTRIES as u32) as usize;
        kept.truncate(retention);
        if let Err(e) = gz_editor::fsx::write_atomic(&manifest, build_manifest(&kept).as_bytes()) {
            let _ = std::fs::remove_file(&path);
            return Err(format!("cannot commit recovery metadata '{}' ({e})", manifest.display()));
        }
        delete_snapshots_not_in(&dir, &kept);
        let interval = self.prefs.lock().interval();
        let mut s = self.inner.lock();
        s.entry = Some(fresh);
        s.available = true;
        if s.op == OpState::Error {
            s.op = OpState::Idle;
        }
        s.last_error.clear();
        s.next_save = Instant::now() + interval;
        Ok(())
    }

    /// `SnapshotNow`: a snapshot on request; answers the operation id.
    pub fn snapshot_now(&self, stream: &[u8], source: &str, game_linked: bool) -> Result<u64, String> {
        self.can_snapshot(game_linked)?;
        let id = self.begin(OpState::SnapshotCreated);
        match self.save_now(stream, source) {
            Ok(()) => Ok(id),
            Err(e) => {
                let mut s = self.inner.lock();
                s.op = OpState::Error;
                s.last_error = e.clone();
                Err(e)
            }
        }
    }

    fn begin(&self, state: OpState) -> u64 {
        let mut s = self.inner.lock();
        s.next_op += 1;
        s.op_id = s.next_op;
        s.op = state;
        s.last_error.clear();
        s.op_id
    }

    fn check_offer(&self) -> Result<(), String> {
        let s = self.inner.lock();
        if !s.offer || !s.available {
            return Err("no startup recovery snapshot is available".into());
        }
        if matches!(s.op, OpState::RestorePending | OpState::DiscardPending) {
            return Err("a recovery operation is already pending".into());
        }
        if !s.owns || s.shared {
            return Err("this process does not own the project recovery session".into());
        }
        Ok(())
    }

    /// `CanRestoreStartup`: what the restore needs; the snapshot's path and source.
    pub fn begin_restore(&self, dirty: bool, game_linked: bool) -> Result<(u64, PathBuf, String), String> {
        self.check_offer()?;
        if !game_linked {
            return Err("the project has no valid game link".into());
        }
        if dirty {
            return Err("the current scene has unsaved changes; save or clear it before restoring startup recovery".into());
        }
        let (root, entry) = {
            let s = self.inner.lock();
            (s.bound.clone().ok_or("recovery session is not available")?, s.entry.clone().ok_or("no startup recovery snapshot is available")?)
        };
        let path = Self::dir_of(&root).join(&entry.snapshot);
        validate(&entry, &path).map_err(|why| format!("the recovery snapshot is no longer valid: {why}"))?;
        let id = self.begin(OpState::RestorePending);
        Ok((id, path, entry.source))
    }

    /// The restore ended.
    pub fn end_restore(&self, result: Result<(), String>) {
        let mut s = self.inner.lock();
        match result {
            Ok(()) => {
                s.offer = false;
                s.startup_error.clear();
                s.last_error.clear();
                s.op = OpState::Restored;
            }
            Err(e) => {
                s.startup_error = e.clone();
                s.last_error = e;
                s.op = OpState::Error;
                s.offer = true;
            }
        }
    }

    /// `RequestDiscard` and its work.
    pub fn discard(&self) -> Result<u64, String> {
        self.check_offer()?;
        let id = self.begin(OpState::DiscardPending);
        let root = self.inner.lock().bound.clone();
        let result = root.map_or(Ok(()), |r| delete_recovery_data(&Self::dir_of(&r)));
        let mut s = self.inner.lock();
        match result {
            Ok(()) => {
                s.offer = false;
                s.available = false;
                s.entry = None;
                s.startup_error.clear();
                s.last_error.clear();
                s.op = OpState::Discarded;
                Ok(id)
            }
            Err(e) => {
                s.startup_error = e.clone();
                s.last_error = e.clone();
                s.op = OpState::Error;
                Err(e)
            }
        }
    }

    /// `OnSceneSaved` / `OnSceneReplaced`: the snapshots belong to a scene that is gone.
    pub fn reset(&self) {
        let (root, ok) = {
            let s = self.inner.lock();
            (s.bound.clone(), s.owns && !s.shared && !s.offer)
        };
        if !ok {
            return;
        }
        if let Some(r) = root {
            let _ = delete_recovery_data(&Self::dir_of(&r));
        }
        let interval = self.prefs.lock().interval();
        let mut s = self.inner.lock();
        s.preserve = false;
        s.available = false;
        s.entry = None;
        s.startup_error.clear();
        if s.op_id == 0 && matches!(s.op, OpState::Available | OpState::Error) {
            s.op = OpState::Idle;
            s.last_error.clear();
        }
        s.next_save = Instant::now() + interval;
    }

    /// `EndProjectSession`: a clean end takes the recovery data and the marker away.
    pub fn end(&self, dirty: bool) {
        let mut s = self.inner.lock();
        let Some(root) = s.bound.take() else { return };
        let dir = Self::dir_of(&root);
        if s.owns && !s.offer && !s.preserve && !dirty && delete_recovery_data(&dir).is_ok() {
            let _ = std::fs::remove_file(dir.join("session.lock"));
        }
        s.claim = None;
        if s.owns {
            let _ = std::fs::remove_file(dir.join("session.claim"));
        }
        s.owns = false;
        s.shared = false;
        s.offer = false;
        s.available = false;
        s.entry = None;
        s.op = OpState::Idle;
        s.op_id = 0;
    }

    /// `GetStatus`.
    #[must_use]
    pub fn status(&self, dirty: bool, game_linked: bool) -> Status {
        let enabled = self.prefs.lock().enabled;
        let can_snapshot = self.can_snapshot(game_linked).is_ok();
        let can_discard = self.check_offer().is_ok();
        let s = self.inner.lock();
        let active = s.bound.is_some();
        let entry = s.entry.clone().filter(|_| active && s.available);
        let dir = s.bound.as_deref().map(Self::dir_of);
        let state = if !active {
            "unavailable"
        } else if s.shared {
            "shared_session"
        } else {
            match s.op {
                OpState::Available => "available",
                OpState::SnapshotCreated => "snapshot_created",
                OpState::RestorePending => "restore_pending",
                OpState::Restored => "restored",
                OpState::DiscardPending => "discard_pending",
                OpState::Discarded => "discarded",
                OpState::Error => "error",
                OpState::Idle => {
                    if s.available {
                        "snapshot_ready"
                    } else {
                        "idle"
                    }
                }
            }
        };
        let source_newer = entry.as_ref().is_some_and(|e| !e.source.is_empty() && file_meta(Path::new(&e.source)) != Some((e.source_mtime, e.source_size)));
        Status {
            state,
            operation_id: s.op_id,
            pending: active && matches!(s.op, OpState::RestorePending | OpState::DiscardPending),
            enabled,
            owns_session: active && s.owns,
            shared_session: active && s.shared,
            dirty,
            can_snapshot: active && can_snapshot,
            can_restore: active && can_discard && game_linked && !dirty,
            can_discard: active && can_discard,
            snapshot_available: entry.is_some(),
            source_newer,
            saved_at: entry.as_ref().map_or(0, |e| e.saved_at),
            source: entry.as_ref().map(|e| e.source.clone()).unwrap_or_default(),
            snapshot: match (&entry, &dir) {
                (Some(e), Some(d)) => d.join(&e.snapshot).display().to_string(),
                _ => String::new(),
            },
            last_error: if s.last_error.is_empty() { s.startup_error.clone() } else { s.last_error.clone() },
        }
    }
}

fn validate(e: &Entry, path: &Path) -> Result<(), String> {
    if e.snapshot_size == 0 || e.snapshot_hash == 0 {
        return Err("missing size or checksum metadata".into());
    }
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read recovery file '{}' ({err})", path.display()))?;
    if bytes.len() as u64 != e.snapshot_size || fnv1a(&bytes) != e.snapshot_hash {
        return Err("size or checksum mismatch".into());
    }
    io::scene_kind(&bytes).map(|_| ())
}

fn select_startup(dir: &Path) -> Option<Entry> {
    let text = std::fs::read_to_string(dir.join("manifest.ltx")).ok()?;
    read_manifest(&text).into_iter().find(|e| validate(e, &dir.join(&e.snapshot)).is_ok())
}

fn remove_temporary(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.contains(".tmp.") || n.ends_with(".gz~") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn delete_snapshots_not_in(dir: &Path, keep: &[Entry]) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.to_ascii_lowercase().ends_with(".recovery") && !keep.iter().any(|k| k.snapshot.eq_ignore_ascii_case(&n)) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// `DeleteRecoveryData`.
fn delete_recovery_data(dir: &Path) -> Result<(), String> {
    let manifest = dir.join("manifest.ltx");
    if manifest.exists() {
        gz_editor::fsx::write_atomic(&manifest, build_manifest(&[]).as_bytes()).map_err(|e| format!("cannot commit recovery metadata '{}' ({e})", manifest.display()))?;
    }
    delete_snapshots_not_in(dir, &[]);
    let _ = std::fs::remove_file(&manifest);
    remove_temporary(dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_round_trips() {
        let e = Entry { snapshot: "snapshot-1.recovery".into(), source: "C:\\a.level".into(), saved_at: 5, snapshot_size: 10, snapshot_hash: 11, source_mtime: 3, source_size: 4 };
        let text = build_manifest(std::slice::from_ref(&e));
        assert!(text.starts_with("[recovery]\r\nversion=1\r\ncount=1\r\n\r\n[entry_0]\r\n"));
        assert_eq!(read_manifest(&text), vec![e]);
        assert!(read_manifest("[recovery]\r\nversion=2\r\ncount=1\r\n").is_empty());
    }

    #[test]
    fn unsafe_names_are_skipped() {
        let e = Entry { snapshot: "..\\x".into(), ..Entry::default() };
        assert!(read_manifest(&build_manifest(&[e])).is_empty());
        assert_eq!(fnv1a(b""), 14_695_981_039_346_656_037);
    }
}
