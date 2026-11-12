"""Test code creates no program files.

An antivirus reads a freshly built, unsigned test binary that writes an `.exe` into a temp folder
as a dropper. So no test of the workspace names a program file (`.exe`, `.dll`, `.com`, `.scr`,
`.bat`, `.cmd`, `.ps1`, `.msi`) or spells the `MZ` header of one in a literal: what a test writes
gets a harmless name, and the code under test takes the name as a parameter where it must.

Test code is a file under a crate's `tests/`, an item under `#[cfg(test)]` (a `mod name;` takes its
whole file), and a `#[test]` function. A literal that never reaches the disk carries
`// program-files: in memory` on its line or the line above.

    python tools/rust/check_test_files.py [crates]

Prints every finding as `path:line: text` and exits 1 when there is one.
"""
import os
import re
import sys

EXTENSIONS = "exe|dll|com|scr|bat|cmd|ps1|msi"
NAMED = re.compile(rf"\.({EXTENSIONS})(?![\w/])", re.IGNORECASE)
BARE = re.compile(r"^(exe|dll|scr|bat|ps1|msi)$", re.IGNORECASE)
HEADER_BYTES = re.compile(r"b'M'[\s\S]{0,40}?b'Z'|0x4[dD]\s*,\s*0x5[aA]\b|0x5[aA]4[dD]\b")
MARKER = "program-files: in memory"


def lex(text):
    """Yields (kind, start, end, value): 'str' literals with their content, 'code' runs between them.

    Comments are dropped; char literals and lifetimes stay code."""
    i, n, code_from = 0, len(text), 0
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            yield "code", code_from, i, text[code_from:i]
            j = text.find("\n", i)
            i = code_from = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            yield "code", code_from, i, text[code_from:i]
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            i = code_from = j
            continue
        m = re.match(r'(b|c)?r(#*)"', text[i:]) if c in "bcr" else None
        if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            yield "code", code_from, i, text[code_from:i]
            close = '"' + m.group(2)
            body = i + m.end()
            j = text.find(close, body)
            j = n if j < 0 else j
            yield "str", i, j + len(close), text[body:j]
            i = code_from = j + len(close)
            continue
        if c == '"' or (c in "bc" and text.startswith('"', i + 1) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_"))):
            yield "code", code_from, i, text[code_from:i]
            body = i + (1 if c == '"' else 2)
            j = body
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            yield "str", i, j + 1, text[body:j]
            i = code_from = j + 1
            continue
        if c == "'":
            # a char literal ('x', '\n', '\'', '\u{..}'); a lifetime has no closing quote
            m = re.match(r"'(\\.[^']*|[^'\\])'", text[i:])
            i += m.end() if m else 1
            continue
        i += 1
    yield "code", code_from, n, text[code_from:n]


def item_end(tokens, at):
    """The end offset of the item that starts at offset `at`: its matching `}` or its `;`."""
    depth = 0
    for kind, start, end, value in tokens:
        if kind != "code" or end <= at:
            continue
        for k, ch in enumerate(value):
            if start + k < at:
                continue
            if ch in "{[(":
                depth += 1
            elif ch in "}])":
                depth -= 1
                if depth == 0 and ch == "}":
                    return start + k + 1
            elif ch == ";" and depth == 0:
                return start + k + 1
    return None


ATTR = re.compile(r"#\[\s*(cfg\s*\((?:[^()]|\([^()]*\))*\btest\b(?:[^()]|\([^()]*\))*\)|test)\s*\]")
MOD_DECL = re.compile(r"\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
PATH_ATTR = re.compile(r'#\[\s*path\s*=\s*"([^"]+)"\s*\]')


def test_spans(text, tokens):
    """Spans of `text` that are test code, and the module files a `#[cfg(test)] mod name;` names."""
    spans, files = [], []
    for t, (kind, start, _end, value) in enumerate(tokens):
        if kind != "code":
            continue
        for m in ATTR.finditer(value):
            if m.group(1).startswith("cfg") and "not" in m.group(1):
                continue
            at = start + m.end()
            decl = MOD_DECL.match(text, at)
            if decl:
                path = PATH_ATTR.search(text, start + m.start(), decl.end())
                files.append(path.group(1) if path else decl.group(1))
                continue
            end = item_end(tokens[t:], at)
            spans.append((start + m.start(), end or len(text)))
    return spans, files


def module_dir(path):
    base = os.path.basename(path)
    if base in ("lib.rs", "main.rs", "mod.rs", "build.rs"):
        return os.path.dirname(path)
    return os.path.splitext(path)[0]


def findings(path, text, whole):
    tokens = list(lex(text))
    spans, files = test_spans(text, tokens)
    lines = text.split("\n")

    def inside(pos):
        return whole or any(a <= pos < b for a, b in spans)

    def marked(line):
        return MARKER in lines[line] or (line > 0 and MARKER in lines[line - 1])

    out = []
    for kind, start, _end, value in tokens:
        if not inside(start):
            continue
        at = start
        if kind == "str":
            url = "://" in value or "www." in value
            hit = (value.startswith("MZ") or BARE.match(value)
                   or any(not (url and m.group(1).lower() == "com") for m in NAMED.finditer(value)))
        else:
            hit = HEADER_BYTES.search(value)
            at = start + hit.start() if hit else start
        if hit:
            line = text.count("\n", 0, at)
            if not marked(line):
                out.append((line + 1, lines[line].strip()))
    children = []
    for name in files:
        if name.endswith(".rs"):
            children.append(os.path.join(os.path.dirname(path), name))
        else:
            d = module_dir(path)
            for cand in (os.path.join(d, name + ".rs"), os.path.join(d, name, "mod.rs")):
                if os.path.isfile(cand):
                    children.append(cand)
                    break
    return out, children


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "crates"
    sources = []
    for d, subdirs, names in os.walk(root):
        subdirs[:] = [s for s in subdirs if s not in ("target", "data")]
        sources += [os.path.join(d, x) for x in names if x.endswith(".rs")]
    whole = {p for p in sources if os.sep + "tests" + os.sep in p or "/tests/" in p}
    queue, seen, bad = sorted(sources), set(), []
    while queue:
        path = os.path.normpath(queue.pop(0))
        key = (path, path in whole)
        if key in seen:
            continue
        seen.add(key)
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
        found, children = findings(path, text, path in whole)
        bad += [(path, line, src) for line, src in found]
        for child in children:
            whole.add(os.path.normpath(child))
            queue.append(child)
    for path, line, src in sorted(set(bad)):
        print(f"{path.replace(os.sep, '/')}:{line}: {src}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
