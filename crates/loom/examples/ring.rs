//! Prints the ring stream a PPM's grid becomes, built by the ring image.
//!
//! ```sh
//! cargo run -p loom --example ring -- program.ppm
//! ```
//!
//! The frontend is now two images speaking text: [`loom::ppm_grid`] turns the
//! PPM into colour codes, and [`loom::ring_grid`] turns those codes into the
//! `ring stride colour index` records the splitter and the movement stages
//! take. The host only carries the text between them, and checks the result
//! against [`loom::ring_stream`] — the same stream it used to build itself.
//!
//! The PPM is `P3`, `maxval 255`, no comments, one codel per pixel — what
//! `ppm_grid` accepts. `crates/loom/tests/ppm.rs` writes one from any grid.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: ring <program.ppm>");
        std::process::exit(2);
    };

    let source = std::fs::read_to_string(&path)?;
    let grid = loom::ppm_to_grid(&source)?;
    let codes = loom::grid_codes(&grid);
    let stream = loom::ring_to_stream(&codes)?;
    let host = loom::ring_stream(&grid)?;

    println!(
        "{}: {}x{}, {} codels, {} ring cells",
        path.display(),
        grid.width(),
        grid.height(),
        grid.codels().len(),
        stream.split_whitespace().next().unwrap_or("0"),
    );
    println!("{stream}");
    if stream == host {
        println!("--- matches the host's ring ---");
    } else {
        eprintln!("--- the image and the host disagree ---");
        std::process::exit(1);
    }
    Ok(())
}
