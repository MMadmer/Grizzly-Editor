//! [`Github`]: what the network side keeps while the editor runs - the clients, the addresses,
//! the GitHub CLI's last answer, VirusTotal's pace. Made on first use, in `editor.services`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gz_editor::{Cancel, Editor, Executors, Log};
use parking_lot::Mutex;
use reqwest::Method;
use reqwest::header::{ACCEPT, HeaderName};
use zeroize::Zeroizing;

use crate::account::{Account, AccountSource, credential_target, plausible_token, token_variables};
use crate::endpoints::Endpoints;
use crate::http::{self, Body, Clients, Kind, Request, Response};
use crate::secret::Secret;
use crate::vt::{self, VtReport, VtWaits};
use crate::{CANCELLED, cred};

/// The GitHub CLI's answer is kept this long; an older one is still given and read anew behind it.
const CLI_FRESH: Duration = Duration::from_secs(60);

#[derive(Default)]
struct CliCache {
    token: Option<Secret>,
    /// An answer has come at least once.
    read: bool,
    at: Option<Instant>,
}

/// The network side of the session. Lives in `editor.services`.
pub struct Github {
    endpoints: Endpoints,
    waits: VtWaits,
    clients: Result<Clients, String>,
    exec: Executors,
    log: Log,
    cli: Mutex<CliCache>,
    /// One `gh` at a time.
    cli_reading: tokio::sync::Mutex<()>,
    /// The next free slot of VirusTotal's free API, shared by every job.
    vt_next: Mutex<Option<Instant>>,
}

impl Github {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(|| {
            let qa = editor.flags.qa_github;
            Self {
                endpoints: Endpoints::from_process(qa),
                waits: VtWaits::from_env(qa, &|name| std::env::var(name).ok()),
                clients: Clients::build(),
                exec: editor.exec.clone(),
                log: editor.log.clone(),
                cli: Mutex::new(CliCache::default()),
                cli_reading: tokio::sync::Mutex::new(()),
                vt_next: Mutex::new(None),
            }
        })
    }

    #[must_use]
    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// `-qa_github`: every address points at local mocks.
    #[must_use]
    pub fn qa(&self) -> bool {
        self.endpoints.qa
    }

    #[must_use]
    pub fn vt_waits(&self) -> VtWaits {
        self.waits
    }

    pub(crate) fn exec(&self) -> &Executors {
        &self.exec
    }

    pub(crate) async fn send(&self, rq: Request, cancel: &Cancel) -> Response {
        http::send(&self.clients, &self.log, rq, cancel).await
    }

    //--------------------------------------------------------------------------
    // the account
    //--------------------------------------------------------------------------

    /// Environment first (an explicit override), then the token the editor saved, then the GitHub
    /// CLI. A QA run reads `GRIZZLY_GITHUB_TOKEN` and its own credential alone, so a real token
    /// never travels to a host that is not GitHub.
    pub async fn resolve_account(self: &Arc<Self>) -> Account {
        let qa = self.qa();
        for name in token_variables(qa) {
            let value = Zeroizing::new(std::env::var(name).unwrap_or_default());
            if plausible_token(&value) {
                return Account {
                    source: AccountSource::Environment,
                    token: Secret::from(value),
                    detail: (*name).to_owned(),
                };
            }
        }
        let saved = self.exec.io(move || cred::read(credential_target(qa))).await;
        if let Some(token) = saved.filter(|t| plausible_token(t)) {
            return Account {
                source: AccountSource::Saved,
                token: Secret::from(token),
                detail: String::new(),
            };
        }
        if !qa && let Some(token) = self.cli_token().await {
            return Account { source: AccountSource::GithubCli, token, detail: String::new() };
        }
        Account::default()
    }

    /// Starts reading the GitHub CLI's answer, so that the first dialog that asks for the account
    /// has it.
    pub fn warm_account(self: &Arc<Self>) {
        if self.qa() {
            return;
        }
        let me = self.clone();
        self.exec.spawn(async move {
            me.refresh_cli().await;
        });
    }

    /// Stores a token in the Credential Manager; it goes nowhere else.
    pub async fn save_token(&self, token: Secret) -> Result<(), String> {
        if !plausible_token(token.expose()) {
            return Err("this does not look like a GitHub token".to_owned());
        }
        let qa = self.qa();
        self.exec
            .io(move || cred::write(credential_target(qa), token.expose()))
            .await
            .map_err(|e| format!("Windows refused to store the token ({e})"))
    }

    pub async fn forget_token(&self) {
        let qa = self.qa();
        self.exec.io(move || cred::delete(credential_target(qa))).await;
    }

    // The GitHub CLI answers by starting a process, most of a second with an on-access scanner, and
    // every release dialog asks for the account as it opens. So its answer is kept for a minute;
    // an older one is still given, and read anew behind it.
    async fn cli_token(self: &Arc<Self>) -> Option<Secret> {
        let (read, stale, cached) = {
            let c = self.cli.lock();
            (c.read, c.at.is_none_or(|at| at.elapsed() > CLI_FRESH), c.token.clone())
        };
        if !read {
            // the very first answer has to be waited for: there is nothing to say without it
            return self.refresh_cli().await;
        }
        if stale {
            self.warm_account();
        }
        cached
    }

    async fn refresh_cli(&self) -> Option<Secret> {
        let Ok(_one) = self.cli_reading.try_lock() else {
            // somebody reads it already: wait for that answer
            let _wait = self.cli_reading.lock().await;
            return self.cli.lock().token.clone();
        };
        let token = read_github_cli_token().await;
        let mut c = self.cli.lock();
        c.token = token.clone();
        c.read = true;
        c.at = Some(Instant::now());
        token
    }

    //--------------------------------------------------------------------------
    // VirusTotal's requests
    //--------------------------------------------------------------------------

    /// `GRIZZLY_VT_KEY` (or, outside a QA run, `VT_API_KEY`) first, then the key the editor saved
    /// in the Credential Manager. Like the GitHub token it never reaches a log, a file or an answer.
    pub async fn read_vt_key(&self) -> Option<Secret> {
        let qa = self.qa();
        let mut key = Zeroizing::new(std::env::var("GRIZZLY_VT_KEY").unwrap_or_default());
        if key.is_empty() && !qa {
            key = Zeroizing::new(std::env::var("VT_API_KEY").unwrap_or_default());
        }
        if vt::plausible_key(&key) {
            return Some(Secret::from(key));
        }
        let saved = self.exec.io(move || cred::read(vt::credential_target(qa))).await?;
        vt::plausible_key(&saved).then(|| Secret::from(saved))
    }

    pub async fn save_vt_key(&self, key: Secret) -> Result<(), String> {
        if !vt::plausible_key(key.expose()) {
            return Err("this does not look like a VirusTotal API key".to_owned());
        }
        let qa = self.qa();
        self.exec
            .io(move || cred::write(vt::credential_target(qa), key.expose()))
            .await
            .map_err(|e| format!("Windows refused to store the key ({e})"))
    }

    pub async fn forget_vt_key(&self) {
        let qa = self.qa();
        self.exec.io(move || cred::delete(vt::credential_target(qa))).await;
    }

    /// Waits for this request's turn at the free API's pace, whichever job asks.
    async fn vt_pace(&self, cancel: &Cancel) {
        let at = vt::reserve(&mut self.vt_next.lock(), Instant::now(), self.waits.pace);
        tokio::select! {
            () = cancel.cancelled() => {}
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(at)) => {}
        }
    }

    fn vt_request(&self, method: Method, url: String, key: &str) -> Request {
        Request::new(Kind::Api, method, url)
            .header(ACCEPT, "application/json")
            .secret_header(HeaderName::from_static("x-apikey"), key)
    }

    /// What VirusTotal says about a file, by its SHA-256. `Err((status, why))`; 429 = over the rate.
    pub async fn vt_lookup(
        &self,
        sha256: &str,
        key: &str,
        cancel: &Cancel,
    ) -> Result<VtReport, (u16, String)> {
        self.vt_pace(cancel).await;
        let r = self
            .send(self.vt_request(Method::GET, format!("{}/files/{sha256}", self.endpoints.vt), key), cancel)
            .await;
        if cancel.is_cancelled() {
            return Err((r.status, CANCELLED.to_owned()));
        }
        if r.status == 404 {
            return Ok(VtReport { status: 404, ..VtReport::default() });
        }
        let report =
            if r.status == 200 { r.json().and_then(|j| vt::read_report(r.status, &j)) } else { None };
        report.ok_or_else(|| (r.status, vt::problem(&r)))
    }

    /// Sends a file for analysis (an upload address for one over 32 MB). VirusTotal shares uploaded
    /// files with the security industry; results follow within minutes.
    pub async fn vt_upload(&self, path: &Path, key: &str, cancel: &Cancel) -> Result<(), (u16, String)> {
        let size = tokio::fs::metadata(path)
            .await
            .map(|m| m.len())
            .map_err(|_| (0, format!("cannot read {}", path.display())))?;
        if vt::too_large(size) {
            return Err((0, "the file is larger than the 650 MB VirusTotal takes".to_owned()));
        }
        // up to 32 MB straight to /files; a larger file to an address VirusTotal hands out for it
        let mut url = format!("{}/files", self.endpoints.vt);
        if size + vt::WRAPPING > self.waits.direct_limit {
            self.vt_pace(cancel).await;
            let r = self
                .send(
                    self.vt_request(Method::GET, format!("{}/files/upload_url", self.endpoints.vt), key),
                    cancel,
                )
                .await;
            if cancel.is_cancelled() {
                return Err((r.status, CANCELLED.to_owned()));
            }
            let named = if r.status == 200 {
                r.json().and_then(|j| j.get("data").and_then(|d| d.as_str()).map(str::to_owned))
            } else {
                None
            };
            url = named.ok_or_else(|| (r.status, vt::problem(&r)))?;
            // the key goes wherever that address points, and it came out of a response body
            if !self.endpoints.vt_url_trusted(&url) {
                return Err((0, "VirusTotal named an upload address that is not VirusTotal's.".to_owned()));
            }
        }

        let mut random = [0u8; 12];
        getrandom::fill(&mut random).map_err(|e| (0, format!("no random numbers for the upload: {e}")))?;
        let boundary = format!("GrizzlyEditor{}", hex::encode(random));
        let name: String = path
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .chars()
                    .map(|c| if c == '"' || u32::from(c) < 0x20 { '_' } else { c })
                    .collect()
            })
            .unwrap_or_default();
        let prefix = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        );
        let suffix = format!("\r\n--{boundary}--\r\n");
        let mut rq = self.vt_request(Method::POST, url, key);
        rq.kind = Kind::Upload;
        rq.body = Body::File {
            path: path.to_path_buf(),
            prefix: prefix.into_bytes(),
            suffix: suffix.into_bytes(),
            content_type: format!("multipart/form-data; boundary={boundary}"),
            progress: None,
        };
        self.vt_pace(cancel).await;
        let r = self.send(rq, cancel).await;
        if cancel.is_cancelled() {
            return Err((r.status, CANCELLED.to_owned()));
        }
        let accepted = r.status == 200
            && r.json()
                .is_some_and(|j| j.get("data").is_some_and(|d| !crate::json::str_of(d, "id").is_empty()));
        if accepted { Ok(()) } else { Err((r.status, vt::problem(&r))) }
    }
}

/// `gh auth token` prints the token the GitHub CLI is signed in with.
async fn read_github_cli_token() -> Option<Secret> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(["auth", "token", "--hostname", "github.com"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
    }
    let child = cmd.spawn().ok()?;
    // one short line: it fits the pipe, so gh never blocks on us
    let out = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output()).await.ok()?.ok()?;
    let stdout = Zeroizing::new(out.stdout);
    if !out.status.success() {
        return None;
    }
    let head = stdout.get(..stdout.len().min(511))?;
    let text = std::str::from_utf8(head).ok()?.trim_end_matches(['\n', '\r', ' ']);
    plausible_token(text).then(|| Secret::new(text.to_owned()))
}
