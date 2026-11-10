#!/bin/sh
# Run every case in tools/gates/ and print one verdict line for each.
#
# A case prints `OK: <name>` or `FAIL: <name>` and exits zero only on success.
# The runner writes the case's full output to target/gate-logs/<case>.log and
# fails when a case prints FAIL, exits non-zero after OK, or never prints a
# verdict. Set GATE_CASES to run a different directory; the protocol does not
# change.

set -u

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd) || exit 1
cd "$root" || exit 1

cases_dir=${GATE_CASES:-"$root/tools/gates"}
logs="$root/target/gate-logs"
rm -rf "$logs"
mkdir -p "$logs"

status=0
count=0

# CI redirects each case to a log the runner cannot see, so a FAIL line alone
# leaves nothing to debug. Print the tail of the case that failed; the verdict
# lines stay one per case.
show_log() {
    printf '  --- %s ---\n' "$1"
    tail -n 100 "$1" | sed 's/^/  /'
}

for case in "$cases_dir"/*.sh; do
    [ -f "$case" ] || continue
    count=$((count + 1))
    name=$(basename "$case" .sh)
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
