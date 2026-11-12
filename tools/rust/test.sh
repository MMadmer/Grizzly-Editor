#!/usr/bin/env bash
# Runs the workspace's tests: every test binary built once, then run - several at a time, each
# from its package folder as cargo runs it - and timed.
#
#   tools/rust/test.sh            only the binaries that changed since their last green run
#   tools/rust/test.sh --all      every binary (the proof of a whole run)
#   tools/rust/test.sh -p gz-ltx  one package (repeatable); with --all or without
#
# A binary is cached when its fingerprint matches its last green run: the executable (size and
# write time) and, for a corpus test, the corpus variables and a stamp of the folders it reads
# (see `corpus_stamp`; an edit in place that leaves its folder's time alone needs --all). Green
# runs are kept in Intermediate/test-stamps.txt; a failed binary is never cached. A binary that asks for a graphics device runs alone, after the
# others. The corpus variables are set to this machine's data when they are not set already.
# No test writes a program file, held at run time: the binaries' TEMP / TMP is a folder of this
# run's, watched by tools/rust/watch_programs.py while they run (tools/rust/check_test_files.py
# holds the source). Exit code 1 on any failure.
set -u
cd "$(dirname "$0")/../.." || exit 2
root=$(pwd)
mkdir -p Intermediate/test-logs
unset NoDefaultCurrentDirectoryInExePath

all=0
packages=()
while [ $# -gt 0 ]; do
    case "$1" in
        --all) all=1 ;;
        -p) shift; packages+=("$1") ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

# the corpus: this machine's game and projects unless the caller named others; with --all every
# file of it is read (GRIZZLY_TEST_FULL), the default run takes every n-th
set_here=0
if [ -z "${GRIZZLY_TEST_GAME+x}" ] && [ -d "D:/Games/Dead Air/database" ]; then
    export GRIZZLY_TEST_GAME="D:/Games/Dead Air"
    set_here=1
fi
[ $all -eq 1 ] && export GRIZZLY_TEST_FULL=1
if [ -z "${GRIZZLY_TEST_PROJECTS+x}" ]; then
    list=""
    for p in "D:/XFinedProjects/DARF-Overnight-Delivery" "D:/XFinedProjects/Test" "D:/XFinedProjects/WeaponsEvolution"; do
        [ -d "$p" ] && list="${list:+$list;}$p"
    done
    [ -n "$list" ] && export GRIZZLY_TEST_PROJECTS="$list" && set_here=1
fi

now() { date +%s.%N; }
since() { awk -v a="$1" -v b="$(now)" 'BEGIN { printf "%.1f", b - a }'; }

t0=$(now)
pkg_args=()
if [ ${#packages[@]} -eq 0 ]; then pkg_args=(--workspace); else for p in "${packages[@]}"; do pkg_args+=(-p "$p"); done; fi

# build every test binary once; the JSON names each executable
build_log=Intermediate/test-logs/build.log
cargo test "${pkg_args[@]}" --no-run --message-format=json > Intermediate/test-logs/build.json 2> "$build_log"
build_code=$?
if [ $build_code -ne 0 ]; then
    grep -E '^(error|warning)' -A 12 "$build_log" | head -80
    grep -o '"rendered":"[^"]*' Intermediate/test-logs/build.json | head -5 | sed 's/"rendered":"//; s/\\n/\n/g'
    echo "build failed"
    exit 1
fi
build_secs=$(since "$t0")


# name|package folder|executable, one per binary - one pass, a process per binary costs a
# second per 30 binaries on this system
grep '"executable":"[^"]' Intermediate/test-logs/build.json |
    sed -E 's/.*"manifest_path":"([^"]*)","target":\{"kind":\[[^]]*\],"crate_types":\[[^]]*\],"name":"([^"]*)".*"executable":"([^"]*)".*/\1|\2|\3/; s/\\\\/\//g' |
    awk -F'|' '{
        dir = $1; sub(/\/Cargo\.toml$/, "", dir)
        pkg = dir; sub(/.*\//, "", pkg)
        crate = pkg; gsub(/-/, "_", crate)
        name = ($2 == crate) ? pkg : pkg "/" $2
        print name "|" dir "|" $3
    }' | sort > Intermediate/test-logs/binaries.txt

# what a corpus test reads: the variables and a stamp of the folders behind them - each tree's
# folder count and newest folder write time (a file added, removed or saved through a rename
# moves it; walking every file cost 5 s), and the game's archives one by one (size and time)
corpus_stamp() {
    local roots=("$root/rawdata" "$root/gamedata" "$root/docs/nq/examples" "$root/tools/qa/fixtures")
    [ -n "${GRIZZLY_TEST_GAME:-}" ] && roots+=("$GRIZZLY_TEST_GAME/gamedata" "$GRIZZLY_TEST_GAME/modules")
    if [ -n "${GRIZZLY_TEST_PROJECTS:-}" ]; then
        IFS=';' read -ra ps <<< "$GRIZZLY_TEST_PROJECTS"
        roots+=("${ps[@]}")
    fi
    {
        printf '%s|%s\n' "${GRIZZLY_TEST_GAME:-}" "${GRIZZLY_TEST_PROJECTS:-}"
        for r in "${roots[@]}"; do
            [ -d "$r" ] || continue
            find "$r" -name '.*' -prune -o -type d -printf '%T@\n' 2>/dev/null |
                awk -v r="$r" 'BEGIN { n = 0; m = 0 } { n++; if ($1 > m) m = $1 } END { printf "%s %d:%s\n", r, n, m }'
        done
        [ -n "${GRIZZLY_TEST_GAME:-}" ] && find "$GRIZZLY_TEST_GAME/database" -maxdepth 1 -type f -printf '%f %s %T@\n' 2>/dev/null
    } | cksum | cut -d' ' -f1
}


stamps=Intermediate/test-stamps.txt
touch "$stamps"
# the corpus stamp is taken once (it walks the data folders), and only when a corpus test is due
corpus=""
stamp_secs=0
if grep -q '/corpus|' Intermediate/test-logs/binaries.txt; then
    t_stamp=$(now)
    corpus=$(corpus_stamp)
    stamp_secs=$(since "$t_stamp")
fi

# every binary's fingerprint (one stat for all), and what to do with it: run or cached
cut -d'|' -f3 Intermediate/test-logs/binaries.txt | tr '\n' '\0' | xargs -0 stat -c '%s:%Y' > Intermediate/test-logs/sizes.txt 2>/dev/null
paste -d'|' Intermediate/test-logs/binaries.txt Intermediate/test-logs/sizes.txt |
    awk -F'|' -v corpus="$corpus" -v all=$all -v stamps="$stamps" '
        BEGIN { while ((getline l < stamps) > 0) { split(l, f, " "); green[f[1] " " f[2]] = f[3] } }
        {
            fp = $4; if ($1 ~ /\/corpus$/) fp = fp ":" corpus
            if (!all && ((($1 " " fp) in green))) print "cached|" $1 "|" green[$1 " " fp]
            else print "run|" $1 "|" $2 "|" $3 "|" fp
        }' > Intermediate/test-logs/plan.txt

# one binary: run, retry once when the file is still locked (a scanner meeting the new
# executable), record the result
run_one() { # name dir exe fp
    local name=$1 dir=$2 exe=$3 fp=$4 log="Intermediate/test-logs/${1//\//__}.log" start code tests secs
    start=$(now)
    rm -f "$root/$log.skips"
    for attempt in 1 2; do
        (cd "$dir" && TEMP="$watch_win" TMP="$watch_win" GRIZZLY_TEST_SKIPS="$root/$log.skips" timeout 120 "$exe" > "$root/$log" 2>&1)
        code=$?
        if [ $code -ne 0 ] && grep -q "os error 32" "$root/$log" && [ $attempt -eq 1 ]; then sleep 1; continue; fi
        break
    done
    # when it ran: a file the watcher reports is put down to the binaries running at its time
    printf '%s %s %s\n' "$start" "$(now)" "$name" > "$root/$log.span"
    secs=$(since "$start")
    tests=$(grep -E '^test result' "$root/$log" | tail -1 | sed -E 's/.* ([0-9]+) passed.*/\1/')
    # a corpus test that did not read its data says why: shown, never mistaken for a pass
    if [ -s "$root/$log.skips" ]; then
        sort -u "$root/$log.skips" | sed "s|^|skipped $name: |"
    fi
    if [ $code -eq 0 ]; then
        printf 'ok     %-34s %6ss  %s tests\n' "$name" "$secs" "${tests:-0}"
        printf '%s %s %s %s\n' "$name" "$fp" "${tests:-0}" "$secs" > "$root/$log.green"
    else
        printf 'FAILED %-34s %6ss  (exit %d, %s)\n' "$name" "$secs" "$code" "$log"
        grep -E '^---- |panicked at|^test result' "$root/$log" | head -12 | sed 's/^/       /'
        : > "$root/$log.failed"
    fi
}


jobs_max=$(( $(nproc 2>/dev/null || echo 4) / 2 ))
[ $jobs_max -lt 1 ] && jobs_max=1
rm -f Intermediate/test-logs/*.green Intermediate/test-logs/*.failed Intermediate/test-logs/*.skips \
    Intermediate/test-logs/*.span Intermediate/test-logs/watch.txt Intermediate/test-logs/watch.ready
cached=$(grep -c '^cached|' Intermediate/test-logs/plan.txt)
ran=$(grep -c '^run|' Intermediate/test-logs/plan.txt)
grep '^cached|' Intermediate/test-logs/plan.txt | awk -F'|' '{ printf "cached %-34s %s tests\n", $2, $3 }'

# the run-time net (tools/rust/watch_programs.py): the binaries get a temp folder of their own
# (TEMP / TMP), watched through its change notifications while they run; a file created there
# with a program's name or an MZ header fails the run, even one deleted at once
watch_dir=$(mktemp -d /tmp/gztest_XXXXXX)
watch_win=$(cygpath -w "$watch_dir")
watch_failed=0
if [ "$ran" -gt 0 ]; then
    # disowned: the job loop's `jobs` and `wait` see the test binaries alone
    python -I tools/rust/watch_programs.py "$watch_win" Intermediate/test-logs/watch.txt \
        --ready Intermediate/test-logs/watch.ready --remove &
    disown
    t_arm=$(now)
    while [ ! -f Intermediate/test-logs/watch.ready ]; do
        if awk -v a="$t_arm" -v b="$(now)" 'BEGIN { exit !(b - a > 10) }'; then
            echo "FAILED: the run-time net could not watch $watch_win"
            watch_failed=1
            break
        fi
        sleep 0.05
    done
fi
alone=()
t1=$(now)
while IFS='|' read -r what name dir exe fp; do
    [ "$what" = run ] || continue
    # the renderer's tests make a graphics device: alone, after the rest
    case "$name" in gz-render|gz-render/*) alone+=("$name|$dir|$exe|$fp"); continue ;; esac
    while [ "$(jobs -rp | wc -l)" -ge $jobs_max ]; do wait -n; done
    run_one "$name" "$dir" "$exe" "$fp" &
# the corpus binaries take longest: they start first, the short ones fill the cores beside them
done < <(grep '/corpus|' Intermediate/test-logs/plan.txt; grep -v '/corpus|' Intermediate/test-logs/plan.txt)
wait
for a in "${alone[@]+"${alone[@]}"}"; do
    IFS='|' read -r name dir exe fp <<< "$a"
    run_one "$name" "$dir" "$exe" "$fp"
done
run_secs=$(since "$t1")

# the net's verdict: the watcher stops at `.stop`, removes its folder and writes its report last
watch_note=""
if [ "$ran" -gt 0 ] && [ $watch_failed -eq 0 ]; then
    : > "$watch_dir/.stop"
    t_stop=$(now)
    while [ ! -f Intermediate/test-logs/watch.txt ]; do
        if awk -v a="$t_stop" -v b="$(now)" 'BEGIN { exit !(b - a > 60) }'; then
            echo "FAILED: the run-time net did not report"
            watch_failed=1
            break
        fi
        sleep 0.05
    done
    if [ -f Intermediate/test-logs/watch.txt ]; then
        # a finding with the binaries that ran at its time (a second either side)
        cat Intermediate/test-logs/*.span 2>/dev/null > Intermediate/test-logs/spans.txt
        awk -F'|' -v spans=Intermediate/test-logs/spans.txt '
            BEGIN { while ((getline l < spans) > 0) { split(l, f, " "); n++; a[n] = f[1]; b[n] = f[2]; who[n] = f[3] } }
            NF == 3 {
                list = ""
                for (i = 1; i <= n; i++) if ($1 >= a[i] - 1 && $1 <= b[i] + 1) list = list (list == "" ? "" : ", ") who[i]
                printf "FAILED: a test wrote a program file: %s (%s) - while %s ran\n", $2, $3, (list == "" ? "no binary" : list)
                for (i = 1; i <= n; i++) if ($1 >= a[i] - 1 && $1 <= b[i] + 1) print who[i] > "/dev/stderr"
            }' Intermediate/test-logs/watch.txt > Intermediate/test-logs/watch_findings.txt 2> Intermediate/test-logs/watch_blame.txt
        cat Intermediate/test-logs/watch_findings.txt
        [ -s Intermediate/test-logs/watch_findings.txt ] && watch_failed=1
        watch_note=$(awk '$1 == "events" { e = $2 } $1 == "overflows" { o = $2 } $1 == "cpu" { c = $2 }
            END { printf "; the net: %d events, %d overflows, %s s of CPU", e, o, c }' Intermediate/test-logs/watch.txt)
    fi
fi
[ -d "$watch_dir" ] && [ $watch_failed -eq 0 ] && rmdir "$watch_dir" 2>/dev/null
# a binary the net blames is no green run (never cached); a finding nobody can be blamed for, or a
# net that did not watch, takes every green of this run
if [ $watch_failed -eq 1 ]; then
    if [ -s Intermediate/test-logs/watch_blame.txt ]; then
        sort -u Intermediate/test-logs/watch_blame.txt | while read -r n; do
            rm -f "Intermediate/test-logs/${n//\//__}.log.green"
            : > "Intermediate/test-logs/${n//\//__}.log.failed"
        done
    else
        rm -f Intermediate/test-logs/*.green
    fi
fi

# the green fingerprints: this run's, then the earlier ones of binaries it did not run green
failed=$(( $(find Intermediate/test-logs -name '*.failed' | wc -l) + watch_failed ))
{
    cat Intermediate/test-logs/*.green 2>/dev/null
    find Intermediate/test-logs -name '*.failed' | sed 's/.*\///; s/\.log\.failed$//; s/__/\//g; s/^/FAILED /'
    cat "$stamps"
} | awk '$1 == "FAILED" { bad[$2] = 1; next } !($1 in seen) && !($1 in bad) { seen[$1] = 1; print }' |
    sort > "$stamps.new"
mv "$stamps.new" "$stamps"

total_tests=$(awk '{ s += $3 } END { print s + 0 }' "$stamps")
skips=$(cat Intermediate/test-logs/*.skips 2>/dev/null | wc -l)
# --all on a machine whose data this script found: a corpus test that skipped did not prove anything
if [ $all -eq 1 ] && [ $set_here -eq 1 ] && [ "$skips" -gt 0 ]; then
    echo "FAILED: $skips corpus skip(s) in a full run with the corpus variables set by this script"
    failed=$((failed + 1))
fi
printf '\nbuild %ss, corpus stamp %ss, run %ss, total %ss: %d binaries run, %d cached, %d failed, %d skipped; %d tests green in all%s\n' \
    "$build_secs" "$stamp_secs" "$run_secs" "$(since "$t0")" "$ran" "$cached" "$failed" "$skips" "$total_tests" "$watch_note"
[ "$failed" -eq 0 ] || exit 1
