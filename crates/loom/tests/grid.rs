#![allow(clippy::unwrap_used)]
//! The splitter: blocks from colours alone.
//!
//! Every case compares the image's labels with `canvas::Blocks` — never with a
//! second implementation of the same rule. Two cells share a label exactly when
//! the host partition puts them in one block.

use std::collections::HashMap;

use canvas::{BlockId, Blocks, CodelGrid, Color, Hue, Lightness, Point};
use interp::{ExitReason, Options, Run};
use loom::{block_edge, exit_state, record_code, splitter, start_state, step_state};
use runtime::BufferIo;
use tessera::{Chooser, Direction};

fn run(grid: &CodelGrid, input: &str, steps: u64) -> (String, Run) {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let options = Options {
        max_steps: Some(steps),
        ..Options::default()
    };
    let result = interp::run(&blocks, &mut io, &options).unwrap();
    (io.into_output(), result)
}

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
                'd' => Color::chromatic(Hue::Cyan, Lightness::Dark),
                'l' => Color::chromatic(Hue::Cyan, Lightness::Light),
                'k' => Color::chromatic(Hue::Red, Lightness::Dark),
                'b' => Color::chromatic(Hue::Blue, Lightness::Normal),
                other => panic!("unknown cell {other:?}"),
            });
        }
    }
    CodelGrid::from_colors(width, height, codels).unwrap()
}

/// The labels the splitter should print: the lowest ring position in each
/// component, with each guard its own component.
fn expected(grid: &CodelGrid) -> Vec<u32> {
    let (width, height) = (grid.width(), grid.height());
    let row = width + 2;
    let cells = row * (height + 2);
    let blocks = Blocks::partition(grid.clone());

    let mut lowest: HashMap<_, u32> = HashMap::new();
    for index in 0..cells {
        let Some(point) = codel(width, height, index) else {
            continue;
        };
        let id = blocks.block_id_at(point).unwrap();
        lowest.entry(id).or_insert(index);
    }

    (0..cells)
        .map(|index| match codel(width, height, index) {
            Some(point) => lowest[&blocks.block_id_at(point).unwrap()],
            None => index,
        })
        .collect()
}

/// The codel a ring index stands for, mirroring the module's layout.
fn codel(width: u32, height: u32, index: u32) -> Option<Point> {
    let row = index / (width + 2);
    let column = index % (width + 2);
    if row == 0 || row > height || column == 0 || column > width {
        return None;
    }
    Some(Point::new(column - 1, row - 1))
}

/// Run the splitter on a grid and read its labels back.
fn labels(grid: &CodelGrid, steps: u64) -> Vec<u32> {
    let (image, stream) = splitter(grid).unwrap();
    let (output, result) = run(&image, &stream, steps);
    assert_eq!(result.reason, ExitReason::Terminated);
    output
        .split_whitespace()
        .map(|token| token.parse().unwrap())
        .collect()
}

fn check(rows: &[&str]) {
    let grid = grid(rows);
    assert_eq!(labels(&grid, 40_000_000), expected(&grid), "{rows:?}");
}

#[test]
fn a_row_of_two_colours_is_two_blocks() {
    check(&["rg"]);
}

#[test]
fn equal_neighbours_share_a_label() {
    check(&["rrgg"]);
    check(&["rrrr"]);
}

#[test]
fn a_horizontal_merge_takes_the_lowest_position() {
    check(&["rggr"]);
}

#[test]
fn a_vertical_merge_reaches_up() {
    check(&["r", "r", "r"]);
    check(&["r", "g", "r"]);
}

#[test]
fn blocks_are_joined_only_through_edges() {
    // The two reds touch a corner, not an edge.
    check(&["rg", "gr"]);
}

#[test]
fn a_hole_is_its_own_block() {
    check(&["rrr", "r.r", "rrr"]);
}

#[test]
fn the_guard_row_does_not_join_the_last_row_to_the_first() {
    // The top and bottom rows are both red but not connected.
    let grid = grid(&["r", "g", "r"]);
    let labels = labels(&grid, 40_000_000);
    // (0, 0) and (0, 2) are three rows apart in a three-row-wide ring.
    assert_ne!(labels[4], labels[10], "rows wrap through the ring");
    assert_eq!(labels[4], 4);
    assert_eq!(labels[10], 10);
}

#[test]
fn white_and_black_are_colours_like_any_other() {
    check(&["..#..", "..#..", "###.."]);
}

#[test]
fn a_larger_grid_partitions_the_same_way() {
    check(&["rgggr", "rg.gd", "rg.gg", "rrrrd"]);
}

#[test]
fn the_image_is_the_same_for_every_grid() {
    let (small, _) = splitter(&grid(&["r"])).unwrap();
    let (large, _) = splitter(&grid(&["rgggr", "rg.gd", "rg.gg", "rrrrd"])).unwrap();
    assert_eq!(small, large);
}

/// Every state of every block of `rows`, against the same selection in
/// `tessera`.
fn check_edges(rows: &[&str], blocks_to_check: Option<usize>) {
    let grid = grid(rows);
    let blocks = Blocks::partition(grid.clone());
    let reached = blocks_to_check.unwrap_or(usize::MAX);
    for block in blocks.blocks().iter().take(reached) {
        for dp in Direction::ALL {
            for cc in [Chooser::Left, Chooser::Right] {
                let (image, stream) = block_edge(&grid, block.id(), dp, cc).unwrap();
                let (output, result) = run(&image, &stream, 40_000_000);
                assert_eq!(result.reason, ExitReason::Terminated);
                let found: Vec<i64> = output
                    .split_whitespace()
                    .map(|token| token.parse().unwrap())
                    .collect();
                let expected = tessera::block_edge(&blocks, block.id(), dp, cc);
                let what = format!("{rows:?} block {} {dp:?} {cc:?}", block.id().0);
                match expected {
                    Some(point) => assert_eq!(
                        found,
                        vec![i64::from(point.x), i64::from(point.y)],
                        "{what}"
                    ),
                    None => assert!(found.is_empty(), "{what}: unexpected {found:?}"),
                }
            }
        }
    }
}

#[test]
fn the_edge_codel_matches_tessera() {
    // A two-codel block, every direction and chooser; the sweep below covers
    // the other shapes.
    check_edges(&["rrgg"], Some(1));
}

#[test]
#[ignore = "the full sweep: cargo test -p loom --release --test grid -- --ignored"]
fn the_edge_codel_matches_tessera_everywhere() {
    check_edges(&["rrgg"], None);
    check_edges(&["rg", "gr"], None);
    check_edges(&["rrr", "r.r", "rrr"], None);
    check_edges(&["rgggr", "rg.gd", "rg.gg", "rrrrd"], None);
    check_edges(&[".r.", "rrr", ".r."], None);
}

/// Every state of every block of `rows`, one attempt of the movement rules.
///
/// The oracle is `tessera`'s own primitive: the block's edge codel, one step in
/// the direction, and the colour it lands on.
fn check_steps(rows: &[&str], blocks_to_check: Option<usize>) {
    let grid = grid(rows);
    let blocks = Blocks::partition(grid.clone());
    let width = grid.width();
    let label = |id: BlockId| {
        blocks
            .block(id)
            .unwrap()
            .codels()
            .iter()
            .map(|point| (point.y + 1) * (width + 2) + (point.x + 1))
            .min()
            .unwrap()
    };

    let reached = blocks_to_check.unwrap_or(usize::MAX);
    for block in blocks.blocks().iter().take(reached) {
        for dp in Direction::ALL {
            for cc in [Chooser::Left, Chooser::Right] {
                let (image, stream) = step_state(&grid, block.id(), dp, cc).unwrap();
                let (output, result) = run(&image, &stream, 40_000_000);
                assert_eq!(result.reason, ExitReason::Terminated);
                let found: Vec<i64> = output
                    .split_whitespace()
                    .map(|token| token.parse().unwrap())
                    .collect();

                let expected = match tessera::block_edge(&blocks, block.id(), dp, cc) {
                    None => Vec::new(),
                    Some(edge) => match tessera::step_codel(edge, dp) {
                        None => vec![-1],
                        Some(next) => match grid.get(next) {
                            None | Some(Color::Black) => vec![-1],
                            Some(Color::White) => vec![-2],
                            Some(_) => {
                                let id = blocks.block_id_at(next).unwrap();
                                vec![
                                    i64::from(label(id)),
                                    i64::from(dp as u32),
                                    i64::from(cc as u32),
                                ]
                            }
                        },
                    },
                };
                let what = format!("{rows:?} block {} {dp:?} {cc:?}", block.id().0);
                assert_eq!(found, expected, "{what}");
            }
        }
    }
}

#[test]
fn the_step_matches_tessera() {
    check_steps(&["rrgg"], Some(1));
}

#[test]
#[ignore = "the full sweep: cargo test -p loom --release --test grid -- --ignored"]
fn the_step_matches_tessera_everywhere() {
    check_steps(&["rrgg"], None);
    check_steps(&["rg", "gr"], None);
    check_steps(&["r", "r"], None);
    check_steps(&["r.", "gg"], None);
    check_steps(&[".r.", "rrr", ".r."], None);
}

/// Every state of every block of `rows`, the whole movement rule.
///
/// The oracle is `tessera`'s `exit`: the eight attempts and the white slide.
fn check_exits(rows: &[&str], blocks_to_check: Option<usize>) {
    let grid = grid(rows);
    let blocks = Blocks::partition(grid.clone());
    let width = grid.width();
    let label = |id: BlockId| {
        blocks
            .block(id)
            .unwrap()
            .codels()
            .iter()
            .map(|point| (point.y + 1) * (width + 2) + (point.x + 1))
            .min()
            .unwrap()
    };

    let reached = blocks_to_check.unwrap_or(usize::MAX);
    for block in blocks.blocks().iter().take(reached) {
        for dp in Direction::ALL {
            for cc in [Chooser::Left, Chooser::Right] {
                let (image, stream) = exit_state(&grid, block.id(), dp, cc).unwrap();
                let (output, result) = run(&image, &stream, 40_000_000);
                assert_eq!(result.reason, ExitReason::Terminated);
                let found: Vec<i64> = output
                    .split_whitespace()
                    .map(|token| token.parse().unwrap())
                    .collect();

                let expected = match tessera::exit(&blocks, block.id(), dp, cc) {
                    tessera::Exit::Terminate => vec![-1],
                    tessera::Exit::Move {
                        target,
                        dp,
                        cc,
                        command,
                    } => vec![
                        i64::from(label(target)),
                        i64::from(dp as u32),
                        i64::from(cc as u32),
                        i64::from(record_code(command)),
                    ],
                };
                let what = format!("{rows:?} block {} {dp:?} {cc:?}", block.id().0);
                assert_eq!(found, expected, "{what}");
            }
        }
    }
}

#[test]
fn the_exit_matches_tessera() {
    check_exits(&["rrgg"], Some(1));
}

#[test]
fn the_exit_reports_pointer_and_switch() {
    // Red normal to cyan dark is hue +3, lightness +1: `pointer` (17).
    check_exits(&["rd"], Some(1));
    // Cyan light to red dark is hue +3, lightness +2: `switch` (18).
    check_exits(&["lk"], Some(1));
}

#[test]
fn the_exit_matches_tessera_through_white() {
    check_exits(&["r...b"], Some(1));
    check_exits(&["r.", "#."], Some(1));
}

#[test]
#[ignore = "the full sweep: cargo test -p loom --release --test grid -- --ignored"]
fn the_exit_matches_tessera_everywhere() {
    check_exits(&["rrgg"], None);
    check_exits(&["rg", "gr"], None);
    check_exits(&["r", "r"], None);
    check_exits(&["r.", "gg"], None);
    check_exits(&[".r.", "rrr", ".r."], None);
    check_exits(&["r...b"], None);
    check_exits(&["r.", "#."], None);
    check_exits(&["r..", "#.#", ".b."], None);
    check_exits(&["rd"], None);
    check_exits(&["lk"], None);
    check_exits(&["rgggr", "rg.gd", "rg.gg", "rrrrd"], None);
}

/// The start state of `rows`, against `tessera::start`.
fn check_starts(rows: &[&str]) {
    let grid = grid(rows);
    let blocks = Blocks::partition(grid.clone());
    let width = grid.width();
    let label = |id: BlockId| {
        blocks
            .block(id)
            .unwrap()
            .codels()
            .iter()
            .map(|point| (point.y + 1) * (width + 2) + (point.x + 1))
            .min()
            .unwrap()
    };

    let (image, stream) = start_state(&grid).unwrap();
    let (output, result) = run(&image, &stream, 40_000_000);
    assert_eq!(result.reason, ExitReason::Terminated);
    let found: Vec<i64> = output
        .split_whitespace()
        .map(|token| token.parse().unwrap())
        .collect();

    let expected = match tessera::start(&blocks) {
        tessera::Start::Terminate => vec![-1],
        tessera::Start::State(state) => vec![
            i64::from(label(state.block)),
            i64::from(state.dp as u32),
            i64::from(state.cc as u32),
        ],
    };
    assert_eq!(found, expected, "{rows:?}");
}

#[test]
fn the_start_matches_tessera() {
    check_starts(&["r"]);
    check_starts(&["rg", "gg"]);
    check_starts(&["#r"]);
}

#[test]
fn the_start_slides_through_a_white_corner() {
    check_starts(&["..b"]);
    check_starts(&[".#", "r."]);
    check_starts(&["r.", "#."]);
}

#[test]
#[ignore = "a real image: cargo test -p loom --release --test grid -- --ignored"]
fn the_canonical_hello_world_starts() {
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let grid = CodelGrid::from_image(png).unwrap();
    let blocks = Blocks::partition(grid.clone());
    let width = grid.width();
    let label = |id: BlockId| {
        blocks
            .block(id)
            .unwrap()
            .codels()
            .iter()
            .map(|point| (point.y + 1) * (width + 2) + (point.x + 1))
            .min()
            .unwrap()
    };

    let (image, stream) = start_state(&grid).unwrap();
    let (output, result) = run(&image, &stream, 400_000_000);
    assert_eq!(result.reason, ExitReason::Terminated);
    let found: Vec<i64> = output
        .split_whitespace()
        .map(|token| token.parse().unwrap())
        .collect();
    let expected = match tessera::start(&blocks) {
        tessera::Start::Terminate => vec![-1],
        tessera::Start::State(state) => vec![
            i64::from(label(state.block)),
            i64::from(state.dp as u32),
            i64::from(state.cc as u32),
        ],
    };
    assert_eq!(found, expected);
}

#[test]
fn the_edge_stage_is_the_same_image_for_every_grid() {
    let small = grid(&["r"]);
    let large = grid(&["rgggr", "rg.gd", "rg.gg", "rrrrd"]);
    let (a, _) = block_edge(&small, BlockId(0), Direction::Right, Chooser::Left).unwrap();
    let (b, _) = block_edge(&large, BlockId(0), Direction::Right, Chooser::Left).unwrap();
    assert_eq!(a, b);
}

#[test]
#[ignore = "a real image: cargo test -p loom --release --test grid -- --ignored"]
fn the_canonical_hello_world_partitions() {
    // 30x29 codels: 39 blocks, from colours alone.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let grid = CodelGrid::from_image(png).unwrap();
    let found = labels(&grid, 200_000_000_000);
    assert_eq!(found.len(), expected(&grid).len());
    assert_eq!(found, expected(&grid));
}
