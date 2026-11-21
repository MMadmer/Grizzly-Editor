//! The upload to the report hub (docs/REPORTS.md, "The hub"): one `multipart/form-data` POST of the
//! title, the text, the version, `product = grizzly-editor`, the kind and the package, with the build's
//! key as a bearer token and the report's id as its `Idempotency-Key` - a retry after a cut answer
//! stores nothing twice. The key and the bodies are never logged.
//!
//! A QA run (`-qa_github`) speaks to this computer alone: `GRIZZLY_REPORT_URL` names the mock, any
//! other address is the dead `127.0.0.1:9`, and the key is `GRIZZLY_REPORT_TOKEN`, never the build's.

use std::time::Duration;

use gz_editor::Cancel;
use serde_json::Value;

/// Where the editor's reports go: the hub the game's go to.
pub const HUB: &str = "https://dead-air-refined-madmer.mia0.amvera.tech/api/v1/reports";
const NOWHERE: &str = "http://127.0.0.1:9/api/v1/reports";
/// The build's key of the hub, as build.rs hid it.
const BUILD_HIDDEN: &str = env!("GRIZZLY_REPORT_UPLOAD_HIDDEN");

fn build_token() -> String {
    (0..BUILD_HIDDEN.len() / 2)
        .filter_map(|i| u8::from_str_radix(BUILD_HIDDEN.get(i * 2..i * 2 + 2)?, 16).ok())
        .map(|b| char::from(b ^ 0xA7))
        .collect()
}

/// Where a report goes and with what key.
pub struct Target {
    pub url: String,
    token: String,
}

impl Target {
    /// The hub, or the mock of a QA run. None: this build carries no key of the hub.
    #[must_use]
    pub fn of(qa: bool) -> Option<Self> {
        if qa {
            let url = std::env::var("GRIZZLY_REPORT_URL")
                .ok()
                .filter(|u| gz_base::web::is_loopback(u))
                .unwrap_or_else(|| NOWHERE.to_owned());
            let token =
                std::env::var("GRIZZLY_REPORT_TOKEN").unwrap_or_else(|_| "qa-report-token".to_owned());
            return Some(Self { url, token });
        }
        (!BUILD_HIDDEN.is_empty()).then(|| Self { url: HUB.to_owned(), token: build_token() })
    }

    /// Whether this build can send at all (the window's Send asks before it is pressed).
    #[must_use]
    pub fn available(qa: bool) -> bool {
        qa || !BUILD_HIDDEN.is_empty()
    }
}

/// What the report is.
pub struct Form<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub kind: &'a str,
    pub report_id: &'a str,
    pub zip: Option<&'a [u8]>,
}

/// The hub's answer to a report it took.
#[derive(Debug)]
pub struct Taken {
    /// The hub's own number of the report.
    pub id: Value,
    /// The hub had it already (a retry of one that arrived).
    pub duplicate: bool,
}

fn body(form: &Form<'_>, boundary: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(form.zip.map_or(0, <[u8]>::len) + 16 * 1024);
    let mut field = |name: &str, value: &str| {
        b.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        b.extend_from_slice(value.as_bytes());
        b.extend_from_slice(b"\r\n");
    };
    field("title", form.title);
    field("description", form.text);
    field("version", env!("CARGO_PKG_VERSION"));
    field("product", "grizzly-editor");
    field("kind", form.kind);
    if let Some(zip) = form.zip {
        b.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"report\"; filename=\"grizzly-report-{}.zip\"\r\nContent-Type: application/zip\r\n\r\n",
                form.report_id
            )
            .as_bytes(),
        );
        b.extend_from_slice(zip);
        b.extend_from_slice(b"\r\n");
    }
    b.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    b
}

/// What a try came to when the hub did not take the report.
enum Miss {
    /// Worth another try: no answer, a cut one, the hub busy.
    Again(String),
    /// The hub refused it: asking again changes nothing.
    Refused(String),
}

fn hub_message(status: u16, body: &[u8]) -> String {
    let said = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    let what = match status {
        401 | 403 => "The report hub did not take this build's key".to_owned(),
        413 => "The report is larger than the report hub takes".to_owned(),
        400 | 422 => "The report hub refused the report".to_owned(),
        429 | 503 => "The report hub is busy".to_owned(),
        507 => "The report hub is full".to_owned(),
        _ => "The report hub could not store the report".to_owned(),
    };
    if said.is_empty() { format!("{what}.") } else { format!("{what}: {said}.") }
}

async fn try_once(
    client: &reqwest::Client,
    target: &Target,
    form: &Form<'_>,
    cancel: &Cancel,
) -> Result<Taken, Miss> {
    let boundary = format!("grizzly-{}", form.report_id);
    let request = client
        .post(&target.url)
        .bearer_auth(&target.token)
        .header("Idempotency-Key", form.report_id)
        .header(reqwest::header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
        .body(body(form, &boundary));
    let response = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(Miss::Refused("Stopped.".to_owned())),
        r = request.send() => r.map_err(|e| Miss::Again(if e.is_timeout() {
            "The report hub did not answer in time.".to_owned()
        } else {
            "The report hub could not be reached.".to_owned()
        }))?,
    };
    let status = response.status().as_u16();
    let bytes = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(Miss::Refused("Stopped.".to_owned())),
        b = response.bytes() => b.map_err(|_| Miss::Again("The report hub's answer was cut short.".to_owned()))?,
    };
    match status {
        200..=299 => {
            let v: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            Ok(Taken {
                id: v.pointer("/report/id").cloned().unwrap_or(Value::Null),
                duplicate: v.get("duplicate").and_then(Value::as_bool).unwrap_or(false),
            })
        }
        408 | 429 | 500..=599 if status != 507 => Err(Miss::Again(hub_message(status, &bytes))),
        _ => Err(Miss::Refused(hub_message(status, &bytes))),
    }
}

/// Sends `form` to `target`, trying three times over half a minute when the hub is busy or the line
/// is cut. The error is a sentence for the author.
pub async fn send(target: &Target, form: &Form<'_>, cancel: &Cancel) -> Result<Taken, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Grizzly-Editor/{}", env!("CARGO_PKG_VERSION")))
        .tls_backend_native()
        .tls_version_min(reqwest::tls::Version::TLS_1_2)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "The network client could not start.".to_owned())?;
    let mut last = String::new();
    for wait in [0_u64, 3, 10] {
        if wait > 0 {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err("Stopped.".to_owned()),
                () = tokio::time::sleep(Duration::from_secs(wait)) => {}
            }
        }
        match try_once(&client, target, form, cancel).await {
            Ok(taken) => return Ok(taken),
            Err(Miss::Refused(why)) => return Err(why),
            Err(Miss::Again(why)) => last = why,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_the_hubs_form() {
        let form =
            Form { title: "Crash", text: "It fell over.", kind: "bug", report_id: "abc", zip: Some(b"PK") };
        let b = String::from_utf8_lossy(&body(&form, "B")).into_owned();
        assert!(b.starts_with("--B\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nCrash\r\n"));
        assert!(b.contains("name=\"product\"\r\n\r\ngrizzly-editor\r\n"));
        assert!(b.contains("name=\"report\"; filename=\"grizzly-report-abc.zip\"\r\nContent-Type: application/zip\r\n\r\nPK\r\n"));
        assert!(b.ends_with("--B--\r\n"));
        let words = Form { zip: None, ..form };
        assert!(!String::from_utf8_lossy(&body(&words, "B")).contains("name=\"report\""));
    }

    #[test]
    fn a_qa_run_reads_only_this_computer() {
        let t = Target::of(true);
        assert!(t.is_some_and(|t| gz_base::web::is_loopback(&t.url)));
    }

    #[test]
    fn the_hubs_refusal_is_a_sentence() {
        assert_eq!(
            hub_message(400, br#"{"error":{"code":"x","message":"title is required"}}"#),
            "The report hub refused the report: title is required."
        );
        assert_eq!(hub_message(413, b""), "The report is larger than the report hub takes.");
    }
}
