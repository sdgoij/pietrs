//! Runs a Piet program written as a PPM, all the way to its output.
//!
//! ```sh
//! cargo run -p loom --example run-ppm -- program.ppm
//! cargo run -p loom --example run-ppm -- program.ppm " 10 3"
//! ```
//!
//! This is the lazy machine end to end: the PPM image decodes the picture into
//! a grid, the run image partitions the ring and then loops over the movement
//! and the commands. The host still carries the ring stream between the two
//! images; putting both in one image is slice 4.
//!
//! The second argument is the interpreted program's own input, appended to the
//! machine's stream after the start state. It is taken verbatim, so the caller
//! separates it as the stream needs: a leading space before a number read,
//! nothing before a character read.
//!
//! The PPM is `P3`, `maxval 255`, no comments, one codel per pixel — what
//! `ppm_grid` accepts. `crates/loom/tests/ppm.rs` writes one from any grid.

use std::path::PathBuf;

use canvas::Blocks;
use interp::Options;
use runtime::BufferIo;
use tessera::Start;

/// The run image's budget: a move is a scan over the whole ring.
const STEPS: u64 = 2_000_000_000;

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
    let blocks = Blocks::partition(grid.clone());
    println!(
        "{}: {}x{} codels, {} blocks",
        path.display(),
        grid.width(),
        grid.height(),
        blocks.len()
    );

    let Start::State(start) = tessera::start(&blocks) else {
        eprintln!("--- the program terminates before it runs ---");
        return Ok(());
    };
    let (image, stream) = loom::run(&grid, start.block, start.dp, start.cc)?;
    let stream = format!("{stream}{input}");
    let machine = Blocks::partition(image);
    let mut io = BufferIo::new(&stream);
    let options = Options {
        max_steps: Some(STEPS),
        ..Options::default()
    };
    let result = interp::run(&machine, &mut io, &options)?;
    println!("--- output ---");
    print!("{}", io.output());
    if !io.output().ends_with('\n') {
        println!();
    }
    eprintln!("--- {:?} after {} steps ---", result.reason, result.steps);
    Ok(())
}
