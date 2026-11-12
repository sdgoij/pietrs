//! Runs a Piet program written as a PPM, all the way to its output.
//!
//! ```sh
//! cargo run -p loom --example run-ppm -- program.ppm
//! cargo run -p loom --example run-ppm -- program.ppm " 10 3"
//! ```
//!
//! This is the single image: the PPM decodes to colour codes, the codes build the
//! ring, the splitter partitions it, the start stage finds the state, and the
//! movement loop runs the commands. Nothing is carried between the stages by the
//! host.
//!
//! The second argument is the interpreted program's own input, appended to the
//! PPM's pixels. It is taken verbatim, so the caller separates it as the stream
//! needs: a leading space before a number read, nothing before a character read.
//!
//! The PPM is `P3`, `maxval 255`, no comments, one codel per pixel — what
//! `ppm_grid` accepts. `crates/loom/tests/ppm.rs` writes one from any grid.

use std::path::PathBuf;

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
    let output = loom::ppm_run(&format!("{source}{input}"))?;
    println!("--- output ---");
    print!("{output}");
    if !output.ends_with('\n') {
        println!();
    }
    Ok(())
}
