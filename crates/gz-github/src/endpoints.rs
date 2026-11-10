//! Where the requests go: GitHub, VirusTotal and the Mod Browser catalog - or, under
//! `-qa_github`, the local mock the environment names (`GRIZZLY_GITHUB_API`,
//! `GRIZZLY_GITHUB_WEB`, `GRIZZLY_VT_API`, `GRIZZLY_CATALOG_REPO`, `GRIZZLY_CATALOG_BASE`).
//! Outside a QA run no variable changes an address.

/// The client id of the editor's GitHub OAuth app (device flow). It is public by design - the
/// device flow has no client secret. Empty: the device sign-in is not offered and the account
/// comes from a pasted token, the environment or the GitHub CLI.
const CLIENT_ID: &str = "";

/// The Mod Browser catalog of Dead Air: Refined and where its public files are served; the game
/// carries the same mirrors (`xms_catalog.cpp`, `kMirrors`) - change both together.
pub const CATALOG_REPO: &str = "MMadmer/darf-mods";
pub const CATALOG_MIRRORS: [&str; 3] = [
    "https://mmadmer.github.io/darf-mods/",
    "https://cdn.jsdelivr.net/gh/MMadmer/darf-mods@main/public/",
    "https://raw.githubusercontent.com/MMadmer/darf-mods/main/public/",
];

/// Every address the clients use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub qa: bool,
    /// `https://api.github.com`, no trailing slash.
    pub api: String,
    /// `https://github.com`.
    pub web: String,
    /// `https://www.virustotal.com/api/v3`.
    pub vt: String,
    pub client_id: String,
    /// owner/repo of the catalog the editor submits to.
    pub catalog_repo: String,
    /// The base URLs of the catalog's public files, in the order to try; each ends with `/`.
    pub mirrors: Vec<String>,
    /// A QA run with its own API address: uploads go there instead of uploads.github.com.
    api_mocked: bool,
}

fn base(qa: bool, env: &dyn Fn(&str) -> Option<String>, variable: &str, real: &str) -> String {
    let given = if qa { env(variable).unwrap_or_default() } else { String::new() };
    let v = if given.is_empty() { real.to_owned() } else { given };
    v.trim_end_matches('/').to_owned()
}

impl Endpoints {
    /// The real services, or with `qa` what the environment names. `env` reads a variable.
    #[must_use]
    pub fn from_env(qa: bool, env: &dyn Fn(&str) -> Option<String>) -> Self {
        let qa_text = |name: &str| if qa { env(name).unwrap_or_default() } else { String::new() };
        let client_id = qa_text("GRIZZLY_GITHUB_CLIENT_ID");
        let catalog_repo = qa_text("GRIZZLY_CATALOG_REPO");
        let mirrors = if qa {
            // loopback only, like the game's own QA override
            let b = qa_text("GRIZZLY_CATALOG_BASE");
            if (b.starts_with("http://127.0.0.1:") || b.starts_with("http://localhost:")) && b.ends_with('/') { vec![b] } else { Vec::new() }
        } else {
            CATALOG_MIRRORS.iter().map(|m| (*m).to_owned()).collect()
        };
        Self {
            qa,
            api: base(qa, env, "GRIZZLY_GITHUB_API", "https://api.github.com"),
            web: base(qa, env, "GRIZZLY_GITHUB_WEB", "https://github.com"),
            vt: base(qa, env, "GRIZZLY_VT_API", "https://www.virustotal.com/api/v3"),
            client_id: if client_id.is_empty() { CLIENT_ID.to_owned() } else { client_id },
            catalog_repo: if catalog_repo.is_empty() { CATALOG_REPO.to_owned() } else { catalog_repo },
            mirrors,
            api_mocked: qa && !qa_text("GRIZZLY_GITHUB_API").is_empty(),
        }
    }

    /// From the process environment.
    #[must_use]
    pub fn from_process(qa: bool) -> Self {
        Self::from_env(qa, &|name| std::env::var(name).ok())
    }

    /// The token goes wherever the upload address points, and that address comes out of a
    /// response body: only GitHub's upload host (the mock's API host in a QA run) is trusted.
    #[must_use]
    pub fn upload_url_trusted(&self, url: &str) -> bool {
        let want = if self.api_mocked { origin(&self.api) } else { "https://uploads.github.com" };
        origin(url).eq_ignore_ascii_case(want) && !origin(url).is_empty()
    }

    /// The VirusTotal key follows an upload address VirusTotal hands out; only its own host.
    #[must_use]
    pub fn vt_url_trusted(&self, url: &str) -> bool {
        !origin(url).is_empty() && origin(url).eq_ignore_ascii_case(origin(&self.vt))
    }

    /// Whether the device sign-in can be offered.
    #[must_use]
    pub fn device_sign_in(&self) -> bool {
        !self.client_id.is_empty()
    }

    /// `https://github.com/<repo>/releases`.
    #[must_use]
    pub fn releases_page(&self, repo: &str) -> String {
        format!("{}/{repo}/releases", self.web)
    }

    /// `https://github.com/<repo>`.
    #[must_use]
    pub fn repo_page(&self, repo: &str) -> String {
        format!("{}/{repo}", self.web)
    }

    /// The address the game downloads `name` of the latest release of `repo` from.
    #[must_use]
    pub fn latest_download(&self, repo: &str, name: &str) -> String {
        format!("{}/{repo}/releases/latest/download/{}", self.web, url_encode(name))
    }
}

/// `scheme://host[:port]` of an address; empty when it has no scheme.
#[must_use]
pub fn origin(url: &str) -> &str {
    let Some(scheme) = url.find("://") else { return "" };
    match url.get(scheme + 3..).and_then(|rest| rest.find('/')) {
        Some(slash) => url.get(..scheme + 3 + slash).unwrap_or(url),
        None => url,
    }
}

/// Percent-encoding of everything but `A-Z a-z 0-9 - . _ ~`.
#[must_use]
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A ref keeps its slashes; everything between them is escaped.
#[must_use]
pub fn encode_ref(r: &str) -> String {
    r.split('/').map(url_encode).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn upload_address_check() {
        let real = Endpoints::from_env(false, &env_of(&[("GRIZZLY_GITHUB_API", "http://127.0.0.1:9000")]));
        // outside QA no variable moves an address
        assert_eq!(real.api, "https://api.github.com");
        assert!(real.upload_url_trusted("https://uploads.github.com/repos/a/b/releases/1/assets"));
        assert!(real.upload_url_trusted("HTTPS://UPLOADS.GITHUB.COM/x"));
        assert!(!real.upload_url_trusted("https://uploads.github.com.evil.example/x"));
        assert!(!real.upload_url_trusted("http://uploads.github.com/x"));
        assert!(!real.upload_url_trusted("https://api.github.com/x"));
        assert!(!real.upload_url_trusted("uploads.github.com/x"));
        assert!(!real.upload_url_trusted(""));

        let qa = Endpoints::from_env(true, &env_of(&[("GRIZZLY_GITHUB_API", "http://127.0.0.1:9000/")]));
        assert_eq!(qa.api, "http://127.0.0.1:9000");
        assert!(qa.upload_url_trusted("http://127.0.0.1:9000/uploads/repos/a/b/releases/1/assets"));
        assert!(!qa.upload_url_trusted("https://uploads.github.com/x"));
        assert!(!qa.upload_url_trusted("http://127.0.0.1:9001/x"));

        // a QA run without its own API keeps GitHub's upload host
        let qa_real = Endpoints::from_env(true, &env_of(&[]));
        assert!(qa_real.upload_url_trusted("https://uploads.github.com/x"));
    }

    #[test]
    fn qa_overrides() {
        let qa = Endpoints::from_env(
            true,
            &env_of(&[
                ("GRIZZLY_CATALOG_BASE", "http://127.0.0.1:5000/catalog/public/"),
                ("GRIZZLY_CATALOG_REPO", "qa/catalog"),
                ("GRIZZLY_GITHUB_CLIENT_ID", "qa-client"),
            ]),
        );
        assert_eq!(qa.mirrors, vec!["http://127.0.0.1:5000/catalog/public/".to_owned()]);
        assert_eq!(qa.catalog_repo, "qa/catalog");
        assert!(qa.device_sign_in());
        let remote = Endpoints::from_env(true, &env_of(&[("GRIZZLY_CATALOG_BASE", "https://example.com/catalog/")]));
        assert!(remote.mirrors.is_empty());
        let real = Endpoints::from_env(false, &env_of(&[("GRIZZLY_CATALOG_REPO", "evil/catalog")]));
        assert_eq!(real.catalog_repo, CATALOG_REPO);
        assert_eq!(real.mirrors.len(), 3);
        assert!(!real.device_sign_in());
    }

    #[test]
    fn encoding() {
        assert_eq!(url_encode("v1.0.0-r2"), "v1.0.0-r2");
        assert_eq!(url_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(encode_ref("rel/v 1"), "rel/v%201");
        assert_eq!(origin("https://h.example:8443/a/b"), "https://h.example:8443");
        assert_eq!(origin("https://h.example"), "https://h.example");
        assert_eq!(origin("nothing"), "");
    }
}
