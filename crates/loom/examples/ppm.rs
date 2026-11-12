//! Writes the image that reads a PPM.
//!
//! ```sh
//! cargo run -p loom --example ppm -- target/weave/ppm.png
//! target/debug/pietrs run target/weave/ppm.png < program.ppm
//! ```
//!
//! The image reads a P3 PPM from its input and prints the grid it decodes to:
//! the width, the height, then one colour code per codel in row-major order,
//! each followed by a space, and a final newline. The codes are indices into
//! `canvas::Color::ALL`, so 18 is white and 19 is black.
//!
//! `P6`, a maxval other than 255, and comments are refused: the first prints
//! `-1` and stops, the other two print the dimensions and then `-1`, which no
//! colour code can be.

use std::path::PathBuf;

use loom::ppm_grid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: ppm <output.png>");
        std::process::exit(2);
    };
    let image = ppm_grid()?;
    std::fs::write(&path, image.encode_png()?)?;
    println!(
        "wrote {}: {}x{}, {} blocks",
        path.display(),
        image.width(),
        image.height(),
        canvas::Blocks::partition(image).len()
    );
    println!("read it with: pietrs run {} < program.ppm", path.display());
    Ok(())
}
