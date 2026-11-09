#!/usr/bin/env bash
# Runs cargo from the repository root with its output in a log, and prints what matters:
# errors, warnings and the last line.
#
#   tools/rust/cg.sh check -p gz-base
#
# Before it runs, a crate folder that is still being written (no manifest or no lib.rs yet) gets
# a placeholder, so one unfinished crate does not stop the whole workspace from loading.
set -u
cd "$(dirname "$0")/../.." || exit 2
mkdir -p Intermediate

for d in crates/*/; do
    n=$(basename "$d")
    if [ ! -f "$d/Cargo.toml" ]; then
        printf '[package]\nname = "%s"\nversion.workspace = true\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\npublish.workspace = true\n\n[lints]\nworkspace = true\n' "$n" > "$d/Cargo.toml"
    fi
    if [ ! -f "$d/src/lib.rs" ] && [ ! -f "$d/src/main.rs" ]; then
        mkdir -p "$d/src"
        printf '//! Being written.\n' > "$d/src/lib.rs"
    fi
done

log="Intermediate/cg-$$.log"
cargo "$@" > "$log" 2>&1
code=$?
grep -E '^(error|warning)' -A 16 "$log" | head -"${CG_LINES:-120}"
tail -1 "$log"
rm -f "$log"
exit $code
