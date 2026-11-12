#!/bin/sh
# The feature shapes: the interpreter alone, and codegen without object emission.
#
# Release, so these reuse the profile the suite and the long cases already built.

set -u

name=features

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo test -p cli --no-default-features --release || fail "cargo test -p cli --no-default-features --release"
cargo test -p cli --no-default-features --features codegen --release || fail "cargo test -p cli --no-default-features --features codegen --release"
cargo test -p cli --no-default-features --features editor --release || fail "cargo test -p cli --no-default-features --features editor --release"
cargo test -p gui --no-default-features --release || fail "cargo test -p gui --no-default-features --release"
cargo test -p codegen --no-default-features --release || fail "cargo test -p codegen --no-default-features --release"

printf 'OK: %s\n' "$name"
