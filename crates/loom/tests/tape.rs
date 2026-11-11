#![allow(clippy::unwrap_used)]
//! A tape: values stored in memory and read back by a computed depth.
//!
//! Plain stack code can only reach the top. The program here fills a tape with
//! 1..=n, then prints it in order — which means reaching the *oldest* value
//! every time, a depth that changes as the tape drains. It does that with a
//! `roll` whose depth is computed at run time from the remaining length, which
//! is the piece the state-graph interpreter needs and the piece the checkpoints
//! in `flow.rs` did not exercise.
//!
//! The tape sits *below* the length, so the layout is
//!
//! ```text
//!   [ value 1, value 2, …, value len, len ]
//! ```
//!
//! and the depth of `value 1` is `len + 1`. Rolling that many — the length plus
//! the counter itself — moves the counter to depth two and `value 1` to the top,
//! after which one pop and one decrement leave the tape one shorter with the
//! counter still on top.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::{Block, End, Node, Program};
use runtime::BufferIo;
use tessera::Command;

fn run(grid: &CodelGrid, input: &str) -> (String, Run) {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let options = Options {
        max_steps: Some(1_000_000),
        ..Options::default()
    };
    let result = interp::run(&blocks, &mut io, &options).unwrap();
    (io.into_output(), result)
}

/// Fill a tape with 1..=n, then print it in order.
fn tape_program() -> Program {
    Program::new(
        vec![
            // 1: read n; the stack is [n, filled]
            Block::new(
                vec![Node::Command(Command::InNumber), Node::Const(0)],
                End::Jump(2),
            ),
            // 2: while filled < n
            Block::new(
                vec![
                    Node::Dup,
                    Node::DupLocal(3),
                    Node::Swap,
                    Node::Command(Command::Greater),
                ],
                End::If {
                    then: 3,
                    otherwise: 4,
                },
            ),
            // 3: push filled + 1 under the counter, then advance it
            Block::new(
                vec![
                    Node::Dup,
                    Node::Const(1),
                    Node::Command(Command::Add),
                    Node::Roll(3, 1),
                    Node::Const(1),
                    Node::Command(Command::Add),
                ],
                End::Jump(2),
            ),
            // 4: drop the fill counter; the length is n
            Block::new(vec![Node::Command(Command::Pop)], End::Jump(5)),
            // 5: while the tape is not empty
            Block::new(
                vec![Node::Dup, Node::Const(0), Node::Command(Command::Greater)],
                End::If {
                    then: 6,
                    otherwise: 7,
                },
            ),
            // 6: roll the oldest value up, print it, shrink the tape
            Block::new(
                vec![
                    // depth = len + 1, rolls = depth - 1
                    Node::Dup,
                    Node::Const(1),
                    Node::Command(Command::Add),
                    Node::Dup,
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                    // the computed roll itself
                    Node::Command(Command::Roll),
                    Node::Command(Command::OutNumber),
                    Node::Const(32),
                    Node::Command(Command::OutChar),
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                ],
                End::Jump(5),
            ),
            Block::new(Vec::new(), End::Halt),
        ],
        1,
    )
    .unwrap()
}

#[test]
fn a_tape_drains_in_order() {
    let image = tape_program().image().unwrap();
    for n in [-3, 0, 1, 2, 5, 9, 40] {
        let expected = (1..=n).map(|value| format!("{value} ")).collect::<String>();
        let (output, result) = run(&image, &format!("{n}\n"));
        assert_eq!(output, expected, "n = {n}");
        assert_eq!(result.reason, ExitReason::Terminated);
    }
}

#[test]
fn the_tape_stays_a_reasonable_size() {
    // One lane per block, and the longest block is the drain body.
    let image = tape_program().image().unwrap();
    assert!(image.width() <= 96, "width {}", image.width());
    assert!(image.height() <= 96, "height {}", image.height());
}
