//! Straight-line Piet programs: one row of blocks, one command between each.
//!
//! The program is a launcher at the top-left, then a row of blocks. Every
//! transition between blocks runs exactly one command, so the program's trace
//! is the command sequence the layout was built from. A three-codel bar at the
//! end is entered above its middle codel, so the movement rules cannot find an
//! exit and the program terminates.
//!
//! Printing a character costs three blocks: a block of `code` codels, a
//! transition into a one-codel block that runs `push` (pushing the block's
//! size, which is the character code), then a transition into a one-codel block
//! that runs `out char`.

use canvas::CodelGrid;
use tessera::Command;

use crate::error::Error;
use crate::paint::Canvas;
use crate::shade::Shade;

/// The launcher block: two codels in column zero and one in row one, so the
/// upper-left codel is part of the start block and the exit is in row one where
/// the terminator can reach above it.
const LAUNCHER: [(u32, u32); 3] = [(0, 0), (0, 1), (1, 1)];

/// A straight-line program that prints `text`.
pub fn linear_program(text: &str) -> Result<CodelGrid, Error> {
    let mut blocks = Vec::new();
    for character in text.chars() {
        let code = character as u32;
        if code == 0 {
            return Err(Error::EmptyBlock);
        }
        blocks.push((Command::Pop, code));
        blocks.push((Command::Push, 1));
        blocks.push((Command::OutChar, 1));
    }

    let row_start = 2;
    let row_length = blocks.iter().map(|(_, size)| *size).sum::<u32>();
    let width = row_start + row_length + 1;

    let start = Shade::new(0, 0);
    let mut canvas = Canvas::new(width, 3)?;
    for (x, y) in LAUNCHER {
        canvas.put(x, y, start.color())?;
    }

    let mut shade = start;
    let mut x = row_start;
    for (command, size) in blocks {
        shade = shade.advance(command);
        for _ in 0..size {
            canvas.put(x, 1, shade.color())?;
            x += 1;
        }
    }

    let terminator = shade.advance(Command::Push);
    canvas.put_block(x, 0, 1, 3, terminator.color())?;
    canvas.into_grid()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::Blocks;

    #[test]
    fn a_program_starts_above_its_first_block() {
        let grid = linear_program("A").expect("a program");
        assert_eq!(grid.height(), 3);
        assert_eq!(
            grid.get(canvas::Point::new(0, 0)),
            Some(Shade::new(0, 0).color())
        );
    }

    #[test]
    fn each_character_becomes_a_block_of_its_code() {
        let grid = linear_program("A").expect("a program");
        let blocks = Blocks::partition(grid);
        let mut sizes = blocks
            .blocks()
            .iter()
            .filter(|block| block.color().is_chromatic())
            .map(|block| block.size())
            .collect::<Vec<_>>();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![1, 1, 3, 3, 65]);
    }

    #[test]
    fn an_empty_program_is_just_a_launcher_and_a_terminator() {
        let grid = linear_program("").expect("a program");
        let blocks = Blocks::partition(grid);
        let chromatic = blocks
            .blocks()
            .iter()
            .filter(|block| block.color().is_chromatic())
            .count();
        assert_eq!(chromatic, 2);
    }
}
