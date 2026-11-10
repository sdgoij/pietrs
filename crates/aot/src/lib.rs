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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Codegen(#[from] codegen::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("cannot package the program: {0}")]
    Payload(String),
}

/// Builds `output` from the program's payload and the given runner bytes.
///
/// `max_steps` is baked into the executable; `None` means no limit.
pub fn build(
    blocks: &Blocks,
    output: &Path,
    max_steps: Option<u64>,
    division: Division,
    runner: &[u8],
) -> Result<(), Error> {
    let object = codegen::emit_object(blocks, division)?;
    let payload = payload::encode(&object).map_err(Error::Payload)?;
    let footer = payload::footer(payload.len() as u64, max_steps.unwrap_or(u64::MAX));
    let mut bytes = Vec::with_capacity(runner.len() + payload.len() + footer.len());
    bytes.extend_from_slice(runner);
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&footer);
    std::fs::write(output, bytes)?;
    Ok(())
}
