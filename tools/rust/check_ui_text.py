#!/usr/bin/env python3
"""Fails when a surface shows text past the kit's language boundary (docs/UI_DESIGN.md, section 9).

The kit (`crates/gz-ui`) is the one way to draw UI, and its widgets take a `Tx`: a literal is the
editor's own words and is translated, data says so with `Tx::raw`. egui's own text calls take any
string and translate nothing, so outside the kit a surface must not call them:

    ui.label(..), ui.heading(..), ui.button(..), ui.small(..), ui.strong(..), ui.monospace(..),
    ui.checkbox(..), ui.selectable_label(..), ui.hyperlink..(..), ui.menu_button(..),
    egui::Label::new(..), egui::RichText::new(..) / RichText::new(..), egui::Window::new(..),
    egui::ComboBox, .hint_text(..), .on_hover_text(..), .on_disabled_hover_text(..)

(a `.hint_text` of a text already resolved - `i18n::t(ctx, ..)`, `tx.text(ctx)` - is the kit's), and a
painter's `.text(..)` with a string literal in it (data painted by hand - a node's name, a
number - is fine). The kit's replacements: `gz_ui::text::{label, plain, wrapped, prose, status,
paint}`, `gz_ui::TipExt::tip` / `tip_disabled`, `gz_ui::form::hint_field`, `gz_ui::i18n::t`.

    python tools/rust/check_ui_text.py           exit 1 on a hit, one line per hit

A line that carries `// ui-text: <why>` is allowed (a glyph, a key name, a test).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"
# the crates that draw UI, the kit excepted
UI = [p for p in sorted(CRATES.iterdir()) if p.name.startswith(("gz-ui-", "gz-app", "gz-particles"))]

CALLS = re.compile(
    r"\bui\.(?:label|heading|button|small|strong|monospace|checkbox|selectable_label|hyperlink\w*|menu_button)\("
    r"|\begui::Label::new\(|\bLabel::new\(|\bRichText::new\(|\begui::Window::new\(|\bComboBox\b"
    r"|\.hint_text\(|\.on_hover_text\(|\.on_disabled_hover_text\("
)
PAINT = re.compile(r"\.text\(")
# a text resolved through the kit: `i18n::t(ctx, ..)`, a Tx's `.text(ctx)` / `.get(ctx)`
RESOLVED = re.compile(r"\bi18n::t\(|\bt\(\s*(?:&?\w+\.)?ctx|\.text\(\s*&?\w*ctx|\.get\(\s*&?\w*ctx")


def call_text(src: str, at: int) -> str:
    """The text of a call from its opening parenthesis to the matching one."""
    depth = 0
    i = src.find("(", at)
    start = i
    in_str = False
    while 0 <= i < len(src):
        c = src[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return src[start : i + 1]
        i += 1
    return src[start:]


def code_of(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    # tests are not the window
    cut = re.search(r"^#\[cfg\(test\)\]\s*\n\s*mod \w+", text, re.M)
    return text[: cut.start()] if cut else text


def main() -> int:
    hits = []
    for crate in UI:
        for path in sorted((crate / "src").rglob("*.rs")):
            src = code_of(path)
            lines = src.splitlines()
            for m in CALLS.finditer(src):
                line_no = src.count("\n", 0, m.start()) + 1
                line = lines[line_no - 1] if line_no - 1 < len(lines) else ""
                if line.lstrip().startswith("//") or "ui-text:" in line:
                    continue
                # a hint already in the window's language is the kit's text
                if m.group(0) == ".hint_text(" and RESOLVED.search(call_text(src, m.start())):
                    continue
                hits.append(f"{path.relative_to(ROOT).as_posix()}:{line_no}: {line.strip()[:110]}")
            for m in PAINT.finditer(src):
                line_no = src.count("\n", 0, m.start()) + 1
                line = lines[line_no - 1] if line_no - 1 < len(lines) else ""
                if line.lstrip().startswith("//") or "ui-text:" in line:
                    continue
                args = call_text(src, m.start())
                # a literal among the arguments is the editor's own words
                if re.search(r'(?<![\w!])"[^"]*[A-Za-z]{2}[^"]*"', args) and not RESOLVED.search(args):
                    hits.append(f"{path.relative_to(ROOT).as_posix()}:{line_no}: {line.strip()[:110]}")
    for h in hits:
        print(h)
    if hits:
        print(f"{len(hits)} text calls past the kit's language boundary (see the docstring for the kit's own)")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
