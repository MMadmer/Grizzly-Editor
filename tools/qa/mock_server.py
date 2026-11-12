"""The Go mock of GitHub and VirusTotal (tools/qa/github_mock), built once per version of its sources.

The build is kept in Intermediate/qa/mock under a name that hashes the sources, so a suite that runs again - or
beside another - starts the mock at once instead of building it again; an edited source builds a new one.
"""
from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile

SOURCE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "github_mock")
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
KEPT = os.path.join(REPO, "Intermediate", "qa", "mock")

_exe = ""


def _sources_key() -> str:
    h = hashlib.sha256()
    for base, dirs, files in os.walk(SOURCE):
        dirs.sort()
        for name in sorted(files):
            if name.endswith((".go", ".mod", ".sum")):
                path = os.path.join(base, name)
                h.update(os.path.relpath(path, SOURCE).replace("\\", "/").encode())
                with open(path, "rb") as f:
                    h.update(f.read())
    return h.hexdigest()[:16]


def mock_exe() -> str:
    """The built mock's path: the kept build of these sources, made on the first call when there is none."""
    global _exe
    if not _exe:
        exe = os.path.join(KEPT, "github_mock-%s.exe" % _sources_key())
        if not os.path.isfile(exe):
            os.makedirs(KEPT, exist_ok=True)
            fd, tmp = tempfile.mkstemp(prefix="github_mock_", suffix=".exe", dir=KEPT)
            os.close(fd)
            # no VCS stamp: it would run git status over the whole repository
            subprocess.run(["go", "build", "-buildvcs=false", "-o", tmp, "."], cwd=SOURCE, check=True)
            try:
                # two suites built it at once: the first one's stays
                os.replace(tmp, exe)
            except OSError:
                os.remove(tmp)
            # the builds of older sources go; one a running suite still uses stays until next time
            for name in os.listdir(KEPT):
                if name.startswith("github_mock-") and os.path.join(KEPT, name) != exe:
                    try:
                        os.remove(os.path.join(KEPT, name))
                    except OSError:
                        pass
        _exe = exe
    return _exe
