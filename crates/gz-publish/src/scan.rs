//! A module the Mod Browser lists has every release scanned on VirusTotal before
//! anything is created on GitHub. The catalog only looks packages up and holds back a release
//! VirusTotal does not know, and a listed module's players take a release through their updates
//! at once - so it leaves only with a VirusTotal report of every package.

use gz_editor::Cancel;
use gz_github::{Github, Package, Scan, Secret, VtPolicy, ini, scan};

use crate::job::PublishState;

/// What the scan needs, taken with the job.
pub(crate) struct ScanPlan {
    pub module_id: String,
    pub packages: Vec<Package>,
    /// `None` = no key.
    pub vt_key: Option<Secret>,
}

/// Before anything is created on GitHub. A module the index does not list is published as ever;
/// one it lists - or any, while the index does not answer and there is a key - is scanned.
/// `Ok(warnings)`; `Err(why)` = not published.
pub(crate) async fn scan_first(
    gh: &Github,
    plan: &ScanPlan,
    cancel: &Cancel,
    stage: &(dyn Fn(&str) + Send + Sync),
    set_scan: &(dyn Fn(PublishState, &Scan) + Send + Sync),
) -> Result<String, String> {
    stage("Waiting for the Mod Browser catalog");
    let index = gh.fetch_catalog_index(cancel).await;
    if cancel.is_cancelled() {
        return Err(gz_github::CANCELLED.to_owned());
    }
    let reached = index.is_some();
    let index = index.unwrap_or_default();
    let listed = reached && ini::listed_key(&index, &plan.module_id).is_some();
    if reached && !listed {
        if ini::withdrawn_key(&index, &plan.module_id).is_some() {
            return Ok(format!(
                "{} is withdrawn from the Mod Browser: players who installed it there do not get this release. Submitting the module again lists it again.",
                plan.module_id
            ));
        }
        return Ok(String::new());
    }

    let policy = if reached { VtPolicy::read(&index) } else { VtPolicy::default() };
    let Some(key) = plan.vt_key.as_ref().filter(|k| !k.is_empty()) else {
        // no key: every package is `no_key`, and nothing is asked of VirusTotal
        let scan = gh.scan(&plan.packages, "", &policy, cancel, &|_| {}).await;
        set_scan(PublishState::Uploading, &scan);
        if !listed {
            return Ok("The Mod Browser catalog did not answer, and without a VirusTotal key nothing was scanned: if this module is in the Mod Browser, the release is offered there only once VirusTotal knows its packages.".to_owned());
        }
        return Err(format!(
            "{} is in the Mod Browser, and every release of it is scanned on VirusTotal with your own free key before it goes out. Add the key in Mod > Submit to Mod Browser (MCP: release.catalog action=vt_key) - a free account at virustotal.com has one.",
            plan.module_id
        ));
    };

    set_scan(PublishState::Scanning, &Scan { state: "scanning".to_owned(), packages: Vec::new() });
    let scan = gh.scan(&plan.packages, key.expose(), &policy, cancel, stage).await;
    set_scan(PublishState::Uploading, &scan);
    if cancel.is_cancelled() {
        return Err(gz_github::CANCELLED.to_owned());
    }
    let Some(deciding) = scan::deciding(&scan) else { return Ok(String::new()) };
    let what = scan::describe(deciding);
    let state = scan.state.as_str();
    if state == "block" {
        return Err(format!("{what}. The Mod Browser would withdraw this release."));
    }
    if state == "too_large" {
        return Err(format!(
            "{what}, and the Mod Browser holds back a release VirusTotal cannot know. Package Release keeps every package under that size unless a single file alone is larger: make that file smaller."
        ));
    }
    if scan::refuses(state) {
        return Err(format!("{what}. The Mod Browser would hold this release back."));
    }
    // No report yet: a listed module's release reaches players at once through their updates, so it
    // never leaves before VirusTotal has spoken on every package. Without an index nobody can tell
    // whether the catalog lists it, and it goes out with a warning as ever.
    if state == "analysing" || state == "unavailable" {
        if listed {
            let again = if state == "analysing" {
                "publish again in a few minutes - VirusTotal has the package already."
            } else {
                "publish again when VirusTotal answers."
            };
            return Err(format!(
                "{what}. A release of a module in the Mod Browser goes out only with a VirusTotal report of every package: {again}"
            ));
        }
        return Ok(format!(
            "{what}. The Mod Browser catalog did not answer: if this module is in the Mod Browser, it is offered there only once VirusTotal has a report of every package."
        ));
    }
    Ok(String::new())
}
