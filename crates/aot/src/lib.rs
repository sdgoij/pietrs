#![allow(unexpected_cfgs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Package a program as a standalone executable.
//!
//! `codegen::emit_object` produces a relocatable object. This crate translates
//! it into a small payload and lays out the precompiled runner, the payload,
//! and a footer. The runner maps the payload and executes it; no compiler or
//! linker runs when a program is built.

use std::path::Path;

use canvas::Blocks;
use runtime::Division;

mod payload;

/// The runner, compiled by `build.rs` for this target.
#[cfg(not(rust_analyzer))]
const RUNNER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/", env!("PIETRS_RUNNER_FILE")));

/// rust-analyzer does not run build scripts; the empty slice keeps its analysis
/// type-correct. `cargo` never sets this cfg.
#[cfg(rust_analyzer)]
const RUNNER: &[u8] = &[];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Codegen(#[from] codegen::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("cannot package the program: {0}")]
    Payload(String),
}

/// Builds `output` from the program's payload and the embedded runner.
///
/// `max_steps` is baked into the executable; `None` means no limit.
pub fn build(
    blocks: &Blocks,
    output: &Path,
    max_steps: Option<u64>,
    division: Division,
) -> Result<(), Error> {
    let object = codegen::emit_object(blocks, division)?;
    let payload = payload::encode(&object).map_err(Error::Payload)?;
    let footer = payload::footer(payload.len() as u64, max_steps.unwrap_or(u64::MAX));
    let mut bytes = Vec::with_capacity(RUNNER.len() + payload.len() + footer.len());
    bytes.extend_from_slice(RUNNER);
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&footer);
    std::fs::write(output, bytes)?;
    Ok(())
}
