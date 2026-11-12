//! Runs a Piet program written as a PPM, all the way to its output.
//!
//! ```sh
//! cargo run -p loom --example run-ppm -- program.ppm
//! cargo run -p loom --example run-ppm -- program.ppm "10 3"
//! ```
//!
//! The pipeline is the frontend end to end. The generated image decodes the
//! PPM into a grid of colour codes; the generated movement image resolves every
//! move into a state graph; the generated walker runs the records. The host
//! carries text between them and supplies the bookkeeping the records need —
//! block sizes, the label-to-block map, and the ring offsets.
//!
//! The second argument is the interpreted program's own input. A graph decides
//! which reads happen, so its reads are **numbers**: a number read takes the
//! number, a character read takes the character's code point.
//!
//! The PPM is `P3`, `maxval 255`, no comments, one codel per pixel — what
//! `ppm_grid` accepts. `crates/loom/tests/ppm.rs` writes one from any grid.

use std::path::PathBuf;

use canvas::Blocks;
use interp::Options;
use runtime::BufferIo;

/// The grid image's budget for one run; a move is a scan over the whole ring.
const GRID_STEPS: u64 = 400_000_000;
/// The walker's budget: one interpreted command is hundreds of machine steps.
const WALK_STEPS: u64 = 10_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: run-ppm <program.ppm> [input]");
        std::process::exit(2);
    };
    let input = args
        .next()
        .map(|argument| argument.to_string_lossy().into_owned())
        .unwrap_or_default();

    let source = std::fs::read_to_string(&path)?;
    let grid = loom::ppm_to_grid(&source)?;
    // Both come from images: the splitter names and sizes the blocks, and the
    // start and exit stages resolve every move.
    let (graph, partition) = loom::grid_graph(&grid, GRID_STEPS)?;
    println!(
        "{}: {}x{} codels, {} blocks",
        path.display(),
        grid.width(),
        grid.height(),
        partition.len()
    );
    println!("{} states", graph.states().len());

    let (image, records) = loom::graph_walker(&graph, &partition, &input)?;
    let walker = Blocks::partition(image);
    let mut io = BufferIo::new(&records);
    let options = Options {
        max_steps: Some(WALK_STEPS),
        ..Options::default()
    };
    let result = interp::run(&walker, &mut io, &options)?;
    println!("--- output ---");
    print!("{}", io.output());
    if !io.output().ends_with('\n') {
        println!();
    }
    eprintln!("--- {:?} after {} steps ---", result.reason, result.steps);
    Ok(())
}
