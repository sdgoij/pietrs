//! The fixed-image walker: a program that lives in memory instead of in the
//! image.
//!
//! Every other generated image bakes the program into its own geometry, so a
//! fifteen-thousand-state program needs an impossibly wide bus. Here the
//! program is data: a ring of records on the stack, with the head at the top. A
//! record is
//!
//! ```text
//!   [ command, block size, offset ]
//! ```
//!
//! and, at the head, those are the top three cells — the offset on top, the
//! command three down. A jump is one `roll` of the ring by the record's offset,
//! so there is no program counter to keep: the head is the program counter.
//!
//! The stream is read **highest record first**, so the last record read is
//! record zero and it ends up on top of the stack. That is what makes the
//! offsets plain: with the head on record `i`, a roll of `3 * k` moves the head
//! forward `k` records, and a roll of `-3 * k` moves it back. Every offset must
//! be a multiple of [`RECORD`], because a roll that does not land on a record
//! boundary leaves the head straddling two records.
//!
//! ```text
//!   stream:  r[n-1]  r[n-2]  …  r[1]  r[0]
//!   stack:  [ r[n-1] | r[n-2] | … | r[1] | r[0] ]   r[0] on top, the head
//! ```
//!
//! [`skeleton`] is the control path alone: it loads the ring, reports each
//! record's command, and jumps. [`walker`] adds the handlers, and with them the
//! other half of the memory model.
//!
//! # The value stack
//!
//! The interpreted program's values live **below** the ring, so the ring is
//! always the top [`RECORD`] cells and the head's fields are at fixed depths.
//! The cost is that `roll` only reaches the top, so every access to the value
//! stack rotates the whole ring past the values:
//!
//! | operation | commands |
//! |---|---|
//! | push (insert below the ring) | `roll(N+1, 1)` |
//! | unary (bring one value up) | `roll(N+1, N)` |
//! | binary (bring two up, order kept) | `roll(N+2, N)` |
//! | put a result back | `roll(N+1, 1)` |
//! | duplicate | `roll(N+1, N)`, `dup`, `roll(N+2, 2)` |
//!
//! The rotations are of the ring, so they keep its cyclic order and its
//! alignment: a roll by a multiple of [`RECORD`] leaves the head on a record
//! boundary. That is why `N` appears as the roll amount rather than `N mod 3`.
//!
//! Two value-stack operations do not fit this model and are refused when the
//! ring is built: `roll` (which needs a dynamic rotation of the *values*) and
//! the two input commands (whose input stream is not carried after the ring
//! yet). A ring built by hand that names one of them stops at that record.
//!
//! A command that empties the value stack **diverges** here, and this is the
//! sharpest known gap. The runtime ignores a `roll` whose depth is deeper than
//! the stack, so a handler that reaches for a value that is not there does not
//! fail: the roll is a no-op and the handler then pops a ring cell. `interp`
//! ignores the underflowing command instead. Closing this needs a depth counter
//! in the record — a fourth cell, rolled with the ring and therefore at a fixed
//! depth — that each handler can test before it reaches. That is the next piece.
//!

use canvas::CodelGrid;
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};
use crate::program::{Instruction, opcode};

/// Cells per record: command, block size, offset.
pub const RECORD: u32 = 3;

/// Where a record's command sits, counted from the top, when the head is on it.
pub const COMMAND_DEPTH: u32 = RECORD;

/// The block that runs once the ring is loaded.
const DISPATCH: u32 = 5;
/// The block that turns the command code into a handler id.
const SELECT: u32 = 6;
/// The block a zero command stops at, and the base of the handler table.
const HALT: u32 = 7;
/// How many command codes have a handler slot.
const CODES: u32 = 15;
/// The block that rolls the ring by the head's offset.
const JUMP: u32 = HALT + CODES + 1;

/// The skeleton walker: load `ring` cells, then follow the records, reporting
/// each command code as a decimal number.
///
/// The ring must hold at least one whole record. A single record can only halt.
/// The offsets are not checked; an offset that is not a multiple of [`RECORD`]
/// leaves the head misaligned, which the walker cannot detect.
pub fn skeleton(ring: u32) -> Result<Program, Error> {
    if ring < RECORD || !ring.is_multiple_of(RECORD) {
        return Err(Error::Ring { cells: ring });
    }
    let cells = i64::from(ring);
    let mut blocks = load_loop(ring);
    blocks.extend([
        // 5: report the head's command; zero halts
        Block::new(
            vec![
                Node::DupLocal(COMMAND_DEPTH),
                Node::Dup,
                Node::Command(Command::OutNumber),
                Node::Const(32),
                Node::Command(Command::OutChar),
                Node::Command(Command::Not),
            ],
            End::If {
                then: 6,
                otherwise: 7,
            },
        ),
        // 6: done
        Block::new(Vec::new(), End::Halt),
        // 7: roll the ring by the head's offset
        Block::new(
            vec![
                Node::Const(cells),
                Node::DupLocal(2),
                Node::Command(Command::Roll),
            ],
            End::Jump(5),
        ),
    ]);
    Program::new(blocks, 1)
}

/// A walker image and the stream that loads its ring.
///
/// The stream carries the records only; an interpreted program that reads input
/// has nowhere to get it, which is why [`Instruction::command`] cannot be one of
/// the input commands. Commands the walker cannot run are refused by
/// [`Error::NoHandler`].
pub fn walker(instructions: &[Instruction]) -> Result<(CodelGrid, String), Error> {
    let mut records = Vec::with_capacity(instructions.len() + 1);
    for instruction in instructions {
        let code = code_of(instruction.command).ok_or(Error::NoHandler {
            command: instruction.command,
        })?;
        let size = i64::try_from(instruction.block_size).map_err(|_| Error::BlockTooLarge {
            operand: instruction.block_size,
        })?;
        records.push((i64::from(code), size, i64::from(RECORD)));
    }
    records.push((0, 0, 0));

    let count = u32::try_from(records.len()).map_err(|_| Error::TooLarge)?;
    let cells = count.checked_mul(RECORD).ok_or(Error::TooLarge)?;

    let mut stream = String::new();
    for (command, size, offset) in records.iter().rev() {
        stream.push_str(&format!("{command} {size} {offset} "));
    }

    let mut blocks = load_loop(cells);
    blocks.extend([
        // 5: a zero command halts; anything else is a handler
        Block::new(
            vec![Node::DupLocal(COMMAND_DEPTH)],
            End::If {
                then: SELECT,
                otherwise: HALT,
            },
        ),
        // 6: the handler table is contiguous from HALT, so the code indexes it
        Block::new(
            vec![
                Node::DupLocal(COMMAND_DEPTH),
                Node::Const(i64::from(HALT)),
                Node::Command(Command::Add),
            ],
            End::Computed,
        ),
        // 7: done
        Block::new(Vec::new(), End::Halt),
    ]);
    for code in 1..=CODES {
        blocks.push(handler(code, cells));
    }
    // 23: roll the ring by the head's offset
    blocks.push(Block::new(
        vec![
            Node::Const(i64::from(cells)),
            Node::DupLocal(2),
            Node::Command(Command::Roll),
        ],
        End::Jump(DISPATCH),
    ));

    let program = Program::new(blocks, 1)?;
    Ok((program.image()?, stream))
}

/// The four blocks that read `cells` numbers into the ring, leaving the head on
/// record zero.
///
/// Block 1 is the entry and block 4 hands over with [`DISPATCH`].
fn load_loop(cells: u32) -> Vec<Block> {
    let total = i64::from(cells);
    vec![
        // 1: the read counter starts at zero
        Block::new(vec![Node::Const(0)], End::Jump(2)),
        // 2: while the counter is not the whole ring
        Block::new(
            vec![
                Node::Dup,
                Node::Const(total),
                Node::Command(Command::Subtract),
                Node::Command(Command::Not),
            ],
            End::If {
                then: 4,
                otherwise: 3,
            },
        ),
        // 3: read a cell under the counter, then advance
        Block::new(
            vec![
                Node::Command(Command::InNumber),
                Node::Swap,
                Node::Const(1),
                Node::Command(Command::Add),
            ],
            End::Jump(2),
        ),
        // 4: drop the counter; the last record read is now the head
        Block::new(vec![Node::Command(Command::Pop)], End::Jump(DISPATCH)),
    ]
}

/// One handler block: apply the command of code `code` to the value stack, then
/// jump to the next record.
///
/// Codes 11 to 13 are `roll`, `in number` and `in char`, which the walker cannot
/// run. They get a halt rather than a gap, because the handler table has to stay
/// contiguous for the code to index it.
fn handler(code: u32, cells: u32) -> Block {
    let up_one = Node::Roll(cells + 1, cells);
    let up_two = Node::Roll(cells + 2, cells);
    let insert = Node::Roll(cells + 1, 1);
    let nodes = match code {
        // push: the record carries the block size the interpreted push pushes
        1 => vec![Node::DupLocal(2), insert],
        2 => vec![up_one, Node::Command(Command::Pop)],
        3 => vec![up_two, Node::Command(Command::Add), insert],
        4 => vec![up_two, Node::Command(Command::Subtract), insert],
        5 => vec![up_two, Node::Command(Command::Multiply), insert],
        6 => vec![up_two, Node::Command(Command::Divide), insert],
        7 => vec![up_two, Node::Command(Command::Modulo), insert],
        8 => vec![up_one, Node::Command(Command::Not), insert],
        9 => vec![up_two, Node::Command(Command::Greater), insert],
        // duplicate: bringing the top value up leaves it above the ring, so a
        // copy is made there and both go back below
        10 => vec![
            up_one,
            Node::Command(Command::Duplicate),
            Node::Roll(cells + 2, 2),
        ],
        14 => vec![up_one, Node::Command(Command::OutNumber)],
        15 => vec![up_one, Node::Command(Command::OutChar)],
        _ => return Block::new(Vec::new(), End::Halt),
    };
    Block::new(nodes, End::Jump(JUMP))
}

/// The opcode of a command the walker has a handler for.
fn code_of(command: Command) -> Option<u8> {
    match command {
        Command::Roll | Command::InNumber | Command::InChar => None,
        other => opcode(other),
    }
}
