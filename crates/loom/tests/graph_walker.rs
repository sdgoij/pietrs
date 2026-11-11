#![allow(clippy::unwrap_used)]
//! A program's state graph, run from one fixed walker image.
//!
//! The graph decides where to go next, so this is the case where the image —
//! not the host — picks the successor. Every case is checked against `interp`
//! running the original image.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::graph_walker;
use runtime::BufferIo;
use tessera::StateGraph;

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

/// Runs a fixture's graph from the walker, on an input of code points.
fn walk(png: &[u8], input: &str, steps: u64) -> (String, Run) {
    let grid = CodelGrid::from_image(png).unwrap();
    let blocks = Blocks::partition(grid);
    let graph = StateGraph::build(&blocks);
    let (image, stream) = graph_walker(&graph, &blocks, input).unwrap();
    run(&image, &stream, steps)
}

#[test]
fn a_terminating_graph_runs_to_the_same_output() {
    // The canonical hello world: a straight graph, run from records.
    let png = include_bytes!("../fixtures/hello-world/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let (expected, result) = run(&source, "", 10_000);
    assert_eq!(result.reason, ExitReason::Terminated);
    assert_eq!(expected, "Hello world!");

    let (actual, result) = walk(png, "", 4_000_000);
    assert_eq!(actual, expected);
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn a_branching_graph_makes_the_same_decisions() {
    // `pointer` and `switch`, selected at run time from the popped value. The
    // reference reads "A0" as two characters; the walker's input is their code
    // points, because a graph chooses its reads at run time.
    let png = include_bytes!("../../xtask/fixtures/pointer/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let (expected, _) = run(&source, "A0", 40);
    assert!(!expected.is_empty(), "the reference printed nothing");

    let (actual, _) = walk(png, "65 48", 4_000_000);
    assert!(!actual.is_empty(), "the walker printed nothing");
    let shorter = expected.len().min(actual.len());
    assert_eq!(&actual[..shorter], &expected[..shorter]);
}

#[test]
fn a_graph_with_no_successor_halts_immediately() {
    // The single-codel fixture terminates without running a command.
    let png = include_bytes!("../../xtask/fixtures/single/program.png");
    let (output, result) = walk(png, "", 4_000_000);
    assert_eq!(output, "");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn a_looping_graph_keeps_looping() {
    // A graph with a loop: both sides print prefixes of the same endless run.
    let png = include_bytes!("../../xtask/fixtures/hello/program.png");
    let source = CodelGrid::from_image(png).unwrap();
    let (expected, _) = run(&source, "", 400);
    assert!(expected.len() > 1, "the reference printed {expected:?}");
    assert!(expected.chars().all(|c| c == 'A'));

    let (actual, _) = walk(png, "", 4_000_000);
    assert!(actual.len() >= 8, "the walker printed {actual:?}");
    let shorter = expected.len().min(actual.len());
    assert_eq!(&actual[..shorter], &expected[..shorter]);
}
