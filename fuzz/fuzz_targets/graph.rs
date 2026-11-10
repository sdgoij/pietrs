#![no_main]

//! `tessera` graph construction: the movement rules over an arbitrary grid.

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;

#[path = "grid.rs"]
mod grid;

fuzz_target!(|data: &[u8]| {
    let Some(codel_grid) = grid::grid(data, 12) else {
        return;
    };
    let (width, height) = (codel_grid.width(), codel_grid.height());
    let blocks = canvas::Blocks::partition(codel_grid);
    grid::assert_partition(width, height, &blocks);

    let graph = tessera::StateGraph::build(&blocks);
    let states: HashSet<tessera::State> = graph.states().iter().copied().collect();
    for edge in graph.edges() {
        assert!(states.contains(&edge.from), "an edge starts at a state");
        assert!(states.contains(&edge.to), "an edge ends at a state");
    }
    if let Some(start) = graph.start() {
        assert!(states.contains(&start), "the start state is in the graph");
    }
    assert!(
        graph.states().len() <= 8 * blocks.len(),
        "a block has at most eight (DP, CC) states"
    );
    // The graph is a pure function of the image.
    assert_eq!(
        graph,
        tessera::StateGraph::build(&blocks),
        "the graph is deterministic"
    );
});
