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
    walk_with(program, "")
}

/// The same, for a program that reads `input`.
fn walk_with(program: &[Instruction], input: &str) -> (String, Run) {
    let (image, stream) = walker(program, input).unwrap();
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
/// real image, run by `interp` with its own runtime and `input`.
fn straight_line(program: &[Instruction], input: &str) -> (String, Run) {
    run(&assemble(program).unwrap(), input)
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
    let (small, _) = walker(&[push(1)], "").unwrap();
    let long: Vec<Instruction> = (0..64).map(push).collect();
    let (large, _) = walker(&long, "").unwrap();
    assert_eq!(small.width(), large.width());
    // 63 blocks of dispatch table, five cells each.
    assert!(small.width() <= 384, "width {}", small.width());
    assert!(small.height() <= 320, "height {}", small.height());
    assert!(large.height() <= 320, "height {}", large.height());
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
        let (expected, _) = straight_line(&program, "");
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
        let (expected, _) = straight_line(&program, "");
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
        let (expected, _) = straight_line(&program, "");
        assert_eq!(output, expected, "{command:?}");
    }
}

#[test]
fn a_roll_rotates_the_top_values() {
    // 10 20 30, then a depth of 3 and the rolls. Output pops, so it reads the
    // stack from the top down.
    for (rolls, expected) in [(1, "201030"), (2, "103020"), (3, "302010")] {
        let mut program = vec![push(10), push(20), push(30), push(3), push(rolls)];
        program.push(step(Command::Roll));
        for _ in 0..3 {
            program.push(step(Command::OutNumber));
        }
        let (output, _) = walk(&program);
        assert_eq!(output, expected, "rolls {rolls}");
        let (reference, _) = straight_line(&program, "");
        assert_eq!(output, reference, "rolls {rolls}");
    }
}

#[test]
fn a_roll_of_each_depth_the_table_covers_agrees() {
    for depth in 2..=8 {
        let mut program = (1..=depth).map(push).collect::<Vec<_>>();
        program.extend([push(depth), push(1), step(Command::Roll)]);
        for _ in 0..depth {
            program.push(step(Command::OutNumber));
        }
        let (output, _) = walk(&program);
        let (reference, _) = straight_line(&program, "");
        assert_eq!(output, reference, "depth {depth}");
    }
}

#[test]
fn a_roll_of_one_is_a_no_op_but_consumes_its_operands() {
    let program = [
        push(7),
        push(1),
        push(1),
        step(Command::Roll),
        step(Command::OutNumber),
    ];
    let (output, _) = walk(&program);
    assert_eq!(output, "7");
    let (reference, _) = straight_line(&program, "");
    assert_eq!(output, reference);
}

#[test]
fn a_roll_deeper_than_the_values_is_ignored() {
    // Depth 5 with one value left: the reference ignores it and consumes both.
    let program = [
        push(7),
        push(5),
        push(1),
        step(Command::Roll),
        step(Command::OutNumber),
    ];
    let (output, _) = walk(&program);
    assert_eq!(output, "7");
    let (reference, _) = straight_line(&program, "");
    assert_eq!(output, reference);
}

#[test]
fn a_roll_past_the_table_stops() {
    // Nine is deeper than the table covers. Stopping is the documented boundary:
    // the alternative is rotating the ring instead of the values.
    let mut program = (1..=9).map(push).collect::<Vec<_>>();
    program.extend([push(9), push(1), step(Command::Roll)]);
    for _ in 0..9 {
        program.push(step(Command::OutNumber));
    }
    let (output, result) = walk(&program);
    assert_eq!(output, "", "it stops before printing anything");
    assert_eq!(result.reason, ExitReason::Terminated);
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
        let (expected, reference) = straight_line(&program, "");
        let (actual, walked) = walk(&program);
        assert_eq!(actual, expected, "seed {seed}: {program:?}");
        assert_eq!(walked.reason, reference.reason, "seed {seed}");
    }
}

#[test]
fn a_command_without_a_handler_is_refused() {
    // `pointer` and `switch` have no opcode: a straight line cannot run them.
    assert!(matches!(
        walker(&[step(Command::Pointer)], ""),
        Err(Error::NoHandler {
            command: Command::Pointer
        })
    ));
}

#[test]
fn an_in_char_pushes_the_character_it_reads() {
    let program = [step(Command::InChar), step(Command::OutChar)];
    let (output, result) = walk_with(&program, "A");
    assert_eq!(output, "A");
    assert_eq!(result.reason, ExitReason::Terminated);
    let (expected, _) = straight_line(&program, "A");
    assert_eq!(output, expected);
}

#[test]
fn a_digit_can_be_read_as_a_character() {
    // The separator is what keeps a digit from being read as a number.
    let program = [step(Command::InChar), step(Command::OutChar)];
    let (output, _) = walk_with(&program, "7");
    assert_eq!(output, "7");
    let (expected, _) = straight_line(&program, "7");
    assert_eq!(output, expected);
}

#[test]
fn an_in_number_pushes_the_number_it_reads() {
    let program = [step(Command::InNumber), step(Command::OutNumber)];
    let (output, _) = walk_with(&program, "  -42 ");
    assert_eq!(output, "-42");
    let (expected, _) = straight_line(&program, "  -42 ");
    assert_eq!(output, expected);
}

#[test]
fn input_events_are_read_in_order() {
    // A character, then a number, then a character.
    let program = [
        step(Command::InChar),
        step(Command::OutNumber),
        step(Command::InNumber),
        step(Command::OutChar),
    ];
    let (output, _) = walk_with(&program, "A 42B");
    assert_eq!(output, "65*");
    let (expected, _) = straight_line(&program, "A 42B");
    assert_eq!(output, expected);
}

#[test]
fn input_that_runs_out_is_refused() {
    assert!(matches!(
        walker(&[step(Command::InChar)], ""),
        Err(Error::InputExhausted)
    ));
    assert!(matches!(
        walker(&[step(Command::InNumber)], ""),
        Err(Error::InputExhausted)
    ));
}

/// A program that reads, and an input built alongside it so every read has
/// something to read. A character is written bare, a number after a space, so
/// the reference and the encoder consume the same values.
fn random_reader(seed: u64, length: usize) -> (Vec<Instruction>, String) {
    let commands = [
        Command::InChar,
        Command::OutChar,
        Command::InNumber,
        Command::OutNumber,
        Command::Push,
        Command::Add,
        Command::Subtract,
        Command::Duplicate,
        Command::Pop,
        Command::Not,
    ];
    let mut random = XorShift(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
    let mut input = String::new();
    let mut program = Vec::with_capacity(length);
    for _ in 0..length {
        let command = commands[(random.next() % commands.len() as u64) as usize];
        match command {
            Command::InChar => input.push(char::from(b'a' + (random.next() % 26) as u8)),
            Command::InNumber => {
                input.push(' ');
                input.push_str(&(random.next() % 90 + 1).to_string());
            }
            _ => {}
        }
        program.push(Instruction {
            command,
            block_size: if command == Command::Push {
                random.next() % 9 + 1
            } else {
                1
            },
        });
    }
    (program, input)
}

#[test]
fn reading_programs_agree_with_the_straight_line_reference() {
    for seed in 0..24 {
        let (program, input) = random_reader(seed, 20);
        let (expected, reference) = straight_line(&program, &input);
        let (actual, walked) = walk_with(&program, &input);
        assert_eq!(actual, expected, "seed {seed}: {program:?} on {input:?}");
        assert_eq!(walked.reason, reference.reason, "seed {seed}");
    }
}
