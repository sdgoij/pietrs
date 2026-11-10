#!/bin/sh
# The long cases: conformance, differential, and end-to-end.

set -u

name=long-tests

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test --workspace -- --include-ignored || fail "cargo test --include-ignored"

printf 'OK: %s\n' "$name"
