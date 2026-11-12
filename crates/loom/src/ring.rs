//! The ring a grid's colours become, in the image.
//!
//! [`ring_grid`] weaves a program that reads the grid [`crate::ppm_grid`] prints
//! — the width, the height, and one colour code per codel — and prints the
//! stream [`crate::ring_stream`] builds: the ring size, then one
//! `ring stride colour index` record per cell, highest index first.
//!
//! That is the stream the splitter and the movement stages take, so with this
//! stage a grid's colours never have to pass through the host as a `CodelGrid`
//! on the way to them. The host still reads the records back, because a stream
//! is what a stage can print.
//!
//! # How it is built
//!
//! The ring is one cell wider on every side than the grid, and each cell is four
//! values. Border cells are guards carrying `22 + index`; every other cell
//! carries the next colour code from the input, which arrives in ring order
//! because the ring's interior is the grid in row-major order.
//!
//! The records come out highest index first, so the head ends up on top of the
//! reader's stack. The image gets there the same way a stack does: it pushes
//! every record in forward order and then pops them, which reverses them for
//! free. The cost is that the loop's counters sit above the records it has
//! accumulated, so each lap rolls them back to the top — one roll over a window
//! that grows by four values per cell. That makes a lap O(cells), so the whole
//! ring is O(cells squared); the splitter already pays that, and this is why the
//! completeness claim stops at small images.
//!
//! The guard test is arithmetic rather than a chain of comparisons. A cell is a
//! guard when its row is `0` or `height + 1`, or its column is `0` or
//! `width + 1`, and `row * (row - height - 1) * column * (column - width - 1)` is
//! zero exactly then. The two row factors are computed with the subtraction
//! reversed, which only flips the sign.

use canvas::{Blocks, CodelGrid, Color};
use interp::{ExitReason, Options};
use runtime::BufferIo;
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};

/// The colour every guard cell carries, offset by its ring index.
const GUARD_CODE: i64 = 22;

const ENTRY: u32 = 1;
const LOOP: u32 = 2;
/// Computes the next cell's colour and decides guard or codel.
const BODY: u32 = 3;
/// Pushes `22 + index`.
const GUARD: u32 = 4;
/// Reads the next colour code.
const CODEL: u32 = 5;
/// Pushes the record, advances the index, and rolls the counters back up.
const PUSH: u32 = 6;
/// Pops the counters, leaving the records over the sentinel.
const SENTINEL: u32 = 7;
/// Prints one record per lap, until the sentinel surfaces.
const PRINT: u32 = 8;
const EMIT: u32 = 9;
const HALT: u32 = 10;

/// The program that turns grid colours into the ring stream.
pub fn ring_program() -> Result<Program, Error> {
    Program::new(blocks(), ENTRY)
}

/// The same program, laid out as an image.
pub fn ring_grid() -> Result<CodelGrid, Error> {
    ring_program()?.image()
}

/// The colour codes [`crate::ppm_grid`] prints for a grid: the width, the
/// height, then one code per codel in row-major order.
///
/// This is the ring image's input, so it is the seam between the two stages.
pub fn grid_codes(grid: &CodelGrid) -> String {
    let mut text = format!("{} {}", grid.width(), grid.height());
    for color in grid.codels() {
        let code = Color::ALL
            .iter()
            .position(|candidate| candidate == color)
            .unwrap_or(0);
        text.push_str(&format!(" {code}"));
    }
    text
}

/// The stream [`crate::ring_stream`] builds, printed by [`ring_grid`] itself.
///
/// The input is [`grid_codes`] — what [`crate::ppm_grid`] prints — and the
/// output is what the splitter and the movement stages take, so this is the
/// host side of the stage: run the image, hand the text on.
pub fn ring_to_stream(codes: &str) -> Result<String, Error> {
    /// One lap reads at most one code and prints four values; this is ample.
    const STEPS: u64 = 20_000_000;

    let image = Blocks::partition(ring_grid()?);
    let mut io = BufferIo::new(codes);
    let options = Options {
        max_steps: Some(STEPS),
        ..Options::default()
    };
    let result = interp::run(&image, &mut io, &options)?;
    if result.reason != ExitReason::Terminated {
        return Err(Error::StepLimit(STEPS));
    }
    Ok(io.into_output())
}

fn blocks() -> Vec<Block> {
    let add = Node::Command(Command::Add);
    let subtract = Node::Command(Command::Subtract);
    let multiply = Node::Command(Command::Multiply);
    let divide = Node::Command(Command::Divide);
    let modulo = Node::Command(Command::Modulo);
    let not = Node::Command(Command::Not);
    let pop = Node::Command(Command::Pop);
    let out = Node::Command(Command::OutNumber);
    let out_char = Node::Command(Command::OutChar);
    let space = || vec![Node::Const(32), out_char];

    let mut blocks: Vec<Block> = Vec::with_capacity(HALT as usize);

    // [w h] -> [i h w -1], with i = 0, printing the ring's size on the way. The
    // sentinel goes in first so that it sits under every record.
    blocks.push(Block::new(
        [vec![
            Node::Command(Command::InNumber),
            Node::Command(Command::InNumber),
            Node::DupLocal(2),
            Node::Const(2),
            add,
            Node::DupLocal(2),
            Node::Const(2),
            add,
            multiply,
            Node::Dup,
            out,
            pop,
            Node::Const(0),
            Node::Const(-1),
            Node::Roll(4, 1),
        ]]
        .concat(),
        End::Jump(LOOP),
    ));

    // [i h w ..]: stop when the index has reached the ring's size.
    blocks.push(Block::new(
        vec![
            Node::DupLocal(3),
            Node::Const(2),
            add,
            Node::DupLocal(3),
            Node::Const(2),
            add,
            multiply,
            Node::DupLocal(2),
            subtract,
            not,
        ],
        End::If {
            then: SENTINEL,
            otherwise: BODY,
        },
    ));

    // [i h w ..]: row = i / (w + 2), column = i % (w + 2), and the guard test
    // leaves one value that is zero for a guard.
    blocks.push(Block::new(
        vec![
            Node::DupLocal(3),
            Node::Const(2),
            add,
            Node::DupLocal(2),
            Node::Roll(2, 1),
            divide, // row = i / (w + 2)      [row i h w]
            Node::DupLocal(2),
            Node::DupLocal(5),
            Node::Const(2),
            add,
            modulo, // column = i % (w + 2)   [col row i h w]
            Node::DupLocal(5),
            Node::Const(2),
            add,
            Node::DupLocal(2),
            Node::Const(1),
            add,
            Node::Roll(2, 1),
            subtract,          // col - (w + 2) = -(col - w - 1)
            multiply,          // col * (col - w - 1) = c
            Node::Roll(2, 1),  // [row c i h w]
            Node::Dup,         // row, kept for the next product
            Node::DupLocal(5), // h
            Node::Const(1),
            add,
            Node::Roll(2, 1),
            subtract, // (h + 1) - row = -(row - h - 1)
            multiply, // row * (row - h - 1)
            multiply, // row * (row - h - 1) * c
            not,
        ],
        End::If {
            then: GUARD,
            otherwise: CODEL,
        },
    ));

    // [i h w ..] -> [.. 22+i i h w ..]
    blocks.push(Block::new(
        vec![Node::Dup, Node::Const(GUARD_CODE), add],
        End::Jump(PUSH),
    ));
    // [i h w ..] -> [.. code i h w ..]
    blocks.push(Block::new(
        vec![Node::Command(Command::InNumber)],
        End::Jump(PUSH),
    ));

    // [colour i h w ..]: push index, colour, stride, ring, then advance the
    // index and roll the counters back above everything the ring has pushed.
    blocks.push(Block::new(
        vec![
            Node::DupLocal(2),
            Node::Roll(2, 1),  // index goes under colour
            Node::DupLocal(5), // w
            Node::Const(2),
            add,
            Node::Const(4),
            multiply,          // stride
            Node::Dup,         // one for the record, one for the ring
            Node::DupLocal(6), // h
            Node::Const(2),
            add,
            multiply,         // ring = stride * (h + 2)
            Node::Roll(5, 4), // the index to the top, past the whole record
            Node::Const(1),
            add,
            Node::Roll(5, 1), // the record under the counters
            Node::Roll(7, 4), // and the counters back above it
        ],
        End::Jump(LOOP),
    ));

    // [i h w ..]: the counters have done their work.
    blocks.push(Block::new(vec![pop, pop, pop], End::Jump(PRINT)));

    // [.. value]: -1 ends the stream, anything else is a record's ring size.
    blocks.push(Block::new(
        vec![Node::Dup, Node::Const(-1), subtract, not],
        End::If {
            then: HALT,
            otherwise: EMIT,
        },
    ));

    // [.. ring stride colour index]: one record, space-separated.
    blocks.push(Block::new(
        [
            space(),
            vec![out],
            space(),
            vec![out],
            space(),
            vec![out],
            space(),
            vec![out],
        ]
        .concat(),
        End::Jump(PRINT),
    ));

    blocks.push(Block::new(vec![], End::Halt));

    assert_eq!(blocks.len() as u32, HALT, "the layout constants must match");
    blocks
}
