#!/bin/sh
# Documentation.

set -u

name=doc

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test --workspace --doc || fail "cargo test --doc"

printf 'OK: %s\n' "$name"
