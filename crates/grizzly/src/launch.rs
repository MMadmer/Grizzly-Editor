//! One editor per home. The editor that holds a home keeps `<home>\editor.lock` (its process id
//! and the port of its control endpoint). A second start of the editor on the same home with a
//! project to open hands the project to that editor (`project.open path= from_launch=true`: the
//! running one asks about unsaved work in its window), lets it bring its window to the front and
//! ends; without a project it only brings that window forward. A commandlet is an instance of its
//! own and never hands over.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

/// The lock's name in the home.
pub const LOCK: &str = "editor.lock";

/// The editor a lock names: its process and its port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub port: u16,
}

/// Reads a lock's text: `pid=<n>` and `port=<n>` lines.
#[must_use]
pub fn parse_lock(text: &str) -> Option<Holder> {
    let mut pid = None;
    let mut port = None;
    for line in text.lines() {
        match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("pid", v)) => pid = v.parse().ok(),
            Some(("port", v)) => port = v.parse().ok(),
            _ => {}
        }
    }
    Some(Holder { pid: pid?, port: port? })
}

/// The editor that holds `home` and still runs, when it is not this process.
#[must_use]
pub fn holder(home: &Path) -> Option<Holder> {
    let text = std::fs::read_to_string(home.join(LOCK)).ok()?;
    let h = parse_lock(&text)?;
    (h.pid != std::process::id() && gz_win::process_alive(h.pid)).then_some(h)
}

/// One request to the editor on `port`, its answer.
fn ask(port: u16, request: &Value, wait: Duration) -> Option<Value> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(wait)).ok()?;
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).ok()?;
    serde_json::from_str(answer.trim()).ok()
}

/// Hands `project` (or nothing) to the editor that holds `home`. `true`: it took it, and this
/// start is done.
pub fn hand_over(home: &Path, project: Option<&Path>, log: &gz_log::Log) -> bool {
    let Some(h) = holder(home) else { return false };
    let alive = ask(h.port, &json!({ "cmd": "ping" }), Duration::from_secs(5))
        .is_some_and(|r| r.get("ok").and_then(Value::as_bool) == Some(true));
    if !alive {
        return false;
    }
    gz_win::allow_foreground(h.pid);
    if let Some(project) = project {
        let path = std::path::absolute(project).unwrap_or_else(|_| project.to_path_buf());
        let request =
            json!({ "cmd": "project.open", "path": path.display().to_string(), "from_launch": true });
        let answer = ask(h.port, &request, Duration::from_secs(120));
        let why = answer
            .as_ref()
            .filter(|a| a.get("ok").and_then(Value::as_bool) != Some(true))
            .and_then(|a| a.get("error").and_then(Value::as_str).map(str::to_owned));
        match why {
            Some(why) => log.warn(format!("the running editor did not open {}: {why}", path.display())),
            None => log.info(format!("{} handed to the running editor (process {})", path.display(), h.pid)),
        }
    }
    let _ = ask(h.port, &json!({ "cmd": "app.window", "action": "focus" }), Duration::from_secs(10));
    true
}

/// This editor's hold on its home: the lock goes when the editor ends.
pub struct Lock(PathBuf);

impl Lock {
    /// Writes the lock for this process listening on `port`.
    pub fn take(home: &Path, port: u16) -> Option<Lock> {
        let path = home.join(LOCK);
        let text = format!("pid={}\r\nport={port}\r\n", std::process::id());
        gz_editor::fsx::write_atomic(&path, text.as_bytes()).ok()?;
        Some(Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // only our own: another editor may have taken the home since
        let ours = std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|t| parse_lock(&t))
            .is_some_and(|h| h.pid == std::process::id());
        if ours {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_names_its_editor_and_a_stale_one_names_none() {
        assert_eq!(parse_lock("pid=12\r\nport=28917\r\n"), Some(Holder { pid: 12, port: 28917 }));
        assert_eq!(parse_lock("pid=12"), None);
        let dir = std::env::temp_dir().join(format!("grizzly-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // this process's own lock is no other editor
        let lock = Lock::take(&dir, 1).unwrap();
        assert_eq!(holder(&dir), None);
        drop(lock);
        assert!(!dir.join(LOCK).exists());
        // a process that ended holds nothing
        std::fs::write(dir.join(LOCK), "pid=0\r\nport=1\r\n").unwrap();
        assert_eq!(holder(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
