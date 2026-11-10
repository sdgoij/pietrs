#!/bin/sh
# The suite: unit and integration tests.

set -u

name=test

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test --workspace || fail "cargo test --workspace"

printf 'OK: %s\n' "$name"
