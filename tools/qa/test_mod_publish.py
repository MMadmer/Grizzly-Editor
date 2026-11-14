#!/usr/bin/env python3
"""End-to-end check of Mod > Publish Release against tools/qa/github_mock (Go).

Starts the editor with -qa_github on a project, drives release.publish over the MCP
socket and takes every verdict from the mock's own state. Never talks to GitHub:
in QA mode the editor reads only GRIZZLY_GITHUB_TOKEN and a separate QA
credential, so the real account cannot leak into a test either.

A module the Mod Browser lists (the mock's catalog index names its id) has every
release scanned on VirusTotal - the mock's - before anything is created on GitHub:
phase V. Phase U opens the Releases tab and the Publish Release dialog over MCP, looks at
them and closes them - nothing in them is clicked. The run works on a trimmed copy of the
project in the system temp folder (no rawdata, design, docs or textures: no phase is about a
package's size), the source is never touched; the QA VirusTotal key is forgotten. The editor
is closed the way its caption cross would.

    python tools/qa/test_mod_publish.py --project <project> [--only A,R,V,...]

Each phase stands on its own setup and mock (B publishes the release A made when A was not
asked for, U lays out R's releases).
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
import time
import urllib.request

import qa
from qa import check, cmd, sha256



TOKEN = "qa_token_0123456789abcdefghij"
GITHUB_REPO = "QaOwner/qa-mod"
VT_KEY = "qa-vt-key-publish-7c1d"
TRUSTED = "EngineT"
# where the mock's catalog index comes from: seeds written by main()
SEEDS: dict[str, str] = {}

class Mock(qa.Mock):
    def __init__(self, port: int, *flags: str, token: str = TOKEN, catalog: str = "unlisted"):
        """`catalog` names the seed of the catalog's public files: unlisted (an index without this module),
        listed (with it) or none (no index at all)."""
        super().__init__(port, *self.options(*flags, token=token, catalog=catalog))

    @staticmethod
    def options(*flags: str, token: str = TOKEN, catalog: str = "unlisted") -> list[str]:
        return ["--token", token, "--repo", GITHUB_REPO, "--vt-key", VT_KEY, "--catalog-seed", SEEDS[catalog], *flags]

    def restart(self, *flags: str, token: str = TOKEN, catalog: str = "unlisted") -> None:
        """Other switches and a fresh state on the same port, as a new process would have (POST /__restart)."""
        body = json.dumps(["--port", str(self.port), *self.options(*flags, token=token, catalog=catalog)]).encode()
        if (self.post("/__restart", body, "application/json") or {}).get("port") != self.port:
            raise RuntimeError("the mock did not restart on its port")

    def vt(self) -> dict:
        return self.get("/__vt")


class Editor(qa.Editor):
    """The suite's editor, started under -qa_github with every service on the mock's port."""

    def __init__(self, project: str, port: int, with_token: bool):
        env: dict = {name: None for name in ("GRIZZLY_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN", "GRIZZLY_VT_KEY",
                                              "VT_API_KEY")}
        env["GRIZZLY_GITHUB_API"] = f"http://127.0.0.1:{port}"
        env["GRIZZLY_GITHUB_WEB"] = f"http://127.0.0.1:{port}"
        env["GRIZZLY_GITHUB_CLIENT_ID"] = "qa-client"
        # the Mod Browser's index and VirusTotal are the mock's too; the key is the QA credential (vt_key)
        env["GRIZZLY_CATALOG_BASE"] = f"http://127.0.0.1:{port}/catalog/public/"
        env["GRIZZLY_VT_API"] = f"http://127.0.0.1:{port}/vt/api/v3"
        env["GRIZZLY_VT_PACE_MS"] = "100"
        env["GRIZZLY_VT_SCAN_WAIT_MS"] = "1500"
        env["GRIZZLY_VT_RATE_WAIT_MS"] = "200"
        # what the game is served is looked at again, and a broken upload tried again, after a pause: seconds for
        # the author, a moment here
        env["GRIZZLY_GITHUB_PAUSE_MS"] = "300"
        # the author key the releases are signed with: a throw-away home, never the user's own - gone with the editor
        self.catalog_home = tempfile.mkdtemp(prefix="grizzly_publish_home_")
        env["GRIZZLY_CATALOG_HOME"] = self.catalog_home
        if with_token:
            env["GRIZZLY_GITHUB_TOKEN"] = TOKEN
        super().__init__(project, github=True, env=env)
        self.start()

    def close(self, **kw):
        code = super().close(**kw)
        qa.remove_tree(self.catalog_home)
        return code


def set_release(version: str) -> dict:
    return cmd("mod.set", set={"module": {"version": version}, "update": {"github": GITHUB_REPO}})


def publish(version: str, wait_seconds: float = 180.0, **extra) -> dict:
    """mod.set the version, then release.publish, which answers once the release is out or refused."""
    set_result = set_release(version)
    if not set_result.get("ok"):
        return set_result
    return cmd("release.publish", timeout=wait_seconds + 30, **extra)


def releases(**extra) -> dict:
    """release.list, release.edit or release.delete by `action` (list by default); each answers when GitHub did."""
    action = extra.pop("action", "list")
    name = {"list": "release.list", "status": "release.list", "edit": "release.edit", "delete": "release.delete"}[action]
    return cmd(name, **extra)


def ui_phase(mock: "Mock") -> None:
    """U: the Releases tab (a document of its own) and the Publish Release dialog, opened, looked at and closed over
    MCP. Nothing in them is clicked: a look never publishes, edits or deletes anything."""
    window = str((cmd("ui.contract").get("dialogs") or {}).get("publish_release") or "Publish Release###dlg.publish_release")
    # the dialog on top is answered by its short id
    dialog = (window.partition("###")[2] or window).removeprefix("dlg.")
    before = [(r["id"], r["name"], r["draft"]) for r in mock.state()["releases"]]

    opened = cmd("ui.tab", action="open", kind="releases")
    check(opened.get("ok") and (opened.get("tab") or {}).get("id") == "releases",
          "U the Releases tab opens as a document of its own, the active one", str(opened))
    data = releases()
    check(data.get("ok") and data.get("listed") and data.get("tab") is True,
          "U the tab reads the list, and release.list says the tab is up", str(data)[:300])
    latest = next((r for r in data.get("releases", []) if r.get("latest")), {})
    picked = cmd("release.view", select=latest.get("tag", ""))
    check(picked.get("ok") and picked.get("selected") == latest.get("id"),
          "U select marks the release the details show, and asks GitHub nothing", str(picked)[:300])
    refused = cmd("release.view", select="v7.7.7")
    check(not refused.get("ok") and "no release carries the tag" in str(refused.get("error")),
          "U selecting a tag the list does not hold is refused", str(refused))
    pic = cmd("shot.window")
    check(pic.get("active_tab") == "releases" and not pic.get("modal"), "U the picture shows the Releases tab, no dialog",
          str({k: pic.get(k) for k in ("active_tab", "modal")}))
    # the active tab's work area, whatever the contract's lists say of "releases"
    audit = cmd("ui.audit", id="work_area")
    controls = {c.get("label"): c for c in audit.get("controls") or []}
    check(audit.get("ok") and not audit.get("escaped") and (controls.get("Edit...") or {}).get("enabled") is True and
          (controls.get("Delete...") or {}).get("enabled") is True,
          "U the details offer Edit... and Delete... for the selected release, and nothing leaves the tab",
          str(audit.get("escaped")) + " " + str(sorted(controls)))

    # the edit dialog, put up to be looked at: only a click in it would save anything
    shown = cmd("release.view", select=latest.get("tag", ""), edit=True)
    pic = cmd("shot.window")
    check(shown.get("ok") and (pic.get("modal") or {}).get("id") == "dlg.edit_release",
          "U release.view select= edit=true puts the release's edit dialog up", str(pic.get("modal")))
    audit = cmd("ui.audit", id="dlg.edit_release")
    labels = [c.get("label") for c in audit.get("controls") or []]
    check("Save" in labels and "Cancel" in labels and not audit.get("escaped"),
          "U the edit dialog offers Save and Cancel, and nothing leaves it", str(audit.get("escaped")) + " " + str(labels))
    cmd("ui.dialog", action="close", id="dlg.edit_release")
    check(not cmd("shot.window").get("modal"), "U ui.dialog close takes the edit dialog down")

    # The manifest still names 0.9.11, which is out: the dialog reads the releases as it opens and lists that before
    # the click, where GitHub would refuse it only after the packaging.
    opened = cmd("ui.dialog", action="open", id="publish_release")
    listed = releases()
    pic = cmd("shot.window")
    modal = pic.get("modal") or {}
    check(opened.get("ok") and modal.get("id") == dialog, "U Publish Release opens as the dialog " + dialog, str(modal))
    check(listed.get("ok") and listed.get("listed"), "U the dialog reads the releases as it opens",
          str(listed)[:300])
    audit = cmd("ui.audit", id="publish_release")
    controls = {c.get("label"): c for c in audit.get("controls") or []}
    publish_button = controls.get("Publish") or {}
    check(audit.get("ok") and publish_button.get("enabled") is False and "already published" in str(publish_button.get("why")) and
          "v0.9.11" in str(publish_button.get("why")), "U a version that is out is listed, and Publish stays off with it as the reason",
          str(publish_button))
    check("Open Mod Settings" in controls and "Open Releases" in controls,
          "U the problem names where it is fixed: Open Mod Settings, Open Releases", str(sorted(controls)))
    raised = set_release("0.9.99")
    cmd("shot.window")
    audit = cmd("ui.audit", id="publish_release")
    controls = {c.get("label"): c for c in audit.get("controls") or []}
    check(raised.get("ok") and audit.get("ok") and not audit.get("escaped") and (controls.get("Publish") or {}).get("enabled") is True and
          "Cancel" in controls and "Open Releases" not in controls,
          "U a version that is not out: its footer holds Publish, ready, and Cancel; nothing leaves the dialog",
          str(audit.get("escaped")) + " " + str(sorted(controls)))
    closed = cmd("ui.dialog", action="close", id="publish_release")
    pic = cmd("shot.window")
    check(closed.get("ok") and not pic.get("modal"), "U ui.dialog close takes it down, as Esc would", str(pic.get("modal")))

    closed = cmd("ui.tab", action="close", doc="releases")
    data = cmd("release.view")
    check(closed.get("ok") and data.get("releases_tab") is False, "U the tab closes; release.view says so", str(data))
    after = [(r["id"], r["name"], r["draft"]) for r in mock.state()["releases"]]
    check(after == before, "U looking changed nothing on GitHub", f"{before} -> {after}")


def mock_api(mock: "Mock", method: str, path: str, body: dict | None = None):
    """The harness talking to the mock as the repository owner would, past the editor."""
    request = urllib.request.Request(mock.base + path, method=method, data=json.dumps(body).encode() if body else None,
                                     headers={"Authorization": "Bearer " + TOKEN, "User-Agent": "qa-harness",
                                              "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=10) as r:
        raw = r.read()
    return json.loads(raw.decode("utf-8")) if raw else None


def published(state: dict) -> list[dict]:
    return [r for r in state["releases"] if not r["draft"]]


def drafts(state: dict) -> list[dict]:
    return [r for r in state["releases"] if r["draft"]]


def web_requests(state: dict) -> list[dict]:
    return [r for r in state["requests"] if "/releases/latest/download/" in r["path"] or r["path"].startswith("/cdn/")]


def vt_requests(state: dict) -> list[dict]:
    return [r for r in state["requests"] if r["path"].startswith("/vt/")]


def github_requests(state: dict) -> list[dict]:
    """What the job asked GitHub's API, the catalog's public files and VirusTotal left out."""
    return [r for r in state["requests"] if r["path"].startswith(("/repos/", "/upload/", "/user"))]


def write_seed(folder: str, module_id: str | None, withdrawn: bool = False) -> str:
    """A catalog whose public/index.ltx lists another module and, with `module_id`, this one - under [withdrawn]
    when `withdrawn`; None = no index."""
    os.makedirs(folder, exist_ok=True)
    if module_id is None:
        return folder
    rows = ["qa-other-module = QaSomeone/other-module, p256:qa-other, 1.0.0, 2026-09-25"]
    own = ["%s = %s, p256:qa-author, 1.0.0, 2026-09-25" % (module_id, GITHUB_REPO)] if module_id else []
    text = "\n".join(["[catalog]", "format  = 1", "serial  = 1", "issued  = 2026-09-25", "",
                      "[virustotal]", "trusted       = " + TRUSTED, "block_trusted = 1", "warn_trusted  = 1",
                      "block_others  = 5", "warn_others   = 2", "", "[mods]", *rows, *([] if withdrawn else own), "",
                      "[withdrawn]", *(own if withdrawn else []), "", "[revoked]", ""]) + "\n"
    os.makedirs(os.path.join(folder, "public"), exist_ok=True)
    with open(os.path.join(folder, "public", "index.ltx"), "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    return folder


def vt_forget_key() -> None:
    """The QA VirusTotal credential goes with the run - and with a run before it that stopped halfway."""
    try:
        cmd("release.account", action="forget_vt_key")
    except (OSError, ValueError):
        pass


PHASES = "ABCDEFGHIJRUVKLNM"
# what each phase needs to have run before it (its setup, run without its checks when it was not asked for)
NEEDS = {"B": "A", "U": "R"}


class Run:
    def __init__(self, project: str, port: int, work: str) -> None:
        self.project, self.port, self.work = project, port, work
        self.mock: Mock | None = None
        self.editor: Editor | None = None
        self.token_editor = True
        self.releases_root = os.path.join(project, "releases")
        self.manifest_path = os.path.join(project, "mod.ltx")
        self.original = open(self.manifest_path, "rb").read()
        self.module_id = ""
        self.module_name = ""
        self.descriptor_name = ""
        # a phase run for another's sake runs without its checks
        self.quiet = False

    # -- the mock and the editor
    def new_mock(self, *flags: str, **kw) -> Mock:
        # one mock process for the run: a phase's switches are a restart of it, not a process start
        if self.mock and self.mock.proc.poll() is None:
            self.mock.restart(*flags, **kw)
        else:
            self.mock = Mock(self.port, *flags, **kw)
        return self.mock

    def ensure_editor(self, with_token: bool) -> None:
        if self.editor and self.token_editor == with_token:
            return
        if self.editor:
            vt_forget_key()
            self.editor.close()
            self.editor = None
        if not self.mock:
            self.new_mock()
        self.editor = Editor(self.project, self.port, with_token)
        self.token_editor = with_token
        vt_forget_key()
        if not self.module_id:
            manifest = cmd("mod.info").get("manifest", {})
            self.module_id = manifest["id"]
            # a release is titled "<module name> <version>" - whatever project the run is given
            self.module_name = manifest.get("name") or self.module_id
            self.descriptor_name = self.module_id + ".update.ltx"
            SEEDS["listed"] = write_seed(os.path.join(self.work, "listed"), self.module_id)
            SEEDS["withdrawn"] = write_seed(os.path.join(self.work, "withdrawn"), self.module_id, withdrawn=True)

    def close(self) -> None:
        if self.editor:
            vt_forget_key()
            self.editor.close()
            self.editor = None
        if self.mock:
            self.mock.stop()
            self.mock = None

    def check(self, ok: bool, text: str, detail: str = "") -> bool:
        if self.quiet:
            return bool(ok)
        return check(ok, text, detail)

    # -- phase 1: the account comes from the environment
    def a(self) -> None:
        c = self.check
        self.new_mock()
        self.ensure_editor(True)
        account = cmd("release.account")
        c(account.get("signed_in") and account.get("source") == "environment variable", "A account: the environment token is in effect", str(account))
        c(account.get("device_sign_in") is True, "A account: the device sign-in is on offer in QA mode", str(account))
        data = publish("0.9.1", notes="First line.\nSecond line with \"quotes\" and a \\ backslash.", title="QA release 0.9.1")
        c(data.get("state") == "done", "A publish 0.9.1 ends in 'done'", str(data))
        c(data.get("served") is True, "A the address the game asks serves the new descriptor", str(data))
        c(str(data.get("url", "")).endswith("/releases/tag/v0.9.1"), "A the answer carries the release page", str(data.get("url")))
        state = self.mock.state()
        done = published(state)
        c(len(done) == 1 and not drafts(state) and state["latest_id"] == done[0]["id"], "A one published release, no draft, marked latest")
        release = done[0] if done else {"assets": [], "body": "", "name": ""}
        c(release.get("name") == "QA release 0.9.1" and "Second line with \"quotes\" and a \\ backslash." in release.get("body", ""),
          "A title and notes arrive unharmed", release.get("body", ""))
        folder = os.path.join(self.releases_root, "0.9.1")
        local = {name: sha256(os.path.join(folder, name)) for name in os.listdir(folder)} if os.path.isdir(folder) else {}
        remote = {a["name"]: a["sha256"] for a in release["assets"]}
        c(local and local == remote, "A GitHub holds exactly the packaged files, byte for byte", f"{local} vs {remote}")
        mid = self.module_id
        c(self.descriptor_name in remote and f"{mid}-0.9.1.files" in remote and f"{mid}-0.9.1.zip" in remote,
          "A descriptor, index and package carry the names the game expects", str(sorted(remote)))
        order = [r["path"] for r in state["requests"] if r["method"] in ("POST", "PATCH")]
        create_at = next((i for i, p in enumerate(order) if p.endswith("/releases")), -1)
        publish_at = max((i for i, p in enumerate(order) if "/releases/" in p and "/assets" not in p and not p.startswith("/upload/")), default=-1)
        uploads_at = [i for i, p in enumerate(order) if p.startswith("/upload/")]
        c(create_at >= 0 and uploads_at and create_at < min(uploads_at) and max(uploads_at) < publish_at,
          "A draft first, uploads next, publication last", str(order))
        web = web_requests(state)
        c(web and all(not r["authorized"] for r in web), "A the token never travels to the download address", str(web))

    def b(self) -> None:
        self.ensure_editor(True)
        data = publish("0.9.1")
        check(not data.get("ok") and "already published" in str(data.get("error")), "B the same version again is refused", str(data))
        check(len(self.mock.state()["releases"]) == 1, "B and nothing was created for it")

    def c_(self) -> None:
        self.new_mock("--drop-upload", "1")
        self.ensure_editor(True)
        data = publish("0.9.2")
        state = self.mock.state()
        check(data.get("state") == "done" and state["uploads"] == 4, "C a connection lost mid-upload is retried", f"{data} uploads={state.get('uploads')}")
        check(len(published(state)) == 1 and all(a["state"] == "uploaded" for a in published(state)[0]["assets"]) and
              len(published(state)[0]["assets"]) == 3, "C the stub the broken upload left is gone", str(state["releases"]))

    def d(self) -> None:
        self.new_mock("--rename-assets")
        self.ensure_editor(True)
        data = publish("0.9.3")
        state = self.mock.state()
        check(not data.get("ok") and "as it was packaged" in str(data.get("error")), "D an asset GitHub renamed stops the release", str(data))
        check(not published(state) and state["latest_id"] is None, "D and nothing is published")

    def e(self) -> None:
        self.new_mock("--corrupt-digest", ".update.ltx")
        self.ensure_editor(True)
        data = publish("0.9.3")
        state = self.mock.state()
        check(not data.get("ok") and "SHA-256" in str(data.get("error")), "E a digest that differs stops the release", str(data))
        check(not published(state), "E and nothing is published")

    def f(self) -> None:
        seed = os.path.join(self.work, "_seed_foreign.json")
        with open(seed, "w", encoding="utf-8") as fh:
            json.dump([{"tag_name": "v3.0.0", "name": "Other", "draft": False, "prerelease": False, "latest": True,
                        "assets": {"other.mod.update.ltx": "[release]\n"}}], fh)
        self.new_mock("--seed", seed)
        self.ensure_editor(True)
        data = publish("0.9.3")
        state = self.mock.state()
        check(not data.get("ok") and "another module" in str(data.get("error")), "F a repository that serves another module is refused", str(data))
        check(len(state["releases"]) == 1, "F and no draft was created")
        data = publish("0.9.3", prerelease=True)
        state = self.mock.state()
        check(data.get("state") == "done" and state["latest_id"] == [r for r in state["releases"] if r["tag_name"] == "v3.0.0"][0]["id"],
              "F a pre-release is allowed there and leaves the latest release alone", str(data))

    def g(self) -> None:
        self.new_mock("--private")
        self.ensure_editor(True)
        data = publish("0.9.4")
        check(data.get("state") == "done" and data.get("served") is False and "private" in str(data.get("warnings")),
              "G a private repository publishes with a warning", str(data))

    def h(self) -> None:
        self.new_mock("--stale-latest", "2")
        self.ensure_editor(True)
        data = publish("0.9.5")
        check(data.get("state") == "done" and data.get("served") is True, "H a lagging download address is asked again", str(data))

    def i(self) -> None:
        self.new_mock()
        self.ensure_editor(True)
        # the upload has to outlast the second before the cancel, and a package the socket's buffers swallow whole
        # never would: the trimmed copy gets a few megabytes that do not pack. The throttle is chosen from the package
        # so that it takes about a second and a half: the cancel comes 0.4 s into it
        weight_file = os.path.join(self.project, "gamedata", "textures", "qa_publish_weight.dds")
        os.makedirs(os.path.dirname(weight_file), exist_ok=True)
        with open(weight_file, "wb") as fh:
            fh.write(os.urandom(6 << 20))
        set_release("0.9.6")
        packaged = cmd("mod.package")
        weight = sum(p.get("size", 0) for p in packaged.get("packages", [])) + (packaged.get("index") or {}).get("size", 0)
        kbps = max(2, weight * 8 * 2 // 1000 // 3)
        self.new_mock("--throttle-kbps", str(kbps))
        started = cmd("release.publish", wait=False)
        check(started.get("ok") and started.get("job"), "I wait=false answers the job at once", str(started))
        # the packaging, then the upload: the second job is the one to stop
        upload = None
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline and upload is None:
            running = [j for j in cmd("job.list", running=True).get("jobs", []) if j.get("group") == "publish"]
            upload = running[0] if running else None
            time.sleep(0.05)
        time.sleep(0.4)
        stopped = cmd("job.stop", job=(upload or {}).get("id", 0))
        ended = cmd("job.wait", job=(upload or {}).get("id", 0), timeout=90)
        state = self.mock.state()
        check(stopped.get("ok") and ended.get("state") == "cancelled", "I job.stop ends the upload as 'cancelled'",
              json.dumps(ended)[:300])
        check(len(drafts(state)) == 1 and not published(state), "I nothing is published and the draft stays")
        # the throttled mock drains what the socket had buffered before it meets the closed connection
        stubs = []
        deadline = time.monotonic() + 40
        while not stubs and time.monotonic() < deadline:
            state = self.mock.state()
            stubs = [a["name"] for a in drafts(state)[0]["assets"] if a["state"] != "uploaded"] if drafts(state) else []
            if not stubs:
                time.sleep(0.1)
        check(bool(stubs), "I the abandoned upload left its stub in the draft, as GitHub does", str(state["releases"]))
        data = publish("0.9.6", wait_seconds=240)
        state = self.mock.state()
        check(data.get("state") == "done" and len(state["releases"]) == 1 and len(published(state)) == 1,
              "I the next attempt reuses that draft", f"{data} releases={len(state['releases'])}")
        assets = published(state)[0]["assets"] if published(state) else []
        check(len(assets) == 3 and all(a["state"] == "uploaded" for a in assets),
              "I the stub is cleared away and exactly the three assets remain", str(assets))
        os.remove(weight_file)

    def j(self) -> None:
        self.new_mock(token="another_token_0123456789abcdef")
        self.ensure_editor(True)
        data = publish("0.9.7")
        check(not data.get("ok") and "rejected the sign-in" in str(data.get("error")), "J a token GitHub does not accept is reported as such", str(data))
        self.new_mock("--archived")
        data = publish("0.9.7")
        check(not data.get("ok") and "archived" in str(data.get("error")), "J an archived repository is refused", str(data))
        self.new_mock("--no-push")
        data = publish("0.9.7")
        check(not data.get("ok") and "cannot write" in str(data.get("error")), "J a repository without push access is refused", str(data))
        self.new_mock("--empty")
        data = publish("0.9.7")
        check(not data.get("ok") and "empty" in str(data.get("error")).lower(), "J an empty repository: GitHub's own words reach the author", str(data))

    # -- the releases that are out: list, edit, delete with the tag
    def r_list(self) -> dict:
        """R's first half: two releases and a draft, listed and edited. Answers the listing."""
        c = self.check
        # --latest-on-edit: an edit that leaves make_latest to GitHub's default moves the mark
        self.new_mock("--latest-on-edit")
        self.ensure_editor(True)
        for version in ("0.9.10", "0.9.11"):
            data = publish(version)
            c(data.get("state") == "done", f"R publish {version} for the list", str(data))
        mock_api(self.mock, "POST", f"/repos/{GITHUB_REPO}/releases",
                 {"tag_name": "v0.9.12", "name": "Left behind", "draft": True, "prerelease": False})
        data = releases()
        listed = {r["tag"]: r for r in data.get("releases", [])}
        c(data.get("ok") and data.get("listed") and sorted(listed) == ["v0.9.10", "v0.9.11", "v0.9.12"],
          "R the list holds both releases and the draft", str(data))
        c([r["tag"] for r in data.get("releases", [])] == ["v0.9.12", "v0.9.11", "v0.9.10"], "R newest first")
        c(listed.get("v0.9.11", {}).get("latest") is True and not listed.get("v0.9.10", {}).get("latest"),
          "R the latest release is marked", str(listed.get("v0.9.11")))
        c(listed.get("v0.9.12", {}).get("draft") is True and listed.get("v0.9.12", {}).get("published") == "",
          "R the draft is marked and has no publication time", str(listed.get("v0.9.12")))
        c(len(listed.get("v0.9.11", {}).get("assets", [])) == 3 and listed.get("v0.9.11", {}).get("bytes", 0) > 0 and
          len(listed.get("v0.9.11", {}).get("published", "")) == 16, "R files, size and time of a release", str(listed.get("v0.9.11")))
        notes = "Line one.\nLine \"two\" with a \\ backslash."
        data = releases(action="edit", tag="v0.9.10", title="Renamed 0.9.10", notes=notes)
        state = self.mock.state()
        edited = next((r for r in state["releases"] if r["tag_name"] == "v0.9.10"), {})
        newest = next((r for r in state["releases"] if r["tag_name"] == "v0.9.11"), {})
        c(data.get("ok") and "v0.9.10 is updated" in str(data.get("note")), "R editing a release by tag", str(data))
        c(edited.get("name") == "Renamed 0.9.10" and edited.get("body") == notes, "R title and notes arrive unharmed", str(edited))
        c(state["latest_id"] == newest.get("id"), "R editing an older release leaves the latest mark where it was",
          f"latest {state['latest_id']}, newest {newest.get('id')}")
        c(len(edited.get("assets", [])) == 3 and not edited.get("draft"), "R its files and its state are untouched")
        answered = {r["tag"]: r for r in data.get("releases", [])}
        c(answered.get("v0.9.10", {}).get("notes") == notes and answered.get("v0.9.10", {}).get("title") == "Renamed 0.9.10",
          "R the refreshed list carries the new text", str(answered.get("v0.9.10")))
        data = releases(action="edit", tag="v0.9.11", notes="Only the notes.")
        state = self.mock.state()
        newest = next((r for r in state["releases"] if r["tag_name"] == "v0.9.11"), {})
        c(data.get("ok") and newest.get("body") == "Only the notes." and newest.get("name") == self.module_name + " 0.9.11" and
          state["latest_id"] == newest.get("id"), "R a field that is left out keeps its value; the latest stays the latest", str(newest))
        draft_id = listed.get("v0.9.12", {}).get("id", 0)
        data = releases(action="edit", id=draft_id, title="Left behind, renamed")
        state = self.mock.state()
        draft = next((r for r in state["releases"] if r["id"] == draft_id), {})
        c(data.get("ok") and draft.get("name") == "Left behind, renamed" and draft.get("draft") is True,
          "R a draft is edited by id and stays a draft", str(draft))
        refused = cmd("release.edit", tag="v0.9.10")
        c(not refused.get("ok") and "title or notes" in str(refused.get("error")), "R an edit that changes nothing is refused", str(refused))
        refused = releases(action="edit", tag="v7.7.7", title="x")
        c(not refused.get("ok") and "no release carries the tag" in str(refused.get("error")), "R an edit of an unknown tag is refused", str(refused))
        return listed

    def r(self) -> None:
        self.r_list()
        if "U" in self.only:
            ui_phase(self.mock)
        self.r_delete()

    def u(self) -> None:
        # U alone: the releases R lists and edits, laid out first
        if "R" not in self.only:
            self.quiet = True
            try:
                self.r_list()
            finally:
                self.quiet = False
            ui_phase(self.mock)

    def r_delete(self) -> None:
        mock = self.mock
        data = releases(action="delete", tag="v0.9.11")
        state = mock.state()
        check(data.get("ok") and "and its tag are deleted" in str(data.get("note")) and data.get("tag_kept") is False,
              "R deleting by tag reports the release and the tag gone", str(data))
        check(sorted(r["tag_name"] for r in state["releases"]) == ["v0.9.10", "v0.9.12"] and state["tags"] == ["v0.9.10"],
              "R GitHub holds neither the release nor its tag", f"{[r['tag_name'] for r in state['releases']]} {state['tags']}")
        check(not [r for r in state["releases"] if r["tag_name"] == "v0.9.10" and r["draft"]],
              "R the release that stays is still published")
        deletes = [r["path"] for r in state["requests"] if r["method"] == "DELETE"]
        check(len(deletes) == 2 and "/releases/" in deletes[0] and deletes[1].endswith("/git/refs/tags/v0.9.11"),
              "R the release goes first, the tag second", str(deletes))
        after = {r["tag"]: r for r in data.get("releases", [])}
        check(sorted(after) == ["v0.9.10", "v0.9.12"] and after["v0.9.10"]["latest"] is True,
              "R the answer carries the refreshed list, latest moved back", str(data.get("releases")))
        with urllib.request.urlopen(f"{mock.base}/{GITHUB_REPO}/releases/latest/download/{self.descriptor_name}", timeout=10) as r:
            served = r.read().decode("utf-8")
        check("0.9.10" in served and "0.9.11" not in served, "R the game is served the previous release again", served[:200])
        draft_id = after.get("v0.9.12", {}).get("id", 0)
        data = releases(action="delete", id=draft_id)
        state = mock.state()
        check(data.get("ok") and "Draft release" in str(data.get("note")), "R deleting a draft by id", str(data))
        check([r["tag_name"] for r in state["releases"]] == ["v0.9.10"] and state["tags"] == ["v0.9.10"],
              "R the draft is gone and no tag was touched for it", f"{state['tags']}")
        data = publish("0.9.11")
        check(data.get("state") == "done", "R the freed version can be published again", str(data))
        # A tag that is used again keeps its download address, and GitHub serves the deleted files
        # from it for minutes - so the editor never offers a tag twice.
        check(data.get("tag") == "v0.9.11-r2" and "v0.9.11-r2" in mock.state()["tags"],
              "R under a fresh tag, so that its files get a fresh address", f"{data.get('tag')} {mock.state()['tags']}")
        data = publish("0.9.11")
        check(not data.get("ok") and "already published" in str(data.get("error")) and "v0.9.11-r2" in str(data.get("error")),
              "R a fresh tag is no way around a version that is out", str(data))
        data = releases(action="delete", tag="v7.7.7")
        check(not data.get("ok") and "no release carries the tag" in str(data.get("error")), "R an unknown tag is refused", str(data))
        data = cmd("release.delete")
        check(not data.get("ok") and "id or tag" in str(data.get("error")), "R delete without a target is refused", str(data))
        check(len(mock.state()["releases"]) == 2, "R and nothing else was deleted")
        mock = self.new_mock("--protect-tags")
        data = publish("0.9.13")
        check(data.get("state") == "done", "R publish 0.9.13 under a protected tag", str(data))
        data = releases(action="delete", tag="v0.9.13")
        state = mock.state()
        check(data.get("ok") and data.get("tag_kept") is True and "its tag is not" in str(data.get("note")) and
              "protected" in str(data.get("note")), "R a tag GitHub refuses to delete is reported, in GitHub's words", str(data))
        check(not state["releases"] and state["tags"] == ["v0.9.13"], "R the release is gone, the tag stayed")

    # -- V: a module the Mod Browser lists has every release scanned on VirusTotal first
    def v(self) -> None:
        self.ensure_editor(True)
        mock = self.new_mock("--vt-hide-new", catalog="listed")
        stored = cmd("release.account", action="save_vt_key", key=VT_KEY)
        check(stored.get("ok") and stored.get("vt_key") is True and VT_KEY not in json.dumps(stored),
              "V the author's VirusTotal key is stored, and not echoed", str(stored))
        data = publish("0.9.30")
        state = mock.state()
        vt = data.get("virustotal") or {}
        package = (vt.get("packages") or [{}])[0]
        zip_path = os.path.join(self.releases_root, "0.9.30", f"{self.module_id}-0.9.30.zip")
        uploads = (mock.vt()["files"].get(package.get("sha256", "")) or {}).get("uploads", 0)
        check(data.get("state") == "done" and vt.get("state") == "clean" and package.get("state") == "clean" and
              len(published(state)) == 1, "V listed: the package is scanned and the release goes out", str(data))
        check(package.get("name") == f"{self.module_id}-0.9.30.zip" and os.path.isfile(zip_path) and
              package.get("sha256") == sha256(zip_path) and uploads == 1,
              "V the packaged zip itself went to VirusTotal once, and its report was waited for", f"{package} uploads={uploads}")
        vt_at = [i for i, r in enumerate(state["requests"]) if r["path"].startswith("/vt/")]
        writes = [i for i, r in enumerate(state["requests"]) if r["method"] != "GET" and not r["path"].startswith("/vt/")]
        check(vt_at and writes and max(vt_at) < min(writes), "V the scan ran before anything was created on GitHub",
              str([(r["method"], r["path"]) for r in state["requests"]][:12]))
        check(all(r["vt_key"] for r in vt_requests(state)) and not any(r["vt_key"] for r in github_requests(state)) and
              not any(r["authorized"] for r in vt_requests(state)), "V the key went to VirusTotal only, the token never did")

        vt_forget_key()
        mock = self.new_mock(catalog="listed")
        data = publish("0.9.31")
        state = mock.state()
        error = str(data.get("error"))
        check(not data.get("ok") and data.get("state") == "failed" and "is in the Mod Browser" in error and
              "own free key" in error and "vt_key" in error, "V listed without a key: refused, saying why and where the key goes",
              str(data))
        check(not github_requests(state) and not state["releases"] and not state["tags"],
              "V and nothing reached GitHub: no draft, no tag, no request", str(github_requests(state)))
        check(not vt_requests(state) and (data.get("virustotal") or {}).get("state") == "no_key",
              "V and VirusTotal was not asked", str(vt_requests(state)))

        cmd("release.account", action="save_vt_key", key=VT_KEY)
        mock = self.new_mock("--vt-upload-results", json.dumps({TRUSTED: "malicious", "EngineA": "undetected"}), catalog="listed")
        data = publish("0.9.32")
        state = mock.state()
        error = str(data.get("error"))
        check(not data.get("ok") and "VirusTotal blocks" in error and TRUSTED in error and "withdraw" in error and
              (data.get("virustotal") or {}).get("state") == "block",
              "V listed, a package VirusTotal blocks: refused, the engine named", str(data))
        check(not github_requests(state) and not state["releases"] and not state["tags"],
              "V and nothing reached GitHub", str(github_requests(state)))

        mock = self.new_mock("--vt-queue", "1000", catalog="listed")
        data = publish("0.9.33")
        state = mock.state()
        error = str(data.get("error"))
        check(not data.get("ok") and data.get("state") == "failed" and
              (data.get("virustotal") or {}).get("state") == "analysing" and "still analysing" in error and
              "only with a VirusTotal report of every package" in error and "publish again in a few minutes" in error,
              "V still analysing after the wait: refused - a listed module's release leaves only with a report", str(data))
        check(not github_requests(state) and not state["releases"] and not state["tags"],
              "V and nothing reached GitHub", str(github_requests(state)))

        mock = self.new_mock(catalog="withdrawn")
        data = publish("0.9.33")
        state = mock.state()
        warnings = str(data.get("warnings"))
        check(data.get("state") == "done" and len(published(state)) == 1 and not vt_requests(state) and
              (data.get("virustotal") or {}).get("state") == "none" and "withdrawn from the Mod Browser" in warnings and
              "lists it again" in warnings,
              "V withdrawn from the Mod Browser: published without a scan, with a warning its players there do not get it",
              str(data))

        mock = self.new_mock(catalog="none")
        data = publish("0.9.34")
        state = mock.state()
        check(data.get("state") == "done" and len(published(state)) == 1 and (data.get("virustotal") or {}).get("state") == "clean"
              and vt_requests(state), "V the catalog does not answer: with a key, the release is scanned all the same",
              str(data))
        vt_forget_key()
        mock = self.new_mock(catalog="none")
        data = publish("0.9.35")
        state = mock.state()
        check(data.get("state") == "done" and len(published(state)) == 1 and not vt_requests(state) and
              "did not answer" in str(data.get("warnings")) and "once VirusTotal knows" in str(data.get("warnings")),
              "V the catalog does not answer and there is no key: published, with a note", str(data))

        cmd("release.account", action="save_vt_key", key=VT_KEY)
        mock = self.new_mock()
        data = publish("0.9.36")
        state = mock.state()
        check(data.get("state") == "done" and len(published(state)) == 1 and not vt_requests(state) and
              (data.get("virustotal") or {}).get("state") == "none" and not data.get("warnings"),
              "V not listed: no VirusTotal request at all, published as ever", str(data))
        vt_forget_key()

    def k(self) -> None:
        self.ensure_editor(True)
        bad = cmd("release.publish", tag="bad tag")
        check(not bad.get("ok") and "invalid tag" in str(bad.get("error")), "K an invalid tag is refused", str(bad))
        open(self.manifest_path, "wb").write(self.original)
        if b"github" not in self.original:
            data = cmd("release.publish")
            check(not data.get("ok") and "no GitHub repository" in str(data.get("error")), "K a manifest without a repository is refused", str(data))

    # -- phase 2: no environment token - the device sign-in and the saved credential
    def l(self) -> None:
        self.new_mock("--device-approve-after", "2", "--device-interval", "1")
        self.ensure_editor(False)
        cmd("release.account", action="sign_out")
        account = cmd("release.account")
        check(account.get("signed_in") is False, "L without a token the editor is not signed in", str(account))
        data = publish("0.9.8")
        check(not data.get("ok") and "not signed in" in str(data.get("error")), "L and publishing says so before it packages anything", str(data))
        check(not os.path.isdir(os.path.join(self.releases_root, "0.9.8")), "L nothing was packaged for it")
        data = cmd("release.list")
        check(not data.get("ok") and "not signed in" in str(data.get("error")), "L the release list asks for a sign-in too", str(data))

    def n(self) -> None:
        self.new_mock("--device-approve-after", "2", "--device-interval", "1")
        self.ensure_editor(False)
        # the account over MCP alone: what a window's Save Token does, without the window
        account = cmd("release.account", action="save_token", token=TOKEN)
        check(account.get("signed_in") is True and account.get("source") == "token saved by the editor" and
              TOKEN not in json.dumps(account), "N save_token signs the editor in and echoes nothing", str(account))
        refused = cmd("release.account", action="save_token", token="not a token")
        check(not refused.get("ok") and "does not look like" in str(refused.get("error")), "N a string that is no token is refused", str(refused))
        refused = cmd("release.account", action="save_token")
        check(not refused.get("ok") and "needs token" in str(refused.get("error")), "N save_token without a token is refused", str(refused))
        data = publish("0.9.14")
        check(data.get("state") == "done" and data.get("served") is True, "N publishing works with the token saved over MCP", str(data))
        data = releases(action="delete", tag="v0.9.14")
        check(data.get("ok") and "and its tag are deleted" in str(data.get("note")), "N and so does deleting", str(data))
        account = cmd("release.account", action="sign_out")
        check(account.get("signed_in") is False, "N sign_out forgets the saved token", str(account))

    def m(self) -> None:
        # the first poll after the code is handed out finds it confirmed
        self.new_mock("--device-approve-after", "1", "--device-interval", "1")
        self.ensure_editor(False)
        cmd("release.account", action="sign_out")
        data = cmd("release.account", action="sign_in")
        check(data.get("ok") and data.get("user_code") == "WDJB-MJHT" and data.get("job"),
              "M the sign-in hands out the code for the person, and the job that waits for it", str(data))
        check(TOKEN not in json.dumps(data), "M and no token")
        done = cmd("job.wait", job=data.get("job", 0), timeout=90)
        check(done.get("state") == "done", "M the confirmation on GitHub completes it", str(done))
        account = cmd("release.account")
        check(account.get("source") == "token saved by the editor" and TOKEN not in json.dumps(account),
              "M the token sits in the credential store, not in an answer", str(account))
        data = publish("0.9.8")
        check(data.get("state") == "done" and data.get("served") is True, "M publishing works with the saved sign-in", str(data))
        sign_in_traffic = [r for r in self.mock.state()["requests"] if r["path"].startswith("/login/")]
        check(sign_in_traffic and all(not r["authorized"] for r in sign_in_traffic),
              "M the sign-in itself carries no token", str(sign_in_traffic))
        account = cmd("release.account", action="sign_out")
        check(account.get("signed_in") is False, "M sign_out forgets it", str(account))


def run(ctx: qa.Run) -> None:
    asked = [p for p in (ctx.only.replace(",", "").upper() or PHASES) if p in PHASES]
    only = [p for p in PHASES if p in asked]
    work = ctx.work
    SEEDS["unlisted"] = write_seed(os.path.join(work, "unlisted"), "")
    SEEDS["none"] = write_seed(os.path.join(work, "none"), None)
    run_ = Run(ctx.project, qa.free_port(), work)
    run_.only = only
    try:
        for p in only:
            t = time.monotonic()
            for need in NEEDS.get(p, ""):
                if need not in only:
                    run_.quiet = True
                    try:
                        getattr(run_, need.lower() + ("_" if need == "C" else ""))()
                    finally:
                        run_.quiet = False
            getattr(run_, p.lower() + ("_" if p == "C" else ""))()
            print("TIME phase %s %.1f s" % (p, time.monotonic() - t), flush=True)
    finally:
        run_.close()
        # a token is allowed into the editor, never into what the editor writes down
        import glob
        logs = os.path.join(qa.home(), "logs", "grizzly_*.log")
        leaked = [p for p in glob.glob(logs) if os.path.isfile(p) and TOKEN.encode() in open(p, "rb").read()]
        check(not leaked, "Z no log holds the token", str(leaked))


def main() -> int:
    return qa.suite("MOD PUBLISH", run, doc=__doc__, trimmed=True, phases=PHASES)


if __name__ == "__main__":
    sys.exit(main())
