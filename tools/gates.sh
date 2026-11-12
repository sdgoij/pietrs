#!/bin/sh
# Run the gate cases in tools/gates/ and print one verdict line for each.
#
# A case prints `OK: <name>` or `FAIL: <name>` and exits zero only on success.
# The runner writes the case's full output to target/gate-logs/<case>.log and
# fails when a case prints FAIL, exits non-zero after OK, or never prints a
# verdict. Set GATE_CASES to run a different directory; the protocol does not
# change.
#
# With arguments, only the named cases run: `sh tools/gates.sh 03-test`.
# That is the local loop; no argument is the whole gate, which is what CI runs
# and what a push should be preceded by.

set -u

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd) || exit 1
cd "$root" || exit 1

cases_dir=${GATE_CASES:-"$root/tools/gates"}
logs="$root/target/gate-logs"
rm -rf "$logs"
mkdir -p "$logs"

status=0
count=0

# A named case that does not exist is a typo, not a silently empty run.
for name in "$@"; do
    if [ ! -f "$cases_dir/$name.sh" ]; then
        printf 'FAIL: no gate case called %s\n  cases: ' "$name"
        for case in "$cases_dir"/*.sh; do
            [ -f "$case" ] && printf '%s ' "$(basename "$case" .sh)"
        done
        printf '\n'
        exit 1
    fi
done

# CI redirects each case to a log the runner cannot see, so a FAIL line alone
# leaves nothing to debug. Print the tail of the case that failed; the verdict
# lines stay one per case.
show_log() {
    printf '  --- %s ---\n' "$1"
    tail -n 100 "$1" | sed 's/^/  /'
}

for case in "$cases_dir"/*.sh; do
    [ -f "$case" ] || continue
    name=$(basename "$case" .sh)
    if [ "$#" -gt 0 ]; then
        any=0
        for chosen in "$@"; do
            [ "$chosen" = "$name" ] && any=1
        done
        [ "$any" -eq 1 ] || continue
    fi
    count=$((count + 1))
    log="$logs/$name.log"
    if sh "$case" >"$log" 2>&1; then
        code=0
    else
        code=$?
    fi
    verdict=$(grep -E '^(OK|FAIL): ' "$log" | tail -n 1)
    case "$verdict" in
        "OK: "*)
            if [ "$code" -eq 0 ]; then
                printf '%s\n' "$verdict"
            else
                printf 'FAIL: %s (exit %s after its verdict)\n' "$name" "$code"
                show_log "$log"
                status=1
            fi
            ;;
        "FAIL: "*)
            printf '%s\n' "$verdict"
            show_log "$log"
            status=1
            ;;
        *)
            printf 'FAIL: %s (no verdict)\n' "$name"
            show_log "$log"
            status=1
            ;;
    esac
done

if [ "$count" -eq 0 ]; then
    printf 'FAIL: no gate cases in %s\n' "$cases_dir"
    exit 1
fi

if [ "$status" -ne 0 ]; then
    printf 'gate failed; logs are in %s\n' "$logs"
fi
exit "$status"
