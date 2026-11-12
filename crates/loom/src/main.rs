//! Writes the interpreter's image.
//!
//! ```sh
//! cargo run -r -p loom -- target/weave/interpreter.png
//! ```
//!
//! One image, the same for every program: it takes a ring stream and a start
//! state on its input and prints the trace of the walk it resolves. Slicing 2 and
//! 3 of the note add the value stack and the commands, and slice 4 puts the PPM
//! decoder in front, at which point the input is a picture and the output is the
//! interpreted program's own output.
//!
//! Run it the way any image runs:
//!
//! ```sh
//! cargo run -r -p cli -- build target/weave/interpreter.png -o interpreter.exe
//! ./interpreter.exe < program.ppm
//! ```

use std::path::PathBuf;

/// Where the image goes when no path is given.
const DEFAULT: &str = "target/weave/interpreter.png";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from(DEFAULT), PathBuf::from);
    let image = loom::machine_grid()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, image.encode_png()?)?;
    println!(
        "{}: {}x{} codels",
        path.display(),
        image.width(),
        image.height()
    );
    Ok(())
}
