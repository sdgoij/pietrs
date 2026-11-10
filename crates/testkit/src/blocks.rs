//! Hand-built codel grids.

use canvas::{BlockId, Blocks, CodelGrid, Color, Point};
use tessera::{Command, command};

/// Partitions a row-major list of codels.
pub fn blocks(codels: Vec<Color>, width: u32) -> Blocks {
    let height = (codels.len() as u32) / width;
    Blocks::partition(CodelGrid::from_colors(width, height, codels).unwrap())
}

/// The block at a codel coordinate.
pub fn id_at(blocks: &Blocks, x: u32, y: u32) -> BlockId {
    blocks.block_id_at(Point::new(x, y)).unwrap()
}

/// The forward deltas the command table assigns to each command.
fn deltas(wanted: Command) -> (u8, u8) {
    match wanted {
        Command::Push => (0, 1),
        Command::Pop => (0, 2),
        Command::Add => (1, 0),
        Command::Subtract => (1, 1),
        Command::Multiply => (1, 2),
        Command::Divide => (2, 0),
        Command::Modulo => (2, 1),
        Command::Not => (2, 2),
        Command::Greater => (3, 0),
        Command::Pointer => (3, 1),
        Command::Switch => (3, 2),
        Command::Duplicate => (4, 0),
        Command::Roll => (4, 1),
        Command::InNumber => (4, 2),
        Command::InChar => (5, 0),
        Command::OutNumber => (5, 1),
        Command::OutChar => (5, 2),
    }
}

/// A single-row program whose transitions run `commands` in order.
///
/// Every codel is its own block, so the row walks left to right and each
/// transition is the command the caller asked for.
pub fn row(commands: &[Command]) -> Blocks {
    let mut colours = vec![Color::chromatic(
        canvas::Hue::Red,
        canvas::Lightness::Normal,
    )];
    for &wanted in commands {
        let from = *colours.last().unwrap();
        let deltas = deltas(wanted);
        let to = Color::ALL
            .into_iter()
            .find(|colour| from.deltas(*colour) == Some(deltas))
            .unwrap();
        colours.push(to);
    }
    for (index, &wanted) in commands.iter().enumerate() {
        assert_eq!(
            command(colours[index], colours[index + 1]),
            Some(wanted),
            "the fixture must run {wanted:?} at step {index}"
        );
    }
    let width = colours.len() as u32;
    blocks(colours, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{Hue, Lightness};
    use tessera::StateGraph;

    #[test]
    fn a_row_partitions_each_codel() {
        let blocks = blocks(
            vec![
                Color::chromatic(Hue::Red, Lightness::Normal),
                Color::chromatic(Hue::Green, Lightness::Normal),
            ],
            2,
        );
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn a_row_runs_its_commands_in_order() {
        let blocks = row(&[Command::Push, Command::OutChar]);
        let graph = StateGraph::build(&blocks);
        let start = graph.start().unwrap();
        let first = graph.successors(start).next().unwrap();
        assert_eq!(first.command, Some(Command::Push));
    }
}
