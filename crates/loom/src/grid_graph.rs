//! Building the walker's records from the image's moves.
//!
//! The grid image resolves the movement rules from pixels, so this walk builds
//! the same state graph [`StateGraph::build`] does — one state per
//! `(block, dp, cc)`, its outgoing command and its successors — and hands it to
//! [`crate::graph_walker`] along with the partition its records are sized from.
//! Both come from images: the splitter names and sizes the blocks, and the start
//! and exit stages resolve the moves.

use interp::{ExitReason, Options};
use runtime::BufferIo;
use tessera::{Chooser, Direction, Exit, Resolver, Start, State, StateGraph};

use canvas::{BlockId, Blocks, CodelGrid};

use crate::error::Error;
use crate::grid::{Partition, exit_image, grid_ring, partition_of, start_image, with_exit_query};
use crate::walker::record_command;

/// Builds the state graph reachable from the upper-left codel, resolving every
/// move with the grid image, and the partition it is keyed by.
///
/// `steps` is the budget for one image run; a grid that needs more is reported
/// as [`Error::StepLimit`].
pub fn grid_graph(grid: &CodelGrid, steps: u64) -> Result<(StateGraph, Partition), Error> {
    let ring = grid_ring(grid)?;
    let partition = partition_of(grid.width(), grid.height(), &ring, steps)?;
    let resolver = GridResolver::new(&ring, &partition, steps)?;
    let graph = StateGraph::build_with(&resolver)?;
    Ok((graph, partition))
}

/// The image as a resolver of the movement rules.
struct GridResolver<'a> {
    ring: &'a str,
    partition: &'a Partition,
    start: Blocks,
    exit: Blocks,
    steps: u64,
}

impl<'a> GridResolver<'a> {
    fn new(ring: &'a str, partition: &'a Partition, steps: u64) -> Result<Self, Error> {
        Ok(Self {
            ring,
            partition,
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
        u32::try_from(label)
            .ok()
            .and_then(|label| self.partition.id_of(label))
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

impl Resolver for GridResolver<'_> {
    type Error = Error;

    fn start(&self) -> Result<Start, Error> {
        let output = self.run(&self.start, self.ring)?;
        Ok(self.state(&output)?.map_or(Start::Terminate, Start::State))
    }

    fn exit(&self, state: State) -> Result<Exit, Error> {
        // Every state came from a label the splitter printed, so the label is
        // there; a missing one would be the image disagreeing with itself.
        let label = self
            .partition
            .label_of(state.block)
            .ok_or(Error::TooLarge)?;
        let stream = with_exit_query(self.ring, state.dp, state.cc, label);
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
