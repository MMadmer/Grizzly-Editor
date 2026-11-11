//! The Mod Browser catalog's issue mechanics (game repo `docs/dead-air/MOD_CATALOG.md` 10): a
//! submission is an ISSUE `Submit <id>` of the catalog, a withdrawal an issue `Withdraw <id>`,
//! and the catalog's workflow comments its verdict on them as `github-actions[bot]`.

use crate::secret::Secret;

/// The catalog's workflow comments as this account. A person cannot take a login with brackets,
/// so a comment by it is the workflow's own.
pub const CATALOG_BOT: &str = "github-actions[bot]";

/// Which issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    Submission,
    Withdrawal,
}

/// `Submit <id>` / `Withdraw <id>`.
#[must_use]
pub fn issue_title(kind: IssueKind, module_id: &str) -> String {
    match kind {
        IssueKind::Submission => format!("Submit {module_id}"),
        IssueKind::Withdrawal => format!("Withdraw {module_id}"),
    }
}

/// The account's newest submission (or withdrawal) of a module and the catalog's word on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogIssue {
    /// 0 = none.
    pub number: u64,
    /// open, closed; `none` when there is none.
    pub state: String,
    pub url: String,
    /// The account that asked.
    pub login: String,
    /// A request that sent nothing says why.
    pub note: String,
    /// accepted, waiting, refused; empty = the catalog has not spoken yet.
    pub verdict: String,
    /// The verdict's one plain sentence.
    pub verdict_text: String,
}

impl Default for CatalogIssue {
    fn default() -> Self {
        Self {
            number: 0,
            state: "none".to_owned(),
            url: String::new(),
            login: String::new(),
            note: String::new(),
            verdict: String::new(),
            verdict_text: String::new(),
        }
    }
}

/// A submission to send.
#[derive(Clone, Debug)]
pub struct CatalogRequest {
    pub module_id: String,
    /// `mods/<id>.ltx`, byte for byte as the catalog is to commit it.
    pub file_text: String,
    /// Markdown after it: the links, the verdicts, the editor's check.
    pub details: String,
    pub token: Secret,
}

/// A withdrawal to send.
#[derive(Clone, Debug)]
pub struct CatalogWithdrawal {
    pub module_id: String,
    /// Markdown after the ini block, for people.
    pub details: String,
    pub token: Secret,
}

/// The issue's body: the marker, a line for people, then the file the catalog commits in an ini
/// block of its own - byte for byte, the catalog takes it from there - and the details after it.
#[must_use]
pub fn submission_body(file_text: &str, details: &str) -> String {
    let mut body = String::from(
        "<!-- xms-catalog submission -->\nSubmitted from Grizzly Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 7).\n\n```ini\n",
    );
    body.push_str(file_text);
    if !file_text.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("```\n");
    if !details.is_empty() {
        body.push('\n');
        body.push_str(details);
    }
    body
}

/// A withdrawal names the module in an ini block of its own; the catalog reads nothing else of it.
#[must_use]
pub fn withdrawal_body(module_id: &str, details: &str) -> String {
    let mut body = format!(
        "<!-- xms-catalog withdrawal -->\nWithdrawn by its author from Grizzly Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 10.2).\n\n```ini\n[withdraw]\nid = {module_id}\n```\n"
    );
    if !details.is_empty() {
        body.push('\n');
        body.push_str(details);
    }
    body
}

fn blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r')
}

fn trim_blank(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\r'])
}

/// A verdict comment: `<!-- xms-catalog verdict=STATE -->` on its first line, one plain sentence
/// on the next line that holds anything, then a report for people. Spacing and line ends are the
/// writer's. `Some((state, sentence))`.
#[must_use]
pub fn parse_verdict(body: &str) -> Option<(String, String)> {
    let mut lines = body.split('\n').map(trim_blank).filter(|l| !l.is_empty());
    let marker = lines.next()?;
    let text = lines.next().unwrap_or("");
    if marker.len() < 7 {
        return None;
    }
    let inner = marker.strip_prefix("<!--")?.strip_suffix("-->")?;
    let mut p = inner.as_bytes();
    let skip = |p: &mut &[u8]| {
        while p.first().is_some_and(|c| blank(*c)) {
            *p = p.get(1..).unwrap_or(&[]);
        }
    };
    let take = |p: &mut &[u8], word: &str| -> bool {
        match p.strip_prefix(word.as_bytes()) {
            Some(rest) => {
                *p = rest;
                true
            }
            None => false,
        }
    };
    skip(&mut p);
    if !take(&mut p, "xms-catalog") || !p.first().is_some_and(|c| blank(*c)) {
        return None;
    }
    skip(&mut p);
    if !take(&mut p, "verdict") {
        return None;
    }
    skip(&mut p);
    if !take(&mut p, "=") {
        return None;
    }
    skip(&mut p);
    let word: String =
        p.iter().take_while(|c| !blank(**c)).map(|c| char::from(c.to_ascii_lowercase())).collect();
    if !matches!(word.as_str(), "accepted" | "waiting" | "refused") {
        return None;
    }
    Some((word, text.to_owned()))
}

/// The verdict in force among comments read oldest first: the newest one the catalog's bot wrote.
/// A look-alike from anyone else counts for nothing.
#[must_use]
pub fn newest_verdict<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<(String, String)> {
    let mut found = None;
    for (login, body) in comments {
        if login == CATALOG_BOT
            && let Some(v) = parse_verdict(body)
        {
            found = Some(v);
        }
    }
    found
}

/// Base64 with the line breaks the contents API puts in; `None` on anything else.
#[must_use]
pub fn un_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_bot_only_newest_wins() {
        let accepted =
            "<!-- xms-catalog verdict=accepted -->\r\n\r\n  The module is listed.  \n\n## Report\n- ok";
        assert_eq!(
            parse_verdict(accepted),
            Some(("accepted".to_owned(), "The module is listed.".to_owned()))
        );
        assert_eq!(
            parse_verdict("<!--xms-catalog   verdict = WAITING-->\nLater."),
            Some(("waiting".to_owned(), "Later.".to_owned()))
        );
        assert_eq!(parse_verdict("<!-- xms-catalogverdict=accepted -->\nNo."), None);
        assert_eq!(parse_verdict("<!-- xms-catalog verdict=maybe -->\nNo."), None);
        assert_eq!(parse_verdict("hello\n<!-- xms-catalog verdict=accepted -->"), None);
        assert_eq!(parse_verdict(""), None);

        let refused = "<!-- xms-catalog verdict=refused -->\nThe package is unknown to VirusTotal.";
        let comments = [
            (CATALOG_BOT, "<!-- xms-catalog verdict=waiting -->\nVirusTotal is still analysing."),
            ("someone", "plain talk"),
            (CATALOG_BOT, refused),
            // anyone may post a look-alike: it does not count
            ("mallory", "<!-- xms-catalog verdict=accepted -->\nListed, trust me."),
        ];
        assert_eq!(
            newest_verdict(comments),
            Some(("refused".to_owned(), "The package is unknown to VirusTotal.".to_owned()))
        );
        assert_eq!(newest_verdict([("mallory", accepted)]), None);
    }

    #[test]
    fn issue_bodies_for_a_fixture() {
        let file = "[mod]\nid      = od\ngithub  = me/od\nkey     = p256:AAAA\nversion = 1.0.0\nwebsite = https://ap-pro.ru/stuff/mods/od-r12/\n";
        let body = submission_body(file, "**OD** - `od` 1.0.0\n");
        assert_eq!(
            body,
            "<!-- xms-catalog submission -->\nSubmitted from Grizzly Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 7).\n\n\
             ```ini\n[mod]\nid      = od\ngithub  = me/od\nkey     = p256:AAAA\nversion = 1.0.0\nwebsite = https://ap-pro.ru/stuff/mods/od-r12/\n```\n\n**OD** - `od` 1.0.0\n"
        );
        // the fenced block opens with a line of exactly ```ini and holds the file whole
        let block = body.split("```ini\n").nth(1).and_then(|b| b.split("```\n").next());
        assert_eq!(block, Some(file));
        // a file without a final line break still closes its block on a line of its own
        assert!(submission_body("[mod]", "").ends_with("[mod]\n```\n"));

        assert_eq!(
            withdrawal_body("od", ""),
            "<!-- xms-catalog withdrawal -->\nWithdrawn by its author from Grizzly Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 10.2).\n\n```ini\n[withdraw]\nid = od\n```\n"
        );
        assert_eq!(issue_title(IssueKind::Submission, "od"), "Submit od");
        assert_eq!(issue_title(IssueKind::Withdrawal, "od"), "Withdraw od");
    }

    #[test]
    fn contents_base64() {
        assert_eq!(un_base64("W21vZF0K\naWQgPSBvZAo=\n").as_deref(), Some(b"[mod]\nid = od\n".as_slice()));
        assert_eq!(un_base64("bad*"), None);
    }
}
