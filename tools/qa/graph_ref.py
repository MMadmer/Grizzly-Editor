"""The node kinds as `asset.reference kind=quest|behaviour` lists them, read into dicts.

The reference is text for a reader (docs/nq/NQ_FORMAT.md and the linked game's kinds): a kind is
a line "- `id` - Title", then "  params: name: type, required; ...", "  each row: ...",
"  pins: a (what it means), b", "  summary: ...". QA reads it back with `kinds(text)`.
"""
from __future__ import annotations

import re


def _params(body: str) -> list[dict]:
    out = []
    for part in body.split("; "):
        name, _, rest = part.partition(": ")
        if not name.strip():
            continue
        p = {"name": name.strip(), "type": rest.split(" (")[0].split(",")[0].strip()}
        if ", required" in rest:
            p["required"] = True
        d = re.search(r", default (.+?)(?:, [-\d.]+\.\.[-\d.]+)?$", rest)
        if d:
            p["default"] = d.group(1)
        out.append(p)
    return out


def kinds(text: str) -> dict[str, dict]:
    """Every kind of the reference by its id: params, rows, pins (their meanings left out), summary."""
    out: dict[str, dict] = {}
    cur = None
    for line in text.splitlines():
        m = re.match(r"^- `([^`]+)`", line)
        if m:
            cur = out.setdefault(m.group(1), {"id": m.group(1), "params": [], "rows": [], "pins": [], "summary": []})
            continue
        if cur is None:
            continue
        if line.startswith("  params: "):
            cur["params"] = _params(line[len("  params: "):])
        elif line.startswith("  each row: "):
            cur["rows"] = _params(line[len("  each row: "):])
        elif line.startswith("  pins: "):
            body = re.sub(r"\s*\([^)]*\)", "", line[len("  pins: "):])
            cur["pins"] = [x.strip() for x in body.split(",") if x.strip()]
        elif line.startswith("  summary: "):
            cur["summary"] = [x.strip() for x in line[len("  summary: "):].split(",") if x.strip()]
    return out
