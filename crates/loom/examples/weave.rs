//! Writes the generated interpreter next to a sample program it can run.
//!
//! ```sh
//! cargo run -p loom --example weave -- target/weave
//! target/debug/pietrs run target/weave/interpreter.png --input target/weave/program.txt
//! ```

use std::fs;
use std::path::PathBuf;

use canvas::Blocks;
use interp::Options;
use loom::{encode, instructions, interpreter, linear_program};
use runtime::BufferIo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/weave"), PathBuf::from);
    fs::create_dir_all(&directory)?;

    let program = linear_program("Hello, Piet!")?;
    fs::write(directory.join("program.png"), program.encode_png()?)?;

    let blocks = Blocks::partition(program);
    let mut io = BufferIo::new("");
    let trace = instructions(&blocks, &mut io, &Options::default())?;
    let stream = encode(&trace, "")?;
    fs::write(directory.join("program.txt"), &stream)?;

    let machine = interpreter()?;
    fs::write(directory.join("interpreter.png"), machine.encode_png()?)?;

    let mut io = BufferIo::new(&stream);
    interp::run(&Blocks::partition(machine), &mut io, &Options::default())?;
    println!("printed {:?}", io.output());
    Ok(())
}
