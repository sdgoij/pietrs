#![allow(clippy::unwrap_used)]
//! The machine's first slice: the movement rule in a loop, printed as a trace.
//!
//! Every case compares the image's trace with the walk `tessera` resolves from
//! the same state, mapped through the splitter's labels. The machine prints the
//! state each move reaches, so the two sequences are the same numbers.
//!
//! A fixture that loops is compared for as many moves as the budget allows: the
//! point of these cases is that the image and the host agree move for move, not
//! that a toy program halts.

use std::fmt::Write;

use canvas::{Blocks, CodelGrid, Color, Hue, Lightness};
use interp::Options;
use runtime::BufferIo;
use tessera::{Exit, Start, State};

/// The splitter's own budget: the partition is ~80 000 steps a cell.
const PARTITION_STEPS: u64 = 400_000_000;
/// A budget for a fixture of a few dozen codels: enough for several moves, so a
/// looping program is compared rather than run for ever.
const SMALL_STEPS: u64 = 40_000_000;
/// The canonical hello world: 30 x 29 codels, 25 states, about 21M steps a move.
const HELLO_STEPS: u64 = 2_000_000_000;
/// A walk longer than this is not a fixture these cases are for.
const MOVES: usize = 400;

/// A grid from rows of characters, as in the `grid` tests.
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
                'k' => Color::chromatic(Hue::Red, Lightness::Dark),
                other => panic!("no colour for {other:?}"),
            });
        }
    }
    CodelGrid::from_colors(width, height, codels).unwrap()
}

/// The trace the host resolves: one move per line, then `-1`.
fn expected(grid: &CodelGrid) -> String {
    let blocks = Blocks::partition(grid.clone());
    let partition = loom::grid_partition(grid, PARTITION_STEPS).unwrap();
    let Start::State(mut state) = tessera::start(&blocks) else {
        return "-1".to_owned();
    };
    let mut text = String::new();
    for _ in 0..MOVES {
        let Exit::Move {
            target,
            dp,
            cc,
            command,
        } = tessera::exit(&blocks, state.block, state.dp, state.cc)
        else {
            text.push_str("-1");
            return text;
        };
        writeln!(
            text,
            "{} {} {} {}",
            partition.label_of(target).unwrap(),
            dp as u32,
            cc as u32,
            loom::record_code(command)
        )
        .unwrap();
        state = State {
            block: target,
            dp,
            cc,
        };
    }
    text
}

/// Runs the machine from the grid's start state and compares the trace.
fn check(grid: &CodelGrid, steps: u64) {
    let blocks = Blocks::partition(grid.clone());
    let Start::State(start) = tessera::start(&blocks) else {
        panic!("the fixture terminates before it runs");
    };
    let want = expected(grid);
    let (image, stream) = loom::machine(grid, start.block, start.dp, start.cc).unwrap();
    let mut io = BufferIo::new(&stream);
    interp::run(
        &Blocks::partition(image),
        &mut io,
        &Options {
            max_steps: Some(steps),
            ..Options::default()
        },
    )
    .unwrap();

    let got: Vec<&str> = io.output().lines().collect();
    let want: Vec<&str> = want.lines().collect();
    assert!(
        got.len() >= 2,
        "the machine printed {} moves, the first was {:?}",
        got.len(),
        got.first()
    );
    for (index, (image, host)) in got.iter().zip(want.iter()).enumerate() {
        assert_eq!(image, host, "move {index}");
    }
}

#[test]
fn a_chromatic_walk_is_the_host_s() {
    // Two moves, then a slide that retraces: the trace covers a direct move and
    // the slide's `nop`.
    check(&grid(&["rbg", "###"]), SMALL_STEPS);
}

#[test]
#[ignore = "the canonical hello world, 33 s: cargo test -p loom --release --test machine -- --ignored"]
fn the_hello_world_trace_is_the_host_s() {
    // 24 moves to `-1`, the whole program's control flow.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let grid = CodelGrid::from_image(png).unwrap();
    check(&grid, HELLO_STEPS);
}

#[test]
fn there_is_one_image_for_every_program() {
    // The image is the deliverable, so a program may change only the stream.
    // `cargo run -p loom` writes this image; nothing about a grid reaches it.
    let one = loom::machine_grid().unwrap();
    let (image, _) = loom::machine(
        &grid(&["rg", "gr"]),
        canvas::BlockId(1),
        tessera::Direction::Right,
        tessera::Chooser::Left,
    )
    .unwrap();
    assert_eq!(image, one);
}
