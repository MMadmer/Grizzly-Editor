#!/usr/bin/env python3
"""Local stand-in for the GitHub and VirusTotal endpoints the editor talks to.

Covers, for Publish Release: the REST API of the mod's own repository (--repo:
repository, releases, release assets, uploads, tag refs), the anonymous web
download path (releases/latest/download -> CDN redirect) and the OAuth device
flow. For Submit to Mod Browser: GET /user, the catalog repository
(--catalog-repo) and the account's fork of it (POST forks, git/ref, git/refs,
contents, pulls) and the catalog's public files on the web (/catalog/public/..).
For the package checks: VirusTotal's v3 file API under /vt/api/v3 (files,
files/upload_url, analyses) with the author's own key. A set of fault switches
comes with them (see --help). All state lives in memory. Harness routes:
/__state, /__shutdown, /__catalog (+ /file, /merge, /close), /__vt (+ /seed).
Self-test: test_github_mock.py.

The first stdout line is {"port": N}. Nothing else is printed unless a request
handler fails, so a harness may leave the pipes unread.

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
  * Harness routes (every path starting with /__) take no token and are not logged.
  * Publishing a release creates its git tag, deleting the release leaves the tag
    behind, and deleting the tag of a release that is still published turns that
    release into a draft - all three as on GitHub, which is why a client that
    removes a release removes the release first and the tag second.
  * --repo keeps its releases-only model. Branches, contents and pull requests
    live in the catalog network: the catalog and the fork <login>/<catalog name>,
    which share one object store - a ref in the fork may name a commit made in
    the catalog.
  * GitHub forks in the background: the first --fork-delay GET git/ref/...
    requests to a new fork answer 404. Until then its contents answer 404 "This
    repository is empty." and git writes to it 409 "Git Repository is empty.";
    neither counts toward the delay.
  * Git and contents writes need push: the fork is the account's own, the
    catalog needs --catalog-push. Opening or editing a pull request needs read
    access only. Owning the catalog (--login = its owner) makes forking it a 403
    and gives no push without --catalog-push.
  * An open pull request follows its head branch; a closed one keeps the head it
    had. Its diff (changed_files, what a merge takes) is what the head changed
    since its merge base with the base branch. "No commits between" when that is
    nothing or when head and base hold the same files. /__catalog/merge lays that
    diff onto the base as a merge commit (parents: base, head): with a base that
    did not move this is the head's snapshot, and a file the harness rewrote on
    the base meanwhile survives.
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
FORK_ID = 3
USER_ID = 1001
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


def missing_field(name: str) -> ApiError:
    return ApiError(422, f'Invalid request.\n\n"{name}" wasn\'t supplied.')


def pull_refused(message: str) -> ApiError:
    errors = [{"resource": "PullRequest", "code": "custom", "message": message}]
    return ApiError(422, payload={"message": "Validation Failed", "errors": errors})


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


def valid_branch(name: str) -> bool:
    """The git check-ref-format rules a client can realistically trip over."""
    if name == "@" or name.startswith("-") or name.endswith(".") or ".." in name or "@{" in name:
        return False
    if any(char in " ~^:?*[\\" or ord(char) < 32 or ord(char) == 127 for char in name):
        return False
    return all(part and not part.startswith(".") and not part.endswith(".lock") for part in name.split("/"))


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
class Repository:
    """The catalog, or the account's fork of it: branches over the object store the two share."""

    id: int
    owner: str
    name: str
    fork: bool
    branches: dict[str, str] = field(default_factory=dict)
    # GET git/ref requests a new fork still answers 404.
    pending: int = 0

    @property
    def full_name(self) -> str:
        return f"{self.owner}/{self.name}"


@dataclass
class Pull:
    number: int
    title: str
    body: str | None
    head_repo: Repository
    head_ref: str
    base_ref: str
    maintainer_can_modify: bool
    created_at: str
    updated_at: str
    state: str = "open"
    merged_at: str | None = None
    # A closed pull request stops following its branch.
    closed_head: str | None = None
    closed_changes: int = 0


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
        # The Mod Browser catalog network; main gets its first commit from seed_catalog().
        catalog_owner, catalog_name = args.catalog_repo.split("/")
        self.catalog = Repository(CATALOG_ID, catalog_owner, catalog_name, False)
        self.fork: Repository | None = None
        self.commits: dict[str, Commit] = {}
        self.commit_count = 0
        self.pulls: list[Pull] = []
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

    # ---- the Mod Browser catalog network ------------------------------------

    def user_json(self) -> dict:
        return {"login": self.args.login, "id": USER_ID, "type": "User"}

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

    def ancestors(self, sha: str) -> set[str]:
        seen, stack = set(), [sha]
        while stack:
            if (current := stack.pop()) not in seen:
                seen.add(current)
                stack.extend(self.commits[current].parents)
        return seen

    def merge_base(self, first: str, second: str) -> str | None:
        """The nearest commit both descend from."""
        reachable, queue, seen = self.ancestors(first), deque([second]), set()
        while queue:
            if (current := queue.popleft()) in reachable:
                return current
            if current not in seen:
                seen.add(current)
                queue.extend(self.commits[current].parents)
        return None

    def changes(self, head: str, base: str) -> dict[str, bytes | None]:
        """What the head changed since it left the base: path -> its bytes, None when removed."""
        root = self.merge_base(head, base)
        before = self.commits[root].files if root else {}
        after = self.commits[head].files
        return {path: after.get(path) for path in before.keys() | after.keys() if before.get(path) != after.get(path)}

    def network_repo(self, owner: str, name: str) -> Repository:
        wanted = f"{owner}/{name}".lower()
        for repo in (self.catalog, self.fork):
            if repo and repo.full_name.lower() == wanted:
                return repo
        raise not_found()

    def can_push(self, repo: Repository) -> bool:
        return repo.fork or self.args.catalog_push

    def check_write(self, repo: Repository) -> None:
        if not self.can_push(repo):
            raise ApiError(403, "Resource not accessible by personal access token")
        if repo.pending > 0:
            raise ApiError(409, "Git Repository is empty.")

    def network_repo_json(self, repo: Repository) -> dict:
        result = {
            "id": repo.id,
            "full_name": repo.full_name,
            "name": repo.name,
            "owner": {"login": repo.owner},
            "default_branch": DEFAULT_BRANCH,
            "private": False,
            "fork": repo.fork,
            "archived": False,
            "permissions": {"push": self.can_push(repo), "pull": True},
            "html_url": f"{self.origin}/{repo.full_name}",
        }
        if repo.fork:
            result["parent"] = {"full_name": self.catalog.full_name}
        return result

    def create_fork(self, repo: Repository, body: dict) -> dict:
        # The fork is the account's own too, so this also refuses forking the fork.
        if repo.owner.lower() == self.args.login.lower():
            raise ApiError(403, "Cannot fork a repository you own.")
        only_default = read_flag(body, "default_branch_only", False, "Repository")
        if not self.fork:
            branches = {DEFAULT_BRANCH: repo.branches[DEFAULT_BRANCH]} if only_default else dict(repo.branches)
            self.fork = Repository(FORK_ID, self.args.login, repo.name, True, branches, self.args.fork_delay)
        return self.network_repo_json(self.fork)

    @staticmethod
    def ref_json(branch: str, sha: str) -> dict:
        return {"ref": f"refs/heads/{branch}", "object": {"sha": sha, "type": "commit"}}

    def get_ref(self, repo: Repository, parts: list[str]) -> dict:
        if repo.pending > 0:
            repo.pending -= 1
            raise not_found()
        branch = "/".join(parts[1:])
        if parts[:1] != ["heads"] or branch not in repo.branches:
            raise not_found()
        return self.ref_json(branch, repo.branches[branch])

    def commit_of(self, body: dict) -> str:
        sha = body.get("sha")
        if not isinstance(sha, str) or not sha:
            raise missing_field("sha")
        if sha.lower() not in self.commits:
            raise ApiError(422, "Object does not exist")
        return sha.lower()

    def create_ref(self, repo: Repository, body: dict) -> dict:
        self.check_write(repo)
        ref = body.get("ref")
        if not isinstance(ref, str) or not ref:
            raise missing_field("ref")
        # Only branches are modelled, under git's naming rules.
        branch = ref.removeprefix("refs/heads/")
        if branch == ref or not valid_branch(branch):
            raise ApiError(422, "Reference name is invalid")
        sha = self.commit_of(body)
        if branch in repo.branches:
            raise ApiError(422, "Reference already exists")
        repo.branches[branch] = sha
        return self.ref_json(branch, sha)

    def update_ref(self, repo: Repository, branch: str, body: dict) -> dict:
        self.check_write(repo)
        force = read_flag(body, "force", False, "Reference")
        if branch not in repo.branches:
            raise ApiError(422, "Reference does not exist")
        sha = self.commit_of(body)
        if not force and repo.branches[branch] not in self.ancestors(sha):
            raise ApiError(422, "Update is not a fast forward")
        repo.branches[branch] = sha
        return self.ref_json(branch, sha)

    def snapshot(self, repo: Repository, ref: str) -> dict[str, bytes]:
        """The files ?ref= names: a branch, refs/heads/<branch> or a commit of the network."""
        if repo.pending > 0:
            raise ApiError(404, "This repository is empty.")
        sha = repo.branches.get(ref.removeprefix("refs/heads/")) or (ref.lower() if ref.lower() in self.commits else None)
        if not sha:
            raise ApiError(404, f"No commit found for the ref {ref}")
        return self.commits[sha].files

    def get_contents(self, repo: Repository, path: str, ref: str) -> dict:
        if (data := self.snapshot(repo, ref).get(path)) is None:
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

    def put_contents(self, repo: Repository, path: str, body: dict) -> tuple[bool, dict]:
        """(created, answer): a new commit on the branch, its head the parent."""
        self.check_write(repo)
        message, content = body.get("message"), body.get("content")
        if not isinstance(message, str) or not message:
            raise missing_field("message")
        if not isinstance(content, str):
            raise missing_field("content")
        try:
            data = base64.b64decode("".join(content.split()), validate=True)
        except ValueError:
            raise ApiError(422, "content is not valid Base64") from None
        branch = read_text(body, "branch", "", "Content") or DEFAULT_BRANCH
        if branch not in repo.branches:
            raise ApiError(404, f"Branch {branch} not found")
        head = self.commits[repo.branches[branch]]
        current, sha = head.files.get(path), body.get("sha")
        if current is not None and sha is None:
            raise missing_field("sha")
        if sha is not None and (current is None or sha != git_blob_sha(current)):
            raise ApiError(409, f"{path} does not match {sha}")
        commit = self.new_commit({**head.files, path: data}, (head.sha,), message)
        repo.branches[branch] = commit.sha
        answer = {
            "content": {"name": path.rsplit("/", 1)[-1], "path": path, "sha": git_blob_sha(data)},
            "commit": {"sha": commit.sha, "message": message},
        }
        return current is None, answer

    def resolve_head(self, head: str) -> tuple[Repository | None, str]:
        """"owner:branch" (the fork's or the catalog's owner) or a bare branch of the catalog."""
        owner, colon, branch = head.partition(":")
        if not colon:
            return self.catalog, head
        repos = (repo for repo in (self.catalog, self.fork) if repo and repo.owner.lower() == owner.lower())
        return next(repos, None), branch

    def pull_head(self, pull: Pull) -> str:
        return pull.closed_head or pull.head_repo.branches[pull.head_ref]

    def open_pull_for(self, repo: Repository, branch: str, besides: Pull | None = None) -> Pull | None:
        return next(
            (pull for pull in self.pulls
             if pull.state == "open" and pull.head_repo is repo and pull.head_ref == branch and pull is not besides),
            None,
        )

    def pull_json(self, pull: Pull) -> dict:
        head, base = self.pull_head(pull), self.catalog.branches[pull.base_ref]
        return {
            "number": pull.number,
            "state": pull.state,
            "merged": pull.merged_at is not None,
            "merged_at": pull.merged_at,
            "html_url": f"{self.origin}/{self.catalog.full_name}/pull/{pull.number}",
            "title": pull.title,
            "body": pull.body,
            "user": {"login": self.args.login},
            "head": {
                "ref": pull.head_ref,
                "label": f"{pull.head_repo.owner}:{pull.head_ref}",
                "sha": head,
                "repo": {"full_name": pull.head_repo.full_name},
            },
            "base": {"ref": pull.base_ref, "sha": base, "repo": {"full_name": self.catalog.full_name}},
            "maintainer_can_modify": pull.maintainer_can_modify,
            "changed_files": pull.closed_changes if pull.closed_head else len(self.changes(head, base)),
            "created_at": pull.created_at,
            "updated_at": pull.updated_at,
        }

    def list_pulls(self, query: dict) -> list[dict]:
        state = query.get("state") or "open"
        if state not in ("open", "closed", "all"):
            raise validation_failed("PullRequest", "invalid", "state")
        pulls = [pull for pull in reversed(self.pulls) if state in ("all", pull.state)]
        if head := query.get("head"):
            repo, branch = self.resolve_head(head)
            pulls = [pull for pull in pulls if pull.head_repo is repo and pull.head_ref == branch]
        return [self.pull_json(pull) for pull in page_of(pulls, query)]

    def pull_by_number(self, number: int) -> Pull:
        if not 0 < number <= len(self.pulls):
            raise not_found()
        return self.pulls[number - 1]

    def create_pull(self, body: dict) -> dict:
        title, head, base = (read_text(body, key, "", "PullRequest") for key in ("title", "head", "base"))
        text = read_text(body, "body", None, "PullRequest")
        can_modify = read_flag(body, "maintainer_can_modify", False, "PullRequest")
        for key, value in (("title", title), ("head", head), ("base", base)):
            if not value:
                raise validation_failed("PullRequest", "missing_field", key)
        if base not in self.catalog.branches:
            raise validation_failed("PullRequest", "invalid", "base")
        repo, branch = self.resolve_head(head)
        if not repo or repo.pending > 0 or branch not in repo.branches:
            raise validation_failed("PullRequest", "invalid", "head")
        if self.open_pull_for(repo, branch):
            raise pull_refused(f"A pull request already exists for {repo.owner}:{branch}.")
        tip, target = repo.branches[branch], self.catalog.branches[base]
        if self.commits[tip].files == self.commits[target].files or not self.changes(tip, target):
            raise pull_refused(f"No commits between {base} and {head}")
        now = timestamp()
        pull = Pull(len(self.pulls) + 1, title, text, repo, branch, base, can_modify, now, now)
        self.pulls.append(pull)
        return self.pull_json(pull)

    def update_pull(self, number: int, body: dict) -> dict:
        pull = self.pull_by_number(number)
        title = read_text(body, "title", pull.title, "PullRequest")
        text = read_text(body, "body", None, "PullRequest") if "body" in body else pull.body
        state = read_text(body, "state", pull.state, "PullRequest")
        if not title:
            raise validation_failed("PullRequest", "missing_field", "title")
        if state not in ("open", "closed"):
            raise validation_failed("PullRequest", "invalid", "state")
        reopening = state == "open" and pull.state == "closed"
        if reopening and pull.merged_at:
            raise pull_refused("A merged pull request cannot be reopened.")
        if reopening and self.open_pull_for(pull.head_repo, pull.head_ref, besides=pull):
            raise pull_refused(f"A pull request already exists for {pull.head_repo.owner}:{pull.head_ref}.")

        # Every check has passed: a refused PATCH leaves the pull request untouched.
        pull.title, pull.body = title, text
        if reopening:
            pull.state, pull.closed_head, pull.closed_changes = "open", None, 0
        elif state == "closed" and pull.state == "open":
            self.freeze(pull)
        pull.updated_at = timestamp()
        return self.pull_json(pull)

    def freeze(self, pull: Pull) -> None:
        """Close a pull request: from now on it keeps the head it has."""
        pull.closed_head = self.pull_head(pull)
        pull.closed_changes = len(self.changes(pull.closed_head, self.catalog.branches[pull.base_ref]))
        pull.state, pull.updated_at = "closed", timestamp()

    def open_pull(self, number: int) -> Pull:
        pull = self.pull_by_number(number)
        if pull.state != "open":
            raise ApiError(422, f"Pull request #{number} is not open")
        return pull

    def merge_pull(self, number: int) -> dict:
        """Harness: the base takes what the head changed since their merge base, as a merge commit."""
        pull = self.open_pull(number)
        head, base = self.pull_head(pull), self.catalog.branches[pull.base_ref]
        files = dict(self.commits[base].files)
        for path, data in self.changes(head, base).items():
            if data is None:
                files.pop(path, None)
            else:
                files[path] = data
        self.freeze(pull)
        message = f"Merge pull request #{pull.number} from {pull.head_repo.owner}/{pull.head_ref}"
        self.catalog.branches[pull.base_ref] = self.new_commit(files, (base, head), message).sha
        pull.merged_at = pull.updated_at
        return self.pull_json(pull)

    def close_pull(self, number: int) -> dict:
        pull = self.open_pull(number)
        self.freeze(pull)
        return self.pull_json(pull)

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

    def catalog_state_json(self) -> dict:
        def branch_json(sha: str) -> dict:
            commit, files, binary = self.commits[sha], {}, {}
            for path, data in sorted(commit.files.items()):
                try:
                    files[path] = data.decode("utf-8")
                except UnicodeDecodeError:
                    binary[path] = base64.b64encode(data).decode("ascii")
            return {"sha": sha, "message": commit.message, "files": files, "files_base64": binary}

        return {
            "repositories": {
                repo.full_name: {
                    "fork": repo.fork,
                    "pending": repo.pending,
                    "branches": {name: branch_json(sha) for name, sha in sorted(repo.branches.items())},
                }
                for repo in (self.catalog, self.fork) if repo
            },
            "pulls": [self.pull_json(pull) for pull in self.pulls],
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
            case ("POST", ["__catalog", "merge"]):
                return Answer(200, mock.merge_pull(parse_id(query.get("number", ""))))
            case ("POST", ["__catalog", "close"]):
                return Answer(200, mock.close_pull(parse_id(query.get("number", ""))))
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
            case (_, ["repos", owner, repo, *rest]):
                return self.route_network(mock.network_repo(owner, repo), rest, query, raw)
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

    def route_network(self, repo: Repository, rest: list[str], query: dict, raw: bytes) -> Answer:
        """The catalog and the account's fork of it."""
        mock = self.mock
        catalog = repo is mock.catalog
        match (self.command, rest):
            case ("GET", []):
                return Answer(200, mock.network_repo_json(repo))
            case ("POST", ["forks"]):
                return Answer(202, mock.create_fork(repo, self.json_body(raw) if raw.strip() else {}))
            case ("GET", ["git", "ref", *ref]):
                return Answer(200, mock.get_ref(repo, ref))
            case ("POST", ["git", "refs"]):
                return Answer(201, mock.create_ref(repo, self.json_body(raw)))
            case ("PATCH", ["git", "refs", "heads", *branch]) if branch:
                return Answer(200, mock.update_ref(repo, "/".join(branch), self.json_body(raw)))
            case ("GET", ["contents", *path]) if path:
                return Answer(200, mock.get_contents(repo, "/".join(path), query.get("ref") or DEFAULT_BRANCH))
            case ("PUT", ["contents", *path]) if path:
                created, answer = mock.put_contents(repo, "/".join(path), self.json_body(raw))
                return Answer(201 if created else 200, answer)
            case ("GET", ["pulls"]) if catalog:
                return Answer(200, mock.list_pulls(query))
            case ("POST", ["pulls"]) if catalog:
                return Answer(201, mock.create_pull(self.json_body(raw)))
            case ("GET", ["pulls", number]) if catalog:
                return Answer(200, mock.pull_json(mock.pull_by_number(parse_id(number))))
            case ("PATCH", ["pulls", number]) if catalog:
                return Answer(200, mock.update_pull(parse_id(number), self.json_body(raw)))
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
    parser.add_argument("--login", metavar="NAME", help="the account's login (GET /user); default: the owner part of --repo")
    parser.add_argument("--catalog-repo", default="QaMaintainer/darf-mods", metavar="O/N",
                        help="owner/name of the Mod Browser catalog repository; default branch main")
    parser.add_argument("--catalog-seed", metavar="DIR", help="every file below DIR goes into the catalog's first commit")
    parser.add_argument("--catalog-push", action="store_true",
                        help="the account may push to the catalog (a maintainer submitting a module of their own; no fork)")
    parser.add_argument("--fork-delay", type=int, default=1, metavar="N",
                        help="a new fork's first N GET git/ref/... requests answer 404, as GitHub forks in the background")
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
    fork = f"{args.login}/{args.catalog_repo.split('/')[1]}"
    if args.repo.lower() in (args.catalog_repo.lower(), fork.lower()):
        parser.error("--repo must be neither the catalog nor the account's fork of it")
    if args.catalog_seed and not os.path.isdir(args.catalog_seed):
        parser.error("--catalog-seed takes a folder")
    if min(args.fork_delay, args.vt_queue, args.vt_rate) < 0 or args.vt_max_upload <= 0:
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
