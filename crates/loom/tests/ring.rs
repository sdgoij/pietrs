#![allow(clippy::unwrap_used)]
//! The ring, built from colours alone.
//!
//! Every case compares the image's stream with `loom::ring_stream`, token for
//! token — the same helper `splitter`, `start_state` and `exit_state` take their
//! streams from, so a disagreement here is a disagreement with every stage
//! downstream.

use canvas::{CodelGrid, Color, Hue, Lightness};

/// A grid from rows of characters, as in the `tessera` tests.
fn grid(rows: &[&str]) -> CodelGrid {
    let height = rows.len() as u32;
    let width = rows.first().map_or(0, |row| row.len() as u32);
    let mut codels = Vec::with_capacity((width as usize) * (height as usize));
    for row in rows {
        assert_eq!(row.len() as u32, width, "rows must be rectangular");
        for cell in row.chars() {
            codels.push(match cell {
                '.' => Color::White,
                '#' => Color::Black,
                'r' => Color::chromatic(Hue::Red, Lightness::Normal),
                'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                'b' => Color::chromatic(Hue::Blue, Lightness::Normal),
                'd' => Color::chromatic(Hue::Cyan, Lightness::Dark),
                'l' => Color::chromatic(Hue::Blue, Lightness::Light),
                other => panic!("no colour for {other:?}"),
            });
        }
    }
    CodelGrid::from_colors(width, height, codels).unwrap()
}

/// Runs the ring image over the grid's codes and compares with the host's ring.
fn check(grid: &CodelGrid) {
    let expected = loom::ring_stream(grid).unwrap();
    let output = loom::ring_to_stream(&loom::grid_codes(grid)).unwrap();
    assert_eq!(output, expected, "\n{}", loom::grid_codes(grid));
}

#[test]
fn a_single_codel_is_a_ring_of_guards_around_it() {
    // 3 x 3 cells: eight guards and the one colour.
    check(&grid(&["r"]));
}

#[test]
fn every_cell_of_a_small_grid_lands_where_the_host_puts_it() {
    check(&grid(&["rg", "gr"]));
}

#[test]
fn white_black_and_the_borders_keep_their_codes() {
    check(&grid(&["r..b", ".#g.", "l..d"]));
}

#[test]
fn the_hello_world_grid_becomes_its_ring() {
    // 992 ring cells from a real program, not a fixture built to fit the test.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let grid = CodelGrid::from_image(png).unwrap();
    check(&grid);
}

#[test]
fn the_stream_is_the_one_the_splitter_reads() {
    // The ordering is the part that matters: the head must be read last, so the
    // last record printed is cell zero.
    let grid = grid(&["rg", "gr"]);
    let (_, stream) = loom::splitter(&grid).unwrap();
    assert_eq!(stream, loom::ring_stream(&grid).unwrap());
    assert_eq!(
        stream,
        loom::ring_to_stream(&loom::grid_codes(&grid)).unwrap(),
        "the image's stream is the splitter's"
    );
}
