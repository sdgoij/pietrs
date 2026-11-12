//! Writes the interpreter's image.
//!
//! ```sh
//! cargo run -r -p loom -- target/weave/interpreter.png
//! ```
//!
//! One image, the same for every program: it takes a ring stream and a start
//! state on its input, resolves the walk, and runs each state's command on a
//! value stack, printing what the interpreted program prints. Slice 4 puts the
//! PPM decoder in front, at which point the input is a picture instead of a
//! stream.
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
    let image = loom::run_grid()?;
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
