//! The fixed-image walker: a program that lives in memory instead of in the
//! image.
//!
//! Every other generated image bakes the program into its own geometry, so a
//! fifteen-thousand-state program needs an impossibly wide bus. Here the
//! program is data: a ring of records on the stack, with the head at the top. A
//! record is
//!
//! ```text
//!   [ command, block size, ring cells, offset ]
//! ```
//!
//! and, at the head, those are the top four cells — the offset on top, the ring
//! size below it, the command four down. A jump is one `roll` of the ring by the
//! record's offset, so there is no program counter to keep: the head is the
//! program counter.
//!
//! **The ring size travels in every record, so one image serves every program.**
//! The alternative — weaving an image per program — bakes the size into every
//! roll amount, and a fixed capacity cannot work, because a jump is a roll taken
//! modulo the program's record count (see `.agents/notes/piet-in-piet-full.md`).
//! With the size at depth two, `dup_local(2)` reads it in a constant number of
//! commands, and every amount that used to be a constant `N + j` is built from
//! it.
//!
//! The stream is read **highest record first**, so the last record read is
//! record zero and it ends up on top of the stack. That is what makes the
//! offsets plain: with the head on record `i`, a roll of `4 * k` moves the head
//! forward `k` records, and a roll of `-4 * k` moves it back. Every offset must
//! be a multiple of [`RECORD`], because a roll that does not land on a record
//! boundary leaves the head straddling two records.
//!
//! ```text
//!   stream:  n  r[n-1]  r[n-2]  …  r[1]  r[0]
//!   stack:  [ r[n-1] | r[n-2] | … | r[1] | r[0] ]   r[0] on top, the head
//! ```
//!
//! The leading `n` is the ring's size in cells. [`load_loop`] reads it and then
//! counts down, so how large the ring is never reaches the image.
//!
//! [`skeleton`] is the control path alone: it loads the ring, reports each
//! record's command, and jumps. [`walker`] adds the handlers for a straight-line
//! program; [`graph_walker`] compiles a program's state graph into records, so
//! the image follows branches the host resolved and selects the ones the program
//! decides.
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
//! The **ring is on top**, so the head's fields are at depths 1 to 4 whatever the
//! value stack holds. The **depth counter sits between the values and the ring**,
//! so a jump — which rolls whole records — leaves it at depth `N+1`, and so does
//! every value access, because a value inserted at the top of the value stack
//! goes just below it. That is why the counter is a register below the ring
//! rather than a record cell: the depth is global, and a record's cell would move
//! with the head. The ring *size* is the opposite case — it must move with the
//! head to stay at a fixed depth, which is why it is a record cell.
//!
//! `roll` only reaches the top, so an access rotates the ring past the values:
//!
//! | operation | commands |
//! |---|---|
//! | push (insert below the depth) | `roll(N+2, 1)` |
//! | unary (bring one value up) | `roll(N+2, N+1)` |
//! | binary (bring two up, order kept) | `roll(N+3, N+1)` |
//! | put a result back | `roll(N+2, 1)` |
//! | duplicate | `roll(N+2, N+1)`, `dup`, `roll(N+3, 2)` |
//! | read or write the depth | `roll(N+1, N)`, …, `roll(N+1, 1)` |
//!
//! Every amount is a multiple of [`RECORD`], so the head stays on a record
//! boundary. All of them are built at run time now: [`push_size`], [`roll_size`]
//! and [`roll_with`] read `N` from the head's ring-size cell and add a literal
//! offset, so [`RECORD`] is the only constant left in an amount.
//!
//! # Underflow
//!
//! A command that needs more values than the stack holds is *ignored*, as in the
//! reference interpreter: the runtime would ignore the over-deep roll instead,
//! and the handler would then pop a ring cell. So each handler is guarded by two
//! blocks — a test on the depth counter, then the body — and a failed test jumps
//! past the command. `push` and the reads are the unguarded handlers.
//!
//! `divide` and `modulo` need one more branch: a zero divisor consumes both
//! operands and pushes nothing, so the depth falls by two rather than one, and
//! the handler has to know which happened. It tests the divisor, and the two
//! paths differ only in the amount they subtract from the counter.
//!
//! `pointer` and `switch` are the one case where an empty stack is not a skip:
//! the reference pops nothing and uses zero, so the guard's other side is a block
//! that branches to the first successor.
//!
//! # Input
//!
//! The interpreted program's input follows the ring in the same stream. A
//! straight-line program writes one space-prefixed **code point** per read, in
//! the order it runs them, and [`walker`] refuses a program whose reads outrun
//! the input. A character is written as its code point rather than as itself so
//! that the same handler serves a graph, whose reads are chosen at run time: for
//! [`graph_walker`] the input is whitespace-separated numbers, one per read.
//!
//! # Codes
//!
//! Records name one of the fifteen Piet commands, or one of three records the
//! graph builder needs: `nop` (run nothing and jump, used for branch
//! trampolines), `pointer` and `switch` (pop a value, branch by it modulo four
//! or two).
//!
//! # Roll
//!
//! `roll` is the one command whose *depth* decides the work, and a depth known
//! only at run time cannot compute its own put-back (see the note in
//! `.agents/notes/piet-in-piet-full.md`). What works is a depth known at weave
//! time, because then the lift, the rotation and the put-back are three
//! rotations of constant size:
//!
//! ```text
//!   roll(d + N + 3, N + 1)     lift the values and both operands above the ring
//!   roll                        rotate the d values by the rolls operand
//!   roll(d + N + 1, d)         put them back below the depth register
//! ```
//!
//! so the table has one case per depth from 2 to [`ROLL_DEPTH`], selected
//! arithmetically from the popped value. A roll deeper than the table stops the
//! walker: the alternative is rotating ring cells instead of values.

use std::collections::HashMap;

use canvas::{BlockId, Blocks, CodelGrid};
use tessera::{Branch, Command, State, StateGraph};

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};
use crate::program::{Feed, Instruction, OPCODES, opcode};

/// Cells per record: command, block size, ring cells, offset.
pub const RECORD: u32 = 4;

/// Where a record's command sits, counted from the top, when the head is on it.
pub const COMMAND_DEPTH: u32 = RECORD;

/// Where the ring size sits, counted from the top, when the head is on it.
const SIZE_DEPTH: u32 = 2;

/// The block that runs once the ring is loaded.
const DISPATCH: u32 = 5;
/// The block that turns the command code into a guard id.
const SELECT: u32 = 6;
/// The block a zero command stops at, and the base of the guard table.
const HALT: u32 = 7;
/// How many codes have a block in the table.
const CODES: u32 = 18;
/// Run nothing and jump: a branch trampoline, or a state with no command.
const NOP: u32 = 16;
/// Pop, then branch by the value modulo four.
const POINTER: u32 = 17;
/// Pop, then branch by the value modulo two.
const SWITCH: u32 = 18;
/// `guard(c)` is `HALT + c`; `body(c)` is `BODY_BASE + c`.
const BODY_BASE: u32 = HALT + CODES;
/// The blocks behind `divide`, `modulo`, the two branch handlers and `roll`.
const EXTRA_BASE: u32 = BODY_BASE + CODES + 1;
const DIVIDE_NORMAL: u32 = EXTRA_BASE;
const DIVIDE_ZERO: u32 = EXTRA_BASE + 1;
const MODULO_NORMAL: u32 = EXTRA_BASE + 2;
const MODULO_ZERO: u32 = EXTRA_BASE + 3;
const POINTER_ZERO: u32 = EXTRA_BASE + 4;
const SWITCH_ZERO: u32 = EXTRA_BASE + 5;

/// The deepest `roll` the table covers. A roll deeper than this stops the walker
/// rather than doing the wrong thing: the depth has to be known at weave time to
/// compute the put-back, and the table is one block per depth. Sixty-four covers
/// the longest string a program is likely to keep on the stack; Piet's Quest
/// needs 29 for a room description.
const ROLL_DEPTH: u32 = 64;
/// Case `d` is `ROLL_CASE_BASE + d`. The base sits below the table, so the two
/// shallowest values cannot index a case; they take the skip path instead.
const ROLL_CASE_BASE: u32 = EXTRA_BASE + 4;
/// `d > ROLL_DEPTH`: past what the table covers.
const ROLL_HIGH: u32 = ROLL_CASE_BASE + ROLL_DEPTH + 1;
/// `d > k - 2`: deeper than the value stack, so the reference ignores it.
const ROLL_FITS: u32 = ROLL_HIGH + 1;
/// Widen the depth into a case id.
const ROLL_SELECT: u32 = ROLL_HIGH + 2;
/// A skip that still has the depth copy on the stack.
const ROLL_SHALLOW: u32 = ROLL_HIGH + 3;
/// A skip whose depth copy the test consumed.
const ROLL_DEEP: u32 = ROLL_HIGH + 4;
/// A roll the table cannot run: the walker stops.
const ROLL_UNSUPPORTED: u32 = ROLL_HIGH + 5;
/// The block that rolls the ring by the head's offset.
const JUMP: u32 = ROLL_HIGH + 6;

/// A record: command code, block size, ring size in cells, offset in cells.
type Record = (i64, i64, i64, i64);

/// The skeleton walker: follow a ring of records, reporting each command code as
/// a decimal number.
///
/// The stream starts with the ring's size in cells and then the records; the
/// image does not know either in advance. The offsets are not checked; an offset
/// that is not a multiple of [`RECORD`] leaves the head misaligned, which the
/// walker cannot detect.
pub fn skeleton() -> Result<Program, Error> {
    let mut blocks = load_loop(0);
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
                Node::DupLocal(SIZE_DEPTH),
                Node::DupLocal(SIZE_DEPTH),
                Node::Command(Command::Roll),
            ],
            End::Jump(5),
        ),
    ]);
    Program::new(blocks, 1)
}

/// A walker image and the stream that loads its ring and feeds its input, for a
/// straight-line `instructions` sequence.
///
/// Commands the walker cannot run are refused by [`Error::NoHandler`]; a program
/// whose input reads outrun `input` is refused by [`Error::InputExhausted`].
pub fn walker(instructions: &[Instruction], input: &str) -> Result<(CodelGrid, String), Error> {
    let count = u32::try_from(instructions.len() + 1).map_err(|_| Error::TooLarge)?;
    let cells = cells(count)?;
    let mut records = Vec::with_capacity(instructions.len() + 1);
    for instruction in instructions {
        let code = code_of(instruction.command).ok_or(Error::NoHandler {
            command: instruction.command,
        })?;
        let size = i64::try_from(instruction.block_size).map_err(|_| Error::BlockTooLarge {
            operand: instruction.block_size,
        })?;
        records.push((i64::from(code), size, i64::from(cells), i64::from(RECORD)));
    }
    records.push((0, 0, i64::from(cells), 0));

    let mut stream = ring_stream(cells, &records);
    let mut feed = Feed::new(input);
    for instruction in instructions {
        match instruction.command {
            Command::InNumber => {
                let value = feed.number().ok_or(Error::InputExhausted)?;
                stream.push_str(&format!(" {value}"));
            }
            Command::InChar => {
                let character = feed.character().ok_or(Error::InputExhausted)?;
                stream.push_str(&format!(" {}", character as u32));
            }
            _ => {}
        }
    }

    Ok((program()?.image()?, stream))
}

/// Where the walker's records get a block's size.
///
/// A record carries the size of the block its state runs in, which is what a
/// `push` command costs. `canvas::Blocks` answers for a partition the host built;
/// [`crate::Partition`] answers for the one the splitter printed.
pub trait Sizes {
    /// The number of codels in `block`, or `None` when there is no such block.
    fn size(&self, block: BlockId) -> Option<u32>;
}

impl Sizes for Blocks {
    fn size(&self, block: BlockId) -> Option<u32> {
        self.block(block)
            .and_then(|target| u32::try_from(target.size()).ok())
    }
}

/// A walker image and the stream that runs a program's `graph`.
///
/// The graph decides where to go next, so the records carry the offsets: a
/// deterministic state jumps straight to its successor, and a branch state has
/// its trampolines — one per successor — directly after its own record, so the
/// handler only has to widen the popped value into `4 * (1 + index)`.
///
/// `sizes` is where each record's block size comes from: the same partition the
/// graph was built with, either the host's `Blocks` or the image's
/// [`crate::Partition`].
///
/// `input` is read as whitespace-separated numbers, one per read: a graph picks
/// its reads at run time, so they cannot be encoded per command. A character
/// read pushes the number as its code point.
pub fn graph_walker<S: Sizes + ?Sized>(
    graph: &StateGraph,
    sizes: &S,
    input: &str,
) -> Result<(CodelGrid, String), Error> {
    let Some(start) = graph.start() else {
        return Err(Error::NoStart);
    };
    // The head starts on the last record read, so the start state is placed
    // last in the stream, which is record zero.
    let mut order = vec![start];
    for state in graph.states() {
        if *state != start {
            order.push(*state);
        }
    }

    // Allocate: a record per state, a branch state's trampolines directly after
    // it, and nothing else. A state with no successor gets a zero command, which
    // is the halt the dispatch already understands.
    let mut records: Vec<Record> = Vec::new();
    let mut at: HashMap<State, usize> = HashMap::new();
    for state in &order {
        at.insert(*state, records.len());
        records.push((0, 0, 0, 0));
        let trampolines = graph
            .successors(*state)
            .next()
            .map_or(0, |edge| trampolines(edge.branch)) as usize;
        records.resize(records.len() + trampolines, (0, 0, 0, 0));
    }
    let count = u32::try_from(records.len()).map_err(|_| Error::TooLarge)?;
    let cells = i64::from(cells(count)?);
    for record in &mut records {
        record.2 = cells;
    }

    // Fill: every state runs its outgoing command, then jumps.
    for state in &order {
        let slot = at[state];
        let edges = graph.successors(*state).copied().collect::<Vec<_>>();
        let Some(first) = edges.first() else {
            // A state with no successor halts, running no command.
            continue;
        };
        let code = record_code(first.command);
        let size = sizes.size(state.block).map_or(0, i64::from);
        let offset = match first.branch {
            Branch::Deterministic => jump(slot, at[&first.to], count),
            Branch::Pointer(_) | Branch::Switch(_) => {
                let wanted = trampolines(first.branch) as usize;
                if edges.len() != wanted {
                    return Err(Error::NoLayout);
                }
                let base = slot + 1;
                for (index, edge) in edges.iter().enumerate() {
                    records[base + index] = (
                        i64::from(NOP),
                        0,
                        cells,
                        jump(base + index, at[&edge.to], count),
                    );
                }
                // The handler overwrites this with `4 * (1 + index)`.
                i64::from(RECORD)
            }
        };
        records[slot] = (i64::from(code), size, cells, offset);
    }

    let mut stream = ring_stream(cells as u32, &records);
    let text = input.trim();
    if !text.is_empty() {
        stream.push(' ');
        stream.push_str(text);
    }
    Ok((program()?.image()?, stream))
}

/// How many successors a branch has.
fn trampolines(branch: Branch) -> u32 {
    match branch {
        Branch::Deterministic => 0,
        Branch::Pointer(_) => 4,
        Branch::Switch(_) => 2,
    }
}

/// The offset that moves the head from record `from` to record `to`.
fn jump(from: usize, to: usize, count: u32) -> i64 {
    let count = count as usize;
    i64::from(RECORD) * (((to + count - from) % count) as i64)
}

/// The ring half of the stream: the size, then the cells highest record first.
fn ring_stream(cells: u32, records: &[Record]) -> String {
    let mut stream = cells.to_string();
    for (command, size, ring, offset) in records.iter().rev() {
        stream.push_str(&format!(" {command} {size} {ring} {offset}"));
    }
    stream
}

/// The ring's size in cells.
fn cells(count: u32) -> Result<u32, Error> {
    count.checked_mul(RECORD).ok_or(Error::TooLarge)
}

/// The walker image.
///
/// It is the same image for every program: the ring's size is a record field, so
/// nothing about a particular program reaches the geometry.
fn program() -> Result<Program, Error> {
    let mut blocks = load_loop(1);
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
        blocks.push(guard(code));
    }
    for code in 1..=CODES {
        blocks.push(body(code));
    }
    blocks.extend([
        Block::new(division(Command::Divide, true), End::Jump(JUMP)),
        Block::new(division(Command::Divide, false), End::Jump(JUMP)),
        Block::new(division(Command::Modulo, true), End::Jump(JUMP)),
        Block::new(division(Command::Modulo, false), End::Jump(JUMP)),
        // 48, 49: a branch on an empty stack takes the first successor
        Block::new(zero_branch(), End::Jump(JUMP)),
        Block::new(zero_branch(), End::Jump(JUMP)),
    ]);
    for depth in 2..=ROLL_DEPTH {
        blocks.push(roll_case(depth));
    }
    blocks.extend([
        // 57: past the table's bound
        Block::new(
            vec![
                Node::Dup,
                Node::Const(i64::from(ROLL_DEPTH)),
                Node::Command(Command::Greater),
            ],
            End::If {
                then: ROLL_UNSUPPORTED,
                otherwise: ROLL_FITS,
            },
        ),
        // 58: deeper than the values, so the reference ignores it
        Block::new(
            {
                let mut nodes = dup_size(3, 2);
                nodes.extend([
                    Node::Const(2),
                    Node::Command(Command::Subtract),
                    Node::Command(Command::Greater),
                ]);
                nodes
            },
            End::If {
                then: ROLL_DEEP,
                otherwise: ROLL_SELECT,
            },
        ),
        // 59: the case for this depth is `ROLL_CASE_BASE + d`
        Block::new(
            {
                let mut nodes = dup_size(2, 3);
                nodes.extend([
                    Node::Const(i64::from(ROLL_CASE_BASE)),
                    Node::Command(Command::Add),
                ]);
                nodes
            },
            End::Computed,
        ),
        // 60: shallow enough to be a no-op, and the copy is still there
        Block::new(roll_skip(true), End::Jump(JUMP)),
        // 61: the same, with the copy already consumed by the test above
        Block::new(roll_skip(false), End::Jump(JUMP)),
        // 62: the table cannot run this roll
        Block::new(Vec::new(), End::Halt),
        // 63: roll the ring by the head's offset
        Block::new(
            vec![
                Node::DupLocal(SIZE_DEPTH),
                Node::DupLocal(SIZE_DEPTH),
                Node::Command(Command::Roll),
            ],
            End::Jump(DISPATCH),
        ),
    ]);
    Program::new(blocks, 1)
}

/// The four blocks that read the ring into the stack, leaving the head on record
/// zero.
///
/// The stream's first number is the ring's size in cells; the loop then counts
/// it down, one cell per pass. `registers` zeroes are pushed first, so they end
/// up below the ring. Block 1 is the entry and block 4 hands over with
/// [`DISPATCH`].
fn load_loop(registers: u32) -> Vec<Block> {
    let mut start = vec![Node::Const(0); registers as usize];
    // The count arrives on top of the registers.
    start.push(Node::Command(Command::InNumber));
    vec![
        // 1: the registers, then the cell count from the stream
        Block::new(start, End::Jump(2)),
        // 2: while the count is not zero
        Block::new(
            vec![Node::Dup, Node::Command(Command::Not)],
            End::If {
                then: 4,
                otherwise: 3,
            },
        ),
        // 3: read a cell under the count, then decrement
        Block::new(
            vec![
                Node::Command(Command::InNumber),
                Node::Swap,
                Node::Const(1),
                Node::Command(Command::Subtract),
            ],
            End::Jump(2),
        ),
        // 4: drop the count; the last record read is now the head
        Block::new(vec![Node::Command(Command::Pop)], End::Jump(DISPATCH)),
    ]
}

/// The number of values a command needs before it does anything.
fn needed(code: u32) -> u32 {
    match code {
        1 | 12 | 13 | NOP => 0,
        3..=7 | 9 | 11 => 2,
        _ => 1,
    }
}

/// A guard block: test the depth counter, run `body(code)`, or leave.
///
/// The two branch codes are the exception: an empty stack is not a skip there,
/// so the other side is a block that takes the first successor.
fn guard(code: u32) -> Block {
    let body = BODY_BASE + code;
    let needed = needed(code);
    if needed == 0 {
        return Block::new(Vec::new(), End::Jump(body));
    }
    let otherwise = match code {
        POINTER => POINTER_ZERO,
        SWITCH => SWITCH_ZERO,
        _ => JUMP,
    };
    let mut nodes = dup_size(SIZE_DEPTH, 1);
    nodes.extend([
        Node::Const(i64::from(needed) - 1),
        Node::Command(Command::Greater),
    ]);
    Block::new(
        nodes,
        End::If {
            then: body,
            otherwise,
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
fn body(code: u32) -> Block {
    let mut nodes = match code {
        // push: the record's block size becomes a value
        1 => {
            let mut nodes = vec![Node::DupLocal(3)];
            nodes.extend(insert_value(3));
            nodes
        }
        2 => {
            let mut nodes = lift_one();
            nodes.push(Node::Command(Command::Pop));
            nodes
        }
        3 | 4 | 5 | 9 => {
            let mut nodes = lift_two();
            nodes.push(Node::Command(match code {
                3 => Command::Add,
                4 => Command::Subtract,
                5 => Command::Multiply,
                _ => Command::Greater,
            }));
            nodes.extend(insert_value(3));
            nodes
        }
        // divide and modulo branch on the divisor; see `division`
        6 | 7 => {
            let (zero, normal) = if code == 6 {
                (DIVIDE_ZERO, DIVIDE_NORMAL)
            } else {
                (MODULO_ZERO, MODULO_NORMAL)
            };
            let mut nodes = lift_two();
            nodes.extend([
                Node::Command(Command::Duplicate),
                Node::Command(Command::Not),
            ]);
            return Block::new(
                nodes,
                End::If {
                    then: zero,
                    otherwise: normal,
                },
            );
        }
        8 => {
            let mut nodes = lift_one();
            nodes.push(Node::Command(Command::Not));
            nodes.extend(insert_value(3));
            nodes
        }
        10 => {
            let mut nodes = lift_one();
            nodes.push(Node::Command(Command::Duplicate));
            nodes.extend(roll_with(4, 3, 2));
            nodes
        }
        // a character arrives as its code point, so both reads are the same
        12 | 13 => {
            let mut nodes = vec![Node::Command(Command::InNumber)];
            nodes.extend(insert_value(3));
            nodes
        }
        14 => {
            let mut nodes = lift_one();
            nodes.push(Node::Command(Command::OutNumber));
            nodes
        }
        15 => {
            let mut nodes = lift_one();
            nodes.push(Node::Command(Command::OutChar));
            nodes
        }
        // roll branches on its depth; see `roll_case`
        11 => {
            let mut nodes = dup_size(SIZE_DEPTH, 3);
            nodes.extend([Node::Dup, Node::Const(1), Node::Command(Command::Greater)]);
            return Block::new(
                nodes,
                End::If {
                    then: ROLL_HIGH,
                    otherwise: ROLL_SHALLOW,
                },
            );
        }
        NOP => Vec::new(),
        // pointer and switch: pop the value, widen it, and replace the offset
        POINTER | SWITCH => {
            let mut nodes = lift_one();
            nodes.extend([
                Node::Const(if code == POINTER { 4 } else { 2 }),
                Node::Command(Command::Modulo),
                Node::Const(1),
                Node::Command(Command::Add),
                Node::Const(i64::from(RECORD)),
                Node::Command(Command::Multiply),
                Node::Swap,
                Node::Command(Command::Pop),
            ]);
            nodes
        }
        _ => return Block::new(Vec::new(), End::Halt),
    };
    let depth = match code {
        1 | 10 | 12 | 13 => Depth::Plus(1),
        8 | NOP => Depth::Same,
        _ => Depth::Minus(1),
    };
    adjust(&mut nodes, depth);
    Block::new(nodes, End::Jump(JUMP))
}

/// A branch on an empty stack: the first successor is one record forward.
fn zero_branch() -> Vec<Node> {
    vec![
        Node::Const(i64::from(RECORD)),
        Node::Swap,
        Node::Command(Command::Pop),
    ]
}

/// The roll case for a depth known at weave time.
///
/// The operands are the top two values, so the lift is one rotation of a size the
/// ring decides: it puts the `depth` values and both operands above the ring, in
/// the order `Roll` wants. The `Roll` command then rotates the values themselves,
/// and the put-back — the step that is impossible when `depth` is data — is a
/// rotation by the constant `depth`. Only the two amounts are data.
fn roll_case(depth: u32) -> Block {
    // lift: values, then both operands, above the ring
    let mut nodes = roll_size(SIZE_DEPTH, depth + 3, 1);
    // rotate the values by the rolls operand
    nodes.push(Node::Command(Command::Roll));
    // put them back below the depth register
    nodes.extend(roll_with(depth + 2, depth + 1, i64::from(depth)));
    adjust(&mut nodes, Depth::Minus(2));
    Block::new(nodes, End::Jump(JUMP))
}

/// A roll shallow enough to be a no-op: both operands are still consumed.
///
/// `copy` says whether the depth copy the dispatch left on top is still there.
fn roll_skip(copy: bool) -> Vec<Node> {
    let mut nodes = Vec::new();
    if copy {
        nodes.push(Node::Command(Command::Pop));
    }
    for _ in 0..2 {
        nodes.extend(lift_one());
        nodes.push(Node::Command(Command::Pop));
    }
    adjust(&mut nodes, Depth::Minus(2));
    nodes
}

/// The two paths behind `divide` and `modulo`: a zero divisor consumes both
/// operands and pushes nothing, so the depth falls by two.
fn division(command: Command, non_zero: bool) -> Vec<Node> {
    let mut nodes = if non_zero {
        let mut nodes = vec![Node::Command(command)];
        nodes.extend(insert_value(3));
        nodes
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
    );
    nodes
}

/// Appends the commands that bring the depth counter by `change`.
///
/// The counter is the top of the register block: roll it up, change it, and roll
/// it back. Both amounts are built from the ring size, so the head stays
/// aligned.
fn adjust(nodes: &mut Vec<Node>, change: Depth) {
    let (amount, operation) = match change {
        Depth::Same => return,
        Depth::Plus(amount) => (i64::from(amount), Command::Add),
        Depth::Minus(amount) => (i64::from(amount), Command::Subtract),
    };
    nodes.extend(roll_size(SIZE_DEPTH, 1, 0));
    nodes.extend([Node::Const(amount), Node::Command(operation)]);
    nodes.extend(roll_with(3, 1, 1));
}

/// The commands that push `N + j`, where `N` is the ring's size in cells and
/// sits `at` slots down, counting the top as one.
fn push_size(at: u32, j: u32) -> Vec<Node> {
    if j == 0 {
        return vec![Node::DupLocal(at)];
    }
    vec![
        Node::DupLocal(at),
        Node::Const(i64::from(j)),
        Node::Command(Command::Add),
    ]
}

/// `roll(N + a, N + b)`: a rotation whose depth and rolls are both the ring's
/// size plus a literal. `at` is where `N` sits before the amounts are pushed.
fn roll_size(at: u32, a: u32, b: u32) -> Vec<Node> {
    let mut nodes = push_size(at, a);
    nodes.extend(push_size(at + 1, b));
    nodes.push(Node::Command(Command::Roll));
    nodes
}

/// `roll(N + a, rolls)`: a rotation whose depth is the ring's size plus a
/// literal and whose rolls is a literal.
fn roll_with(at: u32, a: u32, rolls: i64) -> Vec<Node> {
    let mut nodes = push_size(at, a);
    nodes.extend([Node::Const(rolls), Node::Command(Command::Roll)]);
    nodes
}

/// Duplicate the value at depth `N + j`, with `N` `at` slots down.
///
/// This is [`Node::DupLocal`] with a computed depth: bring the value up, copy it,
/// and put the original back where it was. `j` must be at least one.
fn dup_size(at: u32, j: u32) -> Vec<Node> {
    let mut nodes = roll_size(at, j, j - 1);
    nodes.push(Node::Dup);
    nodes.extend(roll_with(at + 2, j + 1, 1));
    nodes
}

/// `roll(N + 2, 1)`: insert the top value below the depth register, above the
/// values already there. `at` is where `N` sits, with the value on top.
fn insert_value(at: u32) -> Vec<Node> {
    roll_with(at, 2, 1)
}

/// `roll(N + 2, N + 1)`: bring one value up above the ring.
fn lift_one() -> Vec<Node> {
    roll_size(SIZE_DEPTH, 2, 1)
}

/// `roll(N + 3, N + 1)`: bring two values up, the stack's top first.
fn lift_two() -> Vec<Node> {
    roll_size(SIZE_DEPTH, 3, 1)
}

/// The opcode of a command the walker has a handler for.
///
/// `pointer` and `switch` have no opcode: a straight-line program cannot run
/// them, and the graph builder gives them codes of its own.
/// The record code for a state's command.
///
/// `nop` when the transition runs no command, and the branch codes for
/// `pointer` and `switch`; every other command is its opcode.
pub fn record_code(command: Option<Command>) -> u32 {
    match command {
        None => NOP,
        Some(Command::Pointer) => POINTER,
        Some(Command::Switch) => SWITCH,
        Some(command) => code_of(command).map_or(NOP, u32::from),
    }
}

/// The command a record code names: `nop` is `None`, and the branch codes are
/// `pointer` and `switch`.
pub fn record_command(code: u32) -> Option<Command> {
    match code {
        NOP => None,
        POINTER => Some(Command::Pointer),
        SWITCH => Some(Command::Switch),
        code => code
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| OPCODES.get(index).copied()),
    }
}

fn code_of(command: Command) -> Option<u8> {
    opcode(command)
}
