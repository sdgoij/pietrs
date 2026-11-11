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
//! | duplicate | `roll(N+2, N+1)`, `dup`, `roll(N+3, 2)` |
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

use canvas::{Blocks, CodelGrid};
use tessera::{Branch, Command, State, StateGraph};

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};
use crate::program::{Feed, Instruction, opcode};

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
/// compute the put-back, and the table is one block per depth.
const ROLL_DEPTH: u32 = 8;
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

/// A record: command code, block size, offset in cells.
type Record = (i64, i64, i64);

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

/// A walker image and the stream that loads its ring and feeds its input, for a
/// straight-line `instructions` sequence.
///
/// Commands the walker cannot run are refused by [`Error::NoHandler`]; a program
/// whose input reads outrun `input` is refused by [`Error::InputExhausted`].
pub fn walker(instructions: &[Instruction], input: &str) -> Result<(CodelGrid, String), Error> {
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

    let mut stream = ring_stream(&records);
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

    Ok((program(cells(count)?)?.image()?, stream))
}

/// A walker image and the stream that runs a program's `graph`.
///
/// The graph decides where to go next, so the records carry the offsets: a
/// deterministic state jumps straight to its successor, and a branch state has
/// its trampolines — one per successor — directly after its own record, so the
/// handler only has to widen the popped value into `3 * (1 + index)`.
///
/// `input` is read as whitespace-separated numbers, one per read: a graph picks
/// its reads at run time, so they cannot be encoded per command. A character
/// read pushes the number as its code point.
pub fn graph_walker(
    graph: &StateGraph,
    blocks: &Blocks,
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
        records.push((0, 0, 0));
        let trampolines = graph
            .successors(*state)
            .next()
            .map_or(0, |edge| trampolines(edge.branch)) as usize;
        records.resize(records.len() + trampolines, (0, 0, 0));
    }
    let count = u32::try_from(records.len()).map_err(|_| Error::TooLarge)?;

    // Fill: every state runs its outgoing command, then jumps.
    for state in &order {
        let slot = at[state];
        let edges = graph.successors(*state).copied().collect::<Vec<_>>();
        let Some(first) = edges.first() else {
            // A state with no successor halts, running no command.
            continue;
        };
        let code = match first.command {
            None => NOP,
            Some(Command::Pointer) => POINTER,
            Some(Command::Switch) => SWITCH,
            Some(command) => u32::from(code_of(command).ok_or(Error::NoHandler { command })?),
        };
        let size = blocks
            .block(state.block)
            .map_or(0, |block| block.size() as i64);
        let offset = match first.branch {
            Branch::Deterministic => jump(slot, at[&first.to], count),
            Branch::Pointer(_) | Branch::Switch(_) => {
                let wanted = trampolines(first.branch) as usize;
                if edges.len() != wanted {
                    return Err(Error::NoLayout);
                }
                let base = slot + 1;
                for (index, edge) in edges.iter().enumerate() {
                    records[base + index] =
                        (i64::from(NOP), 0, jump(base + index, at[&edge.to], count));
                }
                // The handler overwrites this with `3 * (1 + index)`.
                i64::from(RECORD)
            }
        };
        records[slot] = (i64::from(code), size, offset);
    }

    let mut stream = ring_stream(&records);
    let text = input.trim();
    if !text.is_empty() {
        stream.push(' ');
        stream.push_str(text);
    }
    Ok((program(cells(count)?)?.image()?, stream))
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

/// The ring half of the stream: highest record first, spaces between cells.
fn ring_stream(records: &[Record]) -> String {
    let mut stream = String::new();
    for (index, (command, size, offset)) in records.iter().rev().enumerate() {
        if index > 0 {
            stream.push(' ');
        }
        stream.push_str(&format!("{command} {size} {offset}"));
    }
    stream
}

/// The ring's size in cells.
fn cells(count: u32) -> Result<u32, Error> {
    count.checked_mul(RECORD).ok_or(Error::TooLarge)
}

/// The walker image for a ring of `cells`.
fn program(cells: u32) -> Result<Program, Error> {
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
        Block::new(division(Command::Divide, cells, true), End::Jump(JUMP)),
        Block::new(division(Command::Divide, cells, false), End::Jump(JUMP)),
        Block::new(division(Command::Modulo, cells, true), End::Jump(JUMP)),
        Block::new(division(Command::Modulo, cells, false), End::Jump(JUMP)),
        // 48, 49: a branch on an empty stack takes the first successor
        Block::new(zero_branch(), End::Jump(JUMP)),
        Block::new(zero_branch(), End::Jump(JUMP)),
    ]);
    for depth in 2..=ROLL_DEPTH {
        blocks.push(roll_case(depth, cells));
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
            vec![
                Node::DupLocal(cells + 2),
                Node::Const(2),
                Node::Command(Command::Subtract),
                Node::Command(Command::Greater),
            ],
            End::If {
                then: ROLL_DEEP,
                otherwise: ROLL_SELECT,
            },
        ),
        // 59: the case for this depth is `ROLL_CASE_BASE + d`
        Block::new(
            vec![
                Node::DupLocal(cells + 3),
                Node::Const(i64::from(ROLL_CASE_BASE)),
                Node::Command(Command::Add),
            ],
            End::Computed,
        ),
        // 60: shallow enough to be a no-op, and the copy is still there
        Block::new(roll_skip(cells, true), End::Jump(JUMP)),
        // 61: the same, with the copy already consumed by the test above
        Block::new(roll_skip(cells, false), End::Jump(JUMP)),
        // 62: the table cannot run this roll
        Block::new(Vec::new(), End::Halt),
        // 63: roll the ring by the head's offset
        Block::new(
            vec![
                Node::Const(i64::from(cells)),
                Node::DupLocal(2),
                Node::Command(Command::Roll),
            ],
            End::Jump(DISPATCH),
        ),
    ]);
    Program::new(blocks, 1)
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
        1 | 12 | 13 | NOP => 0,
        3..=7 | 9 | 11 => 2,
        _ => 1,
    }
}

/// A guard block: test the depth counter, run `body(code)`, or leave.
///
/// The two branch codes are the exception: an empty stack is not a skip there,
/// so the other side is a block that takes the first successor.
fn guard(code: u32, cells: u32) -> Block {
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
    Block::new(
        vec![
            Node::DupLocal(cells + DEPTH_DEPTH),
            Node::Const(i64::from(needed) - 1),
            Node::Command(Command::Greater),
        ],
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
        6 | 7 => {
            let (zero, normal) = if code == 6 {
                (DIVIDE_ZERO, DIVIDE_NORMAL)
            } else {
                (MODULO_ZERO, MODULO_NORMAL)
            };
            return Block::new(
                vec![
                    up_two,
                    Node::Command(Command::Duplicate),
                    Node::Command(Command::Not),
                ],
                End::If {
                    then: zero,
                    otherwise: normal,
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
        // a character arrives as its code point, so both reads are the same
        12 | 13 => vec![Node::Command(Command::InNumber), insert],
        14 => vec![up_one, Node::Command(Command::OutNumber)],
        15 => vec![up_one, Node::Command(Command::OutChar)],
        // roll branches on its depth; see `roll_case`
        11 => {
            return Block::new(
                vec![
                    Node::DupLocal(cells + 3),
                    Node::Dup,
                    Node::Const(1),
                    Node::Command(Command::Greater),
                ],
                End::If {
                    then: ROLL_HIGH,
                    otherwise: ROLL_SHALLOW,
                },
            );
        }
        NOP => Vec::new(),
        // pointer and switch: pop the value, widen it, and replace the offset
        POINTER | SWITCH => vec![
            up_one,
            Node::Const(if code == POINTER { 4 } else { 2 }),
            Node::Command(Command::Modulo),
            Node::Const(1),
            Node::Command(Command::Add),
            Node::Const(i64::from(RECORD)),
            Node::Command(Command::Multiply),
            Node::Swap,
            Node::Command(Command::Pop),
        ],
        _ => return Block::new(Vec::new(), End::Halt),
    };
    let depth = match code {
        1 | 10 | 12 | 13 => Depth::Plus(1),
        8 | NOP => Depth::Same,
        _ => Depth::Minus(1),
    };
    adjust(&mut nodes, depth, cells);
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
/// The operands are the top two values, so the lift is one rotation of a constant
/// size: it puts the `depth` values and both operands above the ring, in the
/// order `Roll` wants. The `Roll` command then rotates the values themselves,
/// and the put-back — the step that is impossible when `depth` is data — is a
/// rotation by the constant `depth`.
fn roll_case(depth: u32, cells: u32) -> Block {
    let mut nodes = vec![
        // lift: values, then both operands, above the ring
        Node::Roll(cells + depth + 3, cells + 1),
        // rotate the values by the rolls operand
        Node::Command(Command::Roll),
        // put them back below the depth register
        Node::Roll(cells + depth + 1, depth),
    ];
    adjust(&mut nodes, Depth::Minus(2), cells);
    Block::new(nodes, End::Jump(JUMP))
}

/// A roll shallow enough to be a no-op: both operands are still consumed.
///
/// `copy` says whether the depth copy the dispatch left on top is still there.
fn roll_skip(cells: u32, copy: bool) -> Vec<Node> {
    let up_one = Node::Roll(cells + 2, cells + 1);
    let mut nodes = Vec::new();
    if copy {
        nodes.push(Node::Command(Command::Pop));
    }
    nodes.extend([
        up_one,
        Node::Command(Command::Pop),
        up_one,
        Node::Command(Command::Pop),
    ]);
    adjust(&mut nodes, Depth::Minus(2), cells);
    nodes
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
///
/// `pointer` and `switch` have no opcode: a straight-line program cannot run
/// them, and the graph builder gives them codes of its own.
fn code_of(command: Command) -> Option<u8> {
    opcode(command)
}
