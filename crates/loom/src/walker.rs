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
//! This file is the control path only. It loads the ring, reports each record's
//! command, and jumps. The handlers — the commands' effects on a value stack —
//! are the next piece, and they are the hard one: a roll always starts at the
//! top, so a program ring and a mutable value stack cannot both be at the top of
//! one stack.

use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};

/// Cells per record: command, block size, offset.
pub const RECORD: u32 = 3;

/// Where a record's command sits, counted from the top, when the head is on it.
pub const COMMAND_DEPTH: u32 = RECORD;

/// The skeleton walker: load `ring` cells, then follow the records, reporting
/// each command code as a decimal number.
///
/// The ring must hold at least two whole records. The offsets are not checked;
/// an offset that is not a multiple of [`RECORD`] leaves the head misaligned,
/// which the walker cannot detect and the handlers will not survive.
pub fn skeleton(ring: u32) -> Result<Program, Error> {
    if ring < 2 * RECORD || !ring.is_multiple_of(RECORD) {
        return Err(Error::Ring { cells: ring });
    }
    let cells = i64::from(ring);

    Program::new(
        vec![
            // 1: the read counter starts at zero
            Block::new(vec![Node::Const(0)], End::Jump(2)),
            // 2: while the counter is not the whole ring
            Block::new(
                vec![
                    Node::Dup,
                    Node::Const(cells),
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
            Block::new(vec![Node::Command(Command::Pop)], End::Jump(5)),
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
        ],
        1,
    )
}
