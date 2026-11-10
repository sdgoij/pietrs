#!/bin/sh
# The browser shape: `gui` with the `web` shell, compiled and linked for wasm.
#
# This is a build, not a browser test. It proves the target links against the
# egui/eframe stack and that the wasm entry is exported. Producing the bundle
# (`wasm-bindgen` plus the page) is `cargo run -p xtask -- wasm`, a developer
# and release step, because it needs a tool the build does not vendor.

set -u

name=wasm

fail() {
    printf 'FAIL: %s (%s)\n' "$name" "$1"
    exit 1
}

cargo build -p gui --no-default-features --features web --target wasm32-unknown-unknown ||
    fail "cargo build -p gui --features web --target wasm32-unknown-unknown"

printf 'OK: %s\n' "$name"
