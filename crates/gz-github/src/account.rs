//! The GitHub account: where its token comes from and what a token looks like.

use crate::secret::Secret;

/// Where the token came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccountSource {
    #[default]
    None,
    /// `GRIZZLY_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`.
    Environment,
    /// The Windows Credential Manager, written by the editor.
    Saved,
    /// `gh auth token`.
    GithubCli,
}

impl AccountSource {
    /// What the windows and the answers say about it.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            AccountSource::Environment => "environment variable",
            AccountSource::Saved => "token saved by the editor",
            AccountSource::GithubCli => "GitHub CLI (gh)",
            AccountSource::None => "not signed in",
        }
    }
}

/// The account a request goes out with.
#[derive(Clone, Debug, Default)]
pub struct Account {
    pub source: AccountSource,
    pub token: Secret,
    /// The variable that supplied it.
    pub detail: String,
}

impl Account {
    #[must_use]
    pub fn signed_in(&self) -> bool {
        self.source != AccountSource::None
    }
}

/// What a token looks like; keeps a pasted sentence out of the credential store.
#[must_use]
pub fn plausible_token(token: &str) -> bool {
    (20..=255).contains(&token.len())
        && token.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// The variables a token is read from, in order; a QA run reads its own alone.
pub(crate) fn token_variables(qa: bool) -> &'static [&'static str] {
    if qa { &["GRIZZLY_GITHUB_TOKEN"] } else { &["GRIZZLY_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] }
}

/// The Credential Manager target of the GitHub token; a QA run has its own.
pub(crate) fn credential_target(qa: bool) -> &'static str {
    if qa { "GrizzlyEditor:GitHub:qa" } else { "GrizzlyEditor:GitHub" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert!(plausible_token("ghp_0123456789abcdefghijABCDEF"));
        assert!(plausible_token("github_pat_11AAAA-bbbb_cccc0000"));
        assert!(!plausible_token("short"));
        assert!(!plausible_token("this is a pasted sentence, not a token"));
        assert!(!plausible_token(&"a".repeat(256)));
        assert_eq!(token_variables(true), &["GRIZZLY_GITHUB_TOKEN"]);
        assert_eq!(credential_target(true), "GrizzlyEditor:GitHub:qa");
    }
}
