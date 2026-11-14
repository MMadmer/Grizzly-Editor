#!/usr/bin/env python3
"""Fails when the editor's code speaks by another program.

The editor is its own program. Its comments, doc comments and messages say what the code does and
why, never "as the C++ editor did"; a message's level is a value, never a mark in its text; and no
text names a command that no longer exists. This check keeps all of it out of `crates/`:

    python tools/rust/check_words.py            exit 1 on a hit, one line per hit
    python tools/rust/check_words.py --list     the allow-list as it is applied

What it reads in the code (`crates/**/*.rs`, `target` folders left out):

- the words `C++`, `cpp`, `oracle`, `parity`, `Source/`, `SDK 0.7`, `LevelEditor` and `xrLC` and the former editor ("the original editor"), in any
  comment, string literal or name;
- the names of the old code - the classes and functions of the program the editor replaced
  (`TUI_...`, `EScene`, `CUI...`, `CBuild::...`) - in a comment or a string literal;
- the names of the commands 0.7 replaced (`tools/rust/removed_commands.txt`), as a whole word in a
  comment or a string literal (a Rust identifier of the same spelling is no mention);
- a message whose first character is a level mark (`!`, `~`, `*`, `ERROR:`): the level is a value;
- another program's member or call cited in a comment or a string (`m_Name`, `Build(bFlag)`);
- the plan the work was done by and the documents it replaced (`docs/plan`, a phase or a stream of it,
  `docs/rust`, the migration document), anywhere: they are gone and nothing points at them.

What it reads beside the code - the tools (`tools/**`), the documents (`docs/**/*.md`, `README.md`,
`CLAUDE.md`) and the manifests (`Cargo.toml`, `crates/*/Cargo.toml`, `Build.ps1`): the plan's words and the
replaced program by its names (not `C++`, `cpp` or `parity`: the documents describe the game's own code and
the toolchain in those words). `docs/plan` itself is the plan and is not read.

`tools/rust/words_allowed.txt` is the short list of places where a format's or a game program's real
name is the fact. One entry per line: `path-glob | pattern | reason` (separated by " | "); a hit is allowed when its file
matches the glob and its text matches the pattern (a regular expression).
"""
from __future__ import annotations

import fnmatch
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"
ALLOWED = Path(__file__).with_name("words_allowed.txt")
REMOVED = Path(__file__).with_name("removed_commands.txt")

# Words that name the program the editor replaced or the way it was proved. Anywhere in the text.
WORDS = [
    ("C++", re.compile(r"C\+\+")),
    ("cpp", re.compile(r"(?i:(?<![a-z0-9])cpp(?![a-z0-9]))|Cpp[A-Z]")),
    ("oracle", re.compile(r"(?i:\boracles?\b)")),
    ("parity", re.compile(r"(?i:\bparity\b)")),
    ("Source/", re.compile(r"Source/")),
    ("SDK 0.7", re.compile(r"SDK 0\.7")),
    ("LevelEditor", re.compile(r"LevelEditor")),
    ("former editor", re.compile(r"(?i:\b(?:original|earlier|older|previous|former) editors?\b)")),
    ("xrLC", re.compile(r"xrLC")),
]

# The names of the old code, in a comment or a string literal.
OLD = [
    ("old editor class", re.compile(r"\b(?:TUI_|XUI_|CUI)[A-Za-z_]+|\bEScene[A-Za-z_]*|\bUI(?:Spawn|AIMap|Sector|Portal|Shape|Way|Light|Group|Wallmark)Tool\b")),
    ("old class::function", re.compile(r"\b(?:C|E|S|I)[A-Z][A-Za-z0-9]+::[A-Za-z_~][A-Za-z0-9_]*")),
    ("old class", re.compile(r"\bC(?:Build|Inifile|Deflector|Lightmap|EditableObject|EditableMesh|CustomObject|EditShape|SceneObject|SpawnPoint|WayPoint|WayObject|Portal|Sector|GroupObject|Glow|Light|ParticleEffect|ParticleGroup|PGDef|PEDef|GameGraphBuilder|ImageManager|LocatorAPI|StringTable|Xml|Envelope|Bone|Frustum|Blender_?\w*|Surface)\b")),
    ("old function prefix", re.compile(r"\bxr(?:Phase|Load|MU|DO|CDB)_?[A-Za-z_]*")),
    ("old math type", re.compile(r"\bF(?:vector2?|matrix(?:33)?|box2?|plane|color|sphere|obb)\b")),
]

MARK = re.compile(r"^(?:[!~*]{1,3} |ERROR: |!(?=[A-Za-z.]))")

# The plan the work was done by, and the documents it left behind. Anywhere in the text.
PLAN = [
    ("a document that is gone", re.compile(r"RUST_MIGRATION|docs[/\\]rust\b")),
    ("the plan", re.compile(r"docs[/\\]plan\b|\bplan [A-Z]\d\b|\bphase \d+[a-z]?'s gate\b|\b[Aa]fter phase \d"
                            r"|\bstreams? [A-Z]\d[a-z]?\d?\b|\bin their own streams\b")),
    ("the editor by its language", re.compile(r"\bthe (?:Rust|C\+\+) (?:editor|port|side|version)\b")),
]

# Another program's members and calls, in a comment or a string literal.
MEMBER = [
    ("another program's member", re.compile(r"`m_[A-Za-z]\w*`|\bm_[A-Z][A-Za-z0-9]+\b")),
    ("another program's call",
     re.compile(r"`[A-Z][A-Za-z0-9]+\([^`)]*\b(?:b|p|dw|sz|n|f)[A-Z][A-Za-z0-9]*[^`)]*\)`")),
]

# What the files beside the code are held to: the plan's words and the replaced program by name.
BESIDE = [w for w in WORDS if w[0] in ("oracle", "Source/", "SDK 0.7", "LevelEditor", "former editor", "xrLC")]


def beside_the_code():
    """The tools, the documents and the manifests: every text file that is not Rust source."""
    own = {Path(__file__).resolve(), ALLOWED.resolve(), REMOVED.resolve()}
    for path in sorted((ROOT / "tools").rglob("*")):
        if (path.is_file() and path.suffix in (".py", ".sh", ".go", ".md", ".txt", ".ps1")
                and "__pycache__" not in path.parts and path.resolve() not in own):
            yield path
    for path in sorted((ROOT / "docs").rglob("*.md")):
        if "plan" not in path.relative_to(ROOT).parts:
            yield path
    for name in ("README.md", "CLAUDE.md", "Cargo.toml", "Build.ps1"):
        if (ROOT / name).is_file():
            yield ROOT / name
    yield from sorted(CRATES.glob("*/Cargo.toml"))


def allowed_rules() -> list[tuple[str, re.Pattern[str], str]]:
    rules = []
    for raw in ALLOWED.read_text(encoding="utf8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        glob, pattern, reason = [p.strip() for p in line.split(" | ", 2)]
        rules.append((glob, re.compile(pattern), reason))
    return rules


def workspace_types() -> set[str]:
    """The struct, enum, trait and type names this workspace declares: `Name::item` of one is no old code."""
    found: set[str] = set()
    decl = re.compile(r"\b(?:struct|enum|trait|type|union)\s+([A-Z][A-Za-z0-9_]*)")
    for path in CRATES.rglob("*.rs"):
        if "/target/" in path.as_posix():
            continue
        found.update(decl.findall(path.read_text(encoding="utf8", errors="replace")))
    return found


def removed_names() -> re.Pattern[str]:
    names = [n.strip() for n in REMOVED.read_text(encoding="utf8").splitlines() if n.strip() and not n.startswith("#")]
    return re.compile(r"(?<![A-Za-z0-9_.:])(?:" + "|".join(re.escape(n) for n in sorted(names, key=len, reverse=True)) + r")(?![A-Za-z0-9_])")


def segments(text: str):
    """Yields (kind, line, text): `comment`, `string` and `code` pieces of a Rust source."""
    i, n, line = 0, len(text), 1
    code_start = 0
    code_line = 1

    def flush_code(upto: int):
        nonlocal code_start
        if upto > code_start:
            yield ("code", code_line, text[code_start:upto])

    while i < n:
        c = text[i]
        if c == "/" and text.startswith("//", i):
            yield from flush_code(i)
            j = text.find("\n", i)
            j = n if j < 0 else j
            yield ("comment", line, text[i:j])
            i = code_start = j
            code_line = line
            continue
        if c == "/" and text.startswith("/*", i):
            yield from flush_code(i)
            depth, j, start_line = 1, i + 2, line
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            body = text[i:j]
            yield ("comment", start_line, body)
            line += body.count("\n")
            i = code_start = j
            code_line = line
            continue
        m = re.match(r"(?:b?r)(#*)\"", text[i:i + 12]) if c in "br" else None
        if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            yield from flush_code(i)
            hashes = m.group(1)
            end = text.find('"' + hashes, i + m.end())
            end = n if end < 0 else end + 1 + len(hashes)
            body = text[i + m.end():max(end - 1 - len(hashes), i + m.end())]
            yield ("string", line, body)
            line += text[i:end].count("\n")
            i = code_start = end
            code_line = line
            continue
        if c == '"':
            yield from flush_code(i)
            j, start_line = i + 1, line
            while j < n and text[j] != '"':
                if text[j] == "\\":
                    j += 1
                j += 1
            body = text[i + 1:j]
            yield ("string", start_line, body)
            line += text[i:j + 1].count("\n")
            i = code_start = j + 1
            code_line = line
            continue
        if c == "'":
            m2 = re.match(r"'(?:\\.[^']*|[^\\'])'", text[i:i + 12])
            if m2:
                i += m2.end()
                continue
        if c == "\n":
            line += 1
        i += 1
    yield from flush_code(n)


def main() -> int:
    if "--list" in sys.argv:
        for glob, pat, reason in allowed_rules():
            print(f"{glob} | {pat.pattern} | {reason}")
        return 0
    rules = allowed_rules()
    removed = removed_names()
    own = workspace_types()
    hits: list[str] = []
    used = [0] * len(rules)
    for path in sorted(CRATES.rglob("*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        if "/target/" in rel:
            continue
        text = path.read_text(encoding="utf8", errors="replace")
        lines_of: dict[int, str] = {}
        for kind, line, body in segments(text):
            checks = list(WORDS) + PLAN
            if kind != "code":
                checks += OLD
                checks += MEMBER
                checks.append(("removed command", removed))
            for label, pat in checks:
                for m in pat.finditer(body):
                    if label.startswith("old class") and m.group(0).split("::")[0] in own:
                        continue
                    at = line + body.count("\n", 0, m.start())
                    shown = body.splitlines()[body.count("\n", 0, m.start())] if body else ""
                    ok = False
                    for k, (glob, apat, _reason) in enumerate(rules):
                        if fnmatch.fnmatch(rel, glob) and (apat.search(m.group(0)) or apat.search(shown)):
                            used[k] += 1
                            ok = True
                            break
                    if not ok:
                        hits.append(f"{rel}:{at}: {label}: {shown.strip()[:110]}")
            if kind == "string" and MARK.match(body) and len(body) > 3:
                at = line
                shown = body.splitlines()[0] if body else ""
                ok = False
                for k, (g, a, _r) in enumerate(rules):
                    if fnmatch.fnmatch(rel, g) and a.search(shown):
                        used[k] += 1
                        ok = True
                        break
                if not ok:
                    hits.append(f"{rel}:{at}: level mark in a message: {shown.strip()[:110]}")
    for path in beside_the_code():
        rel = path.relative_to(ROOT).as_posix()
        for at, shown in enumerate(path.read_text(encoding="utf8", errors="replace").splitlines(), 1):
            for label, pat in BESIDE + PLAN:
                for m in pat.finditer(shown):
                    ok = False
                    for k, (glob, apat, _reason) in enumerate(rules):
                        if fnmatch.fnmatch(rel, glob) and (apat.search(m.group(0)) or apat.search(shown)):
                            used[k] += 1
                            ok = True
                            break
                    if not ok:
                        hits.append(f"{rel}:{at}: {label}: {shown.strip()[:110]}")
    for h in dict.fromkeys(hits):
        print(h)
    stale = [r for r, u in zip(rules, used) if not u]
    for glob, pat, _ in stale:
        print(f"words_allowed.txt: an entry matches nothing: {glob} | {pat.pattern}")
    if hits or stale:
        print(f"check_words: {len(hits)} hit(s), {len(stale)} stale allow-list entr(ies)", file=sys.stderr)
        return 1
    print("check_words: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
