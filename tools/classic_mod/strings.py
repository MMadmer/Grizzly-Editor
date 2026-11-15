"""String tables (`configs\\text\\<lang>\\*.xml`): what the mod adds or changes, per id, against its base."""
from __future__ import annotations

import html
import re

_COMMENT = re.compile(rb"<!--.*?-->", re.S)
_STRING = re.compile(rb'<string\s+id\s*=\s*"([^"]*)"\s*>(.*?)</string>', re.S)
_TEXT = re.compile(rb"<text>(.*?)</text>", re.S)


def table(data: bytes) -> dict[bytes, bytes]:
    out = {}
    for sid, body in _STRING.findall(_COMMENT.sub(b"", data)):
        t = _TEXT.search(body)
        out[sid] = t.group(1) if t else b""
    return out


def languages(view) -> list[str]:
    return sorted({n.split("\\")[2] for n in view.names if n.startswith("configs\\text\\") and n.count("\\") == 3 and n.endswith(".xml")})


def merged(view, lang: str) -> dict[bytes, bytes]:
    """The language's table as the game folds it: the files in name order, a later file wins."""
    folder = f"configs\\text\\{lang}\\"
    out: dict[bytes, bytes] = {}
    for n in sorted(x for x in view.names if x.startswith(folder) and x.endswith(".xml")):
        data = view.read(n)
        if data:
            out.update(table(data))
    return out


def intent(base_view, mod_view, game_view, lang):
    """(wanted ids -> text, ids the mod carries from its base only, ids whose text the game already has)."""
    b, m, g = merged(base_view, lang), merged(mod_view, lang), merged(game_view, lang)
    want, same_as_game, rolled = {}, [], 0
    for sid, text in m.items():
        if b.get(sid) == text:
            if g.get(sid) != text:
                rolled += 1
            continue
        if g.get(sid) == text:
            same_as_game.append(sid)
            continue
        want[sid] = text
    return want, rolled, same_as_game, m


def plain_text(raw: bytes) -> str:
    """The text a player reads, as an asset carries it (entities decoded, the game's `\\n` kept)."""
    return html.unescape(raw.decode("cp1251", "surrogateescape"))


def write_table(ids: dict[bytes, bytes]) -> bytes:
    body = [b'<?xml version="1.0" encoding="windows-1251"?>', b"<string_table>"]
    for sid in sorted(ids):
        body.append(b'\t<string id="' + sid + b'">\r\n\t\t<text>' + ids[sid] + b"</text>\r\n\t</string>")
    body.append(b"</string_table>\r\n")
    return b"\r\n".join(body)
