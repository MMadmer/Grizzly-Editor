#!/usr/bin/env python3
"""The editor updates itself (docs/UPDATES.md, section 7), against tools/qa/github_mock.

A fake installation in the suite's temp folder - harmless text and binary blobs, no program, no
`MZ` head - is the editor's installation (`GRIZZLY_UPDATE_ROOT`, honoured only under
`-qa_github`); the releases are mock releases whose archives this suite packs the way
tools/rust/package.py does. A headless QA editor on its own port and home drives it over the
control socket; a last phase brings up an editor with a window on a project copy and looks at the updater's face.
Nothing is fetched from GitHub or published anywhere.

    python tools/qa/test_update.py --project <project> [--only A,B,...]

  A  the check reads the archive's end and its manifest, nothing else; the delta is what differs
  B  only the delta's ranges are requested; a connection cut inside a record resumes at its byte
  C  a file of the installation held open rolls the whole transaction back
  D  apply replaces, removes what the new version dropped and touches nothing unmanaged
  E  the first start of the new version says so and tidies up
  F  a record that does not match the manifest is refused and nothing is applied
  G  an editor restarted mid-download continues where the last one stopped
  H  the restart handshake: the new start reports and answers updated_from
  I  a 200 to a ranged request falls back to the whole archive
  J  the window: the chip in the status bar for each state (available, downloading, waiting, ready, failed,
     manual), the toast with its Details button, Help > Check for Updates..., the Update dialog with its buttons,
     the Updates group of Preferences > General - listed by ui.audit and ui.list, one picture each (--shots)

The phases run in order on one installation: each stands on the version the one before left (J has an
installation of its own, a version behind).
"""
from __future__ import annotations

import hashlib
import json
import os
import random
import sys
import time
import urllib.request
import zipfile

import qa
from qa import check, cmd

sys.path.insert(0, os.path.join(qa.REPO, "tools", "rust"))
import package  # noqa: E402

TOKEN = "qa_token_update_0123456789abcdef"
REPO_NAME = "QaOwner/Grizzly-Editor"
GAP = 256 * 1024

def blob(seed: int, size: int) -> bytes:
    data = random.Random(seed).randbytes(size)
    # never the head of a program
    return b"\0" + data[1:] if data[:2] == b"MZ" else data


def bump(version: str, n: int) -> str:
    major, minor, patch = (int(x) for x in version.split("."))
    return f"{major}.{minor}.{patch + n}"


# ---- the mock ----------------------------------------------------------------------------------


class Mock(qa.Mock):
    def __init__(self) -> None:
        super().__init__(0, "--token", TOKEN, "--repo", REPO_NAME)

    def call(self, method: str, url: str, body: bytes | None = None, kind: str = "application/json", token: bool = True):
        headers = {"User-Agent": "grizzly-qa", "Content-Type": kind}
        if token:
            headers["Authorization"] = "Bearer " + TOKEN
        rq = urllib.request.Request(url if url.startswith("http") else self.base + url, data=body, method=method,
                                    headers=headers)
        with urllib.request.urlopen(rq, timeout=30) as r:
            raw = r.read()
            return json.loads(raw.decode("utf-8")) if raw else {}

    def publish(self, version: str, archive: str, notes: str) -> None:
        """A release with the archive as its one asset, published as the latest."""
        rel = self.call("POST", f"/repos/{REPO_NAME}/releases",
                        json.dumps({"tag_name": version, "name": version, "body": notes, "draft": True}).encode())
        upload = rel["upload_url"].split("{", 1)[0] + "?name=" + os.path.basename(archive)
        with open(archive, "rb") as f:
            self.call("POST", upload, f.read(), "application/zip")
        self.call("PATCH", f"/repos/{REPO_NAME}/releases/{rel['id']}",
                  json.dumps({"draft": False, "make_latest": "true"}).encode())

    def harness(self, route: str, method: str = "POST") -> dict:
        return self.call(method, "/__asset/" + route, b"" if method == "POST" else None, token=False)

    def reads(self, version: str) -> list[dict]:
        stats = self.harness("stats", "GET")["assets"]
        return stats.get(f"Grizzly-Editor-{version}.zip", {}).get("reads", [])

    def requests(self) -> list[dict]:
        return self.call("GET", "/__state", token=False)["requests"]


# ---- the installation and its releases ---------------------------------------------------------


class Releases:
    """The versions: their files, archives and the layout of each archive."""

    def __init__(self, base: str, out: str) -> None:
        self.base, self.out = base, out
        self.files: dict[str, dict[str, bytes]] = {}
        self.archives: dict[str, str] = {}

    def make(self, version: str, files: dict[str, bytes], bad: str | None = None, schema: str | None = None) -> str:
        """Packs `files` as `version`; with `bad` the manifest names other bytes of the same size
        for that path; with `schema` the manifest says it is another schema."""
        self.files[version] = files
        folder = os.path.join(self.out, version)
        os.makedirs(folder, exist_ok=True)
        if bad is None and schema is None:
            archive, _ = package.pack(version, dict(files), folder)
        else:
            rows = []
            for name, data in files.items():
                claimed = bytes(b ^ 0xFF for b in data) if bad and name == bad else data
                rows.append((hashlib.sha256(claimed).hexdigest(), len(data), name))
            manifest = package.manifest_text(version, rows)
            if schema:
                manifest = manifest.replace(package.SCHEMA.encode(), schema.encode(), 1)
            archive = os.path.join(folder, f"Grizzly-Editor-{version}.zip")
            package.write_zip(archive, {**files, package.MANIFEST: manifest})
        self.archives[version] = archive
        return archive

    def spans(self, version: str) -> dict[str, tuple[int, int]]:
        """Each entry's record: its local header up to the next record or the directory."""
        with zipfile.ZipFile(self.archives[version]) as z:
            infos = sorted(z.infolist(), key=lambda i: i.header_offset)
            ends = [i.header_offset for i in infos[1:]] + [z.start_dir]
            return {i.filename: (i.header_offset, end) for i, end in zip(infos, ends)}


def merge(spans: list[tuple[int, int]]) -> list[tuple[int, int]]:
    out: list[list[int]] = []
    for a, b in sorted(spans):
        if out and a < out[-1][1] + GAP:
            out[-1][1] = max(out[-1][1], b)
        else:
            out.append([a, b])
    return [(a, b) for a, b in out]


def install(root: str, version: str, files: dict[str, bytes]) -> None:
    for name, data in files.items():
        path = os.path.join(root, *name.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
    rows = [(hashlib.sha256(d).hexdigest(), len(d), n) for n, d in files.items()]
    with open(os.path.join(root, package.MANIFEST), "wb") as f:
        f.write(package.manifest_text(version, rows))


def snapshot(root: str) -> dict[str, tuple[str, int]]:
    """Every file and folder outside the updater's work folder: content hash and write time."""
    out = {}
    for base, dirs, names in os.walk(root):
        dirs[:] = [d for d in dirs if not (base == root and d == ".update")]
        rel = os.path.relpath(base, root).replace("\\", "/")
        if rel != ".":
            out[rel + "/"] = ("", 0)
        for n in names:
            p = os.path.join(base, n)
            with open(p, "rb") as f:
                out[os.path.relpath(p, root).replace("\\", "/")] = (hashlib.sha256(f.read()).hexdigest(),
                                                                   os.stat(p).st_mtime_ns)
    return out


def read(root: str, name: str) -> bytes | None:
    try:
        with open(os.path.join(root, *name.split("/")), "rb") as f:
            return f.read()
    except OSError:
        return None


# ---- the editor --------------------------------------------------------------------------------


class Editor(qa.Editor):
    """The updater's editor: headless, under -qa_github, with the fake installation named by its environment."""

    def __init__(self, env: dict) -> None:
        super().__init__(github=True, headless=True, env=env, stalls=False)
        self.start()


def status() -> dict:
    return cmd("update.status")


# ---- the phases --------------------------------------------------------------------------------


def is_chip(label: str) -> bool:
    """The updater's chip: Update <version>, Updating <n>%, Updating, Waiting or Restart to Update (the Task Center's
    chip says 'Updating Grizzly Editor')."""
    return (label.startswith("Update ") or label in ("Updating", "Waiting", "Restart to Update")
            or (label.startswith("Updating ") and label.endswith("%")))


def chip_of(ed: qa.Editor) -> tuple[str, str]:
    """The updater's chip in the status bar, as `ui.audit` lists it: its label and its tip."""
    for c in ed.cmd("ui.audit").get("controls", []):
        label = c.get("label", "")
        if c.get("window") == "" and is_chip(label) and c.get("rect", [0, 0])[1] > 600:
            return label, c.get("help", "")
    return "", ""


def dialog_buttons(ed: qa.Editor) -> list[str]:
    return [c["label"] for c in ed.cmd("ui.audit").get("controls", []) if c.get("window") == "Update###dlg.update"]


def window_phase(ctx: qa.Run, mock: Mock, rel: Releases, base: dict, env: dict, v0: str, v4: str, only) -> None:
    """J: the updater's face in the window - the chip for each state, the toast, Help > Check for Updates, the
    dialog with its notes and its buttons, the Updates group of Preferences - on an installation of its own that is
    a version behind. One picture each (kept under --shots)."""
    wanted = only is None or "J" in only
    root = os.path.join(ctx.work, "Window Editor")
    install(root, v0, base)
    v5, v6 = bump(v0, 5), bump(v0, 6)
    ed = qa.Editor(ctx.project, github=True, env={**env, "GRIZZLY_UPDATE_ROOT": root})
    ed.start()
    asset4 = os.path.basename(rel.archives[v4])

    def face(state: str, label_start: str) -> tuple[str, str]:
        """Waits for the chip to say `label_start`, then takes the picture of that state."""
        got = qa.until(lambda: chip_of(ed)[0].startswith(label_start) and chip_of(ed), 20) or chip_of(ed)
        pic = qa.shot(ctx.shots, "update_" + state)
        if wanted:
            check(got[0].startswith(label_start), "J the chip says '%s' while the update is %s" % (label_start.strip(), state), got)
            check(pic.get("ok") and bool(pic.get("png_base64")), "J the window is drawn with it (picture)", qa.short(pic))
        return got

    try:
        # the check at startup runs after the first frame: that frame is drawn here
        ed.cmd("ui.list")
        st = qa.until(lambda: (s := status()).get("state") == "available" and s, 20) or status()
        label, tip = face("available", "Update ")
        if wanted:
            check(label == "Update " + v4 and st.get("latest") == v4 and tip == st.get("text"),
                  "J the chip names the new version and its tip is the updater's sentence", (label, tip, st))
            notes = ed.cmd("ui.list").get("toast_notes") or []
            check(any(n.get("button") == "Details" and n.get("action") == "help.update" and v4 in n.get("text", "")
                      for n in notes), "J the check at startup raised one toast with a Details button", notes)

            # Help > Check for Updates...: a check, then the dialog
            menu = ed.cmd("ui.popup", show="menu:Help")
            item = [c for c in ed.cmd("ui.audit").get("controls", []) if c.get("label") == "Check for Updates..."]
            check(menu.get("ok") and item, "J Help lists Check for Updates...", qa.short(menu))
            ed.cmd("ui.popup", show="none")
            ran = ed.cmd("ui.palette", action="run", id="help.check_updates")
            dialogs = qa.until(lambda: [d for d in ed.cmd("ui.list").get("dialogs", []) if d.get("id") == "update"], 10)
            check(ran.get("ok") and dialogs, "J Check for Updates... opens the Update dialog", qa.short(ran))

            # the dialog: the notes it shows are the release's, the buttons are the state's
            buttons = dialog_buttons(ed)
            check("Download and Install" in buttons and "Later" in buttons,
                  "J an available update offers Download and Install and Later", buttons)
            check(st.get("notes") == "Read whole." or v4 in str(st.get("notes")) or bool(st.get("notes")),
                  "J the updater holds the release's notes for the dialog", st.get("notes"))
            pic = qa.shot(ctx.shots, "update_dialog")
            check((pic.get("modal") or {}).get("id") == "update", "J the dialog is the window's modal (picture)",
                  qa.short(pic))
            ed.cmd("ui.dialog", action="close")

            # the Updates group of Preferences > General
            ed.cmd("app.prefs", action="show", page="general")
            controls = {c.get("label"): c for c in ed.cmd("ui.audit").get("controls", [])}
            check("Check at Startup" in controls and "Check Now" in controls,
                  "J Preferences > General has the Updates group: Check at Startup and Check Now",
                  sorted(k for k in controls if k)[:40])
            qa.shot(ctx.shots, "update_preferences")
            ed.cmd("update.set", check_at_startup=False)
            check(status().get("check_at_startup") is False, "J Check at Startup is the updater's own setting")
            ed.cmd("update.set", check_at_startup=True)
            ed.cmd("ui.tab", action="open", doc="level")

        # downloading: a connection held open mid-record
        mock.harness("reset")
        mock.harness(f"drop?after=30000&name={asset4}&hold=20000")
        ed.cmd("update.download", wait=False)
        label, _ = face("downloading", "Updating")
        ed.cmd("update.stop")
        qa.until(lambda: status().get("state") in ("available", "idle", "failed"), 20)

        # waiting: every attempt cut, five in a row
        mock.harness("reset")
        mock.harness(f"drop?after=0&name={asset4}&times=1000")
        ed.cmd("update.download", wait=False)
        face("waiting", "Waiting")
        ed.cmd("update.stop")
        qa.until(lambda: status().get("state") in ("available", "idle", "failed"), 20)

        # ready: the download ends
        mock.harness("reset")
        ed.cmd("update.download")
        face("ready", "Restart to Update")
        if wanted:
            ed.cmd("ui.dialog", action="open", id="update")
            buttons = qa.until(lambda: dialog_buttons(ed), 10)
            check("Restart and Update" in buttons and "Update on Exit" in buttons,
                  "J a ready update offers Restart and Update and Update on Exit", buttons)
            qa.shot(ctx.shots, "update_ready_dialog")
            ed.cmd("ui.dialog", action="close")

        # failed: a release whose record does not match its manifest
        rel.make(v5, dict(base, **{"data/a.txt": b"alpha of a broken release\n" * 20}), bad="data/a.txt")
        mock.publish(v5, rel.archives[v5], "A release whose manifest is wrong.")
        ed.cmd("update.check")
        ed.cmd("update.download")
        face("failed", "Update ")
        if wanted:
            check(status().get("state") == "failed" and "data/a.txt" in str(status().get("error")),
                  "J the failed update says which file did not match", status().get("error"))

        # manual: a release whose archive this client cannot read the schema of
        rel.make(v6, dict(base), schema="grizzly-editor.update/99")
        mock.publish(v6, rel.archives[v6], "A release from the future.")
        ed.cmd("update.check")
        face("manual", "Update ")
        if wanted:
            check(status().get("state") == "manual", "J a schema this client does not know is a manual update",
                  status())
            ed.cmd("ui.dialog", action="open", id="update")
            buttons = qa.until(lambda: dialog_buttons(ed), 10)
            check("Open Release Page" in buttons, "J a manual update offers the release page", buttons)
            ed.cmd("ui.dialog", action="close")
    finally:
        ed.close()


def run(ctx: qa.Run) -> None:
    only = set(ctx.only.upper().replace(",", "")) if ctx.only else None
    work = ctx.work
    root = os.path.join(work, "Grizzly Editor")
    out = os.path.join(work, "releases")
    os.makedirs(root)
    v0 = package.workspace_version()
    v1, v2, v3, v4 = (bump(v0, n) for n in (1, 2, 3, 4))
    rel = Releases(root, out)

    base = {
        "blobs/x.dat": blob(2, 300 * 1024),
        "data/a.txt": b"alpha of the first version\n" * 20,
        "data/b.txt": b"beta, the same in every version\n",
        "data/big.bin": blob(1, 600 * 1024),
        "data/gone.txt": b"the next version drops this file\n",
        "docs/c.dat": blob(3, 200 * 1024),
    }
    install(root, v0, base)
    # what the author keeps there: never the updater's
    unmanaged = {"settings.ltx": b"[editor]\ntheme = dark\n", "notes/mine.txt": b"my own notes\n"}
    for name, data in unmanaged.items():
        os.makedirs(os.path.dirname(os.path.join(root, *name.split("/"))), exist_ok=True)
        with open(os.path.join(root, *name.split("/")), "wb") as f:
            f.write(data)

    first = dict(base)
    first["data/a.txt"] = b"alpha of the second version, longer\n" * 20
    first["docs/c.dat"] = blob(4, 200 * 1024)
    first["data/new.txt"] = b"a file the second version brings\n"
    del first["data/gone.txt"]
    rel.make(v1, first)

    mock = Mock()
    env: dict = {name: None for name in ("GRIZZLY_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")}
    env.update({
        "GRIZZLY_GITHUB_API": mock.base,
        "GRIZZLY_GITHUB_WEB": mock.base,
        "GRIZZLY_UPDATE_ROOT": root,
        "GRIZZLY_UPDATE_REPO": REPO_NAME,
        "GRIZZLY_UPDATE_PACE_MS": "50",
        "GRIZZLY_UPDATE_MOVE_WAIT_MS": "1500",
    })
    mock.publish(v1, rel.archives[v1], f"# Grizzly Editor {v1}\n\n- The second version.\n- It drops gone.txt.")
    qa.mark("setup")
    editor = Editor(env)
    qa.mark("editor start")
    asset1 = os.path.basename(rel.archives[v1])

    def phase(name: str) -> bool:
        return only is None or name in only

    try:
        # A: the check
        st = status()
        check(st.get("state") == "idle" and st.get("version") == v0, "A an installation of %s is idle before a check" % v0, st)
        spans1 = rel.spans(v1)
        needed1 = ["data/a.txt", "data/new.txt", "docs/c.dat"]
        delta1 = merge([spans1[n] for n in needed1])
        st = cmd("update.check")
        if phase("A"):
            check(st.get("ok") and st.get("state") == "available" and st.get("latest") == v1,
                  "A the check finds %s" % v1, st)
            dl = st.get("download") or {}
            check(dl.get("files") == 3 and dl.get("mode") == "ranges", "A the delta is the three files that differ", dl)
            check(dl.get("bytes") == sum(b - a for a, b in delta1) and dl.get("archive") == os.path.getsize(rel.archives[v1]),
                  "A the size shown is the delta's ranges, of the archive", (dl, delta1))
            reads = mock.reads(v1)
            size = os.path.getsize(rel.archives[v1])
            check(len(reads) == 2 and reads[0]["last"] == size - 1 and
                  (reads[1]["first"], reads[1]["last"] + 1) == spans1[package.MANIFEST],
                  "A the check reads the archive's end and the manifest's record, nothing else", reads)
            gets = [r for r in mock.requests() if r["method"] == "GET"]
            check(gets and not any(r["authorized"] for r in gets), "A no request of the updater carries a token", gets)

        # B: the download, cut inside a record
        mock.harness("reset")
        cut = 70 * 1024
        mock.harness(f"drop?after={cut}&name={asset1}")
        st = cmd("update.download")
        if phase("B"):
            check(st.get("ok") and st.get("state") == "ready", "B the download ends ready", st)
            reads = mock.reads(v1)
            asked = [(r["first"], r["last"] + 1) for r in reads]
            check(len(reads) == 3 and asked[0] == delta1[0] and reads[1]["sent"] == cut,
                  "B the delta's ranges are asked for, and the second is cut", reads)
            check(len(reads) == 3 and reads[2]["first"] == reads[1]["first"] + cut and asked[2][1] == delta1[1][1],
                  "B the cut range resumes at the byte it stopped at", reads)
            sent = sum(r["sent"] for r in reads)
            check(sent == sum(b - a for a, b in delta1), "B no byte is downloaded twice, none outside the delta",
                  (sent, delta1))
            stage = os.path.join(root, ".update", v1, "files")
            check(all(read(stage, n) == first[n] for n in needed1), "B the stage holds the three files, verified")
        qa.mark("download")

        # C: a file held open rolls the transaction back
        before = snapshot(root)
        held = open(os.path.join(root, "data", "gone.txt"), "rb")
        try:
            t = time.monotonic()
            st = cmd("update.apply", restart=False)
            took = time.monotonic() - t
        finally:
            held.close()
        if phase("C"):
            check(not st.get("ok") and "data/gone.txt" in str(st.get("error")) and "in use" in str(st.get("error")),
                  "C the apply stops at the file held open and names it", st)
            check(snapshot(root) == before, "C the installation is exactly what it was")
            check(took >= 1.0, "C the move is tried for a while first", took)
            st = status()
            check(st.get("state") == "ready", "C the update is still ready", st.get("state"))

        # D: apply and exit
        st = cmd("update.apply", restart=False)
        exited = qa.until(lambda: editor.proc.poll() is not None, 30)
        qa.mark("apply")
        if phase("D"):
            check(st.get("ok") and st.get("updated") is True and st.get("from") == v0, "D the update is applied", st)
            check(exited, "D the editor ends after the update")
            now = snapshot(root)
            check(all(read(root, n) == d for n, d in first.items()), "D every managed file is the new version's")
            check(read(root, "data/gone.txt") is None, "D what the new version dropped is gone")
            check(all(now[n] == before[n] for n in ["data/b.txt", "data/big.bin", "blobs/x.dat"]),
                  "D the files that did not change were not touched")
            check(all(now[n] == before[n] for n in unmanaged), "D nothing unmanaged was touched")
            manifest = read(root, package.MANIFEST) or b""
            check(manifest.startswith(f"schema={package.SCHEMA}\nversion={v1}\n".encode()),
                  "D the new manifest went in", manifest[:80])

        # E: the first start of the new version
        editor = Editor(env)
        st = qa.until(lambda: (s := status()).get("updated_from") and not os.path.exists(os.path.join(root, ".update")) and s, 20)
        if phase("E"):
            check(bool(st) and st.get("updated_from") == v0 and st.get("updated_to") == v1 and st.get("version") == v1,
                  "E the new start knows it came from %s and is %s" % (v0, v1), st or status())
            check(not os.path.exists(os.path.join(root, ".update")), "E the updater's work folder is gone")

        # F: a record that does not match its manifest
        second = dict(first)
        second["data/a.txt"] = b"alpha of a broken release\n" * 20
        rel.make(v2, second, bad="data/a.txt")
        mock.publish(v2, rel.archives[v2], "A release whose manifest is wrong.")
        before = snapshot(root)
        st = cmd("update.check")
        st2 = cmd("update.download")
        st3 = cmd("update.apply", restart=False)
        if phase("F"):
            check(st.get("state") == "available" and st.get("latest") == v2, "F the check finds %s" % v2, st)
            check(not st2.get("ok") and "data/a.txt" in str(st2.get("error")) and "does not match" in str(st2.get("error")),
                  "F the record that does not match the manifest is refused", st2)
            check(status().get("state") == "failed", "F the update failed")
            check(not st3.get("ok") and editor.proc.poll() is None, "F nothing is applied, the editor runs on", st3)
            check(snapshot(root) == before, "F the installation is untouched")
            stage = os.path.join(root, ".update", v2, "files")
            check(read(stage, "data/a.txt") is None, "F the refused file is not in the stage")

        # G: an editor restarted mid-download continues
        third = dict(first)
        third["blobs/x.dat"] = blob(5, 400 * 1024)
        rel.make(v3, third)
        mock.publish(v3, rel.archives[v3], f"# {v3}\n- A larger blob.")
        asset3 = os.path.basename(rel.archives[v3])
        spans3 = rel.spans(v3)
        x0, x1 = spans3["blobs/x.dat"]
        # more than the archive's end the check reads, less than the record
        held_bytes = 100 * 1024
        mock.harness("reset")
        mock.harness(f"drop?after={held_bytes}&name={asset3}&hold=15000")
        st = cmd("update.download", wait=False)
        raw = os.path.join(root, ".update", v3, "files", "blobs", "x.dat.raw")
        got = qa.until(lambda: os.path.isfile(raw) and os.path.getsize(raw) >= held_bytes, 30)
        editor.close()
        qa.mark("mid-download")
        mock.harness("reset")
        editor = Editor(env)
        st = qa.until(lambda: (s := status()).get("state") == "ready" and s, 40, 0.1)
        if phase("G"):
            check(bool(got), "G the first editor had %d bytes of the record when it closed" % held_bytes)
            check(bool(st) and st.get("latest") == v3, "G the next start continues the download by itself", st or status())
            reads = [r for r in mock.reads(v3) if (r["first"], r["last"] + 1) != spans3[package.MANIFEST]
                     and r["last"] != os.path.getsize(rel.archives[v3]) - 1]
            check(len(reads) == 1 and reads[0]["first"] == x0 + held_bytes and reads[0]["last"] + 1 == x1,
                  "G it asks for the record from the byte the last editor stopped at", (reads, x0, x1))
            check(read(os.path.join(root, ".update", v3, "files"), "blobs/x.dat") == third["blobs/x.dat"],
                  "G the record is whole and verified")

        # H: the restart handshake
        old = editor.proc
        st = cmd("update.apply", restart=True)
        ended = qa.until(lambda: old.poll() is not None, 40)
        back = qa.until(lambda: qa.port_open() and (s := status()).get("updated_from") and s, 40, 0.1)
        qa.mark("restart")
        if phase("H"):
            check(st.get("ok") and st.get("restart") is True, "H the update is applied for a restart", st)
            check(bool(ended) and old.returncode == 0, "H the old editor ends once the new one reported", old.returncode)
            check(bool(back) and back.get("updated_from") == v1 and back.get("version") == v3,
                  "H the restarted editor answers updated_from %s and is %s" % (v1, v3), back)
            check(all(read(root, n) == d for n, d in third.items()), "H the installation is %s" % v3)
            check(bool(qa.until(lambda: not os.path.exists(os.path.join(root, ".update")), 20)),
                  "H the new start removed the updater's work folder")

        # I: a server without ranges gets the whole archive
        fourth = dict(third)
        fourth["data/a.txt"] = b"alpha of the fifth version\n" * 20
        rel.make(v4, fourth)
        mock.publish(v4, rel.archives[v4], "Read whole.")
        mock.harness("reset")
        mock.harness("noranges?on=1")
        st = cmd("update.check")
        st2 = cmd("update.download")
        if phase("I"):
            check(st.get("state") == "available" and (st.get("download") or {}).get("mode") == "whole",
                  "I a 200 to the ranged check means the whole archive", st)
            reads = mock.reads(v4)
            size = os.path.getsize(rel.archives[v4])
            check(st2.get("ok") and st2.get("state") == "ready", "I the whole archive is downloaded and unpacked", st2)
            check(any(r["status"] == 200 and r["sent"] == size for r in reads), "I the archive came whole", reads)
            stage = os.path.join(root, ".update", v4)
            check(read(os.path.join(stage, "files"), "data/a.txt") == fourth["data/a.txt"] and
                  not os.path.exists(os.path.join(stage, "archive.zip.part")),
                  "I the file that differs is staged and the archive is gone")

        # J: the window's side - on an installation of its own that is a version behind
        editor.close()
        window_phase(ctx, mock, rel, base, env, v0, v4, only)
    finally:
        try:
            editor.close()
        finally:
            mock.stop()


def main() -> int:
    # the window's phase needs a project with a game: the menus and Preferences are not on the Start page
    return qa.suite("UPDATE", run, doc=__doc__, trimmed=True)


if __name__ == "__main__":
    sys.exit(main())
