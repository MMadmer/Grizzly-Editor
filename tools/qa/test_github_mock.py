#!/usr/bin/env python3
"""Self-test of github_mock.py.

Starts the mock as a subprocess, once per flag set, and drives it the way the
editor's release publisher will. Prints one PASS/FAIL line per check and ends
with "GITHUB MOCK: DONE passed/total"; the exit code is 0 only when every check
passed.

Usage:
    python tools/qa/test_github_mock.py [--only A,E,J] [--skip-large]
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import http.client
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
import traceback
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote, urlencode, urlsplit


MOCK = Path(__file__).with_name("github_mock.py")
TOKEN = "qa-token"
REPO = "QaOwner/qa-mod"
API = f"/repos/{REPO}"
USER_AGENT = "XFinedEditor-QA/1.0"
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
DESCRIPTOR = "qa_mod.update.ltx"
LATEST_DESCRIPTOR = f"/{REPO}/releases/latest/download/{DESCRIPTOR}"
API_ROOTS = ("/repos", "/upload", "/user")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


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
            [sys.executable, str(MOCK), *self.flags], stdout=subprocess.PIPE, text=True
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
            and all(set(item) == {"n", "method", "path", "status", "authorized", "bytes_in"} for item in requests),
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
        checks.check("G GET /user: 200 qa-user", user.status == 200 and user.json() == {"login": "qa-user"})

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

    with Mock("--protect-tags") as mock:
        release = publish_release(mock, "v2.0.0", assets)
        mock.api("DELETE", f"{API}/releases/{release['id']}")
        reply = mock.api("DELETE", f"{API}/git/refs/tags/v2.0.0")
        checks.check("Q --protect-tags refuses the deletion: 422 with GitHub's words",
                     reply.status == 422 and "protected" in (reply.json() or {}).get("message", ""), str(reply.body))
        checks.equal("Q and the tag stays", mock.state()["tags"], ["v2.0.0"])


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
}


def main() -> int:
    parser = argparse.ArgumentParser(description="Self-test of github_mock.py.")
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
