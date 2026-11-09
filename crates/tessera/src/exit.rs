//! Movement, the eight-attempt fallback, white sliding, and the start state.

use std::collections::HashSet;

use canvas::{Block, BlockId, Blocks, Color, Point};

use crate::{Chooser, Command, Direction, command};

/// A resting state: a chromatic block, the direction pointer, and the codel chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct State {
    pub block: BlockId,
    pub dp: Direction,
    pub cc: Chooser,
}

/// What the interpreter does from a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Move {
        target: BlockId,
        dp: Direction,
        cc: Chooser,
        command: Option<Command>,
    },
    Terminate,
}

/// The state a program starts in, or that it cannot start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    State(State),
    Terminate,
}

/// Resolves the upper-left codel into the first state.
pub fn start(blocks: &Blocks) -> Start {
    let point = Point::new(0, 0);
    let Some(block_id) = blocks.block_id_at(point) else {
        return Start::Terminate;
    };
    let Some(block) = blocks.block(block_id) else {
        return Start::Terminate;
    };
    match block.color() {
        Color::Chromatic { .. } => Start::State(State {
            block: block_id,
            dp: Direction::Right,
            cc: Chooser::Left,
        }),
        Color::White => match slide(blocks, point, Direction::Right, Chooser::Left) {
            Slide::Move { target, dp, cc } => Start::State(State {
                block: target,
                dp,
                cc,
            }),
            Slide::Terminate => Start::Terminate,
        },
        Color::Black => Start::Terminate,
    }
}

/// Resolves one step of the movement rules.
pub fn exit(blocks: &Blocks, block_id: BlockId, dp: Direction, cc: Chooser) -> Exit {
    let Some(block) = blocks.block(block_id) else {
        return Exit::Terminate;
    };
    if !block.color().is_chromatic() {
        return Exit::Terminate;
    }

    for attempt in 0..8u32 {
        let candidate_dp = dp.clockwise_by(attempt / 2);
        let candidate_cc = if attempt.div_ceil(2) & 1 == 1 {
            cc.toggled()
        } else {
            cc
        };
        let Some(edge) = edge_codel(block, candidate_dp, candidate_cc) else {
            continue;
        };
        let Some(next) = step(edge, candidate_dp) else {
            continue;
        };
        match blocks.grid().get(next) {
            None | Some(Color::Black) => continue,
            Some(Color::White) => {
                return match slide(blocks, next, candidate_dp, candidate_cc) {
                    Slide::Move { target, dp, cc } => Exit::Move {
                        target,
                        dp,
                        cc,
                        command: None,
                    },
                    Slide::Terminate => Exit::Terminate,
                };
            }
            Some(to) => {
                let Some(target) = blocks.block_id_at(next) else {
                    continue;
                };
                return Exit::Move {
                    target,
                    dp: candidate_dp,
                    cc: candidate_cc,
                    command: command(block.color(), to),
                };
            }
        }
    }

    Exit::Terminate
}

enum Slide {
    Move {
        target: BlockId,
        dp: Direction,
        cc: Chooser,
    },
    Terminate,
}

fn slide(blocks: &Blocks, start: Point, dp: Direction, cc: Chooser) -> Slide {
    let mut codel = start;
    let mut dp = dp;
    let mut cc = cc;
    let mut visited = HashSet::new();

    loop {
        if !visited.insert((codel, dp)) {
            return Slide::Terminate;
        }
        let mut current = codel;
        while let Some(next) = step(current, dp) {
            match blocks.grid().get(next) {
                None | Some(Color::Black) => break,
                Some(Color::White) => current = next,
                Some(_) => {
                    let Some(target) = blocks.block_id_at(next) else {
                        break;
                    };
                    return Slide::Move { target, dp, cc };
                }
            }
        }
        cc = cc.toggled();
        dp = dp.clockwise();
        codel = current;
    }
}

fn edge_codel(block: &Block, dp: Direction, cc: Chooser) -> Option<Point> {
    let codels = block.codels();
    let first = codels.first()?;
    let mut edge = primary(*first, dp);
    for point in codels.iter().skip(1) {
        let value = primary(*point, dp);
        edge = match dp {
            Direction::Right | Direction::Down => edge.max(value),
            Direction::Left | Direction::Up => edge.min(value),
        };
    }

    let choose_max = choose_max(dp, cc);
    let mut chosen: Option<Point> = None;
    for point in codels {
        if primary(*point, dp) != edge {
            continue;
        }
        let value = secondary(*point, dp);
        chosen = match chosen {
            None => Some(*point),
            Some(best) => {
                let best_value = secondary(best, dp);
                let better = if choose_max {
                    value > best_value
                } else {
                    value < best_value
                };
                if better { Some(*point) } else { Some(best) }
            }
        };
    }
    chosen
}

fn primary(point: Point, dp: Direction) -> u32 {
    match dp {
        Direction::Right | Direction::Left => point.x,
        Direction::Down | Direction::Up => point.y,
    }
}

fn secondary(point: Point, dp: Direction) -> u32 {
    match dp {
        Direction::Right | Direction::Left => point.y,
        Direction::Down | Direction::Up => point.x,
    }
}

fn choose_max(dp: Direction, cc: Chooser) -> bool {
    matches!(
        (dp, cc),
        (Direction::Right, Chooser::Right)
            | (Direction::Left, Chooser::Left)
            | (Direction::Down, Chooser::Left)
            | (Direction::Up, Chooser::Right)
    )
}

fn step(point: Point, dp: Direction) -> Option<Point> {
    let (dx, dy) = dp.delta();
    let x = point.x.checked_add_signed(dx)?;
    let y = point.y.checked_add_signed(dy)?;
    Some(Point::new(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Hue, Lightness};
    use proptest::prelude::*;

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
                    'r' | 'x' => Color::chromatic(Hue::Red, Lightness::Normal),
                    'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                    'b' => Color::chromatic(Hue::Blue, Lightness::Normal),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        CodelGrid::from_colors(width, height, codels).unwrap()
    }

    fn blocks(rows: &[&str]) -> Blocks {
        Blocks::partition(grid(rows))
    }

    fn id_at(blocks: &Blocks, x: u32, y: u32) -> BlockId {
        blocks.block_id_at(Point::new(x, y)).unwrap()
    }

    #[test]
    fn edge_codel_selects_the_cc_extreme_of_the_dp_edge() {
        let blocks = blocks(&["xx.", "xxx", "..."]);
        let block = blocks.block(id_at(&blocks, 0, 0)).unwrap();
        let cases = [
            (Direction::Right, Chooser::Left, Point::new(2, 1)),
            (Direction::Right, Chooser::Right, Point::new(2, 1)),
            (Direction::Down, Chooser::Left, Point::new(2, 1)),
            (Direction::Down, Chooser::Right, Point::new(0, 1)),
            (Direction::Left, Chooser::Left, Point::new(0, 1)),
            (Direction::Left, Chooser::Right, Point::new(0, 0)),
            (Direction::Up, Chooser::Left, Point::new(0, 0)),
            (Direction::Up, Chooser::Right, Point::new(1, 0)),
        ];
        for (dp, cc, expected) in cases {
            assert_eq!(
                edge_codel(block, dp, cc),
                Some(expected),
                "dp={dp:?} cc={cc:?}"
            );
        }
    }

    #[test]
    fn a_direct_transition_runs_its_command() {
        let blocks = blocks(&["rrgg"]);
        let red = id_at(&blocks, 0, 0);
        let green = id_at(&blocks, 3, 0);
        assert_eq!(
            exit(&blocks, red, Direction::Right, Chooser::Left),
            Exit::Move {
                target: green,
                dp: Direction::Right,
                cc: Chooser::Left,
                command: Some(Command::Divide),
            }
        );
    }

    #[test]
    fn the_fallback_rotates_until_it_finds_an_exit() {
        let blocks = blocks(&["rrgg"]);
        let red = id_at(&blocks, 0, 0);
        let green = id_at(&blocks, 3, 0);
        assert_eq!(
            exit(&blocks, green, Direction::Right, Chooser::Left),
            Exit::Move {
                target: red,
                dp: Direction::Left,
                cc: Chooser::Left,
                command: Some(Command::Duplicate),
            }
        );
    }

    #[test]
    fn a_block_with_no_neighbour_terminates() {
        let blocks = blocks(&["rr"]);
        let red = id_at(&blocks, 0, 0);
        assert_eq!(
            exit(&blocks, red, Direction::Right, Chooser::Left),
            Exit::Terminate
        );
    }

    #[test]
    fn a_white_slide_enters_the_first_colour_and_runs_no_command() {
        let blocks = blocks(&["r...b"]);
        let red = id_at(&blocks, 0, 0);
        let blue = id_at(&blocks, 4, 0);
        assert_eq!(
            exit(&blocks, red, Direction::Right, Chooser::Left),
            Exit::Move {
                target: blue,
                dp: Direction::Right,
                cc: Chooser::Left,
                command: None,
            }
        );
    }

    #[test]
    fn a_white_dead_end_terminates_on_retrace() {
        let blocks = blocks(&["r.", "#."]);
        let red = id_at(&blocks, 0, 0);
        assert_eq!(
            exit(&blocks, red, Direction::Right, Chooser::Left),
            Exit::Terminate
        );
    }

    #[test]
    fn start_uses_the_upper_left_codel() {
        let blocks = blocks(&["r."]);
        let red = id_at(&blocks, 0, 0);
        assert_eq!(
            start(&blocks),
            Start::State(State {
                block: red,
                dp: Direction::Right,
                cc: Chooser::Left,
            })
        );
    }

    #[test]
    fn start_slides_through_a_white_upper_left_codel() {
        let blocks = blocks(&["..b"]);
        let blue = id_at(&blocks, 2, 0);
        assert_eq!(
            start(&blocks),
            Start::State(State {
                block: blue,
                dp: Direction::Right,
                cc: Chooser::Left,
            })
        );
    }

    #[test]
    fn start_terminates_on_black_or_a_white_dead_end() {
        assert_eq!(start(&blocks(&["#r"])), Start::Terminate);
        assert_eq!(start(&blocks(&[".#"])), Start::Terminate);
    }

    fn color_from_code(code: u8) -> Color {
        match code % 5 {
            0 => Color::White,
            1 => Color::Black,
            2 => Color::chromatic(Hue::Red, Lightness::Normal),
            3 => Color::chromatic(Hue::Green, Lightness::Normal),
            _ => Color::chromatic(Hue::Blue, Lightness::Normal),
        }
    }

    proptest! {
        #[test]
        fn exit_is_deterministic_and_targets_an_existing_block(
            width in 1u32..6,
            height in 1u32..6,
            codes in proptest::collection::vec(any::<u8>(), 0..36),
            dp_index in 0usize..4,
            cc_index in 0usize..2,
        ) {
            let count = (width as usize) * (height as usize);
            let codels: Vec<Color> = (0..count)
                .map(|index| color_from_code(codes.get(index).copied().unwrap_or(0)))
                .collect();
            let grid = CodelGrid::from_colors(width, height, codels).unwrap();
            let blocks = Blocks::partition(grid);
            let block = blocks
                .blocks()
                .get(usize::from(codes.first().copied().unwrap_or(0)) % blocks.len())
                .map_or(BlockId(0), |block| block.id());
            let dp = Direction::ALL[dp_index];
            let cc = if cc_index == 0 { Chooser::Left } else { Chooser::Right };

            let first = exit(&blocks, block, dp, cc);
            let second = exit(&blocks, block, dp, cc);
            prop_assert_eq!(first, second);
            if let Exit::Move { target, .. } = first {
                prop_assert!(blocks.block(target).is_some());
            }
        }
    }
}
