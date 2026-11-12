//! Writes the machine's image.
//!
//! ```sh
//! cargo run -r -p loom -- target/weave/machine.png
//! ```
//!
//! One image, the same for every program: it reads a P3 PPM from its input,
//! partitions the grid, finds the start state and runs the program's commands on
//! a value stack, printing what the interpreted program prints. Anything after
//! the PPM's pixels is that program's own input.
//!
//! Run it the way any image runs:
//!
//! ```sh
//! cargo run -r -p cli -- build target/weave/machine.png -o machine.exe
//! ./machine.exe < program.ppm
//! ```

use std::path::PathBuf;

/// Where the image goes when no path is given.
const DEFAULT: &str = "target/weave/machine.png";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from(DEFAULT), PathBuf::from);
    let image = loom::ppm_run_grid()?;
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
