//! Building a grid from fuzz bytes, and the invariants every grid must hold.
//!
//! `cargo-fuzz` builds each target as its own binary, so this module is included
//! by path instead of linked. Each target uses a subset of it.

#![allow(dead_code)]

use canvas::{Blocks, CodelGrid, Color};

/// The colour a byte selects. All twenty appear, so white and black are common.
pub fn color(code: u8) -> Color {
    Color::ALL[usize::from(code) % Color::ALL.len()]
}

/// Builds a grid of at most `max_side` codels on a side.
///
/// The first two bytes pick the shape, so the fuzzer steers the geometry as well
/// as the colours. A grid that cannot be built is not a finding.
pub fn grid(data: &[u8], max_side: u32) -> Option<CodelGrid> {
    let (header, rest) = data.split_first_chunk::<2>()?;
    let width = u32::from(header[0]) % max_side + 1;
    let height = u32::from(header[1]) % max_side + 1;
    let count = (width as usize) * (height as usize);
    let codels: Vec<Color> = (0..count)
        .map(|index| color(rest.get(index).copied().unwrap_or(0)))
        .collect();
    CodelGrid::from_colors(width, height, codels).ok()
}

/// Asserts the partition invariants: total, disjoint, and self-consistent.
pub fn assert_partition(width: u32, height: u32, blocks: &Blocks) {
    let mut total = 0;
    for block in blocks.blocks() {
        total += block.size();
        for point in block.codels() {
            assert_eq!(
                blocks.block_id_at(*point),
                Some(block.id()),
                "a codel of a block must name that block"
            );
        }
    }
    assert_eq!(
        total,
        (width as usize) * (height as usize),
        "every codel belongs to exactly one block"
    );
    assert_eq!(
        blocks.len(),
        blocks.blocks().len(),
        "the block count agrees"
    );
}
