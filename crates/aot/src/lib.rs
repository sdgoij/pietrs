//! Link the emitted object and the Rust host shim into an executable.

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

    #[error("no Rust compiler found: tried {0}")]
    NoCompiler(String),

    #[error("building with {compiler} failed: {message}")]
    Build { compiler: String, message: String },
}

/// Builds `output` from the program's object and the Rust host shim.
///
/// `max_steps` is baked into the executable; `None` means no limit.
pub fn build(
    blocks: &Blocks,
    output: &Path,
    max_steps: Option<u64>,
    division: Division,
    compiler: Option<&str>,
) -> Result<(), Error> {
    let object = codegen::emit_object(blocks, division)?;
    let directory = tempfile::tempdir()?;
    let object_path = directory
        .path()
        .join(if cfg!(windows) { "piet.obj" } else { "piet.o" });
    let source_path = directory.path().join("piet_host.rs");
    std::fs::write(&object_path, object)?;
    std::fs::write(&source_path, include_str!("host.rs"))?;
    compile(
        compiler,
        &source_path,
        &object_path,
        output,
        max_steps.unwrap_or(u64::MAX),
    )
}

/// The first usable Rust compiler, or `Some(explicit)` when one is named.
pub fn find_compiler(explicit: Option<&str>) -> Option<String> {
    match explicit {
        Some(name) => Some(name.to_owned()),
        None => ["rustc"]
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

fn compile(
    compiler: Option<&str>,
    source: &Path,
    object: &Path,
    output: &Path,
    max_steps: u64,
) -> Result<(), Error> {
    let candidates: Vec<&str> = match compiler {
        Some(name) => vec![name],
        None => vec!["rustc"],
    };
    let mut missing = Vec::new();
    for compiler in candidates {
        let result = Command::new(compiler)
            .arg("--edition=2024")
            .arg("-C")
            .arg("opt-level=2")
            .arg("-D")
            .arg("warnings")
            .arg("-o")
            .arg(output)
            .arg(source)
            .arg("-C")
            .arg(format!("link-arg={}", object.display()))
            .env("PIET_AOT_MAX_STEPS", max_steps.to_string())
            .output();
        match result {
            Ok(run) if run.status.success() => return Ok(()),
            Ok(run) => {
                return Err(Error::Build {
                    compiler: compiler.to_owned(),
                    message: String::from_utf8_lossy(&run.stderr).into_owned(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(compiler),
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Err(Error::NoCompiler(missing.join(", ")))
}
