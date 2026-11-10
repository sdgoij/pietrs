//! The block partition and state graph for a document revision.

use std::sync::Arc;

use canvas::{Blocks, Color};
use tessera::{Start, StateGraph, start};

use crate::Document;

/// The numbers `pietrs check` prints, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub codel_size: u32,
    pub blocks: usize,
    pub states: usize,
    pub edges: usize,
    pub can_terminate: bool,
    pub start: Start,
    pub unknown_colours: Vec<([u8; 3], u32)>,
    /// Codel counts in [`Color::ALL`] order.
    pub palette: [u32; 20],
}

/// The partition and graph for one revision.
#[derive(Debug, Clone)]
pub struct Analysis {
    revision: u64,
    blocks: Arc<Blocks>,
    graph: StateGraph,
    report: Report,
}

impl Analysis {
    pub fn build(document: &Document) -> Result<Self, canvas::Error> {
        let grid = document.to_grid()?;
        let blocks = Arc::new(Blocks::partition(grid));
        let graph = StateGraph::build(&blocks);
        let report = report_of(&blocks, &graph);
        Ok(Self {
            revision: document.revision(),
            blocks,
            graph,
            report,
        })
    }

    /// Rebuilds when the document revision moved, and does nothing otherwise.
    pub fn refresh(&mut self, document: &Document) -> Result<bool, canvas::Error> {
        if self.revision == document.revision() {
            return Ok(false);
        }
        let grid = document.to_grid()?;
        let blocks = Arc::new(Blocks::partition(grid));
        let graph = StateGraph::build(&blocks);
        let report = report_of(&blocks, &graph);
        self.blocks = blocks;
        self.graph = graph;
        self.report = report;
        self.revision = document.revision();
        Ok(true)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn blocks(&self) -> &Arc<Blocks> {
        &self.blocks
    }

    pub fn graph(&self) -> &StateGraph {
        &self.graph
    }

    pub fn report(&self) -> &Report {
        &self.report
    }
}

fn report_of(blocks: &Blocks, graph: &StateGraph) -> Report {
    let grid = blocks.grid();
    let mut palette = [0u32; 20];
    for color in grid.codels() {
        if let Some(index) = Color::ALL.iter().position(|candidate| candidate == color)
            && let Some(slot) = palette.get_mut(index)
        {
            *slot += 1;
        }
    }
    Report {
        width: grid.width(),
        height: grid.height(),
        codel_size: grid.codel_size(),
        blocks: blocks.len(),
        states: graph.states().len(),
        edges: graph.edges().len(),
        can_terminate: graph.can_terminate(),
        start: start(blocks),
        unknown_colours: grid.unknown_colours().to_vec(),
        palette,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Hue, Lightness, Point};

    fn document(rows: &[&str]) -> Document {
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, |row| row.len() as u32);
        let mut codels = Vec::with_capacity((width as usize) * (height as usize));
        for row in rows {
            for cell in row.chars() {
                codels.push(match cell {
                    '.' => Color::White,
                    '#' => Color::Black,
                    'r' => Color::chromatic(Hue::Red, Lightness::Normal),
                    'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        Document::from_grid(&CodelGrid::from_colors(width, height, codels).unwrap())
    }

    #[test]
    fn the_report_counts_the_partition_and_the_graph() {
        let document = document(&["rrgg"]);
        let analysis = Analysis::build(&document).unwrap();
        let report = analysis.report();

        assert_eq!(report.width, 4);
        assert_eq!(report.height, 1);
        assert_eq!(report.codel_size, 1);
        assert_eq!(report.blocks, 2);
        assert_eq!(report.states, 3);
        assert_eq!(report.edges, 3);
        assert!(!report.can_terminate);
        assert_eq!(report.palette.iter().sum::<u32>(), 4);
        assert!(matches!(report.start, Start::State(_)));
    }

    #[test]
    fn a_black_start_is_reported_as_terminating_immediately() {
        let analysis = Analysis::build(&document(&["#"])).unwrap();
        assert_eq!(analysis.report().start, Start::Terminate);
        assert!(analysis.report().can_terminate);
        assert_eq!(analysis.report().blocks, 1);
    }

    #[test]
    fn refresh_does_nothing_until_the_revision_moves() {
        let mut document = document(&["rrgg"]);
        let mut analysis = Analysis::build(&document).unwrap();
        assert!(!analysis.refresh(&document).unwrap());

        document.set(Point::new(2, 0), Color::Black);
        assert!(analysis.refresh(&document).unwrap());
        assert!(!analysis.refresh(&document).unwrap());
        assert_eq!(analysis.report().blocks, 3);
    }

    #[test]
    fn refresh_replaces_the_partition_only_when_the_revision_moves() {
        let mut document = document(&["rr", "gg"]);
        let mut analysis = Analysis::build(&document).unwrap();
        let before = Arc::clone(analysis.blocks());

        assert!(!analysis.refresh(&document).unwrap());
        assert!(Arc::ptr_eq(&before, analysis.blocks()));

        document.set(Point::new(0, 0), Color::Black);
        assert!(analysis.refresh(&document).unwrap());
        assert!(!Arc::ptr_eq(&before, analysis.blocks()));
    }
}
