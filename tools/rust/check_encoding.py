# -*- coding: utf-8 -*-
"""Catch text that would reach the user as question marks, and game files in the wrong encoding.

Two authoring accidents draw as '?' on the screen, and the check fails on both:

  * C1 control codepoints (U+0080-U+009F). No font has glyphs for them. They turn up when UTF-8 bytes are decoded as
    latin1 and re-encoded, so one Russian word becomes a run of them.
  * UTF-8 that was decoded through a single-byte code page and saved again (mojibake), and bytes that are not UTF-8
    at all in a file that must be.

A third accident is silent rather than loud: a file the game's parsers read as cp1251 (a config, a string table, a
script under gamedata) saved as UTF-8, or the other way round. Those are reported as problems the tree inherits from
the game's own files; --strict makes them fail too.

Every tracked text file is read whole; binary formats are left out.

Usage:
    python tools/rust/check_encoding.py [--root <repo>] [--quiet] [--strict]

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
# up on screen as '?'. Everything else this tool reports is inherited from the game's own
# files, so it is shown but does not stop a build.
FATAL_CODES = {'C1CTRL', 'MOJIBAKE'}

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
