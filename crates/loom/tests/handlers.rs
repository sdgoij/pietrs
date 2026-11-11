#![allow(clippy::unwrap_used)]
//! The walker's handlers: commands applied to a value stack below the ring.
//!
//! Every case is checked against `interp` running the same program, so the
//! image is never its own oracle.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::{Error, Instruction, instructions, walker};
use runtime::BufferIo;
use tessera::Command;

fn options() -> Options {
    Options {
        max_steps: Some(2_000_000),
        ..Options::default()
    }
}

fn run(grid: &CodelGrid, input: &str) -> (String, Run) {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let result = interp::run(&blocks, &mut io, &options()).unwrap();
    (io.into_output(), result)
}

/// Weave the walker for `program` and run it on the stream it wants.
fn walk(program: &[Instruction]) -> (String, Run) {
    let (image, stream) = walker(program).unwrap();
    run(&image, &stream)
}

fn step(command: Command) -> Instruction {
    Instruction {
        command,
        block_size: 1,
    }
}

fn push(value: u64) -> Instruction {
    Instruction {
        command: Command::Push,
        block_size: value,
    }
}

#[test]
fn a_push_and_an_out_char_print_the_block_size() {
    let (output, result) = walk(&[push(65), step(Command::OutChar)]);
    assert_eq!(output, "A");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn an_empty_program_halts_without_output() {
    let (output, result) = walk(&[]);
    assert_eq!(output, "");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn arithmetic_reads_the_value_stack_in_piet_order() {
    // subtract pops the top value first, so 10 4 subtract is 10 - 4...
    let (output, _) = walk(&[
        push(10),
        push(4),
        step(Command::Subtract),
        step(Command::OutNumber),
    ]);
    assert_eq!(output, "6");

    // ...and 4 10 subtract is 4 - 10.
    let (output, _) = walk(&[
        push(4),
        push(10),
        step(Command::Subtract),
        step(Command::OutNumber),
    ]);
    assert_eq!(output, "-6");
}

#[test]
fn a_chain_of_commands_keeps_the_stack() {
    // ((2 + 3) * 4) / 2 = 10
    let (output, _) = walk(&[
        push(2),
        push(3),
        step(Command::Add),
        push(4),
        step(Command::Multiply),
        push(2),
        step(Command::Divide),
        step(Command::OutNumber),
    ]);
    assert_eq!(output, "10");
}

#[test]
fn duplicate_leaves_exactly_one_extra_copy() {
    // Two copies, two prints.
    let (output, _) = walk(&[
        push(7),
        step(Command::Duplicate),
        step(Command::OutNumber),
        step(Command::OutNumber),
    ]);
    assert_eq!(output, "77");

    // The copies are independent, so doubling the top is 14.
    let (output, _) = walk(&[
        push(7),
        step(Command::Duplicate),
        step(Command::Add),
        step(Command::OutNumber),
    ]);
    assert_eq!(output, "14");
}

#[test]
fn a_pop_drops_only_the_top_value() {
    let (output, _) = walk(&[
        push(7),
        push(9),
        step(Command::Pop),
        step(Command::OutNumber),
    ]);
    assert_eq!(output, "7");
}

#[test]
fn the_hello_world_program_runs_from_a_ring() {
    // The whole point of the walker: a real program's instruction stream, fed
    // to one fixed image, against `interp` running the original.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let blocks = Blocks::partition(source.clone());
    let mut probe = BufferIo::new("");
    let program = instructions(&blocks, &mut probe, &options()).unwrap();
    assert!(
        program.len() > 8,
        "the fixture ran {} commands",
        program.len()
    );

    let (expected, _) = run(&source, "");
    assert_eq!(expected, "Hello world!");

    let (actual, result) = walk(&program);
    assert_eq!(actual, expected);
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_walker_width_does_not_grow_with_the_program() {
    // One fixed image serves every program: the ring is data, not geometry. The
    // width is exactly the dispatch table; the height moves only because a
    // constant for a bigger ring takes more cells to build.
    let (small, _) = walker(&[push(1)]).unwrap();
    let long: Vec<Instruction> = (0..64).map(push).collect();
    let (large, _) = walker(&long).unwrap();
    assert_eq!(small.width(), large.width());
    assert!(small.width() <= 256, "width {}", small.width());
    assert!(small.height() <= 192, "height {}", small.height());
    assert!(large.height() <= 192, "height {}", large.height());
}

#[test]
fn a_command_without_a_handler_is_refused() {
    for command in [Command::Roll, Command::InNumber, Command::InChar] {
        assert!(matches!(
            walker(&[step(command)]),
            Err(Error::NoHandler { command: refused }) if refused == command
        ));
    }
}
