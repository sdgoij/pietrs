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
//! The grid is laid out row-major with a border of guards on every side, so a
//! cell's four neighbours are always ring cells and a step off the grid lands on
//! a guard rather than on another row:
//!
//! ```text
//!   index = row * (width + 2) + column     the border is guards
//!   real codel (x, y) is index (y + 1) * (width + 2) + (x + 1)
//!   up   = index - stride                  left  = index - 4
//!   down = index + stride                  right = index + 4
//! ```
//!
//! The guards are what make the neighbour arithmetic total: no edge test, no
//! special case, and a step that leaves the grid is a guard, which the movement
//! rules treat like black. Each guard carries a colour code no codel and no other
//! guard has, so a guard can never merge, and wrapping round the ring from index
//! zero cannot join two codels.
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
//! # The edge stage
//!
//! [`block_edge`] extends the image with the movement rule that picks a codel:
//! given a block and a state, the block's own edge codel for that direction and
//! chooser — the single attempt, before the eight-attempt fallback.
//!
//! The scan is one pass over the ring. Each cell of the block is scored with a
//! single number that packs the primary above the secondary,
//! `primary * big + secondary`, with each half flipped when the state wants a
//! minimum instead of a maximum. That makes the whole rule one `greater` per
//! cell, and the eight `(dp, cc)` cases differ only in which halves are flipped
//! and whether the row or the column is primary. `big` is the ring's cell count,
//! so the two halves cannot carry into each other, and decoding the winner is a
//! division and a remainder.
//!
//! # Input and output
//!
//! The stream is the number of ring cells, then every record highest index
//! first, so the last record read is ring cell zero and the head starts there.
//! The image prints one label per ring cell, in ring order, separated by spaces.

use canvas::{BlockId, Blocks, CodelGrid, Color, Point};
use tessera::{Chooser, Command, Direction};

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

/// Where the edge stage starts, once the partition has finished.
const EDGE_SETUP: u32 = 13;
/// Does this cell belong to the block? Then score it.
const EDGE_CELL: u32 = 14;
/// `index += 1`, and stop when every cell has been seen.
const EDGE_NEXT: u32 = 15;
/// The next cell.
const EDGE_ADVANCE: u32 = 16;
/// The codel's coordinates, then the case for this `(dp, cc)`.
const EDGE_MATCH: u32 = 17;
/// Eight scoring blocks, one per `(dp, cc)`.
const EDGE_CASE: u32 = 18;
/// Eight print blocks, one per `(dp, cc)`.
const EDGE_PRINT: u32 = 26;
/// Picks the print block for this case.
const EDGE_SELECT: u32 = 34;

/// A splitter image and the stream that carries `grid`'s colours to it.
///
/// The image is the same for every grid. Running it prints one label per ring
/// cell; two cells share a label exactly when they are in the same block.
pub fn splitter(grid: &CodelGrid) -> Result<(CodelGrid, String), Error> {
    Ok((program()?.image()?, stream_of(grid)?))
}

/// A splitter image that also resolves a state's own edge codel, and the stream
/// that asks for it.
///
/// `block` must exist in `grid`. The stream is the grid's colours, then the
/// block's label and the state; the image prints the edge codel as `x y` after
/// it has partitioned the grid itself. This is the single attempt — the codel
/// the state's direction and chooser select — not [`tessera::edge_codel`], which
/// keeps trying until a move leaves the block.
pub fn block_edge(
    grid: &CodelGrid,
    block: BlockId,
    dp: Direction,
    cc: Chooser,
) -> Result<(CodelGrid, String), Error> {
    let blocks = Blocks::partition(grid.clone());
    let label = blocks
        .block(block)
        .and_then(|target| {
            target
                .codels()
                .iter()
                .map(|point| ring_index(grid, *point))
                .min()
        })
        .ok_or(Error::TooLarge)?;

    let mut stream = stream_of(grid)?;
    stream.push_str(&format!(" {label} {} {}", dp as u32, cc as u32));
    Ok((edge_program()?.image()?, stream))
}

/// The ring position of a codel.
fn ring_index(grid: &CodelGrid, point: Point) -> u32 {
    (point.y + 1) * (grid.width() + 2) + (point.x + 1)
}

/// The grid half of the stream.
fn stream_of(grid: &CodelGrid) -> Result<String, Error> {
    let width = grid.width();
    let height = grid.height();
    let row = width + 2;
    let cells =
        u32::try_from((row as usize) * ((height + 2) as usize)).map_err(|_| Error::TooLarge)?;
    let ring = cells.checked_mul(PER_CELL).ok_or(Error::TooLarge)?;

    let mut stream = cells.to_string();
    let stride = row * PER_CELL;
    // Highest index first: the last record read is cell zero, the head.
    for index in (0..cells).rev() {
        let colour = match codel_of(width, height, index) {
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
    Ok(stream)
}

/// The codel a ring index stands for, or `None` when the index is a guard.
fn codel_of(width: u32, height: u32, index: u32) -> Option<Point> {
    let row = index / (width + 2);
    let column = index % (width + 2);
    if row == 0 || row > height || column == 0 || column > width {
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

/// The splitter image, extended with the edge stage.
fn edge_program() -> Result<Program, Error> {
    let mut all = blocks();
    // The partition stops once it has converged: no labels, straight to the edge.
    all[(DONE - 1) as usize] = Block::new(Vec::new(), End::Jump(EDGE_SETUP));
    all.extend(edge_blocks());
    Program::new(all, ENTRY)
}

/// The edge stage: read the query, score every cell of the block, print the
/// winner.
///
/// The live values are `[index, best, case, big, row cells, label]`: the ring
/// index being visited, the best score so far, the `(dp, cc)` case, the score
/// base (`big`, the cell count), the row length in cells, and the block's label.
fn edge_blocks() -> Vec<Block> {
    let mut blocks = vec![
        // 13: read label, dp, cc; find the row length and the score base
        Block::new(edge_setup(), End::Jump(EDGE_CELL)),
        // 14: does this cell belong to the block?
        Block::new(
            vec![
                Node::DupLocal(7),
                Node::DupLocal(7),
                Node::Command(Command::Subtract),
                Node::Command(Command::Not),
            ],
            End::If {
                then: EDGE_MATCH,
                otherwise: EDGE_NEXT,
            },
        ),
        // 15: one more cell seen; all of them means the score is final
        Block::new(
            vec![
                Node::Const(1),
                Node::Command(Command::Add),
                Node::DupLocal(1),
                Node::DupLocal(5),
                Node::Command(Command::Subtract),
                Node::Command(Command::Not),
            ],
            End::If {
                then: EDGE_SELECT,
                otherwise: EDGE_ADVANCE,
            },
        ),
        // 16: the next cell
        Block::new(ring_by(6, i64::from(PER_CELL)), End::Jump(EDGE_CELL)),
        // 17: the codel's coordinates, then the scoring block for this case
        Block::new(
            {
                let mut nodes = cell_coordinates();
                nodes.extend([
                    Node::DupLocal(5),
                    Node::Const(i64::from(EDGE_CASE)),
                    Node::Command(Command::Add),
                ]);
                nodes
            },
            End::Computed,
        ),
    ];
    for case in 0..8 {
        blocks.push(Block::new(score_case(case), End::Jump(EDGE_NEXT)));
    }
    for case in 0..8 {
        blocks.push(Block::new(print_case(case), End::Halt));
    }
    // 34: the print block for this case
    blocks.push(Block::new(
        vec![
            Node::DupLocal(3),
            Node::Const(i64::from(EDGE_PRINT)),
            Node::Command(Command::Add),
        ],
        End::Computed,
    ));
    blocks
}

/// `[index, best, case, big, row cells, label]` from the query.
///
/// `big` is the cell count, so it exceeds every coordinate and the score's
/// primary and secondary fields cannot overflow into each other.
fn edge_setup() -> Vec<Node> {
    vec![
        // the partition's changed flag and counter are done with
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Command(Command::InNumber),
        // row cells = stride / 4
        Node::DupLocal(4),
        Node::Const(4),
        Node::Command(Command::Divide),
        // big = ring cells / 4
        Node::DupLocal(6),
        Node::Const(4),
        Node::Command(Command::Divide),
        Node::Command(Command::InNumber),
        Node::Command(Command::InNumber),
        // case = dp * 2 + cc
        Node::DupLocal(2),
        Node::Const(2),
        Node::Command(Command::Multiply),
        Node::Roll(2, 1),
        Node::Command(Command::Add),
        // dp has been folded into the case
        Node::Roll(2, 1),
        Node::Command(Command::Pop),
        // best = -1: every real score beats it; index = 0
        Node::Const(-1),
        Node::Const(0),
    ]
}

/// The head's `(column, row)` in ring coordinates, from its index.
///
/// The index arrives as `[index, best, case, big, row cells, label]`, and is
/// still there afterwards: `[column, row, index, best, case, big, row cells,
/// label]`.
fn cell_coordinates() -> Vec<Node> {
    vec![
        // row = index / row cells, from a copy
        Node::DupLocal(1),
        Node::DupLocal(6),
        Node::Command(Command::Divide),
        // column = index % row cells, from another
        Node::DupLocal(2),
        Node::DupLocal(7),
        Node::Command(Command::Modulo),
    ]
}

/// Which coordinate is primary, and whether the primary and secondary are
/// maximised, for `case = dp * 2 + cc`.
///
/// The directions are `right, down, left, up` and the choosers `left, right`,
/// which is the order `tessera` uses. The secondary rule is
/// `choose_max(dp, cc)`.
fn flags(case: u32) -> (bool, bool, bool) {
    let dp = case / 2;
    let cc = case % 2;
    let primary_is_x = dp.is_multiple_of(2);
    let primary_max = dp < 2;
    let secondary_max = matches!((dp, cc), (0, 1) | (2, 0) | (1, 0) | (3, 1));
    (primary_is_x, primary_max, secondary_max)
}

/// Score the head cell and keep it if it beats the best.
///
/// The score packs the primary above the secondary: `primary * big + secondary`,
/// with the halves transformed so a larger score is the answer the state wants.
/// That is the whole comparison — one `greater` per cell instead of a branch per
/// rule — and the eight `(dp, cc)` cases differ only in which half is flipped.
fn score_case(case: u32) -> Vec<Node> {
    let (primary_is_x, primary_max, secondary_max) = flags(case);
    let primary = if primary_is_x { 1 } else { 2 };
    let secondary = if primary_is_x { 2 } else { 1 };
    let mut nodes = Vec::new();
    // the primary half
    if primary_max {
        nodes.push(Node::DupLocal(primary));
    } else {
        nodes.extend([
            Node::DupLocal(7),
            Node::DupLocal(primary + 1),
            Node::Command(Command::Subtract),
        ]);
    }
    nodes.extend([Node::DupLocal(7), Node::Command(Command::Multiply)]);
    // the secondary half
    if secondary_max {
        nodes.push(Node::DupLocal(secondary + 1));
    } else {
        nodes.extend([
            Node::DupLocal(8),
            Node::DupLocal(secondary + 2),
            Node::Command(Command::Subtract),
        ]);
    }
    nodes.push(Node::Command(Command::Add));
    // best = max(best, score)
    nodes.extend([
        Node::DupLocal(1),
        Node::DupLocal(6),
        Node::Command(Command::Greater),
        Node::DupLocal(2),
        Node::DupLocal(7),
        Node::Command(Command::Subtract),
        Node::Roll(2, 1),
        Node::Command(Command::Multiply),
        Node::Roll(6, 5),
        Node::Command(Command::Add),
        Node::Roll(5, 1),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]);
    nodes
}

/// Print the edge codel's `x` and `y` from the winning score.
fn print_case(case: u32) -> Vec<Node> {
    let (primary_is_x, primary_max, secondary_max) = flags(case);
    let mut nodes = vec![
        // primary = best / big
        Node::DupLocal(2),
        Node::DupLocal(5),
        Node::Command(Command::Divide),
        // secondary = best % big
        Node::DupLocal(3),
        Node::DupLocal(6),
        Node::Command(Command::Modulo),
    ];
    // [secondary, primary, index, best, case, big, row cells, label, ring]
    if primary_max {
        nodes.push(Node::DupLocal(2));
    } else {
        nodes.extend([
            Node::DupLocal(7),
            Node::DupLocal(3),
            Node::Command(Command::Subtract),
        ]);
    }
    if secondary_max {
        nodes.push(Node::DupLocal(2));
    } else {
        nodes.extend([
            Node::DupLocal(8),
            Node::DupLocal(3),
            Node::Command(Command::Subtract),
        ]);
    }
    // [secondary value, primary value, secondary, primary, index, best, ...]
    let (column, row) = if primary_is_x { (2, 1) } else { (1, 2) };
    for depth in [column, row] {
        nodes.extend([
            Node::DupLocal(depth),
            Node::Const(1),
            Node::Command(Command::Subtract),
            Node::Command(Command::OutNumber),
            Node::Const(32),
            Node::Command(Command::OutChar),
        ]);
    }
    nodes
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

    /// A stack machine over `Node`, for checking a sequence's discipline without
    /// compiling an image. The semantics are `runtime::Stack`'s.
    struct Machine {
        stack: Vec<i64>,
        out: String,
    }

    impl Machine {
        fn new(stack: Vec<i64>) -> Self {
            Self {
                stack,
                out: String::new(),
            }
        }

        fn roll(&mut self, depth: u32, rolls: i64) {
            let depth = depth as usize;
            assert!(depth <= self.stack.len(), "roll depth {depth} too deep");
            let start = self.stack.len() - depth;
            let shift = rolls.rem_euclid(depth as i64) as usize;
            self.stack[start..].rotate_right(shift);
        }

        fn pop(&mut self) -> i64 {
            self.stack.pop().expect("pop from empty")
        }

        fn run(&mut self, nodes: &[Node]) {
            for node in nodes {
                match *node {
                    Node::Const(value) => self.stack.push(value),
                    Node::Dup => {
                        let value = *self.stack.last().expect("dup of empty");
                        self.stack.push(value);
                    }
                    Node::Swap => {
                        let n = self.stack.len();
                        self.stack.swap(n - 1, n - 2);
                    }
                    Node::Roll(depth, rolls) => self.roll(depth, i64::from(rolls)),
                    Node::DupLocal(depth) => {
                        self.roll(depth, i64::from(depth) - 1);
                        let value = *self.stack.last().expect("dup_local of empty");
                        self.stack.push(value);
                        self.roll(depth + 1, 1);
                    }
                    Node::Command(command) => self.command(command),
                }
            }
        }

        fn command(&mut self, command: Command) {
            match command {
                Command::Add => {
                    let top = self.pop();
                    let second = self.pop();
                    self.stack.push(second + top);
                }
                Command::Subtract => {
                    let top = self.pop();
                    let second = self.pop();
                    self.stack.push(second - top);
                }
                Command::Multiply => {
                    let top = self.pop();
                    let second = self.pop();
                    self.stack.push(second * top);
                }
                Command::Divide => {
                    let top = self.pop();
                    let second = self.pop();
                    self.stack.push(second.div_euclid(top));
                }
                Command::Modulo => {
                    let top = self.pop();
                    let second = self.pop();
                    self.stack.push(second.rem_euclid(top));
                }
                Command::Greater => {
                    let top = self.pop();
                    let second = self.pop();
                    self.stack.push(i64::from(second > top));
                }
                Command::Not => {
                    let top = self.pop();
                    self.stack.push(i64::from(top == 0));
                }
                Command::Roll => {
                    let rolls = self.pop();
                    let depth = self.pop();
                    let depth = u32::try_from(depth).expect("negative roll depth");
                    self.roll(depth, rolls);
                }
                Command::Duplicate => {
                    let value = *self.stack.last().expect("duplicate of empty");
                    self.stack.push(value);
                }
                Command::Pop => {
                    self.pop();
                }
                Command::OutNumber => {
                    let value = self.pop();
                    self.out.push_str(&value.to_string());
                }
                Command::OutChar => {
                    let value = self.pop();
                    self.out
                        .push(char::from(u8::try_from(value).expect("bad char")));
                }
                other => panic!("unsupported command {other:?}"),
            }
        }
    }

    /// The live values the edge stage expects, bottom-up, under a fake ring.
    fn edge_stack(label: i64, big: i64, row_cells: i64, case: i64, best: i64) -> Vec<i64> {
        let ring = (0..big * 4).map(|_| 0).collect::<Vec<_>>();
        let mut stack = ring;
        stack.extend([label, row_cells, big, case, best, 0]);
        stack
    }

    /// A block-level VM over the generated program, with the flow backend's
    /// conventions: every block pops the dispatcher's id first, `jump` and `if`
    /// leave one on the stack for that pop.
    fn execute(blocks: &[Block], input: &[i64], limit: usize) -> String {
        // The image's prologue pushes the entry id, which block one pops.
        let mut machine = Machine::new(vec![i64::from(ENTRY)]);
        let mut input = input.to_vec();
        let mut pc = 1u32;
        for _ in 0..limit {
            if pc == 0 {
                return machine.out;
            }
            let block = &blocks[(pc - 1) as usize];
            machine.command(Command::Pop);
            for node in &block.nodes {
                if let Node::Command(Command::InNumber) = node {
                    machine.stack.push(input.remove(0));
                } else {
                    machine.run(std::slice::from_ref(node));
                }
            }
            match block.end {
                End::Jump(id) => {
                    machine.stack.push(i64::from(id));
                    pc = id;
                }
                End::Computed => {
                    // The id is already on the stack; the target block's own
                    // prologue pop consumes it, exactly as the dispatcher does.
                    pc = u32::try_from(*machine.stack.last().expect("no id")).expect("bad id");
                }
                End::If { then, otherwise } => {
                    let condition = machine.stack.pop().expect("no condition");
                    let id = if condition != 0 { then } else { otherwise };
                    machine.stack.push(i64::from(id));
                    pc = id;
                }
                End::Halt => pc = 0,
            }
        }
        panic!("step limit in the vm\n{}", machine.out);
    }

    #[test]
    fn a_score_keeps_the_live_values() {
        // Case 0 is dp=right, cc=left: max x, then min y.
        let mut machine = Machine::new(edge_stack(8, 10, 5, 0, -1));
        machine.stack.extend([1, 2]); // row, then column on top
        machine.run(&score_case(0));
        assert_eq!(machine.stack.len(), 46, "live count drifted");
        assert_eq!(machine.stack[45], 0, "index is on top");
        // score = 2 * 10 + (5 - 1) = 24
        assert_eq!(machine.stack[44], 24, "best");
    }

    #[test]
    fn a_score_decodes_back_to_its_codel() {
        let mut machine = Machine::new(edge_stack(8, 10, 5, 0, 24));
        machine.run(&print_case(0));
        assert_eq!(machine.out, "1 0 ");
    }

    /// The edge program's blocks, as `edge_program` assembles them.
    fn edge_blocks_for_test() -> Vec<Block> {
        let mut all = blocks();
        all[(DONE - 1) as usize] = Block::new(Vec::new(), End::Jump(EDGE_SETUP));
        all.extend(edge_blocks());
        all
    }

    fn numbers(text: &str) -> Vec<i64> {
        text.split_whitespace()
            .map(|token| token.parse().unwrap())
            .collect()
    }

    #[test]
    fn the_edge_program_answers_for_a_row_of_two() {
        use canvas::{Color, Hue, Lightness};
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        let grid = canvas::CodelGrid::from_colors(2, 1, vec![red, red]).unwrap();
        let blocks = canvas::Blocks::partition(grid.clone());
        let label = blocks.blocks()[0]
            .codels()
            .iter()
            .map(|point| ring_index(&grid, *point))
            .min()
            .unwrap();

        let stream = format!("{} {} 0 0", stream_of(&grid).unwrap(), label);
        let output = execute(&edge_blocks_for_test(), &numbers(&stream), 5_000_000);
        // Right, left chooser: the rightmost codel, topmost of that column.
        assert_eq!(output, "1 0 ");
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
