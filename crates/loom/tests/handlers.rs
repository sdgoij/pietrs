#![allow(clippy::unwrap_used)]
//! The walker's handlers: commands applied to a value stack below the ring.
//!
//! Every case is checked against `interp` running the same program, so the
//! image is never its own oracle.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::{Error, Instruction, assemble, instructions, walker};
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

/// The reference path for a straight-line sequence: the same instructions as a
/// real image, run by `interp` with its own runtime.
fn straight_line(program: &[Instruction]) -> (String, Run) {
    run(&assemble(program).unwrap(), "")
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
fn an_underflowing_command_is_ignored() {
    // Popping, duplicating, negating, outputting or combining an empty stack does
    // nothing, so the program still prints A.
    for command in [
        Command::Pop,
        Command::Duplicate,
        Command::Not,
        Command::OutNumber,
        Command::OutChar,
        Command::Add,
        Command::Subtract,
        Command::Greater,
    ] {
        let mut program = vec![step(command), step(command)];
        program.extend([push(65), step(Command::OutChar)]);
        let (output, result) = walk(&program);
        assert_eq!(output, "A", "{command:?}");
        assert_eq!(result.reason, ExitReason::Terminated, "{command:?}");
        let (expected, _) = straight_line(&program);
        assert_eq!(output, expected, "{command:?}");
    }
}

#[test]
fn a_zero_divisor_consumes_both_values() {
    // 7 / 0 and 7 % 0 leave nothing behind, so the next push is the only value.
    // The zero comes from 1 - 1: a Piet push cannot push zero.
    for command in [Command::Divide, Command::Modulo] {
        let program = [
            push(7),
            push(1),
            push(1),
            step(Command::Subtract),
            step(command),
            push(65),
            step(Command::OutChar),
        ];
        let (output, _) = walk(&program);
        assert_eq!(output, "A", "{command:?}");
        let (expected, _) = straight_line(&program);
        assert_eq!(output, expected, "{command:?}");
    }
}

#[test]
fn a_zero_divisor_leaves_the_depth_counter_exact() {
    // After 7 / 0 the stack is empty, so the subtract needs two values it does
    // not have and must be skipped: only the 9 is left to print. A counter that
    // still said one would let the subtract read a ring cell.
    for command in [Command::Divide, Command::Modulo] {
        let program = [
            push(7),
            push(1),
            push(1),
            step(Command::Subtract),
            step(command),
            push(9),
            step(Command::Subtract),
            step(Command::OutNumber),
        ];
        let (output, _) = walk(&program);
        assert_eq!(output, "9", "{command:?}");
        let (expected, _) = straight_line(&program);
        assert_eq!(output, expected, "{command:?}");
    }
}

/// A deterministic generator, so a failure names a seed that reproduces it.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn random_program(seed: u64, length: usize) -> Vec<Instruction> {
    // The output and push commands are repeated so the programs say something.
    let commands = [
        Command::Push,
        Command::Push,
        Command::OutNumber,
        Command::OutChar,
        Command::OutNumber,
        Command::Add,
        Command::Subtract,
        Command::Multiply,
        Command::Divide,
        Command::Modulo,
        Command::Not,
        Command::Greater,
        Command::Duplicate,
        Command::Pop,
    ];
    let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..length)
        .map(|_| {
            let command = commands[(random.next() % commands.len() as u64) as usize];
            let block_size = if command == Command::Push {
                random.next() % 9 + 1
            } else {
                1
            };
            Instruction {
                command,
                block_size,
            }
        })
        .collect()
}

#[test]
fn the_walker_agrees_with_the_straight_line_reference() {
    for seed in 0..32 {
        let program = random_program(seed, 24);
        let (expected, reference) = straight_line(&program);
        let (actual, walked) = walk(&program);
        assert_eq!(actual, expected, "seed {seed}: {program:?}");
        assert_eq!(walked.reason, reference.reason, "seed {seed}");
    }
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
