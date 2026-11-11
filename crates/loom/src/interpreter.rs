//! The Piet-in-Piet interpreter.
//!
//! The image is a ring with a dispatch bus along the top:
//!
//! ```text
//!   S H [push][sub][dup][not][pointer] ... [push][pointer]
//!   | |   |     |                      ...            |
//!   | |   +-----+-- descent lanes ----...            |
//!   | |          (one per opcode)                     |
//!   | +----- ascent ----------------------------------+
//!   |                |
//!   +----------------+---- lane -----------------------+
//! ```
//!
//! `H` is the loop head. It is entered from the launcher `S` at start and from
//! the ascent at the end of every lap, and both transitions run `in number`, so
//! each lap reads exactly one opcode. The fifteen tests each subtract one,
//! duplicate, negate and branch with `pointer`: a match turns the direction
//! pointer down into that opcode's lane, no match walks on to the next test.
//!
//! A lane's first two transitions pop the zero the test left behind and run the
//! opcode itself, so the interpreter's own stack *is* the interpreted program's
//! stack. The rest of the lane is stack-neutral filler that walks the colour
//! cycle until the shared return path can be entered.
//!
//! The return path is one wide block: entering it runs one filler command, and
//! travelling left through it is free. It exits into a single codel, turns up
//! into the ascent, and re-enters `H`. Every block on the path is one codel
//! wide, so the codel chooser never changes which codel a block leaves from;
//! only the codel counts, and they are all one.
//!
//! Running out of tests is the halt: two more cells push and rotate, and a
//! three-codel bar entered above its middle codel has no exit at all.

use canvas::{Blocks, CodelGrid, Color, Point};
use tessera::Command;

use crate::error::Error;
use crate::paint::Canvas;
use crate::program::OPCODES;
use crate::shade::{Shade, neutral_filler};

/// The launcher: the upper-left codel, so the program starts here.
const START_CELL: (u32, u32) = (0, 0);
/// The loop head, where every lap begins.
const HEAD_X: u32 = 1;
/// The column the ascent returns up.
const ASCENT_X: u32 = 1;
/// The first test's `push` cell.
const FIRST_TEST_X: u32 = 2;
/// Tests are five cells apart: push, subtract, duplicate, not, pointer.
const TEST_STRIDE: u32 = 5;
/// Padding between the last test and the halt branch.
const HALT_FILL: u32 = 13;
/// The shortest and longest lane rows the search tries.
const MIN_LANE: u32 = 6;
const MAX_LANE: u32 = 48;

/// The x of a test's `push` cell; tests are numbered from one.
const fn test_x(test: u32) -> u32 {
    FIRST_TEST_X + TEST_STRIDE * (test - 1)
}

/// The x of a test's `pointer` cell, which is also its lane's column.
const fn lane_x(test: u32) -> u32 {
    test_x(test) + 4
}

/// The x of the `push` cell of the halt branch.
const HALT_PUSH_X: u32 = lane_x(OPCODES.len() as u32) + 1 + HALT_FILL;
/// The x of the halt branch's `pointer`, which is also the lane's column.
const HALT_POINTER_X: u32 = HALT_PUSH_X + 1;
/// The canvas is wide enough for the lane to run past the terminator.
const WIDTH: u32 = 96;

/// Builds the interpreter image.
pub fn interpreter() -> Result<CodelGrid, Error> {
    for start in Shade::ALL {
        for lane in Shade::ALL {
            for lane_row in MIN_LANE..=MAX_LANE {
                if let Ok(grid) = build(start, lane, lane_row)
                    && blocks_are_clean(&grid, lane_row)
                {
                    return Ok(grid);
                }
            }
        }
    }
    Err(Error::NoLayout)
}

/// The command that runs an opcode. Every opcode runs itself except `push`,
/// whose operand is the block size the interpreter reads from its input.
const fn handler(opcode: Command) -> Command {
    match opcode {
        Command::Push => Command::InNumber,
        other => other,
    }
}

/// The dispatch bus, in execution order: everything after the loop head, up to
/// and including the halt branch.
fn bus_commands() -> Result<Vec<Command>, Error> {
    let mut commands = Vec::new();
    for _ in 0..OPCODES.len() {
        commands.extend([
            Command::Push,
            Command::Subtract,
            Command::Duplicate,
            Command::Not,
            Command::Pointer,
        ]);
    }
    let mut filler = neutral_filler(Shade::new(0, 0), Shade::new(0, 0), HALT_FILL as usize)?;
    commands.append(&mut filler);
    commands.push(Command::Push);
    commands.push(Command::Pointer);
    Ok(commands)
}

/// Lays out one interpreter with a given start colour, lane colour and lane
/// row.
fn build(start: Shade, lane: Shade, lane_row: u32) -> Result<CodelGrid, Error> {
    if lane_row < 4 {
        return Err(Error::NoLayout);
    }
    let head = start.advance(Command::InNumber);
    let bus = bus_commands()?;

    // Cell x on the bus is `shades[x - 1]`: the head is at x = 1.
    let mut shades = Vec::with_capacity(bus.len() + 1);
    let mut shade = head;
    shades.push(head);
    for command in &bus {
        shade = shade.advance(*command);
        shades.push(shade);
    }

    // The shared return path. Entering the lane block pushes a one, which the
    // first ascent codel pops; the rest of the ascent is neutral filler from
    // that colour to the loop head's predecessor colour.
    let lane_entry = Command::Push;
    let above_lane = lane.before(lane_entry);
    let first_ascent = lane.advance(Command::Pop);
    let ascent = neutral_filler(first_ascent, start, (lane_row - 1) as usize)?;

    let mut canvas = Canvas::new(WIDTH, lane_row + 1)?;
    canvas.put(START_CELL.0, START_CELL.1, start.color())?;
    for (offset, shade) in shades.iter().enumerate() {
        canvas.put(HEAD_X + offset as u32, 0, shade.color())?;
    }

    for (index, opcode) in OPCODES.iter().enumerate() {
        let test = index as u32 + 1;
        let column = lane_x(test);
        let pointer = shades[(column - 1) as usize];
        let first = pointer.advance(Command::Pop);
        let second = first.advance(handler(*opcode));
        let filler = neutral_filler(second, above_lane, (lane_row - 3) as usize)?;

        canvas.put(column, 1, first.color())?;
        canvas.put(column, 2, second.color())?;
        let mut shade = second;
        for (offset, command) in filler.iter().enumerate() {
            shade = shade.advance(*command);
            canvas.put(column, 3 + offset as u32, shade.color())?;
        }
    }

    canvas.put_block(2, lane_row, WIDTH - 2, 1, lane.color())?;
    canvas.put(ASCENT_X, lane_row, first_ascent.color())?;
    let mut shade = first_ascent;
    for (offset, command) in ascent.iter().enumerate() {
        shade = shade.advance(*command);
        canvas.put(ASCENT_X, lane_row - 1 - offset as u32, shade.color())?;
    }

    let halt = shades[(HALT_POINTER_X - 1) as usize];
    let mut shade = halt.advance(Command::Pop);
    canvas.put(HALT_POINTER_X, 1, shade.color())?;
    let filler = neutral_filler(shade, shade, (lane_row - 4) as usize)?;
    for (offset, command) in filler.iter().enumerate() {
        shade = shade.advance(*command);
        canvas.put(HALT_POINTER_X, 2 + offset as u32, shade.color())?;
    }

    let terminator = shade.advance(Command::Push);
    canvas.put_block(HALT_POINTER_X - 1, lane_row - 2, 3, 1, terminator.color())?;
    canvas.into_grid()
}

/// Whether the partition really is one block per intended run: single codels
/// everywhere except the shared lane and the terminator bar.
fn blocks_are_clean(grid: &CodelGrid, lane_row: u32) -> bool {
    let blocks = Blocks::partition(grid.clone());
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            let point = Point::new(x, y);
            let Some(color) = grid.get(point) else {
                continue;
            };
            if color == Color::Black {
                continue;
            }
            let expected = if y == lane_row && x >= 2 {
                (WIDTH - 2) as usize
            } else if y + 2 == lane_row && (HALT_POINTER_X - 1..=HALT_POINTER_X + 1).contains(&x) {
                // The terminator bar is three codels wide.
                3
            } else {
                1
            };
            let Some(block) = blocks.block_at(point) else {
                return false;
            };
            if block.size() != expected {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_ends_on_the_halt_branch() {
        let commands = bus_commands().expect("a bus");
        assert_eq!(commands.len(), 5 * OPCODES.len() + HALT_FILL as usize + 2);
        assert_eq!(commands.first(), Some(&Command::Push));
        assert_eq!(commands.last(), Some(&Command::Pointer));
    }

    #[test]
    fn the_layout_leaves_a_lane_for_every_opcode() {
        for test in 1..=OPCODES.len() as u32 {
            assert!(lane_x(test) + 1 < HALT_PUSH_X);
        }
    }

    #[test]
    fn the_interpreter_builds_a_clean_partition() {
        let grid = interpreter().expect("an interpreter");
        assert_eq!(grid.width(), WIDTH);
        assert!(grid.height() > MIN_LANE);
        let blocks = Blocks::partition(grid);
        assert!(blocks.len() > 20);
    }

    #[test]
    fn the_interpreter_is_a_real_png() {
        let grid = interpreter().expect("an interpreter");
        let png = grid.encode_png().expect("a PNG");
        let decoded = CodelGrid::from_image(&png).expect("the PNG decodes");
        assert_eq!(decoded.codels(), grid.codels());
    }
}
