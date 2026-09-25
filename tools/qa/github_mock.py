#!/usr/bin/env python3
"""Local stand-in for the GitHub and VirusTotal endpoints the editor talks to.

Covers, for Publish Release: the REST API of the mod's own repository (--repo:
repository, releases, release assets, uploads, tag refs), the anonymous web
download path (releases/latest/download -> CDN redirect) and the OAuth device
flow. For Submit to Mod Browser: GET /user and the catalog repository
(--catalog-repo) - the repository, its files (GET contents), its issues (list,
open, read, edit) with their comments - and the catalog's public files on the
web (/catalog/public/..). For the package checks: VirusTotal's v3 file API
under /vt/api/v3 (files, files/upload_url, analyses) with the author's own key.
A set of fault switches comes with them (see --help). All state lives in
memory. Self-test: test_github_mock.py.

The first stdout line is {"port": N}. Nothing else is printed unless a request
handler fails, so a harness may leave the pipes unread.

Harness routes take no token and are never logged:
  GET  /__state       the releases, tags, upload count and request log
  POST /__shutdown
  GET  /__catalog     {"repositories": {"<catalog>": {"branches": {"<name>":
                      {"sha", "message", "files": {path: text},
                      "files_base64": {path: base64}}}}}, "issues": [the issue
                      JSON plus "comment_list": [the comment JSON]]}, oldest first
  POST /__catalog/file?path=P[&branch=B]
                      the request body becomes P in one commit on B (main): the
                      catalog committing an accepted file or publishing its index
  POST /__catalog/issue?creator=LOGIN&title=T[&pull=1]
                      an issue opened by anyone, the request body (UTF-8, may be
                      empty) its text; pull=1 makes it an entry standing for a
                      pull request
  POST /__catalog/comment?number=N[&author=LOGIN][&count=K]
                      K comments (1..1000, default 1) on N, the request body
                      (UTF-8) their text; answers the list of them
  POST /__catalog/close?number=N[&reason=completed|not_planned][&by=LOGIN]
                      closes N, which has to be open (422), as completed unless
                      reason= says otherwise
  GET  /__vt, POST /__vt/seed
                      VirusTotal's files and analyses; verdicts for known files
  author= and by= default to github-actions[bot], the account a workflow writes
  as: that is how a test plays the catalog's workflow - and a stranger forging it.

Behaviour worth knowing before writing a client against it:
  * Every response closes its connection. The request body is always consumed
    first, so an error answer is never lost to a connection reset.
  * API routes (/user, /repos, /upload) demand a User-Agent header, as GitHub
    does, then the bearer token, then the repository name.
  * "authorized" in the request log means the request carried an Authorization
    header, whatever its value. Web, CDN, device-flow, /catalog/public and /vt/
    requests must show false: a true there is a client leaking its token.
    "vt_key" means it carried an x-apikey header, whatever its value: only /vt/
    requests may show true.
  * Publishing a release creates its git tag, deleting the release leaves the tag
    behind, and deleting the tag of a release that is still published turns that
    release into a draft - all three as on GitHub, which is why a client that
    removes a release removes the release first and the tag second.
  * --repo keeps its releases-only model. The catalog has files and issues and
    nothing else - no forks, git refs, content writes or pull request routes: a
    submission is an issue, and the catalog's workflow judges it, comments,
    commits the file itself and closes the issue.
  * Issues and the entries standing for pull requests share one numbering from
    1, as on GitHub, and GET issues lists both, newest first: state=open by
    default, creator= matched case-insensitively, per_page / page as every list
    here. A pull entry carries "pull_request", a plain issue no such key. An
    issue without text has the body null.
  * POST issues and POST issues/{n}/comments write as --login; a comment goes on
    any issue, open or closed. PATCH issues/{n} needs --login to be the issue's
    author or the catalog's owner (403 "Must have admin rights to Repository."
    otherwise), and an author who is not the owner cannot reopen what somebody
    else closed. Closing sets closed_at, closed_by and state_reason (completed
    unless given); reopening clears the first two and says reopened. The
    state_reason counts only when the state changes, and then it has to fit the
    change. A refused PATCH changes nothing; an accepted one, a comment and a
    closing move updated_at.
  * VirusTotal: an Authorization header on a /vt/ request answers 400 before the
    key is looked at, a wrong or missing x-apikey 401. Only requests with the
    right key count toward --vt-rate, and a 429 does not count.
  * The VirusTotal size limits apply to the request body, the multipart envelope
    included: a file of exactly --vt-max-upload bytes does not fit POST /files.
  * An upload URL is spent by the first POST that reaches it with the right key,
    whatever that POST ends with.
  * Every upload queues the file's report for --vt-queue lookups; GET /files/{sha}
    and GET /analyses/{id} of a queued analysis both count. A known file uploaded
    again keeps its verdicts, a new one gets --vt-upload-results, and a seeded
    one reports size 0 until it is uploaded.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import secrets
import socket
import socketserver
import struct
import sys
import threading
import time
import traceback
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, quote, unquote, urlsplit


FIRST_RELEASE_ID = 250000001
FIRST_ASSET_ID = 310000001
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
MAKE_LATEST_VALUES = ("true", "false", "legacy")
READ_CHUNK = 1 << 20
# Timer jitter on the client must not turn an honest poll into slow_down.
POLL_GRACE_SECONDS = 0.1
DEFAULT_BRANCH = "main"
CATALOG_ID = 2
USER_ID = 1001
# Issue ids on github.com no longer fit a signed 32-bit integer; an issue's number is added to this.
ISSUE_ID_BASE = 3000000000
ISSUE_STATES = ("open", "closed")
# The reasons GitHub records when an issue is closed, and the one when it is reopened.
CLOSE_REASONS = ("completed", "not_planned")
REOPENED = "reopened"
FIRST_COMMENT_ID = 2150000001
# The account GitHub Actions workflows comment as, with its id on github.com.
ACTIONS_BOT = "github-actions[bot]"
ACTIONS_BOT_ID = 41898282
# The contents API breaks its base64 after every 60 characters.
CONTENT_LINE = 60
VT_CATEGORIES = ("malicious", "suspicious", "undetected", "harmless", "type-unsupported", "timeout")
VT_DETECTED = ("malicious", "suspicious")
VT_DEFAULT_RESULTS = {"EngineA": "undetected", "EngineB": "undetected", "EngineC": "harmless"}
VT_UPLOAD_URL_LIMIT = 681574400
VT_RATE_WINDOW = 60.0
PART_HEADERS_LIMIT = 16384
HEADER_PARAM = re.compile(r';\s*([^\s=;]+)\s*=\s*("(?:[^"\\]|\\.)*"|[^;]*)')


class ApiError(Exception):
    """An error answer: a status plus a JSON payload in GitHub's shape."""

    def __init__(self, status: int, message: str = "", payload: dict | None = None, headers: dict | None = None):
        super().__init__(message)
        self.status = status
        self.payload = payload if payload is not None else {"message": message}
        self.headers = headers or {}


def not_found() -> ApiError:
    return ApiError(404, "Not Found")


def validation_failed(resource: str, code: str, field_name: str) -> ApiError:
    errors = [{"resource": resource, "code": code, "field": field_name}]
    return ApiError(422, payload={"message": "Validation Failed", "errors": errors})


def parse_id(text: str) -> int:
    if not text.isdigit():
        raise not_found()
    return int(text)


def read_flag(body: dict, key: str, default: bool, resource: str = "Release") -> bool:
    value = body.get(key, default)
    if not isinstance(value, bool):
        raise validation_failed(resource, "invalid", key)
    return value


def read_text(body: dict, key: str, default: str | None, resource: str = "Release") -> str | None:
    value = body.get(key)
    if value is None:
        return default
    if not isinstance(value, str):
        raise validation_failed(resource, "invalid", key)
    return value


def no_admin_rights() -> ApiError:
    return ApiError(403, "Must have admin rights to Repository.")


def harness_text(raw: bytes, what: str) -> str:
    """A harness request's body as text: UTF-8, or a 400."""
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        raise ApiError(400, f"the {what} is not UTF-8") from None


def vt_error(status: int, code: str, message: str) -> ApiError:
    return ApiError(status, payload={"error": {"code": code, "message": message}})


def timestamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_range(header: str | None, total: int) -> tuple[int, int] | None:
    """Resolve a single bytes range. None means the whole body: no header, or a form this mock ignores."""
    unit, _, spec = (header or "").partition("=")
    first_text, dash, last_text = spec.strip().partition("-")
    if unit.strip() != "bytes" or not dash or "," in spec:
        return None
    if first_text:
        if not first_text.isdigit() or (last_text and not last_text.isdigit()):
            return None
        first = int(first_text)
        if last_text and int(last_text) < first:
            return None
        last = min(int(last_text), total - 1) if last_text else total - 1
    else:
        # bytes=-n: the last n bytes.
        if not last_text.isdigit():
            return None
        first, last = max(0, total - int(last_text)), total - 1
    if first > last:
        raise ApiError(416, "Range Not Satisfiable", headers={"Content-Range": f"bytes */{total}"})
    return first, last


def page_of(items: list, query: dict) -> list:
    """The slice ?per_page= (30 by default, 100 at most) and ?page= (from 1) select."""

    def number(key: str, default: int) -> int:
        text = query.get(key, "")
        return int(text) if text.isdigit() and int(text) > 0 else default

    per_page = min(number("per_page", 30), 100)
    start = (number("page", 1) - 1) * per_page
    return items[start:start + per_page]


def git_blob_sha(data: bytes) -> str:
    digest = hashlib.sha1(b"blob %d\0" % len(data))
    digest.update(data)
    return digest.hexdigest()


def wrapped_base64(data: bytes) -> str:
    text = base64.b64encode(data).decode("ascii")
    return "".join(text[start:start + CONTENT_LINE] + "\n" for start in range(0, len(text), CONTENT_LINE))


def header_params(value: str) -> tuple[str, dict[str, str]]:
    """'form-data; name="file"' -> ("form-data", {"name": "file"}), quotes removed."""
    main, _, rest = value.partition(";")
    params = {}
    for match in HEADER_PARAM.finditer(";" + rest):
        text = match.group(2).strip()
        if len(text) >= 2 and text[0] == text[-1] == '"':
            text = re.sub(r"\\(.)", r"\1", text[1:-1])
        params[match.group(1).lower()] = text
    return main.strip().lower(), params


def multipart_boundary(content_type: str | None) -> str:
    kind, params = header_params(content_type or "")
    boundary = params.get("boundary", "")
    if kind != "multipart/form-data" or not 0 < len(boundary) <= 70:
        raise vt_error(400, "BadRequestError", "Expected a multipart/form-data body with a boundary")
    return boundary


def read_vt_results(value, source: str) -> dict[str, str]:
    """{"<engine>": "<category>"} with categories VirusTotal uses; ValueError otherwise."""
    if not isinstance(value, dict):
        raise ValueError(f"{source}: expected a JSON object of engine names")
    for engine, category in value.items():
        if category not in VT_CATEGORIES:
            raise ValueError(f"{source}: {engine} has the category {category!r}, not one of {', '.join(VT_CATEGORIES)}")
    return dict(value)


def read_vt_seed(value) -> dict[str, dict[str, str]]:
    """{"<sha256>": {"<engine>": "<category>"}}, hashes in lower case; ValueError otherwise."""
    if not isinstance(value, dict):
        raise ValueError("expected a JSON object keyed by SHA-256")
    seed = {}
    for sha, results in value.items():
        if not re.fullmatch(r"[0-9a-fA-F]{64}", sha):
            raise ValueError(f"{sha!r} is not a SHA-256")
        seed[sha.lower()] = read_vt_results(results, sha)
    return seed


def vt_stats(results: dict[str, str]) -> dict[str, int]:
    stats = dict.fromkeys(VT_CATEGORIES, 0)
    for category in results.values():
        stats[category] += 1
    return stats


def read_tree(folder: str) -> dict[str, bytes]:
    """Every file below `folder`, keyed by its relative path with '/' separators."""
    files = {}
    for root, dirs, names in os.walk(folder):
        dirs.sort()
        for name in sorted(names):
            path = os.path.join(root, name)
            with open(path, "rb") as file:
                files[os.path.relpath(path, folder).replace(os.sep, "/")] = file.read()
    return files


@dataclass
class Asset:
    id: int
    name: str
    content_type: str
    data: bytes | bytearray
    sha256: str
    state: str = "uploaded"


@dataclass
class Release:
    id: int
    tag_name: str
    name: str
    body: str
    draft: bool
    prerelease: bool
    untagged: str
    created_at: str = ""
    published_at: str | None = None
    assets: list[Asset] = field(default_factory=list)

    @property
    def slug(self) -> str:
        """The tag as the web URLs spell it: a draft has no tag there yet."""
        return f"untagged-{self.untagged}" if self.draft else self.tag_name

    @property
    def eligible(self) -> bool:
        return not self.draft and not self.prerelease


@dataclass
class Commit:
    sha: str
    files: dict[str, bytes]
    parents: tuple[str, ...]
    message: str


@dataclass
class Comment:
    id: int
    body: str
    user: dict
    created_at: str


@dataclass
class Issue:
    number: int
    title: str
    body: str
    user: dict
    created_at: str
    updated_at: str
    # An entry standing for a pull request: GitHub lists those with the issues.
    pull: bool = False
    state: str = "open"
    state_reason: str | None = None
    closed_at: str | None = None
    closed_by: dict | None = None
    # Oldest first.
    comments: list[Comment] = field(default_factory=list)


@dataclass
class Catalog:
    """The Mod Browser catalog: branches over the mock's commit store, and issues numbered from 1."""

    owner: str
    name: str
    branches: dict[str, str] = field(default_factory=dict)
    issues: list[Issue] = field(default_factory=list)

    @property
    def full_name(self) -> str:
        return f"{self.owner}/{self.name}"


@dataclass
class VtFile:
    sha256: str
    size: int
    results: dict[str, str]
    # Lookups that will still find the report queued.
    queued: int = 0
    uploads: int = 0
    analysis_date: int | None = None
    names: list[str] = field(default_factory=list)


@dataclass
class VtAnalysis:
    id: str
    sha256: str
    size: int
    # Set when the file's report is out; None while it is queued.
    stats: dict | None = None


class GitHubMock:
    """The repositories, VirusTotal's files and every rule applied to them. Callers hold `lock`."""

    def __init__(self, args: argparse.Namespace):
        self.args = args
        self.lock = threading.Lock()
        self.port = 0
        self.log_file = None
        self.releases: list[Release] = []
        self.tags: set[str] = set()
        self.latest_id: int | None = None
        # CDN lag: the web download path serves `stale_id` for `stale_left` more requests.
        self.stale_id: int | None = None
        self.stale_left = 0
        self.next_release_id = FIRST_RELEASE_ID
        self.next_asset_id = FIRST_ASSET_ID
        self.uploads = 0
        self.requests: list[dict] = []
        self.device_codes: dict[str, dict] = {}
        # The Mod Browser catalog; main gets its first commit from seed_catalog().
        catalog_owner, catalog_name = args.catalog_repo.split("/")
        self.catalog = Catalog(catalog_owner, catalog_name)
        self.commits: dict[str, Commit] = {}
        self.commit_count = 0
        self.next_comment_id = FIRST_COMMENT_ID
        self.vt_files: dict[str, VtFile] = {}
        self.vt_analyses: dict[str, VtAnalysis] = {}
        self.vt_upload_tokens: set[str] = set()
        # When the requests that count toward --vt-rate arrived.
        self.vt_calls: deque[float] = deque()

    # ---- access ---------------------------------------------------------

    def check_token(self, authorization: str | None) -> None:
        scheme, _, token = (authorization or "").partition(" ")
        if scheme.lower() != "bearer" or token.strip() != self.args.token:
            raise ApiError(401, payload={"message": "Bad credentials", "status": "401"})

    def is_repo(self, owner: str, repo: str) -> bool:
        return f"{owner}/{repo}".lower() == self.args.repo.lower()

    def check_repo(self, owner: str, repo: str, writing: bool) -> None:
        if not self.is_repo(owner, repo):
            raise not_found()
        if writing and self.args.archived:
            raise ApiError(403, "Repository was archived so is read-only.")
        if writing and self.args.no_push:
            raise ApiError(403, "Resource not accessible by personal access token")

    # ---- JSON shapes ----------------------------------------------------

    @property
    def origin(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def repo_json(self) -> dict:
        can_push = not self.args.no_push
        return {
            "id": 1,
            "full_name": self.args.repo,
            "private": self.args.private,
            "archived": self.args.archived,
            "default_branch": "main",
            "permissions": {"admin": can_push, "push": can_push, "pull": True},
        }

    def reported_digest(self, asset: Asset) -> str | None:
        if asset.state != "uploaded":
            return None
        value = asset.sha256
        if self.args.corrupt_digest and asset.name.endswith(self.args.corrupt_digest):
            value = hashlib.sha256(value.encode("ascii")).hexdigest()
        return f"sha256:{value}"

    def asset_json(self, release: Release, asset: Asset, with_sha256: bool = False) -> dict:
        result = {
            "id": asset.id,
            "name": asset.name,
            "size": len(asset.data),
            "state": asset.state,
            "content_type": asset.content_type,
        }
        if not self.args.no_digest:
            result["digest"] = self.reported_digest(asset)
        result["browser_download_url"] = (
            f"{self.origin}/{self.args.repo}/releases/download/{quote(release.slug)}/{quote(asset.name)}"
        )
        if with_sha256:
            result["sha256"] = asset.sha256
        return result

    def release_json(self, release: Release, with_sha256: bool = False) -> dict:
        return {
            "id": release.id,
            "tag_name": release.tag_name,
            "name": release.name,
            "body": release.body,
            "draft": release.draft,
            "prerelease": release.prerelease,
            "created_at": release.created_at,
            "published_at": release.published_at,
            "html_url": f"{self.origin}/{self.args.repo}/releases/tag/{quote(release.slug)}",
            "upload_url": f"{self.origin}/upload/repos/{self.args.repo}/releases/{release.id}/assets{{?name,label}}",
            "assets": [self.asset_json(release, asset, with_sha256) for asset in release.assets],
        }

    def state_json(self) -> dict:
        return {
            "latest_id": self.latest_id,
            "releases": [self.release_json(release, True) for release in reversed(self.releases)],
            "tags": sorted(self.tags),
            "requests": list(self.requests),
            "uploads": self.uploads,
        }

    # ---- releases -------------------------------------------------------

    def new_release(self, tag: str, name: str, body: str, draft: bool, prerelease: bool) -> Release:
        release = Release(self.next_release_id, tag, name, body, draft, prerelease, secrets.token_hex(10), timestamp())
        self.next_release_id += 1
        self.releases.append(release)
        self.mark_published(release)
        return release

    def mark_published(self, release: Release) -> None:
        """A release that is out has a publication time and a git tag; the tag outlives the release."""
        if release.draft:
            release.published_at = None
            return
        release.published_at = release.published_at or timestamp()
        self.tags.add(release.tag_name)

    def delete_tag(self, tag: str) -> None:
        if tag not in self.tags:
            raise ApiError(422, "Reference does not exist")
        if self.args.protect_tags:
            raise ApiError(422, "Repository rule violations found\n\nCannot delete this protected ref.")
        self.tags.discard(tag)
        # GitHub does not drop a release whose tag goes away: it falls back to a draft
        release = self.published_by_tag(tag)
        if release:
            release.draft = True
            release.published_at = None
            if self.latest_id == release.id:
                self.set_latest(self.newest_eligible_id())

    def new_asset(self, release: Release, name: str, content_type: str, data, sha256=None, state="uploaded") -> Asset:
        asset = Asset(self.next_asset_id, name, content_type, data, sha256 or hashlib.sha256(data).hexdigest(), state)
        self.next_asset_id += 1
        release.assets.append(asset)
        return asset

    def seed(self, entries: list[dict]) -> None:
        """Pre-create releases in file order, so the last entry is the newest. Seeding never arms the CDN lag."""
        for entry in entries:
            release = self.new_release(
                entry["tag_name"],
                entry.get("name", ""),
                entry.get("body", ""),
                entry.get("draft", False),
                entry.get("prerelease", False),
            )
            for name, text in entry.get("assets", {}).items():
                self.new_asset(release, name, "application/octet-stream", text.encode("utf-8"))
            if entry.get("latest"):
                self.latest_id = release.id

    def release_by_id(self, release_id: int) -> Release:
        release = next((item for item in self.releases if item.id == release_id), None)
        if not release:
            raise not_found()
        return release

    def published_by_tag(self, tag: str, besides: Release | None = None) -> Release | None:
        return next(
            (item for item in self.releases if not item.draft and item.tag_name == tag and item is not besides),
            None,
        )

    def latest_release(self) -> Release:
        if not self.latest_id:
            raise not_found()
        return self.release_by_id(self.latest_id)

    def set_latest(self, release_id: int | None) -> None:
        if release_id == self.latest_id:
            return
        if release_id and self.args.stale_latest > 0:
            self.stale_id, self.stale_left = self.latest_id, self.args.stale_latest
        self.latest_id = release_id

    def newest_eligible_id(self) -> int | None:
        return max((item.id for item in self.releases if item.eligible), default=None)

    def list_releases(self, query: dict) -> list[dict]:
        return [self.release_json(release) for release in page_of(list(reversed(self.releases)), query)]

    @staticmethod
    def read_make_latest(body: dict, draft: bool, prerelease: bool) -> str:
        make_latest = body.get("make_latest", "legacy")
        if make_latest not in MAKE_LATEST_VALUES or (make_latest == "true" and (draft or prerelease)):
            raise validation_failed("Release", "invalid", "make_latest")
        return make_latest

    def create_release(self, body: dict) -> dict:
        if self.args.empty:
            raise ApiError(422, "Repository is empty.")
        tag = read_text(body, "tag_name", "")
        if not tag:
            raise validation_failed("Release", "missing_field", "tag_name")
        draft = read_flag(body, "draft", False)
        prerelease = read_flag(body, "prerelease", False)
        make_latest = self.read_make_latest(body, draft, prerelease)
        if self.published_by_tag(tag):
            raise validation_failed("Release", "already_exists", "tag_name")

        release = self.new_release(tag, read_text(body, "name", ""), read_text(body, "body", ""), draft, prerelease)
        if make_latest == "true" or (make_latest == "legacy" and release.eligible):
            self.set_latest(release.id)
        return self.release_json(release)

    def update_release(self, release_id: int, body: dict) -> dict:
        release = self.release_by_id(release_id)
        tag = read_text(body, "tag_name", release.tag_name)
        draft = read_flag(body, "draft", release.draft)
        prerelease = read_flag(body, "prerelease", release.prerelease)
        make_latest = self.read_make_latest(body, draft, prerelease)
        if "make_latest" not in body and self.args.latest_on_edit:
            # "Default: true" is what GitHub documents for an update too: an edit that says nothing
            # about the mark takes it.
            make_latest = "true" if not draft and not prerelease else "false"
        if not tag:
            raise validation_failed("Release", "missing_field", "tag_name")
        # A draft may sit on a published tag it already had; taking the tag or publishing onto it is refused.
        if (not draft or tag != release.tag_name) and self.published_by_tag(tag, besides=release):
            raise validation_failed("Release", "already_exists", "tag_name")
        name = read_text(body, "name", release.name)
        text = read_text(body, "body", release.body)

        # Every check has passed: a refused PATCH leaves the release untouched.
        was_eligible = release.eligible
        release.tag_name, release.name, release.body = tag, name, text
        release.draft, release.prerelease = draft, prerelease
        self.mark_published(release)
        if make_latest == "true" or (make_latest == "legacy" and release.eligible and not was_eligible):
            self.set_latest(release.id)
        elif self.latest_id == release.id and not release.eligible:
            self.set_latest(self.newest_eligible_id())
        return self.release_json(release)

    def delete_release(self, release_id: int) -> None:
        self.releases.remove(self.release_by_id(release_id))
        if self.latest_id == release_id:
            self.set_latest(self.newest_eligible_id())

    # ---- assets ---------------------------------------------------------

    def add_asset(self, release_id: int, name: str, content_type: str, data, sha256, state: str) -> dict:
        release = self.release_by_id(release_id)
        if not name:
            raise validation_failed("ReleaseAsset", "missing_field", "name")
        if self.args.rename_assets:
            name = name.replace("-", ".")
        if any(asset.name == name for asset in release.assets):
            raise validation_failed("ReleaseAsset", "already_exists", "name")
        return self.asset_json(release, self.new_asset(release, name, content_type, data, sha256, state))

    def delete_asset(self, asset_id: int) -> None:
        for release in self.releases:
            for asset in release.assets:
                if asset.id == asset_id:
                    release.assets.remove(asset)
                    return
        raise not_found()

    # ---- anonymous downloads ---------------------------------------------

    def cdn_location(self, release: Release | None, name: str) -> str:
        assets = release.assets if release else []
        asset = next((item for item in assets if item.name == name and item.state == "uploaded"), None)
        if not asset:
            raise not_found()
        return f"/cdn/{release.id}/{asset.id}/{quote(asset.name)}"

    def latest_download(self, owner: str, repo: str, name: str) -> str:
        if not self.is_repo(owner, repo) or self.args.private:
            raise not_found()
        release_id = self.latest_id
        if self.stale_left > 0:
            self.stale_left -= 1
            release_id = self.stale_id
        return self.cdn_location(self.release_by_id(release_id) if release_id else None, name)

    def tag_download(self, owner: str, repo: str, tag: str, name: str) -> str:
        if not self.is_repo(owner, repo) or self.args.private:
            raise not_found()
        return self.cdn_location(self.published_by_tag(tag), name)

    def cdn_asset(self, release_id: int, asset_id: int, name: str) -> Asset:
        release = self.release_by_id(release_id)
        asset = next((item for item in release.assets if item.id == asset_id and item.name == name), None)
        if not asset or asset.state != "uploaded":
            raise not_found()
        return asset

    # ---- device flow ----------------------------------------------------

    def device_code(self, form: dict) -> dict:
        if form.get("client_id") != self.args.client_id:
            raise ApiError(404, payload={"error": "Not Found"})
        code = f"dc-{secrets.token_hex(20)}"
        self.device_codes[code] = {"polls": 0, "last": None, "scope": form.get("scope") or "public_repo"}
        return {
            "device_code": code,
            "user_code": "WDJB-MJHT",
            "verification_uri": f"{self.origin}/login/device",
            "expires_in": 900,
            "interval": self.args.device_interval,
        }

    def device_token(self, form: dict) -> dict:
        if form.get("client_id") != self.args.client_id:
            return {"error": "incorrect_client_credentials"}
        if form.get("grant_type") != DEVICE_GRANT:
            return {"error": "unsupported_grant_type"}
        code = form.get("device_code", "")
        entry = self.device_codes.get(code)
        if not entry:
            return {"error": "incorrect_device_code"}

        # Every poll restarts the clock, so a client that ignores slow_down never gets its token.
        now = time.perf_counter()
        too_fast = entry["last"] is not None and now - entry["last"] < self.args.device_interval - POLL_GRACE_SECONDS
        entry["last"] = now
        if too_fast:
            return {"error": "slow_down", "interval": self.args.device_interval + 5}

        entry["polls"] += 1
        if entry["polls"] < self.args.device_approve_after:
            return {"error": "authorization_pending"}
        del self.device_codes[code]
        if self.args.device_deny:
            return {"error": "access_denied"}
        return {"access_token": self.args.token, "token_type": "bearer", "scope": entry["scope"]}

    # ---- the Mod Browser catalog: the repository and its files --------------

    def user_json(self, login: str | None = None) -> dict:
        """GitHub's user object: the account's (GET /user) unless `login` names another; an app's [bot] account is a Bot."""
        login = login or self.args.login
        if login.lower() == ACTIONS_BOT:
            return {"login": ACTIONS_BOT, "id": ACTIONS_BOT_ID, "type": "Bot"}
        if login.lower() == self.args.login.lower():
            return {"login": self.args.login, "id": USER_ID, "type": "User"}
        stable_id = 2000000 + int(hashlib.sha1(login.lower().encode("utf-8")).hexdigest()[:6], 16)
        return {"login": login, "id": stable_id, "type": "Bot" if login.endswith("[bot]") else "User"}

    def is_account(self, user: dict | None) -> bool:
        return bool(user) and user["login"].lower() == self.args.login.lower()

    def is_maintainer(self) -> bool:
        """--login owns the catalog: push permission, and every issue is theirs to edit."""
        return self.args.login.lower() == self.catalog.owner.lower()

    def is_catalog(self, owner: str, repo: str) -> bool:
        return f"{owner}/{repo}".lower() == self.catalog.full_name.lower()

    def catalog_json(self) -> dict:
        catalog = self.catalog
        return {
            "id": CATALOG_ID,
            "full_name": catalog.full_name,
            "name": catalog.name,
            "owner": {"login": catalog.owner},
            "default_branch": DEFAULT_BRANCH,
            "private": False,
            "fork": False,
            "archived": False,
            "permissions": {"push": self.is_maintainer(), "pull": True},
            "html_url": f"{self.origin}/{catalog.full_name}",
        }

    def seed_catalog(self, files: dict[str, bytes]) -> None:
        self.catalog.branches[DEFAULT_BRANCH] = self.new_commit(files, (), "Initial commit").sha

    def new_commit(self, files: dict[str, bytes], parents: tuple[str, ...], message: str) -> Commit:
        self.commit_count += 1
        digest = hashlib.sha1(f"commit {self.commit_count}\n".encode("ascii"))
        for parent in parents:
            digest.update(f"parent {parent}\n".encode("ascii"))
        for path in sorted(files):
            digest.update(f"{path}\0{git_blob_sha(files[path])}\n".encode("utf-8", "replace"))
        digest.update(message.encode("utf-8", "replace"))
        commit = Commit(digest.hexdigest(), files, parents, message)
        self.commits[commit.sha] = commit
        return commit

    def snapshot(self, ref: str) -> dict[str, bytes]:
        """The files ?ref= names: a branch, refs/heads/<branch> or a commit."""
        sha = self.catalog.branches.get(ref.removeprefix("refs/heads/"))
        if not sha and ref.lower() in self.commits:
            sha = ref.lower()
        if not sha:
            raise ApiError(404, f"No commit found for the ref {ref}")
        return self.commits[sha].files

    def get_contents(self, path: str, ref: str) -> dict:
        if (data := self.snapshot(ref).get(path)) is None:
            raise not_found()
        return {
            "type": "file",
            "name": path.rsplit("/", 1)[-1],
            "path": path,
            "sha": git_blob_sha(data),
            "size": len(data),
            "encoding": "base64",
            "content": wrapped_base64(data),
        }

    def public_file(self, path: str) -> bytes:
        """public/<path> on the catalog's default branch: what the catalog publishes."""
        data = self.commits[self.catalog.branches[DEFAULT_BRANCH]].files.get(f"public/{path}")
        if data is None:
            raise not_found()
        return data

    def harness_write(self, path: str, branch: str, data: bytes) -> dict:
        if not path:
            raise ApiError(400, "path= names the file to write")
        if branch not in self.catalog.branches:
            raise not_found()
        head = self.commits[self.catalog.branches[branch]]
        commit = self.new_commit({**head.files, path: data}, (head.sha,), f"Update {path}")
        self.catalog.branches[branch] = commit.sha
        return {"branch": branch, "path": path, "sha": git_blob_sha(data), "commit": commit.sha}

    # ---- the Mod Browser catalog: issues and their comments -----------------

    def new_issue(self, login: str, title: str, body: str, pull: bool) -> Issue:
        now = timestamp()
        issue = Issue(len(self.catalog.issues) + 1, title, body, self.user_json(login), now, now, pull)
        self.catalog.issues.append(issue)
        return issue

    def issue_by_number(self, number: int) -> Issue:
        issues = self.catalog.issues
        if not 0 < number <= len(issues):
            raise not_found()
        return issues[number - 1]

    def issue_page(self, issue: Issue) -> str:
        """The issue on the web; a pull entry's page is the pull request's."""
        return f"{self.origin}/{self.catalog.full_name}/{'pull' if issue.pull else 'issues'}/{issue.number}"

    def issue_json(self, issue: Issue) -> dict:
        api = f"{self.origin}/repos/{self.catalog.full_name}"
        result = {
            "id": ISSUE_ID_BASE + issue.number,
            "number": issue.number,
            "title": issue.title,
            # GitHub answers null for an issue without text.
            "body": issue.body or None,
            "state": issue.state,
            "state_reason": issue.state_reason,
            "user": dict(issue.user),
            "html_url": self.issue_page(issue),
            "url": f"{api}/issues/{issue.number}",
            "comments": len(issue.comments),
            "locked": False,
            "created_at": issue.created_at,
            "updated_at": issue.updated_at,
            "closed_at": issue.closed_at,
            "closed_by": dict(issue.closed_by) if issue.closed_by else None,
        }
        if issue.pull:
            result["pull_request"] = {"url": f"{api}/pulls/{issue.number}", "html_url": self.issue_page(issue)}
        return result

    def list_issues(self, query: dict) -> list[dict]:
        state = query.get("state") or "open"
        if state not in ("open", "closed", "all"):
            raise validation_failed("Issue", "invalid", "state")
        creator = (query.get("creator") or "").lower()
        issues = [
            issue for issue in reversed(self.catalog.issues)
            if state in ("all", issue.state) and creator in ("", issue.user["login"].lower())
        ]
        return [self.issue_json(issue) for issue in page_of(issues, query)]

    def create_issue(self, body: dict) -> dict:
        """POST issues: opened by the account the token belongs to."""
        title = read_text(body, "title", "", "Issue")
        text = read_text(body, "body", "", "Issue")
        if not title.strip():
            raise validation_failed("Issue", "missing_field", "title")
        return self.issue_json(self.new_issue(self.args.login, title, text, False))

    def update_issue(self, number: int, body: dict) -> dict:
        """PATCH issues/{n}: the author's own issue, or any for the maintainer; an author reopens only what they closed."""
        issue = self.issue_by_number(number)
        if not self.is_maintainer() and not self.is_account(issue.user):
            raise no_admin_rights()
        title = read_text(body, "title", issue.title, "Issue")
        # "body": null clears the text.
        text = read_text(body, "body", "", "Issue") if "body" in body else issue.body
        state = read_text(body, "state", issue.state, "Issue")
        reason = read_text(body, "state_reason", None, "Issue")
        if not title.strip():
            raise validation_failed("Issue", "missing_field", "title")
        if state not in ISSUE_STATES:
            raise validation_failed("Issue", "invalid", "state")
        # A reason counts only when the state changes, and then it has to fit the change.
        changing = state != issue.state
        fitting = CLOSE_REASONS if state == "closed" else (REOPENED,)
        if reason is not None and (reason not in (*CLOSE_REASONS, REOPENED) or (changing and reason not in fitting)):
            raise validation_failed("Issue", "invalid", "state_reason")
        if changing and state == "open" and not self.is_maintainer() and not self.is_account(issue.closed_by):
            raise no_admin_rights()

        # Every check has passed: a refused PATCH leaves the issue untouched.
        issue.title, issue.body = title, text
        if changing and state == "closed":
            self.close_issue(issue, self.args.login, reason or CLOSE_REASONS[0])
        elif changing:
            issue.state, issue.state_reason, issue.closed_at, issue.closed_by = "open", REOPENED, None, None
        issue.updated_at = timestamp()
        return self.issue_json(issue)

    def close_issue(self, issue: Issue, login: str, reason: str) -> None:
        issue.state, issue.state_reason, issue.closed_by = "closed", reason, self.user_json(login)
        issue.closed_at = issue.updated_at = timestamp()

    def harness_close(self, number: int, reason: str, login: str) -> dict:
        """Harness: the catalog's workflow - or whoever `login` names - closes an open issue."""
        issue = self.issue_by_number(number)
        if reason not in CLOSE_REASONS:
            raise ApiError(400, "reason= takes completed or not_planned")
        if issue.state != "open":
            raise ApiError(422, f"Issue #{number} is not open")
        self.close_issue(issue, login, reason)
        return self.issue_json(issue)

    def plant_issue(self, creator: str, title: str, raw: bytes, pull: str) -> dict:
        """Harness: an issue opened by anyone; pull=1 makes it an entry standing for a pull request."""
        if not creator or not title.strip():
            raise ApiError(400, "creator= and title= name the issue's author and its title")
        if pull not in ("", "0", "1"):
            raise ApiError(400, "pull= takes 1 or 0")
        return self.issue_json(self.new_issue(creator, title, harness_text(raw, "issue's body"), pull == "1"))

    def comment_json(self, issue: Issue, comment: Comment) -> dict:
        api = f"{self.origin}/repos/{self.catalog.full_name}"
        return {
            "id": comment.id,
            "url": f"{api}/issues/comments/{comment.id}",
            "html_url": f"{self.issue_page(issue)}#issuecomment-{comment.id}",
            "issue_url": f"{api}/issues/{issue.number}",
            "user": dict(comment.user),
            "body": comment.body,
            "created_at": comment.created_at,
            "updated_at": comment.created_at,
        }

    def list_comments(self, number: int, query: dict) -> list[dict]:
        issue = self.issue_by_number(number)
        return [self.comment_json(issue, comment) for comment in page_of(issue.comments, query)]

    def add_comment(self, number: int, login: str, text: str) -> dict:
        issue = self.issue_by_number(number)
        comment = Comment(self.next_comment_id, text, self.user_json(login), timestamp())
        self.next_comment_id += 1
        issue.comments.append(comment)
        issue.updated_at = comment.created_at
        return self.comment_json(issue, comment)

    def post_comment(self, number: int, body: dict) -> dict:
        """POST issues/{n}/comments: written by the account the token belongs to, whatever the issue's state."""
        self.issue_by_number(number)
        text = read_text(body, "body", None, "IssueComment")
        if not text:
            raise validation_failed("IssueComment", "missing_field", "body")
        return self.add_comment(number, self.args.login, text)

    def plant_comments(self, number: int, author: str, raw: bytes, count_text: str) -> list[dict]:
        """Harness: `count` comments by anyone - the catalog's workflow unless `author` names somebody else."""
        self.issue_by_number(number)
        text = harness_text(raw, "comment's body")
        if not text:
            raise ApiError(400, "the request body is the comment's text")
        if not count_text.isdigit() or not 1 <= int(count_text) <= 1000:
            raise ApiError(400, "count= takes 1 to 1000")
        return [self.add_comment(number, author, text) for _ in range(int(count_text))]

    def catalog_state_json(self) -> dict:
        def branch_json(sha: str) -> dict:
            commit, files, binary = self.commits[sha], {}, {}
            for path, data in sorted(commit.files.items()):
                try:
                    files[path] = data.decode("utf-8")
                except UnicodeDecodeError:
                    binary[path] = base64.b64encode(data).decode("ascii")
            return {"sha": sha, "message": commit.message, "files": files, "files_base64": binary}

        catalog = self.catalog
        return {
            "repositories": {
                catalog.full_name: {
                    "branches": {name: branch_json(sha) for name, sha in sorted(catalog.branches.items())},
                },
            },
            "issues": [
                self.issue_json(issue) | {"comment_list": [self.comment_json(issue, comment) for comment in issue.comments]}
                for issue in catalog.issues
            ],
        }

    # ---- VirusTotal -------------------------------------------------------

    def check_vt(self, headers) -> None:
        """Every /vt/ request: no GitHub token, the right key, then the quota."""
        if "Authorization" in headers:
            raise vt_error(400, "BadRequestError", "Authorization header sent to VirusTotal")
        if headers.get("x-apikey") != self.args.vt_key:
            raise vt_error(401, "WrongCredentialsError", "Wrong API key")
        if not self.args.vt_rate:
            return
        now = time.monotonic()
        while self.vt_calls and now - self.vt_calls[0] >= VT_RATE_WINDOW:
            self.vt_calls.popleft()
        if len(self.vt_calls) >= self.args.vt_rate:
            raise vt_error(429, "QuotaExceededError", "Quota exceeded")
        self.vt_calls.append(now)

    def vt_seed(self, seed: dict[str, dict[str, str]]) -> None:
        for sha, results in seed.items():
            entry = self.vt_files.setdefault(sha, VtFile(sha, 0, {}))
            entry.results, entry.queued = dict(results), 0
            self.vt_complete(entry)

    def vt_complete(self, entry: VtFile) -> None:
        """The report is out: the file's queued analyses complete with its verdicts."""
        entry.analysis_date = int(time.time())
        for analysis in self.vt_analyses.values():
            if analysis.sha256 == entry.sha256 and analysis.stats is None:
                analysis.stats = vt_stats(entry.results)

    def vt_lookup(self, entry: VtFile) -> bool:
        """One lookup of a report. True: it is still queued, and this lookup counted down."""
        if entry.queued <= 0:
            return False
        entry.queued -= 1
        if not entry.queued:
            self.vt_complete(entry)
        return True

    def vt_file_json(self, file_id: str) -> dict:
        if not (entry := self.vt_files.get(file_id.lower())):
            raise vt_error(404, "NotFoundError", f'File "{file_id}" not found')
        attributes = {"sha256": entry.sha256, "size": entry.size}
        if self.vt_lookup(entry):
            attributes |= {"last_analysis_results": {}, "last_analysis_stats": vt_stats({})}
        else:
            results = {
                engine: {"category": category, "engine_name": engine,
                         "result": "Trojan.Generic" if category in VT_DETECTED else None}
                for engine, category in entry.results.items()
            }
            attributes |= {
                "last_analysis_date": entry.analysis_date,
                "last_analysis_results": results,
                "last_analysis_stats": vt_stats(entry.results),
            }
        link = f"{self.origin}/vt/api/v3/files/{entry.sha256}"
        return {"data": {"id": entry.sha256, "type": "file", "links": {"self": link}, "attributes": attributes}}

    def vt_register(self, sha256: str, size: int, name: str) -> dict:
        """An upload: the file becomes known, or stays known with its verdicts, and its report queues."""
        entry = self.vt_files.setdefault(sha256, VtFile(sha256, size, dict(self.args.vt_upload_results)))
        entry.size, entry.uploads, entry.queued = size, entry.uploads + 1, self.args.vt_queue
        if name and name not in entry.names:
            entry.names.append(name)
        stamp = f"{sha256[:32]}:{int(time.time())}:{len(self.vt_analyses) + 1}"
        analysis = VtAnalysis(base64.urlsafe_b64encode(stamp.encode("ascii")).decode("ascii"), sha256, size)
        self.vt_analyses[analysis.id] = analysis
        if not entry.queued:
            self.vt_complete(entry)
        link = f"{self.origin}/vt/api/v3/analyses/{analysis.id}"
        return {"data": {"type": "analysis", "id": analysis.id, "links": {"self": link}}}

    def vt_upload_url(self) -> str:
        token = secrets.token_urlsafe(24)
        self.vt_upload_tokens.add(token)
        return f"{self.origin}/vt/upload/{token}"

    def spend_upload_url(self, token: str) -> None:
        if token not in self.vt_upload_tokens:
            raise vt_error(404, "NotFoundError", "This upload URL does not exist or was already used")
        self.vt_upload_tokens.remove(token)

    def vt_analysis_json(self, analysis_id: str) -> dict:
        if not (analysis := self.vt_analyses.get(analysis_id)):
            raise vt_error(404, "NotFoundError", f'Analysis "{analysis_id}" not found')
        entry = self.vt_files[analysis.sha256]
        if analysis.stats is None and self.vt_lookup(entry):
            status, stats = "queued", vt_stats({})
        else:
            status, stats = "completed", analysis.stats or vt_stats(entry.results)
        return {
            "data": {"id": analysis.id, "type": "analysis", "attributes": {"status": status, "stats": stats}},
            "meta": {"file_info": {"sha256": analysis.sha256, "size": analysis.size}},
        }

    def vt_state_json(self) -> dict:
        return {
            "files": {
                sha: {
                    "sha256": sha,
                    "size": entry.size,
                    "results": dict(entry.results),
                    "queued": entry.queued,
                    "uploads": entry.uploads,
                    "names": list(entry.names),
                }
                for sha, entry in self.vt_files.items()
            },
            "analyses": [
                {"id": item.id, "sha256": item.sha256, "status": "queued" if item.stats is None else "completed"}
                for item in self.vt_analyses.values()
            ],
        }


@dataclass
class Answer:
    status: int
    payload: object = None  # sent as JSON unless `body` is given
    body: bytes | bytearray | None = None
    headers: dict = field(default_factory=dict)


class MultipartFile:
    """Streams a multipart/form-data body that must hold exactly one part, the field "file"
    with a filename, and keeps only that part's size and SHA-256. A malformed body is read to
    its end all the same: the error waits for finish()."""

    def __init__(self, boundary: str):
        self.opening = b"--" + boundary.encode("latin-1")
        self.delimiter = b"\r\n" + self.opening
        self.buffer = bytearray()
        self.state = "start"
        self.parts = 0
        self.error = ""
        self.digest = hashlib.sha256()
        self.size = 0
        self.filename = ""

    def fail(self, problem: str) -> None:
        self.error, self.state = problem, "failed"
        self.buffer.clear()

    def feed(self, chunk: bytes) -> None:
        if self.state not in ("failed", "closed"):
            self.buffer += chunk
            while self.step():
                pass

    def step(self) -> bool:
        """Moves the parse on; False when it needs more input or has stopped."""
        buffer = self.buffer
        match self.state:
            case "start":
                if len(buffer) < len(self.opening):
                    return False
                # Without a preamble the body opens with the boundary itself.
                if buffer.startswith(self.opening):
                    del buffer[:len(self.opening)]
                    self.state = "delimiter"
                else:
                    self.state = "preamble"
                return True
            case "preamble":
                if (at := buffer.find(self.delimiter)) < 0:
                    # keep what may be the start of the delimiter
                    del buffer[:max(0, len(buffer) - len(self.delimiter) + 1)]
                    return False
                del buffer[:at + len(self.delimiter)]
                self.state = "delimiter"
                return True
            case "delimiter":
                # transport padding, then CRLF opens a part and "--" closes the body
                at = len(buffer) - len(buffer.lstrip(b" \t"))
                if len(buffer) - at < 2:
                    return False
                mark = bytes(buffer[at:at + 2])
                if mark == b"--":
                    if self.parts:
                        self.state = "closed"
                    else:
                        self.fail("the body closes before its first part")
                    return False
                if mark != b"\r\n":
                    self.fail("a boundary line is malformed")
                elif self.parts:
                    self.fail("the body holds more than one part")
                else:
                    del buffer[:at + 2]
                    self.state = "headers"
                    return True
                return False
            case "headers":
                if buffer.startswith(b"\r\n"):
                    lines, skip = [], 2
                elif (at := buffer.find(b"\r\n\r\n")) >= 0:
                    lines, skip = buffer[:at].decode("utf-8", "replace").split("\r\n"), at + 4
                else:
                    if len(buffer) > PART_HEADERS_LIMIT:
                        self.fail("the part's headers do not end")
                    return False
                if not self.read_headers(lines):
                    return False
                del buffer[:skip]
                self.parts += 1
                self.state = "data"
                return True
            case "data":
                at = buffer.find(self.delimiter)
                end = at if at >= 0 else len(buffer) - len(self.delimiter) + 1
                if end > 0:
                    self.digest.update(buffer[:end])
                    self.size += end
                    del buffer[:end]
                if at < 0:
                    return False
                del buffer[:len(self.delimiter)]
                self.state = "delimiter"
                return True
        return False

    def read_headers(self, lines: list[str]) -> bool:
        headers = {}
        for line in lines:
            name, colon, value = line.partition(":")
            if not colon or not name.strip():
                self.fail(f"a part header is malformed: {line!r}")
                return False
            headers[name.strip().lower()] = value.strip()
        kind, params = header_params(headers.get("content-disposition", ""))
        if kind != "form-data" or params.get("name") != "file" or "filename" not in params:
            self.fail('the part is not Content-Disposition: form-data; name="file"; filename="..."')
            return False
        self.filename = params["filename"]
        return True

    def finish(self) -> tuple[str, int, str]:
        """(sha256, size, filename) of the file part; a 400 for a malformed body."""
        if not self.error and self.state != "closed":
            self.error = "the body ends before its closing boundary"
        if self.error:
            raise vt_error(400, "BadRequestError", f"Malformed multipart/form-data body: {self.error}")
        return self.digest.hexdigest(), self.size, self.filename


class Handler(BaseHTTPRequestHandler):
    """Transport only: parse, read the body, ask GitHubMock, answer, log."""

    protocol_version = "HTTP/1.1"
    server_version = "XFinedGitHubMock/1.0"
    sys_version = ""
    disable_nagle_algorithm = True
    timeout = 120

    def log_message(self, format, *args):  # noqa: A002 - the base class names it
        pass

    @property
    def mock(self) -> GitHubMock:
        return self.server.mock

    def handle_request(self) -> None:
        self.close_connection = True
        self.shutdown_requested = False
        self.bytes_in = 0
        length = self.headers.get("Content-Length") or ""
        self.body_left = int(length) if length.isdigit() else 0
        url = urlsplit(self.path)
        segments = [unquote(part) for part in url.path.split("/") if part]
        query = {key: values[-1] for key, values in parse_qs(url.query, keep_blank_values=True).items()}

        try:
            answer = self.route(segments, query)
        except ApiError as error:
            answer = Answer(error.status, error.payload, headers=error.headers)
        except OSError:
            answer = None  # the client went away mid-body
        except Exception as error:  # a mock defect must reach the client as a visible failure
            traceback.print_exc()
            answer = Answer(500, {"message": f"Mock failure: {error!r}"})

        # Status 0 in the log: the connection ended without a response.
        status = 0
        if answer:
            try:
                self.send_answer(answer)
                status = answer.status
            except OSError:
                pass
        self.record(segments, status)
        if self.shutdown_requested:
            threading.Thread(target=self.server.shutdown, daemon=True).start()

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = handle_request

    # ---- plumbing ---------------------------------------------------------

    def read_body(self, count: int, sink=None, rate: int = 0) -> None:
        """Consume `count` body bytes in chunks, paced to `rate` bytes per second when given."""
        chunk_size = max(1, min(READ_CHUNK, rate // 10)) if rate else READ_CHUNK
        started = time.perf_counter()
        done = 0
        while done < count:
            chunk = self.rfile.read(min(chunk_size, count - done))
            if not chunk:
                raise ConnectionError("the client closed the connection mid-body")
            done += len(chunk)
            self.body_left -= len(chunk)
            self.bytes_in += len(chunk)
            if sink:
                sink(chunk)
            if rate and (ahead := done / rate - (time.perf_counter() - started)) > 0:
                time.sleep(ahead)

    def read_small_body(self) -> bytes:
        parts: list[bytes] = []
        self.read_body(self.body_left, parts.append)
        return b"".join(parts)

    def json_body(self, raw: bytes) -> dict:
        try:
            body = json.loads(raw.decode("utf-8"))
        except ValueError:
            body = None
        if not isinstance(body, dict):
            raise ApiError(400, "Problems parsing JSON")
        return body

    def form_body(self, raw: bytes) -> dict:
        if "json" in (self.headers.get("Content-Type") or ""):
            return self.json_body(raw)
        return {key: values[-1] for key, values in parse_qs(raw.decode("utf-8", "replace")).items()}

    def send_answer(self, answer: Answer) -> None:
        # Closing a socket that still holds unread input resets it and can destroy the response.
        self.read_body(self.body_left)
        body = answer.body
        if body is None:
            body = b"" if answer.payload is None else json.dumps(answer.payload).encode("utf-8")
        self.send_response(answer.status)
        if answer.payload is not None:
            self.send_header("Content-Type", "application/json; charset=utf-8")
        for key, value in answer.headers.items():
            self.send_header(key, value)
        # RFC 7230 3.3.2: a 204 carries no Content-Length.
        if answer.status != 204:
            self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def abort_connection(self) -> None:
        # Linger 0 turns the close into a reset: the client must see an error, not a tidy end of stream.
        linger = struct.pack("hh" if os.name == "nt" else "ii", 1, 0)
        self.connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, linger)
        self.rfile.close()
        self.connection.close()

    def record(self, segments: list[str], status: int) -> None:
        if segments and segments[0].startswith("__"):
            return
        mock = self.mock
        with mock.lock:
            entry = {
                "n": len(mock.requests) + 1,
                "method": self.command,
                "path": self.path,
                "status": status,
                "authorized": "Authorization" in self.headers,
                "vt_key": "x-apikey" in self.headers,
                "bytes_in": self.bytes_in,
            }
            mock.requests.append(entry)
            if mock.log_file:
                stamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
                mock.log_file.write(json.dumps({**entry, "time": stamp}) + "\n")
                mock.log_file.flush()

    def check_api_headers(self) -> None:
        if not self.headers.get("User-Agent"):
            raise ApiError(
                403,
                "Request forbidden by administrative rules. Please make sure your request has a User-Agent header.",
            )
        self.mock.check_token(self.headers.get("Authorization"))

    # ---- routes -----------------------------------------------------------

    def route(self, segments: list[str], query: dict) -> Answer | None:
        """None means the connection was dropped on purpose."""
        if segments[:1] == ["upload"]:
            return self.route_upload(segments, query)
        if segments[:1] == ["vt"]:
            return self.route_vt(segments)
        raw = self.read_small_body()
        with self.mock.lock:
            return self.route_locked(segments, query, raw)

    def route_locked(self, segments: list[str], query: dict, raw: bytes) -> Answer:
        mock, method = self.mock, self.command
        if segments[:1] in (["user"], ["repos"]):
            self.check_api_headers()

        match (method, segments):
            case ("GET", ["__state"]):
                return Answer(200, mock.state_json())
            case ("POST", ["__shutdown"]):
                self.shutdown_requested = True
                return Answer(200, {"ok": True})
            case ("GET", ["__catalog"]):
                return Answer(200, mock.catalog_state_json())
            case ("POST", ["__catalog", "file"]):
                path, branch = query.get("path", "").strip("/"), query.get("branch") or DEFAULT_BRANCH
                return Answer(200, mock.harness_write(path, branch, raw))
            case ("POST", ["__catalog", "issue"]):
                creator, title = query.get("creator", ""), query.get("title", "")
                return Answer(200, mock.plant_issue(creator, title, raw, query.get("pull", "")))
            case ("POST", ["__catalog", "comment"]):
                author = query.get("author") or ACTIONS_BOT
                comments = mock.plant_comments(parse_id(query.get("number", "")), author, raw, query.get("count") or "1")
                return Answer(200, comments)
            case ("POST", ["__catalog", "close"]):
                number, reason = parse_id(query.get("number", "")), query.get("reason") or CLOSE_REASONS[0]
                return Answer(200, mock.harness_close(number, reason, query.get("by") or ACTIONS_BOT))
            case ("GET", ["__vt"]):
                return Answer(200, mock.vt_state_json())
            case ("POST", ["__vt", "seed"]):
                try:
                    mock.vt_seed(read_vt_seed(self.json_body(raw)))
                except ValueError as error:
                    raise ApiError(400, str(error)) from None
                return Answer(200, mock.vt_state_json())

            case ("GET", ["user"]):
                return Answer(200, mock.user_json())
            case (_, ["repos", owner, repo, *rest]) if mock.is_repo(owner, repo):
                mock.check_repo(owner, repo, writing=method != "GET")
                return self.route_repo(rest, query, raw)
            case (_, ["repos", owner, repo, *rest]) if mock.is_catalog(owner, repo):
                return self.route_catalog(rest, query, raw)
            case ("GET", ["catalog", "public", *path]) if path:
                kind = "application/octet-stream" if path[-1].lower().endswith(".dds") else "text/plain; charset=utf-8"
                return Answer(200, body=mock.public_file("/".join(path)), headers={"Content-Type": kind})

            case ("POST", ["login", "device", "code"]):
                return Answer(200, mock.device_code(self.form_body(raw)))
            case ("POST", ["login", "oauth", "access_token"]):
                return Answer(200, mock.device_token(self.form_body(raw)))
            case ("GET", ["login", "device"]):
                page = b"GitHub mock: device verification page. Approval is driven by the mock's command line.\n"
                return Answer(200, body=page, headers={"Content-Type": "text/plain; charset=utf-8"})

            case ("GET", ["cdn", release_id, asset_id, name]):
                return self.cdn_answer(mock.cdn_asset(parse_id(release_id), parse_id(asset_id), name))
            case ("GET", [owner, repo, "releases", "latest", "download", name]):
                return Answer(302, headers={"Location": mock.latest_download(owner, repo, name)})
            case ("GET", [owner, repo, "releases", "download", tag, name]):
                return Answer(302, headers={"Location": mock.tag_download(owner, repo, tag, name)})
        raise not_found()

    def route_repo(self, rest: list[str], query: dict, raw: bytes) -> Answer:
        mock = self.mock
        match (self.command, rest):
            case ("GET", []):
                return Answer(200, mock.repo_json())
            case ("GET", ["releases"]):
                return Answer(200, mock.list_releases(query))
            case ("POST", ["releases"]):
                return Answer(201, mock.create_release(self.json_body(raw)))
            case ("GET", ["releases", "latest"]):
                return Answer(200, mock.release_json(mock.latest_release()))
            case ("GET", ["releases", "tags", tag]):
                if not (release := mock.published_by_tag(tag)):
                    raise not_found()
                return Answer(200, mock.release_json(release))
            case ("DELETE", ["releases", "assets", asset_id]):
                mock.delete_asset(parse_id(asset_id))
                return Answer(204)
            case ("GET", ["releases", release_id]):
                return Answer(200, mock.release_json(mock.release_by_id(parse_id(release_id))))
            case ("PATCH", ["releases", release_id]):
                return Answer(200, mock.update_release(parse_id(release_id), self.json_body(raw)))
            case ("DELETE", ["releases", release_id]):
                mock.delete_release(parse_id(release_id))
                return Answer(204)
            case ("GET", ["releases", release_id, "assets"]):
                release = mock.release_by_id(parse_id(release_id))
                return Answer(200, [mock.asset_json(release, asset) for asset in release.assets])
            case ("GET", ["git", "refs", "tags", *tag]) if tag:
                if "/".join(tag) not in mock.tags:
                    raise not_found()
                return Answer(200, {"ref": "refs/tags/" + "/".join(tag)})
            case ("DELETE", ["git", "refs", "tags", *tag]) if tag:
                mock.delete_tag("/".join(tag))
                return Answer(204)
        raise not_found()

    def route_catalog(self, rest: list[str], query: dict, raw: bytes) -> Answer:
        """The Mod Browser catalog: the repository, its files, its issues and their comments."""
        mock = self.mock
        match (self.command, rest):
            case ("GET", []):
                return Answer(200, mock.catalog_json())
            case ("GET", ["contents", *path]) if path:
                return Answer(200, mock.get_contents("/".join(path), query.get("ref") or DEFAULT_BRANCH))
            case ("GET", ["issues"]):
                return Answer(200, mock.list_issues(query))
            case ("POST", ["issues"]):
                return Answer(201, mock.create_issue(self.json_body(raw)))
            case ("GET", ["issues", number]):
                return Answer(200, mock.issue_json(mock.issue_by_number(parse_id(number))))
            case ("PATCH", ["issues", number]):
                return Answer(200, mock.update_issue(parse_id(number), self.json_body(raw)))
            case ("GET", ["issues", number, "comments"]):
                return Answer(200, mock.list_comments(parse_id(number), query))
            case ("POST", ["issues", number, "comments"]):
                return Answer(201, mock.post_comment(parse_id(number), self.json_body(raw)))
        raise not_found()

    def route_vt(self, segments: list[str]) -> Answer:
        """VirusTotal's API under /vt/api/v3, and the upload URLs it hands out under /vt/upload."""
        mock = self.mock
        with mock.lock:
            mock.check_vt(self.headers)
        match (self.command, segments):
            case ("POST", ["vt", "api", "v3", "files"]):
                return self.vt_upload(mock.args.vt_max_upload, "; a larger file goes to an upload URL (GET /files/upload_url)")
            case ("POST", ["vt", "upload", token]):
                with mock.lock:
                    mock.spend_upload_url(token)
                return self.vt_upload(VT_UPLOAD_URL_LIMIT)
            case ("GET", ["vt", "api", "v3", "files", "upload_url"]):
                with mock.lock:
                    return Answer(200, {"data": mock.vt_upload_url()})
            case ("GET", ["vt", "api", "v3", "files", file_id]):
                with mock.lock:
                    return Answer(200, mock.vt_file_json(file_id))
            case ("GET", ["vt", "api", "v3", "analyses", analysis_id]):
                with mock.lock:
                    return Answer(200, mock.vt_analysis_json(analysis_id))
        raise vt_error(404, "NotFoundError", f"No VirusTotal route {self.command} {urlsplit(self.path).path}")

    def vt_upload(self, limit: int, hint: str = "") -> Answer:
        """A multipart upload, streamed: only the file's size and SHA-256 are kept."""
        if self.body_left > limit:
            raise vt_error(413, "RequestEntityTooLargeError", f"The request body is larger than {limit} bytes{hint}")
        form = MultipartFile(multipart_boundary(self.headers.get("Content-Type")))
        self.read_body(self.body_left, form.feed)
        sha256, size, name = form.finish()
        with self.mock.lock:
            return Answer(200, self.mock.vt_register(sha256, size, name))

    def route_upload(self, segments: list[str], query: dict) -> Answer | None:
        match (self.command, segments):
            case ("POST", ["upload", "repos", owner, repo, "releases", release_id, "assets"]):
                return self.upload(owner, repo, release_id, query.get("name", ""))
        self.check_api_headers()
        raise not_found()

    def upload(self, owner: str, repo: str, release_id: str, name: str) -> Answer | None:
        mock, args = self.mock, self.mock.args
        content_type = self.headers.get("Content-Type") or "application/octet-stream"
        rate = args.throttle_kbps * 125

        def check_access() -> None:
            self.check_api_headers()
            mock.check_repo(owner, repo, writing=True)

        with mock.lock:
            mock.uploads += 1
            dropping = mock.uploads == args.drop_upload
        if dropping:
            self.read_body(self.body_left // 2, rate=rate)
            with mock.lock:
                # Only an upload GitHub would have accepted leaves a starter asset behind.
                try:
                    check_access()
                    mock.add_asset(parse_id(release_id), name, content_type, b"", None, "starter")
                except ApiError:
                    pass
            self.abort_connection()
            return None

        data = bytearray()
        digest = hashlib.sha256()

        def keep(chunk: bytes) -> None:
            data.extend(chunk)
            digest.update(chunk)

        try:
            self.read_body(self.body_left, keep, rate)
        except OSError:
            # GitHub keeps a stub of an upload the client walked away from, and the stub owns the name.
            with mock.lock:
                try:
                    check_access()
                    mock.add_asset(parse_id(release_id), name, content_type, b"", None, "starter")
                except ApiError:
                    pass
            raise
        with mock.lock:
            check_access()
            if self.headers.get("Content-Length") is None:
                raise ApiError(411, "Length Required")
            if not data:
                raise ApiError(400, "Bad Content-Length: 0")
            asset = mock.add_asset(parse_id(release_id), name, content_type, data, digest.hexdigest(), "uploaded")
            return Answer(201, asset)

    def cdn_answer(self, asset: Asset) -> Answer:
        headers = {"Content-Type": "application/octet-stream", "Accept-Ranges": "bytes"}
        span = parse_range(self.headers.get("Range"), len(asset.data))
        if not span:
            return Answer(200, body=asset.data, headers=headers)
        first, last = span
        headers["Content-Range"] = f"bytes {first}-{last}/{len(asset.data)}"
        return Answer(206, body=bytes(asset.data[first:last + 1]), headers=headers)


class MockServer(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 64
    # On Windows SO_REUSEADDR lets a second process bind a port that is already served.
    allow_reuse_address = os.name != "nt"

    def server_bind(self) -> None:
        # HTTPServer.server_bind resolves the host name, which can stall for seconds.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]

    def handle_error(self, request, client_address) -> None:
        # A client dropping its connection is routine here; anything else is worth a traceback.
        if not isinstance(sys.exc_info()[1], ConnectionError):
            super().handle_error(request, client_address)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Local mock of the GitHub and VirusTotal endpoints the editor uses to publish a mod release, "
                    "submit it to the Mod Browser and check its packages."
    )
    parser.add_argument("--port", type=int, default=0, help="listening port on 127.0.0.1; 0 picks a free one")
    parser.add_argument("--token", default="qa-token", help="the only accepted bearer token")
    parser.add_argument("--repo", default="QaOwner/qa-mod",
                        help="owner/name of the mod's own repository (releases); the flags below up to --empty apply to it")
    parser.add_argument("--private", action="store_true", help="private repository: web downloads answer 404")
    parser.add_argument("--archived", action="store_true", help="archived repository: writes answer 403")
    parser.add_argument("--no-push", action="store_true", help="no push permission: writes answer 403")
    parser.add_argument("--empty", action="store_true", help="creating a release answers 422 Repository is empty.")
    parser.add_argument("--seed", metavar="FILE", help="JSON list of releases to pre-create")
    parser.add_argument("--drop-upload", type=int, default=0, metavar="N", help="reset the N-th upload mid-body")
    parser.add_argument("--rename-assets", action="store_true", help="store asset names with '-' replaced by '.'")
    parser.add_argument("--protect-tags", action="store_true", help="deleting a tag ref answers 422, as a tag ruleset does")
    parser.add_argument("--latest-on-edit", action="store_true",
                        help="a PATCH without make_latest marks a published full release latest, GitHub's documented default")
    parser.add_argument("--no-digest", action="store_true", help="asset JSON carries no digest")
    parser.add_argument("--corrupt-digest", metavar="SUFFIX", help="report a wrong digest for matching asset names")
    parser.add_argument("--stale-latest", type=int, default=0, metavar="N",
                        help="latest/download serves the previous latest release for N requests after a change")
    parser.add_argument("--throttle-kbps", type=int, default=0, metavar="K",
                        help="read upload bodies at K kilobits per second (1 kbps = 125 bytes/s)")
    parser.add_argument("--client-id", default="qa-client", help="the only accepted OAuth client id")
    parser.add_argument("--device-approve-after", type=int, default=2, metavar="K",
                        help="the K-th counted token poll ends the device flow")
    parser.add_argument("--device-deny", action="store_true", help="the device flow ends with access_denied")
    parser.add_argument("--device-interval", type=int, default=1, metavar="S", help="minimum seconds between polls")
    parser.add_argument("--log", metavar="FILE", help="append one JSON line per request")
    parser.add_argument("--login", metavar="NAME",
                        help="the account's login (GET /user; the API opens issues and writes comments as it); "
                             "default: the owner part of --repo")
    parser.add_argument("--catalog-repo", default="QaMaintainer/darf-mods", metavar="O/N",
                        help="owner/name of the Mod Browser catalog repository (files and issues; default branch main); "
                             "a --login that owns it is the maintainer: push permission, and every issue is theirs to edit")
    parser.add_argument("--catalog-seed", metavar="DIR", help="every file below DIR goes into the catalog's first commit")
    parser.add_argument("--vt-key", default="qa-vt-key", metavar="KEY", help="the only accepted VirusTotal key (x-apikey)")
    parser.add_argument("--vt-seed", metavar="FILE",
                        help='JSON {"<sha256>": {"<engine>": "<category>"}}: files VirusTotal already knows')
    parser.add_argument("--vt-queue", type=int, default=1, metavar="N",
                        help="after an upload the file's report stays queued, without verdicts, for N lookups")
    parser.add_argument("--vt-upload-results", default=json.dumps(VT_DEFAULT_RESULTS), metavar="JSON",
                        help='the verdicts an uploaded file ends up with, {"<engine>": "<category>"}; categories: '
                             + ", ".join(VT_CATEGORIES))
    parser.add_argument("--vt-rate", type=int, default=0, metavar="N",
                        help="more than N VirusTotal requests within 60 s answer 429; 0 = unlimited")
    parser.add_argument("--vt-max-upload", type=int, default=33554432, metavar="BYTES",
                        help="the largest body POST /files takes (413 above it); an upload URL takes up to "
                             f"{VT_UPLOAD_URL_LIMIT}")
    args = parser.parse_args()
    for option, value in (("--repo", args.repo), ("--catalog-repo", args.catalog_repo)):
        if value.count("/") != 1 or not all(value.split("/")):
            parser.error(f"{option} takes owner/name")
    args.login = args.login or args.repo.split("/")[0]
    if "/" in args.login:
        parser.error("--login takes a user name")
    if args.repo.lower() == args.catalog_repo.lower():
        parser.error("--repo must not be the catalog")
    if args.catalog_seed and not os.path.isdir(args.catalog_seed):
        parser.error("--catalog-seed takes a folder")
    if min(args.vt_queue, args.vt_rate) < 0 or args.vt_max_upload <= 0:
        parser.error("counts cannot be negative and --vt-max-upload must be positive")
    try:
        args.vt_upload_results = read_vt_results(json.loads(args.vt_upload_results), "--vt-upload-results")
    except ValueError as error:
        parser.error(str(error))
    return args


def main() -> int:
    args = parse_args()
    mock = GitHubMock(args)
    if args.seed:
        with open(args.seed, encoding="utf-8-sig") as file:
            mock.seed(json.load(file))
    mock.seed_catalog(read_tree(args.catalog_seed) if args.catalog_seed else {})
    if args.vt_seed:
        with open(args.vt_seed, encoding="utf-8-sig") as file:
            try:
                mock.vt_seed(read_vt_seed(json.load(file)))
            except ValueError as error:
                sys.exit(f"--vt-seed: {error}")
    if args.log:
        mock.log_file = open(args.log, "a", encoding="utf-8")

    server = MockServer(("127.0.0.1", args.port), Handler)
    server.mock = mock
    mock.port = server.server_address[1]
    print(json.dumps({"port": mock.port}), flush=True)
    try:
        server.serve_forever(poll_interval=0.05)
    except KeyboardInterrupt:
        pass
    server.server_close()
    if mock.log_file:
        mock.log_file.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
