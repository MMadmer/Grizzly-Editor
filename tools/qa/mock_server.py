"""Builds the Go mock of GitHub and VirusTotal (tools/qa/github_mock) once per test run."""
from __future__ import annotations

import atexit
import os
import shutil
import subprocess
import tempfile

SOURCE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "github_mock")

_exe = ""


def mock_exe() -> str:
    """The built mock's path; the first call builds it into a temporary folder removed when the process exits."""
    global _exe
    if not _exe:
        folder = tempfile.mkdtemp(prefix="github_mock_")
        atexit.register(shutil.rmtree, folder, ignore_errors=True)
        exe = os.path.join(folder, "github_mock.exe")
        # no VCS stamp: it would run git status over the whole repository
        subprocess.run(["go", "build", "-buildvcs=false", "-o", exe, "."], cwd=SOURCE, check=True)
        _exe = exe
    return _exe
