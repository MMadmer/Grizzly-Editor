//! The jobs of Submit to Mod Browser (`EditorCatalogSubmit`'s worker): `check` (MOD_CATALOG 7
//! against the latest release fetched anonymously, the way the game fetches it), `vt` (the
//! author's scan), `submit` (check, scan, then the issue), `submission` (where the submission, a
//! withdrawal and the listing stand) and `withdraw`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::{Cancel, Executors, JobCtx};
use gz_github::{Github, IssueKind, Scan, VtPolicy, scan};
use gz_release::content::{Finding, Level, check_content, content_head, content_need};

use crate::model::{
    Check, Checked, Input, Release, Standing, read_listing, reason, submission_details, submission_text, vt_level, vt_refusal, withdrawal_details,
};
use crate::service::{Catalog, Job};
use crate::website::website_is_module_page;

const NO_VT_KEY: &str = "No VirusTotal key: the catalog lists a package only when VirusTotal knows it. Create a free account at virustotal.com and paste its API key into the VirusTotal section below.";

/// A running job's hands: the service it reports to, the network, the pools, the stop request.
pub(crate) struct Run {
    pub svc: Arc<Catalog>,
    pub gh: Arc<Github>,
    pub exec: Executors,
    pub ctx: JobCtx,
}

impl Run {
    fn cancel(&self) -> &Cancel {
        &self.ctx.cancel
    }

    fn cancelled(&self) -> bool {
        self.ctx.cancel.is_cancelled()
    }

    pub(crate) fn stage(&self, text: &str) {
        self.svc.set_stage(text);
        self.ctx.stage(text);
    }

    /// The latest release, as the game reads it: `Err((status, why))`.
    async fn fetch_release(&self, input: &Input) -> Result<Release, (u16, String)> {
        let m = &input.manifest;
        let url = self.gh.endpoints().latest_download(&m.update_github, &format!("{}.update.ltx", m.id));
        let (status, body) = self.gh.fetch_public(&url, 256 << 10, self.cancel()).await.map_err(|e| (0, e))?;
        if status != 200 {
            return Err((status, format!("GitHub answered {status}")));
        }
        Ok(Release::read(body, &input.key.public_key))
    }

    /// Section 7, in the order the window lists it. `lookup` asks VirusTotal about the packages;
    /// a submission leaves that to its scan, which follows at once.
    pub(crate) async fn check(&self, input: &Input, lookup: bool, seen: &mut Checked, standing: &mut Standing) -> Check {
        let mut check = Check::default();
        let m = &input.manifest;
        let key = &input.key;

        if !key.exists {
            check.add(Level::Block, "key", "No author key: create one above - it signs every release.".to_owned());
        } else if !key.error.is_empty() {
            check.add(Level::Block, "key", format!("The author key does not read: {}.", key.error));
        } else {
            check.add(Level::Ok, "key", format!("Author key {}.", key.key_id));
        }

        let repo_ok = !m.update_github.is_empty() && gz_mod::validate_github_repo(&m.update_github);
        if m.update_github.is_empty() {
            check.add(Level::Block, "repository", "No GitHub repository in the manifest: set it in Mod > Mod Settings.".to_owned());
        } else if !repo_ok {
            check.add(Level::Block, "repository", format!("The manifest's GitHub repository is not owner/repo: {}.", m.update_github));
        } else {
            check.add(Level::Ok, "repository", format!("Repository {}.", m.update_github));
        }

        // the submission has to come from the account that owns the repository (7.6)
        if !input.signed_in {
            check.add(Level::Block, "account", "Not signed in to GitHub: sign in from Mod > Publish Release.".to_owned());
        } else {
            self.stage("Reading the GitHub account");
            let owner = owner(&m.update_github);
            match self.gh.account_login(&input.token, self.cancel()).await {
                Err(err) => {
                    check.add(Level::Block, "account", format!("The GitHub account could not be read: {}.", reason(&err)));
                    self.svc.set_login("");
                }
                Ok(login) => {
                    if repo_ok && !login.eq_ignore_ascii_case(owner) {
                        check.add(
                            Level::Block,
                            "account",
                            format!("Signed in as {login}, but {} belongs to {owner}: the submission has to come from the repository's owner.", m.update_github),
                        );
                    } else {
                        check.add(Level::Ok, "account", format!("Signed in to GitHub as {login}."));
                    }
                    self.svc.set_login(&login);
                }
            }
        }
        if self.cancelled() {
            return check;
        }

        // the release, fetched the way the game fetches it
        let mut have = false;
        if repo_ok {
            self.stage("Reading the latest release");
            match self.fetch_release(input).await {
                Ok(rel) => {
                    seen.rel = rel;
                    have = true;
                }
                Err((status, err)) if !self.cancelled() => check.add(
                    Level::Block,
                    "release",
                    if status == 404 {
                        format!("The latest release of {} carries no {}.update.ltx: publish one first (Mod > Publish Release).", m.update_github, m.id)
                    } else {
                        format!("The latest release could not be read: {}.", reason(&err))
                    },
                ),
                Err(_) => {}
            }
        }
        if self.cancelled() {
            return check;
        }
        let rel = &seen.rel;
        if have {
            check.version.clone_from(&rel.version);
            check.url = format!("{}/releases/latest", self.gh.endpoints().repo_page(&m.update_github));
            check.key_id.clone_from(&rel.signer);
            if rel.id != m.id {
                check.add(Level::Block, "release", format!("The latest release of {} is for {}, not {}.", m.update_github, rel.id, m.id));
            } else {
                check.add(Level::Ok, "release", format!("Latest release {}.", rel.version));
            }
            if gz_github::ini::compare_versions(&rel.version, "1.0.0").is_lt() {
                check.add(Level::Block, "version", format!("Release {} is below 1.0.0: the Mod Browser lists 1.0.0 and later.", rel.version));
            } else {
                check.add(Level::Ok, "version", format!("Version {} is 1.0.0 or later.", rel.version));
            }
            if !m.version.is_empty() && gz_github::ini::compare_versions(&m.version, &rel.version).is_ne() {
                check.add(Level::Note, "manifest_version", format!("mod.ltx says {}; the catalog reads the latest release, {}.", m.version, rel.version));
            }
            if rel.card {
                check.add(Level::Ok, "card", "The release carries a Mod Browser card.".to_owned());
            } else {
                check.add(Level::Block, "card", "The latest release has no Mod Browser card: package and publish it with this editor.".to_owned());
            }
            check.signed = rel.signed_ok;
            if key.public_key.is_empty() {
                check.add(Level::Note, "signature", "The signature is checked once there is an author key.".to_owned());
            } else if rel.signed_ok {
                check.add(Level::Ok, "signature", "Signed with this author key.".to_owned());
            } else if rel.signer.is_empty() {
                check.add(Level::Block, "signature", "The latest release is not signed: unlock the author key, then publish again.".to_owned());
            } else if rel.signer != key.key_id {
                check.add(Level::Block, "signature", format!("The latest release is signed with another key ({}).", rel.signer));
            } else {
                check.add(Level::Block, "signature", format!("The release's signature does not verify: {}.", rel.sign_error));
            }
        }

        // the website the Mod Browser shows: the card's; the manifest's while there is no card
        let website = if have && rel.card && !rel.website.is_empty() { rel.website.clone() } else { m.website.clone() };
        let moddb = website.contains("moddb.com");
        if website.is_empty() {
            check.add(Level::Block, "website", "No website: the Mod Browser lists a module with its own page on AP-PRO or ModDB.".to_owned());
        } else if !website_is_module_page(&website) {
            check.add(
                Level::Block,
                "website",
                format!("Website {website} is not the module's own page on AP-PRO or ModDB, e.g. https://ap-pro.ru/stuff/<section>/<name>-r<number>/."),
            );
        } else if self.gh.qa() {
            check.add(Level::Note, "website", format!("Website {website} - not opened in a QA run."));
        } else {
            self.stage("Opening the website");
            match self.gh.fetch_public(&website, 8 << 20, self.cancel()).await {
                Ok((200, _)) => check.add(Level::Ok, "website", format!("Website {website} answers.")),
                Ok((403 | 429 | 503, _)) if moddb => {
                    check.add(Level::Note, "website", "ModDB turns automated requests away: the page cannot be checked automatically.".to_owned());
                }
                Ok((status, _)) => check.add(Level::Block, "website", format!("Website {website} answers HTTP {status}.")),
                Err(err) => check.add(Level::Note, "website", format!("Website {website} could not be reached: {}.", reason(&err))),
            }
        }
        if self.cancelled() {
            return check;
        }

        if have {
            self.check_local_release(input, &seen.rel, &mut check).await;
        }
        if self.cancelled() {
            return check;
        }

        // the catalog: is the id free, or already this key's (7.7) - a withdrawn one stays bound to its key
        self.stage("Reading the Mod Browser catalog");
        let index = self.gh.fetch_catalog_index(self.cancel()).await;
        if let Some(index) = &index {
            seen.policy = VtPolicy::read(index);
        }
        standing.listing = read_listing(index.as_deref(), &m.id, &key.public_key);
        let listing = &standing.listing;
        match (index.is_some(), listing.state.as_str(), listing.this_key) {
            (false, ..) => check.add(Level::Note, "catalog", "The Mod Browser catalog did not answer: the catalog's own check covers the id.".to_owned()),
            (_, "free", _) => check.add(Level::Ok, "catalog", format!("The id {} is free in the Mod Browser.", m.id)),
            (_, "listed", true) => check.add(Level::Ok, "catalog", "Listed in the Mod Browser with this key.".to_owned()),
            (_, "listed", false) => check.add(
                Level::Block,
                "catalog",
                format!("{} is listed with another author key: sign the release with that key, or submit the module under another id.", m.id),
            ),
            (_, _, true) => check.add(Level::Note, "catalog", "Withdrawn from the Mod Browser: submitting lists it again.".to_owned()),
            _ => check.add(
                Level::Block,
                "catalog",
                format!(
                    "{} was withdrawn from the Mod Browser and stays bound to another author key: sign the release with that key, or submit the module under another id.",
                    m.id
                ),
            ),
        }
        for (id, line) in gz_github::ini::section(index.as_deref().unwrap_or(""), "revoked") {
            if id != m.id {
                continue;
            }
            let (version, why) = match line.split_once(',') {
                Some((v, r)) => (v.trim_end_matches(' '), r.trim_start_matches(' ')),
                None => (line, ""),
            };
            if version == "*" {
                check.add(Level::Block, "catalog", format!("The Mod Browser revoked {}: {why}", m.id));
            } else {
                check.add(Level::Note, "catalog", format!("The Mod Browser revoked version {version}: {why}"));
            }
        }

        let rel = &seen.rel;
        if have && rel.card {
            if rel.thumbnail.is_empty() {
                check.add(Level::Note, "thumbnail", "No thumbnail: the Mod Browser shows the name's first letter (Mod > Mod Settings).".to_owned());
            } else {
                check.add(Level::Ok, "thumbnail", "Thumbnail.".to_owned());
            }
            if rel.tags.is_empty() {
                check.add(Level::Note, "tags", "No tags: players who filter by tag will not find it (Mod > Mod Settings).".to_owned());
            } else {
                check.add(Level::Ok, "tags", format!("Tags: {}.", rel.tags));
            }
        }

        // VirusTotal (7.10), required: the catalog only looks packages up and refuses one
        // VirusTotal does not know, so the scan is the author's, with their own key
        let no_key = input.vt_key.is_empty();
        if have && (lookup || no_key) {
            let packages = seen.rel.scan_packages(&input.root);
            let stage = |t: &str| self.stage(t);
            seen.vt = self.gh.lookup(&packages, input.vt_key.expose(), &seen.policy, self.cancel(), &stage).await;
        }
        if have && no_key {
            check.add(Level::Block, "virustotal", NO_VT_KEY.to_owned());
        } else if have && lookup {
            for r in &seen.vt.packages {
                let mut text = scan::describe(r);
                text.push_str(match r.state.as_str() {
                    "unknown" => " - a report is required: Submit scans it first.",
                    "analysing" => " - a report is required: Submit waits for it.",
                    "unavailable" => " - a report is required: Submit asks again.",
                    _ => ".",
                });
                check.add(vt_level(&r.state, false), "virustotal", text);
            }
        }
        if self.cancelled() {
            return check;
        }

        // and where the submission and a withdrawal stand
        if input.signed_in {
            self.stage("Reading the submission");
            match self.gh.catalog_issue(&m.id, &input.token, IssueKind::Submission, self.cancel()).await {
                Ok(issue) => {
                    standing.submission = issue;
                    standing.submission_read = true;
                }
                Err(e) => standing.submission_error = e,
            }
            if !self.cancelled()
                && let Ok(issue) = self.gh.catalog_issue(&m.id, &input.token, IssueKind::Withdrawal, self.cancel()).await
            {
                standing.withdrawal = issue;
            }
        }
        check.done = !self.cancelled();
        check
    }

    /// The content rules over the release's own files, read out of this computer's copy of the
    /// packages - the bytes the descriptor pins, checked by their hashes first.
    async fn check_local_release(&self, input: &Input, rel: &Release, check: &mut Check) {
        let (root, id, rel, cancel) = (input.root.clone(), input.manifest.id.clone(), rel.clone(), self.cancel().clone());
        let svc = self.svc.clone();
        let stage = move |t: &str| svc.set_stage(t);
        let found = self.exec.io(move || local_release_findings(&root, &id, &rel, &cancel, &stage)).await;
        for f in found {
            check.add(f.level, f.code, f.text);
        }
    }

    /// `vt`: the scan of the latest release's packages.
    pub(crate) async fn vt(&self, input: &Input) -> (Scan, Result<(), String>) {
        self.stage("Reading the latest release");
        let rel = match self.fetch_release(input).await {
            Ok(rel) => rel,
            Err((status, err)) => {
                let m = &input.manifest;
                let err = if status == 404 {
                    format!("no release to scan: the latest release of {} carries no {}.update.ltx", m.update_github, m.id)
                } else {
                    err
                };
                return (Scan::default(), Err(err));
            }
        };
        self.stage("Reading the Mod Browser catalog");
        let policy = self.gh.fetch_catalog_index(self.cancel()).await.map_or_else(VtPolicy::default, |i| VtPolicy::read(&i));
        let stage = |t: &str| self.stage(t);
        let vt = self.gh.scan(&rel.scan_packages(&input.root), input.vt_key.expose(), &policy, self.cancel(), &stage).await;
        let done = if self.cancelled() { Err(gz_github::CANCELLED.to_owned()) } else { Ok(()) };
        (vt, done)
    }

    /// `submit`: the check again, the scan, then the issue.
    pub(crate) async fn submit(&self, input: &Input, seen: &mut Checked, standing: &mut Standing) -> (Check, Result<(), String>) {
        self.svc.set_step(0);
        let mut check = self.check(input, false, seen, standing).await;
        if self.cancelled() {
            return (check, Err(gz_github::CANCELLED.to_owned()));
        }
        if let Some(block) = check.findings.iter().find(|f| f.level == Level::Block) {
            let err = format!("not submitted: {}", block.text);
            return (check, Err(err));
        }
        self.svc.set_step(1);

        // The catalog only looks the packages up: what VirusTotal does not know goes up now, with
        // the author's key, and its report is waited for. Only a package with a report is sent -
        // VirusTotal answers 404 for a file until its first analysis ends, and the catalog refuses
        // a 404 as a package nobody scanned.
        let packages = seen.rel.scan_packages(&input.root);
        let stage = |t: &str| self.stage(t);
        seen.vt = self.gh.scan(&packages, input.vt_key.expose(), &seen.policy, self.cancel(), &stage).await;
        if self.cancelled() {
            return (check, Err(gz_github::CANCELLED.to_owned()));
        }
        for r in &seen.vt.packages {
            check.add(vt_level(&r.state, true), "virustotal", format!("{}.", scan::describe(r)));
        }
        if let Some(deciding) = scan::deciding(&seen.vt)
            && seen.vt.state != "clean"
            && seen.vt.state != "warn"
        {
            let err = format!("not submitted: {}.", vt_refusal(deciding, &seen.rel.version));
            return (check, Err(err));
        }

        self.svc.set_step(2);
        let rel = &seen.rel;
        let m = &input.manifest;
        let website = if rel.card && !rel.website.is_empty() { rel.website.clone() } else { m.website.clone() };
        let rq = gz_github::CatalogRequest {
            module_id: m.id.clone(),
            file_text: submission_text(m, &input.key, rel, &website),
            details: submission_details(m, &input.key, seen, &check, &website, &self.gh.endpoints().repo_page(&m.update_github)),
            token: input.token.clone(),
        };
        let stage = |t: &str| self.stage(t);
        let sent = self.gh.catalog_submit(&rq, self.cancel(), &stage).await;
        drop(rq);
        // a failure may come after the issue went out: where the submission stands is not known then
        standing.submission_read = sent.is_ok();
        standing.submission_error.clear();
        match sent {
            Ok(issue) => {
                standing.submission = issue;
                (check, Ok(()))
            }
            Err(e) => (check, Err(e)),
        }
    }

    /// The published index's word on the module.
    async fn read_listing_now(&self, input: &Input, standing: &mut Standing) {
        self.stage("Reading the Mod Browser catalog");
        let index = self.gh.fetch_catalog_index(self.cancel()).await;
        standing.listing = read_listing(index.as_deref(), &input.manifest.id, &input.key.public_key);
    }

    /// `submission`: where the submission, a withdrawal and the listing stand.
    pub(crate) async fn standing(&self, input: &Input, standing: &mut Standing) -> Result<(), String> {
        let id = &input.manifest.id;
        self.stage("Reading the submission");
        standing.submission = self.gh.catalog_issue(id, &input.token, IssueKind::Submission, self.cancel()).await?;
        standing.submission_read = true;
        self.stage("Reading the withdrawal");
        standing.withdrawal = self.gh.catalog_issue(id, &input.token, IssueKind::Withdrawal, self.cancel()).await?;
        self.read_listing_now(input, standing).await;
        Ok(())
    }

    /// `withdraw`: only the account that owns the listed repository may withdraw a module - the
    /// editor refuses anyone else before anything is sent, and the catalog checks again by itself.
    pub(crate) async fn withdraw(&self, input: &Input, standing: &mut Standing) -> Result<(), String> {
        let m = &input.manifest;
        let rq = gz_github::CatalogWithdrawal { module_id: m.id.clone(), details: withdrawal_details(m), token: input.token.clone() };
        let stage = |t: &str| self.stage(t);
        standing.withdrawal = self.gh.catalog_withdraw(&rq, self.cancel(), &stage).await?;
        drop(rq);
        self.svc.set_login(&standing.withdrawal.login);
        self.stage("Reading the submission");
        match self.gh.catalog_issue(&m.id, &input.token, IssueKind::Submission, self.cancel()).await {
            Ok(issue) => {
                standing.submission = issue;
                standing.submission_read = true;
            }
            Err(e) => {
                standing.submission_read = false;
                standing.submission_error = e;
            }
        }
        self.read_listing_now(input, standing).await;
        Ok(())
    }

    /// Runs one job to its end and leaves what it learned in the service's view.
    pub(crate) async fn work(&self, job: Job, input: Input) -> Result<(), String> {
        let mut check = Check::default();
        let mut seen = Checked::default();
        let mut standing = Standing::default();
        let outcome = match job {
            Job::Check => {
                check = self.check(&input, true, &mut seen, &mut standing).await;
                Ok(())
            }
            Job::Vt => {
                let (vt, done) = self.vt(&input).await;
                seen.vt = vt;
                done
            }
            Job::Submit => {
                let (c, done) = self.submit(&input, &mut seen, &mut standing).await;
                check = c;
                done
            }
            Job::Submission => self.standing(&input, &mut standing).await,
            Job::Withdraw => self.withdraw(&input, &mut standing).await,
            Job::None => Ok(()),
        };
        let cancelled = self.cancelled();
        self.svc.finish(job, &input, cancelled, &outcome, check, seen, standing);
        drop(input);
        if cancelled { Err(gz_github::CANCELLED.to_owned()) } else { outcome }
    }
}

fn owner(repo: &str) -> &str {
    repo.split('/').next().unwrap_or("")
}

/// The release's folder on this computer, when it holds exactly what was published.
fn local_release_dir(root: &Path, id: &str, rel: &Release) -> Option<PathBuf> {
    let dir = root.join("releases").join(&rel.version);
    let local = std::fs::read(dir.join(format!("{id}.update.ltx"))).ok()?;
    (local == rel.bytes).then_some(dir)
}

fn note(text: String) -> Finding {
    Finding { level: Level::Note, code: "content", text }
}

/// The head of one stored entry the content rules need: all of a script, the head of the rest.
fn entry_head(file: &mut File, e: &gz_release::zip::Stored, rel_path: &str) -> Vec<u8> {
    let mut data = Vec::new();
    if file.seek(SeekFrom::Start(e.data_offset)).is_err() {
        return data;
    }
    let raw = Read::take(&mut *file, e.packed);
    let mut reader: Box<dyn Read + '_> = match e.method {
        0 => Box::new(raw),
        8 => Box::new(flate2::read::DeflateDecoder::new(raw)),
        _ => return data,
    };
    let mut want = content_head(rel_path, e.size).min(e.size);
    while want > data.len() as u64 {
        let start = data.len();
        let Ok(need) = usize::try_from(want - start as u64) else { break };
        data.resize(start + need, 0);
        let mut got = 0;
        while let Some(buf) = data.get_mut(start + got..) {
            if buf.is_empty() {
                break;
            }
            match reader.read(buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        data.truncate(start + got);
        if got == 0 {
            break;
        }
        want = content_need(&data, e.size).min(e.size);
    }
    data
}

/// The content findings of a release's local copy (`CheckLocalRelease`). Blocking.
fn local_release_findings(root: &Path, id: &str, rel: &Release, cancel: &Cancel, stage: &dyn Fn(&str)) -> Vec<Finding> {
    let Some(dir) = local_release_dir(root, id, rel) else {
        return vec![note(format!(
            "Content not checked here: this computer has no copy of release {} (the catalog checks it when it judges the submission).",
            rel.version
        ))];
    };
    let prefix = format!("modules/{id}/");
    let mut found: Vec<Finding> = Vec::new();
    let mut files = 0usize;
    for p in &rel.packages {
        if cancel.is_cancelled() {
            return Vec::new();
        }
        let zip_path = dir.join(&p.name);
        stage(&format!("Checking {}", p.name));
        if scan::file_sha256(&zip_path, cancel).is_none_or(|(sha, _)| sha != p.sha256) {
            return vec![note(format!(
                "Content not checked here: the local {} is not the published one (the catalog checks it when it judges the submission).",
                p.name
            ))];
        }
        let (Ok(entries), Ok(mut file)) = (gz_release::zip::read_layout(&zip_path), File::open(&zip_path)) else {
            return vec![note(format!("Content not checked here: {} does not open.", p.name))];
        };
        for e in &entries {
            if cancel.is_cancelled() {
                return Vec::new();
            }
            if e.name.is_empty() || e.name.ends_with('/') {
                continue;
            }
            let Some(rel_path) = e.name.strip_prefix(&prefix) else {
                found.push(Finding { level: Level::Block, code: "content", text: format!("{}: outside modules/{id}/", e.name) });
                continue;
            };
            let head = entry_head(&mut file, e, rel_path);
            check_content(rel_path, &head, &mut found);
            files += 1;
        }
    }
    let mut out = Vec::with_capacity(found.len() + 1);
    if !found.iter().any(|f| f.level == Level::Block) {
        out.push(Finding { level: Level::Ok, code: "content", text: format!("Content: {files} file(s) checked, nothing the catalog refuses.") });
    }
    out.extend(found);
    out
}
