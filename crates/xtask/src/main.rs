//! Developer tasks: the external conformance gate.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use canvas::{Blocks, CodelGrid, Point};
use clap::{Args, Parser, Subcommand};
use interp::{ExitReason, Options};
use runtime::BufferIo;

/// The npiet release the vendored source and the gate expect.
const EXPECTED_NPIET: &str = "v1.3h";

static BUILD_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
    /// Build and run every case, comparing it with its goldens.
    E2e(E2eArgs),
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

#[derive(Debug, Args)]
struct E2eArgs {
    /// A corpus; each case directory holds `program.png` and its goldens.
    corpus: PathBuf,
    /// Use this C compiler instead of probing for one.
    #[arg(long)]
    linker: Option<String>,
    /// Kill a case that runs longer than this.
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

    #[error("no case directories in {0}")]
    NoCaseDirectories(PathBuf),

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
        Task::E2e(args) => e2e(&args),
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
        let name = case_name(case);
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
        if path.is_dir() {
            let program = path.join("program.png");
            if program.is_file() {
                cases.push(program);
            }
        } else if is_png(&path) {
            cases.push(path);
        }
    }
    cases.sort();
    Ok(cases)
}

fn case_name(path: &Path) -> String {
    if is_case_program(path) {
        return path.parent().and_then(Path::file_name).map_or_else(
            || "?".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
    }
    path.file_stem().map_or_else(
        || "?".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

fn is_case_program(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy() == "program.png")
}

fn case_max_steps(path: &Path) -> Option<u64> {
    let path = if is_case_program(path) {
        path.with_file_name("max_steps")
    } else {
        path.with_extension("max_steps")
    };
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
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

    let max_steps = case_max_steps(path).unwrap_or(args.max_steps);
    let options = Options {
        max_steps: Some(max_steps),
        ..Options::default()
    };
    let mut io = BufferIo::new(input.as_str());
    let interpreted = interp::run(&blocks, &mut io, &options).map_err(|error| error.to_string())?;

    let ppm = temporary.join("case.ppm");
    write_ppm(blocks.grid(), &ppm).map_err(|error| error.to_string())?;

    let mut command = Command::new(npiet);
    command.arg("-q").arg("-cs").arg("1");
    if max_steps > 0 {
        command.arg("-e").arg(max_steps.to_string());
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

fn e2e(args: &E2eArgs) -> Result<bool, Error> {
    let cases = case_directories(&args.corpus)?;
    if cases.is_empty() {
        return Err(Error::NoCaseDirectories(args.corpus.clone()));
    }
    let Some(linker) = aot::find_linker(args.linker.as_deref()) else {
        return Err(Error::NoCompiler("cc, gcc, clang".to_owned()));
    };

    let mut failures = 0usize;
    for case in &cases {
        let name = case.file_name().map_or_else(
            || "?".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        match check_e2e(case, Some(&linker), args.timeout_ms) {
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

fn case_directories(root: &Path) -> Result<Vec<PathBuf>, Error> {
    if root.join("program.png").is_file() {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            cases.push(path);
        }
    }
    cases.sort();
    Ok(cases)
}

fn check_e2e(case: &Path, linker: Option<&str>, timeout_ms: u64) -> Result<(), String> {
    let image = case.join("program.png");
    let bytes = std::fs::read(&image)
        .map_err(|error| format!("cannot read {}: {error}", image.display()))?;
    let grid = CodelGrid::from_png(&bytes).map_err(|error| error.to_string())?;
    let blocks = Blocks::partition(grid);
    let input = read_text_if_present(case.join("stdin"))?.unwrap_or_default();
    let expected_stdout = std::fs::read(case.join("expected.stdout"))
        .map_err(|error| format!("cannot read expected.stdout: {error}"))?;
    let expected_exit = read_text_if_present(case.join("expected.exit"))?
        .map(|text| {
            text.trim()
                .parse::<i32>()
                .map_err(|error| format!("invalid expected.exit: {error}"))
        })
        .transpose()?
        .unwrap_or(0);
    let max_steps = read_text_if_present(case.join("max_steps"))?
        .map(|text| {
            text.trim()
                .parse::<u64>()
                .map_err(|error| format!("invalid max_steps: {error}"))
        })
        .transpose()?;

    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory
        .path()
        .join(format!("case{}", std::env::consts::EXE_SUFFIX));
    aot::build(&blocks, &executable, max_steps, linker)
        .map_err(|error| format!("cannot build the executable: {error}"))?;
    let run = run_with_timeout(
        &mut Command::new(&executable),
        &input,
        Duration::from_millis(timeout_ms),
    )
    .map_err(|error| error.to_string())?;
    if run.timed_out {
        return Err(format!("timed out after {timeout_ms}ms"));
    }
    if run.stdout != expected_stdout {
        let first = run
            .stdout
            .iter()
            .zip(expected_stdout.iter())
            .take_while(|(actual, expected)| actual == expected)
            .count();
        return Err(format!(
            "stdout differs at byte {first} (got {} bytes, expected {} bytes)",
            run.stdout.len(),
            expected_stdout.len()
        ));
    }
    let Some(status) = run.status else {
        return Err("the executable did not report an exit status".to_owned());
    };
    if status.code() != Some(expected_exit) {
        return Err(format!(
            "exit status {status:?} != expected {expected_exit}"
        ));
    }
    Ok(())
}

fn read_text_if_present(path: PathBuf) -> Result<Option<String>, String> {
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn read_input(path: &Path) -> Result<String, String> {
    let input_path = if is_case_program(path) {
        path.with_file_name("stdin")
    } else {
        path.with_extension("in")
    };
    match std::fs::read_to_string(&input_path) {
        Ok(input) => Ok(input),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("cannot read {}: {error}", input_path.display())),
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
    let bytes = std::fs::read(&source)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let digest = hasher.finish();
    let suffix = std::env::consts::EXE_SUFFIX;
    let directory = workspace.join("target/npiet");
    std::fs::create_dir_all(&directory)?;
    let output = directory.join(format!("npiet-{digest:016x}{suffix}"));
    if output.is_file() {
        return Ok(output);
    }

    let unique = BUILD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        "npiet.tmp{}.{}{suffix}",
        std::process::id(),
        unique
    ));
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
                return match std::fs::rename(&temporary, &output) {
                    Ok(()) => Ok(output),
                    // Another process won the build; its binary is equivalent.
                    Err(_) if output.is_file() => {
                        let _ = std::fs::remove_file(&temporary);
                        Ok(output)
                    }
                    Err(error) => Err(error.into()),
                };
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
