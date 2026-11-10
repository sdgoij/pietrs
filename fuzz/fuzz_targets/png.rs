//! PNG decode, quantisation, codel-size detection, and partitioning.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "grid.rs"]
mod grid;

fuzz_target!(|data: &[u8]| {
    // A malformed image is a diagnostic, never a panic.
    let Ok(codel_grid) = canvas::CodelGrid::from_image(data) else {
        return;
    };
    let (width, height) = (codel_grid.width(), codel_grid.height());
    let blocks = canvas::Blocks::partition(codel_grid);
    grid::assert_partition(width, height, &blocks);
});
