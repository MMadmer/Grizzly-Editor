//! VirusTotal (MOD_CATALOG 11): the catalog's policy and the verdict it gives a report, the free
//! API's pacing, what a key looks like.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::http::Response;
use crate::ini;

/// The largest request VirusTotal takes. The multipart wrapper counts toward it, so a file within
/// a margin of the limit is as good as over it.
pub const UPLOAD_LIMIT: u64 = 650 << 20;
pub const WRAPPING: u64 = 64 << 10;

#[must_use]
pub fn too_large(size: u64) -> bool {
    size + WRAPPING > UPLOAD_LIMIT
}

/// The report page a person opens.
#[must_use]
pub fn report_page(sha256: &str) -> String {
    format!("https://www.virustotal.com/gui/file/{sha256}")
}

/// What a key looks like.
#[must_use]
pub fn plausible_key(key: &str) -> bool {
    (8..=128).contains(&key.len()) && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

pub(crate) fn credential_target(qa: bool) -> String {
    if qa {
        format!("GrizzlyEditor:VirusTotal:qa{}", crate::account::qa_credential_suffix())
    } else {
        "GrizzlyEditor:VirusTotal".to_owned()
    }
}

/// The policy of the catalog index's `[virustotal]` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtPolicy {
    /// Lower case.
    pub trusted: Vec<String>,
    pub block_trusted: u32,
    pub warn_trusted: u32,
    pub block_others: u32,
    pub warn_others: u32,
}

impl Default for VtPolicy {
    /// The catalog's template settings: used until the index is read.
    fn default() -> Self {
        const ENGINES: [&str; 21] = [
            "microsoft",
            "kaspersky",
            "eset-nod32",
            "bitdefender",
            "avast",
            "avg",
            "sophos",
            "symantec",
            "malwarebytes",
            "trendmicro",
            "mcafee",
            "f-secure",
            "drweb",
            "emsisoft",
            "gdata",
            "panda",
            "avira",
            "fortinet",
            "crowdstrike",
            "sentinelone",
            "paloalto",
        ];
        Self {
            trusted: ENGINES.iter().map(|e| (*e).to_owned()).collect(),
            block_trusted: 1,
            warn_trusted: 1,
            block_others: 5,
            warn_others: 2,
        }
    }
}

/// `strtoul`: the leading digits, 0 without any.
fn number(v: &str) -> u32 {
    let digits = v.trim_start().bytes().take_while(u8::is_ascii_digit).count();
    v.trim_start()
        .get(..digits)
        .and_then(|d| d.parse::<u64>().ok())
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

impl VtPolicy {
    /// The index's policy (`ReadVtPolicy`); the defaults for what it does not say.
    #[must_use]
    pub fn read(index: &str) -> Self {
        let mut out = Self::default();
        if let Some(value) = ini::value(index, "virustotal", "trusted") {
            out.trusted = value
                .split(',')
                .map(|e| e.trim_matches(' ').to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .collect();
        }
        for (key, field) in [
            ("block_trusted", &mut out.block_trusted),
            ("warn_trusted", &mut out.warn_trusted),
            ("block_others", &mut out.block_others),
            ("warn_others", &mut out.warn_others),
        ] {
            if let Some(v) = ini::value(index, "virustotal", key).filter(|v| !v.is_empty()) {
                *field = number(v);
            }
        }
        out
    }
}

/// One engine's word on a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtEngine {
    pub engine: String,
    /// malicious, suspicious, undetected, harmless, ...
    pub category: String,
}

/// What the catalog makes of a report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VtVerdict {
    /// clean, warn, block.
    pub verdict: String,
    pub engines: u32,
    pub trusted_malicious: u32,
    pub trusted_suspicious: u32,
    pub other_malicious: u32,
    pub flagged: Vec<String>,
}

/// The verdict of a report under a policy (`VtVerdict`).
#[must_use]
pub fn verdict(results: &[VtEngine], policy: &VtPolicy) -> VtVerdict {
    let mut v =
        VtVerdict { engines: u32::try_from(results.len()).unwrap_or(u32::MAX), ..VtVerdict::default() };
    for r in results {
        let engine = r.engine.to_ascii_lowercase();
        let trusted = policy.trusted.contains(&engine);
        if r.category == "malicious" {
            v.flagged.push(r.engine.clone());
            if trusted {
                v.trusted_malicious += 1;
            } else {
                v.other_malicious += 1;
            }
        } else if r.category == "suspicious" && trusted {
            v.flagged.push(r.engine.clone());
            v.trusted_suspicious += 1;
        }
    }
    // a lone no-name detection is noise; a trusted engine's word is not (11)
    v.verdict = if v.trusted_malicious >= policy.block_trusted || v.other_malicious >= policy.block_others {
        "block"
    } else if v.trusted_suspicious >= policy.warn_trusted || v.other_malicious >= policy.warn_others {
        "warn"
    } else {
        "clean"
    }
    .to_owned();
    v
}

/// What VirusTotal says about a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VtReport {
    /// The HTTP answer; 429 = over the free API's rate.
    pub status: u16,
    /// VirusTotal has the file.
    pub known: bool,
    /// Known, analysis not finished: no results yet.
    pub queued: bool,
    pub results: Vec<VtEngine>,
}

/// A lookup's answer read (`VtLookup`): `None` = not a report, the error is [`problem`]'s.
pub(crate) fn read_report(status: u16, body: &Value) -> Option<VtReport> {
    let attributes = body.get("data")?.get("attributes")?;
    let results: Vec<VtEngine> = attributes
        .get("last_analysis_results")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(engine, r)| VtEngine {
                    engine: engine.clone(),
                    category: crate::json::str_of(r, "category").to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    // an upload VirusTotal has not analysed yet is known without a verdict
    Some(VtReport { status, known: true, queued: results.is_empty(), results })
}

/// VirusTotal answers `{"error":{"code","message"}}`.
pub(crate) fn problem(r: &Response) -> String {
    if r.status == 0 {
        return r.error.clone();
    }
    if r.status == 401 {
        return "VirusTotal rejected the key".to_owned();
    }
    if r.status == 429 {
        return "VirusTotal's free API takes 4 requests a minute and 500 a day - try again later".to_owned();
    }
    let text = r
        .json()
        .and_then(|j| j.get("error").map(|e| crate::json::str_of(e, "message").to_owned()))
        .unwrap_or_default();
    if text.is_empty() {
        format!("VirusTotal answered {}", r.status)
    } else {
        format!("VirusTotal answered {}: {text}", r.status)
    }
}

/// The free API takes 4 requests a minute, and two jobs - a submission and a release - may ask at
/// once with the same key: every request takes the next slot, a little over 15 seconds after the
/// last one. Reserves the slot of a request asked at `now` and returns when it may go.
#[must_use]
pub fn reserve(next: &mut Option<Instant>, now: Instant, gap: Duration) -> Instant {
    let start = match *next {
        Some(n) if n > now => n,
        _ => now,
    };
    *next = Some(start + gap);
    start
}

/// The waits of the scan; a QA run shortens them (`GRIZZLY_VT_PACE_MS`, `GRIZZLY_VT_SCAN_WAIT_MS`,
/// `GRIZZLY_VT_RATE_WAIT_MS`, `GRIZZLY_VT_DIRECT_LIMIT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VtWaits {
    pub pace: Duration,
    pub scan_wait: Duration,
    pub rate_wait: Duration,
    /// Up to this many bytes a file goes straight to `/files`; larger ones to an upload address.
    pub direct_limit: u64,
}

impl VtWaits {
    #[must_use]
    pub fn from_env(qa: bool, env: &dyn Fn(&str) -> Option<String>) -> Self {
        let qa_number = |name: &str| {
            if qa { env(name).filter(|v| !v.is_empty()).map(|v| u64::from(number(&v))) } else { None }
        };
        let ms = |name: &str, fallback: u64| Duration::from_millis(qa_number(name).unwrap_or(fallback));
        Self {
            pace: ms("GRIZZLY_VT_PACE_MS", 16_000),
            scan_wait: ms("GRIZZLY_VT_SCAN_WAIT_MS", 10 * 60 * 1000),
            rate_wait: ms("GRIZZLY_VT_RATE_WAIT_MS", 20_000),
            direct_limit: qa_number("GRIZZLY_VT_DIRECT_LIMIT").unwrap_or(32 << 20),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engines(list: &[(&str, &str)]) -> Vec<VtEngine> {
        list.iter().map(|(e, c)| VtEngine { engine: (*e).to_owned(), category: (*c).to_owned() }).collect()
    }

    #[test]
    fn verdicts() {
        let p = VtPolicy::default();
        assert_eq!(verdict(&engines(&[("Microsoft", "undetected"), ("X", "harmless")]), &p).verdict, "clean");
        let one_noname = verdict(&engines(&[("NoName", "malicious")]), &p);
        assert_eq!(one_noname.verdict, "clean");
        assert_eq!(one_noname.flagged, vec!["NoName".to_owned()]);
        assert_eq!(verdict(&engines(&[("A", "malicious"), ("B", "malicious")]), &p).verdict, "warn");
        assert_eq!(verdict(&engines(&[("Kaspersky", "malicious")]), &p).verdict, "block");
        assert_eq!(verdict(&engines(&[("ESET-NOD32", "suspicious")]), &p).verdict, "warn");
        assert_eq!(verdict(&engines(&[("NoName", "suspicious")]), &p).verdict, "clean");
    }

    #[test]
    fn policy_from_index() {
        let index = "[virustotal]\ntrusted = Microsoft , QA-Engine,\nblock_trusted = 2\nwarn_others = 9x\n";
        let p = VtPolicy::read(index);
        assert_eq!(p.trusted, vec!["microsoft".to_owned(), "qa-engine".to_owned()]);
        assert_eq!((p.block_trusted, p.warn_trusted, p.block_others, p.warn_others), (2, 1, 5, 9));
        assert_eq!(VtPolicy::read("[catalog]\n"), VtPolicy::default());
    }

    #[test]
    fn pacing_with_shrunk_waits() {
        let gap = Duration::from_millis(100);
        let t0 = Instant::now();
        let mut next = None;
        // the first request goes at once, the next ones a gap apart, in the order they asked
        assert_eq!(reserve(&mut next, t0, gap), t0);
        assert_eq!(reserve(&mut next, t0, gap), t0 + gap);
        assert_eq!(reserve(&mut next, t0 + Duration::from_millis(10), gap), t0 + gap * 2);
        // a request long after the last one goes at once again
        let later = t0 + Duration::from_secs(5);
        assert_eq!(reserve(&mut next, later, gap), later);
        assert_eq!(next, Some(later + gap));

        let env = |name: &str| match name {
            "GRIZZLY_VT_PACE_MS" => Some("100".to_owned()),
            "GRIZZLY_VT_SCAN_WAIT_MS" => Some("4000".to_owned()),
            "GRIZZLY_VT_RATE_WAIT_MS" => Some("200".to_owned()),
            _ => None,
        };
        let qa = VtWaits::from_env(true, &env);
        assert_eq!(
            (qa.pace, qa.scan_wait, qa.rate_wait),
            (gap, Duration::from_secs(4), Duration::from_millis(200))
        );
        // outside a QA run the variables change nothing
        let real = VtWaits::from_env(false, &env);
        assert_eq!(
            (real.pace, real.scan_wait, real.direct_limit),
            (Duration::from_secs(16), Duration::from_secs(600), 32 << 20)
        );
    }

    #[test]
    fn keys() {
        assert!(plausible_key("0123456789abcdef"));
        assert!(!plausible_key("short"));
        assert!(!plausible_key("has space in it"));
        assert!(too_large(UPLOAD_LIMIT));
        assert!(!too_large(UPLOAD_LIMIT - WRAPPING));
    }
}
