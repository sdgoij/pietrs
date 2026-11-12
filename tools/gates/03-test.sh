#!/bin/sh
# The suite: unit and integration tests.
#
# Release: the slow cases are interpreter work, not compile-time diagnostics.
# A debug build turns a fifteen-minute gate into a coffee break; the release
# profile pays one build and runs the same tests in minutes.

set -u

name=test

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test --workspace --release || fail "cargo test --workspace --release"

printf 'OK: %s\n' "$name"
