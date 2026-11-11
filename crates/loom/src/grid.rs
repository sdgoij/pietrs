//! The splitter: the partition rules in the image, from colours alone.
//!
//! S4 of `.agents/notes/piet-in-piet-full.md` asks for an image that needs
//! nothing but the pixels. The first half is the partition: every codel belongs
//! to exactly one 4-connected same-colour component, and a program's blocks are
//! those components.
//!
//! This image takes the grid's colours and reports a **label** per ring cell —
//! the lowest ring position in that cell's component. It is the same answer
//! `canvas::Blocks` gives, with the ring position standing in for the block id,
//! and it never sees a block: only numbers.
//!
//! # The ring
//!
//! The grid is laid out row-major with a guard column on the left and a whole
//! guard row on top, so a cell's four neighbours are always ring cells:
//!
//! ```text
//!   index = row * (width + 1) + column     row 0 and column 0 are guards
//!   real codel (x, y) is index (y + 1) * (width + 1) + (x + 1)
//!   up   = index - (width + 1)             left = index - 1
//!   down = index + (width + 1)             right = index + 1
//! ```
//!
//! The guards are what make the neighbour arithmetic total: no edge test, no
//! special case. Each guard carries a colour code no codel and no other guard
//! has, so a guard can never merge, and wrapping from index 0 leftwards cannot
//! join two codels.
//!
//! A record is four cells, pushed `ring size, stride, colour, label`, so at the
//! head the label is the top cell:
//!
//! ```text
//!   [ label | colour | stride | ring cells ]     the head record
//! ```
//!
//! `stride` is one row of the ring in cells, `4 * (width + 1)`, so the neighbour
//! rolls are one data value and two constants. The ring size and stride travel
//! with the head for the same reason the walker's size does: every roll amount is
//! built from them, so one image serves every grid. Two live values sit above the
//! ring — the pass counter and a "changed" accumulator — and the helper functions
//! count them in their depths.
//!
//! # The rules
//!
//! A component's label is the lowest position in it, so the image computes it by
//! propagating minima: each pass visits every ring cell and, when a neighbour has
//! the same colour, replaces the label with the smaller of the two. All four
//! neighbours are compared — with only the left and up ones, a minimum could
//! never travel backwards, and the pass would stop at a label that is merely the
//! smallest reachable from the north-west. The pass is repeated until nothing
//! changes, which the `changed` accumulator records.
//!
//! # Input and output
//!
//! The stream is the number of ring cells, then every record highest index
//! first, so the last record read is ring cell zero and the head starts there.
//! The image prints one label per ring cell, in ring order, separated by spaces.

use canvas::{CodelGrid, Color, Point};
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};

/// Cells per ring record: label, colour, stride, ring size.
const PER_CELL: u32 = 4;

/// The colour code guard cells start at: above every real colour.
const GUARD_CODE: u32 = 22;

const ENTRY: u32 = 1;
const TEST: u32 = 2;
const READ: u32 = 3;
const HANDOVER: u32 = 4;
const PASS: u32 = 5;
const CELL: u32 = 6;
const PASS_END: u32 = 7;
const BODY: u32 = 8;
const DONE: u32 = 9;
const PRINT_TEST: u32 = 10;
const PRINT: u32 = 11;
const END: u32 = 12;

/// A splitter image and the stream that carries `grid`'s colours to it.
///
/// The image is the same for every grid. Running it prints one label per ring
/// cell; two cells share a label exactly when they are in the same block.
pub fn splitter(grid: &CodelGrid) -> Result<(CodelGrid, String), Error> {
    let width = grid.width();
    let row = width + 1;
    let cells = u32::try_from((row as usize) * ((grid.height() + 1) as usize))
        .map_err(|_| Error::TooLarge)?;
    let ring = cells.checked_mul(PER_CELL).ok_or(Error::TooLarge)?;

    let mut stream = cells.to_string();
    let stride = row * PER_CELL;
    // Highest index first: the last record read is cell zero, the head.
    for index in (0..cells).rev() {
        let colour = match codel_of(width, index) {
            Some(point) => {
                let Some(color) = grid.get(point) else {
                    return Err(Error::TooLarge);
                };
                code_of(color)
            }
            None => GUARD_CODE + index,
        };
        stream.push_str(&format!(" {ring} {stride} {colour} {index}"));
    }
    Ok((program()?.image()?, stream))
}

/// The codel a ring index stands for, or `None` when the index is a guard.
fn codel_of(width: u32, index: u32) -> Option<Point> {
    let row = index / (width + 1);
    let column = index % (width + 1);
    if row == 0 || column == 0 {
        return None;
    }
    Some(Point::new(column - 1, row - 1))
}

/// A colour's code: its position in the palette. Equality is all that matters.
fn code_of(color: Color) -> u32 {
    Color::ALL
        .iter()
        .position(|candidate| *candidate == color)
        .unwrap_or(0) as u32
}

/// The splitter image. It takes no arguments, so it is the same for every grid.
fn program() -> Result<Program, Error> {
    Program::new(blocks(), ENTRY)
}

/// The blocks of the splitter.
fn blocks() -> Vec<Block> {
    vec![
        // 1: the ring's cell count
        Block::new(vec![Node::Command(Command::InNumber)], End::Jump(TEST)),
        // 2: while there are cells left
        Block::new(
            vec![Node::Dup, Node::Command(Command::Not)],
            End::If {
                then: HANDOVER,
                otherwise: READ,
            },
        ),
        // 3: one record: ring size, width, colour, label
        Block::new(
            {
                let mut nodes = Vec::new();
                for _ in 0..PER_CELL {
                    nodes.extend([Node::Command(Command::InNumber), Node::Swap]);
                }
                nodes.extend([Node::Const(1), Node::Command(Command::Subtract)]);
                nodes
            },
            End::Jump(TEST),
        ),
        // 4: drop the read counter, then push the pass counter and the flag
        Block::new(
            vec![Node::Command(Command::Pop), Node::Const(0), Node::Const(1)],
            End::Jump(PASS),
        ),
        // 5: a pass: changed = 0, counter = the number of cells
        Block::new(pass_start(), End::Jump(CELL)),
        // 6: while the pass counter is not zero
        Block::new(
            vec![Node::DupLocal(2), Node::Command(Command::Not)],
            End::If {
                then: PASS_END,
                otherwise: BODY,
            },
        ),
        // 7: another pass only if something changed
        Block::new(
            vec![
                Node::DupLocal(1),
                Node::Const(0),
                Node::Command(Command::Greater),
            ],
            End::If {
                then: PASS,
                otherwise: DONE,
            },
        ),
        // 8: merge the head cell with its left and up neighbours, then advance
        Block::new(body(), End::Jump(CELL)),
        // 9: printing pass: reuse the counter
        Block::new(pass_start(), End::Jump(PRINT_TEST)),
        // 10: while there are cells to print
        Block::new(
            vec![Node::DupLocal(2), Node::Command(Command::Not)],
            End::If {
                then: END,
                otherwise: PRINT,
            },
        ),
        // 11: print the label, advance, count down
        Block::new(
            {
                let mut nodes = vec![
                    Node::DupLocal(3),
                    Node::Command(Command::OutNumber),
                    Node::Const(32),
                    Node::Command(Command::OutChar),
                ];
                nodes.extend(ring_by(2, i64::from(PER_CELL)));
                nodes.extend([
                    Node::Roll(2, 1),
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                    Node::Roll(2, 1),
                ]);
                nodes
            },
            End::Jump(PRINT_TEST),
        ),
        // 12: done
        Block::new(Vec::new(), End::Halt),
    ]
}

/// `changed = 0`, then the counter = the ring's cells.
fn pass_start() -> Vec<Node> {
    vec![
        Node::Command(Command::Pop),
        Node::Const(0),
        Node::Roll(2, 1),
        Node::Command(Command::Pop),
        Node::DupLocal(5),
        Node::Const(i64::from(PER_CELL)),
        Node::Command(Command::Divide),
        Node::Roll(2, 1),
    ]
}

/// One cell: count down, merge with the left and up neighbours, write the label
/// back, then move the head one cell forward.
fn body() -> Vec<Node> {
    let mut nodes = vec![
        // counter -= 1, keeping it under the flag
        Node::Roll(2, 1),
        Node::Const(1),
        Node::Command(Command::Subtract),
        Node::Roll(2, 1),
        // lift this cell's label and colour above the ring
        Node::DupLocal(3),
        Node::DupLocal(5),
        Node::Roll(2, 1),
    ];
    nodes.extend(merge(Neighbour::Left));
    nodes.extend(merge(Neighbour::Up));
    nodes.extend(merge(Neighbour::Right));
    nodes.extend(merge(Neighbour::Down));
    nodes.extend(write());
    // drop the label and colour copies, then step forward one cell
    nodes.extend([Node::Command(Command::Pop), Node::Command(Command::Pop)]);
    nodes.extend(ring_by(2, i64::from(PER_CELL)));
    nodes
}

/// Which neighbour a merge reaches.
enum Neighbour {
    Left,
    Up,
    Right,
    Down,
}

/// Merge the head's label with one neighbour's: if the colours match, take the
/// smaller label.
///
/// On entry the stack is `[label, colour, changed, counter, ring]`; on exit the
/// same, with `label` possibly lowered.
fn merge(neighbour: Neighbour) -> Vec<Node> {
    let mut nodes = match neighbour {
        Neighbour::Left => ring_by(4, -i64::from(PER_CELL)),
        Neighbour::Right => ring_by(4, i64::from(PER_CELL)),
        Neighbour::Up => ring_up(4),
        Neighbour::Down => ring_down(4),
    };
    // The neighbour's label, then its colour (one deeper after the first push).
    nodes.extend([Node::DupLocal(5), Node::DupLocal(7)]);
    nodes.extend(match neighbour {
        Neighbour::Left => ring_by(6, i64::from(PER_CELL)),
        Neighbour::Right => ring_by(6, -i64::from(PER_CELL)),
        Neighbour::Up => ring_down(6),
        Neighbour::Down => ring_up(6),
    });
    nodes.extend([
        // eq = the colours match
        Node::DupLocal(4),
        Node::Swap,
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
        // take = eq and the neighbour's label is smaller
        Node::DupLocal(3),
        Node::DupLocal(3),
        Node::Command(Command::Greater),
        Node::Command(Command::Multiply),
        // delta = (neighbour - label) * take
        Node::DupLocal(2),
        Node::DupLocal(4),
        Node::Command(Command::Subtract),
        Node::Command(Command::Multiply),
        // label += delta
        Node::Roll(3, 1),
        Node::Command(Command::Pop),
        Node::Command(Command::Add),
    ]);
    nodes
}

/// Replace the head's label with the merged one and fold the change into the
/// flag.
///
/// On entry the stack is `[label, colour, changed, counter, ring]`; on exit the
/// same, with the head's label cell rewritten.
fn write() -> Vec<Node> {
    vec![
        // d = the label is different from the cell's, keeping the label itself
        Node::DupLocal(1),
        Node::DupLocal(6),
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
        Node::Command(Command::Not),
        // changed = changed + d, rolled up and put back
        Node::Roll(4, 3),
        Node::Swap,
        Node::Command(Command::Add),
        Node::Roll(3, 1),
        // the new label goes into the head record; the old one comes out
        Node::DupLocal(1),
        Node::Roll(6, 1),
        Node::Roll(5, 4),
        Node::Command(Command::Pop),
    ]
}

/// `roll(ring size, rolls)`: spin the ring by a constant, past the live values.
///
/// `live` is how many values sit above the ring. A roll always acts on the top
/// of the stack, so a ring spin is three rotations: lift the ring above the
/// values, spin it, and put the values back. The amounts are data — the ring's
/// size from the head record — so one image serves every grid.
fn ring_by(live: u32, rolls: i64) -> Vec<Node> {
    let mut nodes = lift_ring(live);
    nodes.extend([
        Node::DupLocal(4),
        Node::Const(rolls),
        Node::Command(Command::Roll),
    ]);
    nodes.extend(drop_ring(live));
    nodes
}

/// One row up: spin the ring by `-width`.
fn ring_up(live: u32) -> Vec<Node> {
    let mut nodes = lift_ring(live);
    nodes.extend([Node::DupLocal(4), Node::DupLocal(4)]);
    nodes.extend(negate());
    nodes.push(Node::Command(Command::Roll));
    nodes.extend(drop_ring(live));
    nodes
}

/// One row down: spin the ring by `width`.
fn ring_down(live: u32) -> Vec<Node> {
    let mut nodes = lift_ring(live);
    nodes.extend([
        Node::DupLocal(4),
        Node::DupLocal(4),
        Node::Command(Command::Roll),
    ]);
    nodes.extend(drop_ring(live));
    nodes
}

/// Pull the ring above the `live` values.
fn lift_ring(live: u32) -> Vec<Node> {
    vec![
        Node::DupLocal(live + 4),
        Node::Const(i64::from(live)),
        Node::Command(Command::Add),
        Node::Const(i64::from(live)),
        Node::Command(Command::Roll),
    ]
}

/// Put the `live` values back above the ring.
fn drop_ring(live: u32) -> Vec<Node> {
    vec![
        Node::DupLocal(4),
        Node::Const(i64::from(live)),
        Node::Command(Command::Add),
        Node::DupLocal(5),
        Node::Command(Command::Roll),
    ]
}

/// `0 - x`.
fn negate() -> Vec<Node> {
    vec![Node::Const(0), Node::Swap, Node::Command(Command::Subtract)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use interp::Options;
    use runtime::BufferIo;

    fn run(program: &Program, input: &str, steps: u64) -> (String, String) {
        let image = program.image().unwrap();
        let blocks = canvas::Blocks::partition(image);
        let mut io = BufferIo::new(input);
        let options = Options {
            max_steps: Some(steps),
            ..Options::default()
        };
        let result = interp::run(&blocks, &mut io, &options).unwrap();
        (io.into_output(), format!("{:?}", result.reason))
    }

    /// One pass of the real program: block 7 is rewritten to stop after it.
    fn one_pass(input: &str) -> (String, String) {
        let mut blocks = blocks();
        blocks[(PASS_END - 1) as usize] = Block::new(Vec::new(), End::Jump(DONE));
        let program = Program::new(blocks, ENTRY).unwrap();
        run(&program, input, 20_000_000)
    }

    #[test]
    fn one_pass_leaves_unrelated_neighbours_alone() {
        // One codel, guarded on every side: nothing to merge with.
        let (output, reason) = one_pass("4 16 8 7 3 16 8 24 2 16 8 23 1 16 8 22 0");
        assert_eq!(reason, "Terminated");
        assert_eq!(output, "0 1 2 3 ");
    }

    #[test]
    fn one_pass_merges_a_pair() {
        // Two reds side by side: the second takes the first's label.
        let (output, reason) =
            one_pass("6 24 12 7 5 24 12 7 4 24 12 25 3 24 12 24 2 24 12 23 1 24 12 22 0");
        assert_eq!(reason, "Terminated");
        assert_eq!(output, "0 1 2 3 4 4 ");
    }

    /// A program that loads the ring and prints `colour label` per cell, with no
    /// merging at all: the loader, the record layout and the advance in isolation.
    #[test]
    fn the_loader_and_advance_read_the_ring_back() {
        let mut blocks = vec![
            Block::new(vec![Node::Command(Command::InNumber)], End::Jump(TEST)),
            Block::new(
                vec![Node::Dup, Node::Command(Command::Not)],
                End::If {
                    then: HANDOVER,
                    otherwise: READ,
                },
            ),
            Block::new(
                {
                    let mut nodes = Vec::new();
                    for _ in 0..PER_CELL {
                        nodes.extend([Node::Command(Command::InNumber), Node::Swap]);
                    }
                    nodes.extend([Node::Const(1), Node::Command(Command::Subtract)]);
                    nodes
                },
                End::Jump(TEST),
            ),
            Block::new(
                vec![Node::Command(Command::Pop), Node::Const(0), Node::Const(0)],
                End::Jump(PASS),
            ),
            Block::new(pass_start(), End::Jump(PRINT_TEST)),
        ];
        // Blocks 6 to 9 are unused, so the print blocks keep their ids.
        while blocks.len() < (PRINT_TEST - 1) as usize {
            blocks.push(Block::new(Vec::new(), End::Halt));
        }
        blocks.push(Block::new(
            vec![Node::DupLocal(2), Node::Command(Command::Not)],
            End::If {
                then: END,
                otherwise: PRINT,
            },
        ));
        blocks.push(Block::new(
            {
                let mut nodes = vec![
                    Node::DupLocal(3),
                    Node::DupLocal(5),
                    Node::Command(Command::OutNumber),
                    Node::Command(Command::OutNumber),
                    Node::Const(32),
                    Node::Command(Command::OutChar),
                ];
                nodes.extend(ring_by(2, i64::from(PER_CELL)));
                nodes.extend([
                    Node::Roll(2, 1),
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                    Node::Roll(2, 1),
                ]);
                nodes
            },
            End::Jump(PRINT_TEST),
        ));
        blocks.push(Block::new(Vec::new(), End::Halt));
        let program = Program::new(blocks, ENTRY).unwrap();

        // Two cells, highest index first: ring 8, width 1, colour, label.
        let input = "2 8 1 9 1 8 1 7 0";
        let (output, reason) = run(&program, input, 10_000_000);
        assert_eq!(reason, "Terminated");
        // One space per cell, after both numbers: labels survive the round trip.
        assert_eq!(output, "70 91 ");
    }
}
