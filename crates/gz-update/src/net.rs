//! The updater's own HTTP client: anonymous - never the account token, never an `Authorization`
//! header - with a `User-Agent` of `Grizzly-Editor/<version>`. Timeouts are per read, not per
//! file: a large archive over a slow line is fine, a line that went quiet is not.
//!
//! A ranged GET goes to the asset's address the first time; the storage address it is redirected
//! to is remembered and asked directly from then on, and forgotten when it refuses (a signed
//! address expires).

use std::time::Duration;

use gz_editor::Cancel;
use parking_lot::Mutex;
use reqwest::header::{ACCEPT, CONTENT_RANGE, LOCATION, RANGE};
use reqwest::{StatusCode, redirect};
use serde_json::Value;

use crate::version::Version;

/// The latest release as the API describes it.
#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub version: Option<Version>,
    pub notes: String,
    pub page: String,
    /// `Grizzly-Editor-<tag>.zip`.
    pub asset: Option<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub url: String,
}

/// What a request came to when it did not deliver.
#[derive(Debug)]
pub enum NetError {
    /// Stopped.
    Cancelled,
    /// No answer, or one cut short: worth another try.
    Network(String),
    /// An answer that will not change by asking again.
    Refused(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Stopped."),
            Self::Network(s) | Self::Refused(s) => f.write_str(s),
        }
    }
}

/// An answer to a ranged GET whose body is still to be read.
pub enum Ranged {
    /// `206`: the bytes asked for.
    Partial(reqwest::Response),
    /// `200`: this server ignores ranges and sends the whole archive.
    Whole(reqwest::Response),
}

pub struct Net {
    client: reqwest::Client,
    api: String,
    /// Under `-qa_github` only loopback addresses are spoken to.
    loopback_only: bool,
}

/// The asset being read, with the storage address its download was redirected to.
pub struct Source {
    pub url: String,
    storage: Mutex<Option<String>>,
}

impl Source {
    #[must_use]
    pub fn new(url: &str) -> Self {
        Self { url: url.to_owned(), storage: Mutex::new(None) }
    }

    /// The storage address in use, when there is one.
    #[must_use]
    pub fn storage(&self) -> Option<String> {
        self.storage.lock().clone()
    }
}

fn origin(url: &str) -> &str {
    let Some(scheme) = url.find("://") else { return "" };
    match url.get(scheme + 3..).and_then(|rest| rest.find('/')) {
        Some(slash) => url.get(..scheme + 3 + slash).unwrap_or(url),
        None => url,
    }
}

/// Whether an address is this computer's (the one test of it: `gz_base::web::is_loopback`).
#[must_use]
pub fn is_loopback(url: &str) -> bool {
    gz_base::web::is_loopback(url)
}

fn network(e: &reqwest::Error) -> NetError {
    let text = if e.is_timeout() {
        "the connection timed out".to_owned()
    } else if e.is_connect() {
        "no connection to the server".to_owned()
    } else if e.is_body() || e.is_request() || e.is_decode() {
        "the connection was lost".to_owned()
    } else {
        "the connection failed".to_owned()
    };
    NetError::Network(text)
}

impl Net {
    pub fn new(api: &str, loopback_only: bool) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(format!("Grizzly-Editor/{}", env!("CARGO_PKG_VERSION")))
            .tls_backend_native()
            .tls_version_min(reqwest::tls::Version::TLS_1_2)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            // redirects are followed by hand: the storage address is worth remembering
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|e| format!("the network client could not start: {e}"))?;
        Ok(Self { client, api: api.trim_end_matches('/').to_owned(), loopback_only })
    }

    fn allowed(&self, url: &str) -> Result<(), NetError> {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(NetError::Refused(format!("'{url}' is not an address the updater reads")));
        }
        if self.loopback_only && !is_loopback(url) {
            return Err(NetError::Refused("a QA run reads nothing from outside this computer".to_owned()));
        }
        Ok(())
    }

    async fn send(
        &self,
        rq: reqwest::RequestBuilder,
        cancel: &Cancel,
    ) -> Result<reqwest::Response, NetError> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(NetError::Cancelled),
            r = rq.send() => r.map_err(|e| network(&e)),
        }
    }

    /// `GET /repos/<repo>/releases/latest`; `None` when the repository has no release.
    pub async fn latest(&self, repo: &str, cancel: &Cancel) -> Result<Option<Release>, NetError> {
        let url = format!("{}/repos/{repo}/releases/latest", self.api);
        self.allowed(&url)?;
        let rq = self.client.get(&url).header(ACCEPT, "application/vnd.github+json");
        let resp = self.send(rq, cancel).await?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(NetError::Network("GitHub asks to wait before the next look".to_owned()));
        }
        if !status.is_success() {
            return Err(NetError::Network(format!(
                "GitHub answered the release check with {}",
                status.as_u16()
            )));
        }
        let body = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(NetError::Cancelled),
            b = resp.bytes() => b.map_err(|e| network(&e))?,
        };
        let json: Value = serde_json::from_slice(&body).map_err(|_| {
            NetError::Network("GitHub's answer about the release could not be read".to_owned())
        })?;
        Ok(Some(release_of(&json)))
    }

    /// Asks for `[start, end)` of the asset. A redirect is followed and its target remembered; a
    /// remembered target that refuses is forgotten and the asset's own address asked again.
    pub async fn range(
        &self,
        src: &Source,
        start: u64,
        end: u64,
        cancel: &Cancel,
    ) -> Result<Ranged, NetError> {
        let range = format!("bytes={start}-{}", end.saturating_sub(1));
        let remembered = src.storage();
        let mut url = remembered.clone().unwrap_or_else(|| src.url.clone());
        let mut from_memory = remembered.is_some();
        for _ in 0..8 {
            self.allowed(&url)?;
            let resp = self.send(self.client.get(&url).header(RANGE, &range), cancel).await?;
            let status = resp.status();
            if status.is_redirection() {
                let loc = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| NetError::Network("the download was redirected nowhere".to_owned()))?;
                let next = if loc.starts_with("http://") || loc.starts_with("https://") {
                    loc.to_owned()
                } else if loc.starts_with('/') {
                    format!("{}{loc}", origin(&url))
                } else {
                    return Err(NetError::Network(
                        "the download was redirected to an address it cannot read".to_owned(),
                    ));
                };
                *src.storage.lock() = Some(next.clone());
                url = next;
                from_memory = false;
                continue;
            }
            if from_memory && status.is_client_error() && status != StatusCode::RANGE_NOT_SATISFIABLE {
                // the remembered storage address expired: start from the asset again
                *src.storage.lock() = None;
                url = src.url.clone();
                from_memory = false;
                continue;
            }
            return match status {
                StatusCode::PARTIAL_CONTENT => {
                    let first = resp
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("bytes "))
                        .and_then(|v| v.split('-').next())
                        .and_then(|v| v.trim().parse::<u64>().ok());
                    if first == Some(start) {
                        Ok(Ranged::Partial(resp))
                    } else {
                        Err(NetError::Network("the server sent other bytes than were asked for".to_owned()))
                    }
                }
                StatusCode::OK => Ok(Ranged::Whole(resp)),
                s if s.is_server_error() || s == StatusCode::TOO_MANY_REQUESTS => {
                    Err(NetError::Network(format!("the download server answered {}", s.as_u16())))
                }
                s => Err(NetError::Refused(format!("the download server answered {}", s.as_u16()))),
            };
        }
        Err(NetError::Network("the download was redirected too many times".to_owned()))
    }
}

/// The next piece of a body, or its end.
pub async fn chunk(
    resp: &mut reqwest::Response,
    cancel: &Cancel,
) -> Result<Option<impl AsRef<[u8]>>, NetError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(NetError::Cancelled),
        c = resp.chunk() => c.map_err(|e| network(&e)),
    }
}

fn release_of(json: &Value) -> Release {
    let text = |k: &str| json.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let tag = text("tag_name");
    let version = Version::parse(&tag);
    let want = format!("Grizzly-Editor-{tag}.zip");
    let asset = json.get("assets").and_then(Value::as_array).and_then(|list| {
        list.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(want.as_str())).and_then(|a| {
            Some(Asset {
                id: a.get("id")?.as_u64()?,
                name: want.clone(),
                size: a.get("size")?.as_u64()?,
                url: a.get("browser_download_url")?.as_str()?.to_owned(),
            })
        })
    });
    Release { tag, version, notes: text("body"), page: text("html_url"), asset }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_names_its_archive() {
        let json = serde_json::json!({
            "tag_name": "0.7.1", "body": "- Fixes", "html_url": "https://github.com/MMadmer/Grizzly-Editor/releases/tag/0.7.1",
            "assets": [
                { "id": 1, "name": "Grizzly-Editor-0.7.1-symbols.zip", "size": 5, "browser_download_url": "https://x/s" },
                { "id": 2, "name": "Grizzly-Editor-0.7.1.zip", "size": 9, "browser_download_url": "https://x/a" }
            ]
        });
        let r = release_of(&json);
        assert_eq!(r.version.map(|v| v.to_string()).as_deref(), Some("0.7.1"));
        assert_eq!(
            r.asset,
            Some(Asset {
                id: 2,
                name: "Grizzly-Editor-0.7.1.zip".into(),
                size: 9,
                url: "https://x/a".into()
            })
        );
        assert!(release_of(&serde_json::json!({ "tag_name": "v0.7.1" })).version.is_none());
    }

    #[test]
    fn only_this_computer_is_loopback() {
        assert!(is_loopback("http://127.0.0.1:5000/x"));
        assert!(is_loopback("http://localhost:80"));
        assert!(!is_loopback("http://127.0.0.1.example.com:5000/x"));
        assert!(!is_loopback("https://api.github.com/repos"));
        assert!(!is_loopback("http://localhostile:5/"));
        assert!(!is_loopback("http://127.0.0.1:1@api.github.com/x"));
    }
}
