#!/usr/bin/env python3
"""End-to-end check of Mod > Submit to Mod Browser (mod_catalog) against tools/qa/github_mock.py.

Drives the editor over MCP on a THROW-AWAY COPY of a project. The editor runs with -qa_github:
GitHub, the catalog repository, the catalog's public files and VirusTotal are all the mock, and the
author key file and catalog.ltx live in a temporary XFINED_CATALOG_HOME instead of
Documents\\XFined Editor - the real author key and the real Windows credentials are never touched.
Every verdict about keys and signatures is taken from the game repository's own reference
implementation (tools/catalog/xms_catalog.py and catalog.py), never from the editor's word.

  A  the author key file (MOD_CATALOG 4.1): made protected, never overwritten, a wrong password,
     another copy taken and unlocked - each read back with xms_catalog.py
  B  the manifest: the thumbnail made from a picture (6.1) and the tags (6.2)
  C  the content policy (8) and the script report (10.3) at package time
  D  the signed [card] of the release descriptor (4), verified with xms_catalog.py
  E  the release published to the mock
  F  the listing requirements (7) against the published release and the catalog index - a package VirusTotal does
     not know is a 'required' finding, never a note; the window opened and closed over MCP (show / hide), and drawn
     where a screenshot holds it although the layout file saved its place far below the editor, and while the
     editor is minimized (every later picture is taken of a minimized editor)
  G  VirusTotal with the author's own key (11), required: without one the check blocks and nothing is sent;
     the scan uploads what VirusTotal does not know, waits for the report and judges it; a block stops a submission
  H  the submission, scanned first: an issue 'Submit <id>' whose ini block is mods/<id>.ltx byte for byte, edited not
     duplicated, the verdicts the catalog's workflow comments (github-actions[bot] only, the newest one, past
     the first page), accepted and listed, nothing sent once the catalog holds the file
  W  the withdrawal: an issue 'Withdraw <id>' from the listing's owner whose ini block names the module, edited not
     duplicated, the catalog's verdict, the index's [withdrawn], nothing sent once withdrawn, the check's note, and
     a new submission that lists it again
  I  hygiene: no token, password or VirusTotal key where it must not go
  J  the maintainer listing a module of their own the same way; refused and closed, then a new submission; a
     withdrawal of a module the catalog does not hold, or one listed from another account, sends nothing

    python tools/qa/test_mod_catalog.py --project D:\\XFinedProjects\\Test [--shots <folder>]
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import ctypes.wintypes
import hashlib
import json
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.parse
import urllib.request
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
MCP = ("127.0.0.1", 28016)
TOKEN = "qa_token_catalog_0123456789abcdef"
VT_KEY = "qa-vt-key-5b9e2c71d4"
GITHUB_REPO = "QaOwner/qa-mod"
MAINTAINER = "QaMaintainer"
CATALOG_REPO = MAINTAINER + "/darf-mods"
MAINTAINER_REPO = MAINTAINER + "/qa-mod"
MODULE_ID = "qa_catalog_mod"
MODULE_NAME = "QA Catalog Mod"
AUTHOR = "QA Author"
DESCRIPTION = 'Mod Browser QA module.\nA "quoted" word.'
VERSION = "1.0.0"
WEBSITE = "https://ap-pro.ru/stuff/qa/qa-mod-r1/"
SUBMIT_TITLE = "Submit " + MODULE_ID
SUBMITTED_LINE = "Submitted from XFined Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 7)."
WITHDRAW_TITLE = "Withdraw " + MODULE_ID
WITHDRAWN_LINE = "Withdrawn by its author from XFined Editor. The catalog's own checks judge it (MOD_CATALOG.md, section 10.2)."
# Cyrillic, quotes and a backslash: the password crosses JSON, UTF-8 and PBKDF2 on both sides
PASSWORD = 'Qa-пароль "7" \\ ok'
PASSWORD2 = "second-key-pass-9"
TRUSTED = "EngineT"
VT_PAGE = "https://www.virustotal.com/gui/file/"
JOB_SECONDS = 120.0
SECRETS = (PASSWORD, PASSWORD2, VT_KEY, TOKEN)
SUBMIT_WINDOW = "XFined Editor - Submit to Mod Browser"
# the editor's ImGui layout ($local_root$) and the one a fresh checkout starts from, and its preferences - the
# window's size and state among them: the editor writes both when it closes
LAYOUT = os.path.join(REPO, "level_imgui.ini")
LAYOUT_DEFAULT = os.path.join(REPO, "rawdata", "level_imgui_default.ini")
PREFS = os.path.join(REPO, "level.ini")

# the game repository's reference implementation, imported in main()
xc = None
cat = None

results: list[tuple[bool, str]] = []
# every MCP answer, for the secret scan
answers: list[str] = []


class Abort(Exception):
    """Going on would do harm - e.g. write a key outside the QA home."""


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def phase(name: str, step, *args) -> None:
    """One phase; a phase that throws is a FAIL of its own and the next one still runs."""
    try:
        step(*args)
    except Abort:
        raise
    except Exception as exc:  # noqa: BLE001 - the report is the point
        traceback.print_exc()
        check(False, "%s ran to its end" % name, repr(exc))


def mcp(cmd: str, timeout: float = 300.0, **extra) -> dict:
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    data = json.loads(buf.decode("utf-8"))
    answers.append(json.dumps(data, ensure_ascii=False))
    return data


def mcp_up() -> bool:
    try:
        socket.create_connection(MCP, timeout=1).close()
        return True
    except OSError:
        return False


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def short(data) -> str:
    return json.dumps(data, ensure_ascii=False)[:600]


class Mock:
    def __init__(self, port: int, work: str, name: str, *flags: str, repo: str = GITHUB_REPO):
        self.port = port
        self.base = f"http://127.0.0.1:{port}"
        self.log = os.path.join(work, name + ".log")
        # a failing handler prints its traceback there; the operator reads it after a FAIL
        self.errors = open(os.path.join(work, name + ".stderr.txt"), "w", encoding="utf-8")
        self.proc = subprocess.Popen(
            [sys.executable, os.path.join(HERE, "github_mock.py"), "--port", str(port), "--token", TOKEN,
             "--repo", repo, "--catalog-repo", CATALOG_REPO, "--vt-key", VT_KEY, "--log", self.log, *flags],
            stdout=subprocess.PIPE, stderr=self.errors, text=True)
        line = self.proc.stdout.readline()
        if not line or json.loads(line)["port"] != port:
            raise RuntimeError("the mock did not take its port: " + line)

    def get(self, path: str):
        with urllib.request.urlopen(self.base + path, timeout=10) as r:
            return json.loads(r.read().decode("utf-8"))

    def post(self, path: str, body: bytes = b"", content_type: str = "application/octet-stream"):
        request = urllib.request.Request(self.base + path, data=body, method="POST",
                                         headers={"Content-Type": content_type})
        with urllib.request.urlopen(request, timeout=10) as r:
            raw = r.read()
        return json.loads(raw.decode("utf-8")) if raw else None

    def state(self) -> dict:
        return self.get("/__state")

    def catalog(self) -> dict:
        return self.get("/__catalog")

    def vt(self) -> dict:
        return self.get("/__vt")

    def put_file(self, path: str, text: str, branch: str = "main") -> None:
        """Commits one file to the catalog repository, past the editor - the catalog's own push."""
        query = urllib.parse.urlencode({"path": path, "branch": branch})
        self.post("/__catalog/file?" + query, text.encode("utf-8"))

    def close(self, number: int, reason: str = "") -> None:
        """Closes an issue of the catalog as its workflow (github-actions[bot]) does; `reason` completed by default."""
        query = {"number": number}
        if reason:
            query["reason"] = reason
        self.post("/__catalog/close?" + urllib.parse.urlencode(query))

    def comment(self, number: int, text: str, author: str = "", count: int = 1) -> None:
        """`count` comments on an issue of the catalog: its workflow's (github-actions[bot]) unless `author` says."""
        query = {"number": number, "count": count}
        if author:
            query["author"] = author
        self.post("/__catalog/comment?" + urllib.parse.urlencode(query), text.encode("utf-8"))

    def issue(self, creator: str, title: str, body: str, pull: bool = False) -> None:
        """An issue of the catalog by anyone; `pull` makes it an entry that stands for a pull request."""
        query = {"creator": creator, "title": title}
        if pull:
            query["pull"] = 1
        self.post("/__catalog/issue?" + urllib.parse.urlencode(query), body.encode("utf-8"))

    def vt_seed(self, verdicts: dict) -> None:
        self.post("/__vt/seed", json.dumps(verdicts).encode("utf-8"), "application/json")

    def vt_forget(self, sha: str) -> None:
        """VirusTotal no longer knows the file, as if it had never seen it."""
        self.post("/__vt/forget?sha=" + sha)

    def requests(self) -> list[dict]:
        if not os.path.isfile(self.log):
            return []
        with open(self.log, encoding="utf-8") as f:
            return [json.loads(line) for line in f if line.strip()]

    def stop(self) -> None:
        try:
            urllib.request.urlopen(urllib.request.Request(self.base + "/__shutdown", method="POST"), timeout=5).read()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.errors.close()


class Editor:
    def __init__(self, exe: str, project: str, port: int, home: str, vt_variable: bool, extra_env: dict | None = None):
        if mcp_up():
            raise RuntimeError("an editor already answers on 127.0.0.1:28016 - close it first")
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        env = dict(os.environ)
        for name in ("XFINED_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN", "XFINED_VT_KEY", "VT_API_KEY"):
            env.pop(name, None)
        origin = f"http://127.0.0.1:{port}"
        env.update({
            "XFINED_GITHUB_API": origin,
            "XFINED_GITHUB_WEB": origin,
            "XFINED_GITHUB_CLIENT_ID": "qa-client",
            "XFINED_GITHUB_TOKEN": TOKEN,
            "XFINED_CATALOG_REPO": CATALOG_REPO,
            "XFINED_CATALOG_BASE": origin + "/catalog/public/",
            "XFINED_VT_API": origin + "/vt/api/v3",
            "XFINED_CATALOG_HOME": home,
            # the free API's pace and the wait for a report, in QA terms
            "XFINED_VT_PACE_MS": "100",
            "XFINED_VT_SCAN_WAIT_MS": "15000",
        })
        if vt_variable:
            env["XFINED_VT_KEY"] = VT_KEY
        env.update(extra_env or {})
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        self.proc = subprocess.Popen([exe, "-qa_github", "-nodlg", "-project", project],
                                     cwd=os.path.dirname(exe), env=env, startupinfo=startup)
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(1)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("window_control", action="close")
        except (OSError, ValueError):
            pass
        try:
            self.proc.wait(timeout=120)
        except subprocess.TimeoutExpired:
            print("! the editor did not close on its own - leaving it to the operator", flush=True)


# ---- MCP helpers ---------------------------------------------------------------------------------

def follow(data: dict, seconds: float = JOB_SECONDS) -> dict:
    """A mod_catalog answer, followed through action=status until its job is over."""
    deadline = time.monotonic() + seconds
    while data.get("ok") and (data.get("job") or {}).get("pending") and time.monotonic() < deadline:
        time.sleep(0.5)
        data = mcp("mod_catalog", action="status")
    return data


def job(action: str, seconds: float = JOB_SECONDS, **extra) -> dict:
    """mod_catalog <action> on its worker, followed until the job is over."""
    return follow(mcp("mod_catalog", action=action, **extra), seconds)


def shot(folder: str | None, name: str) -> dict:
    """The editor as it looks now, for the operator - only when --shots names a folder."""
    if not folder:
        return {}
    pic = mcp("screenshot_editor")
    if pic.get("png_base64"):
        path = os.path.join(folder, name + ".png")
        write_bytes(path, base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)
    return pic


def platform_windows(pid: int) -> list[str]:
    """Titles of the visible windows ImGui opened for the process on its own ('ImGui Platform' class): what they
    draw is in no screenshot of the editor, which mirrors the main window alone."""
    user32 = ctypes.windll.user32
    found: list[str] = []

    @ctypes.WINFUNCTYPE(ctypes.wintypes.BOOL, ctypes.wintypes.HWND, ctypes.wintypes.LPARAM)
    def each(hwnd, _):
        owner = ctypes.wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            text = ctypes.create_unicode_buffer(256)
            user32.GetClassNameW(hwnd, text, 256)
            if "ImGui" in text.value:
                user32.GetWindowTextW(hwnd, text, 256)
                found.append(text.value)
        return True

    user32.EnumWindows(each, 0)
    return found


def png_rows(data: bytes, count: int) -> tuple[int, int, int, list[bytes]]:
    """Width, height, bytes per pixel and the first `count` rows, unfiltered, of an 8-bit RGB or RGBA PNG."""
    pos, idat, width, height, bpp = 8, [], 0, 0, 4
    while pos + 8 <= len(data):
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            width, height, _, color = struct.unpack(">IIBB", body[:10])
            bpp = 3 if color == 2 else 4
        elif kind == b"IDAT":
            idat.append(body)
        pos += 12 + length
    raw = zlib.decompress(b"".join(idat))
    stride = width * bpp
    rows, prev = [], bytes(stride)
    for y in range(min(count, height)):
        start = y * (stride + 1)
        kind, line = raw[start], bytearray(raw[start + 1:start + 1 + stride])
        if kind == 1:
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 255
        elif kind == 2:
            line = bytearray((a + b) & 255 for a, b in zip(line, prev))
        elif kind == 3:
            for i in range(stride):
                line[i] = (line[i] + ((line[i - bpp] if i >= bpp else 0) + prev[i]) // 2) & 255
        elif kind == 4:
            for i in range(stride):
                a, b, c = (line[i - bpp], prev[i], prev[i - bpp]) if i >= bpp else (0, prev[i], 0)
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = rows[-1]
    return width, height, bpp, rows


def window_drawn(data: bytes, modal: dict) -> str:
    """Why the picture does not hold a drawn window where the editor says the modal is - '' when it does: inside
    the picture, and more than a flat fill there (a window has a frame, a title and text)."""
    x, y, w, h = (int(modal.get(k, -1)) for k in ("x", "y", "w", "h"))
    width, height, bpp, rows = png_rows(data, y + h)
    if x < 0 or y < 0 or w <= 0 or h <= 0 or x + w > width or y + h > height:
        return "its rect %s leaves the %dx%d picture" % ([x, y, w, h], width, height)
    colours = set()
    for row in rows[y:y + h]:
        line = row[x * bpp:(x + w) * bpp]
        colours.update(line[i:i + bpp] for i in range(0, len(line), bpp))
        if len(colours) > 16:
            return ""
    return "its rect %s holds %d colour(s): nothing is drawn there" % ([x, y, w, h], len(colours))


def stale_layout(layout: bytes, title: str, y: int) -> bytes:
    """The layout with the window's saved place y pixels down, below any editor window and any monitor."""
    text = layout.decode("utf-8")
    head = "[Window][%s]" % title
    # the window's own entry goes - it may name a viewport of its own as well - and a new one comes last
    text = re.sub(r"^%s\r?\n(?:[^\[\r\n].*(?:\r?\n|$))*" % re.escape(head), "", text, flags=re.M)
    return (text.rstrip("\r\n") + "\n\n%s\nPos=784,%d\nSize=971,700\nCollapsed=0\n" % (head, y)).encode("utf-8")


def job_state(data: dict) -> str:
    return (data.get("job") or {}).get("state", "")


def job_done(data: dict, kind: str) -> bool:
    j = data.get("job") or {}
    return bool(data.get("ok")) and j.get("kind") == kind and j.get("state") == "done" and not j.get("pending")


def job_failed(data: dict) -> bool:
    """Refused at once (ok:false) or started and failed on the worker - both are a refusal."""
    return not data.get("ok") or job_state(data) == "failed"


def job_error(data: dict) -> str:
    return str(data.get("error") or "") + " " + str((data.get("job") or {}).get("error") or "")


def findings(data: dict) -> list[dict]:
    return list((data.get("check") or {}).get("findings") or [])


def levels(data: dict, code: str) -> list[str]:
    return [f.get("level", "") for f in findings(data) if f.get("code") == code]


def blocks(data: dict) -> list[dict]:
    return [f for f in findings(data) if f.get("level") == "block"]


def vt_entry(data: dict, sha: str) -> dict:
    packages = (data.get("virustotal") or {}).get("packages") or []
    return next((v for v in packages if str(v.get("sha256", "")).lower() == sha), {})


def vt_state(data: dict) -> str:
    return (data.get("virustotal") or {}).get("state", "")


def texts(data: dict, code: str) -> list[str]:
    return [f.get("text", "") for f in findings(data) if f.get("code") == code]


def publish(wait_seconds: float = 240.0) -> dict:
    """mod_publish with the manifest as it is, followed to the end."""
    data = mcp("mod_publish")
    deadline = time.monotonic() + wait_seconds
    while data.get("ok") and data.get("pending") and time.monotonic() < deadline:
        time.sleep(0.3)
        data = mcp("mod_publish", action="status")
    return data


# ---- files ----------------------------------------------------------------------------------------

def read_bytes(path: str) -> bytes | None:
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def write_bytes(path: str, data: bytes) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def same_path(a: str, b: str) -> bool:
    return bool(a) and os.path.normcase(os.path.normpath(a)) == os.path.normcase(os.path.normpath(b))


def png(width: int, height: int, pixel, alpha: bool) -> bytes:
    """A PNG from pixel(x, y) -> (r, g, b[, a]): RGB when opaque by format, RGBA otherwise."""
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        for x in range(width):
            raw.extend(pixel(x, y))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 6 if alpha else 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + \
        chunk(b"IEND", b"")


def opaque_picture() -> bytes:
    """300 x 200: a red strip, a green middle square, a blue strip - only the square may reach the DDS."""
    return png(300, 200, lambda x, y: (255, 0, 0) if x < 50 else (0, 0, 255) if x >= 250 else (0, 255, 0), False)


def transparent_picture() -> bytes:
    return png(64, 48, lambda x, y: (200, 180, 40, 0 if x < 32 else 255), True)


def dds_info(data: bytes | None) -> dict:
    if not data or len(data) < 128:
        return {}
    return {"size": len(data), "height": struct.unpack_from("<I", data, 12)[0],
            "width": struct.unpack_from("<I", data, 16)[0], "mips": struct.unpack_from("<I", data, 28)[0],
            "fourcc": data[84:88].decode("latin-1")}


def top_level_is_green(data: bytes) -> bool:
    """Every DXT1 block of the 256 x 256 level shows the middle square: green end points, no red, no blue."""
    for at in range(128, 128 + 64 * 64 * 8, 8):
        ends = struct.unpack_from("<HH", data, at)
        if any((c >> 11) > 4 or (c & 31) > 4 for c in ends) or max((c >> 5) & 63 for c in ends) < 48:
            return False
    return True


def plant_program() -> bytes:
    """A PE image: MZ, e_lfanew at 0x3C pointing at PE\\0\\0."""
    return b"MZ" + b"\0" * 58 + struct.pack("<I", 64) + b"PE\0\0" + b"\0" * 188


BAD_SCRIPT = b"""-- QA: what a catalog module must not do
local code = "return 1"
os.execute("calc")
local chunk = loadstring(code)
"""


def parse_signed_ini(data: bytes) -> dict:
    """{section: {key: value}} of a signed descriptor's body, quotes resolved (catalog.py's parser)."""
    body, _, _, _ = xc.split_signed(data)
    return {name: {k: cat.unquote(v) for k, v in rows} for name, rows in cat.parse_ini(body.decode("utf-8")).items()}


def read_index(path: str) -> dict[str, tuple[int, str]]:
    """path -> (size, sha256) of every file record of a file index (MOD_UPDATES 2.2)."""
    out = {}
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for line in lines[1:]:
        parts = line.split(" ", 7)
        if parts[0] == "file" and len(parts) == 8:
            out[parts[7]] = (int(parts[2]), parts[1].lower())
    return out


def card_file(value: str) -> tuple[str, int, str]:
    parts = [p.strip() for p in value.split(",")]
    if len(parts) != 3 or not parts[1].isdigit():
        return "", -1, ""
    return parts[0], int(parts[1]), parts[2].lower()


def index_text(maintainer: int, serial: int, rows: list[str], withdrawn: tuple[str, ...] = ()) -> str:
    """public/index.ltx as the maintainer publishes it (MOD_CATALOG 5.2), signed with a QA maintainer key."""
    body = "\n".join(["[catalog]", "format  = 1", "serial  = %d" % serial, "issued  = 2026-09-25", "",
                      "[virustotal]", "trusted       = " + TRUSTED, "block_trusted = 1", "warn_trusted  = 1",
                      "block_others  = 5", "warn_others   = 2", "",
                      "[mods]", *rows, "", "[withdrawn]", *withdrawn, "", "[revoked]", ""]) + "\n"
    return xc.sign_text(body, maintainer)


def listing(module_id: str, github: str, key_text: str) -> str:
    return "%s = %s, %s, 1.0.0, 2026-09-25" % (module_id, github, key_text)


def submission_fields(text: str) -> dict:
    return dict(cat.parse_ini(text).get("mod", []))


def documents_folder() -> str:
    buffer = ctypes.create_unicode_buffer(1024)
    # CSIDL_PERSONAL: where Documents really is, OneDrive redirection included
    ctypes.windll.shell32.SHGetFolderPathW(None, 5, None, 0, buffer)
    return buffer.value


def snapshot(folder: str) -> dict:
    """What the real Documents\\XFined Editor holds of the catalog - a QA run must leave it as it was."""
    out = {}
    for rel in ("catalog.ltx", os.path.join("Keys", "author.xkey")):
        out[rel] = read_bytes(os.path.join(folder, rel))
    keys = os.path.join(folder, "Keys")
    out["Keys/"] = sorted(os.listdir(keys)) if os.path.isdir(keys) else None
    return out


def leaks_in(paths: list[str]) -> list[str]:
    """Files under `paths` that hold a secret, as UTF-8 or as cp1251. A crash dump is process memory, where a
    password lives while it is used - not something the editor writes down - so dumps are left out."""
    needles = []
    for secret in SECRETS:
        needles.append(secret.encode("utf-8"))
        needles.append(secret.encode("cp1251", "ignore"))
    found = []
    for root in paths:
        if os.path.isfile(root):
            walk = [(os.path.dirname(root), [], [os.path.basename(root)])]
        elif os.path.isdir(root):
            walk = os.walk(root)
        else:
            continue
        for base, _, files in walk:
            for name in files:
                if name.lower().endswith((".mdmp", ".dmp")):
                    continue
                path = os.path.join(base, name)
                data = read_bytes(path) or b""
                if any(needle and needle in data for needle in needles):
                    found.append(path)
    return found


def route_class(path: str) -> str:
    p = urllib.parse.urlsplit(path).path
    if p.startswith("/__"):
        return "harness"
    if p.startswith("/vt/"):
        return "vt"
    if p.startswith("/catalog/"):
        return "catalog"
    if p == "/user" or p.startswith(("/user/", "/repos/", "/upload/", "/graphql")):
        return "api"
    if p.startswith("/login/"):
        return "device"
    return "web"


# ---- the mock's catalog and VirusTotal, as its harness routes report them ------------------------------------
# /__catalog: {"repositories": {"<owner/name>": {"branches": {"<name>": {"sha", "message", "files": {path: text},
# "files_base64": {...}}}}}, "issues": [GitHub's issue JSON + "comment_list": [GitHub's comment JSON]]}
# /__vt: {"files": {"<sha256>": {"sha256", "size", "results", "queued", "uploads", "names"}}, "analyses": [...]}

def repo_of(state: dict, full_name: str) -> dict:
    repos = state.get("repositories") or {}
    return next((repo for name, repo in repos.items() if name.lower() == full_name.lower()), {})


def branch_file(state: dict, full_name: str, branch: str, path: str) -> str | None:
    return (((repo_of(state, full_name).get("branches") or {}).get(branch) or {}).get("files") or {}).get(path)


def issues(state: dict) -> list[dict]:
    return list(state.get("issues") or [])


def issue_of(state: dict, number: int) -> dict:
    return next((i for i in issues(state) if i.get("number") == number), {})


def submissions(state: dict, creator: str, title: str = SUBMIT_TITLE) -> list[dict]:
    """The creator's issues titled exactly 'Submit <id>' (or `title`), oldest first; a pull request is no submission."""
    return [i for i in issues(state) if str((i.get("user") or {}).get("login", "")).lower() == creator.lower()
            and i.get("title") == title and "pull_request" not in i]


def ini_block(body: str) -> str | None:
    """What the first fenced ```ini block of an issue body holds, byte for byte, the fence lines left out."""
    lines = body.splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.rstrip("\r\n") == "```ini":
            for j in range(i + 1, len(lines)):
                if lines[j].rstrip("\r\n") == "```":
                    return "".join(lines[i + 1:j])
            return None
    return None


def after_block(body: str) -> str:
    """What an issue body says after its ini block."""
    return body.split("\n```\n", 1)[1] if "\n```\n" in body else ""


def submission_text(github: str, key: str) -> str:
    """mods/<id>.ltx as the catalog is to commit it: the text the editor submits."""
    return "[mod]\nid      = %s\ngithub  = %s\nkey     = %s\nversion = %s\nwebsite = %s\n" % (
        MODULE_ID, github, key, VERSION, WEBSITE)


def api_requests(mock, method: str, tail: str) -> list[dict]:
    """The API requests of the mock's log with this method whose path, query left out, ends with `tail`."""
    return [r for r in mock.requests()
            if r.get("method") == method and urllib.parse.urlsplit(r.get("path", "")).path.endswith(tail)]


def uploads_of(vt_state: dict, sha: str) -> int:
    return ((vt_state.get("files") or {}).get(sha) or {}).get("uploads", 0)


# ---- the run -------------------------------------------------------------------------------------------

class Run:
    def __init__(self, args, work: str, copy: str):
        self.exe = args.editor
        self.shots = os.path.abspath(args.shots) if args.shots else None
        self.work = work
        self.copy = copy
        self.home = os.path.join(work, "catalog_home")
        self.key_path = os.path.join(self.home, "Keys", "author.xkey")
        self.maintainer_key = xc.new_private()
        self.stranger = xc.key_text(xc.public_of(xc.new_private()))
        self.seed = os.path.join(work, "catalog_seed")
        self.other_row = listing("qa-other-module", "QaSomeone/other-module", self.stranger)
        self.serial = 1
        write_bytes(os.path.join(self.seed, "public", "index.ltx"),
                    index_text(self.maintainer_key, self.serial, [self.other_row]).encode("utf-8"))
        write_bytes(os.path.join(self.seed, "mods", "qa-other-module.ltx"),
                    ("[mod]\nid      = qa-other-module\ngithub  = QaSomeone/other-module\nkey     = %s\n"
                     "version = 1.0.0\nwebsite = https://www.moddb.com/mods/qa-other-module\n" % self.stranger)
                    .encode("utf-8"))
        self.key_public = ""
        self.key_id = ""
        self.plain_mode = True
        self.package = ("", 0, "")  # name, size, sha256 of the published zip
        self.pid = 0  # the running editor's process

    def next_index(self, rows: list[str], withdrawn: tuple[str, ...] = ()) -> str:
        self.serial += 1
        return index_text(self.maintainer_key, self.serial, rows, withdrawn)

    def window_in_shot(self, pic: dict, text: str) -> None:
        """The window is in the picture: drawn into the editor's own window, which is what a screenshot mirrors - not
        into a platform window of its own (a dimmed editor with no dialog), and not into a frame the editor drew
        nothing of itself into (a minimized editor: one flat colour)."""
        outside = pic.get("outside")
        platform = platform_windows(self.pid)
        modal = pic.get("modal") or {}
        if not (pic.get("ok") is True and pic.get("png_base64")):
            why = "no picture: " + short(pic)
        elif not isinstance(outside, list) or SUBMIT_WINDOW in outside or SUBMIT_WINDOW in platform:
            why = "in a window of its own: outside=%s, platform windows=%s" % (outside, platform)
        elif modal.get("title") != SUBMIT_WINDOW:
            why = "the modal on top: " + short(pic.get("modal"))
        else:
            why = window_drawn(base64.b64decode(pic["png_base64"]), modal)
        check(not why, text, why)

    def window_shot(self, name: str) -> None:
        """The Submit to Mod Browser window with what it says now, for the operator - only when --shots names a folder."""
        if not self.shots:
            return
        # like the menu, show checks at once - and the check reads the submission too
        follow(mcp("mod_catalog", action="show"))
        self.window_in_shot(shot(self.shots, name), "shot %s holds the window" % name)
        mcp("mod_catalog", action="hide")

    # ------------------------------------------------------------------------------ run 1: an author
    def author(self) -> None:
        port = free_port()
        mock = Mock(port, self.work, "mock_author", "--catalog-seed", self.seed)
        editor = None
        try:
            # a lowered line sends even this small package the way a module over 32 MB goes
            editor = Editor(self.exe, self.copy, port, self.home, vt_variable=False,
                            extra_env={"XFINED_VT_DIRECT_LIMIT": "1000"})
            self.pid = editor.proc.pid
            phase("A", self.keys)
            phase("B", self.manifest)
            phase("C", self.content)
            phase("D", self.descriptor)
            phase("E", self.release, mock)
            phase("F", self.listing_check, mock)
            phase("G", self.virustotal, mock)
            phase("H", self.submission, mock)
            phase("W", self.withdrawal, mock)
        finally:
            if editor:
                # the QA credential goes with the run
                try:
                    mcp("mod_catalog", action="vt_key_forget")
                except (OSError, ValueError):
                    pass
                editor.close()
            mock.stop()
        phase("I", self.hygiene, mock, "author")

    # ------------------------------------------------------------------------------ A: the author key
    def keys(self) -> None:
        st = mcp("mod_catalog")
        key = st.get("key") or {}
        if not check(st.get("ok"), "A mod_catalog answers with the status", short(st)):
            raise Abort("mod_catalog does not answer - this editor build has no Submit to Mod Browser")
        if not check(same_path(key.get("path", ""), self.key_path),
                     "A the key would go to <XFINED_CATALOG_HOME>\\Keys\\author.xkey", short(st)):
            raise Abort("the editor does not keep its key in XFINED_CATALOG_HOME - stopped before it writes one elsewhere")
        check(st.get("ok") and key.get("exists") is False, "A a fresh catalog home has no author key", short(st))
        check(st.get("catalog") == CATALOG_REPO, "A the catalog is the one XFINED_CATALOG_REPO names", str(st.get("catalog")))
        check((st.get("account") or {}).get("signed_in") is True, "A the GitHub account of the environment is in effect",
              short(st.get("account")))
        check(job_state(st) == "idle" and not (st.get("job") or {}).get("pending") and
              (st.get("submission") or {}).get("number") == 0 and not (st.get("check") or {}).get("done"),
              "A no job, no check and no submission yet", short(st))

        st = mcp("mod_catalog", action="key_create", password=PASSWORD)
        key = st.get("key") or {}
        check(st.get("ok") and key.get("exists") is True and key.get("locked") is False,
              "A key_create makes a key that is open for the session", short(st))
        text = (read_bytes(self.key_path) or b"").decode("ascii", "replace")
        fields = {}
        try:
            fields = xc.parse_key_file_text(text)
        except ValueError as error:
            check(False, "A the key file is an xms-key 1 file", str(error))
        check({"public", "kdf", "cipher", "private", "mac"} <= set(fields) and
              fields.get("kdf", "").replace(" ", "").startswith("pbkdf2-sha256,600000,") and
              fields.get("cipher", "").replace(" ", "").startswith("hmac-sha256-ctr,"),
              "A the file is in the protected format: pbkdf2-sha256 600000, hmac-sha256-ctr, private, mac", text)
        try:
            d = xc.read_key_text(text, PASSWORD)
            point = xc.public_of(d)
            check(xc.key_text(point) == fields.get("public") == key.get("public"),
                  "A xms_catalog.py opens it with the password and its scalar gives the public key",
                  f"{fields.get('public')} vs {key.get('public')}")
            check(xc.key_id(point) == key.get("key_id"), "A the key id is the one xms_catalog.py computes",
                  f"{xc.key_id(point)} vs {key.get('key_id')}")
        except ValueError as error:
            check(False, "A xms_catalog.py opens the editor's key file with the password", str(error))
        self.key_public, self.key_id = key.get("public", ""), key.get("key_id", "")

        before = read_bytes(self.key_path)
        again = mcp("mod_catalog", action="key_create", password="something else")
        check(not again.get("ok") and read_bytes(self.key_path) == before, "A a key file is never overwritten", short(again))
        wrong = mcp("mod_catalog", action="key_unlock", password="not the password")
        check(not wrong.get("ok") and "wrong password" in str(wrong.get("error")), "A a wrong password is refused as such",
              short(wrong))

        # another copy of a key, written by the reference implementation
        other = os.path.join(self.work, "keys", "second.xkey")
        os.makedirs(os.path.dirname(other), exist_ok=True)
        d2 = xc.new_private()
        xc.write_key_file(other, d2, PASSWORD2)
        other_id = xc.key_id(xc.public_of(d2))
        st = mcp("mod_catalog", action="key_use", path=other)
        key = st.get("key") or {}
        check(st.get("ok") and same_path(key.get("path", ""), other) and key.get("locked") is True and
              key.get("key_id") == other_id, "A key_use takes another key file; a protected one starts locked", short(st))
        remembered = (read_bytes(os.path.join(self.home, "catalog.ltx")) or b"").decode("utf-8", "replace")
        check("second.xkey" in remembered, "A catalog.ltx remembers the key file in use", remembered)
        st = mcp("mod_catalog", action="key_unlock", password=PASSWORD2)
        key = st.get("key") or {}
        check(st.get("ok") and key.get("locked") is False and key.get("key_id") == other_id and
              key.get("public") == xc.key_text(xc.public_of(d2)),
              "A key_unlock opens a key file xms_catalog.py wrote: same id, same public key", short(st))

        st = mcp("mod_catalog", action="key_use", path=self.key_path)
        st = mcp("mod_catalog", action="key_unlock", password=PASSWORD) if st.get("ok") else st
        key = st.get("key") or {}
        check(st.get("ok") and key.get("locked") is False and key.get("key_id") == self.key_id and
              same_path(key.get("path", ""), self.key_path), "A back to the first key, unlocked", short(st))

    # ------------------------------------------------------------------------------ B: thumbnail and tags
    def manifest(self) -> None:
        current = mcp("mod_manifest")
        fields = {"id": MODULE_ID, "name": MODULE_NAME, "version": VERSION, "author": AUTHOR,
                  "description": DESCRIPTION, "website": WEBSITE, "github": GITHUB_REPO}
        if not current.get("target"):
            fields["target"] = "default"
        r = mcp("mod_set_manifest", **fields)
        check(r.get("ok"), "B the manifest takes id, name, version, author, website and github", short(r))
        self.plain_mode = not current.get("mode")

        thumb = os.path.join(self.copy, "thumbnail.dds")
        opaque = os.path.join(self.work, "pictures", "qa_opaque.png")
        write_bytes(opaque, opaque_picture())
        r = mcp("mod_set_manifest", thumbnail=opaque)
        m = mcp("mod_manifest")
        data = read_bytes(thumb)
        info = dds_info(data)
        check(r.get("ok") and m.get("thumbnail") == "thumbnail.dds" and data is not None,
              "B an opaque PNG (absolute path) becomes thumbnail.dds", f"{short(r)} {m.get('thumbnail')}")
        check(data is not None and cat.valid_thumbnail(data), "B the game's own test takes it as a thumbnail", str(info))
        check(info.get("fourcc") == "DXT1" and info.get("mips") == 9 and info.get("size") == 43832,
              "B 256x256 DXT1 with the full mip chain, exactly 43832 bytes", str(info))
        check(info.get("fourcc") == "DXT1" and top_level_is_green(data),
              "B the middle square is what it shows: the side strips are cut off", str(info))

        alpha = os.path.join(self.copy, "qa_alpha.png")
        write_bytes(alpha, transparent_picture())
        r = mcp("mod_set_manifest", thumbnail="qa_alpha.png")
        data = read_bytes(thumb)
        info = dds_info(data)
        os.remove(alpha)
        check(r.get("ok") and mcp("mod_manifest").get("thumbnail") == "thumbnail.dds",
              "B a PNG with transparency, relative to the project, is taken too", short(r))
        check(data is not None and cat.valid_thumbnail(data) and info.get("fourcc") == "DXT5" and
              info.get("mips") == 9 and info.get("size") == 87536, "B and becomes DXT5 with the full mip chain", str(info))

        junk = os.path.join(self.work, "pictures", "qa_not_a_picture.png")
        write_bytes(junk, b"this is not a picture\n")
        r = mcp("mod_set_manifest", thumbnail=junk)
        check(not r.get("ok") and r.get("error") and mcp("mod_manifest").get("thumbnail") == "thumbnail.dds" and
              read_bytes(thumb) == data, "B a picture that cannot be read is refused and changes nothing", short(r))

        # a DDS that already is a thumbnail is taken as it is - wherever the key ends up pointing
        ready = os.path.join(self.copy, "art", "qa_ready.dds")
        write_bytes(ready, data)
        r = mcp("mod_set_manifest", thumbnail="art/qa_ready.dds")
        named = str(mcp("mod_manifest").get("thumbnail", "")).replace("\\", "/")
        check(r.get("ok") and named in ("art/qa_ready.dds", "thumbnail.dds") and
              read_bytes(os.path.join(self.copy, *named.split("/"))) == data and read_bytes(ready) == data,
              "B a valid thumbnail DDS inside the project is used as is", f"{short(r)} {named}")
        r = mcp("mod_set_manifest", thumbnail="")
        m = mcp("mod_manifest")
        check(r.get("ok") and m.get("thumbnail") == "" and os.path.isfile(ready) and
              os.path.isfile(thumb) == (named != "thumbnail.dds"),
              "B clearing deletes thumbnail.dds only when that is the file the key named", f"{short(r)} {named}")
        shutil.rmtree(os.path.join(self.copy, "art"), ignore_errors=True)

        r = mcp("mod_set_manifest", thumbnail=opaque)
        check(r.get("ok") and os.path.isfile(thumb), "B the opaque picture again", short(r))
        r = mcp("mod_set_manifest", thumbnail="")
        m = mcp("mod_manifest")
        check(r.get("ok") and m.get("thumbnail") == "" and not os.path.exists(thumb),
              "B thumbnail '' clears the key and deletes the thumbnail.dds it named", f"{short(r)} {m.get('thumbnail')}")
        ltx = (read_bytes(os.path.join(self.copy, "mod.ltx")) or b"").decode("utf-8", "replace")
        check(not [line for line in ltx.splitlines() if line.strip().startswith("thumbnail")],
              "B mod.ltx no longer names a thumbnail", ltx)
        r = mcp("mod_set_manifest", thumbnail=opaque)
        check(r.get("ok") and dds_info(read_bytes(thumb)).get("fourcc") == "DXT1", "B and the opaque one for the release",
              short(r))

        r = mcp("mod_set_manifest", tags="story,quests")
        check(r.get("ok") and mcp("mod_manifest").get("tags") == ["story", "quests"], "B tags story,quests are kept in order",
              short(r))
        r = mcp("mod_set_manifest", tags="story,nonsense")
        check(not r.get("ok") and "unknown tag" in str(r.get("error")) and "nonsense" in str(r.get("error")),
              "B a word outside the vocabulary is refused and named", short(r))
        check(mcp("mod_manifest").get("tags") == ["story", "quests"], "B and the tags stay as they were")
        r = mcp("mod_set_manifest", tags="")
        check(r.get("ok") and mcp("mod_manifest").get("tags") == [], "B tags '' clears them", short(r))
        mcp("mod_set_manifest", tags="story,quests")

    # ------------------------------------------------------------------------------ C: content policy
    def content(self) -> None:
        planted = {
            os.path.join(self.copy, "bin", "evil.dll"): plant_program(),
            # a program under a name the catalog allows: the bytes are what counts
            os.path.join(self.copy, "qa_disguised", "program.txt"): plant_program(),
            os.path.join(self.copy, "gamedata", "scripts", "qa_bad.script"): BAD_SCRIPT,
            # somebody's key file dropped into the project: the build must never ship it
            os.path.join(self.copy, "qa_keys", "leaked.xkey"): b"xms-key 1\npublic  = p256:none\nprivate = none\n",
        }
        for path, data in planted.items():
            write_bytes(path, data)
        try:
            r = mcp("mod_package")
            content = r.get("catalog_content") or []
            blocked = [c.get("text", "") for c in content if c.get("level") == "block"]
            reviewed = [c.get("text", "") for c in content if c.get("level") == "review"]
            check(r.get("ok"), "C a module the catalog would refuse still packages", short(r))
            check(any("evil.dll" in t for t in blocked), "C a .dll is a blocking finding that names it", short(content))
            check(any("program.txt" in t for t in blocked), "C so is a program under an allowed extension", short(content))
            check(any("qa_bad.script" in t and "os.execute" in t for t in blocked),
                  "C os.execute in a script blocks, with the script named", short(content))
            check(any("qa_bad.script" in t and "loadstring" in t for t in reviewed),
                  "C loadstring of a computed string is for review", short(content))
            left = r.get("left_out") or []
            check(any("leaked.xkey" in t and "never ships" in t for t in left),
                  "C a key file inside the project is left out of the build, and said so", short(left))
            index_name = "%s-%s.files" % (MODULE_ID, VERSION)
            index = read_index(os.path.join(self.copy, "releases", VERSION, index_name))
            check(index and not [p for p in index if p.lower().endswith(".xkey")], "C and it is in no package",
                  short(sorted(index)))
            inside = mcp("mod_catalog", action="key_create", path=os.path.join(self.copy, "qa_keys", "made_here.xkey"))
            check(not inside.get("ok") and "inside a project" in str(inside.get("error", "")) and
                  not os.path.exists(os.path.join(self.copy, "qa_keys", "made_here.xkey")),
                  "C a key file is never made inside the project", short(inside))
        finally:
            for path in planted:
                if os.path.exists(path):
                    os.remove(path)
            for folder in ("bin", "qa_disguised", "qa_keys"):
                shutil.rmtree(os.path.join(self.copy, folder), ignore_errors=True)

    # ------------------------------------------------------------------------------ D: the signed descriptor
    def descriptor(self) -> None:
        r = mcp("mod_package")
        check(r.get("ok") and r.get("signed") is True and r.get("key_id") == self.key_id,
              "D the package is signed with the author key", short(r))
        check(not [c for c in r.get("catalog_content") or [] if c.get("level") == "block"],
              "D and the module holds nothing the catalog refuses", short(r.get("catalog_content")))
        folder = os.path.join(self.copy, "releases", VERSION)
        data = read_bytes(os.path.join(folder, MODULE_ID + ".update.ltx")) or b""
        try:
            _, key_text, sid, _ = xc.split_signed(data)
            check(key_text == self.key_public and sid == self.key_id, "D [signature] names the author key",
                  f"{key_text} {sid}")
            check(xc.verify_signed(data, xc.parse_key(self.key_public)), "D the signature verifies with xms_catalog.py")
        except ValueError as error:
            check(False, "D the descriptor carries a valid [signature]", str(error))
            return
        sections = parse_signed_ini(data)
        release, card = sections.get("release", {}), sections.get("card", {})
        check(release.get("id") == MODULE_ID and release.get("version") == VERSION, "D the descriptor is this release",
              str(release))
        check(card.get("name") == MODULE_NAME and card.get("author") == AUTHOR and card.get("description") == DESCRIPTION and
              card.get("website") == WEBSITE, "D [card] carries name, author, description and website", str(card))
        check(card.get("mode") == "plain" if self.plain_mode else str(card.get("mode", "")).startswith(("new ", "mode ")),
              "D [card] names the mode", str(card.get("mode")))
        check([t.strip() for t in card.get("tags", "").split(",")] == ["story", "quests"], "D [card] carries the tags",
              str(card.get("tags")))
        index = read_index(os.path.join(folder, "%s-%s.files" % (MODULE_ID, VERSION)))
        thumb_path, thumb_size, thumb_sha = card_file(card.get("thumbnail", ""))
        manifest_path, manifest_size, manifest_sha = card_file(card.get("manifest", ""))
        thumb = read_bytes(os.path.join(self.copy, "thumbnail.dds")) or b""
        check(thumb_path == "thumbnail.dds" and index.get("thumbnail.dds") == (thumb_size, thumb_sha) and
              (len(thumb), sha256(thumb)) == (thumb_size, thumb_sha),
              "D the card's thumbnail is the file the index lists, byte for byte", f"{card.get('thumbnail')} {index.get('thumbnail.dds')}")
        check(manifest_path == "mod.ltx" and index.get("mod.ltx") == (manifest_size, manifest_sha),
              "D the card's manifest is the mod.ltx the index lists", f"{card.get('manifest')} {index.get('mod.ltx')}")

    # ------------------------------------------------------------------------------ E: publish
    def release(self, mock: Mock) -> None:
        data = publish()
        check(data.get("state") == "done" and data.get("served") is True, "E the release is published and served", short(data))
        state = mock.state()
        done = [r for r in state.get("releases", []) if not r.get("draft")]
        assets = {a["name"]: a for a in done[0]["assets"]} if done else {}
        zips = sorted(n for n in assets if n.endswith(".zip"))
        folder = os.path.join(self.copy, "releases", VERSION)
        local = {n: sha256(read_bytes(os.path.join(folder, n)) or b"") for n in os.listdir(folder)} if os.path.isdir(folder) else {}
        check(local and local == {n: a.get("sha256") for n, a in assets.items()},
              "E GitHub holds exactly the packaged files", f"{sorted(local)} vs {sorted(assets)}")
        if zips:
            self.package = (zips[0], assets[zips[0]].get("size", 0), str(assets[zips[0]].get("sha256", "")).lower())
        check(len(zips) == 1, "E one package", str(zips))

    # ------------------------------------------------------------------------------ F: the listing requirements
    def listing_check(self, mock: Mock) -> None:
        # the scan is the author's: without a key the check blocks, so the key is there from here on (G takes it away)
        st = mcp("mod_catalog", action="vt_key", key=VT_KEY)
        check(st.get("ok") and st.get("vt_key") is True, "F the VirusTotal key is stored for the checks", short(st))
        data = job("check")
        c = data.get("check") or {}
        check(job_done(data, "check") and c.get("done") is True, "F check runs to the end", short(data.get("job")))
        check(not blocks(data) and c.get("blocking") is False, "F nothing blocks the published release",
              short(blocks(data)))
        name, _, sha = self.package
        check(levels(data, "virustotal") == ["required"] and
              any("a report is required: Submit scans it first" in t for t in texts(data, "virustotal")) and
              vt_entry(data, sha).get("state") == "unknown" and uploads_of(mock.vt(), sha) == 0,
              "F the check only looks the package up: VirusTotal does not know it - a requirement, never a note - and "
              "Submit scans it first", short(texts(data, "virustotal")) + " " + short(vt_entry(data, sha)))
        for code in ("release", "signature", "account", "catalog", "content"):
            check("ok" in levels(data, code) and "block" not in levels(data, code), "F %s is ok" % code,
                  short([f for f in findings(data) if f.get("code") == code]))
        check(levels(data, "website") == ["note"], "F a QA run never opens the website: a note",
              short([f for f in findings(data) if f.get("code") == "website"]))
        release = c.get("release") or {}
        check(release.get("version") == VERSION and release.get("signed") is True and release.get("key_id") == self.key_id,
              "F the latest release is 1.0.0, signed by this key", short(release))
        print("  findings: " + ", ".join("%s=%s" % (f.get("code"), f.get("level")) for f in findings(data)), flush=True)

        # a local release that is not the published one vouches for nothing
        descriptor = os.path.join(self.copy, "releases", VERSION, MODULE_ID + ".update.ltx")
        original = read_bytes(descriptor)
        try:
            write_bytes(descriptor, (original or b"") + b"; changed after publishing\r\n")
            data = job("check")
            check(levels(data, "content") and set(levels(data, "content")) <= {"note"},
                  "F a local release other than the published one leaves content a note",
                  short([f for f in findings(data) if f.get("code") == "content"]))
        finally:
            if original is not None:
                write_bytes(descriptor, original)

        # the id is taken - by somebody else's key
        mock.put_file("public/index.ltx", self.next_index([self.other_row, listing(MODULE_ID, GITHUB_REPO, self.stranger)]))
        data = job("check")
        check("block" in levels(data, "catalog") and (data.get("check") or {}).get("blocking") is True,
              "F the id listed under another key blocks", short([f for f in findings(data) if f.get("code") == "catalog"]))
        mock.put_file("public/index.ltx", self.next_index([self.other_row]))
        data = job("check")
        check("ok" in levels(data, "catalog") and not blocks(data), "F and the index as it was clears it", short(blocks(data)))

        # the window, for whoever looks from outside: like the menu, show checks at once
        data = mcp("mod_catalog", action="show")
        check(data.get("ok") and (data.get("job") or {}).get("kind") == "check",
              "F show opens the window and starts a check", short(data.get("job")))
        data = follow(data)
        check(job_done(data, "check") and not blocks(data), "F the check it started runs to the end", short(data.get("job")))
        # main() saved the window's place far below the editor, and ImGui gives a window outside the main viewport
        # a platform window of its own - one no screenshot of the editor holds
        self.window_in_shot(mcp("screenshot_editor"), "F the window opens inside the editor, and a screenshot holds it, "
                            "wherever the layout saved it")
        shot(self.shots, "catalog_window")
        hidden = mcp("mod_catalog", action="hide")
        check(hidden.get("ok") and not (hidden.get("job") or {}).get("pending"), "F hide closes it and starts nothing",
              short(hidden.get("job")))
        shot(self.shots, "catalog_hidden")

        # a minimized editor has no client area, and ImGui draws nothing of a minimized window: a screenshot of it
        # was one flat colour. It stays minimized from here on, so every later picture is taken of a minimized editor
        st = mcp("window_control", action="minimize")
        check(st.get("ok") and st.get("minimized") is True, "F the editor is minimized", short(st))
        follow(mcp("mod_catalog", action="show"))
        self.window_in_shot(mcp("screenshot_editor"), "F a screenshot of the minimized editor holds the window as well")
        shot(self.shots, "catalog_window_minimized")
        mcp("mod_catalog", action="hide")

    # ------------------------------------------------------------------------------ G: VirusTotal
    def virustotal(self, mock: Mock) -> None:
        name, size, sha = self.package
        st = mcp("mod_catalog", action="vt_key_forget")
        check(st.get("ok") and st.get("vt_key") is False, "G without a saved key or a variable there is no VirusTotal key",
              short(st))
        refused = job("vt")
        check(job_failed(refused), "G vt without a key is refused", short(refused))
        data = job("check")
        check("block" in levels(data, "virustotal") and (data.get("check") or {}).get("blocking") is True and
              any("virustotal.com" in t and "VirusTotal section" in t for t in texts(data, "virustotal")) and
              vt_state(data) == "no_key",
              "G without a key the check blocks, and says where a key comes from and where it goes",
              short(texts(data, "virustotal")))
        refused = mcp("mod_catalog", action="submit")
        check(job_failed(refused) and "no VirusTotal key" in job_error(refused) and not issues(mock.catalog()),
              "G submit without a key sends nothing", short(refused))
        st = mcp("mod_catalog", action="vt_key", key=VT_KEY)
        check(st.get("ok") and st.get("vt_key") is True and VT_KEY not in json.dumps(st),
              "G vt_key stores the key and echoes nothing", short(st))

        data = job("vt")
        entry = vt_entry(data, sha)
        check(job_done(data, "vt"), "G vt runs to the end", short(data.get("job")))
        check(entry.get("state") == "clean" and entry.get("engines", 0) >= 3 and entry.get("trusted_malicious") == 0 and
              vt_state(data) == "clean",
              "G the package VirusTotal did not know is uploaded, its report waited for: clean by the index's policy",
              short(entry))
        check(entry.get("name") == name and entry.get("size") == size and entry.get("url") == VT_PAGE + sha,
              "G the entry names the package, its size and its report page", short(entry))
        check(uploads_of(mock.vt(), sha) == 1, "G VirusTotal received the package once", short(mock.vt()))
        posts = [r.get("path", "") for r in mock.requests() if r.get("method") == "POST" and r.get("path", "").startswith("/vt/")]
        check(any(p.startswith("/vt/upload/") for p in posts) and not any(p.startswith("/vt/api/v3/files") for p in posts),
              "G over the direct line it went to an address VirusTotal handed out (upload_url)", str(posts))
        data = job("vt")
        check(vt_entry(data, sha).get("state") == "clean" and uploads_of(mock.vt(), sha) == 1,
              "G a package VirusTotal knows is never uploaded again", short(mock.vt()))

        mock.vt_seed({sha: {TRUSTED: "malicious"}})
        data = job("vt")
        entry = vt_entry(data, sha)
        check(entry.get("state") == "block" and entry.get("trusted_malicious", 0) >= 1 and TRUSTED in (entry.get("flagged") or []),
              "G one trusted engine calling it malicious is a block", short(entry))
        data = job("check")
        check("block" in levels(data, "virustotal"), "G check reports the VirusTotal block",
              short([f for f in findings(data) if f.get("code") == "virustotal"]))
        data = job("submit")
        check(job_failed(data) and "virustotal" in job_error(data).lower(), "G submit refuses and names the block",
              short(data.get("job")) + " " + str(data.get("error", "")))
        state = mock.catalog()
        check(not issues(state), "G nothing reached the catalog", short(state))

        mock.vt_seed({sha: {TRUSTED: "undetected", "EngineA": "undetected", "EngineB": "harmless", "EngineC": "undetected"}})
        data = job("vt")
        check(vt_entry(data, sha).get("state") == "clean", "G a clean report clears it again", short(vt_entry(data, sha)))

    # ------------------------------------------------------------------------------ H: the submission
    def submission(self, mock: Mock) -> None:
        name, size, sha = self.package
        path = "mods/%s.ltx" % MODULE_ID
        expected = submission_text(GITHUB_REPO, self.key_public)
        # none of these is the author's submission: a pull request under the title, another account's issue, a look-alike
        mock.issue("QaOwner", SUBMIT_TITLE, "A pull request, not a submission.", pull=True)
        mock.issue("QaSomeone", SUBMIT_TITLE, "Somebody else's.")
        mock.issue("QaOwner", SUBMIT_TITLE + " please", "Almost the title.")
        decoys = {i.get("number"): (i.get("body"), i.get("state")) for i in issues(mock.catalog())}

        # the catalog only looks packages up: the submission scans what VirusTotal does not know first
        mock.vt_forget(sha)
        data = job("submit")
        sub = data.get("submission") or {}
        log = mock.requests()
        uploaded_at = [i for i, r in enumerate(log) if r.get("method") == "POST" and r.get("path", "").startswith("/vt/upload/")]
        opened_at = [i for i, r in enumerate(log) if r.get("method") == "POST" and
                     urllib.parse.urlsplit(r.get("path", "")).path.endswith("/issues")]
        check(job_done(data, "submit") and uploads_of(mock.vt(), sha) == 1 and vt_entry(data, sha).get("state") == "clean" and
              uploaded_at and opened_at and max(uploaded_at) < min(opened_at),
              "H submit scans first: the package VirusTotal did not know goes up, its report comes back, then the issue",
              short(data.get("virustotal")) + " uploads=%d" % uploads_of(mock.vt(), sha))
        check(job_done(data, "submit") and sub.get("number", 0) > 0 and sub.get("state") == "open" and sub.get("url"),
              "H submit opens a submission", short(data.get("job")) + " " + short(sub))
        check(sub.get("verdict") == "" and sub.get("verdict_text") == "", "H no verdict before the catalog speaks",
              short(sub))
        number = sub.get("number", 0)
        state = mock.catalog()
        mine = submissions(state, "QaOwner")
        check(len(mine) == 1 and mine[0].get("number") == number and mine[0].get("state") == "open" and
              str(sub.get("url", "")).endswith("/issues/%d" % number),
              "H one issue '%s' by the author: the pull request, the stranger's issue and the look-alike are not it"
              % SUBMIT_TITLE,
              short([(i.get("number"), i.get("title"), (i.get("user") or {}).get("login")) for i in issues(state)]))
        body = str(issue_of(state, number).get("body") or "")
        check(body.split("\n")[:4] == ["<!-- xms-catalog submission -->", SUBMITTED_LINE, "", "```ini"],
              "H the body opens with the marker, a line for people, a blank line and the ini block", repr(body[:300]))
        check(ini_block(body) == expected, "H the ini block holds %s byte for byte" % path, repr(ini_block(body)))
        check(submission_fields(ini_block(body) or "") == {"id": MODULE_ID, "github": GITHUB_REPO, "key": self.key_public,
                                                           "version": VERSION, "website": WEBSITE},
              "H and the catalog's own parser reads exactly the [mod] fields from it", repr(ini_block(body)))
        check(VT_PAGE + sha in after_block(body) and "Checked by XFined Editor" in after_block(body) and
              ("`%s`: clean, " % name) in after_block(body),
              "H after the block: the scan's verdict with the VirusTotal report, and the editor's check",
              repr(after_block(body)[:400]))
        check(branch_file(state, CATALOG_REPO, "main", path) is None,
              "H the catalog's main is untouched: the catalog commits the file itself")
        check({i.get("number"): (i.get("body"), i.get("state")) for i in issues(state) if i.get("number") in decoys} ==
              decoys, "H and the decoys are left as they were")

        data = job("submission")
        sub = data.get("submission") or {}
        check(job_done(data, "submission") and sub.get("number") == number and sub.get("state") == "open" and
              sub.get("verdict") == "" and sub.get("verdict_text") == "",
              "H submission: no verdict while the catalog has not commented", short(sub))

        data = job("submit")
        state = mock.catalog()
        check(job_done(data, "submit") and (data.get("submission") or {}).get("number") == number and
              len(submissions(state, "QaOwner")) == 1 and len(api_requests(mock, "PATCH", "/issues/%d" % number)) == 1 and
              len(api_requests(mock, "POST", "/issues")) == 1,
              "H submitting again edits the same issue: one PATCH, no second issue", short(data.get("submission")))
        check(ini_block(str(issue_of(state, number).get("body") or "")) == expected,
              "H and the edited body still holds the file byte for byte")

        # the catalog's workflow speaks: CRLF, blank lines and loose spacing in the marker are the writer's business
        waiting = "Waiting: VirusTotal is scanning %s. The catalog looks again every half hour." % name
        mock.comment(number, "\r\n  <!--  xms-catalog   verdict = waiting  -->\r\n\r\n  %s  \r\n\r\n"
                             "## Report\r\n- VirusTotal: queued\r\n" % waiting)
        data = job("submission")
        sub = data.get("submission") or {}
        check(job_done(data, "submission") and sub.get("state") == "open" and sub.get("verdict") == "waiting" and
              sub.get("verdict_text") == waiting, "H waiting: the catalog's sentence while the submission is open",
              short(sub))
        self.window_shot("catalog_submission_waiting")

        # a comment that looks like a verdict is no verdict unless the catalog's workflow wrote it
        forged = "<!-- xms-catalog verdict=accepted -->\nAccepted: the module is listed.\n"
        for author in ("QaOwner", "github-actions", "dependabot[bot]"):
            mock.comment(number, forged, author=author)
        mock.comment(number, "The catalog looks again in half an hour.\n<!-- xms-catalog verdict=refused -->\n")
        data = job("submission")
        sub = data.get("submission") or {}
        check(sub.get("verdict") == "waiting" and sub.get("verdict_text") == waiting,
              "H a verdict by the author, a look-alike or another app is ignored, so is the workflow's comment without a "
              "marker", short(sub))

        # accepted a hundred comments later: the workflow commits the text of the ini block and closes the issue
        mock.comment(number, "+1", author="QaSomeone", count=100)
        accepted = "Accepted: the module is listed and appears in the Mod Browser within minutes."
        mock.comment(number, "<!-- xms-catalog verdict=accepted -->\n%s\n\n## Report\n- nothing blocks\n" % accepted)
        mock.put_file(path, ini_block(str(issue_of(mock.catalog(), number).get("body") or "")) or "")
        mock.close(number)
        data = job("submission")
        sub = data.get("submission") or {}
        check(job_done(data, "submission") and sub.get("state") == "closed" and sub.get("listed") is False,
              "H accepted and closed, but not listed until the catalog publishes its index", short(sub))
        check(sub.get("verdict") == "accepted" and sub.get("verdict_text") == accepted,
              "H accepted: the newest verdict, read past the first hundred comments", short(sub))
        self.window_shot("catalog_submission_accepted")
        mock.put_file("public/index.ltx", self.next_index([self.other_row, listing(MODULE_ID, GITHUB_REPO, self.key_public)]))
        data = job("submission")
        check((data.get("submission") or {}).get("listed") is True, "H listed once the index names the module with this key",
              short(data.get("submission")))

        # the catalog's main holds the very text of the ini block now: nothing to send
        data = job("submit")
        sub = data.get("submission") or {}
        state = mock.catalog()
        check(job_done(data, "submit") and "already holds" in str(sub.get("note", "")) and sub.get("number") == number and
              sub.get("state") == "closed" and sub.get("verdict") == "accepted" and
              len(submissions(state, "QaOwner")) == 1 and len(api_requests(mock, "POST", "/issues")) == 1 and
              len(api_requests(mock, "PATCH", "/issues/%d" % number)) == 1,
              "H submitted again unchanged: nothing is sent - main holds the ini block's text byte for byte",
              short(sub) + " " + str(len(submissions(state, "QaOwner"))))

    # ------------------------------------------------------------------------------ W: the withdrawal
    def withdrawal(self, mock: Mock) -> None:
        path = "mods/%s.ltx" % MODULE_ID
        listed_text = submission_text(GITHUB_REPO, self.key_public)
        mine_row = listing(MODULE_ID, GITHUB_REPO, self.key_public)
        data = job("submission")
        st = data.get("listing") or {}
        check(job_done(data, "submission") and st.get("state") == "listed" and st.get("github") == GITHUB_REPO and
              st.get("this_key") is True and (data.get("withdrawal") or {}).get("number") == 0,
              "W listed from its repository with this key, and nothing withdrawn yet", short(st))

        # another account's issue under the title is not the author's withdrawal
        mock.issue("QaSomeone", WITHDRAW_TITLE, "Somebody else's.")
        posts = len(api_requests(mock, "POST", "/issues"))
        data = job("withdraw")
        w = data.get("withdrawal") or {}
        state = mock.catalog()
        mine = submissions(state, "QaOwner", WITHDRAW_TITLE)
        check(job_done(data, "withdraw") and w.get("number", 0) > 0 and w.get("state") == "open" and w.get("url") and
              len(mine) == 1 and mine[0].get("number") == w.get("number") and
              len(api_requests(mock, "POST", "/issues")) == posts + 1,
              "W withdraw opens one issue '%s' from the owner's account" % WITHDRAW_TITLE, short(data.get("job")) + " " + short(w))
        number = w.get("number", 0)
        body = str(issue_of(state, number).get("body") or "")
        check(body.split("\n")[:4] == ["<!-- xms-catalog withdrawal -->", WITHDRAWN_LINE, "", "```ini"] and
              ini_block(body) == "[withdraw]\nid = %s\n" % MODULE_ID,
              "W the body opens with the marker, a line for people, a blank line and an ini block naming the module",
              repr(body[:300]))
        check(dict(cat.parse_ini(ini_block(body) or "").get("withdraw", [])) == {"id": MODULE_ID},
              "W and the catalog's own parser reads exactly the id from it", repr(ini_block(body)))
        check(branch_file(state, CATALOG_REPO, "main", path) == listed_text and w.get("verdict") == "",
              "W the catalog's main is untouched and there is no verdict yet: the catalog marks the file itself",
              repr(branch_file(state, CATALOG_REPO, "main", path)))

        data = job("withdraw")
        state = mock.catalog()
        check(job_done(data, "withdraw") and (data.get("withdrawal") or {}).get("number") == number and
              len(submissions(state, "QaOwner", WITHDRAW_TITLE)) == 1 and
              len(api_requests(mock, "PATCH", "/issues/%d" % number)) == 1 and
              len(api_requests(mock, "POST", "/issues")) == posts + 1,
              "W withdrawing again edits the open issue: one PATCH, no second issue", short(data.get("withdrawal")))

        # the catalog accepts: it marks the file withdrawn, closes the issue and signs the module into [withdrawn]
        accepted = ("Accepted: the module leaves the Mod Browser within minutes. Installed copies keep working, and "
                    "submitting it again lists it again.")
        mock.comment(number, "<!-- xms-catalog verdict=accepted -->\n%s\n" % accepted)
        mock.put_file(path, listed_text + "withdrawn = 2026-09-26\n")
        mock.close(number)
        mock.put_file("public/index.ltx", self.next_index([self.other_row], (mine_row,)))
        data = job("submission")
        w = data.get("withdrawal") or {}
        st = data.get("listing") or {}
        check(job_done(data, "submission") and w.get("state") == "closed" and w.get("verdict") == "accepted" and
              w.get("verdict_text") == accepted, "W the catalog's verdict: accepted and closed", short(w))
        check(st.get("state") == "withdrawn" and st.get("this_key") is True and
              (data.get("submission") or {}).get("listed") is False,
              "W the index has it under [withdrawn], still bound to this key, and no longer listed", short(st))
        self.window_shot("catalog_withdrawn")

        data = job("withdraw")
        w = data.get("withdrawal") or {}
        check(job_done(data, "withdraw") and "withdrawn" in str(w.get("note", "")) and w.get("number") == number and
              len(api_requests(mock, "POST", "/issues")) == posts + 1 and
              len(api_requests(mock, "PATCH", "/issues/%d" % number)) == 1,
              "W a withdrawn module is not withdrawn again: nothing is sent, a note says why", short(w))

        data = job("check")
        catalog_texts = texts(data, "catalog")
        check(levels(data, "catalog") == ["note"] and any("submitting lists it again" in t for t in catalog_texts) and
              not blocks(data), "W the check: withdrawn with this key is a note - submitting lists it again",
              short(catalog_texts))

        # submitting again lists it again: a new submission whose ini block is the plain listing
        data = job("submit")
        sub = data.get("submission") or {}
        state = mock.catalog()
        body = str(issue_of(state, sub.get("number", 0)).get("body") or "")
        check(job_done(data, "submit") and sub.get("number", 0) > number and sub.get("state") == "open" and
              ini_block(body) == listed_text and "withdrawn" not in (ini_block(body) or ""),
              "W submitting the withdrawn module opens a new submission holding the plain listing", short(sub))
        # the catalog lists it again
        mock.comment(sub.get("number", 0), "<!-- xms-catalog verdict=accepted -->\nAccepted: the module is listed and "
                                           "appears in the Mod Browser within minutes.\n")
        mock.put_file(path, listed_text)
        mock.close(sub.get("number", 0))
        mock.put_file("public/index.ltx", self.next_index([self.other_row, mine_row]))
        data = job("submission")
        check((data.get("listing") or {}).get("state") == "listed" and (data.get("submission") or {}).get("listed") is True,
              "W and once the catalog accepts, the index lists it again", short(data.get("listing")))

    # ------------------------------------------------------------------------------ I: hygiene
    def hygiene(self, mock: Mock, run: str) -> None:
        log = mock.requests()
        by_class = {}
        for r in log:
            by_class.setdefault(route_class(r.get("path", "")), []).append(r)
        token_out = [r["path"] for c in ("web", "catalog", "vt", "device") for r in by_class.get(c, []) if r.get("authorized")]
        check(log and not token_out, "I %s: the GitHub token never left for the web, the catalog's files or VirusTotal" % run,
              str(token_out))
        key_out = [r["path"] for c in ("api", "web", "catalog", "device") for r in by_class.get(c, []) if r.get("vt_key")]
        check(not key_out, "I %s: the VirusTotal key went to VirusTotal only" % run, str(key_out))
        vt = by_class.get("vt", [])
        if vt:
            check(all(r.get("vt_key") for r in vt), "I %s: and every VirusTotal request carried it" % run,
                  str([r["path"] for r in vt if not r.get("vt_key")]))
        check(bool(by_class.get("catalog")), "I %s: the catalog index came from its public base" % run)
        in_paths = [r["path"] for r in log if any(s in urllib.parse.unquote(r.get("path", "")) for s in SECRETS)]
        check(not in_paths, "I %s: no secret in any address" % run, str(in_paths))
        said = [a[:200] for a in answers if any(s in a for s in SECRETS)]
        check(not said, "I %s: no MCP answer holds a password, the VirusTotal key or the token" % run, str(said))
        leaked = leaks_in([os.path.join(REPO, "logs"), self.copy, self.home])
        check(not leaked, "I %s: no log, project file or key file holds one either" % run, str(leaked))

    # ------------------------------------------------------------------------------ run 2 / J: the maintainer
    def maintainer(self) -> None:
        port = free_port()
        # two VirusTotal requests a minute: the free API's refusal, met in seconds
        mock = Mock(port, self.work, "mock_maintainer", "--catalog-seed", self.seed, "--login", MAINTAINER,
                    "--vt-rate", "2", repo=MAINTAINER_REPO)
        editor = None
        try:
            editor = Editor(self.exe, self.copy, port, self.home, vt_variable=True,
                            extra_env={"XFINED_VT_RATE_WAIT_MS": "200"})
            self.pid = editor.proc.pid
            phase("J", self.own_listing, mock)
            phase("J", self.foreign_withdrawal, mock)
        finally:
            if editor:
                editor.close()
            mock.stop()
        phase("I", self.hygiene, mock, "maintainer")

    def own_listing(self, mock: Mock) -> None:
        st = mcp("mod_catalog")
        key = st.get("key") or {}
        check(st.get("ok") and key.get("exists") is True and key.get("locked") is True and key.get("key_id") == self.key_id,
              "J a new session finds the remembered key, locked", short(key))
        check(st.get("vt_key") is True, "J XFINED_VT_KEY is a VirusTotal key too", short(st))
        r = mcp("mod_set_manifest", github=MAINTAINER_REPO)
        check(r.get("ok"), "J the module moves to the maintainer's repository", short(r))
        st = mcp("mod_catalog", action="key_unlock", password=PASSWORD)
        check(st.get("ok") and (st.get("key") or {}).get("locked") is False, "J unlocked", short(st))
        data = publish()
        check(data.get("state") == "done", "J published to the maintainer's repository", short(data))

        # a package under 32 MB goes straight to /files; two requests a minute leave nothing for the report
        data = job("vt")
        entry = ((data.get("virustotal") or {}).get("packages") or [{}])[0]
        posts = [r.get("path", "") for r in mock.requests() if r.get("method") == "POST" and r.get("path", "").startswith("/vt/")]
        check(job_done(data, "vt") and any(p.startswith("/vt/api/v3/files") for p in posts),
              "J a small package is uploaded to /files directly", short(data.get("virustotal")) + " " + str(posts))
        refusals = [r for r in mock.requests() if r.get("status") == 429]
        check(entry.get("state") == "unavailable" and "4 requests a minute" in str(entry.get("error", "")) and
              len(refusals) >= 2, "J over the free API's rate: waited out a few times while the report is awaited, then "
              "reported, not hidden", short(entry) + " 429s: %d" % len(refusals))
        # only a package with a report leaves: the catalog takes one VirusTotal does not answer for as
        # a package nobody scanned
        data = job("submit")
        check(job_failed(data) and "not submitted" in job_error(data) and "submit again when VirusTotal answers" in
              job_error(data) and not submissions(mock.catalog(), MAINTAINER),
              "J VirusTotal not answering: nothing is sent, and the author is told to submit again",
              short(data.get("job")) + " " + job_error(data))
        mock.vt_seed({entry.get("sha256", ""): {"Kaspersky": "undetected", "Microsoft": "undetected"}})
        # the mock's rate window is a real minute (github_mock.VT_RATE_WINDOW): the next scan's lookup has
        # to find it open again
        time.sleep(61)
        data = job("submit")
        sub = data.get("submission") or {}
        check(job_done(data, "submit") and sub.get("number", 0) > 0 and sub.get("state") == "open",
              "J once VirusTotal has the report, submit opens a submission", short(data.get("job")) + " " + short(sub))
        check(vt_state(data) == "clean", "J and it went out on the report", vt_state(data))
        check((data.get("account") or {}).get("login") == MAINTAINER, "J the job learned the account's login",
              short(data.get("account")))
        number = sub.get("number", 0)
        state = mock.catalog()
        mine = submissions(state, MAINTAINER)
        check(len(mine) == 1 and ini_block(str(mine[0].get("body") or "")) == submission_text(MAINTAINER_REPO, self.key_public),
              "J the catalog's owner submits like anyone: an issue whose ini block names the owner's repository",
              short(mine))
        branches = sorted((repo_of(state, CATALOG_REPO).get("branches") or {}))
        check(branches == ["main"] and branch_file(state, CATALOG_REPO, "main", "mods/%s.ltx" % MODULE_ID) is None,
              "J and nothing is pushed to the catalog: no branch, no file", short(branches))

        # the catalog refuses: it closes the issue, and its newest verdict says why
        refused = "Refused: the release is not signed by the submitted key."
        mock.comment(number, "<!-- xms-catalog verdict=waiting -->\nWaiting: VirusTotal is scanning the package.\n")
        mock.comment(number, "<!-- xms-catalog verdict=refused -->\r\n%s\r\n\r\n### Blocking\r\n- signature\r\n" % refused)
        # not even the catalog's owner speaks for its workflow
        mock.comment(number, "<!-- xms-catalog verdict=accepted -->\nAccepted: overruled.\n", author=MAINTAINER)
        mock.close(number, reason="not_planned")
        data = job("submission")
        sub = data.get("submission") or {}
        check(job_done(data, "submission") and sub.get("number") == number and sub.get("state") == "closed" and
              sub.get("verdict") == "refused" and sub.get("verdict_text") == refused,
              "J refused: closed, and the workflow's newest verdict says why", short(sub))
        self.window_shot("catalog_submission_refused")
        if self.shots:
            # the window's check looked the package up too: the mock's two requests a minute are a real minute
            time.sleep(61)

        # a refused submission stays closed: submitting again opens a new one, without the old verdict
        data = job("submit")
        sub = data.get("submission") or {}
        state = mock.catalog()
        check(job_done(data, "submit") and sub.get("number", 0) > number and sub.get("state") == "open" and
              len(submissions(state, MAINTAINER)) == 2 and issue_of(state, number).get("state") == "closed",
              "J after the refusal submitting again opens a new issue; the refused one stays closed", short(sub))
        data = job("submission")
        again = data.get("submission") or {}
        check(job_done(data, "submission") and again.get("number") == sub.get("number") and again.get("verdict") == "" and
              again.get("verdict_text") == "", "J the new submission carries no verdict of the refused one", short(again))

    def foreign_withdrawal(self, mock: Mock) -> None:
        """A withdrawal leaves only from the account the listing came from, and only for a module the catalog holds."""
        issues_before = len(api_requests(mock, "POST", "/issues"))
        data = job("withdraw")
        check(job_failed(data) and "not in the Mod Browser catalog" in job_error(data) and
              not submissions(mock.catalog(), MAINTAINER, WITHDRAW_TITLE),
              "J withdrawing a module the catalog does not hold sends nothing", short(data.get("job")))
        mock.put_file("mods/%s.ltx" % MODULE_ID, submission_text(GITHUB_REPO, self.key_public))
        data = job("withdraw")
        check(job_failed(data) and "only QaOwner can withdraw it" in job_error(data) and
              not submissions(mock.catalog(), MAINTAINER, WITHDRAW_TITLE) and
              len(api_requests(mock, "POST", "/issues")) == issues_before,
              "J the catalog's owner cannot withdraw a module listed from another account: nothing is sent",
              short(data.get("job")) + " " + job_error(data))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--project", required=True, help="a project to COPY and run against")
    parser.add_argument("--editor", default=os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe"))
    parser.add_argument("--game-repo", default=r"D:\Games\Dead Air\DeadAir-x64",
                        help="the game repository: its tools/catalog holds the reference implementation")
    parser.add_argument("--shots", help="a folder for screenshots of the Submit to Mod Browser window")
    parser.add_argument("--keep", action="store_true", help="keep the throw-away copy, the catalog home and the mock logs")
    args = parser.parse_args()
    # an answer may carry text the console's code page lacks: printed escaped, never a crash of the harness
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(errors="backslashreplace")

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2
    tools =os.path.join(os.path.abspath(args.game_repo), "tools", "catalog")
    if not os.path.isfile(os.path.join(tools, "xms_catalog.py")) or not os.path.isfile(os.path.join(tools, "catalog.py")):
        print("no reference implementation in " + tools)
        return 2
    global xc, cat
    sys.path.insert(0, tools)
    import xms_catalog  # noqa: E402
    import catalog  # noqa: E402
    xc, cat = xms_catalog, catalog

    work = tempfile.mkdtemp(prefix="xfined_catalogproj_")
    copy = os.path.join(work, "QaModCatalog")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git", ".gitignore", "releases"))
    print("running against a copy: " + copy, flush=True)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = read_bytes(ini)
    real_home = os.path.join(documents_folder(), "XFined Editor")
    real_before = snapshot(real_home)
    # F wants the Submit to Mod Browser window in the picture although the layout saved it far below the editor (a
    # fresh checkout has no layout yet: the default one is planted). The editor rewrites its layout and preferences
    # when it closes, so both come back byte for byte after the last one has
    layout_backup = read_bytes(LAYOUT)
    prefs_backup = read_bytes(PREFS)
    write_bytes(LAYOUT, stale_layout(layout_backup if layout_backup is not None else read_bytes(LAYOUT_DEFAULT) or b"",
                                     SUBMIT_WINDOW, 3000))

    try:
        run = Run(args, work, copy)
        run.author()
        run.maintainer()
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            write_bytes(ini, ini_backup)
        for path, backup in ((LAYOUT, layout_backup), (PREFS, prefs_backup)):
            if backup is not None:
                write_bytes(path, backup)
            elif os.path.isfile(path):
                os.remove(path)
        check(read_bytes(LAYOUT) == layout_backup and read_bytes(PREFS) == prefs_backup,
              "Z the editor's layout and preferences files are back as they were")
        check(snapshot(real_home) == real_before, "Z the real Documents\\XFined Editor was not touched")
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print("kept: " + work, flush=True)

    passed = sum(1 for ok, _ in results if ok)
    print(f"MODCATALOG: DONE {passed}/{len(results)}", flush=True)
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
