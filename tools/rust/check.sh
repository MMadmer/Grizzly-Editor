#!/usr/bin/env bash
# The rules the code is held to, in one run. The editor has three entry points that prove a change, and
# this is the first: `bash tools/rust/check.sh` (the rules), `bash tools/rust/test.sh` (the unit and
# corpus tests), `python tools/qa/run_all.py` (the QA suites against the built editor).
#   - clippy over the workspace and every target, without a warning;
#   - `cargo fmt --check` without a difference;
#   - `unsafe` in `crates/gz-win` only (the count inside is printed);
#   - the editor builds and answers `help` from a shell (the commands document themselves: the
#     registry's own test in crates/grizzly holds every card to the rules); GRIZZLY_QA_EXE checks
#     that binary instead;
#   - no command without a card: the registry has no way to add one (`Registry::cmd` takes a card and a
#     parameter type) and `help` names none without;
#   - the text encodings, the tests' files, the window's text through the kit (tools/rust/check_ui_text.py), the
#     executables' icon, the third-party notices;
#   - no forbidden word in the code (tools/rust/check_words.py), the last step.
# Exit code 1 when any of them fails.
set -u
cd "$(dirname "$0")/../.." || exit 2
mkdir -p Intermediate
unset NoDefaultCurrentDirectoryInExePath
bad=0
# every program this script starts runs under a time limit and is ended with what it started
limited="python tools/rust/limited.py"

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

echo "== build and commands"
# GRIZZLY_QA_EXE names a binary built already (Bin/x64/Release/Grizzly.exe, the release build): it is the
# one asked, and the debug build is not made
if [ -n "${GRIZZLY_QA_EXE:-}" ] && [ ! -f "$GRIZZLY_QA_EXE" ]; then
    echo "build: $GRIZZLY_QA_EXE is not built"
    bad=1
elif [ -z "${GRIZZLY_QA_EXE:-}" ] && ! bash tools/rust/cg.sh build -p grizzly -p gz-mcp-bridge; then
    echo "build: failed"
    bad=1
else
    exe=${GRIZZLY_QA_EXE:-Intermediate/cargo/debug/Grizzly.exe}
    twin="$(dirname "$exe")/Grizzly-Cmd.exe"
    # a commandlet: no window, no port, its own log; the answer on stdout and 0
    if listing=$($limited 60 "$twin" help -home Intermediate/check-home 2>&1) && echo "$listing" | grep -q "^Areas$"; then
        if echo "$listing" | grep -q "^Without a card"; then
            echo "$listing" | grep "^Without a card" | cut -c1-300
            echo "commands: some command has no card"
            bad=1
        else
            echo "commands: help answers, every command has a card"
        fi
    else
        echo "$listing" | head -5
        echo "commands: Grizzly-Cmd.exe help did not answer"
        bad=1
    fi
fi

# ---- the registry's door, the repository's text, the tests' files, the executables' icon ----
# - no untyped registration: the way to add a command without a card is gone from gz-editor;
# - text encodings: no C1 control character or broken UTF-8, cp1251 where the game's parsers read it
#   (tools/rust/check_encoding.py);
# - no test writes a program file (tools/rust/check_test_files.py);
# - Grizzly.exe, Grizzly-Cmd.exe and grizzly-mcp.exe of the release build (Bin/x64/Release, Build.ps1) carry the
#   editor's icon, the editor is a window program, its twin for a command line and the bridge console ones
#   (tools/rust/check_icon.py), checked when they are built; the twin answers what the editor answers, with
#   its exit code.
echo "== untyped registry"
untyped=$(grep -nE 'pub fn add(_sync)?[<(]|fn bare_effect|trait ArgsExt' crates/gz-editor/src/*.rs || true)
if [ -n "$untyped" ]; then
    printf '%s\n' "$untyped" | head -10
    echo "untyped registry: a command can be registered without a card (Registry::add / add_sync, ArgsExt, bare_effect)"
    bad=1
else
    echo "untyped registry: a command is registered with its card and its parameter type, or not at all"
fi

echo "== encoding"
if python tools/rust/check_encoding.py --quiet; then
    echo "encoding: clean"
else
    echo "encoding: failed (python tools/rust/check_encoding.py --strict lists the files)"
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

echo "== ui text"
# the window's language: egui's own text calls translate nothing, so a surface draws text through the kit
if python tools/rust/check_ui_text.py > Intermediate/check-ui-text.txt 2>&1; then
    echo "ui text: every surface draws its text through the kit's Tx"
else
    head -20 Intermediate/check-ui-text.txt
    echo "ui text: egui text calls past the kit's language boundary (docs/UI_DESIGN.md section 9)"
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
    # a release build older than the commandlets would open a window and wait: every run is limited
    twin=$($limited 30 "$release/Grizzly-Cmd.exe" run ping -json -home Intermediate/check-home 2>&1); twin_code=$?
    editor=$($limited 30 "$release/Grizzly.exe" run ping -json -home Intermediate/check-home 2>&1)
    # a command that fails ends the twin with 1, as it ends the editor
    $limited 30 "$release/Grizzly-Cmd.exe" run level.compile folder="$release/no-such-level" -home Intermediate/check-home > /dev/null 2>&1; refused=$?
    if [ -n "$twin" ] && [ "$twin" = "$editor" ] && [ $twin_code -eq 0 ] && [ $refused -eq 1 ]; then
        echo "command line: Grizzly-Cmd.exe answers as the editor does and ends with its exit code"
    else
        echo "command line: Grizzly-Cmd.exe said '$twin' (exit $twin_code), the editor '$editor'; a refused compile ended with $refused"
        [ $twin_code -eq 124 ] && echo "command line: the release build in $release is older than the sources - pwsh -NoProfile -File ./Build.ps1"
        bad=1
    fi
else
    echo "icon: no release build in $release (pwsh -NoProfile -File ./Build.ps1), not checked"
fi

# ---- the third-party notices are what the lock file gives ----
echo "== notices"
python tools/rust/notices.py --check || bad=1

# ---- no word of the program the editor replaced, no command it no longer has ----
echo "== words"
if python tools/rust/check_words.py; then
    echo "words: clean"
else
    echo "words: failed (python tools/rust/check_words.py lists the hits)"
    bad=1
fi

[ $bad -eq 0 ] && echo "check: ok" || echo "check: FAILED"
exit $bad
