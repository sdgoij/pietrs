//! Compile the AOT runner once, when `pietrs` itself is built.
//!
//! The runner is embedded in the `aot` library, so `pietrs build` only writes
//! bytes and never invokes a compiler or a linker.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let target = env::var("TARGET").expect("TARGET is set by cargo");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runner/src/loader.rs");
    let generated = out_dir.join("runner_main.rs");
    std::fs::write(
        &generated,
        format!("include!({source:?});\nfn main() -> std::process::ExitCode {{ start() }}\n"),
    )
    .expect("cannot write the generated runner source");

    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let file = format!("pietrs-runner{suffix}");
    let output = out_dir.join(&file);

    let status = Command::new(rustc)
        .arg("--edition=2024")
        .arg("-C")
        .arg("opt-level=2")
        .arg("-C")
        .arg("panic=abort")
        .arg("-D")
        .arg("warnings")
        .arg("--target")
        .arg(&target)
        .arg("-o")
        .arg(&output)
        .arg(&generated)
        .status()
        .expect("rustc could not be started");
    assert!(status.success(), "the AOT runner failed to compile");

    println!("cargo:rerun-if-changed=../runner/src/loader.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=PIETRS_RUNNER_FILE={file}");
}
