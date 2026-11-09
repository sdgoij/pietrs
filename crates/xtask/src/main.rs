//! Developer tasks: the external conformance gate.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use canvas::{Blocks, CodelGrid, Point};
use clap::{Args, Parser, Subcommand};
use interp::{ExitReason, Options};
use runtime::BufferIo;

/// The npiet release the vendored source and the gate expect.
const EXPECTED_NPIET: &str = "v1.3h";

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Developer tasks for pietrs")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Compare the interpreter against the npiet oracle.
    Conformance(ConformanceArgs),
}

#[derive(Debug, Args)]
struct ConformanceArgs {
    /// A PNG case, or a directory of `.png` cases; a sibling `.in` supplies input.
    input: PathBuf,
    /// Use this npiet binary instead of building the vendored source.
    #[arg(long)]
    npiet: Option<PathBuf>,
    /// Stop each path after this many transitions.
    #[arg(long, default_value_t = 100_000)]
    max_steps: u64,
    /// Kill a path that runs longer than this.
    #[arg(long, default_value_t = 10_000)]
    timeout_ms: u64,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Canvas(#[from] canvas::Error),

    #[error("no .png cases in {0}")]
    NoCases(PathBuf),

    #[error("the vendored npiet source is missing: {0}")]
    MissingSource(PathBuf),

    #[error("no C compiler found: tried {0}")]
    NoCompiler(String),

    #[error("building npiet with {compiler} failed: {message}")]
    Build { compiler: String, message: String },

    #[error("cannot run {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },

    #[error("expected npiet {EXPECTED_NPIET}, but {path} reported: {reported}")]
    WrongVersion { path: PathBuf, reported: String },
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Task::Conformance(args) => conformance(&args),
    };
    match result {
        Ok(failed) => {
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn conformance(args: &ConformanceArgs) -> Result<bool, Error> {
    let cases = cases(&args.input)?;
    if cases.is_empty() {
        return Err(Error::NoCases(args.input.clone()));
    }

    let workspace = workspace_root();
    let npiet = match &args.npiet {
        Some(path) => path.clone(),
        None => build_npiet(&workspace)?,
    };
    check_version(&npiet)?;

    let temporary = tempfile::tempdir()?;
    let mut failures = 0usize;
    for case in &cases {
        let name = case.file_stem().map_or_else(
            || "?".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        match check_case(case, &npiet, temporary.path(), args) {
            Ok(()) => println!("OK: {name}"),
            Err(reason) => {
                failures += 1;
                println!("FAIL: {name}: {reason}");
            }
        }
    }
    eprintln!("{} cases, {failures} failed", cases.len());
    Ok(failures > 0)
}

fn cases(input: &Path) -> Result<Vec<PathBuf>, Error> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(input)? {
        let path = entry?.path();
        if is_png(&path) {
            cases.push(path);
        }
    }
    cases.sort();
    Ok(cases)
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

fn check_case(
    path: &Path,
    npiet: &Path,
    temporary: &Path,
    args: &ConformanceArgs,
) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let grid = CodelGrid::from_png(&bytes).map_err(|error| error.to_string())?;
    let blocks = Blocks::partition(grid);
    let input = read_input(path)?;

    let options = Options {
        max_steps: Some(args.max_steps),
        ..Options::default()
    };
    let mut io = BufferIo::new(input.as_str());
    let interpreted = interp::run(&blocks, &mut io, &options).map_err(|error| error.to_string())?;

    let ppm = temporary.join("case.ppm");
    write_ppm(blocks.grid(), &ppm).map_err(|error| error.to_string())?;

    let mut command = Command::new(npiet);
    command.arg("-q").arg("-cs").arg("1");
    if args.max_steps > 0 {
        command.arg("-e").arg(args.max_steps.to_string());
    }
    command.arg(&ppm);
    let run = run_with_timeout(&mut command, &input, Duration::from_millis(args.timeout_ms))
        .map_err(|error| error.to_string())?;
    if run.timed_out {
        return Err(format!("npiet timed out after {}ms", args.timeout_ms));
    }
    let Some(status) = run.status else {
        return Err("npiet did not report an exit status".to_owned());
    };
    let stderr = String::from_utf8_lossy(&run.stderr);
    if !status.success() {
        return Err(format!("npiet exited with {status}: {stderr}"));
    }
    if run.stdout != io.output().as_bytes() {
        return Err(format!(
            "stdout differs (interp {} bytes, npiet {} bytes)",
            io.output().len(),
            run.stdout.len()
        ));
    }

    let limited = stderr.contains("execution steps exceeded");
    match (interpreted.reason, limited) {
        (ExitReason::StepLimit, true) | (ExitReason::Terminated, false) => Ok(()),
        (ExitReason::StepLimit, false) => {
            Err("interp hit the step limit but npiet did not".to_owned())
        }
        (ExitReason::Terminated, true) => {
            Err("npiet hit the step limit but interp terminated".to_owned())
        }
    }
}

fn read_input(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path.with_extension("in")) {
        Ok(input) => Ok(input),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.to_string()),
    }
}

fn write_ppm(grid: &CodelGrid, path: &Path) -> Result<(), Error> {
    let codels = grid.width() as usize * grid.height() as usize;
    let mut bytes = Vec::with_capacity(codels * 3 + 32);
    bytes.extend_from_slice(format!("P6\n{} {}\n255\n", grid.width(), grid.height()).as_bytes());
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            let color = grid.get(Point::new(x, y)).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "a codel is missing")
            })?;
            bytes.extend_from_slice(&color.rgb());
        }
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn build_npiet(workspace: &Path) -> Result<PathBuf, Error> {
    let source = workspace.join("vendor/npiet/npiet-1.3h.c");
    if !source.is_file() {
        return Err(Error::MissingSource(source));
    }
    let directory = workspace.join("target/npiet");
    std::fs::create_dir_all(&directory)?;
    let suffix = std::env::consts::EXE_SUFFIX;
    let output = directory.join(format!("npiet{suffix}"));
    let temporary = directory.join(format!("npiet.tmp{}{suffix}", std::process::id()));

    let mut missing = Vec::new();
    for compiler in ["cc", "gcc", "clang"] {
        match Command::new(compiler)
            .arg("-O2")
            .arg("-o")
            .arg(&temporary)
            .arg(&source)
            .status()
        {
            Ok(status) if status.success() => {
                std::fs::rename(&temporary, &output)?;
                return Ok(output);
            }
            Ok(status) => {
                return Err(Error::Build {
                    compiler: compiler.to_owned(),
                    message: format!("exited with {status}"),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(compiler),
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::NoCompiler(missing.join(", ")))
}

fn check_version(npiet: &Path) -> Result<(), Error> {
    let mut command = Command::new(npiet);
    let run = run_with_timeout(&mut command, "", Duration::from_secs(5))?;
    let reported = String::from_utf8_lossy(&run.stderr);
    if reported.contains(&format!("npiet {EXPECTED_NPIET}")) {
        Ok(())
    } else {
        Err(Error::WrongVersion {
            path: npiet.to_path_buf(),
            reported: reported.into_owned(),
        })
    }
}

struct ProcessRun {
    status: Option<std::process::ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

fn run_with_timeout(
    command: &mut Command,
    input: &str,
    timeout: Duration,
) -> Result<ProcessRun, Error> {
    let directory = tempfile::tempdir()?;
    let stdin_path = directory.path().join("stdin");
    let stdout_path = directory.path().join("stdout");
    let stderr_path = directory.path().join("stderr");
    std::fs::write(&stdin_path, input)?;
    command
        .stdin(Stdio::from(std::fs::File::open(&stdin_path)?))
        .stdout(Stdio::from(std::fs::File::create(&stdout_path)?))
        .stderr(Stdio::from(std::fs::File::create(&stderr_path)?));
    let mut child = command.spawn().map_err(|source| Error::Spawn {
        program: command.get_program().to_string_lossy().into_owned(),
        source,
    })?;

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(ProcessRun {
                status: Some(status),
                stdout: std::fs::read(&stdout_path)?,
                stderr: std::fs::read(&stderr_path)?,
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            return Ok(ProcessRun {
                status: None,
                stdout: std::fs::read(&stdout_path)?,
                stderr: std::fs::read(&stderr_path)?,
                timed_out: true,
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
