"""Which editor the QA suites drive.

The suites check behaviour, not a binary, so the editor and its port are parameters. With nothing
set they drive the editor, Intermediate\\cargo\\debug\\Grizzly.exe, on port 28917 with a home of its
own (never the author's 28016 or settings):

    GRIZZLY_QA_EXE=<path>    another binary - Bin\\x64\\Release\\Grizzly.exe, the release build
    GRIZZLY_QA_PORT=<n>      another port
    GRIZZLY_QA_TARGET=cpp    the C++ editor of the tag cpp-editor-last (Bin\\x64\\Release\\LevelEditor.exe on
                             28016): for tools/qa/diff_editors.py's oracle records only - a suite refuses it
"""
from __future__ import annotations

import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

TARGET = (os.environ.get("GRIZZLY_QA_TARGET") or "rust").strip().lower()
if TARGET not in ("rust", "cpp"):
    print("REFUSED: GRIZZLY_QA_TARGET=%s names no editor (rust, the default, or cpp)" % TARGET, flush=True)
    sys.exit(2)
RUST = TARGET == "rust"
if not RUST and os.path.basename(sys.argv[0]).lower() != "diff_editors.py":
    print("REFUSED: the C++ editor left the repository at the cutover (tag cpp-editor-last); it is the oracle of "
          "tools/qa/diff_editors.py only, and no suite runs on it. Unset GRIZZLY_QA_TARGET.", flush=True)
    sys.exit(2)
PORT = int(os.environ.get("GRIZZLY_QA_PORT") or (28917 if RUST else 28016))
MCP = ("127.0.0.1", PORT)

# The Rust editor's home: no suite touches the author's project list or settings, and two
# editors never share a file.
HOME = (os.environ.get("GRIZZLY_QA_HOME") or tempfile.mkdtemp(prefix="grizzly_home_")) if RUST else ""

# What the editor needs on its command line to be this target.
ARGS: list[str] = ["-mcp_port", str(PORT), "-home", HOME] if RUST else []

# Every process a suite starts inherits the home: a Rust editor started without -home (a launcher's,
# a helper's) takes it from GRIZZLY_HOME and never writes into the repository root.
if RUST:
    os.environ["GRIZZLY_HOME"] = HOME

# The files of its own the editor could write in the repository root: a suite puts them back
# byte for byte (for the Rust editor they stay untouched - it writes into HOME).
KEPT = [os.path.join(REPO, name) for name in (
    ("_projects.ini", "grizzly_editor.ltx") if RUST else ("_projects.ini", "level.ini", "level_imgui.ini"))]


def exe() -> str:
    given = os.environ.get("GRIZZLY_QA_EXE")
    if given:
        return given
    if RUST:
        return os.path.join(REPO, "Intermediate", "cargo", "debug", "Grizzly.exe")
    return os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")


def projects_ini() -> str:
    """The project list the editor reads: its home's for the Rust editor, the repository's for the C++."""
    return os.path.join(HOME if RUST else REPO, "_projects.ini")


def log_glob() -> str:
    """The editor's own log files."""
    if RUST:
        return os.path.join(HOME, "logs", "grizzly_*.log")
    return os.path.join(REPO, "logs", "leveleditor_*.log")


def busy_text() -> str:
    return "an editor already answers on 127.0.0.1:%d - close it first" % PORT


# ---- where a suite's time goes ------------------------------------------------------------------
# Every suite imports this module first: its wall time is printed when it ends, split by the marks
# the suite sets ("copy", "editor start", ...; the rest is "checks").

import atexit
import fnmatch
import shutil
import sys
import time

_T0 = time.monotonic()
_LAST = [_T0]
_SPENT: dict[str, float] = {}


def mark(label: str) -> None:
    """Books the time since the last mark under `label`."""
    now = time.monotonic()
    _SPENT[label] = _SPENT.get(label, 0.0) + now - _LAST[0]
    _LAST[0] = now


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


def _drop_home() -> None:
    # a home this module made for the run goes with it; one named by GRIZZLY_QA_HOME is its owner's
    if RUST and not os.environ.get("GRIZZLY_QA_HOME"):
        remove_tree(HOME)


atexit.register(_drop_home)


def _report() -> None:
    now = time.monotonic()
    rest = now - _LAST[0]
    parts = dict(_SPENT)
    parts["checks"] = parts.get("checks", 0.0) + rest
    split = ", ".join("%s %.1f s" % (k, v) for k, v in parts.items())
    print("WALL %.1f s (%s)" % (now - _T0, split), flush=True)


atexit.register(_report)


TEMP = os.path.realpath(tempfile.gettempdir())

# What a trimmed copy leaves out besides `ignore`, by the folder of the copy: what a build does not ship (rawdata,
# design, docs) and the heavy textures - for a suite none of whose checks is about a package's size.
TRIMMED = {".": ("rawdata", "design", "docs"), "gamedata": ("textures",)}


def in_temp(path: str) -> bool:
    """Whether a path lies under the system temp folder (real paths: no link leads out of it)."""
    real = os.path.normcase(os.path.realpath(path))
    root = os.path.normcase(TEMP)
    return real.startswith(root.rstrip("\\/") + os.sep)


def guard_project(path: str) -> str:
    """The project a suite is about to open: it must be a copy under the system temp folder. A suite never opens a
    source project (D:\\XFinedProjects\\...) in place - it stops here with exit code 2 before any editor starts."""
    if not in_temp(path):
        print("REFUSED: %s is not under the system temp folder (%s): a suite opens only a copy it made "
              "(qa_target.copy_project)" % (path, TEMP), flush=True)
        sys.exit(2)
    return path


def copy_project(src: str, dst: str, ignore=(".git", "releases", "temp", "logs", "backup"), trimmed: bool = False) -> str:
    """THE way a suite copies a project: into `dst`, which must lie under the system temp folder; only what a suite
    needs (no .git, releases, temp, logs, backup unless it names another set; with `trimmed` also TRIMMED),
    project.ltx naming the copy's own folder, and the export target pointed into the suite's temp folder - never at
    the game. Answers `dst`, checked by guard_project."""
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
    pl = os.path.join(dst, "project.ltx")
    if os.path.isfile(pl):
        with open(pl, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        at = text.lower().find(src.lower())
        while at >= 0:
            text = text[:at] + dst + text[at + len(src):]
            at = text.lower().find(src.lower(), at + len(dst))
        away = os.path.join(os.path.dirname(dst), "export_target")
        lines = [l for l in text.splitlines() if not l.strip().lower().startswith("export_target")]
        if not any(l.strip().lower() == "[xms]" for l in lines):
            lines.append("[xms]")
        at = next(i for i, l in enumerate(lines) if l.strip().lower() == "[xms]")
        lines.insert(at + 1, "export_target=" + away)
        with open(pl, "w", encoding="utf-8", newline="") as f:
            f.write("\r\n".join(lines) + "\r\n")

    _SPENT["copy"] = _SPENT.get("copy", 0.0) + time.monotonic() - t
    _LAST[0] = time.monotonic()
    return dst
