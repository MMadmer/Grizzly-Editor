#!/usr/bin/env python3
"""Self-test of the Go github_mock.

Starts the mock as a subprocess, once per flag set, and drives it the way the
editor's release publisher, its Mod Browser submission and its VirusTotal checks
will. Prints one PASS/FAIL line per check and ends with "GITHUB MOCK: DONE
passed/total"; the exit code is 0 only when every check passed.

Usage:
    python tools/qa/test_github_mock.py [--only A,E,J] [--skip-large]
"""

from __future__ import annotations

import argparse
import base64
import concurrent.futures
import hashlib
import http.client
import json
import os
import re
import socket
import subprocess
import tempfile
import time
import traceback
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote, urlencode, urlsplit

from qa import mock_exe


TOKEN = "qa-token"
REPO = "QaOwner/qa-mod"
API = f"/repos/{REPO}"
USER_AGENT = "GrizzlyEditor-QA/1.0"
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
DESCRIPTOR = "qa_mod.update.ltx"
LATEST_DESCRIPTOR = f"/{REPO}/releases/latest/download/{DESCRIPTOR}"
API_ROOTS = ("/repos", "/upload", "/user")

LOGIN = "QaOwner"
ACCOUNT = {"login": LOGIN, "id": 1001, "type": "User"}
BOT = {"login": "github-actions[bot]", "id": 41898282, "type": "Bot"}
CATALOG_OWNER = "QaMaintainer"
CATALOG = f"{CATALOG_OWNER}/darf-mods"
CATALOG_API = f"/repos/{CATALOG}"
ISSUES_API = f"{CATALOG_API}/issues"
MOD_FILE = "mods/qa_mod.ltx"
MOD_TEXT = b"[mod]\nid = qa_mod\ntitle = QA Mod\nversion = 1.0.0\n"
# A submission as the editor opens it: the module's file in a fenced ini block.
SUBMISSION_TITLE = "Submit qa_mod"
SUBMISSION = f"Submitted with Grizzly Editor.\n\n```ini\n{MOD_TEXT.decode()}```\n"
FORBIDDEN = {"message": "Must have admin rights to Repository."}
STAMP = r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ"
VT_KEY = "qa-vt-key"
VT_API = "/vt/api/v3"
VT_ZERO = {"malicious": 0, "suspicious": 0, "undetected": 0, "harmless": 0, "type-unsupported": 0, "timeout": 0}
# what an uploaded file ends up with unless --vt-upload-results says otherwise
VT_DEFAULT_RESULTS = {"EngineA": "undetected", "EngineB": "undetected", "EngineC": "harmless"}
# The mock reads request bodies in blocks of this size (READ_CHUNK there).
MOCK_READ_CHUNK = 1 << 20


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_blob_sha(data: bytes) -> str:
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def multipart(data: bytes, filename: str = "qa_mod-1.0.0.zip",
              boundary: str = "GrizzlyEditor00112233445566778899aabb") -> tuple[bytes, str]:
    """The form the editor sends to VirusTotal: one part, the field "file"."""
    head = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\n'
            "Content-Type: application/octet-stream\r\n\r\n").encode("utf-8")
    return head + data + f"\r\n--{boundary}--\r\n".encode("ascii"), f"multipart/form-data; boundary={boundary}"


class Checks:
    def __init__(self) -> None:
        self.passed = 0
        self.total = 0

    def check(self, name: str, ok, detail: str = "") -> bool:
        self.total += 1
        if ok:
            self.passed += 1
            print(f"PASS {name}", flush=True)
        else:
            print(f"FAIL {name}{': ' + detail if detail else ''}", flush=True)
        return bool(ok)

    def equal(self, name: str, actual, expected) -> bool:
        return self.check(name, actual == expected, f"expected {expected!r}, got {actual!r}")


@dataclass
class Reply:
    status: int
    headers: dict[str, str]
    body: bytes

    def json(self):
        return json.loads(self.body) if self.body else None

    def error(self) -> dict:
        """The first entry of a GitHub "errors" list, or an empty dict."""
        errors = (self.json() or {}).get("errors") or [{}]
        return errors[0]


class Mock:
    """One mock process. Leaving the context shuts it down over /__shutdown."""

    def __init__(self, *flags: str):
        self.flags = flags
        self.port = 0
        self.first_line = ""
        self.exit_code: int | None = None

    def __enter__(self) -> Mock:
        self.process = subprocess.Popen(
            [mock_exe(), *self.flags], stdout=subprocess.PIPE, text=True
        )
        self.first_line = self.process.stdout.readline()
        try:
            self.port = json.loads(self.first_line)["port"]
        except (ValueError, KeyError, TypeError):
            self.process.kill()
            raise RuntimeError(f"the mock did not announce its port: {self.first_line!r}") from None
        return self

    def __exit__(self, *exc_info) -> None:
        try:
            self.call("POST", "/__shutdown")
            self.exit_code = self.process.wait(timeout=10)
        except Exception:
            self.process.kill()
            self.process.wait()
        finally:
            self.process.stdout.close()

    def call(self, method: str, path: str, body=None, token: str | None = None,
             headers: dict | None = None, user_agent: str | None = USER_AGENT) -> Reply:
        send = dict(headers or {})
        if user_agent:
            send["User-Agent"] = user_agent
        if token:
            send["Authorization"] = f"Bearer {token}"
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode("utf-8")
            send.setdefault("Content-Type", "application/json")
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=120)
        try:
            connection.request(method, path, body=body, headers=send)
            response = connection.getresponse()
            return Reply(response.status, {key.lower(): value for key, value in response.getheaders()}, response.read())
        finally:
            connection.close()

    def api(self, method: str, path: str, body=None, **extra) -> Reply:
        return self.call(method, path, body, token=TOKEN, **extra)

    def web(self, path: str, **extra) -> Reply:
        """An anonymous GET, the way the game and a browser download."""
        return self.call("GET", path, **extra)

    def form(self, path: str, fields: dict) -> Reply:
        headers = {"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"}
        return self.call("POST", path, urlencode(fields).encode("ascii"), headers=headers)

    def state(self) -> dict:
        return self.call("GET", "/__state").json()

    def catalog(self) -> dict:
        return self.call("GET", "/__catalog").json()

    def vt_state(self) -> dict:
        return self.call("GET", "/__vt").json()

    def harness(self, path: str, body: bytes = b"") -> Reply:
        return self.call("POST", path, body)

    def vt(self, method: str, path: str, body=None, key: str | None = VT_KEY, headers: dict | None = None) -> Reply:
        """A VirusTotal request: the key in x-apikey, no GitHub token."""
        send = dict(headers or {})
        if key is not None:
            send["x-apikey"] = key
        return self.call(method, path, body, headers=send)

    def vt_upload(self, path: str, data: bytes, filename: str = "qa_mod-1.0.0.zip", **extra) -> Reply:
        body, content_type = multipart(data, filename)
        return self.vt("POST", path, body, headers={"Content-Type": content_type}, **extra)


# ---- what the publisher does ----------------------------------------------


def create_release(mock: Mock, tag: str, draft: bool = True, **extra) -> Reply:
    body = {"tag_name": tag, "name": f"QA Mod {tag}", "body": "Release notes.", "draft": draft, "prerelease": False}
    return mock.api("POST", f"{API}/releases", {**body, **extra})


def upload(mock: Mock, release: dict, name: str, data, token: str = TOKEN, headers: dict | None = None) -> Reply:
    # upload_url is a URI template: "...{?name,label}".
    path = urlsplit(release["upload_url"].split("{", 1)[0]).path
    send = {"Content-Type": "application/octet-stream", **(headers or {})}
    return mock.call("POST", f"{path}?name={quote(name)}", data, token=token, headers=send)


def publish(mock: Mock, release_id: int, make_latest: str | None = "true") -> Reply:
    body = {"draft": False} | ({"make_latest": make_latest} if make_latest else {})
    return mock.api("PATCH", f"{API}/releases/{release_id}", body)


def publish_release(mock: Mock, tag: str, assets: dict[str, bytes]) -> dict:
    """Draft, upload, publish as latest. Raises when any step is refused."""
    release = create_release(mock, tag).json()
    for name, data in assets.items():
        if (reply := upload(mock, release, name, data)).status != 201:
            raise AssertionError(f"upload of {name} answered {reply.status}: {reply.body[:200]!r}")
    if (reply := publish(mock, release["id"])).status != 200:
        raise AssertionError(f"publishing {tag} answered {reply.status}: {reply.body[:200]!r}")
    return reply.json()


def download(mock: Mock, path: str) -> tuple[Reply, Reply | None]:
    """An anonymous download: the redirect answer and, when there is one, what it leads to."""
    first = mock.web(path)
    location = first.headers.get("location")
    return first, (mock.web(location) if first.status == 302 and location else None)


def latest_descriptor(mock: Mock) -> bytes | int:
    """The descriptor of the latest release as the web serves it, or the failing status."""
    first, final = download(mock, LATEST_DESCRIPTOR)
    return final.body if final and final.status == 200 else first.status


# ---- what the Mod Browser submission does -----------------------------------


def write_catalog_seed(folder: Path) -> dict[str, bytes]:
    """The files the catalog starts with, written below `folder` for --catalog-seed."""
    files = {
        "public/index.ltx": b"[catalog]\nversion = 1\nupdated = 2026-09-01\n\n[other_mod]\ntitle = Other Mod\n"
                            b"author = Someone\nrepo = Someone/other-mod\nversion = 2.3.4\n",
        "public/icons/qa_mod.dds": b"DDS |" + bytes(range(256)) * 4,
        "mods/other_mod.ltx": b"[mod]\nid = other_mod\n",
        "README.md": b"# darf-mods\n",
    }
    for path, data in files.items():
        target = folder / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    return files


def main_sha(mock: Mock) -> str:
    """The commit the catalog's main points at, as /__catalog reports it."""
    return mock.catalog()["repositories"][CATALOG]["branches"]["main"]["sha"]


def read_file(mock: Mock, path: str, ref: str | None = None) -> bytes | None:
    """A catalog file through the contents API; without `ref` from the default branch."""
    query = "" if ref is None else f"?ref={quote(ref, safe='')}"
    reply = mock.api("GET", f"{CATALOG_API}/contents/{path}{query}")
    return base64.b64decode(reply.json()["content"]) if reply.status == 200 else None


def open_issue(mock: Mock, title: str = SUBMISSION_TITLE, body: str = SUBMISSION) -> Reply:
    return mock.api("POST", ISSUES_API, {"title": title, "body": body})


def plant_issue(mock: Mock, creator: str, title: str, body: bytes = b"", pull: bool = False) -> Reply:
    """/__catalog/issue: an issue - or an entry standing for a pull request - opened by anyone."""
    query = {"creator": creator, "title": title} | ({"pull": 1} if pull else {})
    return mock.harness(f"/__catalog/issue?{urlencode(query)}", body)


def issue_numbers(reply: Reply) -> list[int] | bytes:
    return [item["number"] for item in reply.json()] if reply.status == 200 else reply.body


# ---- flows ------------------------------------------------------------------


def flow_happy_path(checks: Checks, options) -> None:
    """A and B: the whole publication, and what stays hidden while the release is a draft."""
    files = {
        "qa_mod-1.0.0.zip": os.urandom(3 * 1024 * 1024),
        "qa_mod-1.0.0.files": b"qa_mod-1.0.0.zip\nconfigs/a.ltx 0 12 stored\n",
        DESCRIPTOR: b"[update]\nversion = 1.0.0\n",
    }
    with Mock() as mock:
        checks.check("A first stdout line announces the port", re.fullmatch(r'\{"port": \d+\}\n', mock.first_line))

        repo = mock.api("GET", API)
        info = repo.json()
        checks.equal("A repo: 200", repo.status, 200)
        checks.check(
            "A repo: flags of a public writable repository",
            info["full_name"] == REPO and info["private"] is False and info["archived"] is False
            and info["default_branch"] == "main" and info["permissions"] == {"admin": True, "push": True, "pull": True},
            repr(info),
        )
        checks.equal("A latest with no releases: 404", mock.api("GET", f"{API}/releases/latest").status, 404)
        checks.equal("A tags/v1.0.0 with no releases: 404", mock.api("GET", f"{API}/releases/tags/v1.0.0").status, 404)
        listing = mock.api("GET", f"{API}/releases?per_page=100&page=1")
        checks.check("A list with no releases: 200 []", listing.status == 200 and listing.json() == [], repr(listing.body))

        created = create_release(mock, "v1.0.0")
        draft = created.json()
        release_id = draft["id"]
        checks.check(
            "A create draft: 201 with the sent fields",
            created.status == 201 and draft["draft"] is True and draft["prerelease"] is False
            and draft["tag_name"] == "v1.0.0" and draft["name"] == "QA Mod v1.0.0" and draft["body"] == "Release notes."
            and draft["assets"] == [],
            repr(draft),
        )
        checks.check("A release id does not fit 28 bits", release_id >= 250000001, str(release_id))
        checks.equal(
            "A upload_url is the URI template",
            draft["upload_url"],
            f"http://127.0.0.1:{mock.port}/upload/repos/{REPO}/releases/{release_id}/assets{{?name,label}}",
        )
        checks.check(
            "A draft html_url is untagged",
            re.fullmatch(rf"http://127\.0\.0\.1:{mock.port}/{REPO}/releases/tag/untagged-[0-9a-f]+", draft["html_url"]),
            draft["html_url"],
        )

        asset_ids = {}
        for name, data in files.items():
            reply = upload(mock, draft, name, data)
            asset = reply.json()
            asset_ids[name] = asset.get("id", 0)
            checks.check(
                f"A upload {name}: 201 name/size/state/content_type",
                reply.status == 201 and asset["name"] == name and asset["size"] == len(data)
                and asset["state"] == "uploaded" and asset["content_type"] == "application/octet-stream",
                repr(reply.body[:300]),
            )
            checks.equal(f"A upload {name}: digest", asset.get("digest"), f"sha256:{sha256(data)}")
            checks.check(f"A upload {name}: asset id does not fit 28 bits", asset_ids[name] >= 310000001)

        checks.equal("B tags/v1.0.0 while draft: 404", mock.api("GET", f"{API}/releases/tags/v1.0.0").status, 404)
        checks.equal("B releases/latest while draft: 404", mock.api("GET", f"{API}/releases/latest").status, 404)
        listed = mock.api("GET", f"{API}/releases").json()
        checks.check(
            "B the list shows the draft with its assets",
            len(listed) == 1 and listed[0]["id"] == release_id and listed[0]["draft"] is True
            and [item["name"] for item in listed[0]["assets"]] == list(files),
            repr(listed)[:300],
        )
        by_id = mock.api("GET", f"{API}/releases/{release_id}")
        checks.check("B the draft is readable by id", by_id.status == 200 and by_id.json()["draft"] is True)
        checks.equal("B latest/download while draft: 404", mock.web(LATEST_DESCRIPTOR).status, 404)
        draft_asset_url = urlsplit(listed[0]["assets"][2]["browser_download_url"]).path
        checks.equal("B browser_download_url of a draft asset is not public: 404", mock.web(draft_asset_url).status, 404)

        published = publish(mock, release_id)
        release = published.json()
        checks.check("A publish: 200, no longer a draft", published.status == 200 and release["draft"] is False)
        checks.check("A html_url ends with /releases/tag/v1.0.0", release["html_url"].endswith("/releases/tag/v1.0.0"))
        latest = mock.api("GET", f"{API}/releases/latest")
        checks.check("A releases/latest is the published release", latest.status == 200 and latest.json()["id"] == release_id)
        by_tag = mock.api("GET", f"{API}/releases/tags/v1.0.0")
        checks.check("A releases/tags/v1.0.0 is the published release", by_tag.status == 200 and by_tag.json()["id"] == release_id)

        first, final = download(mock, LATEST_DESCRIPTOR)
        checks.check(
            "A latest/download answers 302 to the CDN path",
            first.status == 302 and first.headers.get("location") == f"/cdn/{release_id}/{asset_ids[DESCRIPTOR]}/{DESCRIPTOR}",
            repr(first.headers),
        )
        checks.check(
            "A the redirect serves the exact descriptor bytes",
            final and final.status == 200 and final.body == files[DESCRIPTOR]
            and final.headers.get("content-type") == "application/octet-stream",
        )
        zip_url = urlsplit(release["assets"][0]["browser_download_url"]).path
        first, final = download(mock, zip_url)
        checks.equal("A browser_download_url names the tag", zip_url, f"/{REPO}/releases/download/v1.0.0/qa_mod-1.0.0.zip")
        checks.check("A browser_download_url serves the package bytes", final and final.body == files["qa_mod-1.0.0.zip"])

        state = mock.state()
        checks.equal("A /__state latest_id", state["latest_id"], release_id)
        stored = {asset["name"]: asset["sha256"] for asset in state["releases"][0]["assets"]}
        checks.equal("A /__state sha256 equals the local hashes", stored, {name: sha256(data) for name, data in files.items()})
        checks.equal("A /__state uploads", state["uploads"], 3)
        requests = state["requests"]
        web_requests = [item for item in requests if not item["path"].startswith(API_ROOTS)]
        api_requests = [item for item in requests if item["path"].startswith(API_ROOTS)]
        checks.check("A no web request carried a token", web_requests and not any(item["authorized"] for item in web_requests))
        checks.check("A every API request carried the token", api_requests and all(item["authorized"] for item in api_requests))
        checks.check(
            "A request log entries are numbered and complete",
            [item["n"] for item in requests] == list(range(1, len(requests) + 1))
            and all(set(item) == {"n", "method", "path", "status", "authorized", "vt_key", "bytes_in"} for item in requests),
        )
        zip_upload = next(item for item in requests if "qa_mod-1.0.0.zip" in item["path"] and item["method"] == "POST")
        checks.check(
            "A the upload is logged with its query, status and size",
            zip_upload["path"].endswith("/assets?name=qa_mod-1.0.0.zip") and zip_upload["status"] == 201
            and zip_upload["bytes_in"] == len(files["qa_mod-1.0.0.zip"]),
            repr(zip_upload),
        )
        checks.check("A control endpoints stay out of the log", not any(item["path"].startswith("/__") for item in requests))

        mock.call("GET", LATEST_DESCRIPTOR, token=TOKEN)
        leaks = [item for item in mock.state()["requests"] if item["authorized"] and not item["path"].startswith(API_ROOTS)]
        checks.check("A a token sent to the web endpoint is recorded as a leak", len(leaks) == 1 and leaks[0]["path"] == LATEST_DESCRIPTOR)
    checks.equal("A the mock exits with code 0 after /__shutdown", mock.exit_code, 0)


def flow_second_release(checks: Checks, options) -> None:
    """C: a newer release takes over latest; --stale-latest keeps the web path behind."""
    old, new = b"[update]\nversion = 1.0.0\n", b"[update]\nversion = 1.0.1\n"
    with Mock() as mock:
        publish_release(mock, "v1.0.0", {DESCRIPTOR: old})
        checks.equal("C the first descriptor is served", latest_descriptor(mock), old)
        second = publish_release(mock, "v1.0.1", {DESCRIPTOR: new})
        checks.equal("C releases/latest is v1.0.1", mock.api("GET", f"{API}/releases/latest").json()["id"], second["id"])
        checks.equal("C the download now returns the new descriptor", latest_descriptor(mock), new)
        tags = [item["tag_name"] for item in mock.api("GET", f"{API}/releases").json()]
        checks.equal("C the list is newest first", tags, ["v1.0.1", "v1.0.0"])
        pages = [mock.api("GET", f"{API}/releases?per_page=1&page={page}").json() for page in (1, 2, 3)]
        checks.equal("C per_page and page slice the list", [[item["tag_name"] for item in page] for page in pages],
                     [["v1.0.1"], ["v1.0.0"], []])

    with Mock("--stale-latest", "2") as mock:
        publish_release(mock, "v1.0.0", {DESCRIPTOR: old})
        served = [latest_descriptor(mock) for _ in range(3)]
        checks.equal("C stale: no previous latest means 404, 404, then the descriptor", served, [404, 404, old])
        second = publish_release(mock, "v1.0.1", {DESCRIPTOR: new})
        checks.equal("C stale: the API is never behind", mock.api("GET", f"{API}/releases/latest").json()["id"], second["id"])
        served = [latest_descriptor(mock) for _ in range(3)]
        checks.equal("C stale: two downloads return the old bytes, the third the new", served, [old, old, new])


def flow_existing_tag(checks: Checks, options) -> None:
    """D: a published tag cannot be published again."""
    already_exists = {"resource": "Release", "code": "already_exists", "field": "tag_name"}
    with Mock() as mock:
        publish_release(mock, "v1.0.0", {DESCRIPTOR: b"one"})
        again = create_release(mock, "v1.0.0", draft=False)
        checks.check(
            "D POST with a published tag: 422 already_exists",
            again.status == 422 and again.json()["message"] == "Validation Failed" and again.error() == already_exists,
            repr(again.body),
        )
        as_draft = create_release(mock, "v1.0.0", draft=True)
        checks.check("D the same as a draft: 422 already_exists", as_draft.status == 422 and as_draft.error() == already_exists)
        checks.equal("D nothing was created by the refused requests", len(mock.api("GET", f"{API}/releases").json()), 1)

        first, second = create_release(mock, "v2.0.0"), create_release(mock, "v2.0.0")
        checks.check(
            "D two drafts may share a tag",
            first.status == 201 and second.status == 201 and first.json()["id"] != second.json()["id"],
        )
        checks.equal("D publishing the first draft: 200", publish(mock, first.json()["id"]).status, 200)
        refused = publish(mock, second.json()["id"])
        checks.check("D publishing the second draft: 422 already_exists", refused.status == 422 and refused.error() == already_exists)
        untouched = mock.api("GET", f"{API}/releases/{second.json()['id']}").json()
        checks.check("D the refused PATCH left the draft a draft", untouched["draft"] is True)

        missing = mock.api("POST", f"{API}/releases", {"name": "No tag", "draft": True})
        checks.check("D tag_name missing: 422", missing.status == 422 and missing.error().get("field") == "tag_name")
        garbage = mock.api("POST", f"{API}/releases", b"{not json", headers={"Content-Type": "application/json"})
        checks.equal("D a body that is not JSON: 400", garbage.status, 400)


def flow_dropped_upload(checks: Checks, options) -> None:
    """E: an upload that dies mid-body leaves a starter asset that blocks the name."""
    data = os.urandom(1024 * 1024)
    with Mock("--drop-upload", "1") as mock:
        draft = create_release(mock, "v1.0.0").json()
        assets_path = f"{API}/releases/{draft['id']}/assets"
        try:
            reply = upload(mock, draft, "qa_mod-1.0.0.zip", data)
            outcome = f"answered {reply.status}"
        except (OSError, http.client.HTTPException) as error:
            outcome = "connection error"
            print(f"     (the client saw {type(error).__name__}: {error})", flush=True)
        checks.equal("E the first upload dies with a connection error", outcome, "connection error")

        assets = mock.api("GET", assets_path).json()
        checks.check(
            "E the asset list shows a starter asset of size 0",
            len(assets) == 1 and assets[0]["name"] == "qa_mod-1.0.0.zip" and assets[0]["state"] == "starter"
            and assets[0]["size"] == 0 and assets[0]["digest"] is None,
            repr(assets),
        )
        blocked = upload(mock, draft, "qa_mod-1.0.0.zip", data)
        checks.check("E the starter asset blocks its name: 422", blocked.status == 422 and blocked.error().get("code") == "already_exists")
        checks.equal("E deleting the starter asset: 204", mock.api("DELETE", f"{API}/releases/assets/{assets[0]['id']}").status, 204)
        retry = upload(mock, draft, "qa_mod-1.0.0.zip", data)
        checks.check(
            "E the upload succeeds after the delete",
            retry.status == 201 and retry.json()["state"] == "uploaded" and retry.json()["digest"] == f"sha256:{sha256(data)}",
            repr(retry.body[:300]),
        )

        state = mock.state()
        checks.equal("E /__state counts every upload attempt", state["uploads"], 3)
        dropped = next(item for item in state["requests"] if item["path"].startswith("/upload"))
        checks.check(
            "E the dropped request is logged with status 0 and about half the body",
            dropped["status"] == 0 and dropped["bytes_in"] == len(data) // 2,
            repr(dropped),
        )


def flow_duplicate_asset(checks: Checks, options) -> None:
    """F: an asset name is unique within a release until the asset is deleted."""
    with Mock() as mock:
        draft = create_release(mock, "v1.0.0").json()
        first = upload(mock, draft, "qa_mod-1.0.0.zip", b"first")
        checks.equal("F first upload: 201", first.status, 201)
        second = upload(mock, draft, "qa_mod-1.0.0.zip", b"second")
        checks.check(
            "F the same name again: 422 already_exists",
            second.status == 422 and second.error() == {"resource": "ReleaseAsset", "code": "already_exists", "field": "name"},
            repr(second.body),
        )
        assets = mock.api("GET", f"{API}/releases/{draft['id']}/assets").json()
        checks.check("F the first asset is untouched", len(assets) == 1 and assets[0]["digest"] == f"sha256:{sha256(b'first')}")

        asset_path = f"{API}/releases/assets/{first.json()['id']}"
        checks.equal("F DELETE asset: 204", mock.api("DELETE", asset_path).status, 204)
        checks.equal("F the asset list is empty", mock.api("GET", f"{API}/releases/{draft['id']}/assets").json(), [])
        checks.equal("F DELETE of a deleted asset: 404", mock.api("DELETE", asset_path).status, 404)
        third = upload(mock, draft, "qa_mod-1.0.0.zip", b"second")
        checks.check("F the upload succeeds after the delete", third.status == 201 and third.json()["size"] == 6)

        upload_path = urlsplit(draft["upload_url"].split("{", 1)[0]).path
        unnamed = mock.api("POST", upload_path, b"data", headers={"Content-Type": "application/octet-stream"})
        checks.check("F upload without a name: 422", unnamed.status == 422 and unnamed.error().get("field") == "name")
        checks.equal("F upload of an empty body: 400", upload(mock, draft, "empty.bin", b"").status, 400)
        missing = {**draft, "upload_url": draft["upload_url"].replace(str(draft["id"]), "999")}
        checks.equal("F upload into an unknown release: 404", upload(mock, missing, "a.bin", b"data").status, 404)


def flow_access(checks: Checks, options) -> None:
    """G: credentials, the repository name and the repository flags."""
    with Mock() as mock:
        wrong = mock.call("GET", API, token="wrong-token")
        checks.check(
            "G wrong token: 401 Bad credentials",
            wrong.status == 401 and wrong.json() == {"message": "Bad credentials", "status": "401"},
            repr(wrong.body),
        )
        checks.equal("G no token: 401", mock.call("GET", API).status, 401)
        checks.equal("G GET /user without a token: 401", mock.call("GET", "/user").status, 401)
        user = mock.api("GET", "/user")
        checks.check(
            "G GET /user: 200 with the account",
            user.status == 200 and user.json() == {"login": LOGIN, "id": 1001, "type": "User"},
            repr(user.body),
        )

        other = mock.api("GET", "/repos/QaOwner/other-mod")
        checks.check("G another repository: 404 Not Found", other.status == 404 and other.json() == {"message": "Not Found"})
        checks.equal("G another repository's releases: 404", mock.api("GET", "/repos/Someone/qa-mod/releases").status, 404)
        checks.equal("G owner and name match case-insensitively", mock.api("GET", "/repos/qaowner/QA-MOD").status, 200)
        checks.equal("G unknown route: 404", mock.web("/no/such/route").json(), {"message": "Not Found"})
        checks.equal("G an id that is not a number: 404", mock.api("GET", f"{API}/releases/abc").status, 404)

        no_agent = mock.api("GET", API, user_agent=None)
        checks.check("G API request without User-Agent: 403", no_agent.status == 403 and "User-Agent" in no_agent.json()["message"])

        # The answer must survive a large unread body: a reset here would hide the 401 from the client.
        draft = create_release(mock, "v1.0.0").json()
        denied = upload(mock, draft, "qa_mod-1.0.0.zip", os.urandom(1024 * 1024), token="wrong-token")
        checks.equal("G upload with a wrong token: 401 after the body", denied.status, 401)
        checks.equal("G a refused upload stores nothing", mock.api("GET", f"{API}/releases/{draft['id']}/assets").json(), [])

    with Mock("--private") as mock:
        checks.check("G --private: the repo JSON says private", mock.api("GET", API).json()["private"] is True)
        release = publish_release(mock, "v1.0.0", {DESCRIPTOR: b"private"})
        checks.equal("G --private: the API still serves latest", mock.api("GET", f"{API}/releases/latest").json()["id"], release["id"])
        checks.equal("G --private: latest/download is 404", mock.web(LATEST_DESCRIPTOR).status, 404)

    with Mock("--archived") as mock:
        checks.check("G --archived: the repo JSON says archived", mock.api("GET", API).json()["archived"] is True)
        refused = create_release(mock, "v1.0.0")
        checks.check("G --archived: creating a release is 403", refused.status == 403 and "archived" in refused.json()["message"])
        checks.equal("G --archived: reads still work", mock.api("GET", f"{API}/releases").status, 200)

    with Mock("--no-push") as mock:
        permissions = mock.api("GET", API).json()["permissions"]
        checks.check("G --no-push: permissions.push is false", permissions["push"] is False and permissions["pull"] is True)
        checks.equal("G --no-push: creating a release is 403", create_release(mock, "v1.0.0").status, 403)

    with Mock("--empty") as mock:
        checks.equal("G --empty: the repo itself answers 200", mock.api("GET", API).status, 200)
        refused = create_release(mock, "v1.0.0")
        checks.check(
            "G --empty: creating a release is 422 Repository is empty.",
            refused.status == 422 and refused.json() == {"message": "Repository is empty."},
            repr(refused.body),
        )


def flow_asset_faults(checks: Checks, options) -> None:
    """H: renamed assets, a missing digest, a wrong digest."""
    data = os.urandom(4096)
    with Mock("--rename-assets", "--no-digest") as mock:
        draft = create_release(mock, "v1.0.0").json()
        reply = upload(mock, draft, "qa_mod-1.0.0.zip", data)
        asset = reply.json()
        checks.check("H --rename-assets: the response carries the stored name", reply.status == 201 and asset["name"] == "qa_mod.1.0.0.zip")
        checks.check("H --no-digest: the upload response has no digest", "digest" not in asset, repr(asset))
        listed = mock.api("GET", f"{API}/releases/{draft['id']}/assets").json()
        checks.check("H both are visible in the asset list", listed[0]["name"] == "qa_mod.1.0.0.zip" and "digest" not in listed[0])
        checks.equal("H the original name now collides: 422", upload(mock, draft, "qa_mod-1.0.0.zip", data).status, 422)
        publish(mock, draft["id"])
        checks.equal("H download by the sent name: 404", mock.web(f"/{REPO}/releases/latest/download/qa_mod-1.0.0.zip").status, 404)
        checks.equal("H download by the stored name: 302", mock.web(f"/{REPO}/releases/latest/download/qa_mod.1.0.0.zip").status, 302)
        checks.equal("H /__state still knows the true hash", mock.state()["releases"][0]["assets"][0]["sha256"], sha256(data))

    with Mock("--corrupt-digest", ".update.ltx") as mock:
        draft = create_release(mock, "v1.0.0").json()
        package = upload(mock, draft, "qa_mod-1.0.0.zip", data).json()
        descriptor = upload(mock, draft, DESCRIPTOR, b"[update]\n").json()
        checks.equal("H --corrupt-digest: other assets keep the right digest", package["digest"], f"sha256:{sha256(data)}")
        checks.check(
            "H --corrupt-digest: the descriptor digest is well-formed and wrong",
            re.fullmatch(r"sha256:[0-9a-f]{64}", descriptor["digest"]) and descriptor["digest"] != f"sha256:{sha256(b'[update]' + bytes([10]))}",
            descriptor["digest"],
        )
        listed = mock.api("GET", f"{API}/releases/{draft['id']}/assets").json()
        checks.equal("H --corrupt-digest: the list repeats the wrong digest", listed[1]["digest"], descriptor["digest"])
        checks.equal("H --corrupt-digest: /__state has the true hash", mock.state()["releases"][0]["assets"][1]["sha256"], sha256(b"[update]\n"))


def flow_make_latest(checks: Checks, options) -> None:
    """I: who becomes the latest release."""
    invalid = {"resource": "Release", "code": "invalid", "field": "make_latest"}

    def latest_tag(mock: Mock) -> str:
        return mock.api("GET", f"{API}/releases/latest").json()["tag_name"]

    with Mock() as mock:
        publish_release(mock, "v1.0.0", {DESCRIPTOR: b"base"})
        candidate = create_release(mock, "v1.1.0-rc1", prerelease=True).json()
        refused = publish(mock, candidate["id"], "true")
        checks.check("I prerelease + make_latest true: 422 invalid", refused.status == 422 and refused.error() == invalid, repr(refused.body))
        still = mock.api("GET", f"{API}/releases/{candidate['id']}").json()
        checks.check("I the refused PATCH published nothing", still["draft"] is True)
        accepted = publish(mock, candidate["id"], "false")
        checks.check("I prerelease + make_latest false: 200", accepted.status == 200 and accepted.json()["prerelease"] is True)
        checks.equal("I a prerelease published with false leaves latest alone", latest_tag(mock), "v1.0.0")
        checks.equal("I the prerelease is visible by tag", mock.api("GET", f"{API}/releases/tags/v1.1.0-rc1").status, 200)

        quiet = create_release(mock, "v1.2.0").json()
        checks.equal("I full release + make_latest false: 200", publish(mock, quiet["id"], "false").status, 200)
        checks.equal("I make_latest false leaves latest alone", latest_tag(mock), "v1.0.0")

        legacy = create_release(mock, "v1.3.0").json()
        checks.equal("I publishing without make_latest: 200", publish(mock, legacy["id"], None).status, 200)
        checks.equal("I a release published without make_latest becomes latest", latest_tag(mock), "v1.3.0")

        boolean = mock.api("PATCH", f"{API}/releases/{quiet['id']}", {"make_latest": True})
        checks.check("I make_latest as a JSON boolean: 422 invalid", boolean.status == 422 and boolean.error() == invalid)
        draft_latest = create_release(mock, "v9.9.9", make_latest="true")
        checks.check("I a draft created with make_latest true: 422 invalid", draft_latest.status == 422 and draft_latest.error() == invalid)

        promoted = mock.api("PATCH", f"{API}/releases/{quiet['id']}", {"make_latest": "true"})
        checks.check("I make_latest true on a published release moves latest", promoted.status == 200 and latest_tag(mock) == "v1.2.0")

        direct = create_release(mock, "v1.4.0", draft=False)
        checks.check("I a release created published becomes latest", direct.status == 201 and latest_tag(mock) == "v1.4.0")
        checks.check("I a release created published has a tagged html_url", direct.json()["html_url"].endswith("/releases/tag/v1.4.0"))
        checks.equal("I DELETE release: 204", mock.api("DELETE", f"{API}/releases/{direct.json()['id']}").status, 204)
        checks.equal("I after deleting latest the newest full release takes over", latest_tag(mock), "v1.3.0")
        checks.equal("I the deleted release is gone", mock.api("GET", f"{API}/releases/{direct.json()['id']}").status, 404)


def flow_device(checks: Checks, options) -> None:
    """J: the OAuth device flow."""
    with Mock() as mock:
        started = mock.form("/login/device/code", {"client_id": "qa-client", "scope": "public_repo"})
        code = started.json()
        checks.check(
            "J device code: 200 with every field",
            started.status == 200 and re.fullmatch(r"dc-[0-9a-f]+", code["device_code"]) and code["user_code"] == "WDJB-MJHT"
            and code["verification_uri"] == f"http://127.0.0.1:{mock.port}/login/device"
            and code["expires_in"] == 900 and code["interval"] == 1,
            repr(started.body),
        )
        unknown = mock.form("/login/device/code", {"client_id": "someone-else", "scope": "public_repo"})
        checks.check("J unknown client id: 404", unknown.status == 404 and unknown.json() == {"error": "Not Found"}, repr(unknown.body))
        checks.equal("J the verification page exists", mock.web("/login/device").status, 200)

        poll = {"client_id": "qa-client", "device_code": code["device_code"], "grant_type": DEVICE_GRANT}
        first = mock.form("/login/oauth/access_token", poll)
        checks.check("J first poll: 200 authorization_pending", first.status == 200 and first.json() == {"error": "authorization_pending"})
        hasty = mock.form("/login/oauth/access_token", poll)
        checks.check("J polling too fast: 200 slow_down with interval + 5", hasty.status == 200 and hasty.json() == {"error": "slow_down", "interval": 6})
        time.sleep(1.1)
        granted = mock.form("/login/oauth/access_token", poll)
        checks.check(
            "J the second counted poll returns the token",
            granted.status == 200 and granted.json() == {"access_token": TOKEN, "token_type": "bearer", "scope": "public_repo"},
            repr(granted.body),
        )
        checks.equal("J the issued token opens the API", mock.call("GET", "/user", token=granted.json().get("access_token")).status, 200)
        checks.equal("J a spent device code is refused", mock.form("/login/oauth/access_token", poll).json(), {"error": "incorrect_device_code"})
        stranger = mock.form("/login/oauth/access_token", {**poll, "device_code": "dc-unknown"})
        checks.check("J unknown device code: 200 incorrect_device_code", stranger.status == 200 and stranger.json() == {"error": "incorrect_device_code"})
        checks.equal("J wrong grant type", mock.form("/login/oauth/access_token", {**poll, "grant_type": "password"}).json(),
                     {"error": "unsupported_grant_type"})
        device_requests = [item for item in mock.state()["requests"] if item["path"].startswith("/login")]
        checks.check("J no device-flow request carried a token", device_requests and not any(item["authorized"] for item in device_requests))

    with Mock("--device-deny", "--device-approve-after", "1", "--device-interval", "0") as mock:
        code = mock.form("/login/device/code", {"client_id": "qa-client", "scope": "public_repo"}).json()
        poll = {"client_id": "qa-client", "device_code": code["device_code"], "grant_type": DEVICE_GRANT}
        denied = mock.form("/login/oauth/access_token", poll)
        checks.check("J --device-deny: 200 access_denied", denied.status == 200 and denied.json() == {"error": "access_denied"})

    with Mock("--device-approve-after", "3", "--device-interval", "0", "--client-id", "other-client", "--token", "other-token") as mock:
        code = mock.form("/login/device/code", {"client_id": "other-client", "scope": "public_repo"}).json()
        checks.equal("J --device-interval is announced", code["interval"], 0)
        poll = {"client_id": "other-client", "device_code": code["device_code"], "grant_type": DEVICE_GRANT}
        answers = [mock.form("/login/oauth/access_token", poll).json() for _ in range(3)]
        checks.check(
            "J --device-approve-after 3: pending, pending, token",
            answers[0] == answers[1] == {"error": "authorization_pending"} and answers[2].get("access_token") == "other-token",
            repr(answers),
        )
        checks.equal("J --client-id replaces the default", mock.form("/login/device/code", {"client_id": "qa-client"}).status, 404)
        checks.equal("J --token replaces the default", mock.api("GET", "/user").status, 401)


def flow_seed(checks: Checks, options) -> None:
    """K: seeded releases, a custom repository and the request log file."""
    text = "[update]\nversion = 0.9.0\n"
    seed = [
        {"tag_name": "v0.8.0", "name": "Older", "draft": False, "prerelease": False},
        {"tag_name": "v0.9.0", "name": "Old", "draft": False, "prerelease": False, "latest": True,
         "assets": {"other.mod.update.ltx": text}},
    ]
    api = "/repos/SeedOwner/seed-mod"
    with tempfile.TemporaryDirectory() as folder:
        seed_path, log_path = Path(folder, "seed.json"), Path(folder, "requests.jsonl")
        seed_path.write_text(json.dumps(seed), encoding="utf-8")
        with Mock("--seed", str(seed_path), "--log", str(log_path), "--repo", "SeedOwner/seed-mod") as mock:
            latest = mock.api("GET", f"{api}/releases/latest")
            release = latest.json()
            checks.check("K releases/latest is the seeded release", latest.status == 200 and release["tag_name"] == "v0.9.0" and release["name"] == "Old")
            asset = release["assets"][0]
            checks.check(
                "K the seeded release carries the foreign descriptor",
                asset["name"] == "other.mod.update.ltx" and asset["size"] == len(text) and asset["state"] == "uploaded"
                and asset["digest"] == f"sha256:{sha256(text.encode())}",
                repr(asset),
            )
            first, final = download(mock, "/SeedOwner/seed-mod/releases/latest/download/other.mod.update.ltx")
            checks.check("K the seeded asset downloads anonymously", final and final.body == text.encode())
            checks.equal("K the list holds both seeded releases", [item["tag_name"] for item in mock.api("GET", f"{api}/releases").json()],
                         ["v0.9.0", "v0.8.0"])
            created = mock.api("POST", f"{api}/releases", {"tag_name": "v1.0.0", "draft": True}).json()
            checks.check("K a new release gets an id above the seeded ones", created["id"] > release["id"])
            checks.check("K --repo shapes the URLs", f"/upload/repos/SeedOwner/seed-mod/releases/{created['id']}/assets" in created["upload_url"])
            checks.equal("K the default repository is unknown here: 404", mock.api("GET", API).status, 404)
            logged = mock.state()["requests"]
        lines = [json.loads(line) for line in log_path.read_text(encoding="utf-8").splitlines()]
        checks.check(
            "K --log holds one JSON line per request",
            len(lines) == len(logged) and all({key: line[key] for key in entry} == entry for line, entry in zip(lines, logged)),
            f"{len(lines)} lines for {len(logged)} requests",
        )


def flow_range(checks: Checks, options) -> None:
    """L: byte ranges on the CDN path."""
    data = os.urandom(300000)
    with Mock() as mock:
        publish_release(mock, "v1.0.0", {"qa_mod-1.0.0.zip": data})
        location = mock.web(f"/{REPO}/releases/latest/download/qa_mod-1.0.0.zip").headers["location"]

        def ranged(spec: str) -> Reply:
            return mock.web(location, headers={"Range": spec})

        part = ranged("bytes=100-199")
        checks.check(
            "L bytes=a-b: 206 with Content-Range and the exact slice",
            part.status == 206 and part.headers.get("content-range") == "bytes 100-199/300000"
            and part.headers.get("content-length") == "100" and part.body == data[100:200],
            repr(part.headers),
        )
        tail = ranged("bytes=299000-")
        checks.check("L bytes=a-: 206 to the end", tail.status == 206 and tail.headers.get("content-range") == "bytes 299000-299999/300000" and tail.body == data[299000:])
        suffix = ranged("bytes=-16")
        checks.check("L bytes=-n: the last n bytes", suffix.status == 206 and suffix.body == data[-16:])
        clamped = ranged("bytes=0-999999999")
        checks.check("L an end past the size is clamped", clamped.status == 206 and clamped.headers.get("content-range") == "bytes 0-299999/300000" and clamped.body == data)
        beyond = ranged("bytes=300000-")
        checks.check("L a start past the size: 416", beyond.status == 416 and beyond.headers.get("content-range") == "bytes */300000")
        whole = mock.web(location)
        checks.check("L no Range: 200 with Accept-Ranges", whole.status == 200 and whole.headers.get("accept-ranges") == "bytes" and whole.body == data)
        checks.equal("L an unknown CDN path: 404", mock.web("/cdn/1/2/nothing.zip").status, 404)


def flow_throttle(checks: Checks, options) -> None:
    """M: --throttle-kbps is kilobits per second."""
    data = os.urandom(200 * 1000)
    with Mock("--throttle-kbps", "800") as mock:
        draft = create_release(mock, "v1.0.0").json()
        started = time.perf_counter()
        reply = upload(mock, draft, "qa_mod-1.0.0.zip", data)
        elapsed = time.perf_counter() - started
        checks.check("M a throttled upload still succeeds", reply.status == 201 and reply.json()["digest"] == f"sha256:{sha256(data)}")
        checks.check("M 200 kB at 800 kbps takes about two seconds", 1.5 <= elapsed <= 6.0, f"{elapsed:.2f} s")


def flow_large_upload(checks: Checks, options) -> None:
    """N: a body above 100 MB, streamed."""
    if options.skip_large:
        print("SKIP N large upload (--skip-large)", flush=True)
        return
    block, count = os.urandom(1024 * 1024), 120
    total = len(block) * count
    expected = hashlib.sha256()
    for _ in range(count):
        expected.update(block)
    with Mock() as mock:
        draft = create_release(mock, "v1.0.0").json()
        started = time.perf_counter()
        reply = upload(mock, draft, "qa_mod-1.0.0.zip", (block for _ in range(count)), headers={"Content-Length": str(total)})
        elapsed = time.perf_counter() - started
        asset = reply.json()
        checks.check(
            f"N a {total // (1024 * 1024)} MB upload: 201 with the right size and digest ({elapsed:.1f} s)",
            reply.status == 201 and asset["size"] == total and asset["digest"] == f"sha256:{expected.hexdigest()}",
            repr(reply.body[:300]),
        )
        publish(mock, draft["id"])
        location = mock.web(f"/{REPO}/releases/latest/download/qa_mod-1.0.0.zip").headers["location"]
        first = 100 * 1024 * 1024 + 12345
        part = mock.web(location, headers={"Range": f"bytes={first}-{first + 999}"})
        checks.check(
            "N a range deep inside the large asset",
            part.status == 206 and part.body == block[12345:13345] and part.headers.get("content-range") == f"bytes {first}-{first + 999}/{total}",
        )


def flow_parallel(checks: Checks, options) -> None:
    """P: concurrent uploads."""
    with Mock() as mock:
        draft = create_release(mock, "v1.0.0").json()
        parts = {f"qa_mod-1.0.0.part{index}.zip": os.urandom(512 * 1024) for index in range(1, 7)}
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(parts)) as pool:
            replies = list(pool.map(lambda item: upload(mock, draft, *item), parts.items()))
        ids = {reply.json().get("id") for reply in replies}
        checks.check("P six parallel uploads: 201 each, distinct ids", all(reply.status == 201 for reply in replies) and len(ids) == 6)
        stored = {asset["name"]: asset["sha256"] for asset in mock.state()["releases"][0]["assets"]}
        checks.equal("P every part arrived intact", stored, {name: sha256(data) for name, data in parts.items()})

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            statuses = sorted(pool.map(lambda _: upload(mock, draft, "same.bin", os.urandom(65536)).status, range(4)))
        checks.equal("P one name raced by four uploads: a single winner", statuses, [201, 422, 422, 422])


def flow_explicit_port(checks: Checks, options) -> None:
    """O: --port."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    with Mock("--port", str(port)) as mock:
        checks.equal("O --port is honoured and announced", mock.port, port)
        checks.equal("O the mock answers there", mock.api("GET", API).status, 200)


def flow_tags(checks: Checks, options) -> None:
    """Q: a release's git tag - made by publishing, left behind by deleting the release, removed as a ref."""
    assets = {"qa.mod.update.ltx": b"[release]\n"}
    with Mock() as mock:
        draft = create_release(mock, "v1.0.0").json()
        checks.equal("Q a draft creates no tag", mock.state()["tags"], [])
        checks.equal("Q a draft has no publication time", draft["published_at"], None)
        checks.equal("Q a tag that does not exist cannot be deleted: 422",
                     mock.api("DELETE", f"{API}/git/refs/tags/v1.0.0").status, 422)
        mock.api("DELETE", f"{API}/releases/{draft['id']}")

        first = publish_release(mock, "v1.0.0", assets)
        second = publish_release(mock, "v1.1.0", assets)
        checks.equal("Q publishing creates the tags", mock.state()["tags"], ["v1.0.0", "v1.1.0"])
        checks.check("Q a published release carries its publication time", bool(second["published_at"]), str(second))
        checks.equal("Q the tag ref reads back", mock.api("GET", f"{API}/git/refs/tags/v1.1.0").status, 200)

        checks.equal("Q deleting the release: 204", mock.api("DELETE", f"{API}/releases/{second['id']}").status, 204)
        state = mock.state()
        checks.equal("Q the tag outlives its release", state["tags"], ["v1.0.0", "v1.1.0"])
        checks.equal("Q latest falls back to the release that is left", state["latest_id"], first["id"])
        checks.equal("Q deleting the tag ref: 204", mock.api("DELETE", f"{API}/git/refs/tags/v1.1.0").status, 204)
        checks.equal("Q and it is gone", mock.state()["tags"], ["v1.0.0"])
        checks.equal("Q deleting it twice: 422", mock.api("DELETE", f"{API}/git/refs/tags/v1.1.0").status, 422)
        checks.equal("Q the tag can be published again", publish_release(mock, "v1.1.0", assets)["tag_name"], "v1.1.0")

        # the wrong order: the tag first
        checks.equal("Q deleting the tag under a published release: 204",
                     mock.api("DELETE", f"{API}/git/refs/tags/v1.0.0").status, 204)
        left = next(item for item in mock.state()["releases"] if item["id"] == first["id"])
        checks.check("Q leaves that release behind as a draft", left["draft"] and left["published_at"] is None, str(left))
        checks.equal("Q a tag ref needs the token: 401",
                     mock.call("DELETE", f"{API}/git/refs/tags/v1.1.0").status, 401)

    with Mock("--latest-on-edit") as mock:
        old = publish_release(mock, "v1.0.0", assets)
        new = publish_release(mock, "v1.1.0", assets)
        mock.api("PATCH", f"{API}/releases/{old['id']}", {"body": "Edited."})
        checks.equal("Q --latest-on-edit: an edit that names no make_latest takes the mark", mock.state()["latest_id"], old["id"])
        mock.api("PATCH", f"{API}/releases/{new['id']}", {"body": "Edited.", "make_latest": "true"})
        mock.api("PATCH", f"{API}/releases/{old['id']}", {"body": "Edited again.", "make_latest": "false"})
        checks.equal("Q and one that says false leaves it", mock.state()["latest_id"], new["id"])

    with Mock("--protect-tags") as mock:
        release = publish_release(mock, "v2.0.0", assets)
        mock.api("DELETE", f"{API}/releases/{release['id']}")
        reply = mock.api("DELETE", f"{API}/git/refs/tags/v2.0.0")
        checks.check("Q --protect-tags refuses the deletion: 422 with GitHub's words",
                     reply.status == 422 and "protected" in (reply.json() or {}).get("message", ""), str(reply.body))
        checks.equal("Q and the tag stays", mock.state()["tags"], ["v2.0.0"])


def flow_catalog_repo(checks: Checks, options) -> None:
    """R: the account, the catalog repository and its owner, and the routes a submission no longer has."""
    with Mock() as mock:
        user = mock.api("GET", "/user")
        checks.check("R GET /user: the owner of --repo by default", user.status == 200 and user.json() == ACCOUNT, repr(user.body))
        catalog = mock.api("GET", CATALOG_API)
        info = catalog.json() or {}
        expected = {
            "full_name": CATALOG, "name": "darf-mods", "owner": {"login": CATALOG_OWNER}, "default_branch": "main",
            "private": False, "fork": False, "archived": False, "permissions": {"push": False, "pull": True},
            "html_url": f"http://127.0.0.1:{mock.port}/{CATALOG}",
        }
        checks.check(
            "R the catalog: public, no fork, branch main, no push for the account",
            catalog.status == 200 and isinstance(info.get("id"), int) and info == {**expected, "id": info.get("id")},
            repr(info),
        )
        checks.equal("R the catalog's name matches case-insensitively, on its routes too",
                     [mock.api("GET", path).status for path in ("/repos/qamaintainer/DARF-MODS", "/repos/QAMAINTAINER/darf-mods/issues")],
                     [200, 200])
        checks.equal("R --repo keeps its own repository", mock.api("GET", API).json()["full_name"], REPO)

        routes = [
            ("GET", CATALOG_API, None),
            ("GET", f"{CATALOG_API}/contents/README.md", None),
            ("GET", ISSUES_API, None),
            ("POST", ISSUES_API, {"title": SUBMISSION_TITLE, "body": SUBMISSION}),
            ("GET", f"{ISSUES_API}/1", None),
            ("PATCH", f"{ISSUES_API}/1", {"state": "closed"}),
            ("GET", f"{ISSUES_API}/1/comments", None),
            ("POST", f"{ISSUES_API}/1/comments", {"body": "Hello."}),
        ]
        checks.equal("R every catalog route needs the token: 401",
                     [mock.call(method, path, body).status for method, path, body in routes], [401] * len(routes))
        checks.equal("R a wrong token: 401", mock.call("POST", ISSUES_API, {"title": SUBMISSION_TITLE}, token="wrong-token").status, 401)
        checks.equal("R no User-Agent: 403", mock.api("POST", ISSUES_API, {"title": SUBMISSION_TITLE}, user_agent=None).status, 403)
        checks.equal("R the refused requests opened nothing", mock.catalog()["issues"], [])

        gone = [
            ("POST", f"{CATALOG_API}/forks", {}),
            ("GET", f"/repos/{LOGIN}/darf-mods", None),
            ("GET", f"{CATALOG_API}/git/ref/heads/main", None),
            ("POST", f"{CATALOG_API}/git/refs", {"ref": "refs/heads/submit-qa_mod", "sha": "0" * 40}),
            ("PATCH", f"{CATALOG_API}/git/refs/heads/main", {"sha": "0" * 40, "force": True}),
            ("PUT", f"{CATALOG_API}/contents/{MOD_FILE}", {"message": "Add qa_mod", "content": "eA=="}),
            ("GET", f"{CATALOG_API}/pulls", None),
            ("POST", f"{CATALOG_API}/pulls", {"title": "Add QA Mod", "head": f"{LOGIN}:submit-qa_mod", "base": "main"}),
            ("GET", f"{CATALOG_API}/pulls/1", None),
            ("GET", f"{API}/issues", None),
        ]
        statuses = [mock.api(method, path, body).status for method, path, body in gone]
        checks.equal(
            "R forks, git refs, content writes, pull requests and /__catalog/merge are gone, --repo has no issues: 404 each",
            statuses + [mock.harness("/__catalog/merge?number=1").status],
            [404] * (len(gone) + 1),
        )

        refused = [
            subprocess.run([mock_exe(), *flags], capture_output=True, text=True, timeout=60)
            for flags in (["--catalog-push"], ["--fork-delay", "1"], ["--repo", CATALOG.lower()])
        ]
        checks.check(
            "R --catalog-push and --fork-delay are gone, and --repo may not be the catalog: exit 2 each",
            [item.returncode for item in refused] == [2, 2, 2] and "--repo must not be the catalog" in refused[2].stderr,
            " | ".join(item.stderr.strip()[-160:] for item in refused),
        )

    with Mock("--login", CATALOG_OWNER) as mock:
        checks.equal("R --login names the account", mock.api("GET", "/user").json(), {"login": CATALOG_OWNER, "id": 1001, "type": "User"})
        checks.equal("R the catalog's owner may push", mock.api("GET", CATALOG_API).json()["permissions"], {"push": True, "pull": True})


def flow_catalog_contents(checks: Checks, options) -> None:
    """S: the catalog's files through the contents API - the default branch, a branch by name, a commit."""
    with tempfile.TemporaryDirectory() as folder:
        seed = write_catalog_seed(Path(folder))
        with Mock("--catalog-seed", folder) as mock:
            index, icon = seed["public/index.ltx"], seed["public/icons/qa_mod.dds"]
            got = mock.api("GET", f"{CATALOG_API}/contents/public/index.ltx")
            info = got.json() or {}
            checks.check(
                "S GET contents without ?ref: the default branch's file with its blob sha and size",
                got.status == 200 and info.get("type") == "file" and info.get("name") == "index.ltx"
                and info.get("path") == "public/index.ltx" and info.get("sha") == git_blob_sha(index)
                and info.get("size") == len(index) and info.get("encoding") == "base64",
                repr(info),
            )
            lines = info.get("content", "").split("\n")
            checks.check(
                "S content: base64 broken after every 60 characters, a line break at the end",
                len(lines) > 2 and all(len(line) == 60 for line in lines[:-2]) and 0 < len(lines[-2]) <= 60
                and lines[-1] == "" and base64.b64decode(info.get("content", "")) == index,
                repr(info.get("content")),
            )
            first = main_sha(mock)
            checks.equal(
                "S ?ref=main, refs/heads/main, an empty ?ref and the commit's sha read the same file",
                [read_file(mock, "public/index.ltx", ref) for ref in ("main", "refs/heads/main", "", first)],
                [index] * 4,
            )
            checks.equal("S a binary file comes back byte for byte", read_file(mock, "public/icons/qa_mod.dds"), icon)

            checks.equal("S a file main does not hold yet: 404", mock.api("GET", f"{CATALOG_API}/contents/{MOD_FILE}").status, 404)
            wrote = mock.harness(f"/__catalog/file?path={MOD_FILE}", MOD_TEXT).json() or {}
            head = main_sha(mock)
            checks.check(
                "S once the harness commits it, the default branch serves it",
                wrote.get("commit") == head != first and read_file(mock, MOD_FILE) == MOD_TEXT
                and read_file(mock, MOD_FILE, "main") == MOD_TEXT,
                repr(wrote),
            )
            checks.check(
                "S the earlier commit still reads the files as they were",
                mock.api("GET", f"{CATALOG_API}/contents/{MOD_FILE}?ref={first}").status == 404
                and read_file(mock, "public/index.ltx", first) == index,
            )
            checks.equal("S a sha in capitals names the same commit", read_file(mock, MOD_FILE, head.upper()), MOD_TEXT)

            missing = mock.api("GET", f"{CATALOG_API}/contents/mods/nothing.ltx")
            checks.check("S a missing file: 404 Not Found", missing.status == 404 and missing.json() == {"message": "Not Found"},
                         repr(missing.body))
            unknown = mock.api("GET", f"{CATALOG_API}/contents/public/index.ltx?ref=nothing")
            checks.check(
                "S an unknown ref: 404 No commit found",
                unknown.status == 404 and unknown.json() == {"message": "No commit found for the ref nothing"},
                repr(unknown.body),
            )
            checks.equal("S a folder is no file: 404", mock.api("GET", f"{CATALOG_API}/contents/public").status, 404)
            checks.equal("S another repository's contents: 404",
                         mock.api("GET", f"/repos/{LOGIN}/darf-mods/contents/README.md").status, 404)

            state = mock.catalog()
            repositories = state["repositories"]
            main = repositories.get(CATALOG, {}).get("branches", {}).get("main", {})
            checks.check(
                "S /__catalog: the catalog alone, main alone, its files as text and a binary one as base64",
                list(repositories) == [CATALOG] and list(repositories[CATALOG]) == ["branches"]
                and list(repositories[CATALOG]["branches"]) == ["main"]
                and main.get("sha") == head and main.get("message") == f"Update {MOD_FILE}"
                and main["files"].get(MOD_FILE) == MOD_TEXT.decode() and "public/icons/qa_mod.dds" not in main["files"]
                and base64.b64decode(main["files_base64"].get("public/icons/qa_mod.dds", "")) == icon,
                repr(state)[:400],
            )


def flow_catalog_issues(checks: Checks, options) -> None:
    """T: the catalog's issues - opening one, listing and reading them, and the entries the harness opens."""
    with Mock() as mock:
        origin = f"http://127.0.0.1:{mock.port}"
        quiet = mock.api("GET", ISSUES_API)
        checks.equal("T no issues yet: 200 []", (quiet.status, quiet.json()), (200, []))

        opened = open_issue(mock)
        issue = opened.json() or {}
        created = issue.get("created_at") or ""
        expected = {
            "number": 1, "title": SUBMISSION_TITLE, "body": SUBMISSION, "state": "open", "state_reason": None,
            "user": ACCOUNT, "html_url": f"{origin}/{CATALOG}/issues/1", "url": f"{origin}{ISSUES_API}/1",
            "comments": 0, "locked": False, "created_at": created, "updated_at": created, "closed_at": None, "closed_by": None,
        }
        checks.check(
            "T POST issues: 201 with the issue, opened by the token's account, no pull_request key",
            opened.status == 201 and issue == {**expected, "id": issue.get("id")} and re.fullmatch(STAMP, created),
            repr(opened.body[:600]),
        )
        checks.check("T an issue id does not fit a signed 32-bit integer",
                     isinstance(issue.get("id"), int) and issue["id"] > 2**31 - 1, repr(issue.get("id")))

        refusals = {
            "no title": ({"body": SUBMISSION}, "missing_field", "title"),
            "an empty title": ({"title": "", "body": SUBMISSION}, "missing_field", "title"),
            "a blank title": ({"title": " \t", "body": SUBMISSION}, "missing_field", "title"),
            "a title that is no string": ({"title": 5, "body": SUBMISSION}, "invalid", "title"),
            "a body that is no string": ({"title": SUBMISSION_TITLE, "body": ["x"]}, "invalid", "body"),
        }
        for name, (body, code, field) in refusals.items():
            reply = mock.api("POST", ISSUES_API, body)
            checks.check(
                f"T POST with {name}: 422 {code} {field}",
                reply.status == 422 and reply.json() == {"message": "Validation Failed",
                                                         "errors": [{"resource": "Issue", "code": code, "field": field}]},
                repr(reply.body),
            )
        garbage = mock.api("POST", ISSUES_API, b"{not json", headers={"Content-Type": "application/json"})
        checks.equal("T POST with a body that is not JSON: 400", garbage.status, 400)
        checks.equal("T the refused requests opened nothing", [item["number"] for item in mock.catalog()["issues"]], [1])
        bare = mock.api("POST", ISSUES_API, {"title": "A question"})
        checks.check("T no body: 201, and the body reads back null",
                     bare.status == 201 and (bare.json() or {}).get("number") == 2 and (bare.json() or {}).get("body", "") is None,
                     repr(bare.body[:300]))

        text = "Другой мод.\r\n\r\n```ini\n[mod]\nid = other_mod\n```\n"
        planted = plant_issue(mock, "Stranger", "Submit other_mod", text.encode("utf-8"))
        theirs = planted.json() or {}
        checks.check(
            "T /__catalog/issue: an issue opened by anyone, its body byte for byte",
            planted.status == 200 and theirs.get("number") == 3 and theirs.get("title") == "Submit other_mod"
            and theirs.get("body") == text and (theirs.get("user") or {}).get("login") == "Stranger"
            and (theirs.get("user") or {}).get("type") == "User" and theirs.get("state") == "open"
            and theirs.get("html_url") == f"{origin}/{CATALOG}/issues/3" and "pull_request" not in theirs,
            repr(planted.body[:400]),
        )
        pull = plant_issue(mock, "Stranger", "Add other_mod", pull=True).json() or {}
        checks.check(
            "T pull=1: an entry standing for a pull request, with GitHub's pull_request key",
            pull.get("number") == 4 and pull.get("html_url") == f"{origin}/{CATALOG}/pull/4"
            and pull.get("url") == f"{origin}{ISSUES_API}/4"
            and pull.get("pull_request") == {"url": f"{origin}{CATALOG_API}/pulls/4", "html_url": f"{origin}/{CATALOG}/pull/4"}
            and pull.get("body", "") is None,
            repr(pull),
        )
        own = plant_issue(mock, LOGIN.lower(), "Add qa_mod", b"An old pull request.", pull=True).json() or {}
        checks.equal("T the account's login in other letters is the account", own.get("user"), ACCOUNT)
        harness_refusals = [
            plant_issue(mock, "", "Submit other_mod").status,
            plant_issue(mock, "Stranger", "").status,
            plant_issue(mock, "Stranger", "Submit other_mod", b"\xff\xfe").status,
            mock.harness("/__catalog/issue?creator=Stranger&title=Submit&pull=yes").status,
        ]
        checks.equal("T /__catalog/issue without creator, without title, with a body not UTF-8, with pull=yes: 400 each",
                     harness_refusals, [400] * 4)
        checks.equal("T and they opened nothing", len(mock.catalog()["issues"]), 5)

        def numbers(query: str = "") -> list[int] | bytes:
            return issue_numbers(mock.api("GET", ISSUES_API + query))

        checks.equal("T GET issues: the open ones, newest first, pull entries included", numbers(), [5, 4, 3, 2, 1])
        checks.equal("T creator= picks one author's, pull entries included", numbers(f"?creator={LOGIN}"), [5, 2, 1])
        checks.equal("T creator= matches case-insensitively", numbers(f"?creator={LOGIN.upper()}"), [5, 2, 1])
        checks.equal("T creator= of somebody else", numbers("?creator=stranger"), [4, 3])
        checks.equal("T an unknown creator: []", numbers("?creator=Nobody"), [])
        marks = ["pull_request" in item for item in mock.api("GET", ISSUES_API).json()]
        checks.equal("T the list marks the pull entries and only them", marks, [True, True, False, False, False])

        mock.harness("/__catalog/close?number=2")
        checks.equal("T state=open by default: a closed issue is left out", numbers(), [5, 4, 3, 1])
        checks.equal("T state=open says the same", numbers("?state=open"), [5, 4, 3, 1])
        checks.equal("T state=closed: the closed ones", numbers("?state=closed"), [2])
        checks.equal("T state=all: every one", numbers("?state=all"), [5, 4, 3, 2, 1])
        checks.equal("T creator=, state=all, per_page=100, page=1: the account's, as the editor asks",
                     numbers(f"?creator={LOGIN}&state=all&per_page=100&page=1"), [5, 2, 1])
        invalid = mock.api("GET", f"{ISSUES_API}?state=merged")
        checks.check(
            "T an unknown state: 422 invalid state",
            invalid.status == 422 and invalid.error() == {"resource": "Issue", "code": "invalid", "field": "state"},
            repr(invalid.body),
        )
        checks.equal("T per_page slices the list", numbers("?state=all&per_page=2"), [5, 4])
        checks.equal("T page picks the next slices, then nothing",
                     [numbers(f"?state=all&per_page=2&page={page}") for page in (2, 3, 4)], [[3, 2], [1], []])
        for index in range(31):
            plant_issue(mock, "Filler", f"Filler {index}")
        checks.equal("T 30 a page by default", len(numbers()), 30)
        checks.equal("T per_page=100: all 35 open ones", len(numbers("?per_page=100")), 35)

        one = mock.api("GET", f"{ISSUES_API}/1")
        checks.check("T GET issues/1: the issue as POST answered it", one.status == 200 and one.json() == issue, repr(one.body[:400]))
        checks.equal("T GET a pull entry's number: its issue JSON, pull_request included",
                     mock.api("GET", f"{ISSUES_API}/4").json(), pull)
        checks.equal("T GET issues/99, issues/0 and issues/abc: 404",
                     [mock.api("GET", f"{ISSUES_API}/{number}").status for number in ("99", "0", "abc")], [404] * 3)
        checks.equal("T another repository has no such issue: 404", mock.api("GET", f"/repos/{LOGIN}/darf-mods/issues/1").status, 404)

        state = mock.catalog()
        entries = state["issues"]
        checks.check(
            "T /__catalog: every issue oldest first, each its issue JSON plus an empty comment_list",
            set(state) == {"repositories", "issues"} and [item["number"] for item in entries] == list(range(1, 37))
            and {key: value for key, value in entries[0].items() if key != "comment_list"} == issue
            and {key: value for key, value in entries[3].items() if key != "comment_list"} == pull
            and all(item.get("comment_list") == [] for item in entries),
            repr(entries[:1])[:400],
        )
        requests = mock.state()["requests"]
        logged = [item for item in requests if item["path"].startswith(ISSUES_API)]
        checks.check("T the issue requests are logged with the token", logged and all(item["authorized"] for item in logged),
                     repr(logged[:3]))
        checks.check("T harness requests stay out of the log", not any(item["path"].startswith("/__") for item in requests))


def flow_catalog_public(checks: Checks, options) -> None:
    """U: the catalog's public files on the web, and the harness rewriting them."""
    with tempfile.TemporaryDirectory() as folder:
        seed = write_catalog_seed(Path(folder))
        with Mock("--catalog-seed", folder) as mock:
            index = mock.web("/catalog/public/index.ltx")
            checks.check(
                "U /catalog/public serves the index as UTF-8 text",
                index.status == 200 and index.body == seed["public/index.ltx"]
                and index.headers.get("content-type") == "text/plain; charset=utf-8",
                repr(index.headers),
            )
            icon = mock.web("/catalog/public/icons/qa_mod.dds")
            checks.check(
                "U a .dds is served as octet-stream",
                icon.status == 200 and icon.body == seed["public/icons/qa_mod.dds"]
                and icon.headers.get("content-type") == "application/octet-stream",
                repr(icon.headers),
            )
            checks.equal("U a missing file: 404", mock.web("/catalog/public/missing.ltx").status, 404)
            checks.equal("U nothing outside public/ is served", mock.web("/catalog/public/../mods/other_mod.ltx").status, 404)
            checks.equal("U a folder is no file: 404", mock.web("/catalog/public/icons").status, 404)

            rewritten = b"[catalog]\nversion = 2\n\n[qa_mod]\ntitle = QA Mod\n"
            wrote = mock.harness("/__catalog/file?path=public/index.ltx", rewritten)
            checks.check(
                "U /__catalog/file: a commit on main with the file's blob",
                wrote.status == 200 and wrote.json()["sha"] == git_blob_sha(rewritten) and wrote.json()["branch"] == "main"
                and wrote.json()["commit"] == main_sha(mock),
                repr(wrote.body),
            )
            checks.equal("U the web serves the rewritten index", mock.web("/catalog/public/index.ltx").body, rewritten)
            checks.equal("U the contents API reads it too", read_file(mock, "public/index.ltx", "main"), rewritten)
            added = mock.harness("/__catalog/file?path=public/new.ltx&branch=main", b"new")
            checks.check("U /__catalog/file creates a file", added.status == 200 and mock.web("/catalog/public/new.ltx").body == b"new")
            checks.equal("U /__catalog/file onto a missing branch: 404", mock.harness("/__catalog/file?path=x.ltx&branch=nothing", b"x").status, 404)
            checks.equal("U /__catalog/file without a path: 400", mock.harness("/__catalog/file", b"x").status, 400)
            requests = mock.state()["requests"]
            public = [item for item in requests if item["path"].startswith("/catalog/")]
            checks.check("U the public reads carried no token and no key",
                         public and not any(item["authorized"] or item["vt_key"] for item in public), repr(public))
            checks.check("U harness requests stay out of the log", not any(item["path"].startswith("/__") for item in requests))


def flow_catalog_comments(checks: Checks, options) -> None:
    """X: comments on the catalog's issues and pull entries - the account's own, and the harness speaking as anyone."""
    with Mock() as mock:
        number = open_issue(mock).json()["number"]
        comments = f"{ISSUES_API}/{number}/comments"
        origin = f"http://127.0.0.1:{mock.port}"

        quiet = mock.api("GET", comments)
        checks.equal("X an issue nobody commented on: 200 []", (quiet.status, quiet.json()), (200, []))
        own = mock.api("POST", comments, {"body": "Thanks!"})
        comment = own.json() or {}
        checks.check(
            "X POST comments: 201, written by the token's account",
            own.status == 201 and comment.get("body") == "Thanks!" and comment.get("user") == ACCOUNT
            and comment.get("html_url") == f"{origin}/{CATALOG}/issues/{number}#issuecomment-{comment.get('id')}"
            and comment.get("issue_url") == f"{origin}{ISSUES_API}/{number}"
            and comment.get("url") == f"{origin}{ISSUES_API}/comments/{comment.get('id')}"
            and re.fullmatch(STAMP, comment.get("created_at") or "") and comment.get("updated_at") == comment.get("created_at"),
            repr(own.body[:400]),
        )
        checks.equal("X no body: 422 missing_field", mock.api("POST", comments, {}).error(),
                     {"resource": "IssueComment", "code": "missing_field", "field": "body"})
        checks.equal("X an empty body: 422 missing_field", mock.api("POST", comments, {"body": ""}).error().get("code"), "missing_field")
        checks.equal("X a body that is no string: 422 invalid", mock.api("POST", comments, {"body": 5}).error().get("code"), "invalid")

        verdict = "<!-- xms-catalog verdict=waiting -->\r\nWaiting: VirusTotal is scanning.\r\n\r\n## Report\r\n"
        planted = mock.harness(f"/__catalog/comment?number={number}", verdict.encode("utf-8"))
        bot = planted.json() or []
        checks.check(
            "X /__catalog/comment writes as github-actions[bot] by default, the body byte for byte",
            planted.status == 200 and len(bot) == 1 and bot[0]["body"] == verdict and bot[0]["user"] == BOT,
            repr(planted.body[:400]),
        )
        text = "Одобрено, честно"
        stranger = mock.harness(f"/__catalog/comment?number={number}&author=Stranger", text.encode("utf-8")).json()
        checks.check(
            "X author= writes as somebody else, a User; UTF-8 survives",
            stranger[0]["user"]["login"] == "Stranger" and stranger[0]["user"]["type"] == "User" and stranger[0]["body"] == text,
            repr(stranger),
        )
        other_bot = mock.harness(f"/__catalog/comment?number={number}&author={quote('lookalike[bot]')}", b"beep").json()
        checks.equal("X another app's [bot] account is a Bot too", other_bot[0]["user"]["type"], "Bot")
        listed = mock.api("GET", comments).json()
        checks.equal("X GET lists every comment oldest first",
                     [(item["user"]["login"], item["body"]) for item in listed],
                     [(LOGIN, "Thanks!"), ("github-actions[bot]", verdict), ("Stranger", text), ("lookalike[bot]", "beep")])
        ids = [item["id"] for item in listed]
        checks.check("X comment ids grow", ids == sorted(set(ids)) and ids[0] >= 2150000001, repr(ids))

        filler = mock.harness(f"/__catalog/comment?number={number}&author=Stranger&count=100", b"filler")
        checks.check("X count= plants that many at once", filler.status == 200 and len(filler.json()) == 100, repr(filler.body[:200]))
        checks.equal("X 30 a page by default", len(mock.api("GET", comments).json()), 30)
        checks.equal("X per_page=100: the first hundred", len(mock.api("GET", f"{comments}?per_page=100").json()), 100)
        rest = mock.api("GET", f"{comments}?per_page=100&page=2").json()
        checks.equal("X page=2: the rest, still oldest first", [item["body"] for item in rest], ["filler"] * 4)
        checks.equal("X the issue counts its comments, read alone and listed",
                     [mock.api("GET", f"{ISSUES_API}/{number}").json()["comments"], mock.api("GET", ISSUES_API).json()[0]["comments"]],
                     [104, 104])

        checks.equal("X an issue that does not exist: GET 404", mock.api("GET", f"{ISSUES_API}/99/comments").status, 404)
        checks.equal("X and POST 404", mock.api("POST", f"{ISSUES_API}/99/comments", {"body": "x"}).status, 404)
        checks.equal("X the harness on one: 404", mock.harness("/__catalog/comment?number=99", b"x").status, 404)
        checks.equal("X the harness without a text: 400", mock.harness(f"/__catalog/comment?number={number}").status, 400)
        checks.equal("X the harness with count=0 or count=1001: 400",
                     [mock.harness(f"/__catalog/comment?number={number}&count={count}", b"x").status for count in (0, 1001)], [400, 400])
        checks.equal("X the harness with a text that is not UTF-8: 400",
                     mock.harness(f"/__catalog/comment?number={number}", b"\xff").status, 400)
        checks.equal("X another repository has no such issue: 404",
                     mock.api("GET", f"/repos/{LOGIN}/darf-mods/issues/{number}/comments").status, 404)
        checks.equal("X comments need the token: 401", mock.call("GET", comments).status, 401)

        pull = plant_issue(mock, "Stranger", "Add other_mod", pull=True).json()["number"]
        posted = mock.api("POST", f"{ISSUES_API}/{pull}/comments", {"body": "Please submit it as an issue."}).json() or {}
        answered = (mock.harness(f"/__catalog/comment?number={pull}", b"Pull requests are not reviewed here.").json() or [{}])[0]
        checks.check(
            "X a pull entry takes comments too, their html_url the pull request's",
            posted.get("html_url") == f"{origin}/{CATALOG}/pull/{pull}#issuecomment-{posted.get('id')}"
            and answered.get("html_url") == f"{origin}/{CATALOG}/pull/{pull}#issuecomment-{answered.get('id')}"
            and posted.get("issue_url") == answered.get("issue_url") == f"{origin}{ISSUES_API}/{pull}",
            repr((posted, answered))[:400],
        )
        checks.equal("X and keeps them apart from the issue's",
                     [item["id"] for item in mock.api("GET", f"{ISSUES_API}/{pull}/comments").json()],
                     [posted.get("id"), answered.get("id")])

        mock.harness(f"/__catalog/close?number={number}")
        late = mock.api("POST", comments, {"body": "Closed, then commented."})
        refused = mock.harness(f"/__catalog/comment?number={number}", b"<!-- xms-catalog verdict=refused -->\nRefused.\n")
        checks.check("X a closed issue still takes comments, from both",
                     late.status == 201 and refused.status == 200 and mock.api("GET", f"{ISSUES_API}/{number}").json()["comments"] == 106,
                     repr(late.body[:200]))

        entry = next((item for item in mock.catalog()["issues"] if item["number"] == number), {})
        tail = mock.api("GET", f"{comments}?per_page=100&page=2").json()
        checks.check(
            "X /__catalog: the issue's comment_list, oldest first, each comment as the API writes it",
            len(entry.get("comment_list", [])) == entry.get("comments") == 106
            and entry["comment_list"][0] == comment and entry["comment_list"][100:] == tail,
            repr(entry)[:400],
        )
        requests = mock.state()["requests"]
        logged = [item for item in requests if "/comments" in item["path"]]
        checks.check("X the API's comment requests are logged with the token",
                     logged and all(item["authorized"] for item in logged if item["status"] != 401), repr(logged[:3]))
        checks.check("X harness comments stay out of the log", not any(item["path"].startswith("/__") for item in requests))


def flow_catalog_issue_edits(checks: Checks, options) -> None:
    """Y: editing and closing issues - the author, somebody else's issue, the workflow closing one, the owner."""
    new_body = "```ini\n[mod]\nid = qa_mod\nversion = 1.0.1\n```\n"
    with Mock() as mock:
        mine = open_issue(mock).json()
        path = f"{ISSUES_API}/{mine['number']}"

        edited = mock.api("PATCH", path, {"title": "Submit qa_mod 1.0.1", "body": new_body})
        result = edited.json() or {}
        checks.check(
            "Y the author edits title and body: 200, still open, updated_at a time no earlier than before",
            edited.status == 200 and result.get("title") == "Submit qa_mod 1.0.1" and result.get("body") == new_body
            and result.get("state") == "open" and re.fullmatch(STAMP, result.get("updated_at") or "")
            and result["updated_at"] >= mine["updated_at"] and result.get("created_at") == mine["created_at"],
            repr(edited.body[:400]),
        )
        checks.equal("Y and it reads back", mock.api("GET", path).json(), result)
        body_only = mock.api("PATCH", path, {"body": SUBMISSION}).json() or {}
        checks.check("Y a PATCH with the body alone keeps the title",
                     body_only.get("title") == "Submit qa_mod 1.0.1" and body_only.get("body") == SUBMISSION, repr(body_only))
        cleared = mock.api("PATCH", path, {"body": None}).json() or {}
        checks.equal("Y body null clears the text", cleared.get("body", ""), None)
        mock.api("PATCH", path, {"body": SUBMISSION})

        closed = mock.api("PATCH", path, {"state": "closed", "state_reason": "not_planned"})
        closure = closed.json() or {}
        checks.check(
            "Y the author closes it as not planned: state, state_reason, closed_at, closed_by",
            closed.status == 200 and closure.get("state") == "closed" and closure.get("state_reason") == "not_planned"
            and re.fullmatch(STAMP, closure.get("closed_at") or "") and closure.get("closed_by") == ACCOUNT,
            repr(closed.body[:400]),
        )
        checks.equal("Y and the list files it under closed", issue_numbers(mock.api("GET", f"{ISSUES_API}?state=closed")), [1])
        reopened = mock.api("PATCH", path, {"state": "open"})
        back = reopened.json() or {}
        checks.check(
            "Y the author reopens what they closed: reopened, closed_at and closed_by cleared",
            reopened.status == 200 and back.get("state") == "open" and back.get("state_reason") == "reopened"
            and back.get("closed_at", "") is None and back.get("closed_by", "") is None,
            repr(reopened.body[:400]),
        )
        plain = mock.api("PATCH", path, {"state": "closed"}).json() or {}
        checks.equal("Y closing without a reason: completed", (plain.get("state"), plain.get("state_reason")), ("closed", "completed"))
        wrong_way = mock.api("PATCH", path, {"state": "open", "state_reason": "completed"})
        checks.check("Y reopening as completed: 422 invalid state_reason",
                     wrong_way.status == 422 and wrong_way.error() == {"resource": "Issue", "code": "invalid", "field": "state_reason"},
                     repr(wrong_way.body))
        explicit = mock.api("PATCH", path, {"state": "open", "state_reason": "reopened"})
        checks.check("Y reopening that names reopened: 200", explicit.status == 200 and (explicit.json() or {}).get("state") == "open",
                     repr(explicit.body[:300]))

        before = mock.api("GET", path).json()
        refusals = {
            "an empty title": ({"title": ""}, "missing_field", "title"),
            "a title that is no string": ({"title": 7}, "invalid", "title"),
            "a body that is no string": ({"body": {"text": "x"}}, "invalid", "body"),
            "an unknown state next to a good title": ({"state": "merged", "title": "Changed"}, "invalid", "state"),
            "a state that is no string": ({"state": False}, "invalid", "state"),
            "an unknown state_reason": ({"state": "closed", "state_reason": "wontfix"}, "invalid", "state_reason"),
            "closing as reopened": ({"state": "closed", "state_reason": "reopened"}, "invalid", "state_reason"),
            "a state_reason that is no string": ({"state": "closed", "state_reason": 1}, "invalid", "state_reason"),
        }
        for name, (body, code, field) in refusals.items():
            reply = mock.api("PATCH", path, body)
            checks.check(f"Y PATCH with {name}: 422 {code} {field}",
                         reply.status == 422 and reply.error() == {"resource": "Issue", "code": code, "field": field}, repr(reply.body))
        checks.equal("Y the refused PATCHes changed nothing", mock.api("GET", path).json(), before)
        checks.equal("Y PATCH of an issue that does not exist: 404", mock.api("PATCH", f"{ISSUES_API}/99", {"state": "closed"}).status, 404)

        theirs = plant_issue(mock, "Stranger", "Submit other_mod", b"Their submission.").json()
        their_path = f"{ISSUES_API}/{theirs['number']}"
        denied = [mock.api("PATCH", their_path, body) for body in ({"title": "Hijacked"}, {"body": "Hijacked"}, {"state": "closed"}, {})]
        checks.check(
            "Y somebody else's issue: 403 Must have admin rights, whatever the PATCH says",
            all(reply.status == 403 and reply.json() == FORBIDDEN for reply in denied),
            repr([reply.body for reply in denied]),
        )
        checks.equal("Y and nothing changed", mock.api("GET", their_path).json(), theirs)

        verdict = mock.harness(f"/__catalog/close?number={mine['number']}")
        closure = verdict.json() or {}
        checks.check(
            "Y /__catalog/close: closed by github-actions[bot] as completed",
            verdict.status == 200 and closure.get("state") == "closed" and closure.get("state_reason") == "completed"
            and closure.get("closed_by") == BOT and re.fullmatch(STAMP, closure.get("closed_at") or ""),
            repr(verdict.body[:400]),
        )
        reopen = mock.api("PATCH", path, {"state": "open"})
        checks.check("Y the author cannot reopen what the workflow closed: 403", reopen.status == 403 and reopen.json() == FORBIDDEN,
                     repr(reopen.body))
        checks.equal("Y and it stays as the workflow left it", mock.api("GET", path).json(), closure)
        note = mock.api("PATCH", path, {"body": "A note after the verdict.", "state": "closed", "state_reason": "not_planned"})
        noted = note.json() or {}
        checks.check(
            "Y the author may still edit it; closing it again keeps the workflow's closure",
            note.status == 200 and noted.get("body") == "A note after the verdict." and noted.get("state") == "closed"
            and noted.get("state_reason") == "completed" and noted.get("closed_by") == BOT
            and noted.get("closed_at") == closure.get("closed_at"),
            repr(note.body[:400]),
        )
        checks.equal("Y /__catalog/close of a closed issue: 422", mock.harness(f"/__catalog/close?number={mine['number']}").status, 422)

        other = open_issue(mock).json()["number"]
        named = mock.harness(f"/__catalog/close?number={other}&reason=not_planned&by={CATALOG_OWNER}").json() or {}
        checks.check(
            "Y reason= and by= name the closure",
            named.get("state_reason") == "not_planned" and (named.get("closed_by") or {}).get("login") == CATALOG_OWNER
            and (named.get("closed_by") or {}).get("type") == "User",
            repr(named),
        )
        own = open_issue(mock).json()["number"]
        mock.harness(f"/__catalog/close?number={own}&by={LOGIN}")
        checks.equal("Y closed by= the account itself, the author may reopen it",
                     mock.api("PATCH", f"{ISSUES_API}/{own}", {"state": "open"}).status, 200)
        checks.equal("Y /__catalog/close of an unknown number: 404", mock.harness("/__catalog/close?number=99").status, 404)
        checks.equal("Y /__catalog/close for another reason: 400, and the issue stays open",
                     (mock.harness(f"/__catalog/close?number={own}&reason=wontfix").status,
                      mock.api("GET", f"{ISSUES_API}/{own}").json()["state"]),
                     (400, "open"))
        entry = plant_issue(mock, LOGIN, "Add qa_mod", pull=True).json()["number"]
        shut = mock.harness(f"/__catalog/close?number={entry}").json() or {}
        checks.check("Y the harness closes a pull entry as well", shut.get("state") == "closed" and "pull_request" in shut, repr(shut))

    with Mock("--login", CATALOG_OWNER.lower()) as mock:
        checks.equal("Y --login in other letters owns the catalog too: push",
                     mock.api("GET", CATALOG_API).json()["permissions"]["push"], True)
        submitted = plant_issue(mock, LOGIN, SUBMISSION_TITLE, SUBMISSION.encode("utf-8")).json()
        path = f"{ISSUES_API}/{submitted['number']}"
        mock.harness(f"/__catalog/close?number={submitted['number']}")
        edited = mock.api("PATCH", path, {"title": "Submit qa_mod (checked)", "body": "Checked by the maintainer."})
        result = edited.json() or {}
        checks.check(
            "Y the owner edits anybody's issue; the author stays the author",
            edited.status == 200 and result.get("title") == "Submit qa_mod (checked)"
            and (result.get("user") or {}).get("login") == LOGIN,
            repr(edited.body[:300]),
        )
        reopened = mock.api("PATCH", path, {"state": "open"})
        checks.check("Y and reopens what the workflow closed",
                     reopened.status == 200 and (reopened.json() or {}).get("state_reason") == "reopened", repr(reopened.body[:300]))
        closed = mock.api("PATCH", path, {"state": "closed", "state_reason": "not_planned"}).json() or {}
        checks.equal("Y and closes it as themselves", (closed.get("closed_by"), closed.get("state_reason")),
                     ({"login": CATALOG_OWNER.lower(), "id": 1001, "type": "User"}, "not_planned"))


def flow_vt_analysis(checks: Checks, options) -> None:
    """Z: VirusTotal still analysing - a known file queued by the harness, a forgotten one, and a new
    upload that answers 404 until its first analysis is done (--vt-hide-new)."""
    sample = b"a package VirusTotal already knows"
    known = sha256(sample)
    with tempfile.TemporaryDirectory() as folder:
        seed_path = Path(folder, "vt.json")
        seed_path.write_text(json.dumps({known: {"EngineA": "undetected", "EngineB": "harmless"}}), encoding="utf-8")
        with Mock("--vt-seed", str(seed_path), "--vt-queue", "2", "--vt-hide-new") as mock:
            queued = mock.harness(f"/__vt/queue?sha={known}&lookups=2")
            checks.check("Z /__vt/queue: the file waits for its report", queued.status == 200 and
                         queued.json()["files"][known]["queued"] == 2, repr(queued.body[:300]))
            first = mock.vt("GET", f"{VT_API}/files/{known}")
            attributes = (first.json() or {}).get("data", {}).get("attributes", {})
            checks.check("Z a known file being analysed: its report without results, not a 404",
                         first.status == 200 and attributes.get("last_analysis_results") == {}, repr(first.body[:300]))
            mock.vt("GET", f"{VT_API}/files/{known}")
            later = mock.vt("GET", f"{VT_API}/files/{known}").json()["data"]["attributes"]
            checks.equal("Z a later lookup has the results again", sorted(later["last_analysis_results"]), ["EngineA", "EngineB"])

            fresh = hashlib.sha256(b"never seen").hexdigest()
            planted = mock.harness(f"/__vt/queue?sha={fresh}&lookups=1")
            entry = planted.json()["files"].get(fresh, {})
            checks.check("Z a file VirusTotal did not know is queued with --vt-upload-results",
                         planted.status == 200 and entry.get("queued") == 1 and entry.get("results") == VT_DEFAULT_RESULTS,
                         repr(entry))
            checks.equal("Z --vt-hide-new: while its first analysis is queued, 404",
                         mock.vt("GET", f"{VT_API}/files/{fresh}").status, 404)
            checks.equal("Z and once it is done, the report", mock.vt("GET", f"{VT_API}/files/{fresh}").status, 200)

            data = os.urandom(50000)
            digest = sha256(data)
            sent = mock.vt_upload(f"{VT_API}/files", data)
            statuses = [mock.vt("GET", f"{VT_API}/files/{digest}").status for _ in range(3)]
            checks.check("Z an upload never analysed before: 404, 404, then the report (--vt-queue 2)",
                         sent.status == 200 and statuses == [404, 404, 200], repr(statuses))
            again = mock.vt_upload(f"{VT_API}/files", data)
            reanalysed = mock.vt("GET", f"{VT_API}/files/{digest}")
            checks.check("Z the same file uploaded again is known: a report without results while queued",
                         again.status == 200 and reanalysed.status == 200 and
                         reanalysed.json()["data"]["attributes"]["last_analysis_results"] == {}, repr(reanalysed.body[:300]))

            forgot = mock.harness(f"/__vt/forget?sha={digest}")
            checks.check("Z /__vt/forget: VirusTotal no longer knows it, nor its analyses",
                         forgot.status == 200 and digest not in forgot.json()["files"] and
                         not [a for a in forgot.json()["analyses"] if a["sha256"] == digest], repr(forgot.body[:300]))
            checks.equal("Z a forgotten file: 404", mock.vt("GET", f"{VT_API}/files/{digest}").status, 404)
            checks.equal("Z forgetting a file VirusTotal does not know: 404", mock.harness(f"/__vt/forget?sha={digest}").status, 404)
            checks.equal("Z /__vt/queue without a SHA-256: 400", mock.harness("/__vt/queue?sha=xyz").status, 400)
            checks.equal("Z /__vt/queue with lookups=0: 400", mock.harness(f"/__vt/queue?sha={known}&lookups=0").status, 400)
            checks.check("Z harness requests stay out of the log",
                         not any(item["path"].startswith("/__") for item in mock.state()["requests"]))


def flow_vt(checks: Checks, options) -> None:
    """V: VirusTotal - seeded verdicts, an upload through the queue, analyses, upload URLs, the log."""
    sample = b"a sample VirusTotal already knows"
    known = sha256(sample)
    seeded = {known: {"EngineA": "malicious", "EngineB": "undetected", "EngineC": "type-unsupported"}}
    uploaded_results = {
        "EngineA": {"category": "undetected", "engine_name": "EngineA", "result": None},
        "EngineB": {"category": "undetected", "engine_name": "EngineB", "result": None},
        "EngineC": {"category": "harmless", "engine_name": "EngineC", "result": None},
    }
    uploaded_stats = {**VT_ZERO, "undetected": 2, "harmless": 1}
    with tempfile.TemporaryDirectory() as folder:
        seed_path, log_path = Path(folder, "vt.json"), Path(folder, "requests.jsonl")
        seed_path.write_text(json.dumps(seeded), encoding="utf-8")
        with Mock("--vt-seed", str(seed_path), "--log", str(log_path)) as mock:
            data = os.urandom(200000)
            digest = sha256(data)
            unknown = mock.vt("GET", f"{VT_API}/files/{digest}")
            checks.check(
                "V an unknown file: 404 NotFoundError",
                unknown.status == 404
                and unknown.json() == {"error": {"code": "NotFoundError", "message": f'File "{digest}" not found'}},
                repr(unknown.body),
            )
            report = mock.vt("GET", f"{VT_API}/files/{known}").json()["data"]
            attributes = report["attributes"]
            checks.check(
                "V a seeded file: its verdicts, a detection named",
                report["id"] == known and report["type"] == "file" and attributes["sha256"] == known
                and isinstance(attributes.get("last_analysis_date"), int)
                and attributes["last_analysis_results"] == {
                    "EngineA": {"category": "malicious", "engine_name": "EngineA", "result": "Trojan.Generic"},
                    "EngineB": {"category": "undetected", "engine_name": "EngineB", "result": None},
                    "EngineC": {"category": "type-unsupported", "engine_name": "EngineC", "result": None},
                },
                repr(report),
            )
            checks.equal("V a seeded file: its stats", attributes["last_analysis_stats"],
                         {**VT_ZERO, "malicious": 1, "undetected": 1, "type-unsupported": 1})
            checks.equal("V a hash in capitals finds the same file", mock.vt("GET", f"{VT_API}/files/{known.upper()}").status, 200)

            sent = mock.vt_upload(f"{VT_API}/files", data)
            analysis = (sent.json() or {}).get("data", {})
            checks.check(
                "V POST /files: 200 with an analysis",
                sent.status == 200 and analysis.get("type") == "analysis" and analysis.get("id")
                and analysis.get("links", {}).get("self") == f"http://127.0.0.1:{mock.port}{VT_API}/analyses/{analysis.get('id')}",
                repr(sent.body),
            )
            queued = mock.vt("GET", f"{VT_API}/files/{digest}").json()["data"]["attributes"]
            checks.check(
                "V right after the upload the report is queued: no date, no verdicts, zero stats",
                queued["sha256"] == digest and queued["size"] == len(data) and "last_analysis_date" not in queued
                and queued["last_analysis_results"] == {} and queued["last_analysis_stats"] == VT_ZERO,
                repr(queued),
            )
            done = mock.vt("GET", f"{VT_API}/files/{digest}").json()["data"]["attributes"]
            checks.check(
                "V --vt-queue 1: the next lookup has the verdicts of --vt-upload-results",
                done["last_analysis_results"] == uploaded_results and done["last_analysis_stats"] == uploaded_stats
                and isinstance(done.get("last_analysis_date"), int),
                repr(done),
            )
            checks.equal(
                "V the analysis: completed, with the stats and the file",
                mock.vt("GET", f"{VT_API}/analyses/{analysis.get('id')}").json(),
                {"data": {"id": analysis.get("id"), "type": "analysis", "attributes": {"status": "completed", "stats": uploaded_stats}},
                 "meta": {"file_info": {"sha256": digest, "size": len(data)}}},
            )
            checks.equal("V an unknown analysis: 404", mock.vt("GET", f"{VT_API}/analyses/nothing").status, 404)

            again = mock.vt_upload(f"{VT_API}/files", sample, "sample.bin").json()["data"]["id"]
            first = mock.vt("GET", f"{VT_API}/analyses/{again}").json()["data"]["attributes"]
            second = mock.vt("GET", f"{VT_API}/analyses/{again}").json()["data"]["attributes"]
            checks.check(
                "V GET /analyses counts as a lookup: queued, then completed",
                first == {"status": "queued", "stats": VT_ZERO} and second["status"] == "completed",
                f"{first}, {second}",
            )
            checks.equal("V a known file uploaded again keeps its verdicts", second["stats"]["malicious"], 1)

            url = mock.vt("GET", f"{VT_API}/files/upload_url")
            address = (url.json() or {}).get("data", "")
            checks.check(
                "V upload_url: an address on the mock's origin",
                url.status == 200 and re.fullmatch(rf"http://127\.0\.0\.1:{mock.port}/vt/upload/[A-Za-z0-9_-]+", address),
                repr(url.body),
            )
            big = os.urandom(3 * 1024 * 1024)
            path = urlsplit(address).path
            through = mock.vt_upload(path, big, "big.zip")
            checks.check("V POST to the upload URL: 200 with an analysis",
                         through.status == 200 and through.json()["data"]["type"] == "analysis", repr(through.body))
            reused = mock.vt_upload(path, big, "big.zip")
            checks.check(
                "V the upload URL is single use: 404",
                reused.status == 404 and reused.json()["error"]["code"] == "NotFoundError",
                repr(reused.body),
            )

            state = mock.vt_state()
            checks.equal(
                "V /__vt: the upload with its size, verdicts, count and name",
                state["files"].get(digest),
                {"sha256": digest, "size": len(data), "queued": 0, "uploads": 1, "names": ["qa_mod-1.0.0.zip"],
                 "results": {"EngineA": "undetected", "EngineB": "undetected", "EngineC": "harmless"}},
            )
            sample_entry = state["files"].get(known) or {}
            checks.check(
                "V /__vt: the seeded file keeps its verdicts and learnt its size from the upload",
                sample_entry.get("results") == seeded[known] and sample_entry.get("size") == len(sample) and sample_entry.get("uploads") == 1,
                repr(sample_entry),
            )
            big_entry = state["files"].get(sha256(big)) or {}
            checks.check(
                "V /__vt: the file sent to the upload URL arrived whole, its report queued",
                big_entry.get("size") == len(big) and big_entry.get("queued") == 1 and big_entry.get("names") == ["big.zip"],
                repr(big_entry),
            )
            checks.equal("V /__vt: every analysis in order", [item["status"] for item in state["analyses"]], ["completed", "completed", "queued"])

            added = sha256(b"added at run time")
            checks.equal("V /__vt/seed: 200", mock.call("POST", "/__vt/seed", {added: {"EngineX": "suspicious"}}).status, 200)
            report = mock.vt("GET", f"{VT_API}/files/{added}").json()["data"]["attributes"]
            checks.check(
                "V /__vt/seed: the file is known at once, size 0",
                report["last_analysis_results"] == {"EngineX": {"category": "suspicious", "engine_name": "EngineX", "result": "Trojan.Generic"}}
                and report["size"] == 0,
                repr(report),
            )
            checks.equal("V /__vt/seed refuses an unknown category: 400", mock.call("POST", "/__vt/seed", {added: {"EngineX": "evil"}}).status, 400)
            checks.equal("V /__vt/seed refuses a key that is no SHA-256: 400", mock.call("POST", "/__vt/seed", {"abc": {}}).status, 400)

            mock.api("GET", "/user")
            mock.web("/catalog/public/index.ltx")

        lines = [json.loads(line) for line in log_path.read_text(encoding="utf-8").splitlines()]
        fields = ["n", "method", "path", "status", "authorized", "vt_key", "bytes_in", "time"]
        checks.check("V --log: vt_key sits next to authorized in every line", lines and all(list(line) == fields for line in lines), repr(lines[:1]))
        vt_lines = [line for line in lines if line["path"].startswith("/vt/")]
        others = [line for line in lines if not line["path"].startswith("/vt/")]
        checks.check("V VirusTotal requests carried the key and no token",
                     vt_lines and all(line["vt_key"] and not line["authorized"] for line in vt_lines), repr(vt_lines[:3]))
        checks.equal(
            "V the other requests: the token on the API only, the key nowhere",
            [(line["path"], line["authorized"], line["vt_key"]) for line in others],
            [("/user", True, False), ("/catalog/public/index.ltx", False, False)],
        )
        checks.check("V harness requests stay out of the log", not any(line["path"].startswith("/__") for line in lines))
        uploads = [line for line in vt_lines if line["path"].startswith("/vt/upload/")]
        checks.check(
            "V the upload URL requests are logged with their status and size",
            [line["status"] for line in uploads] == [200, 404] and all(line["bytes_in"] > len(big) for line in uploads),
            repr(uploads),
        )


def flow_vt_faults(checks: Checks, options) -> None:
    """W: VirusTotal's key, a leaked GitHub token, the size limit, the quota and malformed forms."""
    missing = f"{VT_API}/files/{'0' * 64}"
    leaked = {"Authorization": f"Bearer {TOKEN}"}
    with Mock("--vt-max-upload", "4096") as mock:
        wrong = mock.vt("GET", missing, key="wrong-key")
        checks.check(
            "W a wrong key: 401 WrongCredentialsError",
            wrong.status == 401 and wrong.json() == {"error": {"code": "WrongCredentialsError", "message": "Wrong API key"}},
            repr(wrong.body),
        )
        checks.equal("W no key: 401", mock.vt("GET", missing, key=None).status, 401)
        leak = mock.vt("GET", missing, headers=leaked)
        checks.check(
            "W a GitHub token sent to VirusTotal: 400",
            leak.status == 400
            and leak.json() == {"error": {"code": "BadRequestError", "message": "Authorization header sent to VirusTotal"}},
            repr(leak.body),
        )
        checks.equal("W the token is refused before the key is looked at", mock.vt("GET", missing, key="wrong-key", headers=leaked).status, 400)
        denied = mock.vt_upload(f"{VT_API}/files", os.urandom(2 * 1024 * 1024), key="wrong-key")
        checks.equal("W a 2 MB upload with a wrong key: 401 after the body", denied.status, 401)

        large = os.urandom(8192)
        refused = mock.vt_upload(f"{VT_API}/files", large)
        checks.check(
            "W a body over --vt-max-upload: 413 RequestEntityTooLargeError",
            refused.status == 413 and refused.json()["error"]["code"] == "RequestEntityTooLargeError",
            repr(refused.body),
        )
        checks.equal("W refused uploads store nothing", mock.vt_state()["files"], {})
        address = urlsplit(mock.vt("GET", f"{VT_API}/files/upload_url").json()["data"]).path
        checks.equal("W the same file through an upload URL: 200", mock.vt_upload(address, large).status, 200)
        exact = os.urandom(4096 - len(multipart(b"")[0]))
        checks.equal("W a body of exactly --vt-max-upload bytes: 200", mock.vt_upload(f"{VT_API}/files", exact).status, 200)
        checks.equal("W one byte more: 413", mock.vt_upload(f"{VT_API}/files", exact + b"x").status, 413)

        requests = [item for item in mock.state()["requests"] if item["path"] == missing]
        checks.equal(
            "W the log flags the token and the key of every request",
            [(item["status"], item["authorized"], item["vt_key"]) for item in requests],
            [(401, False, True), (401, False, False), (400, True, True), (400, True, True)],
        )

    with Mock() as mock:
        boundary = "GrizzlyEditorBad"
        form = f"multipart/form-data; boundary={boundary}"
        part = f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="a.bin"\r\n\r\n'
        close = f"\r\n--{boundary}--\r\n"
        malformed = {
            "a body that is not multipart": (b"raw bytes", "application/octet-stream"),
            "no boundary parameter": ((part + "x" + close).encode(), "multipart/form-data"),
            "a boundary the body does not use": ((part + "x" + close).encode(), "multipart/form-data; boundary=Other"),
            "a part without a filename":
                ((f'--{boundary}\r\nContent-Disposition: form-data; name="file"\r\n\r\nx' + close).encode(), form),
            "a field that is not file":
                ((f'--{boundary}\r\nContent-Disposition: form-data; name="upload"; filename="a.bin"\r\n\r\nx' + close).encode(), form),
            "a second part":
                ((part + "x" + f'\r\n--{boundary}\r\nContent-Disposition: form-data; name="password"\r\n\r\nsecret' + close).encode(), form),
            "no closing boundary": ((part + "x").encode(), form),
            "no part at all": (f"--{boundary}--\r\n".encode(), form),
        }
        for name, (body, content_type) in malformed.items():
            reply = mock.vt("POST", f"{VT_API}/files", body, headers={"Content-Type": content_type})
            checks.check(
                f"W {name}: 400 BadRequestError",
                reply.status == 400 and ((reply.json() or {}).get("error") or {}).get("code") == "BadRequestError",
                repr(reply.body[:200]),
            )
        checks.equal("W malformed forms store nothing", mock.vt_state()["files"], {})

        # The parser against the mock's block reads: every byte of a delimiter but its last one
        # ends the first read, and the data is full of delimiters that miss by one byte.
        straddle = "GrizzlyEditorStraddle"
        cut = len(f"\r\n--{straddle}") - 1
        head = f'--{straddle}\r\nContent-Disposition: form-data; name="file"; filename="edge.bin"\r\n\r\n'.encode()
        near = f"\r\n--{straddle[:-1]}".encode()
        data = (near * (MOCK_READ_CHUNK // len(near) + 1))[:MOCK_READ_CHUNK - len(head) - cut]
        body = head + data + f"\r\n--{straddle}--\r\n".encode()
        reply = mock.vt("POST", f"{VT_API}/files", body, headers={"Content-Type": f'multipart/form-data; boundary="{straddle}"'})
        entry = mock.vt_state()["files"].get(sha256(data)) or {}
        checks.check(
            "W a closing boundary cut before its last byte by two reads, near misses, a quoted boundary: the exact bytes",
            reply.status == 200 and entry.get("size") == len(data),
            f"{reply.status} {reply.body[:200]!r} {entry}",
        )
        payload = os.urandom(1000)
        body = b"P" * (MOCK_READ_CHUNK - cut) + b"\r\n" + multipart(payload, "late.bin", straddle)[0]
        reply = mock.vt("POST", f"{VT_API}/files", body, headers={"Content-Type": f"multipart/form-data; boundary={straddle}"})
        entry = mock.vt_state()["files"].get(sha256(payload)) or {}
        checks.check(
            "W a preamble that cuts the first boundary before its last byte",
            reply.status == 200 and entry.get("size") == len(payload) and entry.get("names") == ["late.bin"],
            f"{reply.status} {reply.body[:200]!r} {entry}",
        )

    with Mock("--vt-rate", "2") as mock:
        checks.equal("W --vt-rate: a wrong key is answered before the quota", mock.vt("GET", missing, key="wrong-key").status, 401)
        checks.equal("W --vt-rate 2: two requests within a minute, then 429", [mock.vt("GET", missing).status for _ in range(3)], [404, 404, 429])
        limited = mock.vt("GET", f"{VT_API}/files/upload_url")
        checks.check(
            "W the quota covers every VirusTotal route: 429 QuotaExceededError",
            limited.status == 429 and limited.json() == {"error": {"code": "QuotaExceededError", "message": "Quota exceeded"}},
            repr(limited.body),
        )
        checks.equal("W GitHub routes are outside the quota", mock.api("GET", "/user").status, 200)
        checks.equal("W so are harness routes", mock.call("GET", "/__vt").status, 200)

    with Mock("--vt-queue", "2", "--vt-upload-results", '{"EngineZ": "malicious"}') as mock:
        data = os.urandom(1000)
        mock.vt_upload(f"{VT_API}/files", data)
        looks = [mock.vt("GET", f"{VT_API}/files/{sha256(data)}").json()["data"]["attributes"]["last_analysis_results"] for _ in range(3)]
        checks.equal(
            "W --vt-queue 2 and --vt-upload-results: two queued lookups, then that verdict",
            looks,
            [{}, {}, {"EngineZ": {"category": "malicious", "engine_name": "EngineZ", "result": "Trojan.Generic"}}],
        )

    with Mock("--vt-queue", "0", "--vt-key", "other-key") as mock:
        data = os.urandom(1000)
        checks.equal("W --vt-key replaces the default", mock.vt_upload(f"{VT_API}/files", data).status, 401)
        analysis = mock.vt_upload(f"{VT_API}/files", data, key="other-key").json()["data"]["id"]
        status = mock.vt("GET", f"{VT_API}/analyses/{analysis}", key="other-key").json()["data"]["attributes"]["status"]
        checks.equal("W --vt-queue 0: the analysis is completed at once", status, "completed")

    refused = subprocess.run([mock_exe(), "--vt-upload-results", '{"EngineA": "evil"}'],
                             capture_output=True, text=True, timeout=60)
    checks.check("W an unknown category in --vt-upload-results stops the mock: exit 2",
                 refused.returncode == 2 and "evil" in refused.stderr, refused.stderr[-300:])


FLOWS = {
    "A": flow_happy_path,
    "C": flow_second_release,
    "D": flow_existing_tag,
    "E": flow_dropped_upload,
    "F": flow_duplicate_asset,
    "G": flow_access,
    "H": flow_asset_faults,
    "I": flow_make_latest,
    "J": flow_device,
    "K": flow_seed,
    "L": flow_range,
    "M": flow_throttle,
    "N": flow_large_upload,
    "O": flow_explicit_port,
    "P": flow_parallel,
    "Q": flow_tags,
    "R": flow_catalog_repo,
    "S": flow_catalog_contents,
    "T": flow_catalog_issues,
    "U": flow_catalog_public,
    "V": flow_vt,
    "W": flow_vt_faults,
    "X": flow_catalog_comments,
    "Y": flow_catalog_issue_edits,
    "Z": flow_vt_analysis,
}


def main() -> int:
    parser = argparse.ArgumentParser(description="Self-test of the Go github_mock.")
    parser.add_argument("--only", help="comma-separated flow letters (B runs with A)")
    parser.add_argument("--skip-large", action="store_true", help="skip the 120 MB upload")
    options = parser.parse_args()
    wanted = {letter.strip().upper().replace("B", "A") for letter in options.only.split(",")} if options.only else set(FLOWS)
    if unknown := wanted - set(FLOWS):
        parser.error(f"unknown flows: {', '.join(sorted(unknown))}")

    checks = Checks()
    for letter, flow in FLOWS.items():
        if letter not in wanted:
            continue
        try:
            flow(checks, options)
        except Exception as error:
            traceback.print_exc()
            checks.check(f"{letter} flow ran to the end", False, f"{type(error).__name__}: {error}")
    print(f"GITHUB MOCK: DONE {checks.passed}/{checks.total}", flush=True)
    return 0 if checks.passed == checks.total else 1


if __name__ == "__main__":
    raise SystemExit(main())
