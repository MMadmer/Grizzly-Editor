"""Which editor the QA suites drive.

The suites check behaviour, not a binary, so the editor and its port are parameters:

    GRIZZLY_QA_TARGET=rust   the Rust editor: Intermediate\\cargo\\debug\\Grizzly.exe on port 28917
    GRIZZLY_QA_EXE=<path>    another binary of the chosen target
    GRIZZLY_QA_PORT=<n>      another port

With nothing set the target is the C++ editor (Bin\\x64\\Release\\LevelEditor.exe on 28016).
"""
from __future__ import annotations

import os

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

RUST = os.environ.get("GRIZZLY_QA_TARGET", "").strip().lower() == "rust"
PORT = int(os.environ.get("GRIZZLY_QA_PORT") or (28917 if RUST else 28016))
MCP = ("127.0.0.1", PORT)

# The Rust editor's home: no suite touches the author's project list or settings, and two
# editors never share a file.
HOME = (os.environ.get("GRIZZLY_QA_HOME") or tempfile.mkdtemp(prefix="grizzly_home_")) if RUST else ""

# What the editor needs on its command line to be this target.
ARGS: list[str] = ["-mcp_port", str(PORT), "-home", HOME] if RUST else []

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
