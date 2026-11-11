#![allow(clippy::unwrap_used)]
//! The interpreter runs programs, and agrees with the reference interpreter on
//! the programs it claims to interpret.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::{Instruction, encode, instructions, interpreter, linear_program};
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

fn bounded() -> Options {
    Options {
        max_steps: Some(1_000_000),
        ..Options::default()
    }
}

#[test]
fn a_linear_program_prints_its_text() {
    let grid = linear_program("Piet!").unwrap();
    let (output, result) = run(&grid, "");
    assert_eq!(output, "Piet!");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_interpreter_reproduces_a_linear_program() {
    let program = linear_program("Hello, Piet!").unwrap();
    let (expected, _) = run(&program, "");
    assert_eq!(expected, "Hello, Piet!");

    let blocks = Blocks::partition(program);
    let mut io = BufferIo::new("");
    let trace = instructions(&blocks, &mut io, &bounded()).unwrap();
    let stream = encode(&trace, "").unwrap();

    let (actual, result) = run(&interpreter().unwrap(), &stream);
    assert_eq!(actual, expected);
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_interpreter_adds() {
    // push 2; push 3; add; out number
    let (output, result) = run(&interpreter().unwrap(), "1 2 1 3 3 14 0");
    assert_eq!(output, "5");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_interpreter_duplicates() {
    // push 65; duplicate; out char; out char
    let (output, _) = run(&interpreter().unwrap(), "1 65 10 15 15 0");
    assert_eq!(output, "AA");
}

#[test]
fn the_interpreter_subtracts_in_the_right_order() {
    // push 10; push 3; subtract; out number
    let (output, _) = run(&interpreter().unwrap(), "1 10 1 3 4 14 0");
    assert_eq!(output, "7");
}

#[test]
fn the_interpreter_multiplies() {
    // push 6; push 7; multiply; out number
    let (output, _) = run(&interpreter().unwrap(), "1 6 1 7 5 14 0");
    assert_eq!(output, "42");
}

#[test]
fn the_interpreter_divides_and_takes_modulo() {
    // push -7; push 2; divide; out number
    let (output, _) = run(&interpreter().unwrap(), "1 -7 1 2 6 14 0");
    assert_eq!(output, "-4");
    // push -1; push 3; modulo; out number
    let (output, _) = run(&interpreter().unwrap(), "1 -1 1 3 7 14 0");
    assert_eq!(output, "2");
}

#[test]
fn the_interpreter_negates_and_compares() {
    // push 0; not; out number; push 5; not; out number
    let (output, _) = run(&interpreter().unwrap(), "1 0 8 14 1 5 8 14 0");
    assert_eq!(output, "10");
    // push 2; push 1; greater; out number
    let (output, _) = run(&interpreter().unwrap(), "1 2 1 1 9 14 0");
    assert_eq!(output, "1");
}

#[test]
fn the_interpreter_rolls() {
    // push 10; push 20; push 30; push 3; push 1; roll; out x3
    let (output, _) = run(
        &interpreter().unwrap(),
        "1 10 1 20 1 30 1 3 1 1 11 14 14 14 0",
    );
    assert_eq!(output, "201030");
}

#[test]
fn an_unknown_opcode_halts() {
    // 99 is past the last opcode, so the dispatch runs out and the halt path runs
    let (output, result) = run(&interpreter().unwrap(), "1 65 15 99 1 66 15 0");
    assert_eq!(output, "A");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_interpreter_passes_its_input_through() {
    // in char; out char
    let program = [
        Instruction {
            command: Command::InChar,
            block_size: 1,
        },
        Instruction {
            command: Command::OutChar,
            block_size: 1,
        },
    ];
    let stream = encode(&program, "Z").unwrap();
    let (output, _) = run(&interpreter().unwrap(), &stream);
    assert_eq!(output, "Z");
}

#[test]
fn the_interpreter_ignores_an_underflowing_pop() {
    // pop; push 66; out char
    let (output, _) = run(&interpreter().unwrap(), "2 1 66 15 0");
    assert_eq!(output, "B");
}
