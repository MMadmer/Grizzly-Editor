#!/usr/bin/env python3
"""Raw pixels and colour literals in the editor's UI code (docs/UI_DESIGN.md, section 1.4 and section 2).

Every size in the UI is a logical point that goes through XUI::Pt / XUI::Em (DPI enters in one function), and every
colour is a theme token (GrizzlyTheme::U32 / Vec4, XUI::ToneColor). A literal pixel size is right at one scale only and
a literal colour ignores the theme; this finds both. Needs no build and no editor.

What it flags, in code with its comments and strings taken out:
  colour  IM_COL32(...) and IM_COL32_WHITE / _BLACK; an ImVec4 of four number literals (not all zero); a hex colour
          (0xRRGGBB / 0xAARRGGBB) passed to a draw call or PushStyleColor
  px      a number literal beyond +-1 as an argument of ImVec2(...), or of the calls that take a size or a spacing:
          SetNextItemWidth, PushItemWidth, SameLine, Indent, Unindent, SetCursorPosX/Y, PushTextWrapPos,
          SetColumnWidth, TableSetupColumn (its width), PushStyleVar (its value), SetNextWindowContentSize.
          Numbers in [-1, 1] are ratios, pivots and "fill" markers (-1, 0, .5f), not pixels.
  keys    ImGui::MenuItem / MenuItemEx with a shortcut outside the kit: ImGui prints it as grey text, XUI::MenuItem
          draws the keycaps every other menu shows
A line that really means it (a texture's own pixel size, say) carries the marker  // ui-units: ok <why>

    python tools/qa/lint_ui_units.py                     every UI file of the editor (LevelEditor, XrECore, XrEUI, XrEProps)
    python tools/qa/lint_ui_units.py <file or folder>... only those
    python tools/qa/lint_ui_units.py --summary           one line per file, the worst first
    python tools/qa/lint_ui_units.py --json              the findings as JSON (the tour reads it)
    python tools/qa/lint_ui_units.py --baseline b.json   what the baseline lists is known and passes
    python tools/qa/lint_ui_units.py --write-baseline b.json

Exit code 0 when nothing new is found.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
EDITORS = os.path.join(REPO, "Source", "Editors")
# the Grizzly Editor's own UI: the level editor, its core, the kit and the property grid (the SDK's other editors are not
# part of the redesign)
DEFAULT = [os.path.join(EDITORS, d) for d in ("LevelEditor", "XrECore", "XrEUI", "XrEProps")]
MARKER = "ui-units: ok"

# vendored ImGui, the embedded fonts, the icon geometry (a 16-unit grid scaled when drawn) and the theme, whose metrics
# are pt that ScaleAllSizes turns into px in one place
SKIP_NAMES = re.compile(r"^(imgui(_\w+)?\.(cpp|h)|imstb_\w+\.h|imconfig\.h|XrUIFonts\.cpp|XrUIIcons\.cpp|XrUITheme\.cpp)$", re.I)
SKIP_DIRS = {"external", "sdk", "intermediate", "bin"}

NUM = r"-?(?:\d+\.\d*|\.\d+|\d+)(?:[eE][-+]?\d+)?[fF]?"
NUM_RE = re.compile(r"^\s*(" + NUM + r")\s*$")

SIZE_CALLS = {
    # call: which arguments are sizes (None = all)
    "SetNextItemWidth": [0], "PushItemWidth": [0], "SameLine": None, "Indent": [0], "Unindent": [0],
    "SetCursorPosX": [0], "SetCursorPosY": [0], "PushTextWrapPos": [0], "SetColumnWidth": [1],
    "TableSetupColumn": [2], "PushStyleVar": [1], "SetNextWindowContentSize": None,
}
DRAW_CALLS = re.compile(r"\b(PushStyleColor|AddRect|AddRectFilled|AddRectFilledMultiColor|AddLine|AddText|AddCircle|AddCircleFilled|"
                        r"AddTriangle|AddTriangleFilled|AddQuad|AddQuadFilled|AddNgon|AddNgonFilled|AddPolyline|AddConvexPolyFilled|"
                        r"AddBezierCubic|AddBezierQuadratic|TextColored|ColorButton)\s*\(")
HEX_COLOUR = re.compile(r"\b0[xX][0-9A-Fa-f]{6}(?:[0-9A-Fa-f]{2})?\b")
# kit calls whose ImVec2 is in pt already
PT_CALLS = {"BeginList"}
# the one file that may hand ImGui a menu item's shortcut: the kit draws its keycaps there
KEYS_OK = re.compile(r"^XrUIKit\.cpp$", re.I)
NO_KEYS = {"", "nullptr", "NULL", "0"}


def strip_code(text: str) -> str:
    """The source with comments and string/char literals blanked, line breaks kept (line numbers stay)."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif two == "/*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("".join("\n" if ch == "\n" else " " for ch in text[i:j]))
            i = j
        elif c == '"' or c == "'":
            # a raw string R"(...)" is rare in UI code; a plain one ends at an unescaped quote
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(c + " " * max(0, j - i - 2) + (c if j - i >= 2 else ""))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def call_args(code: str, open_paren: int) -> tuple[list[tuple[str, int]], int]:
    """The top-level arguments of the call whose '(' is at open_paren: [(text, offset)], and the offset after ')'."""
    depth, start, args = 0, open_paren + 1, []
    i = open_paren
    while i < len(code):
        ch = code[i]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
            if depth == 0:
                args.append((code[start:i], start))
                return args, i + 1
        elif ch == "," and depth == 1:
            args.append((code[start:i], start))
            start = i + 1
        i += 1
    return args, len(code)


def number(text: str) -> float | None:
    m = NUM_RE.match(text)
    if not m:
        return None
    try:
        return float(m.group(1).rstrip("fF"))
    except ValueError:
        return None


def enclosing_call(code: str, offset: int) -> str:
    """The name of the call whose argument list holds `offset`, or ""."""
    depth = 0
    i = offset - 1
    while i >= 0:
        ch = code[i]
        if ch in ")]}":
            depth += 1
        elif ch in "([{":
            if depth == 0:
                if ch != "(":
                    return ""
                m = re.search(r"([A-Za-z_]\w*)\s*$", code[max(0, i - 80):i])
                return m.group(1) if m else ""
            depth -= 1
        elif ch == ";" and depth == 0:
            return ""
        i -= 1
    return ""


def line_of(code: str, offset: int) -> int:
    return code.count("\n", 0, offset) + 1


def lint_text(path: str, text: str) -> list[dict]:
    code = strip_code(text)
    raw_lines = text.splitlines()
    found: list[dict] = []

    def add(offset: int, rule: str, detail: str) -> None:
        line = line_of(code, offset)
        raw = raw_lines[line - 1] if line - 1 < len(raw_lines) else ""
        before = raw_lines[line - 2] if line >= 2 else ""
        if MARKER in raw or MARKER in before:
            return
        found.append({"file": os.path.relpath(path, REPO).replace("\\", "/"), "line": line, "rule": rule,
                      "detail": detail, "code": raw.strip()[:160]})

    # ---- colours
    for m in re.finditer(r"\bIM_COL32(_WHITE|_BLACK)?\b(\s*\()?", code):
        if m.group(1):
            # white is how an image is drawn as it is: a tint, not a colour of the UI
            stmt = code[code.rfind(";", 0, m.start()) + 1:code.find(";", m.start())]
            if m.group(1) == "_WHITE" and "Image" in stmt:
                continue
            add(m.start(), "colour", "IM_COL32" + m.group(1))
            continue
        if not m.group(2):
            continue
        args, _ = call_args(code, m.end() - 1)
        values = [number(a) for a, _ in args]
        # a colour built from values (a converter) is not a literal; four numbers are, unless all zero (transparent)
        if len(values) != 4 or any(v is None for v in values) or all(v == 0 for v in values):
            continue
        add(m.start(), "colour", "IM_COL32(...)")
    for m in re.finditer(r"\bImVec4\s*\(", code):
        args, _ = call_args(code, m.end() - 1)
        values = [number(a) for a, _ in args]
        if len(values) == 4 and all(v is not None for v in values) and any(v != 0 for v in values):
            add(m.start(), "colour", "ImVec4 of number literals")
    for m in DRAW_CALLS.finditer(code):
        args, _ = call_args(code, m.end() - 1)
        for a, off in args:
            h = HEX_COLOUR.search(a)
            if h:
                add(off + h.start(), "colour", m.group(1) + " with " + h.group(0))

    # ---- pixels
    # xrCore defines FLT_MIN as -FLT_MAX: ImGui's "fill the row" idiom asks for a width of FLT_MAX here
    for m in re.finditer(r"-\s*FLT_MIN\b", code):
        add(m.start(), "px", "-FLT_MIN is +FLT_MAX in this code base: GetContentRegionAvail().x")
    for m in re.finditer(r"\bImVec2\s*\(", code):
        if enclosing_call(code, m.start()) in PT_CALLS:
            continue
        args, _ = call_args(code, m.end() - 1)
        for a, off in args:
            v = number(a)
            if v is not None and abs(v) > 1.0:
                add(off, "px", "ImVec2(" + ", ".join(x.strip() for x, _ in args) + ")")
                break
    # ---- shortcuts
    if not KEYS_OK.match(os.path.basename(path)):
        for m in re.finditer(r"\bImGui::MenuItem(Ex)?\s*\(", code):
            args, _ = call_args(code, m.end() - 1)
            # MenuItem(label, shortcut, ...), MenuItemEx(label, icon, shortcut, ...)
            k = 2 if m.group(1) else 1
            if k < len(args) and args[k][0].strip() not in NO_KEYS:
                add(args[k][1], "keys", "ImGui::MenuItem%s with a shortcut: XUI::MenuItem draws keycaps" % (m.group(1) or ""))
    for name, which in SIZE_CALLS.items():
        for m in re.finditer(r"\b" + name + r"\s*\(", code):
            args, _ = call_args(code, m.end() - 1)
            picks = range(len(args)) if which is None else [k for k in which if k < len(args)]
            # a stretched column's number is its weight
            if name == "TableSetupColumn" and len(args) > 1 and "WidthStretch" in args[1][0]:
                continue
            for k in picks:
                v = number(args[k][0])
                if v is not None and abs(v) > 1.0:
                    add(args[k][1], "px", "%s(... %s ...)" % (name, args[k][0].strip()))
                    break
    return found


def ui_file(path: str) -> bool:
    if not path.lower().endswith((".cpp", ".h", ".hpp", ".inl")):
        return False
    if SKIP_NAMES.match(os.path.basename(path)):
        return False
    try:
        with open(path, "rb") as f:
            head = f.read()
    except OSError:
        return False
    return b"ImGui::" in head or b"XUI::" in head or b"ImDrawList" in head


def walk(targets: list[str]) -> list[str]:
    files: list[str] = []
    for target in targets:
        target = os.path.abspath(target if os.path.isabs(target) else os.path.join(REPO, target))
        if os.path.isfile(target):
            if ui_file(target) or target.lower().endswith((".cpp", ".h")):
                files.append(target)
            continue
        for root, dirs, names in os.walk(target):
            dirs[:] = [d for d in dirs if d.lower() not in SKIP_DIRS and not d.startswith(".")]
            for name in names:
                p = os.path.join(root, name)
                if ui_file(p):
                    files.append(p)
    return sorted(set(files))


def read(path: str) -> str:
    with open(path, "rb") as f:
        data = f.read()
    for enc in ("utf-8-sig", "cp1251", "latin-1"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("latin-1", "replace")


def key(hit: dict) -> str:
    # the line number moves with every edit above it: a finding is its file, rule and code
    return "%s|%s|%s" % (hit["file"], hit["rule"], hit["code"])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("paths", nargs="*", help="files or folders (default: the editor's UI folders)")
    ap.add_argument("--summary", action="store_true", help="one line per file")
    ap.add_argument("--json", action="store_true", help="the findings as JSON")
    ap.add_argument("--baseline", help="a JSON list of known findings that pass")
    ap.add_argument("--write-baseline", help="write every finding to this file and pass")
    args = ap.parse_args()

    files = walk(args.paths or DEFAULT)
    hits: list[dict] = []
    for path in files:
        hits.extend(lint_text(path, read(path)))

    if args.write_baseline:
        with open(args.write_baseline, "w", encoding="utf-8") as f:
            json.dump(sorted({key(h) for h in hits}), f, indent=1, ensure_ascii=False)
        print("baseline: %d findings in %d files -> %s" % (len(hits), len({h['file'] for h in hits}), args.write_baseline))
        return 0
    known: set[str] = set()
    if args.baseline:
        with open(args.baseline, encoding="utf-8") as f:
            known = set(json.load(f))
    new = [h for h in hits if key(h) not in known]

    if args.json:
        print(json.dumps({"files": len(files), "findings": new, "known": len(hits) - len(new)}, ensure_ascii=False, indent=1))
    elif args.summary:
        per: dict[str, list[int]] = {}
        rules = ("px", "colour", "keys")
        for h in new:
            c = per.setdefault(h["file"], [0, 0, 0])
            c[rules.index(h["rule"])] += 1
        for f, (px, colour, keys) in sorted(per.items(), key=lambda kv: -sum(kv[1])):
            print("%5d px %5d colour %5d keys  %s" % (px, colour, keys, f))
    else:
        for h in new:
            print("%s:%d: %s: %s    %s" % (h["file"], h["line"], h["rule"], h["detail"], h["code"]))
    print("UI UNITS: %d file(s), %d finding(s)%s" % (len(files), len(new),
          " (%d known in the baseline)" % (len(hits) - len(new)) if known else ""), file=sys.stderr)
    return 0 if not new else 1


if __name__ == "__main__":
    sys.exit(main())
