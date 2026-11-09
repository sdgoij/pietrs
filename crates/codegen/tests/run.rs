#![allow(clippy::unwrap_used)]
//! Differential tests: the JIT and the interpreter must agree.

use canvas::{Blocks, CodelGrid, Color, Hue, Lightness};
use codegen::{ExitReason as JitExit, Options as JitOptions, run as run_jit};
use interp::{ExitReason as InterpExit, Options as InterpOptions, run as run_interp};
use runtime::{BufferIo, Division};
use tessera::{Command, command};

fn blocks(codels: Vec<Color>, width: u32) -> Blocks {
    let height = (codels.len() as u32) / width;
    Blocks::partition(CodelGrid::from_colors(width, height, codels).unwrap())
}

/// The forward deltas the command table assigns to each command.
fn deltas(wanted: Command) -> (u8, u8) {
    match wanted {
        Command::Push => (0, 1),
        Command::Pop => (0, 2),
        Command::Add => (1, 0),
        Command::Subtract => (1, 1),
        Command::Multiply => (1, 2),
        Command::Divide => (2, 0),
        Command::Modulo => (2, 1),
        Command::Not => (2, 2),
        Command::Greater => (3, 0),
        Command::Pointer => (3, 1),
        Command::Switch => (3, 2),
        Command::Duplicate => (4, 0),
        Command::Roll => (4, 1),
        Command::InNumber => (4, 2),
        Command::InChar => (5, 0),
        Command::OutNumber => (5, 1),
        Command::OutChar => (5, 2),
    }
}

/// A single-row program whose transitions run `commands` in order.
///
/// Every codel is its own block, so the row walks left to right and each
/// transition is the command the test asked for.
fn row(commands: &[Command]) -> Blocks {
    let mut colours = vec![Color::chromatic(Hue::Red, Lightness::Normal)];
    for &wanted in commands {
        let from = *colours.last().unwrap();
        let deltas = deltas(wanted);
        let to = Color::ALL
            .into_iter()
            .find(|colour| from.deltas(*colour) == Some(deltas))
            .unwrap();
        colours.push(to);
    }
    for (index, &wanted) in commands.iter().enumerate() {
        assert_eq!(
            command(colours[index], colours[index + 1]),
            Some(wanted),
            "the fixture must run {wanted:?} at step {index}"
        );
    }
    let width = colours.len() as u32;
    blocks(colours, width)
}

/// Runs both paths and asserts they agree, returning the output.
fn assert_paths_agree_on(blocks: &Blocks, input: &str, max_steps: u64) -> String {
    assert_paths_agree_on_with(blocks, input, max_steps, Division::Floored)
}

fn assert_paths_agree_on_with(
    blocks: &Blocks,
    input: &str,
    max_steps: u64,
    division: Division,
) -> String {
    let mut interpreter_io = BufferIo::new(input);
    let interpreter = run_interp(
        blocks,
        &mut interpreter_io,
        &InterpOptions {
            max_steps: Some(max_steps),
            division,
            ..InterpOptions::default()
        },
    )
    .unwrap();

    let mut jit_io = BufferIo::new(input);
    let jit = run_jit(
        blocks,
        &mut jit_io,
        &JitOptions {
            stack_capacity: 1 << 20,
            max_steps: Some(max_steps),
            division,
        },
    )
    .unwrap();

    assert_eq!(jit_io.output(), interpreter_io.output());
    assert_eq!(jit.steps, interpreter.steps);
    let reason = match jit.reason {
        JitExit::Terminated => InterpExit::Terminated,
        JitExit::StepLimit => InterpExit::StepLimit,
        JitExit::StackOverflow => panic!("the JIT overflowed the stack"),
    };
    assert_eq!(reason, interpreter.reason);
    interpreter_io.into_output()
}

fn assert_paths_agree(blocks: &Blocks, max_steps: u64) {
    assert_paths_agree_on(blocks, "", max_steps);
}

fn assert_row_agrees(commands: &[Command], input: &str) -> String {
    assert_paths_agree_on(&row(commands), input, commands.len() as u64)
}

#[test]
fn truncating_division_agrees() {
    let divide = [
        Command::InNumber,
        Command::InNumber,
        Command::Divide,
        Command::OutNumber,
    ];
    let modulo = [
        Command::InNumber,
        Command::InNumber,
        Command::Modulo,
        Command::OutNumber,
    ];
    assert_eq!(
        assert_paths_agree_on_with(&row(&divide), "-7 3", 4, Division::Truncating),
        "-2"
    );
    assert_eq!(
        assert_paths_agree_on_with(&row(&modulo), "-7 3", 4, Division::Truncating),
        "-1"
    );
    assert_eq!(
        assert_paths_agree_on_with(&row(&divide), "-7 3", 4, Division::Floored),
        "-3"
    );
    assert_eq!(
        assert_paths_agree_on_with(&row(&modulo), "-7 3", 4, Division::Floored),
        "2"
    );
}

#[test]
fn a_single_block_agrees() {
    let blocks = blocks(vec![Color::chromatic(Hue::Red, Lightness::Normal)], 1);
    assert_paths_agree(&blocks, 1);
}

#[test]
fn push_and_out_char_agree() {
    let mut codels = vec![Color::chromatic(Hue::Red, Lightness::Normal); 65];
    codels.push(Color::chromatic(Hue::Red, Lightness::Dark));
    codels.push(Color::chromatic(Hue::Magenta, Lightness::Normal));
    assert_paths_agree(&blocks(codels, 67), 2);
}

#[test]
fn push_push_add_out_number_agree() {
    let blocks = blocks(
        vec![
            Color::chromatic(Hue::Red, Lightness::Normal),
            Color::chromatic(Hue::Red, Lightness::Dark),
            Color::chromatic(Hue::Red, Lightness::Light),
            Color::chromatic(Hue::Yellow, Lightness::Light),
            Color::chromatic(Hue::Red, Lightness::Normal),
        ],
        5,
    );
    assert_paths_agree(&blocks, 4);
}

#[test]
fn a_white_slide_agrees() {
    let mut codels = vec![Color::chromatic(Hue::Red, Lightness::Normal)];
    codels.extend([Color::White; 3]);
    codels.push(Color::chromatic(Hue::Blue, Lightness::Normal));
    assert_paths_agree(&blocks(codels, 5), 1);
}

#[test]
fn a_white_dead_end_agrees() {
    let blocks = blocks(
        vec![
            Color::chromatic(Hue::Red, Lightness::Normal),
            Color::White,
            Color::Black,
            Color::White,
        ],
        2,
    );
    assert_paths_agree(&blocks, 1);
}

#[test]
fn push_pop_and_out_number_agree() {
    let output = assert_row_agrees(
        &[
            Command::Push,
            Command::Push,
            Command::Pop,
            Command::OutNumber,
        ],
        "",
    );
    assert_eq!(output, "1");
}

#[test]
fn addition_subtraction_and_multiplication_agree() {
    assert_eq!(
        assert_row_agrees(
            &[
                Command::InNumber,
                Command::InNumber,
                Command::Subtract,
                Command::OutNumber,
            ],
            "10 3",
        ),
        "7"
    );
    assert_eq!(
        assert_row_agrees(
            &[
                Command::InNumber,
                Command::InNumber,
                Command::Multiply,
                Command::OutNumber,
            ],
            "6 7",
        ),
        "42"
    );
    assert_eq!(
        assert_row_agrees(
            &[
                Command::InNumber,
                Command::InNumber,
                Command::Add,
                Command::OutNumber,
            ],
            "-7 3",
        ),
        "-4"
    );
}

#[test]
fn floored_division_and_modulo_agree() {
    let divide = [
        Command::InNumber,
        Command::InNumber,
        Command::Divide,
        Command::OutNumber,
    ];
    let modulo = [
        Command::InNumber,
        Command::InNumber,
        Command::Modulo,
        Command::OutNumber,
    ];
    for (a, b, quotient, remainder) in [
        ("7", "3", "2", "1"),
        ("-7", "3", "-3", "2"),
        ("7", "-3", "-3", "-2"),
        ("-7", "-3", "2", "-1"),
    ] {
        let input = format!("{a} {b}");
        assert_eq!(assert_row_agrees(&divide, &input), quotient, "{input}");
        assert_eq!(assert_row_agrees(&modulo, &input), remainder, "{input}");
    }
}

#[test]
fn division_by_zero_consumes_both_operands_on_both_paths() {
    let output = assert_row_agrees(
        &[
            Command::InNumber,
            Command::InNumber,
            Command::Divide,
            Command::OutNumber,
        ],
        "7 0",
    );
    assert_eq!(output, "", "no quotient is pushed");
}

#[test]
fn modulo_by_zero_consumes_both_operands_on_both_paths() {
    let output = assert_row_agrees(
        &[
            Command::InNumber,
            Command::InNumber,
            Command::Modulo,
            Command::OutNumber,
        ],
        "7 0",
    );
    assert_eq!(output, "");
}

#[test]
fn dividing_the_minimum_by_minus_one_consumes_both_operands() {
    let output = assert_row_agrees(
        &[
            Command::InNumber,
            Command::InNumber,
            Command::Divide,
            Command::OutNumber,
        ],
        "-9223372036854775808 -1",
    );
    assert_eq!(output, "");
}

#[test]
fn modulo_of_the_minimum_by_minus_one_is_zero_on_both_paths() {
    let output = assert_row_agrees(
        &[
            Command::InNumber,
            Command::InNumber,
            Command::Modulo,
            Command::OutNumber,
        ],
        "-9223372036854775808 -1",
    );
    assert_eq!(output, "0");
}

#[test]
fn not_and_greater_agree() {
    assert_eq!(
        assert_row_agrees(&[Command::InNumber, Command::Not, Command::OutNumber], "0",),
        "1"
    );
    assert_eq!(
        assert_row_agrees(&[Command::InNumber, Command::Not, Command::OutNumber], "5",),
        "0"
    );
    assert_eq!(
        assert_row_agrees(
            &[
                Command::InNumber,
                Command::InNumber,
                Command::Greater,
                Command::OutNumber,
            ],
            "3 2",
        ),
        "1"
    );
    assert_eq!(
        assert_row_agrees(
            &[
                Command::InNumber,
                Command::InNumber,
                Command::Greater,
                Command::OutNumber,
            ],
            "2 2",
        ),
        "0"
    );
}

#[test]
fn duplicate_agrees_with_the_interpreter() {
    let output = assert_row_agrees(
        &[
            Command::InNumber,
            Command::Duplicate,
            Command::Add,
            Command::OutNumber,
        ],
        "21",
    );
    assert_eq!(output, "42");
}

#[test]
fn roll_agrees_with_the_interpreter() {
    let output = assert_row_agrees(
        &[
            Command::InNumber,
            Command::InNumber,
            Command::InNumber,
            Command::InNumber,
            Command::InNumber,
            Command::Roll,
            Command::OutNumber,
            Command::OutNumber,
            Command::OutNumber,
        ],
        "10 20 30 3 1",
    );
    assert_eq!(output, "201030");
}

#[test]
fn input_and_output_characters_agree() {
    let output = assert_row_agrees(&[Command::InChar, Command::OutChar], "Z");
    assert_eq!(output, "Z");
}

/// A grid that runs `in(char)`, `in(number)`, and `pointer`, then reaches one
/// of three arms: `out(char)` to the right, `out(number)` below, and `switch`
/// to the left. Moving up is off the grid, so the fallback reaches the right
/// arm.
fn pointer_grid() -> Blocks {
    let normal = Lightness::Normal;
    let light = Lightness::Light;
    let dark = Lightness::Dark;
    blocks(
        vec![
            Color::chromatic(Hue::Red, normal),
            Color::chromatic(Hue::Magenta, normal),
            Color::chromatic(Hue::Cyan, light),
            Color::chromatic(Hue::Red, normal),
            Color::chromatic(Hue::Magenta, light),
            Color::Black,
            Color::Black,
            Color::Black,
            Color::chromatic(Hue::Magenta, dark),
            Color::Black,
        ],
        5,
    )
}

#[test]
fn pointer_rotates_the_direction_pointer_on_both_paths() {
    for (value, output) in [(0, "A"), (1, "65"), (2, ""), (3, "A")] {
        let input = format!("A{value}");
        assert_eq!(
            assert_paths_agree_on(&pointer_grid(), &input, 4),
            output,
            "pointer value {value}"
        );
    }
}

#[test]
fn switch_toggles_the_codel_chooser_on_both_paths() {
    // `A` is 65 (odd) and `B` is 66 (even); pointer value 2 reaches `switch`.
    assert_eq!(assert_paths_agree_on(&pointer_grid(), "A2", 4), "");
    assert_eq!(assert_paths_agree_on(&pointer_grid(), "B2", 4), "");
}
