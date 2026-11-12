#!/usr/bin/env python3
"""Grizzly Editor MCP bridge - a launcher of grizzly-mcp.exe.

The bridge is the Rust binary `grizzly-mcp.exe` (crate gz-mcp-bridge, catalogue
crates/gz-mcp/tools.json); configure clients with it directly (tools/mcp/AI_SETUP.md). This file
stays so that a client set up with `python tools/mcp/grizzly_mcp.py` keeps working: it runs the
binary on the same stdin and stdout and exits with its exit code.

The binary is the first of: GRIZZLY_MCP_EXE, Bin/x64/Release (Build.ps1), Intermediate/cargo/release,
Intermediate/cargo/debug (tools/rust/cg.sh build -p gz-mcp-bridge).
"""
import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CANDIDATES = [
    os.environ.get("GRIZZLY_MCP_EXE", ""),
    os.path.join(REPO, "Bin", "x64", "Release", "grizzly-mcp.exe"),
    os.path.join(REPO, "Intermediate", "cargo", "release", "grizzly-mcp.exe"),
    os.path.join(REPO, "Intermediate", "cargo", "debug", "grizzly-mcp.exe"),
]


def main() -> int:
    exe = next((p for p in CANDIDATES if p and os.path.isfile(p)), None)
    if not exe:
        sys.stderr.write("grizzly-mcp.exe is not built. Build it with: pwsh -NoProfile -File Build.ps1 "
                         "(or set GRIZZLY_MCP_EXE). Looked in:\n  " + "\n  ".join(p for p in CANDIDATES if p) + "\n")
        return 1
    try:
        return subprocess.call([exe, *sys.argv[1:]])
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
