//! The `pietrs` command line: check, trace, and run a Piet image.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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
struct SelftestArgs {
    /// Directory of `.png` cases; a sibling `.in` file supplies input.
    corpus: PathBuf,
    /// Stop each path after this many transitions.
    #[arg(long, default_value_t = 100_000)]
    max_steps: u64,
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

    #[error("no .png cases in {0}")]
    NoCases(PathBuf),
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Commands::Check(args) => check(&args).map(|()| ExitCode::SUCCESS),
        Commands::Trace(args) => trace(&args).map(|()| ExitCode::SUCCESS),
        Commands::Run(args) => run_program(&args),
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

    let mut failures = 0usize;
    for path in &cases {
        let name = path.file_stem().map_or_else(
            || "?".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        match check_case(path, args.max_steps) {
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

fn check_case(path: &Path, max_steps: u64) -> Result<(), String> {
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
        | (ExitReason::StepLimit, codegen::ExitReason::StepLimit) => Ok(()),
        (_, codegen::ExitReason::StackOverflow) => Err("the jit overflowed the stack".to_owned()),
        (interpreted, jit) => Err(format!(
            "reasons differ: interp {interpreted:?}, jit {jit:?}"
        )),
    }
}
