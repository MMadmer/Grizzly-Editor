"""The QA client: the one module every suite imports.

It holds what a suite needs and nothing a suite has to repeat:

- the target: the debug build of the editor (`GRIZZLY_QA_EXE` names another binary), a port (`GRIZZLY_QA_PORT`, 28917
  unless set) and a home of its own (`GRIZZLY_QA_HOME`), so no suite ever touches the author's 28016, settings or
  project list;
- the socket client: `cmd`, `batch`, `gate`, and `call` for an editor on another port;
- the editor's lifecycle: `Editor` starts one on its own port and home, waits until it is ready, closes it over the
  socket (never killed: a killed editor leaves a project half written) and, when it closes, asks `perf.stalls` and
  fails the suite on a stall the suite did not declare (`Editor.stalls_allowed`);
- the project copy and its guard: a suite opens only a copy under the system temp folder (`copy_project`);
- what a suite prints: `check`, `run_phases`, the summary of `suite`, pictures (`shot`);
- the mock of GitHub and VirusTotal (`mock_exe`, `Mock`).

A suite is `qa.suite("TITLE", run)`: it parses `--project`, `--only`, `--shots` and `--keep`, copies the project,
calls `run(ctx)` and ends with `TITLE: DONE <passed>/<checks>`. The output lines are what `run_all.py` reads:
`PASS <text>`, `FAIL <text>  <- <detail>` and `SLOW <text>  <- <detail>` for a check made with `timing=True` (a
time measured beside other suites says how busy the machine was: a suite that failed on nothing else is measured
again alone).
"""
from __future__ import annotations

import argparse
import atexit
import base64
import contextlib
import fnmatch
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.request
from typing import Callable, Iterable

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
TEMP = os.path.realpath(tempfile.gettempdir())
PORT = int(os.environ.get("GRIZZLY_QA_PORT") or 28917)
LIMITED = os.path.join(REPO, "tools", "rust", "limited.py")

# The windows a QA editor is started with: hidden, which the editor treats as its minimised start (it draws a frame
# only when asked, and a resize never brings it up), or shown without taking the foreground, for a suite that times
# the window's frames.
SW_HIDDEN = 7  # SW_SHOWMINNOACTIVE
SW_SHOWN = 4  # SW_SHOWNOACTIVATE

# NOTHING A SUITE STARTS REACHES A REAL SERVICE OR THE AUTHOR'S OWN KEY. Importing this module makes the whole
# process tree of the suite a QA run: `GRIZZLY_QA_GITHUB` is the editor's `-qa_github` for every program started
# from here on - an editor, a commandlet, the twin for a command line, however its command line was written -
# and in a QA run the editor speaks to this computer alone (a missing address is nowhere, never the real
# service), reads no token but the QA ones and keeps its author keys inside its own home. Every service is on
# a port of the loopback nothing listens on; a suite that wants one starts the mock and points its editor at
# it. The variables a real token, key or key folder could come in by are taken out of the environment.
NOWHERE = "http://127.0.0.1:9"
LOOPBACK_ENV = {
    "GRIZZLY_QA_GITHUB": "1",
    "GRIZZLY_GITHUB_API": NOWHERE,
    "GRIZZLY_GITHUB_WEB": NOWHERE,
    "GRIZZLY_VT_API": NOWHERE,
    "GRIZZLY_CATALOG_BASE": NOWHERE + "/",
}
OWN_SECRETS = ("GRIZZLY_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN", "GRIZZLY_VT_KEY", "VT_API_KEY",
               "GRIZZLY_CATALOG_HOME", "GRIZZLY_CATALOG_REPO", "GRIZZLY_UPDATE_REPO", "GRIZZLY_UPDATE_ROOT")
os.environ.update(LOOPBACK_ENV)
for _name in OWN_SECRETS:
    os.environ.pop(_name, None)

# ---- where a suite's time goes ------------------------------------------------------------------------------------
# The wall time is printed when the suite ends, split by the marks it sets ("copy", "editor start", ...; the rest is
# "checks").

_T0 = time.monotonic()
_LAST = [_T0]
_SPENT: dict[str, float] = {}


def mark(label: str) -> None:
    """Books the time since the last mark under `label`."""
    now = time.monotonic()
    _SPENT[label] = _SPENT.get(label, 0.0) + now - _LAST[0]
    _LAST[0] = now


def _report_wall() -> None:
    if not _STARTED:
        return
    now = time.monotonic()
    parts = dict(_SPENT)
    parts["checks"] = parts.get("checks", 0.0) + now - _LAST[0]
    print("WALL %.1f s (%s)" % (now - _T0, ", ".join("%s %.1f s" % (k, v) for k, v in parts.items())), flush=True)


_STARTED = False

# ---- the target ---------------------------------------------------------------------------------------------------


def exe() -> str:
    """The editor the suites drive."""
    return os.environ.get("GRIZZLY_QA_EXE") or os.path.join(REPO, "Intermediate", "cargo", "debug", "Grizzly.exe")


_HOME: list[str] = []


def home() -> str:
    """The home of this suite's editors (their settings, project list and logs): made on the first ask and removed
    when the suite ends, unless `GRIZZLY_QA_HOME` names one - then it is its owner's. Every process the suite
    starts inherits it in `GRIZZLY_HOME`, so an editor started without `-home` never writes into the repository."""
    if not _HOME:
        given = os.environ.get("GRIZZLY_QA_HOME")
        _HOME.append(given or tempfile.mkdtemp(prefix="grizzly_home_"))
        os.environ["GRIZZLY_HOME"] = _HOME[0]
        # a suite's editor looks only at what its own home knew - the game a copied project was linked to
        # (`copy_project`) - never at the installed game
        os.environ.setdefault("GRIZZLY_GAME_SEARCH", "home")
        if not given:
            atexit.register(remove_tree, _HOME[0])
    return _HOME[0]


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def port_open(port: int = PORT) -> bool:
    try:
        socket.create_connection(("127.0.0.1", port), timeout=1).close()
        return True
    except OSError:
        return False


# ---- the socket client --------------------------------------------------------------------------------------------


def _exchange(port: int, payload: dict, timeout: float, escaped: bool = True) -> dict:
    with socket.create_connection(("127.0.0.1", port), timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload, ensure_ascii=escaped) + "\n").encode("utf-8"))
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


def call(port: int, name: str, /, *, timeout: float = 300.0, escaped: bool = True, **params) -> dict:
    """One command of the editor on `port`: its answer. A connection a request; `timeout` is how long the answer
    may take (a command that outlasts the editor's own limit answers `running`, which is no failure). Non-ASCII
    text goes escaped, the way the MCP bridge sends it, unless `escaped=False` sends raw UTF-8."""
    return _exchange(port, {"cmd": name, **params}, timeout, escaped)


def cmd(name: str, /, *, timeout: float = 300.0, escaped: bool = True, **params) -> dict:
    """One command of this suite's editor."""
    return _exchange(PORT, {"cmd": name, **params}, timeout, escaped)


def batch(items: Iterable[tuple[str, dict] | dict], *, port: int = 0, keep_going: bool = False, timeout: float = 300.0,
          door: str = "") -> dict:
    """Several commands in order in one request; the answer has `results`. An item is `(name, params)` or the
    request object itself."""
    requests = [it if isinstance(it, dict) else {"cmd": it[0], **it[1]} for it in items]
    payload: dict = {"batch": requests}
    if keep_going:
        payload["keep_going"] = True
    if door:
        payload["gate"] = door
    return _exchange(port or PORT, payload, timeout)


def gate(door: str, name: str, /, *, port: int = 0, timeout: float = 300.0, **params) -> dict:
    """A command through a door (`run`, `destroy` or `release`), as the bridge's tools send it: the editor refuses
    what is not that door's."""
    return _exchange(port or PORT, {"cmd": name, "gate": door, "args": params}, timeout)


def short(answer, limit: int = 400) -> str:
    """An answer as one line for a failure's detail, without a picture's bytes."""
    if isinstance(answer, dict):
        answer = {k: v for k, v in answer.items() if k != "png_base64"}
    return json.dumps(answer, ensure_ascii=False)[:limit]


def png_of(answer: dict) -> bytes:
    """The bytes of a picture command's answer; empty when it carries none."""
    return base64.b64decode(answer["png_base64"]) if answer.get("png_base64") else b""


def shot(folder: str | None, name: str, **params) -> dict:
    """A picture of the window, kept as a PNG in `folder` when there is one (`--shots` of a suite). Asking for it
    also draws a frame in a hidden editor, so a suite may call it without a folder; its answer is returned."""
    pic = cmd("shot.window", **params)
    data = png_of(pic)
    if folder and data:
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(data)
        print("screenshot: " + path, flush=True)
    return pic


# ---- what a suite prints ------------------------------------------------------------------------------------------

results: list[tuple[bool, str]] = []
slow: list[str] = []
_quiet = [False]


def check(ok, text: str, detail="", *, timing: bool = False) -> bool:
    """One thing the suite proves. `timing=True` marks a check about how long something took: a failure of it is
    printed as `SLOW` and `run_all.py` measures a suite that failed on nothing else again, alone."""
    if _quiet[0]:
        return bool(ok)
    results.append((bool(ok), text))
    if ok:
        print("PASS " + text, flush=True)
    else:
        if timing:
            slow.append(text)
        detail = detail if isinstance(detail, str) else json.dumps(detail, ensure_ascii=False, default=str)
        print(("SLOW " if timing else "FAIL ") + text + (("  <- " + detail[:600]) if detail else ""), flush=True)
    return bool(ok)


def stalls(text: str = "no UI pass stalled", *, clear: bool = False, port: int = 0) -> bool:
    """Checks that no UI pass or event held the UI thread over 100 ms since the last ask (a time: see `check`)."""
    answer = _exchange(port or PORT, {"cmd": "perf.stalls", "clear": clear}, 60)
    return check(answer.get("ok") and not answer.get("stalls"), text, short(answer, 500), timing=True)


@contextlib.contextmanager
def quiet(on: bool = True):
    """A stretch whose checks are not counted (a phase run for another's sake)."""
    was = _quiet[0]
    _quiet[0] = on
    try:
        yield
    finally:
        _quiet[0] = was


def run_phases(steps: dict[str, Callable], only: str = "", needs: dict[str, str] | None = None, *args) -> None:
    """The phases of a suite, in the order of `steps` (letter -> function taking `args`). `only` names the letters
    asked for ("A,C"); a phase another one needs (`needs`: letter -> letters) runs quietly - its checks are not
    counted - and a phase that is not needed does not run. A phase that raises is a FAIL of its own and the next
    one still runs; every phase prints its seconds."""
    asked = {c for c in only.upper() if c.isalnum()} or set(steps)
    wanted = set(asked)
    stack = list(asked)
    while stack:
        for need in (needs or {}).get(stack.pop(), ""):
            if need not in wanted:
                wanted.add(need)
                stack.append(need)
    for letter, step in steps.items():
        if letter not in wanted:
            continue
        _quiet[0] = letter not in asked
        t0 = time.monotonic()
        try:
            step(*args)
        except Exception as exc:  # noqa: BLE001 - the report is the point
            traceback.print_exc()
            check(False, "%s ran to its end" % letter, repr(exc))
        finally:
            print("TIME phase %s%s %.1f s" % (letter, " (setup)" if _quiet[0] else "", time.monotonic() - t0),
                  flush=True)
            _quiet[0] = False


def until(what: Callable[[], object], seconds: float = 10.0, step: float = 0.05):
    """Polls `what` until it answers something true; that answer, or the last one when the time is up."""
    deadline = time.monotonic() + seconds
    got = what()
    while not got and time.monotonic() < deadline:
        time.sleep(step)
        got = what()
    return got


# ---- files --------------------------------------------------------------------------------------------------------


def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def write(path: str, text: str) -> None:
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# IO_REPARSE_TAG name-surrogate bit: a junction or a symbolic link, not a placeholder of a cloud folder
_NAME_SURROGATE = 0x20000000


def unlink_links(path: str) -> list[str]:
    """Every junction and symbolic link under `path`, the link itself removed and never followed: what it points at
    (the game's archives, for one) stays. Answers the links removed."""
    gone: list[str] = []
    try:
        entries = list(os.scandir(path))
    except OSError:
        return gone
    for e in entries:
        try:
            st = e.stat(follow_symlinks=False)
        except OSError:
            continue
        if e.is_symlink() or getattr(st, "st_reparse_tag", 0) & _NAME_SURROGATE:
            try:
                (os.rmdir if e.is_dir() else os.unlink)(e.path)
            except OSError:
                os.unlink(e.path)
            gone.append(e.path)
        elif e.is_dir(follow_symlinks=False):
            gone += unlink_links(e.path)
    return gone


def remove_tree(path: str) -> bool:
    """Removes a suite's temp folder for good: read-only entries too (a copy's git objects, which a plain rmtree
    leaves behind), with a few retries for a file the editor or a scanner lets go of a moment late. Says so when
    something stayed."""
    if not path or not os.path.exists(path):
        return True

    def writable(func, p, _exc):
        try:
            os.chmod(p, 0o666)
            func(p)
        except OSError:
            pass

    # a link goes first, as a link: a delete never walks into what it points at
    unlink_links(path)
    for _ in range(20):
        shutil.rmtree(path, onerror=writable)
        if not os.path.exists(path):
            return True
        time.sleep(0.25)
    print("WARNING: %s could not be removed" % path, flush=True)
    return False


# ---- the project copy ---------------------------------------------------------------------------------------------

# What a trimmed copy leaves out besides `ignore`, by the folder of the copy: what a build does not ship (rawdata,
# design, docs) and the heavy textures - for a suite none of whose checks is about a package's size.
TRIMMED = {".": ("rawdata", "design", "docs"), "gamedata": ("textures",)}


def in_temp(path: str) -> bool:
    """Whether a path lies under the system temp folder (real paths: no link leads out of it)."""
    real = os.path.normcase(os.path.realpath(path))
    return real.startswith(os.path.normcase(TEMP).rstrip("\\/") + os.sep)


def guard_project(path: str) -> str:
    """The project a suite is about to open: it must be a copy under the system temp folder. A suite never opens a
    source project in place - it stops here with exit code 2 before any editor starts."""
    if not in_temp(path):
        print("REFUSED: %s is not under the system temp folder (%s): a suite opens only a copy it made "
              "(qa.copy_project)" % (path, TEMP), flush=True)
        sys.exit(2)
    return path


def project_id(folder: str) -> str:
    """The id of the project file (`*.grizzly`) at the root of `folder`; "" without one."""
    for name in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
        if name.lower().endswith(".grizzly") and os.path.isfile(os.path.join(folder, name)):
            m = re.search(r'\bid\s*=\s*"([0-9A-Fa-f-]{36})"', read(os.path.join(folder, name)))
            return m.group(1) if m else ""
    return ""


def _ltx_value(path: str, section: str, key: str) -> str:
    """A value of an LTX file of an editor's home ("" when it is not there)."""
    try:
        text = read(path)
    except OSError:
        return ""
    current = ""
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("[") and "]" in line:
            current = line[1:line.index("]")].strip().lower()
        elif current == section and "=" in line:
            k, v = line.split("=", 1)
            if k.strip().lower() == key:
                return v.strip().strip('"')
    return ""


def project_state(pid: str, home_folder: str = "") -> str:
    """The editor's state of the project `pid` in a home: <home>/projects/<id>.ltx."""
    return os.path.join(home_folder or home(), "projects", pid + ".ltx")


def copy_project(src: str, dst: str, ignore=(".git", "releases", "temp", "logs", "backup"), trimmed: bool = False,
                 home_folder: str = "") -> str:
    """THE way a suite copies a project: into `dst`, which must lie under the system temp folder; only what a suite
    needs (no .git, releases, temp, logs, backup unless it names another set; with `trimmed` also TRIMMED). What is
    this machine's goes into the suite's own home by the project's id: the game the author's editor reads for the
    project, and the export target pointed into the suite's temp folder - never at the game. An old project.ltx
    in the copy names the copy's own folder and the same export target. Answers `dst`, checked by guard_project."""
    guard_project(dst)
    t = time.monotonic()
    names = set(ignore)

    def skip(d: str, entries: list[str]) -> set[str]:
        out = {n for n in entries if any(fnmatch.fnmatch(n, p) for p in names)}
        if trimmed:
            rel = os.path.relpath(d, src).replace("\\", "/")
            out |= set(TRIMMED.get(rel, ())) & set(entries)
        return out

    shutil.copytree(src, dst, ignore=skip)
    away = os.path.join(os.path.dirname(dst), "export_target")
    pl = os.path.join(dst, "project.ltx")
    if os.path.isfile(pl):
        text = read(pl)
        at = text.lower().find(src.lower())
        while at >= 0:
            text = text[:at] + dst + text[at + len(src):]
            at = text.lower().find(src.lower(), at + len(dst))
        lines = [l for l in text.splitlines() if not l.strip().lower().startswith("export_target")]
        if not any(l.strip().lower() == "[xms]" for l in lines):
            lines.append("[xms]")
        at = next(i for i, l in enumerate(lines) if l.strip().lower() == "[xms]")
        lines.insert(at + 1, "export_target=" + away)
        write(pl, "\r\n".join(lines) + "\r\n")
    pid = project_id(dst)
    if pid:
        # the author's editor keeps its home at the repository's data root
        game = _ltx_value(os.path.join(REPO, "projects", pid + ".ltx"), "game", "root") or \
            _ltx_value(os.path.join(REPO, "machine.ltx"), "game", "root")
        lines = ["[project]", "root=" + dst, "[build]", "export_target=" + away]
        if game:
            lines += ["[game]", "root=" + game]
        state = project_state(pid, home_folder)
        os.makedirs(os.path.dirname(state), exist_ok=True)
        write(state, "\r\n".join(lines) + "\r\n")
    _SPENT["copy"] = _SPENT.get("copy", 0.0) + time.monotonic() - t
    _LAST[0] = time.monotonic()
    return dst


# ---- the editor ---------------------------------------------------------------------------------------------------


class Editor:
    """An editor of a suite, on its own port and home.

        ed = qa.Editor(project).start()
        ed.cmd("scene.info")
        ed.close()

    `project` is a copy (`copy_project`; anything else is refused); without one the editor starts on the Start
    page. `port` and `home_folder` default to the suite's own; a second editor of one suite names its own. `args`
    are more switches, `env` more variables (a mock's addresses, a QA token), `headless` without a window,
    `visible` with the window shown (not
    activated) instead of hidden. `ready` says when it is up: `"project"` (the default with a project: `state` says that project is open, the
    first frame is drawn and the scene it reopens loaded), `"ping"` (anything else), or a function.

    `close` ends it over the socket and, for a window, first asks `perf.stalls`: a UI pass or event that held the
    UI thread over 100 ms and that the suite did not declare (`stalls_allowed`) fails the suite."""

    def __init__(self, project: str | None = None, *, port: int = 0, home_folder: str = "", args: Iterable[str] = (),
                 env: dict | None = None, headless: bool = False, visible: bool = False,
                 ready: str | Callable[["Editor"], bool] = "", start_timeout: float = 600.0, stalls: bool = True,
                 log_output: bool = True):
        self.project = project
        self.port = port or PORT
        self.home = home_folder or home()
        self.extra = list(args)
        self.env = env or {}
        self.headless = headless
        self.visible = visible
        self.ready = ready or ("project" if project else "ping")
        self.start_timeout = start_timeout
        self._stalls = stalls and not headless
        self.watch_stalls = False
        self.log_output = log_output
        self.proc: subprocess.Popen | None = None
        self._isolated = False

    # -- the client of this editor
    def cmd(self, name: str, /, *, timeout: float = 300.0, **params) -> dict:
        return call(self.port, name, timeout=timeout, **params)

    def batch(self, items, **kw) -> dict:
        return batch(items, port=self.port, **kw)

    def gate(self, door: str, name: str, /, **params) -> dict:
        return gate(door, name, port=self.port, **params)

    def up(self) -> bool:
        return port_open(self.port)

    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    # -- lifecycle
    def command_line(self) -> list[str]:
        line = [exe(), "-mcp_port", str(self.port), "-home", self.home, "-nodlg", "-qa_github"]
        if self.headless:
            line.append("-headless")
        if self.project:
            line += ["-project", guard_project(self.project)]
        return line + self.extra

    def launch(self, *, check_port: bool = True) -> "Editor":
        """Starts the process and returns at once: `start` is this and the wait until the editor is ready. A suite
        that wants the process alone (a second start that must hand its project over and end) calls this."""
        binary = exe()
        self.watch_stalls = self._stalls
        if not os.path.isfile(binary):
            raise RuntimeError("build the editor first: " + binary)
        if check_port and self.up():
            raise RuntimeError("an editor already answers on 127.0.0.1:%d - close it first" % self.port)
        env = dict(os.environ)
        env.update(LOOPBACK_ENV)
        for name, value in self.env.items():
            # None takes a variable away: an editor must not meet the author's own token
            if value is None:
                env.pop(name, None)
            else:
                env[name] = value
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = SW_SHOWN if self.visible else SW_HIDDEN
        quiet = {} if self.log_output else {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
        self.proc = subprocess.Popen(self.command_line(), cwd=os.path.dirname(binary), startupinfo=startup, env=env,
                                     **quiet)
        return self

    def start(self) -> "Editor":
        self.launch()
        t0 = time.monotonic()
        deadline = t0 + self.start_timeout
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup (code %s)" % self.proc.returncode)
            if self._is_ready():
                break
            time.sleep(0.05)
        else:
            raise RuntimeError("the editor never became ready")
        if self.ready == "project" and not self.headless:
            self.settle()
        _SPENT["editor start"] = _SPENT.get("editor start", 0.0) + time.monotonic() - t0
        _LAST[0] = time.monotonic()
        return self

    def _is_ready(self) -> bool:
        try:
            self._kept_to_this_computer()
            if callable(self.ready):
                return bool(self.ready(self))
            if self.ready == "project":
                return self._project_open()
            return bool(self.cmd("ping", timeout=30).get("ok"))
        except (OSError, ValueError):
            return False

    def _kept_to_this_computer(self) -> None:
        """The editor says it is a QA run before a suite asks it anything else: one that could reach GitHub,
        VirusTotal, the catalog or the author's key is closed at once and the suite stops."""
        if self._isolated:
            return
        ping = self.cmd("ping", timeout=30)
        if not ping.get("ok"):
            raise OSError("the editor does not answer yet")
        if ping.get("qa") is not True:
            try:
                self.cmd("app.quit", discard=True, timeout=30)
            except (OSError, ValueError):
                pass
            raise RuntimeError("this editor is not a QA run (ping answers no `qa`): it could reach a real service; "
                               "the suite stops")
        self._isolated = True

    def _project_open(self) -> bool:
        """The editor answers and `state` says the project it was started on is open: the control endpoint comes up
        before the project has finished opening."""
        st = self.cmd("state", timeout=30)
        project = st.get("project") if st.get("ok") else None
        if not isinstance(project, dict) or not project.get("folder"):
            return False
        real = lambda p: os.path.normcase(os.path.realpath(p))  # noqa: E731
        return real(project["folder"]) == real(self.project or "")

    def settle(self) -> None:
        """A hidden editor draws only when asked, and its first frame reopens the project's last scene - a blocking
        load. That frame is drawn here and the load runs out: the checks read the editor, not a load's busy frames."""
        self.cmd("ui.list")
        deadline = time.monotonic() + self.start_timeout
        while self.cmd("ui.tasks").get("active") and time.monotonic() < deadline:
            time.sleep(0.05)

    def stalls(self, text: str = "no UI pass stalled", *, clear: bool = False) -> bool:
        """Checks that no UI pass or event held the UI thread over 100 ms since the last ask."""
        return stalls(text, clear=clear, port=self.port)

    @contextlib.contextmanager
    def stalls_allowed(self, why: str):
        """A stretch of a suite that is allowed to hold the UI thread (`why` names the reason): the stalls it
        makes are dropped when it ends, so `close` does not count them."""
        self.cmd("perf.stalls", clear=True)
        try:
            yield
        finally:
            dropped = self.cmd("perf.stalls", clear=True).get("stalls") or []
            if dropped:
                print("note: %d stall(s) allowed (%s)" % (len(dropped), why), flush=True)

    def finish(self) -> None:
        """The last check of a run with a window: perf.stalls is empty."""
        if self.watch_stalls and (self.alive() or self.up()):
            try:
                self.stalls()
            except OSError:
                pass
            self.watch_stalls = False

    def close(self, *, discard: bool = True, wait: float = 120.0) -> int | None:
        """Ends the editor over the socket; its exit code. Never killed."""
        self.finish()
        restarted = self.proc is not None and self.proc.poll() is not None
        # whatever answers on this port is this editor, also when it is a restart the editor made of itself
        if self.alive() or self.up():
            try:
                self.cmd("app.quit", discard=discard, timeout=60)
            except (OSError, ValueError):
                pass
        code = None
        if self.proc:
            try:
                code = self.proc.wait(timeout=wait)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)
        # a restart the editor made of itself is a process this object never started: wait for the port to be free
        # and for the process that held it to be gone
        if restarted and until(lambda: not self.up(), 30):
            until(lambda: not self._running_on_port(), 30, 0.25)
        return code

    def _running_on_port(self) -> bool:
        """Whether an editor started with this port is still a process (found by its command line)."""
        probe = ("Get-CimInstance Win32_Process -Filter \"Name='Grizzly.exe'\" | "
                 "Where-Object { $_.CommandLine -like '*-mcp_port %d *' } | Select-Object -First 1 | "
                 "ForEach-Object { $_.ProcessId }" % self.port)
        out = subprocess.run(["powershell", "-NoProfile", "-Command", probe], capture_output=True, text=True,
                             timeout=60).stdout
        return bool(out.strip())

    def __enter__(self) -> "Editor":
        return self.start()

    def __exit__(self, *_exc) -> None:
        self.close()


# ---- the mock of GitHub and VirusTotal ----------------------------------------------------------------------------

MOCK_SOURCE = os.path.join(HERE, "github_mock")
MOCK_KEPT = os.path.join(REPO, "Intermediate", "qa", "mock")
_mock_built: list[str] = []


def _mock_key() -> str:
    h = hashlib.sha256()
    for base, dirs, files in os.walk(MOCK_SOURCE):
        dirs.sort()
        for name in sorted(files):
            if name.endswith((".go", ".mod", ".sum")):
                path = os.path.join(base, name)
                h.update(os.path.relpath(path, MOCK_SOURCE).replace("\\", "/").encode())
                with open(path, "rb") as f:
                    h.update(f.read())
    return h.hexdigest()[:16]


def mock_exe() -> str:
    """The built mock's path: the kept build of its sources (tools/qa/github_mock, Go), made on the first call when
    there is none. An edited source builds a new one; two suites that build at once keep the first."""
    if not _mock_built:
        binary = os.path.join(MOCK_KEPT, "github_mock-%s.exe" % _mock_key())
        if not os.path.isfile(binary):
            os.makedirs(MOCK_KEPT, exist_ok=True)
            fd, tmp = tempfile.mkstemp(prefix="github_mock_", suffix=".exe", dir=MOCK_KEPT)
            os.close(fd)
            # no VCS stamp: it would run git status over the whole repository
            subprocess.run([sys.executable, LIMITED, "300", "go", "build", "-buildvcs=false", "-o", tmp, "."],
                           cwd=MOCK_SOURCE, check=True)
            try:
                os.replace(tmp, binary)
            except OSError:
                os.remove(tmp)
            for name in os.listdir(MOCK_KEPT):
                if name.startswith("github_mock-") and os.path.join(MOCK_KEPT, name) != binary:
                    try:
                        os.remove(os.path.join(MOCK_KEPT, name))
                    except OSError:
                        pass
        _mock_built.append(binary)
    return _mock_built[0]


class Mock:
    """The mock server on a port of its own: `Mock(port, "--token", ..., "--repo", ...)` takes the mock's switches
    as they are. `get` / `post` read its JSON; `state()` is what it holds. `stop` ends it."""

    def __init__(self, port: int = 0, *flags: str, stderr=subprocess.DEVNULL):
        self.port = port or free_port()
        self.base = "http://127.0.0.1:%d" % self.port
        self.flags = ["--port", str(self.port), *flags]
        self.proc = subprocess.Popen([mock_exe(), *self.flags], stdout=subprocess.PIPE, stderr=stderr, text=True)
        line = self.proc.stdout.readline()
        if not line or json.loads(line)["port"] != self.port:
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

    def stop(self) -> None:
        # a graceful shutdown would wait for the editor's idle connections: the state was read already
        self.proc.kill()
        self.proc.wait(timeout=10)


# ---- a suite ------------------------------------------------------------------------------------------------------


class Run:
    """What `suite` hands to a suite's `run`: the copy of the project, the folder it is in, and the options."""

    def __init__(self, project: str, source: str, work: str, args: argparse.Namespace):
        # a suite writes only into its own folder: absolute, under the system temp folder, never the working folder
        if not os.path.isabs(work) or not in_temp(work):
            print("REFUSED: the work folder %r is not an absolute path under the system temp folder (%s)" % (work, TEMP),
                  flush=True)
            sys.exit(2)
        self.project = project
        self.source = source
        self.work = work
        self.args = args
        self.shots = getattr(args, "shots", None)
        self.only = getattr(args, "only", "") or ""


def suite(title: str, run: Callable[[Run], None], *, doc: str = "", add_args: Callable | None = None,
          ignore=(".git", "releases", "temp", "logs", "backup"), trimmed: bool = False, project: bool = True,
          default_project: str = "", requires: Iterable[str] = (), phases: str = "", copy_name: str = "Qa") -> int:
    """The main of a suite: reads `--project` (a project to COPY; `project=False` for a suite that needs none),
    `--only`, `--shots` and `--keep` (and what `add_args(parser)` adds), copies the project, calls `run(Run)` and
    ends with `<title>: DONE <passed>/<checks>`. The copy's folder is `copy_name` (a project file is named after
    it). `requires` are paths the project must hold (the suite says so and
    stops with 2 otherwise). The exit code is 0 when every check passed."""
    global _STARTED
    _STARTED = True
    atexit.register(_report_wall)
    ap = argparse.ArgumentParser(description=doc or None, formatter_class=argparse.RawDescriptionHelpFormatter)
    if project:
        ap.add_argument("--project", required=not default_project, default=default_project,
                        help="a project to COPY and run against")
        ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    ap.add_argument("--only", default="", help="phase letters, e.g. A,C%s" % (" (%s)" % phases if phases else ""))
    ap.add_argument("--shots", help="a folder for the pictures")
    if add_args:
        add_args(ap)
    args = ap.parse_args()

    src = copy = ""
    if project:
        src = os.path.abspath(args.project)
        if not os.path.isdir(src):
            print("no such project: " + src)
            return 2
        for rel in requires:
            if not os.path.exists(os.path.join(src, rel)):
                print("the project has no %s, which this suite needs: %s" % (rel, src))
                return 2
    # a folder of the suite's own: its copy and whatever else it makes goes with it
    work = os.path.realpath(tempfile.mkdtemp(prefix="grizzly_qa_"))
    try:
        if project:
            copy = os.path.join(work, copy_name)
            copy_project(src, copy, ignore=ignore, trimmed=trimmed)
            print("running against a copy: " + copy, flush=True)
        run(Run(copy, src, work, args))
    except SystemExit:
        raise
    except Exception as exc:  # noqa: BLE001 - the report is the point
        traceback.print_exc()
        check(False, "the run finished", repr(exc))
    finally:
        if not getattr(args, "keep", False):
            remove_tree(work)
    ok = sum(1 for good, _ in results if good)
    print("%s: DONE %d/%d%s" % (title, ok, len(results), (" (%d timing)" % len(slow)) if slow else ""), flush=True)
    return 0 if ok == len(results) else 1
