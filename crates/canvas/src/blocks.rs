//! The 4-connected colour-block partition.

use std::collections::VecDeque;

use crate::grid::Point;
use crate::{CodelGrid, Color};

/// The index of a block in a partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// A maximal 4-connected region of one colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    id: BlockId,
    color: Color,
    codels: Vec<Point>,
}

impl Block {
    pub const fn id(&self) -> BlockId {
        self.id
    }

    pub const fn color(&self) -> Color {
        self.color
    }

    pub fn codels(&self) -> &[Point] {
        &self.codels
    }

    pub fn size(&self) -> usize {
        self.codels.len()
    }
}

/// The 4-connected partition of a codel grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocks {
    grid: CodelGrid,
    blocks: Vec<Block>,
    by_codel: Vec<Option<BlockId>>,
}

impl Blocks {
    /// Partitions the grid into its maximal 4-connected colour blocks.
    pub fn partition(grid: CodelGrid) -> Self {
        let width = grid.width();
        let height = grid.height();
        let count = (width as usize) * (height as usize);
        let mut by_codel = vec![None; count];
        let mut blocks = Vec::new();

        for y in 0..height {
            for x in 0..width {
                let start = Point::new(x, y);
                let start_index = start.index(width);
                if by_codel.get(start_index).copied().flatten().is_some() {
                    continue;
                }
                let Some(color) = grid.get(start) else {
                    continue;
                };

                let id = BlockId(blocks.len() as u32);
                let mut codels = Vec::new();
                let mut queue = VecDeque::new();
                queue.push_back(start);
                if let Some(slot) = by_codel.get_mut(start_index) {
                    *slot = Some(id);
                }

                while let Some(point) = queue.pop_front() {
                    codels.push(point);
                    for neighbour in neighbours(point) {
                        let Some(neighbour_index) = index_of(neighbour, width, height) else {
                            continue;
                        };
                        if by_codel.get(neighbour_index).copied().flatten().is_some() {
                            continue;
                        }
                        if grid.get(neighbour) != Some(color) {
                            continue;
                        }
                        if let Some(slot) = by_codel.get_mut(neighbour_index) {
                            *slot = Some(id);
                        }
                        queue.push_back(neighbour);
                    }
                }

                codels.sort_by_key(|point| (point.y, point.x));
                blocks.push(Block { id, color, codels });
            }
        }

        Self {
            grid,
            blocks,
            by_codel,
        }
    }

    pub fn grid(&self) -> &CodelGrid {
        &self.grid
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks.get(id.0 as usize)
    }

    pub fn block_id_at(&self, point: Point) -> Option<BlockId> {
        index_of(point, self.grid.width(), self.grid.height())
            .and_then(|index| self.by_codel.get(index).copied().flatten())
    }

    pub fn block_at(&self, point: Point) -> Option<&Block> {
        self.block_id_at(point).and_then(|id| self.block(id))
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

fn index_of(point: Point, width: u32, height: u32) -> Option<usize> {
    if point.x >= width || point.y >= height {
        return None;
    }
    Some(point.index(width))
}

fn neighbours(point: Point) -> impl Iterator<Item = Point> {
    let mut points = [None; 4];
    points[0] = point.x.checked_sub(1).map(|x| Point::new(x, point.y));
    points[1] = point.x.checked_add(1).map(|x| Point::new(x, point.y));
    points[2] = point.y.checked_sub(1).map(|y| Point::new(point.x, y));
    points[3] = point.y.checked_add(1).map(|y| Point::new(point.x, y));
    points.into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Hue, Lightness};
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn grid(rows: &[&str]) -> CodelGrid {
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
                    'b' => Color::chromatic(Hue::Blue, Lightness::Normal),
                    'y' => Color::chromatic(Hue::Yellow, Lightness::Dark),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        CodelGrid::from_colors(width, height, codels).unwrap()
    }

    #[test]
    fn a_single_colour_is_one_block() {
        let blocks = Blocks::partition(grid(&["rr", "rr"]));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks.blocks()[0].size(), 4);
        assert_eq!(
            blocks.blocks()[0].codels(),
            [
                Point::new(0, 0),
                Point::new(1, 0),
                Point::new(0, 1),
                Point::new(1, 1),
            ]
            .as_slice()
        );
    }

    #[test]
    fn diagonal_contact_does_not_join_blocks() {
        let blocks = Blocks::partition(grid(&["rg", "gr"]));
        assert_eq!(blocks.len(), 4);
    }

    #[test]
    fn a_hole_is_a_separate_block() {
        let blocks = Blocks::partition(grid(&["rrr", "r#r", "rrr"]));
        assert_eq!(blocks.len(), 2);
        let red = blocks
            .blocks()
            .iter()
            .find(|block| block.color() == Color::chromatic(Hue::Red, Lightness::Normal))
            .unwrap();
        assert_eq!(red.size(), 8);
        let black = blocks
            .blocks()
            .iter()
            .find(|block| block.color() == Color::Black)
            .unwrap();
        assert_eq!(black.size(), 1);
    }

    #[test]
    fn every_codel_has_exactly_one_block() {
        let blocks = Blocks::partition(grid(&["r.g", "#.b", "rrr"]));
        let width = blocks.grid().width();
        let height = blocks.grid().height();
        let mut seen = vec![0u8; (width as usize) * (height as usize)];
        for block in blocks.blocks() {
            for point in block.codels() {
                seen[point.index(width)] += 1;
            }
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    #[test]
    fn block_at_names_the_owning_block() {
        let blocks = Blocks::partition(grid(&["rg", "bb"]));
        let point = Point::new(1, 0);
        let block = blocks.block_at(point).unwrap();
        assert_eq!(
            block.color(),
            Color::chromatic(Hue::Green, Lightness::Normal)
        );
        assert_eq!(blocks.block_at(Point::new(2, 0)), None);
    }

    fn color_from_code(code: u8) -> Color {
        match code % 5 {
            0 => Color::White,
            1 => Color::Black,
            2 => Color::chromatic(Hue::Red, Lightness::Normal),
            3 => Color::chromatic(Hue::Green, Lightness::Dark),
            _ => Color::chromatic(Hue::Blue, Lightness::Light),
        }
    }

    fn is_connected(block: &Block) -> bool {
        let members: HashSet<Point> = block.codels().iter().copied().collect();
        let Some(&start) = block.codels().first() else {
            return true;
        };
        let mut stack = vec![start];
        let mut seen = HashSet::new();
        while let Some(point) = stack.pop() {
            if !seen.insert(point) {
                continue;
            }
            for (dx, dy) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
                let x = point.x as i32 + dx;
                let y = point.y as i32 + dy;
                if x < 0 || y < 0 {
                    continue;
                }
                let neighbour = Point::new(x as u32, y as u32);
                if members.contains(&neighbour) {
                    stack.push(neighbour);
                }
            }
        }
        seen.len() == members.len()
    }

    proptest! {
        #[test]
        fn partition_is_total_disjoint_and_connected(
            width in 1u32..7,
            height in 1u32..7,
            codes in proptest::collection::vec(any::<u8>(), 0..49),
        ) {
            let count = (width as usize) * (height as usize);
            let codels: Vec<Color> = (0..count)
                .map(|index| color_from_code(codes.get(index).copied().unwrap_or(0)))
                .collect();
            let grid = CodelGrid::from_colors(width, height, codels).unwrap();
            let blocks = Blocks::partition(grid);
            let mut owners = vec![None; count];

            for block in blocks.blocks() {
                for point in block.codels() {
                    let index = point.index(width);
                    prop_assert!(owners[index].is_none(), "codel {:?} is in two blocks", point);
                    owners[index] = Some(block.id());
                    prop_assert_eq!(blocks.grid().get(*point), Some(block.color()));
                }
                prop_assert!(is_connected(block), "block {:?} is not connected", block.id());
            }

            prop_assert!(owners.iter().all(Option::is_some), "a codel has no block");
        }
    }
}
