//! What the Mod Browser jobs know, and the text they send: the release as the game reads it, the
//! check's findings, where the module stands with the catalog, the submission's file and
//! details.

use std::path::PathBuf;

use gz_github::{CatalogIssue, Scan, ScanResult, Secret, VtPolicy, ini, scan, vt};
use gz_mod::Manifest;
use gz_release::KeyState;
use gz_release::content::{Finding, Level};

/// A package the descriptor pins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelPackage {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// The latest release, as the game reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
    /// The descriptor, verbatim.
    pub bytes: Vec<u8>,
    pub text: String,
    pub id: String,
    pub version: String,
    pub packages: Vec<RelPackage>,
    pub card: bool,
    pub website: String,
    pub thumbnail: String,
    pub tags: String,
    /// The key id the signature claims.
    pub signer: String,
    /// By the author key.
    pub signed_ok: bool,
    pub sign_error: String,
}

/// `strtoull`: the leading digits.
fn leading_u64(s: &str) -> u64 {
    let s = s.trim_start();
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    s.get(..n).and_then(|d| d.parse().ok()).unwrap_or(0)
}

impl Release {
    /// The descriptor read (`FetchRelease`), the signature checked against `public_key`.
    #[must_use]
    pub fn read(bytes: Vec<u8>, public_key: &str) -> Self {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut rel = Self {
            id: ini::value(&text, "release", "id").unwrap_or("").to_owned(),
            version: ini::value(&text, "release", "version").unwrap_or("").to_owned(),
            ..Self::default()
        };
        for (name, value) in ini::section(&text, "packages") {
            let sha256 = value.split_once(',').map_or("", |(_, h)| h).trim_start_matches(' ').to_ascii_lowercase();
            rel.packages.push(RelPackage { name: name.to_owned(), size: leading_u64(value), sha256 });
        }
        let card = ini::section(&text, "card");
        rel.card = !card.is_empty();
        for (key, value) in card {
            match key {
                "website" => rel.website = ini::unquote(value),
                "thumbnail" => rel.thumbnail = value.to_owned(),
                "tags" => rel.tags = value.to_owned(),
                _ => {}
            }
        }
        // the signer is named whatever the outcome: "signed with another key" needs it
        match gz_release::signed::verify_text(&text, public_key) {
            Ok(signer) => {
                rel.signer = signer;
                rel.signed_ok = true;
            }
            Err(e) => {
                rel.signer = e.signer;
                rel.sign_error = e.message;
            }
        }
        rel.text = text;
        rel.bytes = bytes;
        rel
    }

    /// The release's packages the way VirusTotal gets them: this computer's copies from
    /// `releases\<version>`.
    #[must_use]
    pub fn scan_packages(&self, root: &std::path::Path) -> Vec<gz_github::Package> {
        let dir = root.join("releases").join(&self.version);
        self.packages
            .iter()
            .map(|p| gz_github::Package { name: p.name.clone(), path: dir.join(&p.name), size: p.size, sha256: p.sha256.clone() })
            .collect()
    }
}

/// What a check found.
#[derive(Clone, Debug, Default)]
pub struct Check {
    pub done: bool,
    pub blocking: bool,
    pub version: String,
    pub url: String,
    pub key_id: String,
    pub signed: bool,
    pub findings: Vec<Finding>,
}

impl Check {
    pub fn add(&mut self, level: Level, code: &'static str, text: String) {
        self.blocking |= level == Level::Block;
        self.findings.push(Finding { level, code, text });
    }
}

/// Where the published index has the module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// free, listed, withdrawn; unknown when the index did not answer; empty = not read yet.
    pub state: String,
    /// The repository the index names for it.
    pub github: String,
    /// Bound to this computer's author key.
    pub this_key: bool,
}

/// `[mods]` and `[withdrawn]` lines alike: github, key, version, date.
#[must_use]
pub fn read_listing(index: Option<&str>, id: &str, public_key: &str) -> Listing {
    let Some(index) = index else { return Listing { state: "unknown".to_owned(), ..Listing::default() } };
    for (section, state) in [("mods", "listed"), ("withdrawn", "withdrawn")] {
        if let Some((_, line)) = ini::section(index, section).into_iter().find(|(k, _)| *k == id) {
            return Listing {
                state: state.to_owned(),
                github: ini::field(line, 0),
                this_key: !public_key.is_empty() && ini::field(line, 1) == public_key,
            };
        }
    }
    Listing { state: "free".to_owned(), ..Listing::default() }
}

/// What a check learned beyond its findings.
#[derive(Clone, Debug, Default)]
pub struct Checked {
    pub rel: Release,
    /// The index's; the catalog's defaults when it did not answer.
    pub policy: VtPolicy,
    pub vt: Scan,
}

/// Where the module stands with the catalog: the account's issues and the published index.
#[derive(Clone, Debug, Default)]
pub struct Standing {
    pub submission: CatalogIssue,
    pub withdrawal: CatalogIssue,
    /// GitHub answered: `submission` is its word, not an empty start.
    pub submission_read: bool,
    pub submission_error: String,
    pub listing: Listing,
}

/// Taken when a job starts; the secrets go when the job ends.
pub struct Input {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub key: KeyState,
    pub signed_in: bool,
    pub token: Secret,
    /// Empty = none.
    pub vt_key: Secret,
}

/// A report is required of every package: clean passes, warn is reported to players. A block, a
/// package too large for VirusTotal and one even a scan could not get there refuse; anything
/// else - not scanned yet, still analysing, no answer - is the requirement still open, never a
/// mere note.
#[must_use]
pub fn vt_level(state: &str, scanned: bool) -> Level {
    match state {
        "clean" => Level::Ok,
        "warn" => Level::Review,
        _ if scan::refuses(state) && (scanned || state != "unknown") => Level::Block,
        _ => Level::Required,
    }
}

/// Every package has a report the catalog lists by.
#[must_use]
pub fn vt_settled(vt: &Scan) -> bool {
    vt.state == "clean" || vt.state == "warn"
}

/// `mods/<id>.ltx` as the catalog is to commit it.
#[must_use]
pub fn submission_text(m: &Manifest, key: &KeyState, rel: &Release, website: &str) -> String {
    format!(
        "[mod]\nid      = {}\ngithub  = {}\nkey     = {}\nversion = {}\nwebsite = {website}\n",
        m.id, m.update_github, key.public_key, rel.version
    )
}

fn heading(m: &Manifest) -> String {
    let name = if m.name.is_empty() { &m.id } else { &m.name };
    let by = if m.author.is_empty() { String::new() } else { format!(" by {}", m.author) };
    format!("**{name}**{by} - `{}`", m.id)
}

/// What the issue says after the file: the module, the links, the verdicts, the editor's check.
#[must_use]
pub fn submission_details(m: &Manifest, key: &KeyState, seen: &Checked, check: &Check, website: &str, repo_page: &str) -> String {
    let mut body = format!("{} {}\n\n", heading(m), seen.rel.version);
    body.push_str(&format!("- Repository: {repo_page}\n"));
    body.push_str(&format!("- Website: {website}\n"));
    body.push_str(&format!("- Author key: `{}`\n\n", key.key_id));
    body.push_str("VirusTotal:\n");
    for r in &seen.vt.packages {
        let verdict = if matches!(r.state.as_str(), "clean" | "warn" | "block") {
            format!("{}, {} engines", r.state, r.verdict.engines)
        } else {
            scan::describe(r)
        };
        body.push_str(&format!("- `{}`: {verdict} - {}\n", r.package.name, vt::report_page(&r.package.sha256)));
    }
    body.push_str("\nChecked by Grizzly Editor:\n");
    for f in &check.findings {
        body.push_str(&format!("- {}: {}\n", f.level.name(), f.text));
    }
    body
}

/// What the withdrawal issue says after its ini block, for people.
#[must_use]
pub fn withdrawal_details(m: &Manifest) -> String {
    format!(
        "{}\n\nThe author withdraws the module from the Mod Browser. Installed copies keep working, and submitting the module again lists it again.\n",
        heading(m)
    )
}

/// Why the scan keeps the submission back, and what helps.
#[must_use]
pub fn vt_refusal(r: &ScanResult, version: &str) -> String {
    let what = scan::describe(r);
    match r.state.as_str() {
        "block" => format!("{what}: the catalog would refuse it"),
        "too_large" => format!("{what}: package the release again - Package Release keeps every package under that size unless a single file alone is larger"),
        "unknown" => format!("{what}: put the published files into releases\\{version}, or upload the package at virustotal.com"),
        "analysing" => format!("{what}: submit again in a few minutes"),
        "unavailable" => format!("{what}: submit again when VirusTotal answers"),
        _ => what,
    }
}

/// What GitHub or the network gave as the reason, for a sentence that ends after it.
#[must_use]
pub fn reason(text: &str) -> &str {
    text.trim_end_matches(['.', ' '])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            id: "od".to_owned(),
            name: "Overnight Delivery".to_owned(),
            author: "Madmer".to_owned(),
            update_github: "me/od".to_owned(),
            ..Manifest::default()
        }
    }

    fn key() -> KeyState {
        KeyState { exists: true, key_id: "0123456789abcdef".to_owned(), public_key: "p256:AAAA".to_owned(), ..KeyState::default() }
    }

    #[test]
    fn the_descriptor_as_the_game_reads_it() {
        let text = "[release]\nid = od\nversion = 1.2.0\n[packages]\nod-1.2.0.zip = 1234, ABCDEF\n[card]\nwebsite = \"https://ap-pro.ru/stuff/mods/od-r1/\"\ntags = story, quests\n";
        let rel = Release::read(text.as_bytes().to_vec(), "");
        assert_eq!((rel.id.as_str(), rel.version.as_str()), ("od", "1.2.0"));
        assert_eq!(rel.packages, vec![RelPackage { name: "od-1.2.0.zip".to_owned(), size: 1234, sha256: "abcdef".to_owned() }]);
        assert!(rel.card);
        assert_eq!(rel.website, "https://ap-pro.ru/stuff/mods/od-r1/");
        assert_eq!(rel.tags, "story, quests");
        assert!(!rel.signed_ok);
        assert_eq!(rel.sign_error, "not signed");
    }

    #[test]
    fn listings() {
        let index = "[catalog]\n[mods]\nod = me/od, p256:AAAA, 1.0.0, 2026-01-01\n[withdrawn]\nold = me/old, p256:BBBB, 1.0.0, 2026-01-01\n";
        assert_eq!(read_listing(Some(index), "od", "p256:AAAA"), Listing { state: "listed".to_owned(), github: "me/od".to_owned(), this_key: true });
        assert_eq!(read_listing(Some(index), "old", "p256:AAAA").state, "withdrawn");
        assert!(!read_listing(Some(index), "old", "p256:AAAA").this_key);
        assert_eq!(read_listing(Some(index), "new", "").state, "free");
        assert_eq!(read_listing(None, "od", "").state, "unknown");
    }

    #[test]
    fn levels_never_optional() {
        assert_eq!(vt_level("clean", false), Level::Ok);
        assert_eq!(vt_level("warn", true), Level::Review);
        assert_eq!(vt_level("block", false), Level::Block);
        assert_eq!(vt_level("no_key", false), Level::Block);
        // not scanned yet: a requirement Submit meets by itself, never a grey note
        assert_eq!(vt_level("unknown", false), Level::Required);
        assert_eq!(vt_level("unknown", true), Level::Block);
        assert_eq!(vt_level("analysing", false), Level::Required);
        assert_eq!(vt_level("unavailable", true), Level::Required);
    }

    #[test]
    fn the_submission_for_a_fixture() {
        let (m, k) = (manifest(), key());
        let rel = Release { version: "1.2.0".to_owned(), ..Release::default() };
        assert_eq!(
            submission_text(&m, &k, &rel, "https://ap-pro.ru/stuff/mods/od-r1/"),
            "[mod]\nid      = od\ngithub  = me/od\nkey     = p256:AAAA\nversion = 1.2.0\nwebsite = https://ap-pro.ru/stuff/mods/od-r1/\n"
        );
        let mut check = Check::default();
        check.add(Level::Ok, "key", "Author key 0123456789abcdef.".to_owned());
        let mut seen = Checked { rel, ..Checked::default() };
        let mut r = ScanResult::default();
        r.package.name = "od-1.2.0.zip".to_owned();
        r.package.sha256 = "ab".to_owned();
        r.state = "clean".to_owned();
        r.verdict.engines = 70;
        seen.vt.packages.push(r);
        assert_eq!(
            submission_details(&m, &k, &seen, &check, "https://ap-pro.ru/stuff/mods/od-r1/", "https://github.com/me/od"),
            "**Overnight Delivery** by Madmer - `od` 1.2.0\n\n- Repository: https://github.com/me/od\n- Website: https://ap-pro.ru/stuff/mods/od-r1/\n- Author key: `0123456789abcdef`\n\n\
             VirusTotal:\n- `od-1.2.0.zip`: clean, 70 engines - https://www.virustotal.com/gui/file/ab\n\nChecked by Grizzly Editor:\n- ok: Author key 0123456789abcdef.\n"
        );
        assert_eq!(
            withdrawal_details(&m),
            "**Overnight Delivery** by Madmer - `od`\n\nThe author withdraws the module from the Mod Browser. Installed copies keep working, and submitting the module again lists it again.\n"
        );
    }
}
