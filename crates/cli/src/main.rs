//! The `pietrs` command line: check, trace, run, build, and selftest a Piet image.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use canvas::{Blocks, CodelGrid};
use clap::{Args, Parser, Subcommand};
use interp::{ExitReason, Options, run, run_traced};
use runtime::BufferIo;
use tessera::{Start, StateGraph, start};

#[derive(Debug, Parser)]
#[command(
    name = "pietrs",
    version,
    about = "Compile the tessellation flow of a given canvas"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Load the image and report the grid, blocks, and start state.
    Check(CheckArgs),
    /// Run the interpreter and print its transition trace.
    Trace(TraceArgs),
    /// Run the interpreter and print the program's output.
    Run(RunArgs),
    /// Emit and link a standalone executable.
    Build(BuildArgs),
    /// Run every corpus case through each execution path and compare them.
    Selftest(SelftestArgs),
}

#[derive(Debug, Args)]
struct ImageArgs {
    /// The Piet program image.
    image: PathBuf,
    /// Override the detected codel size.
    #[arg(long)]
    codel_size: Option<u32>,
}

#[derive(Debug, Args)]
struct CheckArgs {
    #[command(flatten)]
    image: ImageArgs,
}

#[derive(Debug, Args)]
struct TraceArgs {
    #[command(flatten)]
    image: ImageArgs,
    /// Stop after this many transitions.
    #[arg(long, default_value_t = 100_000)]
    max_steps: u64,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[command(flatten)]
    image: ImageArgs,
    /// Stop after this many transitions.
    #[arg(long)]
    max_steps: Option<u64>,
    /// Read program input from this file instead of an empty stream.
    #[arg(long)]
    input: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BuildArgs {
    #[command(flatten)]
    image: ImageArgs,
    /// Where to write the executable.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Stop after this many transitions (baked into the executable).
    #[arg(long)]
    max_steps: Option<u64>,
    /// Link with this C compiler instead of probing for one.
    #[arg(long)]
    linker: Option<String>,
}

#[derive(Debug, Args)]
struct SelftestArgs {
    /// Directory of `.png` cases; a sibling `.in` file supplies input.
    corpus: PathBuf,
    /// Stop each path after this many transitions.
    #[arg(long, default_value_t = 100_000)]
    max_steps: u64,
    /// Link with this C compiler instead of probing for one.
    #[arg(long)]
    linker: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Canvas(#[from] canvas::Error),

    #[error(transparent)]
    Interp(#[from] interp::Error),

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot write stdout: {0}")]
    Write(#[from] std::io::Error),

    #[error(transparent)]
    Aot(#[from] aot::Error),

    #[error("no .png cases in {0}")]
    NoCases(PathBuf),
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Commands::Check(args) => check(&args).map(|()| ExitCode::SUCCESS),
        Commands::Trace(args) => trace(&args).map(|()| ExitCode::SUCCESS),
        Commands::Run(args) => run_program(&args),
        Commands::Build(args) => build_program(&args).map(|()| ExitCode::SUCCESS),
        Commands::Selftest(args) => selftest(&args),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pietrs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn load(image: &ImageArgs) -> Result<CodelGrid, Error> {
    let bytes = std::fs::read(&image.image).map_err(|source| Error::Read {
        path: image.image.clone(),
        source,
    })?;
    match image.codel_size {
        Some(size) => Ok(CodelGrid::from_png_with_codel_size(&bytes, size)?),
        None => Ok(CodelGrid::from_png(&bytes)?),
    }
}

fn check(args: &CheckArgs) -> Result<(), Error> {
    let grid = load(&args.image)?;
    let blocks = Blocks::partition(grid);
    println!(
        "grid: {}x{} codels (codel size {})",
        blocks.grid().width(),
        blocks.grid().height(),
        blocks.grid().codel_size()
    );
    println!("blocks: {}", blocks.len());
    let graph = StateGraph::build(&blocks);
    println!("states: {}", graph.states().len());
    println!("edges: {}", graph.edges().len());
    match start(&blocks) {
        Start::State(state) => {
            let color = blocks.block(state.block).map_or_else(
                || "unknown".to_owned(),
                |block| format!("{:?}", block.color()),
            );
            println!(
                "start: block {} ({color}), dp {:?}, cc {:?}",
                state.block.0, state.dp, state.cc
            );
        }
        Start::Terminate => println!("start: terminates"),
    }
    Ok(())
}

fn trace(args: &TraceArgs) -> Result<(), Error> {
    let grid = load(&args.image)?;
    let blocks = Blocks::partition(grid);
    let mut io = BufferIo::new("");
    let options = Options {
        max_steps: Some(args.max_steps),
        ..Options::default()
    };
    let (result, trace) = run_traced(&blocks, &mut io, &options)?;
    for step in trace {
        let command = step
            .command
            .map_or_else(|| "none".to_owned(), |command| format!("{command:?}"));
        let target = step
            .target
            .map_or_else(|| "none".to_owned(), |target| target.0.to_string());
        println!(
            "block={} dp={:?} cc={:?} command={command} target={target}",
            step.block.0, step.dp, step.cc
        );
    }
    eprintln!("{} steps: {:?}", result.steps, result.reason);
    if !io.output().is_empty() {
        eprintln!("output: {}", io.output());
    }
    Ok(())
}

fn run_program(args: &RunArgs) -> Result<ExitCode, Error> {
    let grid = load(&args.image)?;
    let blocks = Blocks::partition(grid);
    let input = match &args.input {
        Some(path) => std::fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?,
        None => String::new(),
    };
    let mut io = BufferIo::new(input);
    let options = Options {
        max_steps: args.max_steps,
        ..Options::default()
    };
    let result = run(&blocks, &mut io, &options)?;
    let mut stdout = std::io::stdout();
    stdout.write_all(io.output().as_bytes())?;
    stdout.flush()?;
    match result.reason {
        ExitReason::Terminated => Ok(ExitCode::SUCCESS),
        ExitReason::StepLimit => {
            eprintln!("pietrs: step limit reached after {} steps", result.steps);
            Ok(ExitCode::FAILURE)
        }
    }
}

fn selftest(args: &SelftestArgs) -> Result<ExitCode, Error> {
    let cases = cases_in(&args.corpus)?;
    if cases.is_empty() {
        return Err(Error::NoCases(args.corpus.clone()));
    }

    let linker = aot::find_linker(args.linker.as_deref());
    if linker.is_none() {
        eprintln!("pietrs: no C compiler found (tried cc, gcc, clang); the AOT path is skipped");
    }

    let mut failures = 0usize;
    for path in &cases {
        let name = path.file_stem().map_or_else(
            || "?".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        match check_case(path, args.max_steps, linker.as_deref()) {
            Ok(()) => println!("PASS: {name}"),
            Err(reason) => {
                failures += 1;
                println!("FAIL: {name} ({reason})");
            }
        }
    }
    eprintln!("{} cases, {failures} failed", cases.len());
    Ok(if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn cases_in(directory: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = std::fs::read_dir(directory).map_err(|source| Error::Read {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut cases = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::Read {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        {
            cases.push(path);
        }
    }
    cases.sort();
    Ok(cases)
}

fn check_case(path: &Path, max_steps: u64, linker: Option<&str>) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let grid = CodelGrid::from_png(&bytes).map_err(|error| error.to_string())?;
    let blocks = Blocks::partition(grid);
    let input = std::fs::read_to_string(path.with_extension("in")).unwrap_or_default();

    let options = Options {
        max_steps: Some(max_steps),
        ..Options::default()
    };
    let mut interpreted_io = BufferIo::new(input.as_str());
    let interpreted =
        run(&blocks, &mut interpreted_io, &options).map_err(|error| error.to_string())?;

    let jit_options = codegen::Options {
        stack_capacity: 1 << 20,
        max_steps: Some(max_steps),
    };
    let mut jit_io = BufferIo::new(input.as_str());
    let jit =
        codegen::run(&blocks, &mut jit_io, &jit_options).map_err(|error| error.to_string())?;

    let interpreted_output = interpreted_io.output();
    let jit_output = jit_io.output();
    if interpreted_output != jit_output {
        let first = interpreted_output
            .bytes()
            .zip(jit_output.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        return Err(format!(
            "output differs at byte {first} (interp {} bytes, jit {} bytes)",
            interpreted_output.len(),
            jit_output.len()
        ));
    }
    if interpreted.steps != jit.steps {
        return Err(format!(
            "steps differ: interp {}, jit {}",
            interpreted.steps, jit.steps
        ));
    }
    match (interpreted.reason, jit.reason) {
        (ExitReason::Terminated, codegen::ExitReason::Terminated)
        | (ExitReason::StepLimit, codegen::ExitReason::StepLimit) => {}
        (_, codegen::ExitReason::StackOverflow) => {
            return Err("the jit overflowed the stack".to_owned());
        }
        (interpreted, jit) => {
            return Err(format!(
                "reasons differ: interp {interpreted:?}, jit {jit:?}"
            ));
        }
    }

    if let Some(linker) = linker {
        check_aot(
            &blocks,
            &input,
            max_steps,
            linker,
            interpreted_output,
            interpreted.reason,
        )?;
    }
    Ok(())
}

fn check_aot(
    blocks: &Blocks,
    input: &str,
    max_steps: u64,
    linker: &str,
    interpreted_output: &str,
    reason: ExitReason,
) -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory
        .path()
        .join(format!("case{}", std::env::consts::EXE_SUFFIX));
    aot::build(blocks, &executable, Some(max_steps), Some(linker))
        .map_err(|error| format!("cannot build the executable: {error}"))?;
    let run = run_executable(&executable, input, Duration::from_secs(10))?;
    if run.stdout != interpreted_output.as_bytes() {
        return Err(format!(
            "AOT stdout differs (interp {} bytes, AOT {} bytes)",
            interpreted_output.len(),
            run.stdout.len()
        ));
    }
    match (reason, run.status.success()) {
        (ExitReason::Terminated, true) | (ExitReason::StepLimit, false) => Ok(()),
        (ExitReason::Terminated, false) => {
            Err("the AOT executable failed but interp terminated".to_owned())
        }
        (ExitReason::StepLimit, true) => {
            Err("the AOT executable terminated but interp hit the step limit".to_owned())
        }
    }
}

fn run_executable(
    executable: &Path,
    input: &str,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let stdin_path = directory.path().join("stdin");
    let stdout_path = directory.path().join("stdout");
    let stderr_path = directory.path().join("stderr");
    std::fs::write(&stdin_path, input).map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .stdin(Stdio::from(
            std::fs::File::open(&stdin_path).map_err(|error| error.to_string())?,
        ))
        .stdout(Stdio::from(
            std::fs::File::create(&stdout_path).map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(
            std::fs::File::create(&stderr_path).map_err(|error| error.to_string())?,
        ))
        .spawn()
        .map_err(|error| error.to_string())?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => {
                return Ok(std::process::Output {
                    status,
                    stdout: std::fs::read(&stdout_path).map_err(|error| error.to_string())?,
                    stderr: std::fs::read(&stderr_path).map_err(|error| error.to_string())?,
                });
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the AOT executable timed out after {timeout:?}"));
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn build_program(args: &BuildArgs) -> Result<(), Error> {
    let grid = load(&args.image)?;
    let blocks = Blocks::partition(grid);
    let output = executable_path(
        args.output
            .clone()
            .unwrap_or_else(|| default_output(&args.image.image)),
    );
    aot::build(&blocks, &output, args.max_steps, args.linker.as_deref())?;
    Ok(())
}

fn default_output(image: &Path) -> PathBuf {
    image
        .file_stem()
        .map_or_else(|| PathBuf::from("a.out"), PathBuf::from)
}

fn executable_path(mut path: PathBuf) -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    if !suffix.is_empty() && path.extension().is_none() {
        path.set_extension(&suffix[1..]);
    }
    path
}
