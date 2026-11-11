#![allow(clippy::unwrap_used)]
//! The structured backend: loops and branches, checked against Rust models.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::{Block, End, Error, Node, Program};
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

fn weave(program: &Program) -> CodelGrid {
    program.image().unwrap()
}

/// Sum 1..=n.
///
/// ```text
/// 1  read n, push 0            [i, sum]
/// 2  if i > 0 goto 3 else 4
/// 3  sum += i; i -= 1; goto 2
/// 4  swap, pop, print
/// ```
fn sum_program() -> Program {
    Program::new(
        vec![
            Block::new(
                vec![Node::Command(Command::InNumber), Node::Const(0)],
                End::Jump(2),
            ),
            Block::new(
                vec![
                    Node::DupLocal(2),
                    Node::Const(0),
                    Node::Command(Command::Greater),
                ],
                End::If {
                    then: 3,
                    otherwise: 4,
                },
            ),
            Block::new(
                vec![
                    Node::DupLocal(2),
                    Node::Command(Command::Add),
                    Node::Swap,
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                    Node::Swap,
                ],
                End::Jump(2),
            ),
            Block::new(
                vec![
                    Node::Swap,
                    Node::Command(Command::Pop),
                    Node::Command(Command::OutNumber),
                ],
                End::Halt,
            ),
        ],
        1,
    )
    .unwrap()
}

/// Add even i and subtract odd i, for i in 1..=n.
///
/// Both arms of the inner branch run, so this is the case that would catch a
/// successor selection that always picked the same side.
fn alternating_program() -> Program {
    Program::new(
        vec![
            Block::new(
                vec![Node::Command(Command::InNumber), Node::Const(0)],
                End::Jump(2),
            ),
            Block::new(
                vec![
                    Node::DupLocal(2),
                    Node::Const(0),
                    Node::Command(Command::Greater),
                ],
                End::If {
                    then: 3,
                    otherwise: 5,
                },
            ),
            Block::new(
                vec![
                    Node::DupLocal(2),
                    Node::Const(2),
                    Node::Command(Command::Modulo),
                ],
                End::If {
                    then: 6,
                    otherwise: 4,
                },
            ),
            Block::new(
                vec![Node::DupLocal(2), Node::Command(Command::Add)],
                End::Jump(7),
            ),
            Block::new(
                vec![
                    Node::Swap,
                    Node::Command(Command::Pop),
                    Node::Command(Command::OutNumber),
                ],
                End::Halt,
            ),
            Block::new(
                vec![Node::DupLocal(2), Node::Command(Command::Subtract)],
                End::Jump(7),
            ),
            Block::new(
                vec![
                    Node::Swap,
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                    Node::Swap,
                ],
                End::Jump(2),
            ),
        ],
        1,
    )
    .unwrap()
}

/// A nested loop: for i in 1..=n, count i times.
fn nested_program() -> Program {
    Program::new(
        vec![
            Block::new(
                vec![Node::Command(Command::InNumber), Node::Const(0)],
                End::Jump(2),
            ),
            Block::new(
                vec![
                    Node::DupLocal(2),
                    Node::Const(0),
                    Node::Command(Command::Greater),
                ],
                End::If {
                    then: 3,
                    otherwise: 7,
                },
            ),
            Block::new(vec![Node::DupLocal(2)], End::Jump(4)),
            Block::new(
                vec![Node::Dup, Node::Const(0), Node::Command(Command::Greater)],
                End::If {
                    then: 5,
                    otherwise: 6,
                },
            ),
            Block::new(
                vec![
                    Node::Swap,
                    Node::Const(1),
                    Node::Command(Command::Add),
                    Node::Swap,
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                ],
                End::Jump(4),
            ),
            Block::new(
                vec![
                    Node::Command(Command::Pop),
                    Node::Swap,
                    Node::Const(1),
                    Node::Command(Command::Subtract),
                    Node::Swap,
                ],
                End::Jump(2),
            ),
            Block::new(
                vec![
                    Node::Swap,
                    Node::Command(Command::Pop),
                    Node::Command(Command::OutNumber),
                ],
                End::Halt,
            ),
        ],
        1,
    )
    .unwrap()
}

#[test]
fn a_loop_sums_to_n() {
    let image = weave(&sum_program());
    for n in [0, 1, 2, 5, 10, 100] {
        let expected = (1..=n).sum::<i64>();
        let (output, result) = run(&image, &format!("{n}\n"));
        assert_eq!(output, expected.to_string(), "n = {n}");
        assert_eq!(result.reason, ExitReason::Terminated);
    }
}

#[test]
fn a_loop_with_both_branches_alternates() {
    let image = weave(&alternating_program());
    for n in [0, 1, 2, 3, 4, 5, 6, 7] {
        let expected = (1..=n)
            .map(|i| if i % 2 == 0 { i } else { -i })
            .sum::<i64>();
        let (output, _) = run(&image, &format!("{n}\n"));
        assert_eq!(output, expected.to_string(), "n = {n}");
    }
}

#[test]
fn a_nested_loop_counts_each_outer_step() {
    let image = weave(&nested_program());
    for n in [0, 1, 2, 3, 4, 5, 10, 20] {
        let expected = n * (n + 1) / 2;
        let (output, _) = run(&image, &format!("{n}\n"));
        assert_eq!(output, expected.to_string(), "n = {n}");
    }
}

#[test]
fn constants_reach_the_stack_exactly() {
    // Each constant is printed, then a space separates them.
    let values = [0, 1, 2, 3, 7, 64, 1_000, -1, -42];
    let mut nodes = Vec::new();
    for value in values {
        nodes.extend([
            Node::Const(value),
            Node::Command(Command::OutNumber),
            Node::Const(32),
            Node::Command(Command::OutChar),
        ]);
    }
    let program = Program::new(vec![Block::new(nodes, End::Halt)], 1).unwrap();
    let expected = values
        .iter()
        .map(|value| format!("{value} "))
        .collect::<String>();
    let (output, _) = run(&weave(&program), "");
    assert_eq!(output, expected);
}

#[test]
fn a_jump_to_a_missing_block_is_refused() {
    let program = Program::new(vec![Block::new(vec![], End::Jump(2))], 1);
    assert!(matches!(program, Err(Error::BlockId { id: 2, count: 1 })));
}

#[test]
fn an_empty_program_is_refused() {
    assert!(matches!(
        Program::new(Vec::new(), 1),
        Err(Error::EmptyProgram)
    ));
}

#[test]
fn a_small_program_lays_out_small() {
    // The layout has to stay close to one lane per block; a blow-up here is a
    // layout bug, not a budget overrun.
    for program in [sum_program(), alternating_program(), nested_program()] {
        let image = weave(&program);
        assert!(image.width() <= 64, "width {}", image.width());
        assert!(image.height() <= 64, "height {}", image.height());
    }
}

#[test]
fn a_local_depth_counts_from_one() {
    let blocks = vec![Block::new(vec![Node::DupLocal(0)], End::Halt)];
    assert!(matches!(Program::new(blocks, 1), Err(Error::LocalDepth)));
}
