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
# Exit code 1 on any failure.
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

# the corpus: this machine's game and projects unless the caller named others
if [ -z "${GRIZZLY_TEST_GAME+x}" ] && [ -d "D:/Games/Dead Air/database" ]; then
    export GRIZZLY_TEST_GAME="D:/Games/Dead Air"
fi
if [ -z "${GRIZZLY_TEST_PROJECTS+x}" ]; then
    list=""
    for p in "D:/XFinedProjects/DARF-Overnight-Delivery" "D:/XFinedProjects/Test" "D:/XFinedProjects/WeaponsEvolution"; do
        [ -d "$p" ] && list="${list:+$list;}$p"
    done
    [ -n "$list" ] && export GRIZZLY_TEST_PROJECTS="$list"
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
    for attempt in 1 2; do
        (cd "$dir" && timeout 120 "$exe" > "$root/$log" 2>&1)
        code=$?
        if [ $code -ne 0 ] && grep -q "os error 32" "$root/$log" && [ $attempt -eq 1 ]; then sleep 1; continue; fi
        break
    done
    secs=$(since "$start")
    tests=$(grep -E '^test result' "$root/$log" | tail -1 | sed -E 's/.* ([0-9]+) passed.*/\1/')
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
rm -f Intermediate/test-logs/*.green Intermediate/test-logs/*.failed
cached=$(grep -c '^cached|' Intermediate/test-logs/plan.txt)
ran=$(grep -c '^run|' Intermediate/test-logs/plan.txt)
grep '^cached|' Intermediate/test-logs/plan.txt | awk -F'|' '{ printf "cached %-34s %s tests\n", $2, $3 }'
alone=()
t1=$(now)
while IFS='|' read -r what name dir exe fp; do
    [ "$what" = run ] || continue
    # the renderer's tests make a graphics device: alone, after the rest
    case "$name" in gz-render|gz-render/*) alone+=("$name|$dir|$exe|$fp"); continue ;; esac
    while [ "$(jobs -rp | wc -l)" -ge $jobs_max ]; do wait -n; done
    run_one "$name" "$dir" "$exe" "$fp" &
done < Intermediate/test-logs/plan.txt
wait
for a in "${alone[@]+"${alone[@]}"}"; do
    IFS='|' read -r name dir exe fp <<< "$a"
    run_one "$name" "$dir" "$exe" "$fp"
done
run_secs=$(since "$t1")

# the green fingerprints: this run's, then the earlier ones of binaries it did not run green
failed=$(find Intermediate/test-logs -name '*.failed' | wc -l)
{
    cat Intermediate/test-logs/*.green 2>/dev/null
    find Intermediate/test-logs -name '*.failed' | sed 's/.*\///; s/\.log\.failed$//; s/__/\//g; s/^/FAILED /'
    cat "$stamps"
} | awk '$1 == "FAILED" { bad[$2] = 1; next } !($1 in seen) && !($1 in bad) { seen[$1] = 1; print }' |
    sort > "$stamps.new"
mv "$stamps.new" "$stamps"

total_tests=$(awk '{ s += $3 } END { print s + 0 }' "$stamps")
printf '\nbuild %ss, corpus stamp %ss, run %ss, total %ss: %d binaries run, %d cached, %d failed; %d tests green in all\n' \
    "$build_secs" "$stamp_secs" "$run_secs" "$(since "$t0")" "$ran" "$cached" "$failed" "$total_tests"
[ "$failed" -eq 0 ] || exit 1
