//! GitHub's REST API: the device sign-in, the publication of a packaged release, what a
//! repository already holds (listing, editing, deleting), public files fetched the way the game
//! fetches them, and the Mod Browser catalog's issues.
//!
//! A release is published the way the game expects to find one (game repo
//! `docs/dead-air/MOD_UPDATES.md`, section 2): descriptor, file index and packages go into a
//! DRAFT, every asset is checked against what was packaged, and only then the draft becomes the
//! latest release. The game reads the latest release alone, so it never sees one that is half
//! there.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use gz_editor::Cancel;
use reqwest::Method;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderName};
use serde_json::{Value, json};

use crate::CANCELLED;
use crate::catalog::{self, CatalogIssue, CatalogRequest, CatalogWithdrawal, IssueKind, issue_title};
use crate::endpoints::{encode_ref, url_encode};
use crate::http::{self, Body, Kind, Progress, Request, Response};
use crate::ini;
use crate::json::{array_of, bool_of, str_of, u64_of};
use crate::secret::Secret;
use crate::service::Github;

const UPLOAD_ATTEMPTS: u32 = 3;
/// Uploads in flight at once.
const UPLOADS_AT_ONCE: usize = 3;
/// A release under a tag that was never used is served at once; one under a tag that is used
/// again is not - see the warning at the end of `publish`. Long enough to tell the two apart.
const SERVE_ATTEMPTS: u32 = 15;

fn cancelled<T>() -> Result<T, String> {
    Err(CANCELLED.to_owned())
}

/// What GitHub said, in its own words where it has any.
fn problem(r: &Response) -> String {
    if r.status == 0 {
        return r.error.clone();
    }
    let mut text = String::new();
    if let Some(j) = r.json() {
        text = str_of(&j, "message").to_owned();
        // an entry is an object with code and field, or the sentence itself
        if let Some(first) = array_of(&j, "errors").first() {
            let mut detail =
                first.as_str().map_or_else(|| str_of(first, "message").to_owned(), str::to_owned);
            if detail.is_empty() && !str_of(first, "code").is_empty() {
                detail = format!("{} {}", str_of(first, "field"), str_of(first, "code"));
            }
            if !detail.is_empty() {
                text.push_str(&format!(" ({detail})"));
            }
        }
    }
    if text.is_empty() {
        format!("GitHub answered {}", r.status)
    } else {
        format!("GitHub answered {}: {text}", r.status)
    }
}

/// The refusals that mean the same thing whatever was asked.
fn access_problem(r: &Response, repo: &str) -> String {
    if r.status == 401 {
        return "GitHub rejected the sign-in. Sign in again.".to_owned();
    }
    if (300..400).contains(&r.status) {
        return format!("Repository {repo} has moved. Set its current name in Mod Settings.");
    }
    problem(r)
}

/// Requests with the account's token on board.
struct Api<'a> {
    gh: &'a Github,
    token: &'a Secret,
    cancel: &'a Cancel,
}

impl Api<'_> {
    fn request(&self, kind: Kind, method: Method, url: String) -> Request {
        let bearer = zeroize::Zeroizing::new(format!("Bearer {}", self.token.expose()));
        Request::new(kind, method, url)
            .header(ACCEPT, "application/vnd.github+json")
            .header(HeaderName::from_static("x-github-api-version"), "2022-11-28")
            .secret_header(AUTHORIZATION, &bearer)
    }

    async fn get(&self, url: String) -> Response {
        self.gh.send(self.request(Kind::Api, Method::GET, url), self.cancel).await
    }

    async fn delete(&self, url: String) -> Response {
        self.gh.send(self.request(Kind::Api, Method::DELETE, url), self.cancel).await
    }

    async fn send(&self, method: Method, url: String, body: &Value) -> Response {
        self.gh.send(self.request(Kind::Api, method, url).json(body), self.cancel).await
    }

    async fn upload(&self, url: String, path: PathBuf, progress: Progress) -> Response {
        let mut rq = self.request(Kind::Upload, Method::POST, url);
        rq.body = Body::File {
            path,
            prefix: Vec::new(),
            suffix: Vec::new(),
            content_type: "application/octet-stream".to_owned(),
            progress: Some(progress),
        };
        self.gh.send(rq, self.cancel).await
    }
}

//------------------------------------------------------------------------------
// device sign-in
//------------------------------------------------------------------------------

/// The code the author types on GitHub.
#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub device_code: Secret,
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds between polls.
    pub interval: u32,
    pub expires_in: u32,
}

/// What a poll of the device sign-in came to.
#[derive(Debug)]
pub enum DevicePoll {
    /// Signed in.
    Token(Secret),
    /// Not yet: poll again after the code's interval.
    Pending,
}

//------------------------------------------------------------------------------
// publication
//------------------------------------------------------------------------------

/// A file of the release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asset {
    /// On disk.
    pub path: PathBuf,
    /// The asset name, exactly as the descriptor spells it.
    pub name: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// What to publish.
#[derive(Clone, Debug)]
pub struct PublishRequest {
    /// owner/repo.
    pub repo: String,
    pub module_id: String,
    pub version: String,
    pub tag: String,
    pub title: String,
    pub notes: String,
    pub prerelease: bool,
    pub latest: bool,
    pub token: Secret,
    /// Packages, index, descriptor - the descriptor last: it vouches for the rest.
    pub assets: Vec<Asset>,
    /// `<id>.update.ltx`, for the final check of what the game sees.
    pub descriptor: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishResult {
    /// The release page.
    pub url: String,
    pub warnings: String,
    /// The game's own address already returns the new descriptor.
    pub served: bool,
}

/// What a publication tells its job while it runs.
pub trait PublishSink: Send + Sync {
    fn stage(&self, text: &str);
    /// Bytes of one asset sent so far.
    fn upload(&self, asset: &str, sent: u64, total: u64);
}

struct RemoteAsset {
    id: u64,
    name: String,
    state: String,
}

fn read_assets(release_or_list: &Value) -> Vec<RemoteAsset> {
    let list = if release_or_list.is_array() {
        release_or_list.as_array().map_or(&[][..], Vec::as_slice)
    } else {
        array_of(release_or_list, "assets")
    };
    list.iter()
        .map(|a| RemoteAsset {
            id: u64_of(a, "id"),
            name: str_of(a, "name").to_owned(),
            state: str_of(a, "state").to_owned(),
        })
        .filter(|a| a.id != 0)
        .collect()
}

/// Names a release of this module + version puts on GitHub.
fn own_asset_name(rq: &PublishRequest, name: &str) -> bool {
    let stem = format!("{}-{}.", rq.module_id, rq.version);
    name.eq_ignore_ascii_case(&format!("{}.update.ltx", rq.module_id))
        || (name.len() > stem.len() && name.get(..stem.len()).is_some_and(|h| h.eq_ignore_ascii_case(&stem)))
}

/// The stored asset against what was packaged; empty = identical.
fn asset_mismatch(stored: &Value, local: &Asset) -> String {
    if local.name != str_of(stored, "name") {
        return format!("stored under the name '{}'", str_of(stored, "name"));
    }
    if u64_of(stored, "size") != local.size {
        return format!("{} byte(s) stored, {} packaged", u64_of(stored, "size"), local.size);
    }
    if str_of(stored, "state") != "uploaded" {
        return format!("state '{}'", str_of(stored, "state"));
    }
    // older GitHub Enterprise has no digest; size and state still stand
    let digest = str_of(stored, "digest");
    if !digest.is_empty() && !digest.eq_ignore_ascii_case(&format!("sha256:{}", local.sha256)) {
        return "the SHA-256 differs".to_owned();
    }
    String::new()
}

fn ends_with_ci(s: &str, tail: &str) -> bool {
    s.len() >= tail.len() && s.get(s.len() - tail.len()..).is_some_and(|t| t.eq_ignore_ascii_case(tail))
}

/// `v<version>` and the fresh tags of the same version (`v<version>-r<N>`).
#[must_use]
pub fn is_version_tag(tag: &str, version_tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix(version_tag) else { return false };
    if rest.is_empty() {
        return true;
    }
    rest.strip_prefix("-r").is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

//------------------------------------------------------------------------------
// what is already out there
//------------------------------------------------------------------------------

/// A release of the repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub id: u64,
    pub tag: String,
    pub title: String,
    /// The release body, Markdown.
    pub notes: String,
    pub url: String,
    /// `2026-09-19 12:34` UTC; empty for a draft.
    pub published: String,
    pub draft: bool,
    pub prerelease: bool,
    /// The one the game reads.
    pub latest: bool,
    pub assets: Vec<String>,
    /// All assets together.
    pub bytes: u64,
}

/// What a deletion did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeleteResult {
    pub tag: String,
    /// It was one: no tag to remove.
    pub draft: bool,
    pub tag_removed: bool,
    /// Why the tag is still there, when it is.
    pub tag_note: String,
}

/// `2026-09-19T12:34:56Z` -> `2026-09-19 12:34`.
#[must_use]
pub fn short_time(iso: &str) -> String {
    if iso.len() < 16 || iso.as_bytes().get(10) != Some(&b'T') {
        return iso.to_owned();
    }
    format!("{} {}", iso.get(..10).unwrap_or(""), iso.get(11..16).unwrap_or(""))
}

impl Github {
    fn api<'a>(&'a self, token: &'a Secret, cancel: &'a Cancel) -> Api<'a> {
        Api { gh: self, token, cancel }
    }

    fn repo_url(&self, repo: &str) -> String {
        format!("{}/repos/{repo}", self.endpoints().api)
    }

    //--------------------------------------------------------------------------
    // device sign-in
    //--------------------------------------------------------------------------

    async fn post_form(&self, url: String, form: String, cancel: &Cancel) -> Response {
        let rq = Request::new(Kind::Api, Method::POST, url).header(ACCEPT, "application/json").form(form);
        self.send(rq, cancel).await
    }

    /// Starts the device sign-in: the code the author confirms on GitHub.
    pub async fn device_begin(&self, cancel: &Cancel) -> Result<DeviceCode, String> {
        let ep = self.endpoints();
        // public_repo: a release the game can download lives in a public repository
        let r = self
            .post_form(
                format!("{}/login/device/code", ep.web),
                format!("client_id={}&scope=public_repo", url_encode(&ep.client_id)),
                cancel,
            )
            .await;
        let j = if r.status == 200 { r.json() } else { None };
        let Some(j) =
            j.filter(|j| !str_of(j, "device_code").is_empty() && !str_of(j, "user_code").is_empty())
        else {
            return Err(if r.status != 0 {
                format!("GitHub refused to start the sign-in ({})", r.status)
            } else {
                r.error
            });
        };
        let mut uri = str_of(&j, "verification_uri").to_owned();
        if uri.is_empty() {
            uri = format!("{}/login/device", ep.web);
        }
        let interval = u64_of(&j, "interval");
        let expires = u64_of(&j, "expires_in");
        Ok(DeviceCode {
            device_code: Secret::new(str_of(&j, "device_code").to_owned()),
            user_code: str_of(&j, "user_code").to_owned(),
            verification_uri: uri,
            interval: if interval != 0 { u32::try_from(interval.min(60)).unwrap_or(60) } else { 5 },
            expires_in: if expires != 0 { u32::try_from(expires.min(3600)).unwrap_or(3600) } else { 900 },
        })
    }

    /// One poll of the device sign-in. A lost poll is not a lost sign-in: the code stays good
    /// until it expires.
    pub async fn device_poll(&self, code: &mut DeviceCode, cancel: &Cancel) -> Result<DevicePoll, String> {
        let ep = self.endpoints();
        let form = format!(
            "client_id={}&device_code={}&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
            url_encode(&ep.client_id),
            url_encode(code.device_code.expose())
        );
        let r = self.post_form(format!("{}/login/oauth/access_token", ep.web), form, cancel).await;
        if cancel.is_cancelled() {
            return cancelled();
        }
        if r.status == 0 {
            return Ok(DevicePoll::Pending);
        }
        let Some(j) = (if r.status == 200 { r.json() } else { None }) else {
            return Err(format!("GitHub answered {} to the sign-in", r.status));
        };
        let token = str_of(&j, "access_token");
        if crate::account::plausible_token(token) {
            return Ok(DevicePoll::Token(Secret::new(token.to_owned())));
        }
        match str_of(&j, "error") {
            "authorization_pending" => Ok(DevicePoll::Pending),
            "slow_down" => {
                let asked = u32::try_from(u64_of(&j, "interval")).unwrap_or(60);
                code.interval = asked.max(code.interval + 5).min(60);
                Ok(DevicePoll::Pending)
            }
            "expired_token" => Err("The code expired. Start the sign-in again.".to_owned()),
            "access_denied" => Err("The sign-in was declined on GitHub.".to_owned()),
            "" => Err("GitHub refused the sign-in: no reason given".to_owned()),
            other => Err(format!("GitHub refused the sign-in: {other}")),
        }
    }

    //--------------------------------------------------------------------------
    // publication
    //--------------------------------------------------------------------------

    /// Publishes a packaged release. Leaves a draft behind when it fails or is stopped before the
    /// last step; the next attempt for the same tag picks that draft up again.
    pub async fn publish(
        &self,
        rq: &PublishRequest,
        cancel: &Cancel,
        sink: &dyn PublishSink,
    ) -> Result<PublishResult, String> {
        let api = self.api(&rq.token, cancel);
        let repo_url = self.repo_url(&rq.repo);
        let descriptor_name = format!("{}.update.ltx", rq.module_id);
        let becomes_latest = rq.latest && !rq.prerelease;
        let check = || if cancel.is_cancelled() { cancelled() } else { Ok(()) };

        // 1. the repository, as this account sees it
        sink.stage("Checking the repository");
        let r = api.get(repo_url.clone()).await;
        check()?;
        let repo = match r.status {
            401 => return Err("GitHub rejected the sign-in. Sign in again.".to_owned()),
            404 => {
                return Err(format!("Repository {} was not found, or this account cannot see it.", rq.repo));
            }
            300..=399 => {
                return Err(format!(
                    "Repository {} has moved. Set its current name in Mod Settings.",
                    rq.repo
                ));
            }
            200 => r.json().ok_or_else(|| problem(&r))?,
            _ => return Err(problem(&r)),
        };
        let is_private = bool_of(&repo, "private");
        if bool_of(&repo, "archived") {
            return Err(format!("Repository {} is archived: it accepts no releases.", rq.repo));
        }
        if let Some(perm) = repo.get("permissions")
            && perm.get("push").is_some()
            && !bool_of(perm, "push")
        {
            return Err(format!("This account cannot write to {}.", rq.repo));
        }

        // 2. The game reads the latest release alone, so a module this release does not carry
        // would vanish for its players the moment this one becomes latest.
        if becomes_latest {
            let r = api.get(format!("{repo_url}/releases/latest")).await;
            check()?;
            match (r.status, r.json()) {
                (200, Some(j)) => {
                    if let Some(other) = read_assets(&j).iter().find(|a| {
                        ends_with_ci(&a.name, ".update.ltx") && !a.name.eq_ignore_ascii_case(&descriptor_name)
                    }) {
                        return Err(format!(
                            "The latest release of {} serves another module ({}). This release would hide it from its players. Use a repository of its own for each module.",
                            rq.repo, other.name
                        ));
                    }
                }
                (404, _) => {}
                _ => return Err(problem(&r)),
            }
        }

        // 3. a published release is never touched again
        let r = api.get(format!("{repo_url}/releases/tags/{}", url_encode(&rq.tag))).await;
        check()?;
        if r.status == 200 {
            return Err(format!(
                "Release {} is already published in {}, and a published release is never overwritten. Raise the version - or, to put other files out under this one, delete that release in Releases first: the game then offers the new one to players who have it as a fix.",
                rq.tag, rq.repo
            ));
        }
        if r.status != 404 {
            return Err(problem(&r));
        }

        // 4. a draft an earlier attempt left for this tag is picked up again
        let r = api.get(format!("{repo_url}/releases?per_page=100")).await;
        check()?;
        let list = match r.json() {
            Some(j) if r.status == 200 && j.is_array() => j,
            _ => return Err(problem(&r)),
        };
        let items = list.as_array().map_or(&[][..], Vec::as_slice);
        let mut release_id = 0u64;
        let mut leftovers = Vec::new();
        if let Some(draft) = items.iter().find(|i| bool_of(i, "draft") && str_of(i, "tag_name") == rq.tag) {
            release_id = u64_of(draft, "id");
            leftovers = read_assets(draft);
        }
        // The editor's tags for one version are v<version> and v<version>-r<N>: the suffix is how a
        // version goes out again under a fresh address, not a way around "this version is out".
        let version_tag = format!("v{}", rq.version);
        if let Some(out) = items.iter().find(|i| {
            !bool_of(i, "draft")
                && str_of(i, "tag_name") != rq.tag
                && is_version_tag(str_of(i, "tag_name"), &version_tag)
        }) {
            return Err(format!(
                "Version {} is already published in {} as {}, and a published release is never overwritten. Raise the version - or, to put other files out under this one, delete that release in Releases first: the game then offers the new one to players who have it as a fix.",
                rq.version,
                rq.repo,
                str_of(out, "tag_name")
            ));
        }

        // 5. the draft
        sink.stage(if release_id != 0 { "Reusing the draft release" } else { "Creating a draft release" });
        let body = json!({ "tag_name": rq.tag, "name": rq.title, "body": rq.notes, "draft": true, "prerelease": rq.prerelease });
        let r = if release_id != 0 {
            api.send(Method::PATCH, format!("{repo_url}/releases/{release_id}"), &body).await
        } else {
            api.send(Method::POST, format!("{repo_url}/releases"), &body).await
        };
        check()?;
        if r.status == 403 || r.status == 404 {
            return Err(format!(
                "This sign-in cannot publish to {}. A fine-grained token needs the Contents permission set to Read and write; a classic one needs public_repo.",
                rq.repo
            ));
        }
        let draft = match r.json() {
            Some(j) if (r.status == 200 || r.status == 201) && u64_of(&j, "id") != 0 => j,
            _ => return Err(problem(&r)),
        };
        let release_id = u64_of(&draft, "id");
        let mut upload_url = str_of(&draft, "upload_url").to_owned();
        if let Some(brace) = upload_url.find('{') {
            upload_url.truncate(brace);
        }
        if !self.endpoints().upload_url_trusted(&upload_url) {
            return Err("GitHub named an upload address that is not GitHub's.".to_owned());
        }
        let release_url = format!("{repo_url}/releases/{release_id}");

        // 6. what the earlier attempt left under this release's names
        for a in leftovers.iter().filter(|a| own_asset_name(rq, &a.name) || a.state != "uploaded") {
            let r = api.delete(format!("{repo_url}/releases/assets/{}", a.id)).await;
            check()?;
            if r.status != 204 && r.status != 404 {
                return Err(problem(&r));
            }
        }

        // 7. the assets: the packages and the index side by side, the descriptor last
        let (last, first) = match rq.assets.split_last() {
            Some((last, first)) => (Some(last), first),
            None => (None, &[][..]),
        };
        // no closure builds these futures: a job's future has to stay Send for every lifetime
        let mut queue = first.iter();
        let mut uploads = futures::stream::FuturesUnordered::new();
        for a in queue.by_ref().take(UPLOADS_AT_ONCE) {
            uploads.push(self.upload_asset(&api, &repo_url, &release_url, &upload_url, a, sink));
        }
        while let Some(done) = uploads.next().await {
            done?;
            if let Some(a) = queue.next() {
                uploads.push(self.upload_asset(&api, &repo_url, &release_url, &upload_url, a, sink));
            }
        }
        drop(uploads);
        if let Some(descriptor) = last {
            self.upload_asset(&api, &repo_url, &release_url, &upload_url, descriptor, sink).await?;
        }

        // 8. from here on players can see it
        sink.stage("Publishing the release");
        let make_latest = if becomes_latest { "true" } else { "false" };
        let body = json!({ "draft": false, "prerelease": rq.prerelease, "make_latest": make_latest });
        let r = api.send(Method::PATCH, release_url.clone(), &body).await;
        let Some(published) = (if r.status == 200 { r.json() } else { None }) else {
            // the request may have landed before the connection was cut
            return Err(if cancel.is_cancelled() {
                "Cancelled while the release was being published. Check the release page: it may be out."
                    .to_owned()
            } else {
                problem(&r)
            });
        };
        let mut result =
            PublishResult { url: str_of(&published, "html_url").to_owned(), ..PublishResult::default() };
        if result.url.is_empty() {
            result.url = self.endpoints().releases_page(&rq.repo);
        }

        // 9. the address the game itself asks
        if !becomes_latest {
            return Ok(result);
        }
        if is_private {
            result.warnings = "The repository is private. The game downloads releases anonymously, so players receive nothing until the repository is public.".to_owned();
            return Ok(result);
        }
        sink.stage("Checking what the game will see");
        let local = tokio::fs::read(&rq.descriptor).await.unwrap_or_default();
        let address =
            format!("{}/{}/releases/latest/download/{descriptor_name}", self.endpoints().web, rq.repo);
        for attempt in 0..SERVE_ATTEMPTS {
            if result.served || cancel.is_cancelled() {
                break;
            }
            if attempt > 0 {
                http::pause(cancel, self.endpoints().serve_pause_ms).await;
            }
            let served = self.send(Request::new(Kind::Public, Method::GET, address.clone()), cancel).await;
            result.served = served.status == 200 && !local.is_empty() && served.body == local;
        }
        if !result.served {
            // Seen live: a release deleted and published again under the same tag keeps its
            // download address, and GitHub goes on answering that address with the deleted files
            // for minutes.
            result.warnings = "GitHub still answers the address the game asks with the previous files. That is GitHub's own cache - it happens when a tag is used a second time - and it passes within minutes; the game offers this release after that.".to_owned();
        }
        Ok(result)
    }

    /// One asset into the draft, tried again after a lost connection, a 5xx or a name clash, and
    /// checked against what was packaged.
    async fn upload_asset(
        &self,
        api: &Api<'_>,
        repo_url: &str,
        release_url: &str,
        upload_url: &str,
        asset: &Asset,
        sink: &dyn PublishSink,
    ) -> Result<(), String> {
        let cancel = api.cancel;
        sink.stage(&format!("Uploading {}", asset.name));
        let mut failure = String::new();
        for attempt in 0..UPLOAD_ATTEMPTS {
            if attempt > 0 {
                // a broken upload leaves a stub that owns the name
                http::pause(cancel, self.endpoints().retry_pause_ms).await;
                let r = api.get(format!("{release_url}/assets?per_page=100")).await;
                if cancel.is_cancelled() {
                    return cancelled();
                }
                let now = if r.status == 200 {
                    r.json().map(|j| read_assets(&j)).unwrap_or_default()
                } else {
                    Vec::new()
                };
                for stub in now.iter().filter(|a| a.name.eq_ignore_ascii_case(&asset.name)) {
                    let _ = api.delete(format!("{repo_url}/releases/assets/{}", stub.id)).await;
                }
                if cancel.is_cancelled() {
                    return cancelled();
                }
            }
            sink.upload(&asset.name, 0, asset.size);
            // the body stream only records what it sent; the sink, borrowed for this call, hears
            // of it from the wait below
            let sent = Arc::new(AtomicU64::new(0));
            let progress: Progress = {
                let sent = sent.clone();
                Arc::new(move |done: u64, _total: u64| sent.store(done, Ordering::Relaxed))
            };
            let upload = api.upload(
                format!("{upload_url}?name={}", url_encode(&asset.name)),
                asset.path.clone(),
                progress,
            );
            let r = report_progress(upload, &sent, &asset.name, asset.size, sink).await;
            if cancel.is_cancelled() {
                return cancelled();
            }
            if r.status == 201
                && let Some(j) = r.json()
            {
                let mismatch = asset_mismatch(&j, asset);
                if !mismatch.is_empty() {
                    return Err(format!(
                        "GitHub did not store {} as it was packaged: {mismatch}. Nothing was published.",
                        asset.name
                    ));
                }
                sink.upload(&asset.name, asset.size, asset.size);
                return Ok(());
            }
            failure = problem(&r);
            // a refusal will not change on a second try; a lost connection, a 5xx or a name clash may
            if (400..500).contains(&r.status) && !matches!(r.status, 422 | 408 | 429) {
                break;
            }
        }
        Err(format!("Uploading {} failed: {failure}. Nothing was published.", asset.name))
    }

    //--------------------------------------------------------------------------
    // listing and deleting
    //--------------------------------------------------------------------------

    /// Newest first, drafts included - those are what a failed attempt leaves behind.
    pub async fn list_releases(
        &self,
        repo: &str,
        token: &Secret,
        cancel: &Cancel,
    ) -> Result<Vec<ReleaseInfo>, String> {
        let api = self.api(token, cancel);
        let repo_url = self.repo_url(repo);
        // the list does not say which release is the latest; this does
        let r = api.get(format!("{repo_url}/releases/latest")).await;
        if cancel.is_cancelled() {
            return cancelled();
        }
        let latest_id = match (r.status, r.json()) {
            (200, Some(j)) => u64_of(&j, "id"),
            // 404 is "no latest release" as much as "no such repository"; the list tells them apart
            (404, _) => 0,
            _ => return Err(access_problem(&r, repo)),
        };
        let mut out = Vec::new();
        for page in 1..=10 {
            let r = api.get(format!("{repo_url}/releases?per_page=100&page={page}")).await;
            if cancel.is_cancelled() {
                return cancelled();
            }
            if r.status == 404 {
                return Err(format!("Repository {repo} was not found, or this account cannot see it."));
            }
            let items = match r.json() {
                Some(Value::Array(items)) if r.status == 200 => items,
                _ => return Err(access_problem(&r, repo)),
            };
            for item in &items {
                let id = u64_of(item, "id");
                if id == 0 {
                    continue;
                }
                let draft = bool_of(item, "draft");
                let assets = array_of(item, "assets");
                out.push(ReleaseInfo {
                    id,
                    tag: str_of(item, "tag_name").to_owned(),
                    title: str_of(item, "name").to_owned(),
                    notes: str_of(item, "body").to_owned(),
                    url: str_of(item, "html_url").to_owned(),
                    published: if draft { String::new() } else { short_time(str_of(item, "published_at")) },
                    draft,
                    prerelease: bool_of(item, "prerelease"),
                    latest: id == latest_id,
                    assets: assets.iter().map(|a| str_of(a, "name").to_owned()).collect(),
                    bytes: assets.iter().map(|a| u64_of(a, "size")).sum(),
                });
            }
            if items.len() < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// Title and notes of a release that is already out; `None` keeps what it has. Touches nothing
    /// else: not the assets, not the tag, and not which release is the latest - GitHub's default
    /// for an edit would move that mark onto the edited release, so the request always states
    /// where the mark is now. Answers the release's tag.
    pub async fn edit_release(
        &self,
        repo: &str,
        token: &Secret,
        id: u64,
        title: Option<&str>,
        notes: Option<&str>,
        cancel: &Cancel,
    ) -> Result<String, String> {
        let api = self.api(token, cancel);
        let repo_url = self.repo_url(repo);
        let release_url = format!("{repo_url}/releases/{id}");
        let r = api.get(release_url.clone()).await;
        if cancel.is_cancelled() {
            return cancelled();
        }
        if r.status == 404 {
            return Err("The release no longer exists.".to_owned());
        }
        let Some(j) = (if r.status == 200 { r.json() } else { None }) else {
            return Err(access_problem(&r, repo));
        };
        let tag = str_of(&j, "tag_name").to_owned();
        let can_be_latest = !bool_of(&j, "draft") && !bool_of(&j, "prerelease");

        // where the mark is now
        let mut is_latest = false;
        if can_be_latest {
            let r = api.get(format!("{repo_url}/releases/latest")).await;
            if cancel.is_cancelled() {
                return cancelled();
            }
            match (r.status, r.json()) {
                (200, Some(latest)) => is_latest = u64_of(&latest, "id") == id,
                (404, _) => {}
                _ => return Err(access_problem(&r, repo)),
            }
        }

        let mut body = serde_json::Map::new();
        if let Some(t) = title {
            body.insert("name".to_owned(), Value::String(t.to_owned()));
        }
        if let Some(n) = notes {
            body.insert("body".to_owned(), Value::String(n.to_owned()));
        }
        body.insert(
            "make_latest".to_owned(),
            Value::String(if is_latest { "true" } else { "false" }.to_owned()),
        );
        let r = api.send(Method::PATCH, release_url, &Value::Object(body)).await;
        if r.status == 403 {
            return Err(format!(
                "This sign-in cannot edit releases in {repo}. A fine-grained token needs the Contents permission set to Read and write; a classic one needs public_repo."
            ));
        }
        if r.status != 200 {
            return Err(if cancel.is_cancelled() {
                "Cancelled while the release was being saved. Refresh the list to see what it holds."
                    .to_owned()
            } else {
                access_problem(&r, repo)
            });
        }
        Ok(tag)
    }

    /// The release goes first and its git tag second: a published release whose tag vanishes
    /// turns into a draft instead of going away. A draft owns no tag, so none is touched for it.
    /// GitHub keeps nothing of either - there is no undo. `Ok` = the release is gone; the tag's
    /// fate is in the result.
    pub async fn delete_release(
        &self,
        repo: &str,
        token: &Secret,
        id: u64,
        cancel: &Cancel,
    ) -> Result<DeleteResult, String> {
        let api = self.api(token, cancel);
        let repo_url = self.repo_url(repo);
        let release_url = format!("{repo_url}/releases/{id}");
        // as it is now, not as a list drawn a minute ago had it
        let r = api.get(release_url.clone()).await;
        if cancel.is_cancelled() {
            return cancelled();
        }
        if r.status == 404 {
            return Err("The release no longer exists.".to_owned());
        }
        let Some(j) = (if r.status == 200 { r.json() } else { None }) else {
            return Err(access_problem(&r, repo));
        };
        let mut result = DeleteResult {
            tag: str_of(&j, "tag_name").to_owned(),
            draft: bool_of(&j, "draft"),
            ..DeleteResult::default()
        };

        let r = api.delete(release_url).await;
        match r.status {
            204 => {}
            404 => return Err("The release no longer exists.".to_owned()),
            403 => {
                return Err(format!(
                    "This sign-in cannot delete releases in {repo}. A fine-grained token needs the Contents permission set to Read and write; a classic one needs public_repo."
                ));
            }
            _ => {
                // the request may have landed before the connection was cut
                return Err(if cancel.is_cancelled() {
                    "Cancelled while the release was being deleted. Refresh the list: it may be gone."
                        .to_owned()
                } else {
                    access_problem(&r, repo)
                });
            }
        }
        if result.draft || result.tag.is_empty() {
            return Ok(result);
        }
        let r = api.delete(format!("{repo_url}/git/refs/tags/{}", encode_ref(&result.tag))).await;
        // "Reference does not exist": nothing to remove is as good as removed
        if r.status == 204
            || r.status == 404
            || (r.status == 422 && String::from_utf8_lossy(&r.body).contains("does not exist"))
        {
            result.tag_removed = true;
        } else {
            result.tag_note = if cancel.is_cancelled() {
                "cancelled before the tag was removed".to_owned()
            } else {
                problem(&r)
            };
        }
        Ok(result)
    }

    //--------------------------------------------------------------------------
    // public files
    //--------------------------------------------------------------------------

    /// An anonymous GET that follows redirects. No token travels with it, whatever the address: a
    /// release asset, the catalog index, a website. `Ok((status, body))`.
    pub async fn fetch_public(
        &self,
        url: &str,
        limit: u64,
        cancel: &Cancel,
    ) -> Result<(u16, Vec<u8>), String> {
        let mut rq = Request::new(Kind::Public, Method::GET, url);
        rq.limit = limit;
        let r = self.send(rq, cancel).await;
        if r.too_large {
            return Err(format!("{url} is larger than {limit} bytes"));
        }
        if r.status == 0 {
            return Err(r.error);
        }
        Ok((r.status, r.body))
    }

    //--------------------------------------------------------------------------
    // the Mod Browser catalog
    //--------------------------------------------------------------------------

    /// The catalog's signed index from the first mirror that answers, not verified: it informs the
    /// author, and the catalog's own checks read it again.
    pub async fn fetch_catalog_index(&self, cancel: &Cancel) -> Option<String> {
        for mirror in &self.endpoints().mirrors {
            if cancel.is_cancelled() {
                return None;
            }
            if let Ok((200, body)) = self.fetch_public(&format!("{mirror}index.ltx"), 1 << 20, cancel).await {
                let text = String::from_utf8_lossy(&body).into_owned();
                if text.contains("[catalog]") {
                    return Some(text);
                }
            }
        }
        None
    }

    /// The account's login name (`GET /user`).
    pub async fn account_login(&self, token: &Secret, cancel: &Cancel) -> Result<String, String> {
        read_login(&self.api(token, cancel)).await
    }

    fn catalog_url(&self) -> String {
        self.repo_url(&self.endpoints().catalog_repo)
    }

    /// The account's newest `Submit <id>` (or `Withdraw <id>`) issue of the catalog, in any
    /// state; `out` keeps what it has when there is none. The list is newest first and holds the
    /// account's pull requests as well.
    async fn find_issue(
        &self,
        api: &Api<'_>,
        login: &str,
        module_id: &str,
        kind: IssueKind,
        out: &mut CatalogIssue,
    ) -> Result<(), String> {
        let catalog = &self.endpoints().catalog_repo;
        let title = issue_title(kind, module_id);
        for page in 1..=10 {
            let r = api
                .get(format!(
                    "{}/issues?creator={}&state=all&per_page=100&page={page}",
                    self.catalog_url(),
                    url_encode(login)
                ))
                .await;
            if api.cancel.is_cancelled() {
                return cancelled();
            }
            if r.status == 404 {
                return Err(format!("The Mod Browser catalog {catalog} was not found."));
            }
            let items = match r.json() {
                Some(Value::Array(items)) if r.status == 200 => items,
                _ => return Err(access_problem(&r, catalog)),
            };
            if let Some(found) =
                items.iter().find(|i| i.get("pull_request").is_none() && str_of(i, "title") == title)
            {
                read_issue(found, out);
                return Ok(());
            }
            if items.len() < 100 {
                break;
            }
        }
        Ok(())
    }

    /// The comments come oldest first, a page at a time, and the newest verdict of the catalog's
    /// workflow is the one in force. A failed page, or more comments than the pages hold, leaves
    /// the submission without a verdict rather than with an old one.
    async fn read_verdict(&self, api: &Api<'_>, out: &mut CatalogIssue) -> Result<(), String> {
        for page in 1..=30 {
            let r = api
                .get(format!(
                    "{}/issues/{}/comments?per_page=100&page={page}",
                    self.catalog_url(),
                    out.number
                ))
                .await;
            if api.cancel.is_cancelled() {
                return cancelled();
            }
            let Some(Value::Array(items)) = (if r.status == 200 { r.json() } else { None }) else { break };
            let comments =
                items.iter().map(|c| (c.get("user").map_or("", |u| str_of(u, "login")), str_of(c, "body")));
            if let Some((state, text)) = catalog::newest_verdict(comments) {
                out.verdict = state;
                out.verdict_text = text;
            }
            if items.len() < 100 {
                return Ok(());
            }
        }
        out.verdict.clear();
        out.verdict_text.clear();
        Ok(())
    }

    /// A new issue, or the text of the account's open one - an edit makes the catalog judge it
    /// again. `what` names it in an error: submission, withdrawal.
    async fn send_issue(
        &self,
        api: &Api<'_>,
        title: &str,
        text: &str,
        what: &str,
        out: &mut CatalogIssue,
    ) -> Result<(), String> {
        let r = if out.number != 0 && out.state == "open" {
            api.send(
                Method::PATCH,
                format!("{}/issues/{}", self.catalog_url(), out.number),
                &json!({ "body": text }),
            )
            .await
        } else {
            api.send(
                Method::POST,
                format!("{}/issues", self.catalog_url()),
                &json!({ "body": text, "title": title }),
            )
            .await
        };
        if api.cancel.is_cancelled() {
            return cancelled();
        }
        match r.json() {
            Some(j) if (r.status == 200 || r.status == 201) && u64_of(&j, "number") != 0 => {
                read_issue(&j, out);
                Ok(())
            }
            _ => Err(format!(
                "GitHub refused the {what}: {}",
                access_problem(&r, &self.endpoints().catalog_repo)
            )),
        }
    }

    /// The default branch's `mods/<id>.ltx`: `(status, text)`.
    async fn listing(&self, api: &Api<'_>, module_id: &str) -> (Response, Option<String>) {
        let r = api.get(format!("{}/contents/mods/{module_id}.ltx", self.catalog_url())).await;
        let text = if r.status == 200 {
            r.json()
                .and_then(|j| catalog::un_base64(str_of(&j, "content")))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        } else {
            None
        };
        (r, text)
    }

    /// The account's open submission takes the new text - an edit makes the catalog judge it
    /// again; without one a new issue opens. Nothing is sent when the catalog's default branch
    /// already holds this very file. An issue is public from the first second.
    pub async fn catalog_submit(
        &self,
        rq: &CatalogRequest,
        cancel: &Cancel,
        stage: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CatalogIssue, String> {
        let api = self.api(&rq.token, cancel);
        let mut out = CatalogIssue::default();
        stage("Waiting for GitHub: the account");
        out.login = read_login(&api).await?;
        stage("Waiting for GitHub: the submission");
        self.find_issue(&api, &out.login.clone(), &rq.module_id, IssueKind::Submission, &mut out).await?;

        // Nothing to send when the catalog's default branch already holds this very file - a
        // module listed and submitted again unchanged.
        let (_, current) = self.listing(&api, &rq.module_id).await;
        if cancel.is_cancelled() {
            return cancelled();
        }
        if current.as_deref() == Some(rq.file_text.as_str()) {
            out.note = "The catalog already holds this exact submission - nothing was sent.".to_owned();
            // a closed submission is told apart by its verdict: what the catalog said about the last one
            if out.number != 0 {
                self.read_verdict(&api, &mut out).await?;
            }
            return Ok(out);
        }

        // The open submission takes the new text, and the edit makes the catalog judge it again.
        // Once the last one is closed - accepted or refused - a new one opens.
        stage(if out.number != 0 && out.state == "open" {
            "Updating the submission"
        } else {
            "Opening the submission"
        });
        let body = catalog::submission_body(&rq.file_text, &rq.details);
        self.send_issue(
            &api,
            &issue_title(IssueKind::Submission, &rq.module_id),
            &body,
            "submission",
            &mut out,
        )
        .await?;
        Ok(out)
    }

    /// A withdrawal is an issue too, and only the account that owns the listed repository may send
    /// one. Refused with nothing sent when the default branch does not hold the module or names
    /// another owner; a note and nothing sent when it is withdrawn already. The account's open
    /// withdrawal is edited rather than doubled.
    pub async fn catalog_withdraw(
        &self,
        rq: &CatalogWithdrawal,
        cancel: &Cancel,
        stage: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CatalogIssue, String> {
        let api = self.api(&rq.token, cancel);
        let catalog = self.endpoints().catalog_repo.clone();
        let mut out = CatalogIssue::default();
        stage("Waiting for GitHub: the account");
        out.login = read_login(&api).await?;

        // the catalog judges by the file on its default branch, not by an index a mirror may still hold
        stage("Waiting for GitHub: the listing");
        let (r, listing) = self.listing(&api, &rq.module_id).await;
        if cancel.is_cancelled() {
            return cancelled();
        }
        if r.status == 404 {
            return Err(format!(
                "{} is not in the Mod Browser catalog: there is nothing to withdraw.",
                rq.module_id
            ));
        }
        let Some(listing) = listing else {
            return Err(format!("The catalog's listing could not be read: {}", access_problem(&r, &catalog)));
        };
        let github = ini::value(&listing, "mod", "github").unwrap_or("").to_owned();
        let withdrawn = ini::value(&listing, "mod", "withdrawn").unwrap_or("");
        let owner = github.split('/').next().unwrap_or("");
        if owner.is_empty() {
            return Err(format!("The catalog's listing of {} names no repository.", rq.module_id));
        }
        // the account the listing came from: the catalog accepts a withdrawal from nobody else
        if !owner.eq_ignore_ascii_case(&out.login) {
            return Err(format!(
                "Signed in as {}, but {} is listed from {github}, the repository of {owner}: only {owner} can withdraw it.",
                out.login, rq.module_id
            ));
        }

        stage("Waiting for GitHub: the withdrawal");
        self.find_issue(&api, &out.login.clone(), &rq.module_id, IssueKind::Withdrawal, &mut out).await?;
        if !withdrawn.is_empty() {
            out.note = format!("The catalog has withdrawn {} already - nothing was sent.", rq.module_id);
            if out.number != 0 {
                self.read_verdict(&api, &mut out).await?;
            }
            return Ok(out);
        }
        stage(if out.number != 0 && out.state == "open" {
            "Updating the withdrawal"
        } else {
            "Opening the withdrawal"
        });
        let body = catalog::withdrawal_body(&rq.module_id, &rq.details);
        self.send_issue(
            &api,
            &issue_title(IssueKind::Withdrawal, &rq.module_id),
            &body,
            "withdrawal",
            &mut out,
        )
        .await?;
        Ok(out)
    }

    /// The account's newest submission (or withdrawal) of the module, in any state, with the
    /// catalog's newest verdict.
    pub async fn catalog_issue(
        &self,
        module_id: &str,
        token: &Secret,
        kind: IssueKind,
        cancel: &Cancel,
    ) -> Result<CatalogIssue, String> {
        let api = self.api(token, cancel);
        let mut out = CatalogIssue { login: read_login(&api).await?, ..CatalogIssue::default() };
        self.find_issue(&api, &out.login.clone(), module_id, kind, &mut out).await?;
        if out.number != 0 {
            self.read_verdict(&api, &mut out).await?;
        }
        Ok(out)
    }
}

/// Waits for an upload and passes its progress on to the sink as it goes.
async fn report_progress(
    upload: impl Future<Output = Response>,
    sent: &AtomicU64,
    name: &str,
    total: u64,
    sink: &dyn PublishSink,
) -> Response {
    let mut upload = std::pin::pin!(upload);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));
    loop {
        tokio::select! {
            r = &mut upload => return r,
            _ = tick.tick() => sink.upload(name, sent.load(Ordering::Relaxed), total),
        }
    }
}

async fn read_login(api: &Api<'_>) -> Result<String, String> {
    let r = api.get(format!("{}/user", api.gh.endpoints().api)).await;
    if api.cancel.is_cancelled() {
        return cancelled();
    }
    if r.status == 401 {
        return Err("GitHub rejected the sign-in. Sign in again.".to_owned());
    }
    match r.json() {
        Some(j) if r.status == 200 && !str_of(&j, "login").is_empty() => Ok(str_of(&j, "login").to_owned()),
        _ => Err(problem(&r)),
    }
}

fn read_issue(j: &Value, out: &mut CatalogIssue) {
    out.number = u64_of(j, "number");
    out.url = str_of(j, "html_url").to_owned();
    out.state = str_of(j, "state").to_owned();
    if out.state.is_empty() {
        out.state = "open".to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_tags() {
        assert!(is_version_tag("v1.0.0", "v1.0.0"));
        assert!(is_version_tag("v1.0.0-r2", "v1.0.0"));
        assert!(!is_version_tag("v1.0.0-r", "v1.0.0"));
        assert!(!is_version_tag("v1.0.0-rc1", "v1.0.0"));
        assert!(!is_version_tag("v1.0.0.1", "v1.0.0"));
        assert!(!is_version_tag("v1.0.1", "v1.0.0"));
    }

    #[test]
    fn stored_asset_check() {
        let local =
            Asset { name: "od-1.0.0.zip".to_owned(), size: 10, sha256: "ab".to_owned(), ..Asset::default() };
        let ok = json!({ "name": "od-1.0.0.zip", "size": 10, "state": "uploaded", "digest": "sha256:AB" });
        assert_eq!(asset_mismatch(&ok, &local), "");
        let no_digest = json!({ "name": "od-1.0.0.zip", "size": 10, "state": "uploaded" });
        assert_eq!(asset_mismatch(&no_digest, &local), "");
        assert_eq!(
            asset_mismatch(&json!({ "name": "od-1.0.0.zip", "size": 9, "state": "uploaded" }), &local),
            "9 byte(s) stored, 10 packaged"
        );
        assert_eq!(
            asset_mismatch(&json!({ "name": "x.zip", "size": 10 }), &local),
            "stored under the name 'x.zip'"
        );
        assert_eq!(
            asset_mismatch(&json!({ "name": "od-1.0.0.zip", "size": 10, "state": "starter" }), &local),
            "state 'starter'"
        );
        assert_eq!(
            asset_mismatch(
                &json!({ "name": "od-1.0.0.zip", "size": 10, "state": "uploaded", "digest": "sha256:cd" }),
                &local
            ),
            "the SHA-256 differs"
        );
    }

    #[test]
    fn own_names_and_times() {
        let rq = PublishRequest {
            repo: "me/od".to_owned(),
            module_id: "od".to_owned(),
            version: "1.0.0".to_owned(),
            tag: "v1.0.0".to_owned(),
            title: String::new(),
            notes: String::new(),
            prerelease: false,
            latest: true,
            token: Secret::default(),
            assets: Vec::new(),
            descriptor: PathBuf::new(),
        };
        assert!(own_asset_name(&rq, "OD.update.ltx"));
        assert!(own_asset_name(&rq, "od-1.0.0.part2.zip"));
        assert!(!own_asset_name(&rq, "od-1.0.0."));
        assert!(!own_asset_name(&rq, "other.update.ltx"));
        assert_eq!(short_time("2026-09-19T12:34:56Z"), "2026-09-19 12:34");
        assert_eq!(short_time("soon"), "soon");
        let r = Response {
            status: 422,
            body:
                br#"{"message":"Validation Failed","errors":[{"field":"tag_name","code":"already_exists"}]}"#
                    .to_vec(),
            ..Response::default()
        };
        assert_eq!(problem(&r), "GitHub answered 422: Validation Failed (tag_name already_exists)");
        assert_eq!(
            access_problem(&Response { status: 401, ..Response::default() }, "a/b"),
            "GitHub rejected the sign-in. Sign in again."
        );
    }
}
