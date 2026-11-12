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
//! # The step stage
//!
//! [`step_state`] carries the same scan one step further. It decodes the winning
//! score back to a codel, seeks the ring to that cell, steps once in the state's
//! direction, and reads the neighbour's colour. A guard or a black neighbour
//! means the attempt fails and the answer is `-1`; white is the case that slides,
//! reported as `-2`; anything else is the move, reported as the neighbour's label
//! and the attempt's `(dp, cc)`.
//!
//! The seek is a loop of one-cell advances, so it costs the winning index rather
//! than the ring: the scan already knows where the cell is, and the ring is a
//! cycle. The scan stops on the last cell it scored, so the step stage advances
//! once more before seeking, which is what puts the index's origin at cell zero.
//!
//! # The exit stage
//!
//! [`exit_state`] wraps that step in the movement rules' eight attempts and the
//! white slide. The attempts differ only in the `(dp, cc)` case, which is rebuilt
//! from the attempt and the query, and each attempt starts with the ring back at
//! cell zero because the scan's coordinates are the loop index. The slide steps
//! and turns the case in place, bounded by `4 * cells` turns, which is the
//! retrace the host detects with a visited set. A direct move also reports the
//! walker's record code for the colour transition — the hue and lightness deltas
//! of the two blocks, with `pointer` and `switch` in their walker codes — and a
//! slide reports `nop` (16), because a slide runs no command.
//!
//! # The start stage
//!
//! [`start_state`] needs no query at all: the upper-left codel is one row and one
//! cell from cell zero, so `row cells + 1`. Black terminates, a chromatic codel is
//! `(label, right, left)`, and white enters the same slide with `case` at right
//! and left.
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
/// The edge program's eight print blocks, one per `(dp, cc)`.
const EDGE_PRINT: u32 = 26;
/// Picks the print block for this case.
const EDGE_SELECT: u32 = 34;

/// The colour code of white, and of black: the palette's last two entries.
const WHITE_CODE: u32 = 18;
const BLACK_CODE: u32 = 19;

/// Where the scan starts in the edge program.
const SCAN_BASE: u32 = 14;

/// The exit stage's blocks: the same scan per attempt, then the step's decision
/// and the loop around it.
const EXIT_SETUP: u32 = 13;
const EXIT_SCAN: u32 = SCAN_BASE;
const EXIT_DECODE_SELECT: u32 = 26;
const EXIT_DECODE: u32 = 27;
const EXIT_SEEK_SETUP: u32 = 35;
const EXIT_MOVE: u32 = 46;
const EXIT_NEXT: u32 = 47;
const EXIT_SLIDE_ENTRY: u32 = 48;
const EXIT_SLIDE: u32 = 51;
const EXIT_ZERO: u32 = 65;
const EXIT_ZERO_ADVANCE: u32 = 66;
const EXIT_EDGE_COLOUR: u32 = 67;
const EXIT_SOURCE_COLOUR: u32 = 68;
const EXIT_SLIDE_MOVE: u32 = 69;
const EXIT_TERMINATE: u32 = 49;
const EXIT_CONTINUE: u32 = 50;

/// The start stage's blocks.
const START_SETUP: u32 = 13;
const START_BLACK: u32 = 14;
const START_WHITE: u32 = 15;
const START_SLIDE_SETUP: u32 = 16;
const START_TERMINATE: u32 = 17;
const START_PRINT: u32 = 18;
const START_SLIDE: u32 = 19;
const START_MOVE: u32 = 33;

/// The step stage's blocks: the same scan, then a decode, a seek, a roll and a
/// colour decision.
const STP_SETUP: u32 = 13;
const STP_SCAN: u32 = SCAN_BASE;
const STP_DECODE_SELECT: u32 = 26;
const STP_DECODE: u32 = 27;
const STP_SEEK_SETUP: u32 = 35;
const STP_SUCCESS: u32 = 46;
const STP_FAIL: u32 = 47;
const STP_WHITE: u32 = 48;

/// A splitter image that also resolves one step of the movement rules from a
/// state, and the stream that asks for it.
///
/// This is one attempt: the block's own edge codel, one step in that direction,
/// and the neighbour it lands on. The image prints the move's target label and
/// the attempt's `(dp, cc)` when the neighbour is chromatic, `-1` when it is off
/// the grid or black (the caller would try the next attempt), and `-2` when it is
/// white (the case the exit stage slides through).
pub fn step_state(
    grid: &CodelGrid,
    block: BlockId,
    dp: Direction,
    cc: Chooser,
) -> Result<(CodelGrid, String), Error> {
    let mut stream = ring_stream(grid)?;
    stream.push_str(&format!(
        " {} {} {}",
        label_of(grid, block)?,
        dp as u32,
        cc as u32
    ));
    Ok((step_program()?.image()?, stream))
}

/// A splitter image that resolves the whole movement rule from a state, and the
/// stream that asks for it.
///
/// The eight attempts are tried in `tessera`'s order, and a white neighbour is
/// slid along until a colour is reached. The image prints the move — the target
/// block's label, the attempt's `(dp, cc)`, and the walker's record code for the
/// colour transition (16, `nop`, after a slide) — or `-1` when every attempt
/// fails, when the slide retraces itself, or when the block is not chromatic.
pub fn exit_state(
    grid: &CodelGrid,
    block: BlockId,
    dp: Direction,
    cc: Chooser,
) -> Result<(CodelGrid, String), Error> {
    let mut stream = ring_stream(grid)?;
    // The exit reads its query as dp, cc, then the label.
    stream.push_str(&format!(
        " {} {} {}",
        dp as u32,
        cc as u32,
        label_of(grid, block)?
    ));
    Ok((exit_program()?.image()?, stream))
}

/// The start program's image, without running it.
pub(crate) fn start_image() -> Result<CodelGrid, Error> {
    start_program()?.image()
}

/// The exit program's image, without running it.
pub(crate) fn exit_image() -> Result<CodelGrid, Error> {
    exit_program()?.image()
}

/// A splitter image that also resolves the program's start state, and the stream that
/// carries the grid to it.
///
/// The upper-left codel decides: black terminates, a chromatic codel is the
/// state `(label, right, left)`, and white slides from that codel with the same
/// rules the exit stage uses. The image prints the state's label and `(dp, cc)`,
/// or `-1`.
pub fn start_state(grid: &CodelGrid) -> Result<(CodelGrid, String), Error> {
    Ok((start_program()?.image()?, ring_stream(grid)?))
}

/// The block's label: the lowest ring position among its codels.
fn label_of(grid: &CodelGrid, block: BlockId) -> Result<u32, Error> {
    let blocks = Blocks::partition(grid.clone());
    blocks
        .block(block)
        .and_then(|target| {
            target
                .codels()
                .iter()
                .map(|point| ring_index(grid, *point))
                .min()
        })
        .ok_or(Error::TooLarge)
}

/// A splitter image and the stream that carries `grid`'s colours to it.
///
/// The image is the same for every grid. Running it prints one label per ring
/// cell; two cells share a label exactly when they are in the same block.
pub fn splitter(grid: &CodelGrid) -> Result<(CodelGrid, String), Error> {
    Ok((program()?.image()?, ring_stream(grid)?))
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
    let mut stream = ring_stream(grid)?;
    stream.push_str(&format!(
        " {} {} {}",
        label_of(grid, block)?,
        dp as u32,
        cc as u32
    ));
    Ok((edge_program()?.image()?, stream))
}

/// The ring position of a codel.
pub(crate) fn ring_index(grid: &CodelGrid, point: Point) -> u32 {
    (point.y + 1) * (grid.width() + 2) + (point.x + 1)
}

/// The ring a grid's colours become: the ring size, then one
/// `ring stride colour index` record per cell, highest index first.
///
/// This is the stream [`splitter`], [`start_state`] and [`exit_state`] take, and
/// what [`crate::ring_grid`] prints from the colours alone.
pub fn ring_stream(grid: &CodelGrid) -> Result<String, Error> {
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
    all.push(Block::new(edge_setup(), End::Jump(SCAN_BASE)));
    let (_, scan) = scan_blocks(SCAN_BASE, EDGE_SELECT, 0);
    all.extend(scan);
    for case in 0..8 {
        all.push(Block::new(print_case(case), End::Halt));
    }
    all.push(Block::new(dispatch(3, EDGE_PRINT), End::Computed));
    Program::new(all, ENTRY)
}

/// The splitter image, extended with the whole movement rule: the eight
/// attempts, and what each one lands on.
fn exit_program() -> Result<Program, Error> {
    Program::new(exit_blocks(), ENTRY)
}

/// The exit program's blocks, as `exit_program` assembles them.
fn exit_blocks() -> Vec<Block> {
    let mut all = blocks();
    all[(DONE - 1) as usize] = Block::new(Vec::new(), End::Jump(EXIT_SETUP));
    all.push(Block::new(exit_setup(), End::Jump(EXIT_ZERO)));
    all.extend(scan_and_decode(
        EXIT_SCAN,
        EXIT_DECODE_SELECT,
        EXIT_DECODE,
        3,
    ));
    all.extend(step_tail(
        3,
        EXIT_SEEK_SETUP,
        EXIT_MOVE,
        EXIT_NEXT,
        EXIT_SLIDE_ENTRY,
        EXIT_EDGE_COLOUR,
    ));
    all.push(Block::new(move_nodes(9), End::Halt));
    all.push(Block::new(
        exit_next(),
        End::If {
            then: EXIT_TERMINATE,
            otherwise: EXIT_CONTINUE,
        },
    ));
    // The white branch starts the slide with no turns taken.
    all.push(Block::new(
        vec![Node::Command(Command::Pop), Node::Const(0)],
        End::Jump(EXIT_SLIDE),
    ));
    all.push(Block::new(
        vec![Node::Const(-1), Node::Command(Command::OutNumber)],
        End::Halt,
    ));
    all.push(Block::new(
        {
            let mut nodes = case_nodes();
            nodes.extend([Node::Const(-1), Node::Const(0)]);
            nodes
        },
        End::Jump(EXIT_ZERO),
    ));
    all.extend(slide_blocks(
        &slide_ids(EXIT_SLIDE),
        9,
        EXIT_SLIDE_MOVE,
        EXIT_TERMINATE,
    ));
    // The scan's coordinates are the loop index, so every attempt starts with
    // the ring back at cell zero. Cell zero is the only record whose label is
    // zero: it is a guard, so no component can ever merge into it.
    all.push(Block::new(
        vec![Node::DupLocal(10), Node::Command(Command::Not)],
        End::If {
            then: EXIT_SCAN,
            otherwise: EXIT_ZERO_ADVANCE,
        },
    ));
    all.push(Block::new(
        ring_by(9, i64::from(PER_CELL)),
        End::Jump(EXIT_ZERO),
    ));
    // The movement rules never leave a white or black block. The edge codel is
    // a codel of the block, so its colour is the block's colour.
    all.push(Block::new(
        vec![
            Node::DupLocal(12),
            Node::Const(i64::from(WHITE_CODE) - 1),
            Node::Command(Command::Greater),
        ],
        End::If {
            then: EXIT_TERMINATE,
            otherwise: EXIT_SOURCE_COLOUR,
        },
    ));
    // Keep the source colour in the dead `best` slot: the move prints its
    // command from the source and the target's colours.
    all.push(Block::new(
        vec![
            Node::DupLocal(12),
            Node::Roll(3, 1),
            Node::Roll(4, 3),
            Node::Command(Command::Pop),
        ],
        End::Jump(EXIT_SEEK_SETUP + 3),
    ));
    // A slide runs no command, so its move prints `nop` rather than a
    // transition.
    all.push(Block::new(
        {
            let mut nodes = bare_move_nodes(9);
            nodes.extend([
                Node::Const(32),
                Node::Command(Command::OutChar),
                Node::Const(16),
                Node::Command(Command::OutNumber),
            ]);
            nodes
        },
        End::Halt,
    ));
    all
}

/// The splitter image, extended with the start state.
fn start_program() -> Result<Program, Error> {
    Program::new(start_blocks(), ENTRY)
}

/// The start program's blocks, as `start_program` assembles them.
fn start_blocks() -> Vec<Block> {
    let mut all = blocks();
    all[(DONE - 1) as usize] = Block::new(Vec::new(), End::Jump(START_SETUP));
    all.push(Block::new(start_setup(), End::Jump(START_BLACK)));
    all.push(Block::new(
        colour_is(2, BLACK_CODE),
        End::If {
            then: START_TERMINATE,
            otherwise: START_WHITE,
        },
    ));
    all.push(Block::new(
        colour_is(2, WHITE_CODE),
        End::If {
            then: START_SLIDE_SETUP,
            otherwise: START_PRINT,
        },
    ));
    all.push(Block::new(start_slide_setup(), End::Jump(START_SLIDE)));
    all.push(Block::new(
        vec![Node::Const(-1), Node::Command(Command::OutNumber)],
        End::Halt,
    ));
    all.push(Block::new(start_print(), End::Halt));
    all.extend(slide_blocks(
        &slide_ids(START_SLIDE),
        9,
        START_MOVE,
        START_TERMINATE,
    ));
    all.push(Block::new(bare_move_nodes(9), End::Halt));
    all
}

/// The entry points of a slide's fourteen blocks.
struct Slide {
    /// Step in the case's direction.
    dispatch: u32,
    /// The four step blocks, `step + dp`.
    step: u32,
    /// The guard test; the black and white tests follow it.
    check: u32,
    /// Re-dispatch to the undo blocks after a refusal.
    turn_dispatch: u32,
    /// The four undo blocks, `undo + dp`.
    undo: u32,
    /// Turn the case and check the bound.
    turn: u32,
}

fn slide_ids(base: u32) -> Slide {
    Slide {
        dispatch: base,
        step: base + 1,
        check: base + 5,
        turn_dispatch: base + 8,
        undo: base + 9,
        turn: base + 13,
    }
}

/// The fourteen blocks of a slide.
///
/// `live` is the number of values above the ring — nine in both programs, with
/// the case at depth three, `big` at depth four and the turn counter in the
/// scan's dead index slot. `move_to` and `terminate` are the caller's answers.
fn slide_blocks(ids: &Slide, live: u32, move_to: u32, terminate: u32) -> Vec<Block> {
    let colour = live + 2;
    let mut blocks = vec![Block::new(slide_dispatch(ids.step), End::Computed)];
    for dp in 0..4 {
        blocks.push(Block::new(step_roll(dp, live), End::Jump(ids.check)));
    }
    blocks.push(Block::new(
        vec![
            Node::DupLocal(colour),
            Node::Const(i64::from(GUARD_CODE) - 1),
            Node::Command(Command::Greater),
        ],
        End::If {
            then: ids.turn_dispatch,
            otherwise: ids.check + 1,
        },
    ));
    blocks.push(Block::new(
        colour_is(colour, BLACK_CODE),
        End::If {
            then: ids.turn_dispatch,
            otherwise: ids.check + 2,
        },
    ));
    blocks.push(Block::new(
        colour_is(colour, WHITE_CODE),
        End::If {
            then: ids.dispatch,
            otherwise: move_to,
        },
    ));
    blocks.push(Block::new(slide_dispatch(ids.undo), End::Computed));
    for dp in 0..4 {
        blocks.push(Block::new(
            step_roll((dp + 2) % 4, live),
            End::Jump(ids.turn),
        ));
    }
    blocks.push(Block::new(
        slide_turn(),
        End::If {
            then: terminate,
            otherwise: ids.dispatch,
        },
    ));
    blocks
}

/// Dispatch to the step block for the case's direction, `base + dp_i`.
///
/// The live values are `[turns, best, case, big, row cells, label, attempt, cc,
/// dp]`, so the case is at depth three and the step block pops the direction it
/// leaves on top.
fn slide_dispatch(base: u32) -> Vec<Node> {
    let mut nodes = vec![
        Node::DupLocal(3),
        Node::Const(2),
        Node::Command(Command::Divide),
    ];
    nodes.extend(dispatch(1, base));
    nodes
}

/// A refused step: step back to the white cell, turn, and check the bound.
///
/// `case = 2 * dp_i + cc_i` becomes `2 * ((dp_i + 1) % 4) + (1 - cc_i)`, which is
/// `tessera`'s `dp.clockwise()` and `cc.toggled()`. The turn counter lives in the
/// scan's dead `index` slot; more than `4 * big` turns means a `(codel, dp)` pair
/// has repeated, which is how the host's visited set decides to terminate.
fn slide_turn() -> Vec<Node> {
    vec![
        // turns += 1
        Node::DupLocal(1),
        Node::Const(1),
        Node::Command(Command::Add),
        Node::Roll(2, 1),
        Node::Command(Command::Pop),
        // new case = 2 * ((case / 2 + 1) % 4) + (1 - case % 2)
        Node::DupLocal(3),
        Node::Const(2),
        Node::Command(Command::Divide),
        Node::Const(1),
        Node::Command(Command::Add),
        Node::Const(4),
        Node::Command(Command::Modulo),
        Node::Const(2),
        Node::Command(Command::Multiply),
        Node::DupLocal(4),
        Node::Const(2),
        Node::Command(Command::Modulo),
        Node::Const(1),
        Node::Swap,
        Node::Command(Command::Subtract),
        Node::Command(Command::Add),
        // replace the old case with the new one, keeping it under turns and best
        Node::Roll(4, 3),
        Node::Command(Command::Pop),
        Node::Roll(3, 1),
        // terminate once every (codel, dp) pair has been seen
        Node::DupLocal(1),
        Node::DupLocal(5),
        Node::Const(4),
        Node::Command(Command::Multiply),
        Node::Command(Command::Subtract),
        Node::Const(0),
        Node::Command(Command::Greater),
    ]
}

/// `dp`, `cc`, the block's label, the row length, the score base, and the first
/// attempt's case.
///
/// The live values end up as `[index, best, case, big, row cells, label,
/// attempt, cc, dp]`: the scan's six on top and the exit's own three below, which
/// is why the scan is told there are three extra.
fn exit_setup() -> Vec<Node> {
    let mut nodes = vec![
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Command(Command::InNumber),
        Node::Command(Command::InNumber),
        Node::Const(0),
        Node::Command(Command::InNumber),
        // row cells = stride / 4
        Node::DupLocal(7),
        Node::Const(4),
        Node::Command(Command::Divide),
        // big = ring cells / 4
        Node::DupLocal(9),
        Node::Const(4),
        Node::Command(Command::Divide),
    ];
    nodes.extend(case_nodes());
    nodes.extend([Node::Const(-1), Node::Const(0)]);
    nodes
}

/// Two pops for the partition's flags, then one row and one cell: ring index
/// `row cells + 1`, the upper-left codel.
fn start_setup() -> Vec<Node> {
    let mut nodes = vec![Node::Command(Command::Pop), Node::Command(Command::Pop)];
    nodes.extend(ring_down(0));
    nodes.extend(ring_by(0, i64::from(PER_CELL)));
    nodes
}

/// The nine live values the slide expects, with the case at right/left and the
/// turn counter at zero.
///
/// The slide does not otherwise care about these values; they exist to give
/// `case` the depth it has in the exit, and `big` the ring's cell count.
fn start_slide_setup() -> Vec<Node> {
    vec![
        Node::Const(0), // dp
        Node::Const(0), // cc
        Node::Const(0), // attempt
        Node::Const(0), // label
        Node::Const(0), // row cells
        // big = ring cells / 4, from the ring size five values down
        Node::DupLocal(9),
        Node::Const(4),
        Node::Command(Command::Divide),
        Node::Const(0), // case: right, left
        Node::Const(0), // best
        Node::Const(0), // turns
    ]
}

/// The state `(label, right, left)` for a chromatic upper-left codel.
fn start_print() -> Vec<Node> {
    vec![
        Node::DupLocal(1),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        Node::Const(0),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        Node::Const(0),
        Node::Command(Command::OutNumber),
    ]
}

/// `case = 2 * dp_i + cc_i` for the current attempt, from `[big, row cells,
/// label, attempt, cc, dp]`.
///
/// `dp_i = (dp + attempt/2) % 4` and
/// `cc_i = (cc + (attempt%2 + attempt/2) % 2) % 2`, which is `tessera`'s
/// `attempt(dp, cc, index)`.
fn case_nodes() -> Vec<Node> {
    vec![
        // q = attempt / 2, kept for the direction and the toggle
        Node::DupLocal(4),
        Node::Const(2),
        Node::Command(Command::Divide),
        Node::Dup,
        // dp_i = (dp + q) % 4
        Node::DupLocal(8),
        Node::Command(Command::Add),
        Node::Const(4),
        Node::Command(Command::Modulo),
        // toggle = (attempt%2 + q) % 2
        Node::DupLocal(6),
        Node::Const(2),
        Node::Command(Command::Modulo),
        Node::DupLocal(3),
        Node::Command(Command::Add),
        Node::Const(2),
        Node::Command(Command::Modulo),
        // cc_i = (cc + toggle) % 2
        Node::DupLocal(8),
        Node::Command(Command::Add),
        Node::Const(2),
        Node::Command(Command::Modulo),
        // case = 2 * dp_i + cc_i
        Node::DupLocal(2),
        Node::Const(2),
        Node::Command(Command::Multiply),
        Node::Roll(2, 1),
        Node::Command(Command::Add),
        // drop q and dp_i, leaving the case above big
        Node::Roll(3, 1),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]
}

/// Drop the scan's three, count the attempt up, and stop after eight.
///
/// On entry the stack is `[index, best, case, big, row cells, label, attempt,
/// cc, dp]`; the top three are this attempt's and are rebuilt by the continue
/// block, so dropping them leaves `[big, row cells, label, attempt, cc, dp]`.
fn exit_next() -> Vec<Node> {
    vec![
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Roll(4, 3),
        Node::Const(1),
        Node::Command(Command::Add),
        Node::Roll(4, 1),
        Node::DupLocal(4),
        Node::Const(8),
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
    ]
}

/// The splitter image, extended with the step stage.
fn step_program() -> Result<Program, Error> {
    let mut all = blocks();
    all[(DONE - 1) as usize] = Block::new(Vec::new(), End::Jump(STP_SETUP));
    all.push(Block::new(edge_setup(), End::Jump(STP_SCAN)));
    all.extend(scan_and_decode(STP_SCAN, STP_DECODE_SELECT, STP_DECODE, 0));
    all.extend(step_tail(
        0,
        STP_SEEK_SETUP,
        STP_SUCCESS,
        STP_FAIL,
        STP_WHITE,
        STP_SEEK_SETUP + 3,
    ));
    all.push(Block::new(bare_move_nodes(6), End::Halt));
    all.push(Block::new(
        vec![Node::Const(-1), Node::Command(Command::OutNumber)],
        End::Halt,
    ));
    all.push(Block::new(
        vec![Node::Const(-2), Node::Command(Command::OutNumber)],
        End::Halt,
    ));
    Program::new(all, ENTRY)
}

/// The scan, its decode dispatch, and the eight decode blocks.
///
/// `extra` is how many live values sit under the scan's six; the decode leaves
/// `[column, row]` above them, so the seek starts at `select + 9`.
fn scan_and_decode(scan: u32, select: u32, decode: u32, extra: u32) -> Vec<Block> {
    let (_, mut blocks) = scan_blocks(scan, select, extra);
    blocks.push(Block::new(
        {
            // The scan stops on the last cell, so step once more to reach zero,
            // where the seek's index is measured from.
            let mut nodes = ring_by(6 + extra, i64::from(PER_CELL));
            nodes.extend(dispatch(3, decode));
            nodes
        },
        End::Computed,
    ));
    let seek = select + 9;
    for case in 0..8 {
        blocks.push(Block::new(decode_case(case), End::Jump(seek)));
    }
    blocks
}

/// The seek, the step and the colour decision, with `live` values under the
/// scan's six.
///
/// `base` is the first of eleven blocks; `success`, `fail` and `white` are the
/// three answers, which the caller places, and `done` is where the seek loop
/// goes once it has found the edge codel.
fn step_tail(live: u32, base: u32, success: u32, fail: u32, white: u32, done: u32) -> Vec<Block> {
    let mut blocks = vec![
        // seek: index = row * cells + column, then drop the column and row
        Block::new(seek_setup(), End::Jump(base + 1)),
        // while the index is not zero
        Block::new(
            vec![Node::DupLocal(1), Node::Command(Command::Not)],
            End::If {
                then: done,
                otherwise: base + 2,
            },
        ),
        // one cell forward
        Block::new(
            {
                let mut nodes = ring_by(7 + live, i64::from(PER_CELL));
                nodes.extend([Node::Const(1), Node::Command(Command::Subtract)]);
                nodes
            },
            End::Jump(base + 1),
        ),
        // the direction decides which roll the step takes
        Block::new(
            {
                let mut nodes = vec![
                    Node::Command(Command::Pop),
                    Node::DupLocal(3),
                    Node::Const(2),
                    Node::Command(Command::Divide),
                ];
                nodes.extend(dispatch(1, base + 4));
                nodes
            },
            End::Computed,
        ),
    ];
    for dp in 0..4 {
        blocks.push(Block::new(step_roll(dp, 6 + live), End::Jump(base + 8)));
    }
    blocks.extend([
        // a guard is off the grid, which fails the attempt like black
        Block::new(
            vec![
                Node::DupLocal(8 + live),
                Node::Const(i64::from(GUARD_CODE) - 1),
                Node::Command(Command::Greater),
            ],
            End::If {
                then: fail,
                otherwise: base + 9,
            },
        ),
        Block::new(
            colour_is(8 + live, BLACK_CODE),
            End::If {
                then: fail,
                otherwise: base + 10,
            },
        ),
        Block::new(
            colour_is(8 + live, WHITE_CODE),
            End::If {
                then: white,
                otherwise: success,
            },
        ),
    ]);
    blocks
}

/// The ids one copy of the edge scan occupies.
struct Scan {
    cell: u32,
    next: u32,
    advance: u32,
    matcher: u32,
    cases: u32,
}

/// One copy of the edge scan, starting at block `base`.
///
/// The scan is five shared blocks and eight scoring blocks. It ends at `done`,
/// which is what lets the same loop serve both the edge stage (print the winner)
/// and the step stage (carry on to the roll).
///
/// The live values are `[index, best, case, big, row cells, label]`: the ring
/// index being visited, the best score so far, the `(dp, cc)` case, the score
/// base (`big`, the cell count), the row length in cells, and the block's label.
fn scan_blocks(base: u32, done: u32, extra: u32) -> (Scan, Vec<Block>) {
    let ids = Scan {
        cell: base,
        next: base + 1,
        advance: base + 2,
        matcher: base + 3,
        cases: base + 4,
    };
    let mut blocks = vec![
        // does this cell belong to the block?
        Block::new(
            vec![
                Node::DupLocal(7 + extra),
                Node::DupLocal(7),
                Node::Command(Command::Subtract),
                Node::Command(Command::Not),
            ],
            End::If {
                then: ids.matcher,
                otherwise: ids.next,
            },
        ),
        // one more cell seen; all of them means the score is final
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
                then: done,
                otherwise: ids.advance,
            },
        ),
        // the next cell
        Block::new(ring_by(6 + extra, i64::from(PER_CELL)), End::Jump(ids.cell)),
        // the codel's coordinates, then the scoring block for this case
        Block::new(
            {
                let mut nodes = cell_coordinates();
                nodes.extend([
                    Node::DupLocal(5),
                    Node::Const(i64::from(ids.cases)),
                    Node::Command(Command::Add),
                ]);
                nodes
            },
            End::Computed,
        ),
    ];
    for case in 0..8 {
        blocks.push(Block::new(score_case(case), End::Jump(ids.next)));
    }
    (ids, blocks)
}

/// `base + case`, then dispatch to it.
fn dispatch(depth: u32, base: u32) -> Vec<Node> {
    vec![
        Node::DupLocal(depth),
        Node::Const(i64::from(base)),
        Node::Command(Command::Add),
    ]
}

/// The edge codel's ring index, from the decoded `(column, row)`.
fn seek_setup() -> Vec<Node> {
    vec![
        Node::DupLocal(2),
        Node::DupLocal(8),
        Node::Command(Command::Multiply),
        Node::Swap,
        Node::Command(Command::Add),
        // the row copy sits under the index now
        Node::Roll(2, 1),
        Node::Command(Command::Pop),
    ]
}

/// One step in `dp`, taken with the head on the edge codel.
///
/// The dispatch leaves `dp` on the stack, so the block drops it first.
fn step_roll(dp: u32, live: u32) -> Vec<Node> {
    let mut nodes = vec![Node::Command(Command::Pop)];
    nodes.extend(match dp {
        0 => ring_by(live, i64::from(PER_CELL)),
        2 => ring_by(live, -i64::from(PER_CELL)),
        3 => ring_up(live),
        _ => ring_down(live),
    });
    nodes
}

/// `colour == code`, for the neighbour's colour at depth eight.
fn colour_is(depth: u32, code: u32) -> Vec<Node> {
    vec![
        Node::DupLocal(depth),
        Node::Const(i64::from(code)),
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
    ]
}

/// Print the move: the neighbour's label, then the attempt's `(dp, cc)`.
/// Print the move: the neighbour's label, the attempt's `(dp, cc)`, and the
/// walker's record code for the colour transition from the source block.
fn move_nodes(live: u32) -> Vec<Node> {
    let mut nodes = vec![
        Node::DupLocal(live + 1),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        Node::DupLocal(3),
        Node::Const(2),
        Node::Command(Command::Divide),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        Node::DupLocal(3),
        Node::Const(2),
        Node::Command(Command::Modulo),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
    ];
    nodes.extend(command_nodes(live));
    nodes.push(Node::Command(Command::OutNumber));
    nodes
}

/// The start's move and the step stage's success: the same three fields, but no
/// command, because neither the start state nor a single attempt has one.
fn bare_move_nodes(live: u32) -> Vec<Node> {
    vec![
        Node::DupLocal(live + 1),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        Node::DupLocal(3),
        Node::Const(2),
        Node::Command(Command::Divide),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        Node::DupLocal(3),
        Node::Const(2),
        Node::Command(Command::Modulo),
        Node::Command(Command::OutNumber),
    ]
}

/// The walker's record code for the colour transition from the source block to
/// the target.
///
/// On entry the source colour is at depth two — the `best` slot, dead once the
/// seek has run — and the target's at `live + 2`, the ring head's colour. The
/// code is left on top.
///
/// The transition is the hue and lightness deltas, `hue * 3 + lightness`;
/// `tessera`'s table maps that to a command, and the walker's record code is
/// `e` from one to nine, `17` and `18` for `pointer` and `switch`, `e - 2` above
/// them, and `16` (`nop`) when the transition runs nothing.
fn command_nodes(live: u32) -> Vec<Node> {
    vec![
        // source and target
        Node::DupLocal(2),
        Node::DupLocal(live + 3),
        // hue = (target / 3 + 6 - source / 3) % 6
        Node::DupLocal(1),
        Node::Const(3),
        Node::Command(Command::Divide),
        Node::Const(6),
        Node::Command(Command::Add),
        Node::DupLocal(3),
        Node::Const(3),
        Node::Command(Command::Divide),
        Node::Command(Command::Subtract),
        Node::Const(6),
        Node::Command(Command::Modulo),
        // lightness = (target % 3 + 3 - source % 3) % 3
        Node::DupLocal(2),
        Node::Const(3),
        Node::Command(Command::Modulo),
        Node::Const(3),
        Node::Command(Command::Add),
        Node::DupLocal(4),
        Node::Const(3),
        Node::Command(Command::Modulo),
        Node::Command(Command::Subtract),
        Node::Const(3),
        Node::Command(Command::Modulo),
        // e = 3 * hue + lightness
        Node::Swap,
        Node::Const(3),
        Node::Command(Command::Multiply),
        Node::Command(Command::Add),
        // code = e + 7 * (e / 10) * (e / 12 == 0) - 2 * (e / 12) + 16 * (e == 0)
        Node::DupLocal(1),
        Node::Command(Command::Not),
        Node::Const(16),
        Node::Command(Command::Multiply),
        Node::DupLocal(2),
        Node::Const(10),
        Node::Command(Command::Divide),
        Node::DupLocal(3),
        Node::Const(12),
        Node::Command(Command::Divide),
        Node::Command(Command::Not),
        Node::Command(Command::Multiply),
        Node::Const(7),
        Node::Command(Command::Multiply),
        Node::DupLocal(3),
        Node::Const(12),
        Node::Command(Command::Divide),
        Node::Const(-2),
        Node::Command(Command::Multiply),
        Node::Command(Command::Add),
        Node::Command(Command::Add),
        Node::Command(Command::Add),
    ]
}

/// The step's decode: the winning score back to `(column, row)` as values.
///
/// The inverse of [`score_case`] — the halves are flipped back, and the primary
/// and secondary become column and row by the same rule — leaving the live values
/// as `[column, row, index, best, case, big, row cells, label]`.
fn decode_case(case: u32) -> Vec<Node> {
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
    // Drop the raw halves, which sit under the two values just computed.
    nodes.extend([
        Node::Roll(4, 2),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]);
    let (column, row) = if primary_is_x { (2, 1) } else { (1, 2) };
    nodes.push(Node::DupLocal(row));
    nodes.push(Node::DupLocal(column + 1));
    // [column, row, secondary value, primary value, index, best, ...]: drop the
    // two values the coordinates came from.
    nodes.extend([
        Node::Roll(4, 2),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]);
    nodes
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
    use canvas::{Hue, Lightness};
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
        all.push(Block::new(edge_setup(), End::Jump(SCAN_BASE)));
        let (_, scan) = scan_blocks(SCAN_BASE, EDGE_SELECT, 0);
        all.extend(scan);
        for case in 0..8 {
            all.push(Block::new(print_case(case), End::Halt));
        }
        all.push(Block::new(dispatch(3, EDGE_PRINT), End::Computed));
        all
    }

    /// The step program's blocks, as `step_program` assembles them.
    fn step_blocks_for_test() -> Vec<Block> {
        let mut all = blocks();
        all[(DONE - 1) as usize] = Block::new(Vec::new(), End::Jump(STP_SETUP));
        all.push(Block::new(edge_setup(), End::Jump(STP_SCAN)));
        all.extend(scan_and_decode(STP_SCAN, STP_DECODE_SELECT, STP_DECODE, 0));
        all.extend(step_tail(
            0,
            STP_SEEK_SETUP,
            STP_SUCCESS,
            STP_FAIL,
            STP_WHITE,
            STP_SEEK_SETUP + 3,
        ));
        all.push(Block::new(bare_move_nodes(6), End::Halt));
        all.push(Block::new(
            vec![Node::Const(-1), Node::Command(Command::OutNumber)],
            End::Halt,
        ));
        all.push(Block::new(
            vec![Node::Const(-2), Node::Command(Command::OutNumber)],
            End::Halt,
        ));
        all
    }

    /// The exit program's blocks, as `exit_program` assembles them.
    fn exit_blocks_for_test() -> Vec<Block> {
        exit_blocks()
    }

    /// The start program's blocks, as `start_program` assembles them.
    fn start_blocks_for_test() -> Vec<Block> {
        start_blocks()
    }

    fn numbers(text: &str) -> Vec<i64> {
        text.split_whitespace()
            .map(|token| token.parse().unwrap())
            .collect()
    }

    /// A grid from rows of characters, as in the integration tests.
    fn grid(rows: &[&str]) -> CodelGrid {
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, |row| row.len() as u32);
        let mut codels = Vec::with_capacity((width as usize) * (height as usize));
        for row in rows {
            for cell in row.chars() {
                codels.push(match cell {
                    '.' => Color::White,
                    '#' => Color::Black,
                    'r' => Color::chromatic(Hue::Red, Lightness::Normal),
                    'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                    'b' => Color::chromatic(Hue::Blue, Lightness::Normal),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        CodelGrid::from_colors(width, height, codels).unwrap()
    }

    /// Run the exit program's blocks over one query.
    fn exit_vm(grid: &CodelGrid, block: BlockId, dp: Direction, cc: Chooser) -> String {
        let stream = format!(
            "{} {} {} {}",
            ring_stream(grid).unwrap(),
            dp as u32,
            cc as u32,
            label_of(grid, block).unwrap()
        );
        execute(&exit_blocks_for_test(), &numbers(&stream), 200_000_000)
    }

    /// Run the start program's blocks over one grid.
    fn start_vm(grid: &CodelGrid) -> String {
        execute(
            &start_blocks_for_test(),
            &numbers(&ring_stream(grid).unwrap()),
            200_000_000,
        )
    }

    #[test]
    fn the_edge_program_answers_for_a_row_of_two() {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        let grid = canvas::CodelGrid::from_colors(2, 1, vec![red, red]).unwrap();
        let blocks = canvas::Blocks::partition(grid.clone());
        let label = blocks.blocks()[0]
            .codels()
            .iter()
            .map(|point| ring_index(&grid, *point))
            .min()
            .unwrap();

        let stream = format!("{} {} 0 0", ring_stream(&grid).unwrap(), label);
        let output = execute(&edge_blocks_for_test(), &numbers(&stream), 5_000_000);
        // Right, left chooser: the rightmost codel, topmost of that column.
        assert_eq!(output, "1 0 ");
    }

    #[test]
    fn the_exit_program_steps_onto_the_neighbour() {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        let green = Color::chromatic(Hue::Green, Lightness::Normal);
        let grid = canvas::CodelGrid::from_colors(2, 1, vec![red, green]).unwrap();
        let blocks = canvas::Blocks::partition(grid.clone());
        let label = blocks.blocks()[0]
            .codels()
            .iter()
            .map(|point| ring_index(&grid, *point))
            .min()
            .unwrap();

        // The query is dp, cc, then the label. Red to green is two hues on
        // (code six), the walker's `divide`.
        let stream = format!("{} 0 0 {}", ring_stream(&grid).unwrap(), label);
        let output = execute(&exit_blocks_for_test(), &numbers(&stream), 40_000_000);
        assert_eq!(output, "6 0 0 6");
    }

    #[test]
    fn the_exit_program_gives_up_on_a_single_codel() {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        let grid = canvas::CodelGrid::from_colors(1, 1, vec![red]).unwrap();
        let label = ring_index(&grid, canvas::Point::new(0, 0));
        let stream = format!("{} 0 0 {}", ring_stream(&grid).unwrap(), label);
        let output = execute(&exit_blocks_for_test(), &numbers(&stream), 40_000_000);
        assert_eq!(output, "-1");
    }

    #[test]
    fn the_start_program_finds_the_upper_left_block() {
        let grid = grid(&["rg", "gg"]);
        let blocks = canvas::Blocks::partition(grid.clone());
        let red = blocks.block_id_at(Point::new(0, 0)).unwrap();
        let expected = format!("{} 0 0", label_of(&grid, red).unwrap());
        assert_eq!(start_vm(&grid), expected);
    }

    #[test]
    fn the_start_program_terminates_on_black() {
        assert_eq!(start_vm(&grid(&["#r"])), "-1");
    }

    #[test]
    fn the_start_program_slides_from_a_white_corner() {
        let grid = grid(&["..b"]);
        let blocks = canvas::Blocks::partition(grid.clone());
        let blue = blocks.block_id_at(Point::new(2, 0)).unwrap();
        let expected = format!("{} 0 0", label_of(&grid, blue).unwrap());
        assert_eq!(start_vm(&grid), expected);
    }

    #[test]
    fn the_start_program_turns_a_blocked_slide() {
        // The corner slides right into the black, turns down, and finds the red.
        let grid = grid(&[".#", "r."]);
        let blocks = canvas::Blocks::partition(grid.clone());
        let red = blocks.block_id_at(Point::new(0, 1)).unwrap();
        let expected = format!(
            "{} {} {}",
            label_of(&grid, red).unwrap(),
            Direction::Down as u32,
            Chooser::Right as u32
        );
        assert_eq!(start_vm(&grid), expected);
    }

    #[test]
    fn the_exit_program_slides_to_the_first_colour() {
        // A slide runs no command, so the fourth field is `nop`.
        let grid = grid(&["r...b"]);
        let blocks = canvas::Blocks::partition(grid.clone());
        let red = blocks.block_id_at(Point::new(0, 0)).unwrap();
        let blue = blocks.block_id_at(Point::new(4, 0)).unwrap();
        let expected = format!("{} 0 0 16", label_of(&grid, blue).unwrap());
        assert_eq!(
            exit_vm(&grid, red, Direction::Right, Chooser::Left),
            expected
        );
    }

    #[test]
    fn the_exit_program_terminates_on_a_white_dead_end() {
        let grid = grid(&["r.", "#."]);
        let blocks = canvas::Blocks::partition(grid.clone());
        let red = blocks.block_id_at(Point::new(0, 0)).unwrap();
        assert_eq!(exit_vm(&grid, red, Direction::Right, Chooser::Left), "-1");
    }

    #[test]
    fn the_exit_program_restarts_the_slide_from_the_current_codel() {
        // Right is blocked at the top-right, so the slide turns down, runs to
        // the black, and turns back onto the red with dp left.
        let grid = grid(&["r..", "#.#", ".b."]);
        let blocks = canvas::Blocks::partition(grid.clone());
        let red = blocks.block_id_at(Point::new(0, 0)).unwrap();
        let expected = format!(
            "{} {} {} 16",
            label_of(&grid, red).unwrap(),
            Direction::Left as u32,
            Chooser::Left as u32
        );
        assert_eq!(
            exit_vm(&grid, red, Direction::Right, Chooser::Left),
            expected
        );
    }

    #[test]
    fn the_step_program_answers_for_a_row_of_two() {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        let green = Color::chromatic(Hue::Green, Lightness::Normal);
        let grid = canvas::CodelGrid::from_colors(2, 1, vec![red, green]).unwrap();
        let blocks = canvas::Blocks::partition(grid.clone());
        let label = blocks.blocks()[0]
            .codels()
            .iter()
            .map(|point| ring_index(&grid, *point))
            .min()
            .unwrap();

        let stream = format!("{} {} 0 0", ring_stream(&grid).unwrap(), label);
        let output = execute(&step_blocks_for_test(), &numbers(&stream), 20_000_000);
        // The red is one cell wide: its own edge is (0, 0), and one step right
        // lands on the green, whose label is 6 in a row four cells long.
        assert_eq!(output, "6 0 0");
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
