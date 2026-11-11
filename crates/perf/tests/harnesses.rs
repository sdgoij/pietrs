#![allow(clippy::unwrap_used)]
//! Ignored wall-clock harnesses.
//!
//! Every case is `#[ignore]`d and asserts nothing: a duration is a property of
//! the machine, not of the code. `just perf` runs them in release, which is the
//! only build where the numbers mean anything. The gate's `--include-ignored`
//! step runs them in debug, where the numbers are meaningless but the harnesses
//! still have to compile and finish.
//!
//! Results belong in `.agents/notes/perf.md`.

#[global_allocator]
static COUNTER: perf::Counter = perf::Counter::new();

use canvas::Blocks;
use runtime::{BufferIo, Division};

/// Large enough that the per-codel work dominates the harness overhead.
const PARTITION_SIDE: u32 = 320;
/// Smaller: the graph has up to eight states per block.
const GRAPH_SIDE: u32 = 96;
/// Smaller still: Cranelift is the slow part.
const OBJECT_SIDE: u32 = 32;
/// A million transitions, so the rate is not dominated by startup.
const SIMULATED_STEPS: u64 = 1_000_000;
/// Tape sizes the generated-interpreter plan has to budget for: the canonical
/// hello world's state graph, Piet's Quest's state graph, and Piet's Quest's
/// codel grid.
const TAPE_DEPTHS: [u64; 3] = [25, 17_964, 475_867];
/// Rolls per harness case, so the deepest tape is not minutes long.
const TAPE_ROLLS: u64 = 20_000_000;

#[test]
#[ignore = "wall-clock harness, not a correctness gate; run `just perf`"]
fn partition_a_large_grid() {
    let grid = perf::grid(PARTITION_SIDE);
    let (blocks, measurement) = perf::measure(&COUNTER, || Blocks::partition(grid));
    perf::report(
        &format!(
            "partition {PARTITION_SIDE}x{PARTITION_SIDE} ({} blocks)",
            blocks.len()
        ),
        &measurement,
    );
}

#[test]
#[ignore = "wall-clock harness, not a correctness gate; run `just perf`"]
fn build_a_large_graph() {
    let blocks = Blocks::partition(perf::grid(GRAPH_SIDE));
    let (graph, measurement) = perf::measure(&COUNTER, || tessera::StateGraph::build(&blocks));
    perf::report(
        &format!(
            "graph {GRAPH_SIDE}x{GRAPH_SIDE} ({} blocks, {} states, {} edges)",
            blocks.len(),
            graph.states().len(),
            graph.edges().len()
        ),
        &measurement,
    );
}

#[test]
#[ignore = "wall-clock harness, not a correctness gate; run `just perf`"]
fn emit_an_object() {
    let blocks = Blocks::partition(perf::grid(OBJECT_SIDE));
    let (object, measurement) = perf::measure(&COUNTER, || {
        codegen::emit_object(&blocks, Division::Floored).expect("the host object emits")
    });
    perf::report(
        &format!(
            "codegen {OBJECT_SIDE}x{OBJECT_SIDE} ({} blocks, {} KiB object)",
            blocks.len(),
            object.len() / 1024
        ),
        &measurement,
    );
}

#[test]
#[ignore = "wall-clock harness, not a correctness gate; run `just perf`"]
fn roll_a_tape() {
    for depth in TAPE_DEPTHS {
        let steps = (TAPE_ROLLS / depth).clamp(1_000, 1_000_000);
        let mut stack = runtime::Stack::new(depth as usize);
        for value in 0..depth {
            stack.push(value as i64).unwrap();
        }
        let (_, measurement) = perf::measure(&COUNTER, || {
            for _ in 0..steps {
                stack.roll(depth as i64, 1).unwrap();
            }
        });
        perf::report(
            &format!(
                "roll a {depth}-value tape, {steps} rolls ({} rolls per second)",
                perf::per_second(steps, measurement.elapsed)
            ),
            &measurement,
        );
    }
}

#[test]
#[ignore = "wall-clock harness, not a correctness gate; run `just perf`"]
fn simulate_a_looping_program() {
    let blocks = testkit::row(&[tessera::Command::Add, tessera::Command::Subtract]);
    let mut io = BufferIo::new("");
    let options = interp::Options {
        stack_capacity: 64,
        max_steps: Some(SIMULATED_STEPS),
        division: Division::Floored,
    };
    let (run, measurement) = perf::measure(&COUNTER, || {
        interp::run(&blocks, &mut io, &options).unwrap()
    });
    perf::report(
        &format!(
            "interp {} steps ({} transitions per second)",
            run.steps,
            perf::per_second(run.steps, measurement.elapsed)
        ),
        &measurement,
    );
}
