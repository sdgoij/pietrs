//! Compile the AOT runner for the host and the cross targets, and embed them.
//!
//! The runner for a target is only useful for programs emitted for that target.
//! `rust-toolchain.toml` installs the musl standard libraries, and `rust-lld`
//! links them from any host, so the matrix is deterministic.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Linux targets the compiler embeds runners for. `x86_64` and `aarch64`
/// GNU programs run these static musl artifacts unchanged.
const CROSS: &[&str] = &["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let host = env::var("TARGET").expect("TARGET is set by cargo");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runner/src/loader.rs");
    let generated = out_dir.join("runner_main.rs");
    std::fs::write(
        &generated,
        format!("include!({source:?});\nfn main() -> std::process::ExitCode {{ start() }}\n"),
    )
    .expect("cannot write the generated runner source");

    let mut targets = vec![host.clone()];
    for cross in CROSS {
        if *cross != host {
            targets.push((*cross).to_owned());
        }
    }

    let mut table = String::from("pub static RUNNERS: &[(&str, &[u8])] = &[\n");
    for target in &targets {
        let suffix = if target.contains("windows") {
            ".exe"
        } else {
            ""
        };
        let output = out_dir.join(format!("pietrs-runner-{target}{suffix}"));
        let mut command = Command::new(&rustc);
        command
            .arg("--edition=2024")
            .arg("-C")
            .arg("opt-level=2")
            .arg("-C")
            .arg("panic=abort")
            .arg("-D")
            .arg("warnings")
            .arg("--target")
            .arg(target)
            .arg("-o")
            .arg(&output)
            .arg(&generated);
        if target.contains("musl") {
            command
                .arg("-C")
                .arg("linker=rust-lld")
                .arg("-C")
                .arg("link-self-contained=yes");
        }
        let status = command
            .status()
            .unwrap_or_else(|error| panic!("rustc could not be started: {error}"));
        assert!(
            status.success(),
            "the AOT runner for {target} failed to compile"
        );
        table.push_str(&format!(
            "    ({target:?}, include_bytes!({:?})),\n",
            output.display().to_string()
        ));
    }
    table.push_str("];\n");
    std::fs::write(out_dir.join("runners.rs"), table).expect("cannot write the runner table");

    println!("cargo:rerun-if-changed=../runner/src/loader.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=PIETRS_HOST_TARGET={host}");
}
