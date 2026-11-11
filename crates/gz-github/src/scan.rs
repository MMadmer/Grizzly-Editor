//! The author's VirusTotal scan of a release (MOD_CATALOG 11, `EditorVirusTotal`). The catalog
//! only looks packages up by their SHA-256: it refuses a submission whose package VirusTotal does
//! not know and holds back such a release of a listed module. Getting the packages known is the
//! author's job, with their own free key - Submit to Mod Browser scans before it sends anything,
//! Publish Release before a release of a module the Mod Browser lists goes out. Both run this one
//! routine; the requests keep to the free API's pace ([`crate::Github`]).
//!
//! A package ends `clean` / `warn` / `block` (the report, judged by the catalog's policy),
//! `analysing` (VirusTotal has the file, its report is not out yet), `unknown` (VirusTotal does
//! not know it - a scan: nor could it be uploaded), `too_large`, `unavailable` (no answer: the
//! network, the rate, the key refused) or `no_key`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_editor::Cancel;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::service::Github;
use crate::vt::{self, VtPolicy, VtReport, VtVerdict};

/// The most decisive state first: a scan takes the first one any of its packages has.
pub const ORDER: [&str; 8] =
    ["no_key", "block", "too_large", "unknown", "unavailable", "analysing", "warn", "clean"];

/// A package of the release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// This computer's copy, uploaded only when its hash is `sha256`; empty = none.
    pub path: PathBuf,
    pub size: u64,
    /// What the descriptor pins, lowercase hex.
    pub sha256: String,
}

/// What became of one package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub package: Package,
    pub state: String,
    pub verdict: VtVerdict,
    pub error: String,
}

/// A scan or a lookup of every package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    /// `none` (nothing asked), `scanning`, `cancelled`, or the state of the package that decides.
    pub state: String,
    pub packages: Vec<ScanResult>,
}

impl Default for Scan {
    fn default() -> Self {
        Self { state: "none".to_owned(), packages: Vec::new() }
    }
}

/// Reports a stage of the work.
pub type Stage<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// The catalog would refuse the release or hold it back: nothing may go out.
#[must_use]
pub fn refuses(state: &str) -> bool {
    matches!(state, "no_key" | "block" | "too_large" | "unknown")
}

/// The package whose state is the scan's.
#[must_use]
pub fn deciding(scan: &Scan) -> Option<&ScanResult> {
    scan.packages.iter().find(|p| p.state == scan.state)
}

/// One sentence about a package, without the final period.
#[must_use]
pub fn describe(r: &ScanResult) -> String {
    let name = &r.package.name;
    let flagged = r.verdict.flagged.join(", ");
    match r.state.as_str() {
        "clean" => format!("VirusTotal: {name} is clean ({} engines)", r.verdict.engines),
        "warn" => format!("VirusTotal warns about {name}: flagged by {flagged}"),
        "block" => format!("VirusTotal blocks {name}: flagged by {flagged}"),
        "analysing" => format!("VirusTotal is still analysing {name}"),
        "too_large" => format!("{name} is larger than the 650 MB VirusTotal takes"),
        "unavailable" => format!("VirusTotal did not answer about {name}: {}", r.error),
        "no_key" => format!("No VirusTotal key to scan {name}"),
        "unknown" if r.error.is_empty() => format!("VirusTotal does not know {name}"),
        "unknown" => format!("VirusTotal does not know {name}, and {}", r.error),
        _ => format!("{name} was not scanned"),
    }
}

/// `{"state":...,"packages":[{"name","sha256","size","state","engines",...}]}`.
#[must_use]
pub fn json(scan: &Scan) -> Value {
    let packages: Vec<Value> = scan
        .packages
        .iter()
        .map(|r| {
            json!({
                "name": r.package.name,
                "sha256": r.package.sha256,
                "size": r.package.size,
                "state": r.state,
                "engines": r.verdict.engines,
                "trusted_malicious": r.verdict.trusted_malicious,
                "trusted_suspicious": r.verdict.trusted_suspicious,
                "other_malicious": r.verdict.other_malicious,
                "flagged": r.verdict.flagged,
                "url": vt::report_page(&r.package.sha256),
                "error": r.error,
            })
        })
        .collect();
    json!({ "state": scan.state, "packages": packages })
}

fn begin(packages: &[Package]) -> Scan {
    Scan {
        state: "none".to_owned(),
        packages: packages
            .iter()
            .map(|p| ScanResult { package: p.clone(), ..ScanResult::default() })
            .collect(),
    }
}

// the key refused or its quota used up: asking about another package would fail the same way
fn key_problem(status: u16) -> bool {
    matches!(status, 401 | 403 | 429)
}

fn fail(out: &mut Scan, i: usize, status: u16, err: &str) {
    for (p, r) in out.packages.iter_mut().enumerate() {
        if p == i || (key_problem(status) && r.state.is_empty()) {
            r.state = "unavailable".to_owned();
            r.error = err.to_owned();
        }
    }
}

fn judge(r: &mut ScanResult, report: &VtReport, policy: &VtPolicy) {
    r.verdict = vt::verdict(&report.results, policy);
    r.state = r.verdict.verdict.clone();
    r.error.clear();
}

fn settle(out: &mut Scan, cancelled: bool) {
    if cancelled {
        out.state = "cancelled".to_owned();
        return;
    }
    let first = out.packages.iter().filter_map(|p| ORDER.iter().position(|s| *s == p.state)).min();
    out.state = first.and_then(|k| ORDER.get(k)).map_or("none", |s| *s).to_owned();
}

/// A local file's SHA-256, lowercase hex, and its size; `None` when it does not read or the job is
/// stopped. Blocking: call on the IO pool.
#[must_use]
pub fn file_sha256(path: &Path, cancel: &Cancel) -> Option<(String, u64)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        if cancel.is_cancelled() {
            return None;
        }
        let got = f.read(&mut buf).ok()?;
        if got == 0 {
            break;
        }
        hash.update(buf.get(..got)?);
        size += got as u64;
    }
    Some((hex::encode(hash.finalize()), size))
}

impl Github {
    /// A request refused for the rate is waited out a few times - the key may be busy elsewhere,
    /// the game's Mods window takes one too; after that the quota counts as used up.
    async fn look(
        &self,
        sha256: &str,
        key: &str,
        cancel: &Cancel,
        stage: Stage<'_>,
    ) -> Result<VtReport, (u16, String)> {
        let mut attempt = 0;
        loop {
            let r = self.vt_lookup(sha256, key, cancel).await;
            match &r {
                Err((429, _)) if attempt < 3 && !cancel.is_cancelled() => {}
                _ => return r,
            }
            stage("Waiting for VirusTotal's rate limit");
            crate::http::pause(cancel, self.vt_waits().rate_wait.as_millis() as u64).await;
            attempt += 1;
        }
    }

    async fn upload_within_rate(
        &self,
        path: &Path,
        key: &str,
        cancel: &Cancel,
        stage: Stage<'_>,
    ) -> Result<(), (u16, String)> {
        let mut attempt = 0;
        loop {
            let r = self.vt_upload(path, key, cancel).await;
            match &r {
                Err((429, _)) if attempt < 3 && !cancel.is_cancelled() => {}
                _ => return r,
            }
            stage("Waiting for VirusTotal's rate limit");
            crate::http::pause(cancel, self.vt_waits().rate_wait.as_millis() as u64).await;
            attempt += 1;
        }
    }

    /// What VirusTotal knows now; nothing is uploaded, nothing waited for.
    pub async fn lookup(
        &self,
        packages: &[Package],
        key: &str,
        policy: &VtPolicy,
        cancel: &Cancel,
        stage: Stage<'_>,
    ) -> Scan {
        let mut out = begin(packages);
        for i in 0..out.packages.len() {
            if cancel.is_cancelled() {
                break;
            }
            let Some(r) = out.packages.get(i) else { break };
            if key.is_empty() {
                if let Some(r) = out.packages.get_mut(i) {
                    r.state = "no_key".to_owned();
                }
                continue;
            }
            if !r.state.is_empty() {
                continue;
            }
            let (name, sha, size) = (r.package.name.clone(), r.package.sha256.clone(), r.package.size);
            stage(&format!("Looking up {name} on VirusTotal"));
            let looked = self.look(&sha, key, cancel, stage).await;
            match looked {
                Err((status, err)) => fail(&mut out, i, status, &err),
                Ok(report) => {
                    let Some(r) = out.packages.get_mut(i) else { break };
                    if report.known && !report.queued {
                        judge(r, &report, policy);
                    } else if report.known {
                        r.state = "analysing".to_owned();
                    } else {
                        r.state = if vt::too_large(size) { "too_large" } else { "unknown" }.to_owned();
                    }
                }
            }
        }
        settle(&mut out, cancel.is_cancelled());
        out
    }

    /// Every package looked up, the ones VirusTotal does not know uploaded from this computer,
    /// and their reports waited for - ten minutes at most.
    pub async fn scan(
        &self,
        packages: &[Package],
        key: &str,
        policy: &VtPolicy,
        cancel: &Cancel,
        stage: Stage<'_>,
    ) -> Scan {
        let mut out = begin(packages);
        if key.is_empty() {
            for r in &mut out.packages {
                r.state = "no_key".to_owned();
            }
            settle(&mut out, false);
            return out;
        }

        // 1. what VirusTotal knows; what it does not know goes up from this computer
        let mut waiting: Vec<usize> = Vec::new();
        for i in 0..out.packages.len() {
            if cancel.is_cancelled() {
                break;
            }
            let Some(r) = out.packages.get(i) else { break };
            if !r.state.is_empty() {
                continue;
            }
            let package = r.package.clone();
            stage(&format!("Looking up {} on VirusTotal", package.name));
            let report = match self.look(&package.sha256, key, cancel, stage).await {
                Ok(report) => report,
                Err((status, err)) => {
                    fail(&mut out, i, status, &err);
                    continue;
                }
            };
            if report.known && !report.queued {
                if let Some(r) = out.packages.get_mut(i) {
                    judge(r, &report, policy);
                }
                continue;
            }
            if !report.known {
                if vt::too_large(package.size) {
                    if let Some(r) = out.packages.get_mut(i) {
                        r.state = "too_large".to_owned();
                    }
                    continue;
                }
                // only the very bytes the descriptor pins go up
                stage(&format!("Hashing {}", package.name));
                let (path, c) = (package.path.clone(), cancel.clone());
                let hashed = if path.as_os_str().is_empty() {
                    None
                } else {
                    self.exec().io(move || file_sha256(&path, &c)).await
                };
                if hashed.is_none_or(|(sha, _)| sha != package.sha256) {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Some(r) = out.packages.get_mut(i) {
                        r.state = "unknown".to_owned();
                        r.error = "this computer has no copy of it to upload".to_owned();
                    }
                    continue;
                }
                stage(&format!("Uploading {} to VirusTotal", package.name));
                if let Err((status, err)) = self.upload_within_rate(&package.path, key, cancel, stage).await {
                    fail(&mut out, i, status, &err);
                    continue;
                }
            }
            waiting.push(i);
        }

        // 2. The reports, usually out within minutes. A file VirusTotal has never seen answers
        // "not found" until its first analysis ends, so that is waited out like a report without
        // results.
        let start = Instant::now();
        let limit = self.vt_waits().scan_wait;
        while !waiting.is_empty() && !cancel.is_cancelled() && start.elapsed() < limit {
            let mut w = 0;
            while w < waiting.len() && !cancel.is_cancelled() {
                let Some(&i) = waiting.get(w) else { break };
                let Some(r) = out.packages.get(i) else { break };
                if r.state.is_empty() {
                    let (name, sha) = (r.package.name.clone(), r.package.sha256.clone());
                    let seconds = start.elapsed().as_secs();
                    stage(&format!(
                        "Waiting for VirusTotal's report on {name} ({}:{:02})",
                        seconds / 60,
                        seconds % 60
                    ));
                    match self.look(&sha, key, cancel, stage).await {
                        Ok(report) => {
                            if let Some(r) = out.packages.get_mut(i) {
                                if report.known && !report.queued {
                                    judge(r, &report, policy);
                                } else {
                                    r.error.clear();
                                }
                            }
                        }
                        // a lost answer is asked again next round; a refused key or a used-up
                        // quota ends the wait
                        Err((status, err)) if key_problem(status) => fail(&mut out, i, status, &err),
                        Err((_, err)) => {
                            if let Some(r) = out.packages.get_mut(i) {
                                r.error = err;
                            }
                        }
                    }
                }
                if out.packages.get(i).is_some_and(|r| r.state.is_empty()) {
                    w += 1;
                } else {
                    waiting.remove(w);
                }
            }
        }
        // no report in time: VirusTotal is still at it, or stopped answering
        for i in waiting {
            if let Some(r) = out.packages.get_mut(i)
                && r.state.is_empty()
            {
                r.state = if r.error.is_empty() { "analysing" } else { "unavailable" }.to_owned();
            }
        }
        settle(&mut out, cancel.is_cancelled());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, state: &str) -> ScanResult {
        ScanResult {
            package: Package { name: name.to_owned(), ..Package::default() },
            state: state.to_owned(),
            ..ScanResult::default()
        }
    }

    #[test]
    fn the_most_decisive_state_wins() {
        let mut s = Scan {
            state: String::new(),
            packages: vec![result("a", "clean"), result("b", "analysing"), result("c", "warn")],
        };
        settle(&mut s, false);
        assert_eq!(s.state, "analysing");
        assert_eq!(deciding(&s).map(|r| r.package.name.as_str()), Some("b"));
        s.packages.push(result("d", "no_key"));
        settle(&mut s, false);
        assert_eq!(s.state, "no_key");
        settle(&mut s, true);
        assert_eq!(s.state, "cancelled");
        let mut empty = Scan::default();
        settle(&mut empty, false);
        assert_eq!(empty.state, "none");
        assert!(refuses("unknown") && refuses("block") && !refuses("analysing") && !refuses("unavailable"));
    }

    #[test]
    fn a_refused_key_ends_the_others() {
        let mut s = begin(&[Package::default(), Package::default(), Package::default()]);
        if let Some(r) = s.packages.get_mut(0) {
            r.state = "clean".to_owned();
        }
        fail(&mut s, 1, 401, "VirusTotal rejected the key");
        let states: Vec<&str> = s.packages.iter().map(|r| r.state.as_str()).collect();
        assert_eq!(states, ["clean", "unavailable", "unavailable"]);
        let mut t = begin(&[Package::default(), Package::default()]);
        fail(&mut t, 0, 500, "VirusTotal answered 500");
        assert_eq!(t.packages.get(1).map(|r| r.state.as_str()), Some(""));
    }

    #[test]
    fn sentences() {
        let mut r = result("od-1.0.0.zip", "unknown");
        assert_eq!(describe(&r), "VirusTotal does not know od-1.0.0.zip");
        r.error = "this computer has no copy of it to upload".to_owned();
        assert_eq!(
            describe(&r),
            "VirusTotal does not know od-1.0.0.zip, and this computer has no copy of it to upload"
        );
        r.state = "clean".to_owned();
        r.verdict.engines = 70;
        assert_eq!(describe(&r), "VirusTotal: od-1.0.0.zip is clean (70 engines)");
        let j = json(&Scan { state: "clean".to_owned(), packages: vec![r] });
        assert_eq!(j["packages"][0]["url"], "https://www.virustotal.com/gui/file/");
        assert_eq!(j["state"], "clean");
    }
}
