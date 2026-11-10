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
use testkit::Checks;

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
    /// Build the browser bundle into `target/web`.
    Wasm,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum DivisionArg {
    Floored,
    Truncating,
}

impl From<DivisionArg> for runtime::Division {
    fn from(value: DivisionArg) -> Self {
        match value {
            DivisionArg::Floored => runtime::Division::Floored,
            DivisionArg::Truncating => runtime::Division::Truncating,
        }
    }
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
    /// How division and modulo round; npiet truncates toward zero.
    #[arg(long, value_enum, default_value = "truncating")]
    division: DivisionArg,
}

#[derive(Debug, Args)]
struct E2eArgs {
    /// A corpus; each case directory holds `program.png` and its goldens.
    corpus: PathBuf,
    /// Kill a case that runs longer than this.
    #[arg(long, default_value_t = 10_000)]
    timeout_ms: u64,
    /// How division and modulo round.
    #[arg(long, value_enum, default_value = "floored")]
    division: DivisionArg,
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

    #[error("no C compiler found for npiet: tried {0}")]
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

    #[error("Cargo.lock has no wasm-bindgen package")]
    NoWasmBindgen,

    #[error(
        "cannot find `wasm-bindgen` on PATH; install it with `cargo install wasm-bindgen-cli --version {version}`"
    )]
    MissingWasmBindgen { version: String },

    #[error(
        "expected wasm-bindgen {expected}, but the CLI reported {reported}; run `cargo install -f wasm-bindgen-cli --version {expected}`"
    )]
    WrongWasmBindgen { expected: String, reported: String },

    #[error("`cargo build` for wasm32 failed")]
    BuildFailed,

    #[error("`wasm-bindgen` failed")]
    WasmBindgenFailed,
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Task::Conformance(args) => conformance(&args),
        Task::E2e(args) => e2e(&args),
        Task::Wasm => wasm().map(|()| false),
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
    let mut checks = Checks::new();
    for case in &cases {
        let name = case_name(case);
        checks.check(&name, check_case(case, &npiet, temporary.path(), args));
    }
    eprintln!("{} cases, {} failed", checks.total(), checks.failures());
    Ok(!checks.passed())
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
    let grid = CodelGrid::from_image(&bytes).map_err(|error| error.to_string())?;
    let blocks = Blocks::partition(grid);
    let input = read_input(path)?;

    let max_steps = case_max_steps(path).unwrap_or(args.max_steps);
    let options = Options {
        max_steps: Some(max_steps),
        division: args.division.into(),
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
    let npiet_stdout = normalize_newlines(&run.stdout);
    let limited = stderr.contains("execution steps exceeded");
    let mine = io.output().as_bytes();
    if matches!(interpreted.reason, ExitReason::StepLimit) && limited {
        // A blocked white slide is one transition for `pietrs` and several for
        // npiet, so equal budgets do not mean equal work once a program loops.
        // Compare what both produced: a byte that differs anywhere in that
        // window is still a divergence.
        let common = mine.len().min(npiet_stdout.len());
        if mine[..common] != npiet_stdout[..common] {
            let first = mine
                .iter()
                .zip(npiet_stdout.iter())
                .take_while(|(ours, theirs)| ours == theirs)
                .count();
            return Err(format!(
                "stdout differs at byte {first} of {common} compared"
            ));
        }
    } else if npiet_stdout != mine {
        return Err(format!(
            "stdout differs (interp {} bytes, npiet {} bytes)",
            mine.len(),
            npiet_stdout.len()
        ));
    }

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

/// npiet writes stdout through C stdio, which is in text mode on Windows and
/// turns every `\n` into `\r\n`. That is a property of the reference build, not
/// of Piet, so the comparison normalises it away where it happens.
fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    if !cfg!(windows) {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            out.push(b'\n');
            index += 2;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

fn e2e(args: &E2eArgs) -> Result<bool, Error> {
    let cases = case_directories(&args.corpus)?;
    if cases.is_empty() {
        return Err(Error::NoCaseDirectories(args.corpus.clone()));
    }

    let mut checks = Checks::new();
    for case in &cases {
        let name = case.file_name().map_or_else(
            || "?".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        checks.check(
            &name,
            check_e2e(case, args.timeout_ms, args.division.into()),
        );
    }
    eprintln!("{} cases, {} failed", checks.total(), checks.failures());
    Ok(!checks.passed())
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

fn check_e2e(case: &Path, timeout_ms: u64, division: runtime::Division) -> Result<(), String> {
    let image = case.join("program.png");
    let bytes = std::fs::read(&image)
        .map_err(|error| format!("cannot read {}: {error}", image.display()))?;
    let grid = CodelGrid::from_image(&bytes).map_err(|error| error.to_string())?;
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
    let expected_trace = std::fs::read_to_string(case.join("expected.trace"))
        .map_err(|error| format!("cannot read expected.trace: {error}"))?;

    let options = Options {
        max_steps,
        division,
        ..Options::default()
    };
    let mut trace_io = BufferIo::new(input.as_str());
    let (_, trace) =
        interp::run_traced(&blocks, &mut trace_io, &options).map_err(|error| error.to_string())?;
    let actual_trace: String = trace.iter().map(|step| format!("{step}\n")).collect();
    if actual_trace != expected_trace {
        let first = actual_trace
            .lines()
            .zip(expected_trace.lines())
            .take_while(|(actual, expected)| actual == expected)
            .count();
        return Err(format!("trace differs at line {}", first + 1));
    }

    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory
        .path()
        .join(format!("case{}", std::env::consts::EXE_SUFFIX));
    aot::build(&blocks, &executable, max_steps, division, aot::HOST_TARGET)
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

/// Builds the browser bundle: the release wasm, the wasm-bindgen glue, and the
/// page, all in `target/web`.
///
/// `wasm-bindgen` is not vendored. It is named when it is missing, and its
/// version is checked against the lock before anything is built, because a
/// mismatch is only reported by the tool itself, halfway through.
fn wasm() -> Result<(), Error> {
    let workspace = workspace_root();
    let version = locked_wasm_bindgen(&workspace)?;
    check_wasm_bindgen(&version)?;

    let build = Command::new("cargo")
        .current_dir(&workspace)
        .args([
            "build",
            "--locked",
            "--release",
            "-p",
            "gui",
            "--no-default-features",
            "--features",
            "web",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .status()
        .map_err(|source| Error::Spawn {
            program: "cargo".to_owned(),
            source,
        })?;
    if !build.success() {
        return Err(Error::BuildFailed);
    }

    let out = workspace.join("target/web");
    std::fs::create_dir_all(&out)?;
    let module = workspace.join("target/wasm32-unknown-unknown/release/gui.wasm");
    let bindgen = Command::new("wasm-bindgen")
        .current_dir(&workspace)
        .args(["--target", "web", "--out-name", "pietrs", "--out-dir"])
        .arg(&out)
        .arg(&module)
        .status()
        .map_err(|source| Error::Spawn {
            program: "wasm-bindgen".to_owned(),
            source,
        })?;
    if !bindgen.success() {
        return Err(Error::WasmBindgenFailed);
    }

    for name in ["index.html", "pietrs.css"] {
        std::fs::copy(workspace.join("crates/gui/web").join(name), out.join(name))?;
    }

    eprintln!("wrote {}", out.join("index.html").display());
    Ok(())
}

/// The `wasm-bindgen` version the lock pins.
fn locked_wasm_bindgen(workspace: &Path) -> Result<String, Error> {
    let lock = std::fs::read_to_string(workspace.join("Cargo.lock"))?;
    let mut in_package = false;
    for line in lock.lines() {
        let line = line.trim();
        if line.starts_with("[[package]]") {
            in_package = false;
        } else if line == "name = \"wasm-bindgen\"" {
            in_package = true;
        } else if in_package && let Some(version) = line.strip_prefix("version = \"") {
            return Ok(version.trim_end_matches('"').to_owned());
        }
    }
    Err(Error::NoWasmBindgen)
}

/// Fails, by name, when the CLI is missing or is not the version of the crate.
fn check_wasm_bindgen(version: &str) -> Result<(), Error> {
    let output = match Command::new("wasm-bindgen").arg("--version").output() {
        Ok(output) => output,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::MissingWasmBindgen {
                version: version.to_owned(),
            });
        }
        Err(source) => {
            return Err(Error::Spawn {
                program: "wasm-bindgen".to_owned(),
                source,
            });
        }
    };
    let reported = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let found = reported.rsplit(' ').next().unwrap_or_default();
    if found == version {
        Ok(())
    } else {
        Err(Error::WrongWasmBindgen {
            expected: version.to_owned(),
            reported,
        })
    }
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
