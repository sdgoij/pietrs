#!/bin/sh
# Licences, advisories, banned crates.

set -u

name=deny

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

if ! command -v cargo-deny >/dev/null 2>&1; then
    fail "cargo-deny is not installed (cargo install cargo-deny --locked)"
fi

cargo deny check || fail "cargo deny check"

printf 'OK: %s\n' "$name"
