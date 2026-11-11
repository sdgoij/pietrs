//! A codel canvas: every cell starts black and a block paints its codels.

use canvas::{CodelGrid, Color};

use crate::error::Error;

/// A grid of codels under construction.
pub(crate) struct Canvas {
    width: u32,
    height: u32,
    cells: Vec<Option<Color>>,
}

impl Canvas {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, Error> {
        let length = usize::try_from(width)
            .ok()
            .and_then(|width| usize::try_from(height).ok()?.checked_mul(width))
            .ok_or(Error::TooLarge)?;
        Ok(Self {
            width,
            height,
            cells: vec![None; length],
        })
    }

    /// Paints one codel. An unpainted codel is black; painting over a different
    /// colour means two blocks claimed the same codel, which is a layout bug.
    pub(crate) fn put(&mut self, x: u32, y: u32, color: Color) -> Result<(), Error> {
        let index = self.index(x, y).ok_or(Error::OutOfBounds {
            x,
            y,
            width: self.width,
            height: self.height,
        })?;
        if let Some(existing) = self.cells.get(index).copied().flatten()
            && existing != color
        {
            return Err(Error::Overlap { x, y });
        }
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = Some(color);
        }
        Ok(())
    }

    /// Paints a block of `height` rows and `width` columns from its top-left
    /// corner.
    pub(crate) fn put_block(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        color: Color,
    ) -> Result<(), Error> {
        for row in 0..height {
            for column in 0..width {
                self.put(x + column, y + row, color)?;
            }
        }
        Ok(())
    }

    pub(crate) fn into_grid(self) -> Result<CodelGrid, Error> {
        let codels = self
            .cells
            .iter()
            .map(|cell| cell.unwrap_or(Color::Black))
            .collect();
        Ok(CodelGrid::from_colors(self.width, self.height, codels)?)
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let x = usize::try_from(x).ok()?;
        let y = usize::try_from(y).ok()?;
        let width = usize::try_from(self.width).ok()?;
        y.checked_mul(width)?.checked_add(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::Point;

    #[test]
    fn unpainted_cells_are_black() {
        let mut canvas = Canvas::new(2, 1).expect("small canvas");
        canvas.put(0, 0, Color::White).expect("inside the canvas");
        let grid = canvas.into_grid().expect("a valid grid");
        assert_eq!(grid.get(Point::new(0, 0)), Some(Color::White));
        assert_eq!(grid.get(Point::new(1, 0)), Some(Color::Black));
    }

    #[test]
    fn painting_outside_the_canvas_is_an_error() {
        let mut canvas = Canvas::new(1, 1).expect("small canvas");
        assert!(matches!(
            canvas.put(1, 0, Color::White),
            Err(Error::OutOfBounds { .. })
        ));
    }

    #[test]
    fn painting_a_cell_twice_in_one_colour_is_allowed() {
        let mut canvas = Canvas::new(1, 1).expect("small canvas");
        canvas.put(0, 0, Color::Black).expect("first paint");
        canvas.put(0, 0, Color::Black).expect("same colour");
        assert!(matches!(
            canvas.put(0, 0, Color::White),
            Err(Error::Overlap { .. })
        ));
    }

    #[test]
    fn a_block_paints_every_cell() {
        let mut canvas = Canvas::new(3, 2).expect("canvas");
        canvas
            .put_block(0, 0, 3, 2, Color::Black)
            .expect("a full grid");
        let grid = canvas.into_grid().expect("a valid grid");
        for codel in grid.codels() {
            assert_eq!(*codel, Color::Black);
        }
    }
}
