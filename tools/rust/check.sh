#!/usr/bin/env bash
# The rules the code is held to, in one run:
#   - clippy over the workspace and every target, without a warning;
#   - `cargo fmt --check` without a difference;
#   - `unsafe` in `crates/gz-win` only (the count inside is printed);
#   - the editor builds, and `Grizzly.exe -dump_commands` registers every `cmd` of
#     `crates/gz-mcp/tools.json` (checked again only when the executable or the catalogue is newer
#     than its last pass, kept in Intermediate/check-commands.stamp); a binary built from another
#     catalogue fails first, in one sentence with its build command (tools/qa/catalogue.py);
#     GRIZZLY_QA_EXE checks that binary instead.
# Exit code 1 when any of them fails.
set -u
cd "$(dirname "$0")/../.." || exit 2
mkdir -p Intermediate
unset NoDefaultCurrentDirectoryInExePath
bad=0

echo "== clippy"
cargo clippy --workspace --all-targets > Intermediate/check-clippy.log 2>&1
code=$?
warnings=$(grep -cE '^(warning|error)' Intermediate/check-clippy.log)
if [ $code -ne 0 ] || [ "$warnings" -ne 0 ]; then
    grep -E '^(warning|error)' -A 8 Intermediate/check-clippy.log | head -60
    echo "clippy: failed"
    bad=1
else
    echo "clippy: clean"
fi

echo "== fmt"
if cargo fmt --all -- --check > Intermediate/check-fmt.log 2>&1; then
    echo "fmt: clean"
else
    grep '^Diff in' Intermediate/check-fmt.log | head -20
    echo "fmt: differs (run cargo fmt --all)"
    bad=1
fi

echo "== unsafe"
pattern='unsafe[[:space:]]*(\{|fn |impl |extern |trait )'
outside=$(grep -rnE "$pattern" crates --include='*.rs' | grep -v '^crates/gz-win/' || true)
inside=$(grep -rhoE "$pattern" crates/gz-win --include='*.rs' | wc -l)
if [ -n "$outside" ]; then
    printf '%s\n' "$outside" | head -20
    echo "unsafe: found outside crates/gz-win"
    bad=1
else
    echo "unsafe: only in crates/gz-win ($inside uses)"
fi

echo "== file dialog"
dialogs=$(grep -rn 'rfd::' crates --include='*.rs' | grep -v '^crates/gz-ui/src/file_dialog.rs:' || true)
if [ -n "$dialogs" ]; then
    printf '%s\n' "$dialogs" | head -20
    echo "file dialog: rfd:: outside gz-ui/src/file_dialog.rs (use gz_ui::file_dialog::pick)"
    bad=1
else
    echo "file dialog: one kit function"
fi

echo "== build and registry"
# GRIZZLY_QA_EXE names a binary built already (Bin/x64/Release/Grizzly.exe, the release build): its
# registry is the one checked, and the debug build is not made
if [ -n "${GRIZZLY_QA_EXE:-}" ] && [ ! -f "$GRIZZLY_QA_EXE" ]; then
    echo "build: $GRIZZLY_QA_EXE is not built"
    bad=1
elif [ -z "${GRIZZLY_QA_EXE:-}" ] && ! bash tools/rust/cg.sh build -p grizzly; then
    echo "build: failed"
    bad=1
else
    exe=${GRIZZLY_QA_EXE:-Intermediate/cargo/debug/Grizzly.exe}
    stamp=Intermediate/check-commands.stamp
    if [ -f "$stamp" ] && [ ! "$exe" -nt "$stamp" ] && [ ! crates/gz-mcp/tools.json -nt "$stamp" ]; then
        echo "registry: unchanged since its last pass"
    elif ! stale=$(python tools/qa/catalogue.py "$exe"); then
        # a binary built from another tools.json: the same sentence test_mcp_bridge gives for the bridge
        echo "$stale"
        echo "registry: stale binary"
        rm -f "$stamp"
        bad=1
    else
        timeout 60 "$exe" -dump_commands > Intermediate/check-commands.txt 2>&1
        missing=$(grep -o '"cmd": *"[^"]*"' crates/gz-mcp/tools.json | sed -E 's/.*"([^"]*)"$/\1/' | sort -u |
            while read -r c; do grep -q "^$c " Intermediate/check-commands.txt || echo "$c"; done)
        if [ -n "$missing" ]; then
            printf 'not registered: %s\n' $missing
            echo "registry: incomplete"
            rm -f "$stamp"
            bad=1
        else
            echo "registry: every catalogue command is registered"
            touch "$stamp"
        fi
    fi
fi

# ---- the repository's text, the tests' files, the executables' icon ----
# - text encodings: no C1 control character or broken UTF-8, cp1251 where the game's parsers read it
#   (tools/lint/check_encoding.py, the gate the C++ build ran);
# - no test writes a program file (tools/rust/check_test_files.py);
# - Grizzly.exe, Grizzly-Cmd.exe and grizzly-mcp.exe of the release build (Bin/x64/Release, Build.ps1) carry the
#   editor's icon, the editor is a window program, its twin for a command line and the bridge console ones
#   (tools/rust/check_icon.py), checked when they are built; the twin answers what the editor answers, with
#   its exit code.
echo "== encoding"
if python tools/lint/check_encoding.py --quiet; then
    echo "encoding: clean"
else
    echo "encoding: failed (python tools/lint/check_encoding.py --strict lists the files)"
    bad=1
fi

echo "== test files"
if python tools/rust/check_test_files.py crates > Intermediate/check-test-files.txt 2>&1; then
    echo "test files: no test names a program file or writes MZ"
else
    head -20 Intermediate/check-test-files.txt
    echo "test files: program files in test code (a harmless name, or // program-files: in memory)"
    bad=1
fi

echo "== icon"
release=Bin/x64/Release
if [ -f "$release/Grizzly.exe" ] && [ -f "$release/Grizzly-Cmd.exe" ] && [ -f "$release/grizzly-mcp.exe" ]; then
    if python tools/rust/check_icon.py "$release/Grizzly.exe" "$release/Grizzly-Cmd.exe" "$release/grizzly-mcp.exe"; then
        echo "icon: the executables carry the editor's icon; the editor is a window program, its twin and the bridge console ones"
    else
        echo "icon: failed"
        bad=1
    fi
    # the twin runs the editor beside it for a shell: the same answer, the editor's exit code
    twin=$("$release/Grizzly-Cmd.exe" -catalogue 2>&1); twin_code=$?
    editor=$("$release/Grizzly.exe" -catalogue 2>&1)
    "$release/Grizzly-Cmd.exe" -home Intermediate/check-home -compile "$release/no-such-level" > /dev/null 2>&1; refused=$?
    if [ -n "$twin" ] && [ "$twin" = "$editor" ] && [ $twin_code -eq 0 ] && [ $refused -ne 0 ]; then
        echo "command line: Grizzly-Cmd.exe answers as the editor does and ends with its exit code"
    else
        echo "command line: Grizzly-Cmd.exe said '$twin' (exit $twin_code), the editor '$editor'; a refused compile ended with $refused"
        bad=1
    fi
else
    echo "icon: no release build in $release (pwsh -NoProfile -File ./Build.ps1), not checked"
fi

# ---- the third-party notices are what the lock file gives ----
echo "== notices"
python tools/rust/notices.py --check || bad=1

[ $bad -eq 0 ] && echo "check: ok" || echo "check: FAILED"
exit $bad
