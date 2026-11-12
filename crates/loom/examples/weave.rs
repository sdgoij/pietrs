//! Writes the walker next to the program it runs.
//!
//! ```sh
//! cargo run -p loom --example weave -- target/weave
//! cargo run -p loom --example weave -- target/weave path/to/program.png
//! cargo run -p loom --example weave -- target/weave path/to/program.png "1 2 3"
//! target/debug/pietrs run target/weave/walker.png --input target/weave/program.txt
//! ```
//!
//! A straight-line program is woven from its trace. One that branches is woven
//! from its state graph, where the image chooses the successor at run time, so
//! `pointer` and `switch` are not an error here.
//!
//! The image is written as `walker.png`; `interpreter.png` belonged to the trace
//! interpreter this example used to build, and is stale if it is still there.
//!
//! The third argument is the program's own input. For a straight line it is the
//! text the program reads; for a graph it is **numbers**, one per read, because
//! a graph decides which reads happen. A number read takes the number; a
//! character read takes the character's code point, because the stream is
//! numeric. A graph's reads must also
//! not run out: nothing in the image can tell a read that happened from one that
//! did not.
//!
//! The fourth argument is the woven image's step budget. It is not the same
//! figure as the reference's: one interpreted command costs hundreds of machine
//! steps, so a big program needs a large budget and patience.

use std::fs;
use std::path::PathBuf;

use canvas::{Blocks, CodelGrid};
use interp::Options;
use loom::{Error, graph_walker, instructions, linear_program, walker};
use runtime::BufferIo;
use tessera::StateGraph;

/// Steps the reference is allowed.
const REFERENCE_STEPS: u64 = 1_000_000;
/// Steps the woven image is allowed unless the caller says otherwise.
const WALKER_STEPS: u64 = 50_000_000;

fn main() {
    if let Err(error) = run() {
        eprintln!("weave: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .map_or_else(|| PathBuf::from("target/weave"), PathBuf::from);
    fs::create_dir_all(&directory)?;

    let (program, label) = match arguments.next() {
        Some(path) => (CodelGrid::from_image(&fs::read(&path)?)?, path),
        None => (linear_program("Hello, Piet!")?, String::from("generated")),
    };
    let input = arguments.next().unwrap_or_default();
    let steps = arguments
        .next()
        .map_or(Ok(WALKER_STEPS), |text| text.parse())?;

    let reference = Options {
        max_steps: Some(REFERENCE_STEPS),
        ..Options::default()
    };
    let blocks = Blocks::partition(program.clone());
    let mut io = BufferIo::new(&input);
    let (image, stream, what) = match instructions(&blocks, &mut io, &reference) {
        Ok(trace) => {
            let (image, stream) =
                walker(&trace, &input).map_err(|error| format!("{label}: {error}"))?;
            (image, stream, format!("{} instructions", trace.len()))
        }
        Err(Error::NotLinear { .. }) => {
            let graph = StateGraph::build(&blocks);
            let (image, stream) = graph_walker(&graph, &blocks, &input)
                .map_err(|error| format!("{label}: {error}"))?;
            (image, stream, format!("{} states", graph.states().len()))
        }
        Err(error) => return Err(format!("{label}: {error}").into()),
    };

    let woven = Options {
        max_steps: Some(steps),
        ..Options::default()
    };
    let (actual, walked) = run_grid(&image, &stream, &woven)?;
    let (expected, _) = run_grid(&program, &input, &reference)?;

    fs::write(directory.join("walker.png"), image.encode_png()?)?;
    fs::write(directory.join("program.png"), program.encode_png()?)?;
    fs::write(directory.join("program.txt"), &stream)?;

    println!(
        "{label}: {what}; walker {}x{}; stream {} bytes",
        image.width(),
        image.height(),
        stream.len()
    );
    println!(
        "  reference: {} chars, {} bytes: {:?}",
        expected.chars().count(),
        expected.len(),
        prefix(&expected)
    );
    println!(
        "  walker:    {:?} after {} steps, {} bytes: {:?}",
        walked.reason,
        walked.steps,
        actual.len(),
        prefix(&actual)
    );
    println!(
        "  run it with: pietrs run {}/walker.png --input {}/program.txt --max-steps {steps}",
        directory.display(),
        directory.display()
    );
    println!("  {}", verdict(&expected, &actual));
    Ok(())
}

/// What the two runs say about each other.
///
/// The budgets differ — one interpreted command is hundreds of machine steps —
/// so a shorter run is expected to be a prefix of a longer one.
fn verdict(expected: &str, actual: &str) -> &'static str {
    if actual == expected {
        "the same output"
    } else if expected.starts_with(actual) {
        "the walker stopped inside the reference's output"
    } else if actual.starts_with(expected) {
        "the walker printed more; the reference hit its step limit"
    } else {
        "different output"
    }
}

/// The first line or so, so a long run does not flood the terminal.
fn prefix(output: &str) -> String {
    const LIMIT: usize = 120;
    let mut shown = output.chars().take(LIMIT).collect::<String>();
    if output.chars().count() > LIMIT {
        shown.push('…');
    }
    shown
}

fn run_grid(
    grid: &CodelGrid,
    input: &str,
    options: &Options,
) -> Result<(String, interp::Run), Box<dyn std::error::Error>> {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let result = interp::run(&blocks, &mut io, options)?;
    Ok((io.into_output(), result))
}
