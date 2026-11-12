#!/bin/sh
# Documentation.

set -u

name=doc

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test --workspace --doc --release || fail "cargo test --workspace --doc --release"

printf 'OK: %s\n' "$name"
