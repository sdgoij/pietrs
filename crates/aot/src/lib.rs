#![allow(unexpected_cfgs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Package a program as a standalone executable.
//!
//! `codegen::emit_object_for` produces a relocatable object for a target. This
//! crate translates it into a small payload, selects the runner embedded for
//! that target, and lays out the runner, the payload, and a footer. The runner
//! maps the payload and executes it; no compiler or linker runs when a program
//! is built.

use std::path::Path;

use canvas::Blocks;
use runtime::Division;

mod payload;

#[cfg(not(rust_analyzer))]
mod runners {
    include!(concat!(env!("OUT_DIR"), "/runners.rs"));
}

/// rust-analyzer does not run build scripts; the empty table keeps its analysis
/// type-correct. `cargo` never sets this cfg.
#[cfg(rust_analyzer)]
mod runners {
    pub static RUNNERS: &[(&str, &[u8])] = &[];
}

/// The target `pietrs` was built for.
#[cfg(not(rust_analyzer))]
pub const HOST_TARGET: &str = env!("PIETRS_HOST_TARGET");

/// The host target; rust-analyzer cannot see the build script.
#[cfg(rust_analyzer)]
pub const HOST_TARGET: &str = "x86_64-pc-windows-msvc";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Codegen(#[from] codegen::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("cannot package the program: {0}")]
    Payload(String),

    #[error("no AOT runner was embedded for {0}")]
    NoRunner(String),

    #[error("the AOT compiler does not support {0}")]
    UnsupportedTarget(String),
}

/// Builds `output` for `target` from the program's payload and its runner.
///
/// `max_steps` is baked into the executable; `None` means no limit.
pub fn build(
    blocks: &Blocks,
    output: &Path,
    max_steps: Option<u64>,
    division: Division,
    target: &str,
) -> Result<(), Error> {
    let (runner_target, codegen_target) = resolve(target)?;
    let runner = runners::RUNNERS
        .iter()
        .find(|(name, _)| *name == runner_target)
        .map(|(_, bytes)| *bytes)
        .ok_or_else(|| Error::NoRunner(runner_target.to_owned()))?;
    let object = codegen::emit_object_for(blocks, division, codegen_target)?;
    let (payload, arch) = payload::encode(&object).map_err(Error::Payload)?;
    let footer = payload::footer(payload.len() as u64, max_steps.unwrap_or(u64::MAX), arch);
    let mut bytes = Vec::with_capacity(runner.len() + payload.len() + footer.len());
    bytes.extend_from_slice(runner);
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&footer);
    std::fs::write(output, bytes)?;
    set_executable(output)?;
    Ok(())
}

/// An executable has to be executable. `fs::write` creates it with the caller's
/// umask, which does not include `+x` on Unix, so a fresh build would be refused
/// by the shell until it was chmodded by hand.
fn set_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Maps a user-facing target to the runner and codegen triples.
///
/// GNU Linux programs run the static musl artifacts unchanged, so the GNU
/// spellings select the musl runner and the musl codegen target.
fn resolve(target: &str) -> Result<(&'static str, &'static str), Error> {
    if target == HOST_TARGET {
        return Ok((HOST_TARGET, HOST_TARGET));
    }
    match target {
        "x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => {
            Ok(("x86_64-unknown-linux-musl", "x86_64-unknown-linux-musl"))
        }
        "aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => {
            Ok(("aarch64-unknown-linux-musl", "aarch64-unknown-linux-musl"))
        }
        other => Err(Error::UnsupportedTarget(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{Color, Hue, Lightness};

    fn single_block() -> Blocks {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        testkit::blocks(vec![red], 1)
    }

    #[test]
    fn the_host_target_resolves_to_itself() {
        assert_eq!(resolve(HOST_TARGET).unwrap(), (HOST_TARGET, HOST_TARGET));
    }

    #[test]
    fn a_gnu_linux_target_selects_the_musl_artifact() {
        for (gnu, musl) in [
            ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"),
            ("aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"),
            ("x86_64-unknown-linux-musl", "x86_64-unknown-linux-musl"),
            ("aarch64-unknown-linux-musl", "aarch64-unknown-linux-musl"),
        ] {
            assert_eq!(resolve(gnu).unwrap(), (musl, musl), "{gnu}");
        }
    }

    #[test]
    fn an_unknown_target_names_itself_in_the_error() {
        let error = resolve("riscv64-unknown-none").unwrap_err();
        assert!(
            matches!(&error, Error::UnsupportedTarget(target) if target == "riscv64-unknown-none")
        );
        assert_eq!(
            error.to_string(),
            "the AOT compiler does not support riscv64-unknown-none"
        );
    }

    #[test]
    fn a_cross_build_records_the_architecture_in_the_footer() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("program");
        build(
            &single_block(),
            &output,
            None,
            Division::Floored,
            "aarch64-unknown-linux-gnu",
        )
        .unwrap();

        let bytes = std::fs::read(&output).unwrap();
        let footer = &bytes[bytes.len() - payload::FOOTER_LEN..];
        assert_eq!(&footer[..16], b"PIETRS-AOT\0\0\0\0\0\0");
        assert_eq!(
            u32::from_le_bytes(footer[32..36].try_into().unwrap()),
            1,
            "the payload is aarch64"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_built_executable_is_executable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("program");
        build(
            &single_block(),
            &output,
            None,
            Division::Floored,
            HOST_TARGET,
        )
        .unwrap();

        let mode = std::fs::metadata(&output).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "the shell must be able to run it");
    }
}
