//! The reachable `(block, DP, CC)` execution graph.

use std::collections::{HashMap, VecDeque};

use canvas::Blocks;

use crate::{Command, Exit, Start, State, exit, start};

/// How a transition chooses its successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// One successor, chosen by the geometry.
    Deterministic,
    /// Four successors, indexed by the popped value modulo four.
    Pointer(u32),
    /// Two successors, indexed by the popped value modulo two.
    Switch(bool),
}

/// One outgoing edge of a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: State,
    pub to: State,
    pub command: Option<Command>,
    pub branch: Branch,
}

/// The states reachable from the start, and the edges between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateGraph {
    start: Option<State>,
    states: Vec<State>,
    edges: Vec<Edge>,
}

impl StateGraph {
    /// Builds the graph reachable from the upper-left start state.
    pub fn build(blocks: &Blocks) -> Self {
        let Start::State(start) = start(blocks) else {
            return Self {
                start: None,
                states: Vec::new(),
                edges: Vec::new(),
            };
        };

        let mut seen: HashMap<State, usize> = HashMap::new();
        let mut states = Vec::new();
        let mut edges = Vec::new();
        let mut queue = VecDeque::new();
        seen.insert(start, 0);
        states.push(start);
        queue.push_back(start);

        while let Some(state) = queue.pop_front() {
            let Exit::Move {
                target,
                dp,
                cc,
                command,
            } = exit(blocks, state.block, state.dp, state.cc)
            else {
                continue;
            };
            let successors: Vec<(State, Branch)> = match command {
                Some(Command::Pointer) => (0..4)
                    .map(|turns| {
                        (
                            State {
                                block: target,
                                dp: dp.clockwise_by(turns),
                                cc,
                            },
                            Branch::Pointer(turns),
                        )
                    })
                    .collect(),
                Some(Command::Switch) => vec![
                    (
                        State {
                            block: target,
                            dp,
                            cc,
                        },
                        Branch::Switch(false),
                    ),
                    (
                        State {
                            block: target,
                            dp,
                            cc: cc.toggled(),
                        },
                        Branch::Switch(true),
                    ),
                ],
                _ => vec![(
                    State {
                        block: target,
                        dp,
                        cc,
                    },
                    Branch::Deterministic,
                )],
            };

            for (next, branch) in successors {
                edges.push(Edge {
                    from: state,
                    to: next,
                    command,
                    branch,
                });
                if let std::collections::hash_map::Entry::Vacant(entry) = seen.entry(next) {
                    entry.insert(states.len());
                    states.push(next);
                    queue.push_back(next);
                }
            }
        }

        Self {
            start: Some(start),
            states,
            edges,
        }
    }

    pub fn start(&self) -> Option<State> {
        self.start
    }

    pub fn states(&self) -> &[State] {
        &self.states
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Whether the state has no outgoing edge.
    pub fn is_terminating(&self, state: State) -> bool {
        self.edges.iter().all(|edge| edge.from != state)
    }

    /// The outgoing edges of a state.
    pub fn successors(&self, state: State) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |edge| edge.from == state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Chooser, Direction};
    use canvas::{CodelGrid, Color, Hue, Lightness, Point};

    fn blocks(rows: &[&str]) -> Blocks {
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, |row| row.len() as u32);
        let mut codels = Vec::with_capacity((width as usize) * (height as usize));
        for row in rows {
            assert_eq!(row.len() as u32, width, "rows must be rectangular");
            for cell in row.chars() {
                codels.push(match cell {
                    '.' => Color::White,
                    '#' => Color::Black,
                    'r' => Color::chromatic(Hue::Red, Lightness::Normal),
                    'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                    'd' => Color::chromatic(Hue::Cyan, Lightness::Dark),
                    'l' => Color::chromatic(Hue::Cyan, Lightness::Light),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        Blocks::partition(CodelGrid::from_colors(width, height, codels).unwrap())
    }

    fn state(blocks: &Blocks, x: u32, y: u32, dp: Direction, cc: Chooser) -> State {
        State {
            block: blocks.block_id_at(Point::new(x, y)).unwrap(),
            dp,
            cc,
        }
    }

    #[test]
    fn a_two_block_graph_is_reachable_and_deterministic() {
        let blocks = blocks(&["rrgg"]);
        let graph = StateGraph::build(&blocks);
        assert_eq!(graph.states().len(), 3);
        assert_eq!(graph.edges().len(), 3);
        let start = graph.start().unwrap();
        assert_eq!(start, state(&blocks, 0, 0, Direction::Right, Chooser::Left));
        let first = graph.successors(start).next().unwrap();
        assert_eq!(first.command, Some(Command::Divide));
        assert_eq!(first.branch, Branch::Deterministic);
        assert_eq!(
            first.to,
            state(&blocks, 3, 0, Direction::Right, Chooser::Left)
        );
    }

    #[test]
    fn a_pointer_edge_has_four_successors() {
        let blocks = blocks(&["rd"]);
        let graph = StateGraph::build(&blocks);
        let start = graph.start().unwrap();
        let edges: Vec<&Edge> = graph
            .successors(start)
            .filter(|edge| edge.command == Some(Command::Pointer))
            .collect();
        assert_eq!(edges.len(), 4);
        let turns: Vec<u32> = edges
            .iter()
            .map(|edge| match edge.branch {
                Branch::Pointer(turns) => turns,
                other => panic!("unexpected branch {other:?}"),
            })
            .collect();
        assert_eq!(turns, vec![0, 1, 2, 3]);
        let directions: Vec<Direction> = edges.iter().map(|edge| edge.to.dp).collect();
        assert_eq!(
            directions,
            vec![
                Direction::Right,
                Direction::Down,
                Direction::Left,
                Direction::Up
            ]
        );
    }

    #[test]
    fn a_switch_edge_has_two_successors() {
        let blocks = blocks(&["rl"]);
        let graph = StateGraph::build(&blocks);
        let start = graph.start().unwrap();
        let edges: Vec<&Edge> = graph
            .successors(start)
            .filter(|edge| edge.command == Some(Command::Switch))
            .collect();
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].branch, Branch::Switch(false));
        assert_eq!(edges[1].branch, Branch::Switch(true));
        assert_eq!(edges[0].to.cc, Chooser::Left);
        assert_eq!(edges[1].to.cc, Chooser::Right);
    }

    #[test]
    fn a_single_block_graph_is_a_sink() {
        let blocks = blocks(&["r"]);
        let graph = StateGraph::build(&blocks);
        assert_eq!(graph.states().len(), 1);
        assert!(graph.edges().is_empty());
        assert!(graph.is_terminating(graph.start().unwrap()));
    }

    #[test]
    fn a_black_start_has_no_graph() {
        let graph = StateGraph::build(&blocks(&["#"]));
        assert_eq!(graph.start(), None);
        assert!(graph.states().is_empty());
        assert!(graph.edges().is_empty());
    }
}
