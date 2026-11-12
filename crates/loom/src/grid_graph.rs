//! Building the walker's records from the image's moves.
//!
//! The grid image resolves the movement rules from pixels, so this walk builds
//! the same state graph [`StateGraph::build`] does — one state per
//! `(block, dp, cc)`, its outgoing command and its successors — and hands it to
//! [`crate::graph_walker`]. The host still supplies the record's bookkeeping:
//! the block sizes and the ring offsets.

use std::collections::HashMap;

use canvas::{BlockId, Blocks, CodelGrid};
use interp::{ExitReason, Options};
use runtime::BufferIo;
use tessera::{Chooser, Direction, Exit, Resolver, Start, State, StateGraph};

use crate::error::Error;
use crate::grid::{exit_image, grid_ring, ring_index, start_image, with_exit_query};
use crate::walker::record_command;

/// Builds the state graph reachable from the upper-left codel, resolving every
/// move with the grid image.
///
/// `steps` is the budget for one image run; a grid that needs more is reported
/// as [`Error::StepLimit`].
pub fn grid_graph(grid: &CodelGrid, steps: u64) -> Result<StateGraph, Error> {
    let resolver = GridResolver::new(grid, steps)?;
    StateGraph::build_with(&resolver)
}

/// The image as a resolver of the movement rules.
struct GridResolver {
    /// The ring label the image prints for a block, back to the host's id.
    by_label: HashMap<u32, BlockId>,
    /// The label to ask the images about, for a block the host named.
    by_block: HashMap<BlockId, u32>,
    /// The ring stream, printed once by the ring image for the whole build.
    ring: String,
    start: Blocks,
    exit: Blocks,
    steps: u64,
}

impl GridResolver {
    fn new(grid: &CodelGrid, steps: u64) -> Result<Self, Error> {
        let partition = Blocks::partition(grid.clone());
        let mut by_label = HashMap::new();
        let mut by_block = HashMap::new();
        for block in partition.blocks() {
            let label = block
                .codels()
                .iter()
                .map(|point| ring_index(grid, *point))
                .min();
            if let Some(label) = label {
                by_label.insert(label, block.id());
                by_block.insert(block.id(), label);
            }
        }
        Ok(Self {
            by_label,
            by_block,
            ring: grid_ring(grid)?,
            start: Blocks::partition(start_image()?),
            exit: Blocks::partition(exit_image()?),
            steps,
        })
    }

    /// Runs one image over one stream.
    fn run(&self, image: &Blocks, stream: &str) -> Result<String, Error> {
        let mut io = BufferIo::new(stream);
        let options = Options {
            max_steps: Some(self.steps),
            ..Options::default()
        };
        let result = interp::run(image, &mut io, &options)?;
        if result.reason != ExitReason::Terminated {
            return Err(Error::StepLimit(self.steps));
        }
        Ok(io.into_output())
    }

    fn tokens(&self, output: &str) -> Result<Vec<i64>, Error> {
        output
            .split_whitespace()
            .map(|token| {
                token.parse().map_err(|_| Error::BadOutput {
                    output: output.to_owned(),
                })
            })
            .collect()
    }

    fn block(&self, label: i64, output: &str) -> Result<BlockId, Error> {
        let label = u32::try_from(label).map_err(|_| Error::BadOutput {
            output: output.to_owned(),
        })?;
        self.by_label
            .get(&label)
            .copied()
            .ok_or_else(|| Error::BadOutput {
                output: output.to_owned(),
            })
    }

    fn state(&self, output: &str) -> Result<Option<State>, Error> {
        let tokens = self.tokens(output)?;
        match tokens.as_slice() {
            [-1] => Ok(None),
            [label, dp, cc] => Ok(Some(State {
                block: self.block(*label, output)?,
                dp: direction(*dp, output)?,
                cc: chooser(*cc, output)?,
            })),
            _ => Err(Error::BadOutput {
                output: output.to_owned(),
            }),
        }
    }
}

impl Resolver for GridResolver {
    type Error = Error;

    fn start(&self) -> Result<Start, Error> {
        let output = self.run(&self.start, &self.ring)?;
        Ok(self.state(&output)?.map_or(Start::Terminate, Start::State))
    }

    fn exit(&self, state: State) -> Result<Exit, Error> {
        let Some(&label) = self.by_block.get(&state.block) else {
            return Err(Error::TooLarge);
        };
        let stream = with_exit_query(&self.ring, state.dp, state.cc, label);
        let output = self.run(&self.exit, &stream)?;
        let tokens = self.tokens(&output)?;
        match tokens.as_slice() {
            [-1] => Ok(Exit::Terminate),
            [label, dp, cc, code] => Ok(Exit::Move {
                target: self.block(*label, &output)?,
                dp: direction(*dp, &output)?,
                cc: chooser(*cc, &output)?,
                command: record_command(u32::try_from(*code).map_err(|_| Error::BadOutput {
                    output: output.clone(),
                })?),
            }),
            _ => Err(Error::BadOutput { output }),
        }
    }
}

fn direction(value: i64, output: &str) -> Result<Direction, Error> {
    usize::try_from(value)
        .ok()
        .and_then(|index| Direction::ALL.get(index).copied())
        .ok_or_else(|| Error::BadOutput {
            output: output.to_owned(),
        })
}

fn chooser(value: i64, output: &str) -> Result<Chooser, Error> {
    match value {
        0 => Ok(Chooser::Left),
        1 => Ok(Chooser::Right),
        _ => Err(Error::BadOutput {
            output: output.to_owned(),
        }),
    }
}
