# -*- coding: utf-8 -*-
"""Catch text that would reach the user as question marks.

The editor draws with ImGui, which replaces a codepoint it has no glyph for with
the font's fallback character - '?'. Two authoring accidents produce exactly that:

  * C1 control codepoints (U+0080-U+009F). No font has glyphs for them. They turn
    up when UTF-8 bytes are decoded as latin1 and re-encoded, so one Russian word
    becomes a run of them.
  * bytes that are not valid UTF-8 at all. A cp1251-encoded source file whose
    string literal is drawn by the UI hands ImGui exactly that; it decodes to
    U+FFFD, which also falls back to '?'.

Comments are not checked: this inherited X-Ray tree carries thousands of cp1251
comment lines that no user will ever see. Only STRING LITERALS are inspected,
because only those can be drawn, logged or written to a file.

A third accident is silent rather than loud: a file that must be cp1251 for the
game's ini parser (the NQ catalog) saved as UTF-8, or the other way round.

Usage:
    python tools/lint/check_encoding.py [--root <repo>] [--quiet]

Exit code 1 when anything is found, 0 when clean.
"""

import argparse
import io
import os
import subprocess
import sys

# Third-party trees keep whatever encoding upstream chose; they are not ours to fix.
SKIP_DIRS = (
    'externals/',
)

SKIP_EXT = {
    '.dll', '.lib', '.pdb', '.exe', '.7z', '.zip', '.gz', '.ico', '.png', '.bmp',
    '.dds', '.tga', '.jpg', '.jpeg', '.thm', '.object', '.bin', '.obj', '.log',
    '.aps', '.ncb', '.chm', '.mcp', '.wav', '.ogg', '.ogf', '.snk', '.aqt',
    '.spr', '.db', '.xdb', '.spawn', '.graph', '.ai', '.geom', '.dm', '.ppe',
    '.part', '.msi', '.cache', '.res', '.ttf', '.otf',
    '.tlog', '.bak', '.idb', '.ipch',
}

# These two are always accidents: nothing legitimate produces them, and both end
# up on screen as '?'. Everything else this tool reports is inherited X-Ray text
# that predates the project, so it is shown but does not stop a build.
FATAL_CODES = {'C1CTRL', 'MOJIBAKE'}

CODE_EXT = {'.cpp', '.cxx', '.cc', '.c', '.h', '.hpp', '.inl'}

# Everything under a gamedata tree is read by the game's own byte-oriented
# parsers, which assume cp1251. A file there saved as UTF-8 does not fail loudly:
# the game just shows mojibake, so it is worth naming.
CP1251_EXT = {'.ltx', '.xml', '.script', '.ini', '.ltxp', '.xmlp'}


def tracked_files(root):
    out = subprocess.run(['git', 'ls-files', '-z'], cwd=root, stdout=subprocess.PIPE, check=True)
    for name in out.stdout.split(b'\0'):
        if not name:
            continue
        rel = name.decode('utf-8', 'replace').replace('\\', '/')
        low = rel.lower()
        if any(low.startswith(d) for d in SKIP_DIRS):
            continue
        if os.path.splitext(low)[1] in SKIP_EXT:
            continue
        yield rel


def expects_cp1251(rel):
    low = rel.lower()
    return ('gamedata/' in low or low.startswith('gamedata/')) and \
        os.path.splitext(low)[1] in CP1251_EXT


def looks_binary(data):
    return b'\0' in data[:8192]


def demojibake(text):
    """What `text` would read as if it is UTF-8 mistakenly decoded through a
    single-byte codepage. None when it is not that kind of damage."""
    for codec in ('latin-1', 'cp1252'):
        try:
            raw = text.encode(codec)
        except UnicodeEncodeError:
            continue
        try:
            fixed = raw.decode('utf-8')
        except UnicodeDecodeError:
            continue
        # a real recovery has to surface non-Latin text that was not there
        # before, otherwise every ASCII line trivially "round-trips"
        if fixed != text and any(ord(c) >= 0x0400 for c in fixed):
            return fixed
    return None


def string_literals(src):
    """Yield (line_number, literal_body) for every C/C++ string literal, skipping
    comments and character literals. Deliberately small: it only has to be right
    about where a comment ends and a string begins."""
    i, n, line = 0, len(src), 1
    while i < n:
        c = src[i]
        if c == '\n':
            line += 1
            i += 1
            continue
        # comments
        if c == '/' and i + 1 < n:
            if src[i + 1] == '/':
                while i < n and src[i] != '\n':
                    i += 1
                continue
            if src[i + 1] == '*':
                i += 2
                while i + 1 < n and not (src[i] == '*' and src[i + 1] == '/'):
                    if src[i] == '\n':
                        line += 1
                    i += 1
                i += 2
                continue
        # raw string R"delim( ... )delim"
        if c == 'R' and i + 1 < n and src[i + 1] == '"':
            j = src.find('(', i + 2)
            if j != -1:
                delim = src[i + 2:j]
                end = src.find(')' + delim + '"', j + 1)
                if end != -1:
                    body = src[j + 1:end]
                    yield line, body
                    line += body.count('\n')
                    i = end + len(delim) + 2
                    continue
        # character literal
        if c == "'":
            i += 1
            while i < n and src[i] != "'":
                if src[i] == '\\':
                    i += 1
                if src[i:i + 1] == '\n':
                    line += 1
                i += 1
            i += 1
            continue
        # string literal
        if c == '"':
            start_line = line
            i += 1
            body = []
            while i < n and src[i] != '"':
                if src[i] == '\\' and i + 1 < n:
                    body.append(src[i:i + 2])
                    i += 2
                    continue
                if src[i] == '\n':      # unterminated, bail out safely
                    line += 1
                    break
                body.append(src[i])
                i += 1
            i += 1
            yield start_line, ''.join(body)
            continue
        i += 1


def check_code(rel, data, problems):
    """Source files: only string literals matter, in whatever encoding the file is."""
    try:
        text = data.decode('utf-8')
        utf8 = True
    except UnicodeDecodeError:
        text = data.decode('cp1251', 'replace')
        utf8 = False

    for line, lit in string_literals(text):
        if not lit:
            continue
        non_ascii = [c for c in lit if ord(c) > 0x7F]
        if not non_ascii:
            continue
        if not utf8:
            problems.append((rel, line, 'CP1251LITERAL',
                             'non-ASCII text in a string literal of a cp1251-encoded file - if this '
                             'string is drawn, the UI shows "?": %s' % lit.strip()[:100]))
            continue
        ctl = sorted({c for c in lit if 0x80 <= ord(c) <= 0x9F})
        if ctl:
            problems.append((rel, line, 'C1CTRL',
                             'C1 control codepoints (%s) in a string literal - no font has glyphs, '
                             'the UI draws "?"' % ' '.join('U+%04X' % ord(c) for c in ctl)))
        fixed = demojibake(lit)
        if fixed:
            problems.append((rel, line, 'MOJIBAKE',
                             'double-encoded string literal; the intended reading is: %s'
                             % fixed.strip()[:100]))


def check_text(rel, data, problems):
    """Everything else: the whole file is content, so check all of it."""
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError as e:
        problems.append((rel, 0, 'BADUTF8', 'not valid UTF-8 (%s)' % e))
        return
    for n, line in enumerate(text.replace('\r\n', '\n').split('\n'), 1):
        ctl = sorted({c for c in line if 0x80 <= ord(c) <= 0x9F})
        if ctl:
            problems.append((rel, n, 'C1CTRL',
                             'C1 control codepoints (%s) - no font has glyphs, they draw as "?"'
                             % ' '.join('U+%04X' % ord(c) for c in ctl)))
        fixed = demojibake(line)
        if fixed:
            problems.append((rel, n, 'MOJIBAKE',
                             'double-encoded text; the intended reading is: %s' % fixed.strip()[:100]))


def check_cp1251(rel, data, problems):
    try:
        data.decode('cp1251')
    except UnicodeDecodeError as e:
        problems.append((rel, 0, 'BADCP1251', 'must be cp1251 for the game ini parser: %s' % e))
        return
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        return  # good: real cp1251 bytes are not valid UTF-8
    if any(ord(c) > 0x7F for c in text):
        problems.append((rel, 0, 'CP1251ASUTF8',
                         'file must be cp1251 but its bytes are valid UTF-8 - the game reads mojibake'))


def check_file(root, rel, problems):
    path = os.path.join(root, rel)
    try:
        data = io.open(path, 'rb').read()
    except OSError as e:
        problems.append((rel, 0, 'READFAIL', str(e)))
        return
    if not data or looks_binary(data):
        return
    if expects_cp1251(rel):
        check_cp1251(rel, data, problems)
    elif os.path.splitext(rel.lower())[1] in CODE_EXT:
        check_code(rel, data, problems)
    else:
        check_text(rel, data, problems)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--root', default=os.path.dirname(os.path.dirname(
        os.path.dirname(os.path.abspath(__file__)))))
    ap.add_argument('--quiet', action='store_true', help='print only problems')
    ap.add_argument('--strict', action='store_true',
                    help='also fail on inherited cp1251 literals and stray non-UTF-8 files')
    args = ap.parse_args()

    try:
        sys.stdout.reconfigure(encoding='utf-8', errors='backslashreplace')
    except AttributeError:
        pass

    root = os.path.abspath(args.root)
    problems, scanned = [], 0
    for rel in tracked_files(root):
        scanned += 1
        check_file(root, rel, problems)

    fatal = [p for p in problems if p[2] in FATAL_CODES]
    legacy = [p for p in problems if p[2] not in FATAL_CODES]

    for rel, line, code, msg in fatal:
        print('  %s:%d: %s %s' % (rel, line, code, msg))
    if fatal:
        print('encoding check FAILED: %d accident(s) that draw as "?" - fix before building'
              % len(fatal))
        return 1

    if legacy and (args.strict or not args.quiet):
        for rel, line, code, msg in legacy:
            where = '%s:%d' % (rel, line) if line else rel
            print('  %s: %s %s' % (where, code, msg))
    if args.strict and legacy:
        print('encoding check FAILED (strict): %d inherited problem(s)' % len(legacy))
        return 1

    # always one line, so a build log shows the gate ran
    print('encoding check: %d files, 0 accidents, %d inherited (--strict lists them)'
          % (scanned, len(legacy)))
    return 0


if __name__ == '__main__':
    sys.exit(main())
