#!/usr/bin/env python3
"""Local stand-in for the GitHub endpoints the editor's release publisher uses.

Covers the REST API (repository, releases, release assets, uploads, tag refs), the
anonymous web download path (releases/latest/download -> CDN redirect), the
OAuth device flow and a set of fault switches (see --help). All state lives in
memory. Self-test: test_github_mock.py.

The first stdout line is {"port": N}. Nothing else is printed unless a request
handler fails, so a harness may leave the pipes unread.

Behaviour worth knowing before writing a client against it:
  * Every response closes its connection. The request body is always consumed
    first, so an error answer is never lost to a connection reset.
  * API routes (/user, /repos, /upload) demand a User-Agent header, as GitHub
    does, then the bearer token, then the repository name.
  * "authorized" in the request log means the request carried an Authorization
    header, whatever its value. Web, CDN and device-flow requests must show
    false: a true there is a client leaking its token.
  * /__state and /__shutdown are harness traffic and are not logged.
  * Publishing a release creates its git tag, deleting the release leaves the tag
    behind, and deleting the tag of a release that is still published turns that
    release into a draft - all three as on GitHub, which is why a client that
    removes a release removes the release first and the tag second.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import secrets
import socket
import socketserver
import struct
import sys
import threading
import time
import traceback
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


def read_flag(body: dict, key: str, default: bool) -> bool:
    value = body.get(key, default)
    if not isinstance(value, bool):
        raise validation_failed("Release", "invalid", key)
    return value


def read_text(body: dict, key: str, default: str) -> str:
    value = body.get(key)
    if value is None:
        return default
    if not isinstance(value, str):
        raise validation_failed("Release", "invalid", key)
    return value


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


class GitHubMock:
    """The repository state and every rule applied to it. Callers hold `lock`."""

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
        def number(key: str, default: int) -> int:
            text = query.get(key, "")
            return int(text) if text.isdigit() and int(text) > 0 else default

        per_page = min(number("per_page", 30), 100)
        start = (number("page", 1) - 1) * per_page
        newest_first = list(reversed(self.releases))
        return [self.release_json(release) for release in newest_first[start:start + per_page]]

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


@dataclass
class Answer:
    status: int
    payload: object = None  # sent as JSON unless `body` is given
    body: bytes | bytearray | None = None
    headers: dict = field(default_factory=dict)


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

    do_GET = do_POST = do_PATCH = do_DELETE = handle_request

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
        if segments[:1] in (["__state"], ["__shutdown"]):
            return
        mock = self.mock
        with mock.lock:
            entry = {
                "n": len(mock.requests) + 1,
                "method": self.command,
                "path": self.path,
                "status": status,
                "authorized": "Authorization" in self.headers,
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

            case ("GET", ["user"]):
                return Answer(200, {"login": "qa-user"})
            case (_, ["repos", owner, repo, *rest]):
                mock.check_repo(owner, repo, writing=method != "GET")
                return self.route_repo(rest, query, raw)

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
    parser = argparse.ArgumentParser(description="Local mock of the GitHub endpoints used to publish a mod release.")
    parser.add_argument("--port", type=int, default=0, help="listening port on 127.0.0.1; 0 picks a free one")
    parser.add_argument("--token", default="qa-token", help="the only accepted bearer token")
    parser.add_argument("--repo", default="QaOwner/qa-mod", help="owner/name of the only repository")
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
    args = parser.parse_args()
    if args.repo.count("/") != 1 or not all(args.repo.split("/")):
        parser.error("--repo takes owner/name")
    return args


def main() -> int:
    args = parse_args()
    mock = GitHubMock(args)
    if args.seed:
        with open(args.seed, encoding="utf-8-sig") as file:
            mock.seed(json.load(file))
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
