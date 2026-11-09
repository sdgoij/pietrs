#![allow(clippy::unwrap_used)]
//! Reference runs over hand-built grids.

use canvas::{BlockId, Blocks, CodelGrid, Color, Hue, Lightness, Point};
use interp::{ExitReason, Options, Step, run, run_traced};
use runtime::BufferIo;
use tessera::{Chooser, Command, Direction};

fn blocks(codels: Vec<Color>, width: u32) -> Blocks {
    let height = (codels.len() as u32) / width;
    Blocks::partition(CodelGrid::from_colors(width, height, codels).unwrap())
}

fn id_at(blocks: &Blocks, x: u32, y: u32) -> BlockId {
    blocks.block_id_at(Point::new(x, y)).unwrap()
}

fn print_a_program() -> Blocks {
    let mut codels = vec![Color::chromatic(Hue::Red, Lightness::Normal); 65];
    codels.push(Color::chromatic(Hue::Red, Lightness::Dark));
    codels.push(Color::chromatic(Hue::Magenta, Lightness::Normal));
    blocks(codels, 67)
}

#[test]
fn a_single_block_terminates_without_output() {
    let blocks = blocks(vec![Color::chromatic(Hue::Red, Lightness::Normal)], 1);
    let mut io = BufferIo::new("");
    let result = run(&blocks, &mut io, &Options::default()).unwrap();
    assert_eq!(result.reason, ExitReason::Terminated);
    assert_eq!(result.steps, 0);
    assert_eq!(io.output(), "");
}

#[test]
fn a_push_then_out_char_prints_a() {
    let blocks = print_a_program();
    let mut io = BufferIo::new("");
    let options = Options {
        max_steps: Some(2),
        ..Options::default()
    };
    let result = run(&blocks, &mut io, &options).unwrap();
    assert_eq!(result.reason, ExitReason::StepLimit);
    assert_eq!(result.steps, 2);
    assert_eq!(io.output(), "A");
}

#[test]
fn the_trace_records_each_transition() {
    let blocks = print_a_program();
    let red_normal = id_at(&blocks, 0, 0);
    let red_dark = id_at(&blocks, 65, 0);
    let magenta = id_at(&blocks, 66, 0);
    let mut io = BufferIo::new("");
    let options = Options {
        max_steps: Some(2),
        ..Options::default()
    };
    let (result, trace) = run_traced(&blocks, &mut io, &options).unwrap();
    assert_eq!(result.steps, 2);
    assert_eq!(trace.len(), 2);
    assert_eq!(
        trace[0],
        Step {
            block: red_normal,
            dp: Direction::Right,
            cc: Chooser::Left,
            command: Some(Command::Push),
            target: Some(red_dark),
        }
    );
    assert_eq!(
        trace[1],
        Step {
            block: red_dark,
            dp: Direction::Right,
            cc: Chooser::Left,
            command: Some(Command::OutChar),
            target: Some(magenta),
        }
    );
}

#[test]
fn a_white_slide_runs_no_command() {
    let mut codels = vec![Color::chromatic(Hue::Red, Lightness::Normal)];
    codels.extend([Color::White; 3]);
    codels.push(Color::chromatic(Hue::Blue, Lightness::Normal));
    let blocks = blocks(codels, 5);
    let blue = id_at(&blocks, 4, 0);
    let mut io = BufferIo::new("");
    let options = Options {
        max_steps: Some(1),
        ..Options::default()
    };
    let (result, trace) = run_traced(&blocks, &mut io, &options).unwrap();
    assert_eq!(result.reason, ExitReason::StepLimit);
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0].command, None);
    assert_eq!(trace[0].target, Some(blue));
}

#[test]
fn a_terminating_white_dead_end_stops_the_run() {
    let codels = vec![
        Color::chromatic(Hue::Red, Lightness::Normal),
        Color::White,
        Color::Black,
        Color::White,
    ];
    let blocks = blocks(codels, 2);
    let mut io = BufferIo::new("");
    let result = run(&blocks, &mut io, &Options::default()).unwrap();
    assert_eq!(result.reason, ExitReason::Terminated);
    assert_eq!(result.steps, 0);
}
