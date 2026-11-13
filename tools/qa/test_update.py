#!/usr/bin/env python3
"""The editor updates itself (docs/UPDATES.md, section 7), against tools/qa/github_mock.

A fake installation in the suite's temp folder - harmless text and binary blobs, no program, no
`MZ` head - is the editor's installation (`GRIZZLY_UPDATE_ROOT`, honoured only under
`-qa_github`); the releases are mock releases whose archives this suite packs the way
tools/rust/package.py does. A headless QA editor on its own port and home drives it over the
control socket. Nothing is fetched from GitHub or published anywhere.

    python tools/qa/test_update.py [--only A,B,...]

  A  the check reads the archive's end and its manifest, nothing else; the delta is what differs
  B  only the delta's ranges are requested; a connection cut inside a record resumes at its byte
  C  a file of the installation held open rolls the whole transaction back
  D  apply replaces, removes what the new version dropped and touches nothing unmanaged
  E  the first start of the new version says so and tidies up
  F  a record that does not match the manifest is refused and nothing is applied
  G  an editor restarted mid-download continues where the last one stopped
  H  the restart handshake: the new start reports and answers updated_from
  I  a 200 to a ranged request falls back to the whole archive

The phases run in order on one installation: each stands on the version the one before left.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile

import qa_target  # noqa: I001  (first: it sets the suite's home)
from mock_server import mock_exe

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "tools", "rust"))
import package  # noqa: E402

MCP = qa_target.MCP
TOKEN = "qa_token_update_0123456789abcdef"
REPO_NAME = "QaOwner/Grizzly-Editor"
GAP = 256 * 1024

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + str(detail)[:600]) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 120.0, **args) -> dict:
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps({"cmd": cmd, **args}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def port_open() -> bool:
    try:
        socket.create_connection(MCP, timeout=0.5).close()
        return True
    except OSError:
        return False


def wait_for(what, timeout: float, step: float = 0.05):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        got = what()
        if got:
            return got
        time.sleep(step)
    return what()


def blob(seed: int, size: int) -> bytes:
    data = random.Random(seed).randbytes(size)
    # never the head of a program
    return b"\0" + data[1:] if data[:2] == b"MZ" else data


def bump(version: str, n: int) -> str:
    major, minor, patch = (int(x) for x in version.split("."))
    return f"{major}.{minor}.{patch + n}"


# ---- the mock ----------------------------------------------------------------------------------


class Mock:
    def __init__(self) -> None:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            self.port = s.getsockname()[1]
        self.base = f"http://127.0.0.1:{self.port}"
        self.proc = subprocess.Popen([mock_exe(), "--port", str(self.port), "--token", TOKEN, "--repo", REPO_NAME],
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        if json.loads(self.proc.stdout.readline())["port"] != self.port:
            raise RuntimeError("the mock did not take its port")

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

    def stop(self) -> None:
        self.proc.kill()
        self.proc.wait(timeout=10)


# ---- the installation and its releases ---------------------------------------------------------


class Releases:
    """The versions: their files, archives and the layout of each archive."""

    def __init__(self, base: str, out: str) -> None:
        self.base, self.out = base, out
        self.files: dict[str, dict[str, bytes]] = {}
        self.archives: dict[str, str] = {}

    def make(self, version: str, files: dict[str, bytes], bad: str | None = None) -> str:
        """Packs `files` as `version`; with `bad` the manifest names other bytes of the same size
        for that path."""
        self.files[version] = files
        folder = os.path.join(self.out, version)
        os.makedirs(folder, exist_ok=True)
        if bad is None:
            archive, _ = package.pack(version, dict(files), folder)
        else:
            rows = []
            for name, data in files.items():
                claimed = bytes(b ^ 0xFF for b in data) if name == bad else data
                rows.append((hashlib.sha256(claimed).hexdigest(), len(data), name))
            manifest = package.manifest_text(version, rows)
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


class Editor:
    def __init__(self, env: dict[str, str]) -> None:
        if port_open():
            raise RuntimeError(qa_target.busy_text())
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        exe = qa_target.exe()
        self.proc = subprocess.Popen([exe, *qa_target.ARGS, "-qa_github", "-nodlg", "-headless"],
                                     cwd=os.path.dirname(exe), env=env, startupinfo=startup)
        if not wait_for(lambda: port_open() or self.proc.poll() is not None, 60) or self.proc.poll() is not None:
            raise RuntimeError("the editor did not start")

    def close(self) -> None:
        close_editor()
        try:
            self.proc.wait(timeout=60)
        except subprocess.TimeoutExpired:
            print("! the editor did not close on its own - leaving it to the operator", flush=True)


def close_editor() -> None:
    try:
        mcp("app.quit", timeout=30)
    except OSError:
        pass
    wait_for(lambda: not port_open(), 60)


def status() -> dict:
    return mcp("update.status")


# ---- the phases --------------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", help="phases to report (all run: each stands on the one before)")
    args = ap.parse_args()
    only = set(args.only.upper().split(",")) if args.only else None
    t0 = time.monotonic()

    work = tempfile.mkdtemp(prefix="grizzly_update_")
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
    env = dict(os.environ)
    for name in ("GRIZZLY_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"):
        env.pop(name, None)
    env.update({
        "GRIZZLY_GITHUB_API": mock.base,
        "GRIZZLY_GITHUB_WEB": mock.base,
        "GRIZZLY_UPDATE_ROOT": root,
        "GRIZZLY_UPDATE_REPO": REPO_NAME,
        "GRIZZLY_UPDATE_PACE_MS": "50",
        "GRIZZLY_UPDATE_MOVE_WAIT_MS": "1500",
    })
    mock.publish(v1, rel.archives[v1], f"# Grizzly Editor {v1}\n\n- The second version.\n- It drops gone.txt.")
    qa_target.mark("setup")
    editor = Editor(env)
    qa_target.mark("editor start")
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
        st = mcp("update.check")
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
        st = mcp("update.download")
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
        qa_target.mark("download")

        # C: a file held open rolls the transaction back
        before = snapshot(root)
        held = open(os.path.join(root, "data", "gone.txt"), "rb")
        try:
            t = time.monotonic()
            st = mcp("update.apply", restart=False)
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
        st = mcp("update.apply", restart=False)
        exited = wait_for(lambda: editor.proc.poll() is not None, 30)
        qa_target.mark("apply")
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
        st = wait_for(lambda: (s := status()).get("updated_from") and not os.path.exists(os.path.join(root, ".update")) and s, 20)
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
        st = mcp("update.check")
        st2 = mcp("update.download")
        st3 = mcp("update.apply", restart=False)
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
        st = mcp("update.download", wait=False)
        raw = os.path.join(root, ".update", v3, "files", "blobs", "x.dat.raw")
        got = wait_for(lambda: os.path.isfile(raw) and os.path.getsize(raw) >= held_bytes, 30)
        editor.close()
        qa_target.mark("mid-download")
        mock.harness("reset")
        editor = Editor(env)
        st = wait_for(lambda: (s := status()).get("state") == "ready" and s, 40, 0.1)
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
        st = mcp("update.apply", restart=True)
        ended = wait_for(lambda: old.poll() is not None, 40)
        back = wait_for(lambda: port_open() and (s := status()).get("updated_from") and s, 40, 0.1)
        qa_target.mark("restart")
        if phase("H"):
            check(st.get("ok") and st.get("restart") is True, "H the update is applied for a restart", st)
            check(bool(ended) and old.returncode == 0, "H the old editor ends once the new one reported", old.returncode)
            check(bool(back) and back.get("updated_from") == v1 and back.get("version") == v3,
                  "H the restarted editor answers updated_from %s and is %s" % (v1, v3), back)
            check(all(read(root, n) == d for n, d in third.items()), "H the installation is %s" % v3)
            check(bool(wait_for(lambda: not os.path.exists(os.path.join(root, ".update")), 20)),
                  "H the new start removed the updater's work folder")

        # I: a server without ranges gets the whole archive
        fourth = dict(third)
        fourth["data/a.txt"] = b"alpha of the fifth version\n" * 20
        rel.make(v4, fourth)
        mock.publish(v4, rel.archives[v4], "Read whole.")
        mock.harness("reset")
        mock.harness("noranges?on=1")
        st = mcp("update.check")
        st2 = mcp("update.download")
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

        stalls = mcp("perf.stalls")
        if stalls.get("ok"):
            check(not stalls.get("stalls"), "the editor's UI thread never stalled", stalls)
    finally:
        try:
            if port_open():
                close_editor()
        finally:
            mock.stop()
            qa_target.remove_tree(work)

    failed = [t for ok, t in results if not ok]
    print("%d checks, %d failed, %.1f s" % (len(results), len(failed), time.monotonic() - t0), flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
