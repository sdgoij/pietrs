//! Compiling a tessellation graph into the structured backend.
//!
//! This is where the generated image stops replaying a trace. The host still
//! resolves the geometry — which block the direction pointer reaches and how
//! the codel chooser picks a codel among its codels — because that is a static
//! fact about the image, and `tessera` computes it once. What the image decides
//! at run time is *where to go next*: a `pointer` or `switch` state pops a
//! value and selects one of its successors with it.
//!
//! One block per state. A block runs the state's command and then jumps to its
//! successor; a state with several successors tests the popped value against
//! each in turn and jumps to the matching one. A state with no successors
//! halts.

use std::collections::HashMap;

use canvas::Blocks;
use tessera::{Branch, Command, Edge, State, StateGraph};

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};

/// Compiles a state graph into a program, using the partition for block sizes.
pub fn compile(graph: &StateGraph, blocks: &Blocks) -> Result<Program, Error> {
    let Some(start) = graph.start() else {
        // The program could not even start: one block that halts.
        return Program::new(vec![Block::new(Vec::new(), End::Halt)], 1);
    };

    let ids = graph
        .states()
        .iter()
        .enumerate()
        .map(|(index, state)| (*state, index as u32 + 1))
        .collect::<HashMap<State, u32>>();
    let entry = ids.get(&start).copied().ok_or(Error::NoLayout)?;

    let mut builder = Builder {
        blocks: vec![Block::new(Vec::new(), End::Halt); graph.states().len()],
        ids,
        graph,
        partition: blocks,
    };
    for (index, state) in graph.states().iter().enumerate() {
        builder.blocks[index] = builder.state_block(*state)?;
    }
    Program::new(builder.blocks, entry)
}

struct Builder<'a> {
    blocks: Vec<Block>,
    ids: HashMap<State, u32>,
    graph: &'a StateGraph,
    partition: &'a Blocks,
}

impl Builder<'_> {
    fn state_block(&mut self, state: State) -> Result<Block, Error> {
        let edges = self.graph.successors(state).copied().collect::<Vec<_>>();
        let Some(first) = edges.first().copied() else {
            // A state with no successor terminates, and runs no command.
            return Ok(Block::new(Vec::new(), End::Halt));
        };

        let size = self
            .partition
            .block(state.block)
            .map_or(0, |block| block.size() as i64);
        let mut nodes = match first.command {
            None => Vec::new(),
            Some(Command::Push) => vec![Node::Const(size)],
            Some(Command::Pointer | Command::Switch) => Vec::new(),
            Some(other) => vec![Node::Command(other)],
        };

        let end = match first.branch {
            Branch::Deterministic => End::Jump(self.id_of(first.to)?),
            Branch::Pointer(_) => {
                nodes.extend([Node::Const(4), Node::Command(Command::Modulo)]);
                nodes.extend(test_nodes(0));
                self.chain(&edges)?
            }
            Branch::Switch(_) => {
                nodes.extend([Node::Const(2), Node::Command(Command::Modulo)]);
                nodes.extend(test_nodes(0));
                self.chain(&edges)?
            }
        };
        Ok(Block::new(nodes, end))
    }

    /// The blocks a branch needs: one trampoline per successor, and one test
    /// per successor except the first (taken inline) and the last (the
    /// fall-through).
    fn chain(&mut self, edges: &[Edge]) -> Result<End, Error> {
        let count = edges.len() as u32;
        if count < 2 {
            return Err(Error::NoLayout);
        }
        let base = self.blocks.len() as u32 + 1;

        let mut blocks = Vec::new();
        for edge in edges {
            blocks.push(Block::new(
                // Drop the selector the chain has been carrying.
                vec![Node::Command(Command::Pop)],
                End::Jump(self.id_of(edge.to)?),
            ));
        }
        for index in 1..count - 1 {
            let then = base + index;
            let otherwise = if index + 1 == count - 1 {
                base + count - 1
            } else {
                base + count + index
            };
            blocks.push(Block::new(
                test_nodes(i64::from(index)),
                End::If { then, otherwise },
            ));
        }
        self.blocks.extend(blocks);

        Ok(End::If {
            then: base,
            // With two successors the first test is the whole chain.
            otherwise: if count > 2 { base + count } else { base + 1 },
        })
    }

    fn id_of(&self, state: State) -> Result<u32, Error> {
        self.ids.get(&state).copied().ok_or(Error::NoLayout)
    }
}

/// `v == value`, leaving `v` and the answer on the stack.
///
/// One `not` is the equality test: `not(v - value)` is one exactly when they
/// are equal. Two would be the inequality test and would pick the other arm.
fn test_nodes(value: i64) -> Vec<Node> {
    vec![
        Node::Dup,
        Node::Const(value),
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
    ]
}
