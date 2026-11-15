"""LTX the way the game reads it (`CInifile::Load`), kept as structure: every section's parents and own lines.

A section is resolved dynamically - its parents' resolved keys in order, then its own lines - which is the
composition the game's config engine does (a change of a key reaches every section that inherits it). For a
file the game loads once this is the same as the engine's copy at parse time.
"""
from __future__ import annotations

import fnmatch
import os
import re

REMOVED = object()  # an own line that takes a key away (`!key`)


def decode(data: bytes) -> str:
    return data.decode("cp1251", "surrogateescape")


def encode(text: str) -> bytes:
    return text.encode("cp1251", "surrogateescape")


def lines_of(data: bytes) -> list[str]:
    return re.split(r"\r\n|\r|\n", decode(data))


_SPACE = " \t\n\v\f\r"


_BLANK_OR_QUOTE = re.compile(r'[ \t\n\v\f\r"]')


def clean(value: str) -> tuple[str, bool]:
    """`_parse`: blanks outside quotes go. Returns the value and whether a quote is still open."""
    if not _BLANK_OR_QUOTE.search(value):
        return value, False
    out, quoted, i, n = [], False, 0, len(value)
    while i < n:
        c = value[i]
        if c in _SPACE:
            if quoted:
                out.append(c)
                i += 1
                continue
            while i < n and value[i] in _SPACE:
                i += 1
            continue
        if c == '"':
            quoted = not quoted
        out.append(c)
        i += 1
    return "".join(out), quoted


def cut_comment(line: str) -> str:
    """The engine's rule: the first ';', or a '//' when the first '/' starts one, unless a quote spans it."""
    comm = line.find(";")
    slash = line.find("/")
    if slash >= 0 and line[slash + 1:slash + 2] == "/" and (comm < 0 or slash < comm):
        comm = slash
    if comm < 0:
        return line
    q1 = line.find('"')
    if 0 <= q1 < comm:
        q2 = line.find('"', q1 + 1)
        if q2 > comm:
            return line
    return line[:comm]


class Sect:
    __slots__ = ("name", "parents", "own", "file")

    def __init__(self, name, parents=None, own=None, file=None):
        self.name = name
        self.parents = list(parents or [])
        self.own = dict(own or {})
        self.file = file

    def copy(self):
        return Sect(self.name, self.parents, self.own, self.file)


class Config:
    """Sections by name; `resolve` composes them."""

    def __init__(self, fallback: "Config | None" = None):
        self.sections: dict[str, Sect] = {}
        self.order: list[str] = []
        self.files: list[str] = []
        self.problems: list[str] = []
        self.dupes: list[tuple[str, str, str]] = []
        self.late_parents: list[tuple[str, str]] = []
        self.fallback = fallback
        self._memo: dict[str, dict] = {}

    # ---- structure
    def get(self, name: str) -> Sect | None:
        s = self.sections.get(name)
        if s is None and self.fallback is not None:
            return self.fallback.get(name)
        return s

    def has(self, name: str) -> bool:
        return self.get(name) is not None

    def add(self, sect: Sect):
        if sect.name not in self.sections:
            self.order.append(sect.name)
        self.sections[sect.name] = sect
        self.invalidate()

    def remove(self, name: str):
        if name in self.sections:
            del self.sections[name]
            self.order.remove(name)
            self.invalidate()

    def invalidate(self):
        self._memo.clear()

    def copy(self) -> "Config":
        c = Config(self.fallback)
        c.sections = {n: s.copy() for n, s in self.sections.items()}
        c.order = list(self.order)
        c.files = list(self.files)
        return c

    # ---- composition
    def resolve(self, name: str, _stack=()) -> dict:
        hit = self._memo.get(name)
        if hit is not None:
            return hit
        s = self.sections.get(name)
        if s is None:
            if self.fallback is not None and self.fallback.get(name) is not None:
                return self.fallback.resolve(name)
            return {}
        if name in _stack:
            return {}
        out: dict = {}
        for p in s.parents:
            if self.has(p):
                out.update(self.resolve(p, _stack + (name,)))
        for k, v in s.own.items():
            if v is REMOVED:
                out.pop(k, None)
            else:
                out[k] = v
        self._memo[name] = out
        return out

    def children(self) -> dict[str, list[str]]:
        """Direct children of every section of this config (by its parent lists)."""
        kids: dict[str, list[str]] = {}
        for n in self.order:
            for p in self.sections[n].parents:
                kids.setdefault(p, []).append(n)
        return kids

    def topo(self) -> list[str]:
        """Section names, every parent before its children."""
        seen, out = set(), []

        def visit(n, stack):
            if n in seen or n in stack:
                return
            s = self.sections.get(n)
            if s is None:
                return
            for p in s.parents:
                visit(p, stack | {n})
            seen.add(n)
            out.append(n)

        for n in self.order:
            visit(n, frozenset())
        return out


class Reader:
    """Loads LTX files from a view (rel path -> bytes) into a Config, following `#include` like the engine."""

    def __init__(self, view, config: Config):
        self.view = view
        self.c = config

    def load(self, rel: str, stack=(), optional=False, merge=False):
        data = self.view.read(rel) if self.view.exists(rel) else None
        if data is None:
            if not optional:
                self.c.problems.append(f"missing include {rel}" + (f" (from {stack[-1]})" if stack else ""))
            return
        if rel in stack:
            self.c.problems.append(f"include loop {rel}")
            return
        self.c.files.append(rel)
        folder = os.path.dirname(rel)
        lines = lines_of(data)
        cur: Sect | None = None
        i = 0
        while i < len(lines):
            raw = lines[i].strip(_SPACE)
            i += 1
            line = cut_comment(raw).strip(_SPACE)
            if line and line[0] == "#" and "#include" in line:
                parts = line.split('"')
                if len(parts) >= 2:
                    inc = parts[1]
                    opt = "#include_optional" in line
                    target = os.path.normpath(os.path.join(folder, inc)).lower()
                    if "*.ltx" in inc:
                        base_dir, mask = os.path.split(target)
                        prefix = base_dir + "\\"
                        names = sorted(n for n in self.view.names
                                       if n.startswith(prefix) and fnmatch.fnmatch(n[len(prefix):], mask))
                        for n in names:
                            self.load(n, stack + (rel,), True)
                    else:
                        self.load(target, stack + (rel,), opt)
                continue
            if line and line[0] == "[":
                self._close(cur, rel, merge)
                close = line.find("]")
                if close < 0:
                    self.c.problems.append(f"bad section line in {rel}: {line}")
                    cur = None
                    continue
                name = line[1:close].lower()
                parents = []
                if line[close:close + 2] == "]:":
                    for p in line[close + 2:].split(","):
                        p = p.strip(_SPACE).lower()
                        if not p:
                            self.c.problems.append(f"empty parent of [{name}] in {rel}")
                            continue
                        parents.append(p)
                        if not self.c.has(p):
                            self.c.late_parents.append((name, p))
                elif ":" in line[close + 1:]:
                    self.c.problems.append(f"[{name}] in {rel}: parents after a blank (`] :`), the game ignores them")
                cur = Sect(name, parents, {}, rel)
                continue
            if cur is None:
                continue
            eq = line.find("=")
            if eq >= 0:
                key = line[:eq].strip(_SPACE)
                raw_value = line[eq + 1:]
                value, open_quote = clean(raw_value)
                while open_quote and i < len(lines):
                    nxt = lines[i]
                    if "[" in nxt and "]" in nxt[nxt.index("["):]:
                        self.c.problems.append(f"unclosed quote in [{cur.name}] {key} of {rel}")
                        value = value.replace('"', "")
                        break
                    i += 1
                    raw_value += "\r\n" + nxt
                    value, open_quote = clean(raw_value)
            else:
                key, value = line, ""
            if key:
                cur.own[key] = value if value else None
        self._close(cur, rel, merge)

    def _close(self, cur: Sect | None, rel: str, merge: bool):
        if cur is None:
            return
        old = self.c.sections.get(cur.name)
        if old is not None:
            if merge:
                old.own.update(cur.own)
                self.c.invalidate()
                return
            self.c.dupes.append((cur.name, old.file, rel))
        self.c.add(cur)


def load_tree(view, root_rel: str, fallback: Config | None = None, overlays=()) -> Config:
    c = Config(fallback)
    r = Reader(view, c)
    r.load(root_rel)
    for ov in overlays:
        if view.exists(ov):
            r.load(ov, merge=True)
    return c


def load_file(view, rel: str, fallback: Config | None = None) -> Config:
    """One config as a script or the engine opens it by name: the file with its own includes."""
    c = Config(fallback)
    Reader(view, c).load(rel)
    return c


def norm(v):
    """A value as compared: the game's own parse has no blanks outside quotes; a bare key and `key =` are one."""
    if v is None or v is REMOVED:
        return v
    return clean(v)[0] or None


def items_of(value) -> list[str]:
    """A comma list as `.ltxp` +/- read it."""
    if not value:
        return []
    return [x.strip(" \t") for x in value.split(",") if x.strip(" \t")]
