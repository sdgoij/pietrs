//! Static musl has no separate `libdl`: `dlopen`, `dlsym`, and `dlerror` live in
//! libc. `libloading` (through `wayland-sys`, `x11-dl`, and `xkbcommon-dl`) still
//! asks the linker for `-ldl`, so a static musl link otherwise fails with
//! `unable to find library -ldl`.
//!
//! Provide an empty archive in `OUT_DIR` so `-ldl` resolves. It adds no symbols;
//! the libc definitions are the ones the dynamic loader calls. The archive is
//! eight bytes and generated in `OUT_DIR`, never checked in — the same mechanism
//! `aot/build.rs` uses for the embedded runners.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("musl") {
        return;
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let stub = out.join("libdl.a");
    std::fs::write(&stub, b"!<arch>\n").expect("write the empty libdl archive");
    println!("cargo:rustc-link-search=native={}", out.display());
}
