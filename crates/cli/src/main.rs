//! The `pietrs` command line: check, trace, and run a Piet image.

use std::io::Write as _;
use std::path::PathBuf;
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
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Commands::Check(args) => check(&args).map(|()| ExitCode::SUCCESS),
        Commands::Trace(args) => trace(&args).map(|()| ExitCode::SUCCESS),
        Commands::Run(args) => run_program(&args),
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
