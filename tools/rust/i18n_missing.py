#!/usr/bin/env python3
"""Lists the string literals of the UI crates that read like the editor's own words and have no
entry in a catalog (docs/UI_DESIGN.md, section 9): what a translator works from.

    python tools/rust/i18n_missing.py [crate ...] [--all] [--lang ru] [--plain]

The catalogs are every `crates/**/src/**/<lang>.lua` and `*.<lang>.lua`; the kit's sheet (`-kitsheet`,
a developer's page that stays English) and the catalog's own code are left out. A literal is listed when it holds a word,
is no identifier, key, path, extension or format of a log line, and is not in a test. The list
is a heuristic both ways: a word it lists may be data (then it needs no entry and the call site
says `Tx::raw`), and a word built at run time is not listed at all - `ui.audit`'s `untranslated`
in the window is the proof. Without crates: every UI crate. `--all` lists literals with an entry
too, marked `=`. `--plain` prints only the texts, one per line, each once.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"
UI = ["gz-ui", "gz-app", "gz-ui-app", "gz-ui-content", "gz-ui-graph", "gz-ui-level", "gz-ui-mod", "gz-ui-pda",
      "gz-ui-previews", "gz-ui-release", "gz-ui-text", "gz-ui-viewport", "gz-ui-world", "gz-particles"]

LIT = re.compile(r'(?<![A-Za-z0-9_#])(?:r(#*)"(.*?)"\1|"((?:[^"\\]|\\.)*)")', re.S)
# what a literal that is no word of the window looks like
IDENT = re.compile(r"^[a-z0-9_.:\-/\\$#%*?<>=+\[\]{}|,]*$")
PATHLIKE = re.compile(r"^[^ ]*[/\\][^ ]*$|^\.[A-Za-z0-9]+$|^[A-Za-z0-9_]+\.[a-z0-9]{2,6}$")
NOT_UI_LINE = re.compile(
    r"\blog\.(?:info|warn|error|plain|debug)\(|\bFinding::|\bCmdError\b|\binvalid\(|\bjson!\(|\bSpec::|"
    r"\bpanic!\(|\bassert\w*!\(|\bunreachable!\(|\bexpect\(|\.example\(|#\[|\bserde\b|\bschemars\b|"
    r"\bdebug_assert\w*!\(|\.with\(\s*\"|make_persistent_id|Id::new\(|Id::unique\(|id_salt|\bpush_id\("
)


def catalog(lang: str) -> set[tuple[str, str]]:
    """Every (context, English) of the catalogs of a language."""
    out: set[tuple[str, str]] = set()
    entry = re.compile(r'\{\s*"((?:[^"\\]|\\.)*)"\s*,\s*"((?:[^"\\]|\\.)*)"\s*(?:,\s*ctx\s*=\s*"([^"]*)")?\s*\}')
    for f in CRATES.rglob(f"*{lang}.lua"):
        if "target" in f.parts:
            continue
        for m in entry.finditer(f.read_text(encoding="utf-8")):
            out.add((m.group(3) or "", unescape(m.group(1))))
    return out


def unescape(s: str) -> str:
    return s.replace('\\"', '"').replace("\\\\", "\\").replace("\\n", "\n")


def code_of(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    cut = re.search(r"^#\[cfg\(test\)\]\s*\n\s*mod \w+", text, re.M)
    return text[: cut.start()] if cut else text


def looks_like_words(s: str) -> bool:
    if not re.search(r"[A-Za-z]{2}", s):
        return False
    if IDENT.match(s) or PATHLIKE.match(s):
        return False
    # a format of a value alone: "{n}", "{:.1} ms"
    if re.fullmatch(r"[^A-Za-z]*(\{[^}]*\}[^A-Za-z]*)+[a-z]{0,3}", s):
        return False
    # a lower-case word with no space is a key ("quest", "level"), unless it is a sentence's end
    if " " not in s and s[:1].islower():
        return False
    return True


def main() -> int:
    # the names hold letters outside the console's code page (Rose Pine's accent)
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("crates", nargs="*")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--lang", default="ru")
    ap.add_argument("--plain", action="store_true")
    args = ap.parse_args()
    known = {en for _, en in catalog(args.lang)}
    crates = args.crates or UI
    seen: set[str] = set()
    for name in crates:
        root = CRATES / name / "src"
        for path in sorted(root.rglob("*.rs")):
            if path.name in ("kitsheet.rs", "tests.rs", "testkit.rs") or "i18n" in path.parts:
                continue
            src = code_of(path)
            lines = src.splitlines()
            for m in LIT.finditer(src):
                text = m.group(2) if m.group(2) is not None else unescape(m.group(3) or "")
                line_no = src.count("\n", 0, m.start()) + 1
                line = lines[line_no - 1] if line_no - 1 < len(lines) else ""
                if line.lstrip().startswith("//") or NOT_UI_LINE.search(line) or "format!(" in line and "tr!" not in line and "{" in text and text.strip().startswith("{"):
                    continue
                if not looks_like_words(text):
                    continue
                has = text in known
                if has and not args.all:
                    continue
                if args.plain:
                    if text not in seen:
                        seen.add(text)
                        print(text)
                    continue
                mark = "=" if has else " "
                print(f"{mark} {path.relative_to(ROOT).as_posix()}:{line_no}: {text!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
