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
//! # The stack
//!
//! ```text
//!   [ value 1 … value k | depth | record 0 … ]      steady state
//!   [ value 1 … value k | depth | head record ]     the head rotates
//! ```
//!
//! Two properties make this work.
//!
//! The **ring is on top**, so the head's three fields are at depths 1, 2 and 3
//! whatever the value stack holds. The **depth counter sits between the values
//! and the ring**, so a jump — which rolls whole records — leaves it at depth
//! `N+1`, and so does every value access, because a value inserted at the top of
//! the value stack goes just below it. That is why the counter can be a
//! register rather than a fourth record cell: a record's cell would move with
//! the head, and the depth is global.
//!
//! `roll` only reaches the top, so an access rotates the ring past the values:
//!
//! | operation | commands |
//! |---|---|
//! | push (insert below the depth) | `roll(N+2, 1)` |
//! | unary (bring one value up) | `roll(N+2, N+1)` |
//! | binary (bring two up, order kept) | `roll(N+3, N+1)` |
//! | put a result back | `roll(N+2, 1)` |
//! | duplicate | `roll(N+2, N+1)`, `dup`, `roll(N+3, 1)`, `pop` |
//! | read or write the depth | `roll(N+1, N)`, …, `roll(N+1, 1)` |
//!
//! Every amount is a multiple of [`RECORD`], so the head stays on a record
//! boundary.
//!
//! # Underflow
//!
//! A command that needs more values than the stack holds is *ignored*, as in the
//! reference interpreter: the runtime would ignore the over-deep roll instead,
//! and the handler would then pop a ring cell. So each handler is guarded by two
//! blocks — a test on the depth counter, then the body — and a failed test jumps
//! past the command. `push` is the one handler with no guard.
//!
//! `divide` and `modulo` need one more branch: a zero divisor consumes both
//! operands and pushes nothing, so the depth falls by two rather than one, and
//! the handler has to know which happened. It tests the divisor, and the two
//! paths differ only in the amount they subtract from the counter.
//!
//! Two commands have no handler and are refused when the ring is built: `roll`
//! (which rotates the *values*: the lift works, but putting them back needs the
//! value count after the operands are gone — see `.agents/notes/piet-in-piet-full.md`)
//! and the two input commands (whose input stream is not carried after the ring
//! yet). A ring built by hand that names one of them stops at that record.

use canvas::CodelGrid;
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};
use crate::program::{Instruction, opcode};

/// Cells per record: command, block size, offset.
pub const RECORD: u32 = 3;

/// Where a record's command sits, counted from the top, when the head is on it.
pub const COMMAND_DEPTH: u32 = RECORD;

/// The value-depth register, counted from the top, when the head is on record
/// zero.
const DEPTH_DEPTH: u32 = 1;

/// The block that runs once the ring is loaded.
const DISPATCH: u32 = 5;
/// The block that turns the command code into a guard id.
const SELECT: u32 = 6;
/// The block a zero command stops at, and the base of the guard table.
const HALT: u32 = 7;
/// How many command codes have a block in the table.
const CODES: u32 = 15;
/// `guard(c)` is `HALT + c`; `body(c)` is `BODY_BASE + c`.
const BODY_BASE: u32 = HALT + CODES;
/// The four blocks behind `divide` and `modulo`'s zero-divisor branch.
const EXTRA_BASE: u32 = BODY_BASE + CODES + 1;
const DIVIDE_NORMAL: u32 = EXTRA_BASE;
const DIVIDE_ZERO: u32 = EXTRA_BASE + 1;
const MODULO_NORMAL: u32 = EXTRA_BASE + 2;
const MODULO_ZERO: u32 = EXTRA_BASE + 3;
/// The block that rolls the ring by the head's offset.
const JUMP: u32 = EXTRA_BASE + 4;

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
    let mut blocks = load_loop(ring, 0);
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
/// has nowhere to get it, which is why the input commands are refused. Commands
/// the walker cannot run are refused by [`Error::NoHandler`].
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

    let mut blocks = load_loop(cells, 1);
    blocks.extend([
        // 5: a zero command halts; anything else is a guard
        Block::new(
            vec![Node::DupLocal(COMMAND_DEPTH)],
            End::If {
                then: SELECT,
                otherwise: HALT,
            },
        ),
        // 6: the guard table is contiguous from HALT, so the code indexes it
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
        blocks.push(guard(code, cells));
    }
    for code in 1..=CODES {
        blocks.push(body(code, cells));
    }
    blocks.extend([
        // 38, 39: divide with a divisor that is not zero, and one that is
        Block::new(division(Command::Divide, cells, true), End::Jump(JUMP)),
        Block::new(division(Command::Divide, cells, false), End::Jump(JUMP)),
        // 40, 41: the same for modulo
        Block::new(division(Command::Modulo, cells, true), End::Jump(JUMP)),
        Block::new(division(Command::Modulo, cells, false), End::Jump(JUMP)),
        // 42: roll the ring by the head's offset
        Block::new(
            vec![
                Node::Const(i64::from(cells)),
                Node::DupLocal(2),
                Node::Command(Command::Roll),
            ],
            End::Jump(DISPATCH),
        ),
    ]);

    let program = Program::new(blocks, 1)?;
    Ok((program.image()?, stream))
}

/// The four blocks that read `cells` numbers into the ring, leaving the head on
/// record zero.
///
/// `registers` zeroes are pushed first, so they end up below the ring. Block 1
/// is the entry and block 4 hands over with [`DISPATCH`].
fn load_loop(cells: u32, registers: u32) -> Vec<Block> {
    let total = i64::from(cells);
    let mut start = vec![Node::Const(0); registers as usize];
    // The read counter goes on top of the registers.
    start.push(Node::Const(0));
    vec![
        // 1: the registers, then the read counter, start at zero
        Block::new(start, End::Jump(2)),
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

/// The number of values a command needs before it does anything.
fn needed(code: u32) -> u32 {
    match code {
        1 => 0,
        3..=7 | 9 => 2,
        _ => 1,
    }
}

/// A guard block: test the depth counter, run `body(code)`, or skip the command.
fn guard(code: u32, cells: u32) -> Block {
    let body = BODY_BASE + code;
    let needed = needed(code);
    if needed == 0 {
        return Block::new(Vec::new(), End::Jump(body));
    }
    Block::new(
        vec![
            Node::DupLocal(cells + DEPTH_DEPTH),
            Node::Const(i64::from(needed) - 1),
            Node::Command(Command::Greater),
        ],
        End::If {
            then: body,
            otherwise: JUMP,
        },
    )
}

/// How a handler changes the depth counter.
#[derive(Clone, Copy)]
enum Depth {
    Same,
    Plus(u32),
    Minus(u32),
}

/// A handler block: apply the command of code `code` to the value stack and keep
/// the depth counter true.
///
/// Codes 11 to 13 are `roll`, `in number` and `in char`, which the walker cannot
/// run. They get a halt rather than a gap, because the table has to stay
/// contiguous for the code to index it.
fn body(code: u32, cells: u32) -> Block {
    let insert = Node::Roll(cells + 2, 1);
    let up_one = Node::Roll(cells + 2, cells + 1);
    let up_two = Node::Roll(cells + 3, cells + 1);
    let mut nodes = match code {
        1 => vec![Node::DupLocal(2), insert],
        2 => vec![up_one, Node::Command(Command::Pop)],
        3 => vec![up_two, Node::Command(Command::Add), insert],
        4 => vec![up_two, Node::Command(Command::Subtract), insert],
        5 => vec![up_two, Node::Command(Command::Multiply), insert],
        // divide and modulo branch on the divisor; see `division`
        6 => {
            return Block::new(
                vec![
                    up_two,
                    Node::Command(Command::Duplicate),
                    Node::Command(Command::Not),
                ],
                End::If {
                    then: DIVIDE_ZERO,
                    otherwise: DIVIDE_NORMAL,
                },
            );
        }
        7 => {
            return Block::new(
                vec![
                    up_two,
                    Node::Command(Command::Duplicate),
                    Node::Command(Command::Not),
                ],
                End::If {
                    then: MODULO_ZERO,
                    otherwise: MODULO_NORMAL,
                },
            );
        }
        8 => vec![up_one, Node::Command(Command::Not), insert],
        9 => vec![up_two, Node::Command(Command::Greater), insert],
        10 => vec![
            up_one,
            Node::Command(Command::Duplicate),
            Node::Roll(cells + 3, 2),
        ],
        14 => vec![up_one, Node::Command(Command::OutNumber)],
        15 => vec![up_one, Node::Command(Command::OutChar)],
        _ => return Block::new(Vec::new(), End::Halt),
    };
    let depth = match code {
        1 | 10 => Depth::Plus(1),
        8 => Depth::Same,
        _ => Depth::Minus(1),
    };
    adjust(&mut nodes, depth, cells);
    Block::new(nodes, End::Jump(JUMP))
}

/// The two paths behind `divide` and `modulo`: a zero divisor consumes both
/// operands and pushes nothing, so the depth falls by two.
fn division(command: Command, cells: u32, non_zero: bool) -> Vec<Node> {
    let insert = Node::Roll(cells + 2, 1);
    let mut nodes = if non_zero {
        vec![Node::Command(command), insert]
    } else {
        vec![Node::Command(Command::Pop), Node::Command(Command::Pop)]
    };
    adjust(
        &mut nodes,
        if non_zero {
            Depth::Minus(1)
        } else {
            Depth::Minus(2)
        },
        cells,
    );
    nodes
}

/// Appends the commands that bring the depth counter by `change`.
///
/// The counter is the top of the register block: roll it up, change it, and roll
/// it back. Both rolls are [`RECORD`] apart, so the head stays aligned.
fn adjust(nodes: &mut Vec<Node>, change: Depth, cells: u32) {
    let (amount, operation) = match change {
        Depth::Same => return,
        Depth::Plus(amount) => (i64::from(amount), Command::Add),
        Depth::Minus(amount) => (i64::from(amount), Command::Subtract),
    };
    nodes.extend([
        Node::Roll(cells + 1, cells),
        Node::Const(amount),
        Node::Command(operation),
        Node::Roll(cells + 1, 1),
    ]);
}

/// The opcode of a command the walker has a handler for.
fn code_of(command: Command) -> Option<u8> {
    match command {
        Command::Roll | Command::InNumber | Command::InChar => None,
        other => opcode(other),
    }
}
