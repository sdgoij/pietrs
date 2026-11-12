#!/bin/sh
# The long cases: conformance, differential, and end-to-end.
#
# Release for the same reason as the suite: the S4 image sweeps and the
# interpreted walkers are hundreds of millions of steps, and optimised code is
# where that cost belongs.

set -u

name=long-tests

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test --workspace --release -- --include-ignored || fail "cargo test --workspace --release --include-ignored"

printf 'OK: %s\n' "$name"
