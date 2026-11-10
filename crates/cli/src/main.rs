//! The `pietrs` command line: check, trace, run, build, and selftest a Piet image.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
#[cfg(feature = "codegen")]
use std::process::{Command, Stdio};
use std::sync::OnceLock;
#[cfg(feature = "codegen")]
use std::time::{Duration, Instant};

use canvas::{Blocks, CodelGrid};
use clap::{Args, CommandFactory, Parser, Subcommand};
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
    command: Option<Commands>,
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

#[derive(Debug, Subcommand)]
enum Commands {
    /// Load the image and report the grid, blocks, and start state.
    Check(CheckArgs),
    /// Run the interpreter and print its transition trace.
    Trace(TraceArgs),
    /// Run the program and print its output.
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
    /// How division and modulo round.
    #[arg(long, value_enum, default_value = "floored")]
    division: DivisionArg,
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
    /// How division and modulo round.
    #[arg(long, value_enum, default_value = "floored")]
    division: DivisionArg,
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
    /// How division and modulo round.
    #[arg(long, value_enum, default_value = "floored")]
    division: DivisionArg,
}

#[derive(Debug, Args)]
struct SelftestArgs {
    /// Directory of `.png` cases; a sibling `.in` file supplies input.
    corpus: PathBuf,
    /// Stop each path after this many transitions.
    #[arg(long, default_value_t = 100_000)]
    max_steps: u64,
    /// How division and modulo round.
    #[arg(long, value_enum, default_value = "floored")]
    division: DivisionArg,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Canvas(#[from] canvas::Error),

    #[error(transparent)]
    Interp(#[from] interp::Error),

    #[cfg(feature = "codegen")]
    #[error(transparent)]
    Codegen(#[from] codegen::Error),

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot write stdout: {0}")]
    Write(#[from] std::io::Error),

    #[cfg(feature = "codegen")]
    #[error(transparent)]
    Aot(#[from] aot::Error),

    #[cfg(not(feature = "codegen"))]
    #[error("this pietrs was built without the codegen feature")]
    Unavailable,

    #[error("no .png cases in {0}")]
    NoCases(PathBuf),
}

fn main() -> ExitCode {
    let Some(command) = Cli::parse().command else {
        return help();
    };
    let result = match command {
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

/// A bare invocation is a request for help, so it succeeds.
fn help() -> ExitCode {
    let mut command = Cli::command();
    if let Err(error) = command.print_help() {
        eprintln!("pietrs: {error}");
        return ExitCode::FAILURE;
    }
    println!();
    ExitCode::SUCCESS
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

/// Whether `PIETRS_DUMP_GRAPH` is set; read once per process.
fn dump_graph_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PIETRS_DUMP_GRAPH").is_some())
}

/// Writes the state graph to stderr when the diagnostic switch is set.
fn dump_graph(graph: &StateGraph) {
    if dump_graph_enabled() {
        eprint!("{graph}");
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
    dump_graph(&graph);
    Ok(())
}

fn trace(args: &TraceArgs) -> Result<(), Error> {
    let grid = load(&args.image)?;
    let blocks = Blocks::partition(grid);
    if dump_graph_enabled() {
        dump_graph(&StateGraph::build(&blocks));
    }
    let mut io = BufferIo::new("");
    let options = Options {
        max_steps: Some(args.max_steps),
        division: args.division.into(),
        ..Options::default()
    };
    let (result, trace) = run_traced(&blocks, &mut io, &options)?;
    for step in trace {
        println!("{step}");
    }
    eprintln!("{} steps: {:?}", result.steps, result.reason);
    if !io.output().is_empty() {
        eprintln!("output: {}", io.output());
    }
    Ok(())
}

#[cfg(feature = "codegen")]
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
    let options = codegen::Options {
        max_steps: args.max_steps,
        division: args.division.into(),
        ..codegen::Options::default()
    };
    let result = codegen::run(&blocks, &mut io, &options)?;
    let mut stdout = std::io::stdout();
    stdout.write_all(io.output().as_bytes())?;
    stdout.flush()?;
    match result.reason {
        codegen::ExitReason::Terminated => Ok(ExitCode::SUCCESS),
        codegen::ExitReason::StepLimit => {
            eprintln!("pietrs: step limit reached after {} steps", result.steps);
            Ok(ExitCode::FAILURE)
        }
        codegen::ExitReason::StackOverflow => {
            eprintln!("pietrs: the stack overflowed");
            Ok(ExitCode::FAILURE)
        }
    }
}

#[cfg(not(feature = "codegen"))]
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
        division: args.division.into(),
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

    let mut failures = 0usize;
    for path in &cases {
        let name = case_name(path);
        match check_case(path, args.max_steps, args.division.into()) {
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
        if path.is_dir() {
            let program = path.join("program.png");
            if program.is_file() {
                cases.push(program);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        {
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

fn case_input_path(path: &Path) -> PathBuf {
    if is_case_program(path) {
        path.with_file_name("stdin")
    } else {
        path.with_extension("in")
    }
}

fn case_max_steps(path: &Path) -> Option<u64> {
    let path = if is_case_program(path) {
        path.with_file_name("max_steps")
    } else {
        path.with_extension("max_steps")
    };
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn check_case(path: &Path, max_steps: u64, division: runtime::Division) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let grid = CodelGrid::from_png(&bytes).map_err(|error| error.to_string())?;
    let blocks = Blocks::partition(grid);
    let max_steps = case_max_steps(path).unwrap_or(max_steps);
    let input = std::fs::read_to_string(case_input_path(path)).unwrap_or_default();

    let options = Options {
        max_steps: Some(max_steps),
        division,
        ..Options::default()
    };
    let mut interpreted_io = BufferIo::new(input.as_str());
    let interpreted =
        run(&blocks, &mut interpreted_io, &options).map_err(|error| error.to_string())?;

    #[cfg(feature = "codegen")]
    check_paths(
        &blocks,
        &input,
        max_steps,
        division,
        &interpreted_io,
        interpreted,
    )?;

    #[cfg(not(feature = "codegen"))]
    let _ = (interpreted, interpreted_io);
    Ok(())
}

#[cfg(feature = "codegen")]
fn check_paths(
    blocks: &Blocks,
    input: &str,
    max_steps: u64,
    division: runtime::Division,
    interpreted_io: &BufferIo,
    interpreted: interp::Run,
) -> Result<(), String> {
    let jit_options = codegen::Options {
        stack_capacity: 1 << 20,
        max_steps: Some(max_steps),
        division,
    };
    let mut jit_io = BufferIo::new(input);
    let jit = codegen::run(blocks, &mut jit_io, &jit_options).map_err(|error| error.to_string())?;

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

    check_aot(
        blocks,
        input,
        max_steps,
        division,
        interpreted_output,
        interpreted.reason,
    )?;
    Ok(())
}

#[cfg(feature = "codegen")]
fn check_aot(
    blocks: &Blocks,
    input: &str,
    max_steps: u64,
    division: runtime::Division,
    interpreted_output: &str,
    reason: ExitReason,
) -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = directory
        .path()
        .join(format!("case{}", std::env::consts::EXE_SUFFIX));
    aot::build(blocks, &executable, Some(max_steps), division)
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

#[cfg(feature = "codegen")]
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

#[cfg(feature = "codegen")]
fn build_program(args: &BuildArgs) -> Result<(), Error> {
    let grid = load(&args.image)?;
    let blocks = Blocks::partition(grid);
    let output = executable_path(
        args.output
            .clone()
            .unwrap_or_else(|| default_output(&args.image.image)),
    );
    aot::build(&blocks, &output, args.max_steps, args.division.into())?;
    Ok(())
}

#[cfg(not(feature = "codegen"))]
fn build_program(args: &BuildArgs) -> Result<(), Error> {
    let _ = args;
    Err(Error::Unavailable)
}

#[cfg(feature = "codegen")]
fn default_output(image: &Path) -> PathBuf {
    image
        .file_stem()
        .map_or_else(|| PathBuf::from("a.out"), PathBuf::from)
}

#[cfg(feature = "codegen")]
fn executable_path(mut path: PathBuf) -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    if !suffix.is_empty() && path.extension().is_none() {
        path.set_extension(&suffix[1..]);
    }
    path
}
