"""Lua data, the one syntax of the editor's assets: `return { ... }` of literals. A writer and a strict reader."""
from __future__ import annotations

import re

RESERVED = {"and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local",
            "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"}
_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
_INT = re.compile(r"^-?(0|[1-9][0-9]*)$")
_DEC = re.compile(r"^-?(0|[1-9][0-9]*)\.[0-9]*[1-9]$")


def number_of(text: str):
    """The number a value is when writing it as one loses nothing: the game prints it back as the same text
    (`number_text` of `xms_items.cpp`: a whole number as one, else `%.15g`)."""
    if _INT.match(text):
        n = int(text)
        if abs(n) < 10 ** 15 and text != "-0":
            return n
        return None
    if _DEC.match(text):
        f = float(text)
        if format(f, ".15g") == text:
            return f
    return None


class Raw:
    """Text written into the file as it is (a number kept in its own spelling)."""

    def __init__(self, text):
        self.text = text


class Inline(dict):
    """A small table written on one line (`icon = { file = "...", w = 2, h = 1 }`)."""


def typed(value):
    """A config value as the asset writes it: a number, a boolean, a numeric vector, else the string itself."""
    if value is None:
        return ""
    if value in ("true", "false"):
        return value == "true"
    if number_of(value) is not None:
        return Raw(value)
    if "," in value and '"' not in value:
        parts = value.split(",")
        if len(parts) >= 2 and all(number_of(p) is not None for p in parts):
            return [Raw(p) for p in parts]
    return value


def key_text(k) -> str:
    if isinstance(k, str) and _IDENT.match(k) and k not in RESERVED:
        return k
    return "[" + string_text(str(k)) + "]"


def string_text(s: str) -> str:
    if "\\" in s and not any(ord(c) < 32 for c in s) and not s.startswith("\n"):
        level = 0
        while ("]" + "=" * level + "]") in s or s.endswith("]" + "=" * level):
            level += 1
        eq = "=" * level
        return "[" + eq + "[" + s + "]" + eq + "]"
    out = ['"']
    for c in s:
        o = ord(c)
        if c == "\\":
            out.append("\\\\")
        elif c == '"':
            out.append('\\"')
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif o < 32 or o == 127:
            out.append("\\%03d" % o)
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def value_text(v, indent: int) -> str:
    if isinstance(v, Raw):
        return v.text
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        return format(v, ".14g")
    if isinstance(v, str):
        return string_text(v)
    if isinstance(v, (list, tuple)):
        if all(not isinstance(x, (list, dict)) for x in v) and len(v) <= 8:
            return "{ " + ", ".join(value_text(x, indent) for x in v) + " }"
        pad = "\t" * (indent + 1)
        return "{\n" + "".join(pad + value_text(x, indent + 1) + ",\n" for x in v) + "\t" * indent + "}"
    if isinstance(v, Inline):
        return "{ " + ", ".join(key_text(k) + " = " + value_text(x, indent) for k, x in v.items()) + " }"
    if isinstance(v, dict):
        if not v:
            return "{}"
        pad = "\t" * (indent + 1)
        return "{\n" + "".join(pad + key_text(k) + " = " + value_text(x, indent + 1) + ",\n" for k, x in v.items()) + "\t" * indent + "}"
    raise TypeError(type(v))


def write(table: dict) -> str:
    return "return " + value_text(table, 0) + "\n"


# ---- the reader ------------------------------------------------------------------------------------------
class LuaError(Exception):
    pass


class _Parser:
    def __init__(self, text: str):
        self.s, self.i, self.n = text, 0, len(text)

    def line(self):
        return self.s.count("\n", 0, self.i) + 1

    def fail(self, why):
        raise LuaError(f"line {self.line()}: {why}")

    def skip(self):
        s = self.s
        while self.i < self.n:
            c = s[self.i]
            if c in " \t\r\n":
                self.i += 1
            elif s.startswith("--", self.i):
                self.i += 2
                lvl = self._long_level()
                if lvl is not None:
                    self._long_body(lvl)
                else:
                    while self.i < self.n and s[self.i] != "\n":
                        self.i += 1
            else:
                break

    def _long_level(self):
        m = re.compile(r"\[(=*)\[").match(self.s, self.i)
        if not m:
            return None
        self.i = m.end()
        return len(m.group(1))

    def _long_body(self, lvl):
        close = "]" + "=" * lvl + "]"
        j = self.s.find(close, self.i)
        if j < 0:
            self.fail("a long string is not closed")
        body = self.s[self.i:j]
        self.i = j + len(close)
        if body.startswith("\r\n"):
            body = body[2:]
        elif body.startswith("\n"):
            body = body[1:]
        return body

    def quoted(self):
        q = self.s[self.i]
        self.i += 1
        out = []
        while True:
            if self.i >= self.n:
                self.fail("a string is not closed")
            c = self.s[self.i]
            self.i += 1
            if c == q:
                return "".join(out)
            if c == "\n":
                self.fail("a line break inside a quoted string")
            if c != "\\":
                out.append(c)
                continue
            e = self.s[self.i]
            self.i += 1
            simple = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", '"': '"', "'": "'", "a": "\a", "b": "\b",
                      "f": "\f", "v": "\v", "\n": "\n"}
            if e in simple:
                out.append(simple[e])
            elif e == "x":
                h = self.s[self.i:self.i + 2]
                if not re.match(r"^[0-9A-Fa-f]{2}$", h):
                    self.fail("a \\x escape takes two hex digits")
                out.append(chr(int(h, 16)))
                self.i += 2
            elif e.isdigit():
                d = e
                while len(d) < 3 and self.i < self.n and self.s[self.i].isdigit():
                    d += self.s[self.i]
                    self.i += 1
                if int(d) > 255:
                    self.fail("a decimal escape is above 255")
                out.append(chr(int(d)))
            else:
                self.fail(f"an unknown escape \\{e}")

    def value(self, depth=0):
        self.skip()
        if self.i >= self.n:
            self.fail("a value is missing")
        c = self.s[self.i]
        if c == "{":
            return self.table(depth)
        if c in "\"'":
            return self.quoted()
        if c == "[":
            lvl = self._long_level()
            if lvl is None:
                self.fail("only plain values: no expressions or calls")
            return self._long_body(lvl)
        m = re.compile(r"-?\s*(0[xX][0-9A-Fa-f]+|\d+\.?\d*(?:[eE][-+]?\d+)?|\.\d+(?:[eE][-+]?\d+)?)").match(self.s, self.i)
        if m:
            self.i = m.end()
            text = m.group(0).replace(" ", "").replace("\t", "")
            return Raw(text)
        m = _IDENT_AT.match(self.s, self.i)
        if m:
            w = m.group(0)
            self.i = m.end()
            if w == "true":
                return True
            if w == "false":
                return False
            if w == "nil":
                return None
        self.fail("only plain values: no expressions or calls")

    def table(self, depth):
        if depth > 32:
            self.fail("tables are nested too deep")
        self.i += 1
        arr, named = [], {}
        while True:
            self.skip()
            if self.i >= self.n:
                self.fail("a table is not closed")
            if self.s[self.i] == "}":
                self.i += 1
                break
            key = None
            if self.s[self.i] == "[" and not re.compile(r"\[=*\[").match(self.s, self.i):
                self.i += 1
                k = self.value(depth + 1)
                self.skip()
                if self.s[self.i:self.i + 1] != "]":
                    self.fail("']' is missing after a key")
                self.i += 1
                self.skip()
                if self.s[self.i:self.i + 1] != "=":
                    self.fail("'=' is missing after a key")
                self.i += 1
                key = k.text if isinstance(k, Raw) else k
                if not isinstance(key, str):
                    self.fail("a key is a string or a number")
            else:
                m = _IDENT_AT.match(self.s, self.i)
                if m:
                    j = m.end()
                    save = self.i
                    self.i = j
                    self.skip()
                    if self.s[self.i:self.i + 1] == "=" and self.s[self.i:self.i + 2] != "==":
                        if m.group(0) in RESERVED:
                            self.fail(f"'{m.group(0)}' is a reserved word, write it as [\"{m.group(0)}\"]")
                        self.i += 1
                        key = m.group(0)
                    else:
                        self.i = save
            v = self.value(depth + 1)
            if key is None:
                arr.append(v)
            else:
                if key in named:
                    self.fail(f"the key '{key}' is written twice")
                named[key] = v
            self.skip()
            if self.s[self.i:self.i + 1] in (",", ";"):
                self.i += 1
            elif self.s[self.i:self.i + 1] != "}":
                self.fail("only plain values: no expressions or calls")
        if arr and named:
            self.fail("a table mixes a list and named keys")
        return arr if arr or not named else named


_IDENT_AT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def read(text: str):
    if text.startswith("﻿"):
        text = text[1:]
    p = _Parser(text)
    p.skip()
    if p.s.startswith("return", p.i) and not (p.i + 6 < p.n and (p.s[p.i + 6].isalnum() or p.s[p.i + 6] == "_")):
        p.i += 6
    else:
        p.fail("the file starts with `return`")
    v = p.value()
    p.skip()
    if p.s[p.i:p.i + 1] == ";":
        p.i += 1
        p.skip()
    if p.i != p.n:
        p.fail("text after the table")
    return v


def plain(v):
    """A read value as config text: numbers keep their spelling, a vector joins with commas."""
    if isinstance(v, Raw):
        return v.text
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, list):
        return ",".join(plain(x) for x in v)
    return v
