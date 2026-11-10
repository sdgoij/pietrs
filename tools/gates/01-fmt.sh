#!/bin/sh
# Formatting and whitespace.

set -u

name=fmt

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo fmt --all -- --check || fail "cargo fmt"
git --no-pager diff --check || fail "git diff --check"

printf 'OK: %s\n' "$name"
