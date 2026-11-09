//! Link the emitted object and the C runtime shim into an executable.

use std::path::Path;
use std::process::Command;

use canvas::Blocks;
use runtime::Division;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Codegen(#[from] codegen::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("no C compiler found: tried {0}")]
    NoLinker(String),

    #[error("linking with {linker} failed: {message}")]
    Link { linker: String, message: String },
}

/// Builds `output` from the program's object and the C host shim.
///
/// `max_steps` is baked into the executable; `None` means no limit.
pub fn build(
    blocks: &Blocks,
    output: &Path,
    max_steps: Option<u64>,
    division: Division,
    linker: Option<&str>,
) -> Result<(), Error> {
    let object = codegen::emit_object(blocks, division)?;
    let directory = tempfile::tempdir()?;
    let object_path = directory.path().join("piet.o");
    let source_path = directory.path().join("piet_host.c");
    std::fs::write(&object_path, object)?;
    std::fs::write(&source_path, include_str!("aot.c"))?;
    link(
        linker,
        &source_path,
        &object_path,
        output,
        max_steps.unwrap_or(u64::MAX),
    )
}

/// The first usable C compiler, or `Some(explicit)` when one is named.
pub fn find_linker(explicit: Option<&str>) -> Option<String> {
    match explicit {
        Some(name) => Some(name.to_owned()),
        None => ["cc", "gcc", "clang"]
            .into_iter()
            .find(|name| {
                Command::new(name)
                    .arg("--version")
                    .output()
                    .is_ok_and(|run| run.status.success())
            })
            .map(str::to_owned),
    }
}

fn link(
    linker: Option<&str>,
    source: &Path,
    object: &Path,
    output: &Path,
    max_steps: u64,
) -> Result<(), Error> {
    let candidates: Vec<&str> = match linker {
        Some(name) => vec![name],
        None => vec!["cc", "gcc", "clang"],
    };
    let define = format!("-DPIET_AOT_MAX_STEPS=UINT64_C({max_steps})");
    let mut missing = Vec::new();
    for compiler in candidates {
        let result = Command::new(compiler)
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(&define)
            .arg("-o")
            .arg(output)
            .arg(source)
            .arg(object)
            .output();
        match result {
            Ok(run) if run.status.success() => return Ok(()),
            Ok(run) => {
                return Err(Error::Link {
                    linker: compiler.to_owned(),
                    message: String::from_utf8_lossy(&run.stderr).into_owned(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(compiler),
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Err(Error::NoLinker(missing.join(", ")))
}
