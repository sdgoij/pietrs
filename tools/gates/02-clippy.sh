#!/bin/sh
# Lints, with warnings as errors.

set -u

name=clippy

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo clippy --workspace --all-targets -- -D warnings || fail "cargo clippy"

printf 'OK: %s\n' "$name"
