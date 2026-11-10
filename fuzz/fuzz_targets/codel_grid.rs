//! Arbitrary codel grids from all twenty colours.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "grid.rs"]
mod grid;

fuzz_target!(|data: &[u8]| {
    let Some(codel_grid) = grid::grid(data, 16) else {
        return;
    };
    let (width, height) = (codel_grid.width(), codel_grid.height());
    let blocks = canvas::Blocks::partition(codel_grid);
    grid::assert_partition(width, height, &blocks);
});
