//! The HTTP exchange: one request, its answer or the sentence why there was none.
//! `reqwest` on `native-tls`, so TLS trusts the Windows certificate store - a machine whose
//! antivirus inspects HTTPS keeps working - and the system's proxy settings apply.
//!
//! Three clients share the session: the API (no redirects - a token never follows one), the
//! uploads (the same, with the long wait GitHub needs before it answers a large asset) and the
//! public files (redirects followed, never a token on board).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gz_editor::{Cancel, Log};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderName, HeaderValue};
use reqwest::{Method, redirect};
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::CANCELLED;

const MAX_RESPONSE: u64 = 32 << 20;
const UPLOAD_CHUNK: usize = 256 << 10;

/// Which client a request takes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Api,
    Upload,
    Public,
}

pub(crate) struct Clients {
    api: reqwest::Client,
    upload: reqwest::Client,
    public: reqwest::Client,
}

fn builder(read: Duration) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent("Grizzly-Editor")
        .tls_backend_native()
        .tls_version_min(reqwest::tls::Version::TLS_1_2)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(read)
}

impl Clients {
    pub(crate) fn build() -> Result<Self, String> {
        let fail = |e: reqwest::Error| format!("the network client could not start: {e}");
        Ok(Self {
            api: builder(Duration::from_secs(60)).redirect(redirect::Policy::none()).build().map_err(fail)?,
            // GitHub answers an upload only after it has taken the whole asset in - for a package
            // of a gigabyte that is well past the default minute
            upload: builder(Duration::from_secs(600))
                .redirect(redirect::Policy::none())
                .build()
                .map_err(fail)?,
            public: builder(Duration::from_secs(60))
                .redirect(redirect::Policy::limited(10))
                .build()
                .map_err(fail)?,
        })
    }
}

/// Progress of a streamed body: bytes of the file sent, of its total.
pub(crate) type Progress = Arc<dyn Fn(u64, u64) + Send + Sync>;

pub(crate) enum Body {
    None,
    Bytes {
        data: Vec<u8>,
        content_type: &'static str,
    },
    /// A file streamed from disk, with what goes around it (a multipart form's part headers and
    /// closing boundary).
    File {
        path: PathBuf,
        prefix: Vec<u8>,
        suffix: Vec<u8>,
        content_type: String,
        progress: Option<Progress>,
    },
}

/// A request. Not `Debug`: its headers may carry a token.
pub(crate) struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Body,
    pub kind: Kind,
    /// Answers above it fail as too large.
    pub limit: u64,
}

impl Request {
    pub(crate) fn new(kind: Kind, method: Method, url: impl Into<String>) -> Self {
        Self { method, url: url.into(), headers: Vec::new(), body: Body::None, kind, limit: MAX_RESPONSE }
    }

    pub(crate) fn header(mut self, name: HeaderName, value: &str) -> Self {
        if let Ok(v) = HeaderValue::from_str(value) {
            self.headers.push((name, v));
        }
        self
    }

    /// A header that holds a secret: kept out of every debug print of the request.
    pub(crate) fn secret_header(mut self, name: HeaderName, value: &str) -> Self {
        if let Ok(mut v) = HeaderValue::from_str(value) {
            v.set_sensitive(true);
            self.headers.push((name, v));
        }
        self
    }

    pub(crate) fn json(mut self, body: &Value) -> Self {
        self.body = Body::Bytes { data: body.to_string().into_bytes(), content_type: "application/json" };
        self
    }

    pub(crate) fn form(mut self, form: String) -> Self {
        self.body =
            Body::Bytes { data: form.into_bytes(), content_type: "application/x-www-form-urlencoded" };
        self
    }
}

/// An answer. `status` 0 = none at all, and `error` says why.
#[derive(Clone, Debug, Default)]
pub(crate) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub error: String,
    /// The answer passed the request's limit.
    pub too_large: bool,
}

impl Response {
    fn fail(error: impl Into<String>) -> Self {
        Self { error: error.into(), ..Self::default() }
    }

    pub(crate) fn json(&self) -> Option<Value> {
        crate::json::parse(&self.body)
    }
}

/// What the author reads about a request that got no answer. The raw error is for the log.
fn network_error(e: &reqwest::Error, log: &Log) -> String {
    if e.is_builder() {
        return "invalid address".to_owned();
    }
    if e.is_timeout() {
        return "the connection timed out".to_owned();
    }
    let mut chain = String::new();
    let mut source: Option<&dyn std::error::Error> = Some(e);
    while let Some(s) = source {
        chain.push_str(&s.to_string().to_ascii_lowercase());
        chain.push(' ');
        source = s.source();
    }
    if chain.contains("certificate") || chain.contains("cert ") {
        return "the server's certificate was not accepted".to_owned();
    }
    if chain.contains("tls")
        || chain.contains("ssl")
        || chain.contains("handshake")
        || chain.contains("schannel")
    {
        return "the secure connection failed".to_owned();
    }
    if chain.contains("dns")
        || chain.contains("resolve")
        || chain.contains("no such host")
        || chain.contains("name or service")
    {
        return "no connection: the server's name was not found".to_owned();
    }
    if chain.contains("proxy") {
        return "the proxy settings could not be used".to_owned();
    }
    if e.is_connect() {
        return "no connection to the server".to_owned();
    }
    if e.is_body() || e.is_request() {
        return "the connection was lost".to_owned();
    }
    // the details are for whoever reads the log, not for the sentence the message ends
    log.warn(format!("[XMS] network error: {e}"));
    "the connection failed".to_owned()
}

struct Upload {
    file: tokio::fs::File,
    prefix: Option<Vec<u8>>,
    suffix: Option<Vec<u8>>,
    sent: u64,
    total: u64,
    progress: Option<Progress>,
}

/// The body of a file upload as a stream: the prefix, the file in chunks, the suffix.
fn upload_stream(
    up: Upload,
) -> impl futures::Stream<Item = Result<Vec<u8>, std::io::Error>> + Send + 'static {
    futures::stream::unfold(up, |mut up| async move {
        if let Some(prefix) = up.prefix.take().filter(|p| !p.is_empty()) {
            return Some((Ok(prefix), up));
        }
        if up.sent < up.total {
            let want = usize::try_from((up.total - up.sent).min(UPLOAD_CHUNK as u64)).unwrap_or(UPLOAD_CHUNK);
            let mut chunk = vec![0u8; want];
            let read = up.file.read(&mut chunk).await;
            return match read {
                Ok(0) => Some((
                    Err(std::io::Error::other("cannot read the file to upload")),
                    Upload { total: up.sent, ..up },
                )),
                Ok(n) => {
                    chunk.truncate(n);
                    up.sent += n as u64;
                    if let Some(p) = &up.progress {
                        p(up.sent, up.total);
                    }
                    Some((Ok(chunk), up))
                }
                Err(e) => Some((Err(e), Upload { total: up.sent, ..up })),
            };
        }
        let suffix = up.suffix.take().filter(|s| !s.is_empty())?;
        Some((Ok(suffix), up))
    })
}

async fn exchange(clients: &Clients, log: &Log, rq: Request) -> Response {
    let client = match rq.kind {
        Kind::Api => &clients.api,
        Kind::Upload => &clients.upload,
        Kind::Public => &clients.public,
    };
    let mut b = client.request(rq.method, &rq.url);
    for (name, value) in rq.headers {
        b = b.header(name, value);
    }
    match rq.body {
        Body::None => {}
        Body::Bytes { data, content_type } => b = b.header(CONTENT_TYPE, content_type).body(data),
        Body::File { path, prefix, suffix, content_type, progress } => {
            let Ok(file) = tokio::fs::File::open(&path).await else {
                return Response::fail("cannot read the file to upload");
            };
            let Ok(meta) = file.metadata().await else {
                return Response::fail("cannot read the file to upload");
            };
            let total = meta.len();
            let wrapping = (prefix.len() + suffix.len()) as u64;
            // a release asset is under 2 GiB anyway
            if total + wrapping > u64::from(u32::MAX) {
                return Response::fail("the file is too large to upload");
            }
            let length = total + wrapping;
            let stream = upload_stream(Upload {
                file,
                prefix: Some(prefix),
                suffix: Some(suffix),
                sent: 0,
                total,
                progress,
            });
            b = b
                .header(CONTENT_TYPE, content_type)
                .header(CONTENT_LENGTH, length)
                .body(reqwest::Body::wrap_stream(stream));
        }
    }
    let mut resp = match b.send().await {
        Ok(r) => r,
        Err(e) => return Response::fail(network_error(&e, log)),
    };
    let mut out = Response { status: resp.status().as_u16(), ..Response::default() };
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if (out.body.len() + chunk.len()) as u64 > rq.limit {
                    return Response { too_large: true, ..Response::fail("the answer is too large") };
                }
                out.body.extend_from_slice(&chunk);
            }
            Ok(None) => return out,
            Err(e) => return Response::fail(network_error(&e, log)),
        }
    }
}

/// Sends a request and waits for its answer, or for the job to be stopped.
pub(crate) async fn send(
    clients: &Result<Clients, String>,
    log: &Log,
    rq: Request,
    cancel: &Cancel,
) -> Response {
    if cancel.is_cancelled() {
        return Response::fail(CANCELLED);
    }
    let clients = match clients {
        Ok(c) => c,
        Err(e) => return Response::fail(e.clone()),
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => Response::fail(CANCELLED),
        r = exchange(clients, log, rq) => if cancel.is_cancelled() { Response::fail(CANCELLED) } else { r },
    }
}

/// An interruptible pause: a stopped job does not sit one out.
pub(crate) async fn pause(cancel: &Cancel, ms: u64) {
    tokio::select! {
        () = cancel.cancelled() => {}
        () = tokio::time::sleep(Duration::from_millis(ms)) => {}
    }
}
