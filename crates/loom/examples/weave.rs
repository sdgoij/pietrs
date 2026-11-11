//! Writes the generated interpreter next to a program it can run.
//!
//! ```sh
//! cargo run -p loom --example weave -- target/weave
//! cargo run -p loom --example weave -- target/weave crates/gui/web/hello.png
//! target/debug/pietrs run target/weave/interpreter.png --input target/weave/program.txt
//! ```

use std::fs;
use std::path::PathBuf;

use canvas::{Blocks, CodelGrid};
use interp::Options;
use loom::{encode, instructions, interpreter, linear_program};
use runtime::BufferIo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .map_or_else(|| PathBuf::from("target/weave"), PathBuf::from);
    fs::create_dir_all(&directory)?;

    let program = match arguments.next() {
        Some(path) => CodelGrid::from_image(&fs::read(path)?)?,
        None => linear_program("Hello, Piet!")?,
    };

    let (expected, _) = run(&program, "")?;
    let trace = instructions(
        &Blocks::partition(program.clone()),
        &mut BufferIo::new(""),
        &Options::default(),
    )?;
    let stream = encode(&trace, "")?;

    let machine = interpreter()?;
    let (actual, _) = run(&machine, &stream)?;

    fs::write(directory.join("interpreter.png"), machine.encode_png()?)?;
    fs::write(directory.join("program.png"), program.encode_png()?)?;
    fs::write(directory.join("program.txt"), &stream)?;

    println!("the program prints {expected:?}");
    println!("the interpreter prints {actual:?}");
    Ok(())
}

fn run(grid: &CodelGrid, input: &str) -> Result<(String, interp::Run), Box<dyn std::error::Error>> {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let result = interp::run(&blocks, &mut io, &Options::default())?;
    Ok((io.into_output(), result))
}
