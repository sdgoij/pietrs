#!/bin/sh
# The feature shapes: the interpreter alone, and codegen without object emission.

set -u

name=features

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test -p cli --no-default-features || fail "cargo test -p cli --no-default-features"
cargo test -p codegen --no-default-features || fail "cargo test -p codegen --no-default-features"

printf 'OK: %s\n' "$name"
