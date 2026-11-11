//! Writes the generated interpreter next to a program it can run.
//!
//! ```sh
//! cargo run -p loom --example weave -- target/weave
//! cargo run -p loom --example weave -- target/weave crates/gui/web/hello.png
//! target/debug/pietrs run target/weave/interpreter.png --input target/weave/program.txt
//! ```
//!
//! The instruction stream is read from the program's trace, so the program has
//! to be linear. One that runs `pointer` or `switch` is reported, naming the
//! block to look at, rather than run and left to overflow the stack.

use std::fs;
use std::path::PathBuf;

use canvas::{Blocks, CodelGrid};
use interp::Options;
use loom::{encode, instructions, interpreter, linear_program};
use runtime::BufferIo;

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

    let options = Options {
        max_steps: Some(1_000_000),
        ..Options::default()
    };
    let blocks = Blocks::partition(program.clone());
    let mut io = BufferIo::new("");
    let trace =
        instructions(&blocks, &mut io, &options).map_err(|error| format!("{label}: {error}"))?;
    let stream = encode(&trace, "").map_err(|error| format!("{label}: {error}"))?;

    let machine = interpreter()?;
    let (actual, _) = run_grid(&machine, &stream, &options)?;
    let (expected, _) = run_grid(&program, "", &options)?;

    fs::write(directory.join("interpreter.png"), machine.encode_png()?)?;
    fs::write(directory.join("program.png"), program.encode_png()?)?;
    fs::write(directory.join("program.txt"), &stream)?;

    println!(
        "{label}: {} instructions, the program prints {expected:?}, the interpreter prints {actual:?}",
        trace.len()
    );
    Ok(())
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
