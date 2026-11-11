#![allow(clippy::unwrap_used)]
//! The generated image runs a program's state graph, including its branches.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::compile;
use runtime::BufferIo;

fn run(grid: &CodelGrid, input: &str, steps: u64) -> (String, Run) {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let options = Options {
        max_steps: Some(steps),
        ..Options::default()
    };
    let result = interp::run(&blocks, &mut io, &options).unwrap();
    (io.into_output(), result)
}

fn image_of(png: &[u8]) -> CodelGrid {
    let grid = CodelGrid::from_image(png).expect("the fixture decodes");
    let blocks = Blocks::partition(grid);
    let program =
        compile(&tessera::StateGraph::build(&blocks), &blocks).expect("the graph compiles");
    program.image().expect("the graph lays out")
}

#[test]
fn a_terminating_graph_runs_to_the_same_output() {
    // The canonical hello world: a straight graph, but now the image follows it.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let (expected, result) = run(&source, "", 10_000);
    assert_eq!(result.reason, ExitReason::Terminated);
    assert_eq!(expected, "Hello world!");

    let (actual, result) = run(&image_of(png), "", 1_000_000);
    assert_eq!(actual, expected);
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn a_branching_graph_makes_the_same_decisions() {
    // `pointer` and `switch`, selected at run time from the popped value.
    let png = include_bytes!("../../xtask/fixtures/pointer/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let (expected, _) = run(&source, "A0", 40);

    let (actual, _) = run(&image_of(png), "A0", 2_000_000);
    assert!(!actual.is_empty());
    let shorter = expected.len().min(actual.len());
    assert_eq!(&actual[..shorter], &expected[..shorter]);
}

#[test]
fn a_looping_graph_keeps_looping() {
    // The checkpoint: a graph with a loop, run from the graph rather than from
    // a baked trace. Both sides print prefixes of the same endless run.
    let png = include_bytes!("../../xtask/fixtures/hello/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let (expected, _) = run(&source, "", 400);
    assert!(expected.len() > 1, "the reference printed {expected:?}");
    assert!(expected.chars().all(|c| c == 'A'));

    let (actual, _) = run(&image_of(png), "", 2_000_000);
    assert!(actual.len() >= 16, "the image printed {actual:?}");
    let shorter = expected.len().min(actual.len());
    assert_eq!(&actual[..shorter], &expected[..shorter]);
}

#[test]
fn a_graph_with_no_successor_halts_immediately() {
    // The single-codel fixture terminates without running a command.
    let png = include_bytes!("../../xtask/fixtures/single/program.png");
    let (output, result) = run(&image_of(png), "", 1_000_000);
    assert_eq!(output, "");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_graph_image_stays_small() {
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let image = image_of(png);
    // One lane per state, five cells per dispatch test, and the chain blocks.
    assert!(image.width() <= 512, "width {}", image.width());
    assert!(image.height() <= 256, "height {}", image.height());
}
