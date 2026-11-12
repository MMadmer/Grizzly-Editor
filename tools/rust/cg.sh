#!/usr/bin/env bash
# Runs cargo from the repository root with its output in a log, and prints what matters:
# errors, warnings and the last line.
#
#   tools/rust/cg.sh check -p gz-base
#
# Before it runs, a BRAND-NEW crate folder that is still being written (no manifest or no lib.rs
# yet) gets a placeholder, so one unfinished crate does not stop the whole workspace from
# loading. A placeholder is only ever CREATED, never written over a file: the creation is
# exclusive, and a file git already tracks is never touched - several authors run this script
# at once, and one of them may be rewriting that very file.
set -u
cd "$(dirname "$0")/../.." || exit 2
mkdir -p Intermediate
# LuaJIT's build runs a helper from its own folder; this variable of some shells forbids that.
unset NoDefaultCurrentDirectoryInExePath

# creates a file with the given text only if no file is there at the moment of creation
create_new() {
    ( set -o noclobber; printf '%b' "$2" > "$1" ) 2>/dev/null
}

for d in crates/*/; do
    d=${d%/}
    n=$(basename "$d")
    if [ ! -f "$d/Cargo.toml" ] && ! git ls-files --error-unmatch "$d/Cargo.toml" >/dev/null 2>&1; then
        create_new "$d/Cargo.toml" "[package]\nname = \"$n\"\nversion.workspace = true\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\npublish.workspace = true\n\n[lints]\nworkspace = true\n"
    fi
    if [ ! -f "$d/src/lib.rs" ] && [ ! -f "$d/src/main.rs" ] \
        && ! git ls-files --error-unmatch "$d/src/lib.rs" >/dev/null 2>&1 \
        && ! git ls-files --error-unmatch "$d/src/main.rs" >/dev/null 2>&1; then
        mkdir -p "$d/src"
        create_new "$d/src/lib.rs" "//! Being written.\n"
    fi
done

log="Intermediate/cg-$$.log"
cargo "$@" > "$log" 2>&1
code=$?
grep -E '^(error|warning)' -A 16 "$log" | head -"${CG_LINES:-120}"
# a test run: its failures and its counts
grep -E '^test .* FAILED$|panicked at|^test result:' "$log" | head -"${CG_LINES:-120}"
tail -1 "$log"
rm -f "$log"
exit $code
