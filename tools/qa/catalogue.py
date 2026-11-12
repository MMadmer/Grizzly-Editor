"""Is a binary built from the MCP catalogue that is here? (`crates/gz-mcp/tools.json`)

`Grizzly.exe` and `grizzly-mcp.exe` embed the catalogue when they are built (`include_str!`), so a
binary older than the file serves another set of tools: a suite that drives one fails on whatever
changed, far from the cause. Both answer `-catalogue` with one line,

    catalogue fnv1a64:<16 hex> tools:<n>

FNV-1a 64 of the embedded bytes and the count of its tools, worked out only when asked; this file
computes the same of the file and compares. A binary older than the flag answers nothing (the
editor is given `-dump_commands` too, so an old one prints its commands and exits instead of
opening; the bridge reads its standard input, which is empty) and counts as stale.

    python tools/qa/catalogue.py <exe>     # exit 0 when current, else 1 and the sentence

Used by tools/qa/test_mcp_bridge.py (the bridge) and tools/rust/check.sh (the editor's registry
step). Imports nothing of the QA harness: it starts no editor and makes no folder.
"""

from __future__ import annotations

import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TOOLS_JSON = os.path.join(REPO, "crates", "gz-mcp", "tools.json")


def fnv1a64(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def line_of(data: bytes) -> str:
    import json
    try:
        tools = len(json.loads(data.decode("utf-8")).get("tools") or [])
    except ValueError:
        tools = 0
    return "catalogue fnv1a64:%016x tools:%d" % (fnv1a64(data), tools)


def build_command(exe: str) -> str:
    """How the binary at `exe` is built again: Build.ps1 for Bin\\x64\\*, cg.sh with its profile otherwise."""
    path = os.path.normcase(os.path.abspath(exe))
    bridge = os.path.basename(path).startswith("grizzly-mcp")
    parts = path.split(os.sep)
    if "bin" in parts:
        return r"pwsh -NoProfile -File .\Build.ps1" + (" -Debug" if "debug" in parts else "")
    profile = parts[-2] if len(parts) >= 2 else "debug"
    flag = {"debug": "", "release": " --release"}.get(profile, " --profile " + profile)
    return "bash tools/rust/cg.sh build%s -p %s" % (flag, "gz-mcp-bridge" if bridge else "grizzly")


def stale(exe: str) -> str | None:
    """None when `exe` embeds the catalogue of this tree, else the sentence that says so and how to build it."""
    name = os.path.basename(exe)
    exe = os.path.abspath(exe)
    if not os.path.isfile(exe):
        return "%s is not built: %s" % (exe, build_command(exe))
    with open(TOOLS_JSON, "rb") as f:
        want = line_of(f.read())
    try:
        out = subprocess.run([exe, "-catalogue", "-dump_commands"], stdin=subprocess.DEVNULL,
                             capture_output=True, timeout=60, cwd=os.path.dirname(os.path.abspath(exe)),
                             creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0)).stdout
    except (OSError, subprocess.TimeoutExpired) as exc:
        return "%s did not answer -catalogue (%s): build it again - %s" % (name, exc, build_command(exe))
    have = next((l.strip() for l in out.decode("utf-8", "replace").splitlines() if l.startswith("catalogue ")), "")
    if have == want:
        return None
    count = lambda l: l.rsplit("tools:", 1)[-1] if "tools:" in l else "?"  # noqa: E731
    if not have:
        told = "is older than crates/gz-mcp/tools.json (it does not answer -catalogue)"
    elif count(have) == count(want):
        told = "was built from another crates/gz-mcp/tools.json (%s tools, other text)" % count(want)
    else:
        told = ("was built from another crates/gz-mcp/tools.json (it serves %s tools, the file lists %s)"
                % (count(have), count(want)))
    return "%s %s: build it again - %s" % (exe, told, build_command(exe))


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    why = stale(sys.argv[1])
    if why:
        print(why)
        return 1
    print("%s: built from this catalogue" % os.path.basename(sys.argv[1]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
