# Brief: publishing and the Mod Browser (`gz-github`, `gz-publish`, `gz-catalog`)

The network side: GitHub releases, VirusTotal, the catalog's issues. Read
`docs/rust/CONVENTIONS.md` ("An area crate") and `docs/rust/STATE.md` first; then `CLAUDE.md`
sections "Mod releases" (from "Publish Release" on) and "The Mod Browser" - long and binding:
every rule there was paid for - and the rustdoc at the top of `gz-release`, `gz-mod`,
`gz-editor`.

You own: `crates/gz-github/**`, `crates/gz-publish/**`, `crates/gz-catalog/**` (new;
`src/lib.rs` before `Cargo.toml`) and the registration lines in `crates/grizzly`.

## The C++ that is the spec

- `Source/Editors/XrECore/Editor/EditorGithub.h/.cpp` (2.1k): the REST client, where the token
  comes from, the upload address check, the VirusTotal pacing (`VtPace`), the catalog requests
  (`CatalogSubmit`, `CatalogIssue`, `CatalogWithdraw`), the QA overrides.
- `Source/Editors/XrECore/Editor/EditorModPublish.cpp` (2.4k): the publish job (draft -> verify
  -> latest), `ScanFirst`, `FreshTag`, `releases\deleted.tags`, listing / editing / deleting
  releases. Its drawing is UI.
- `Source/Editors/XrECore/Editor/EditorCatalogSubmit.cpp` (1.8k) and `EditorVirusTotal.cpp`
  (300): the jobs `check`, `vt`, `submit`, `submission`, `withdraw`; the scan.
- Handlers (frozen wire): `mod_publish`, `mod_releases`, `mod_catalog` in
  `Source/Editors/LevelEditor/Editor/Mcp/LevelMcpMod.cpp` and where they delegate. Schemas and
  the SAFETY paragraphs: `crates/gz-mcp/tools.json`.
- What QA drives and asserts: `tools/qa/test_mod_publish.py`, `tools/qa/test_mod_catalog.py`,
  the mock `tools/qa/github_mock/` (Go). Publishing is never tested against GitHub itself.

## Design

**`gz-github` - the client.**
- `reqwest` (workspace: `native-tls`, so TLS trusts the **Windows certificate store** - a
  machine whose antivirus inspects HTTPS must keep working; never a bundled root list), async,
  on the editor's runtime. One client per session; every request cancellable by the job's
  token (`tokio::select!` against `ctx.cancel.cancelled()`).
- The token: the environment, the Windows Credential Manager (`keyring`), `gh auth token`, in
  that order. It is never written into the project, a log or an MCP answer, and is sent to
  `api.github.com` / `uploads.github.com` only - the upload address comes out of a response
  body and is checked before the token goes there. Under `-qa_github`
  (`editor.flags.qa_github`) the client talks to `GRIZZLY_GITHUB_API` / `GRIZZLY_GITHUB_WEB` and
  reads only `GRIZZLY_GITHUB_TOKEN` and a separate QA credential, so a real token cannot reach
  a mock. Hold the token in a type that zeroes itself and does not implement `Debug` with its
  value.
- The device sign-in stays hidden until a client id exists (`kClientId` is empty in the C++).
- Errors are sentences naming what failed; the raw status stays in the MCP `error`.
- VirusTotal: one shared limiter for the free API's 4 requests a minute, used by every job
  (a `tokio` semaphore plus a timer in the service); a 429 is waited out a few times; a refused
  key or a used-up quota ends the scan. Waits shrink under the `GRIZZLY_VT_*_MS` variables.

**`gz-publish` - Publish Release and Releases.**
- A job (`"publish"`): package (through `gz-release`), then - for a module the catalog index
  lists - scan first (`ScanFirst`: a listed module's release leaves only with a VirusTotal
  report of every package; not listed: no scan and no VirusTotal request; the index not
  answering: scan with a key, publish with a note without one), then GitHub: assets into a
  DRAFT, every stored asset checked against the package result (name, size, SHA-256), and only
  then published with `make_latest`. Uploads run in parallel, bounded (3). A published tag is
  never overwritten; a leftover draft of the tag is reused; a tag this project deleted within
  the last hour is not offered again (`FreshTag`: `v<version>`, `-r2`, `-r3`, ...); a latest
  release that carries another module's descriptor is refused.
- Releases: list (drafts included), select, edit (`EditRelease` always sends `make_latest`
  with the mark's CURRENT place), delete - the release FIRST, the tag second; a draft owns no
  tag. The MCP path asks nothing. `mod_publish action=save_token` is the one request that
  carries a token; it stores it.
- `mod_publish` answers when the job has ended while other commands go on; the job's states and
  the `virustotal { state, packages }` block as the C++ answers them.

**`gz-catalog` - Submit to Mod Browser.**
- One job at a time, started by the window or MCP alike: `check` (section 7 of MOD_CATALOG
  against the latest release fetched anonymously, the way the game fetches it), `vt` (the
  author's scan: look every package up by SHA-256, upload what VirusTotal does not know - only
  the bytes the descriptor pins - wait for its report, about ten minutes at most, cancellable,
  with stage text), `submit` (check, scan, then the issue), `submission`, `withdraw`.
- A submission is an ISSUE of the catalog repository, not a pull request: the title, the marker
  comment, the fenced `ini` block holding exactly the text of `mods/<id>.ltx`, one open
  submission per module and account (submitting again EDITS it), nothing sent when the default
  branch already holds this text. The verdict in force is the newest comment starting with the
  verdict marker **written by `github-actions[bot]`** - anyone can post a look-alike. Withdrawal
  is the same mechanics with its own marker and needs neither key.
- The package states (`clean`, `warn`, `block`, `analysing`, `unknown`, `too_large`,
  `unavailable`, `no_key`), the `required` level that is never shown as optional, and every
  message: port them word for word.
- The catalog repository and its mirrors are constants shared with the game
  (`MMadmer/darf-mods`); under `-qa_github` they come from `GRIZZLY_CATALOG_REPO` /
  `GRIZZLY_CATALOG_BASE`.

## Safety, in code and not only in words

- These commands put a module in front of players. Keep the checks where the C++ has them and
  add none that can be switched off by an argument; no command acts on another author's module
  or on the catalog repository beyond the issue mechanics above.
- No secret in a log line, an error text, a `Debug` output or a reply.

## Done means

- `bash tools/rust/cg.sh check -p gz-github -p gz-publish -p gz-catalog --tests` clean, zero
  warnings. Tests written (not run): `FreshTag`, the upload address check, verdict parsing
  (bot-only, newest wins), issue bodies for a fixture, the limiter's pacing with shrunk waits.
- APIs at the top of each `lib.rs`; commands registered.

## Report

Under 300 words: commands, deviations from the C++ wire, anything about secrets handling you
want checked, what is left. Then add rows for the three crates to `docs/rust/STATE.md`.
