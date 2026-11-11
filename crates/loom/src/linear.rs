//! Straight-line Piet programs: one row of blocks, one command between each.
//!
//! The program is a launcher at the top-left, then a row of blocks. The command
//! that leaves a block is its exit, and a `push` pushes that block's size, so
//! each instruction is one block: the block's size is the operand and the
//! transition out of it runs the command. A three-codel bar at the end is
//! entered above its middle codel, so the movement rules cannot find an exit
//! and the program terminates.
//!
//! The launcher is always three codels and its exit always runs `pop`, which
//! the start stack ignores. That puts the upper-left codel in the start block
//! and keeps the program row low enough for the terminator bar to reach above
//! it, without spending any instruction on either.

use canvas::CodelGrid;
use tessera::Command;

use crate::error::Error;
use crate::paint::Canvas;
use crate::program::Instruction;
use crate::shade::Shade;

/// The launcher block: two codels in column zero and one in row one.
const LAUNCHER: [(u32, u32); 3] = [(0, 0), (0, 1), (1, 1)];
/// The instruction the launcher's exit runs: a `pop` on the start stack is
/// ignored, so it costs nothing.
const LAUNCHER_COMMAND: Command = Command::Pop;

/// A straight-line program image for an instruction sequence.
pub fn assemble(instructions: &[Instruction]) -> Result<CodelGrid, Error> {
    let mut blocks = Vec::with_capacity(instructions.len());
    for instruction in instructions {
        let size = if instruction.command == Command::Push {
            u32::try_from(instruction.block_size).map_err(|_| Error::BlockTooLarge {
                operand: instruction.block_size,
            })?
        } else {
            1
        };
        if size == 0 {
            return Err(Error::EmptyBlock);
        }
        blocks.push((instruction.command, size));
    }

    let row_length = blocks.iter().map(|(_, size)| *size).sum::<u32>();
    let width = 2 + row_length + 1;

    let start = Shade::new(0, 0);
    let mut canvas = Canvas::new(width, 3)?;
    for (x, y) in LAUNCHER {
        canvas.put(x, y, start.color())?;
    }

    // The launcher's exit is the first instruction block's colour.
    let mut shade = start.advance(LAUNCHER_COMMAND);
    let mut x = 2;
    for (command, size) in blocks {
        for _ in 0..size {
            canvas.put(x, 1, shade.color())?;
            x += 1;
        }
        shade = shade.advance(command);
    }

    canvas.put_block(x, 0, 1, 3, shade.color())?;
    canvas.into_grid()
}

/// A straight-line program that prints `text`.
pub fn linear_program(text: &str) -> Result<CodelGrid, Error> {
    let mut instructions = Vec::new();
    for character in text.chars() {
        let code = character as u32;
        if code == 0 {
            return Err(Error::EmptyBlock);
        }
        instructions.push(Instruction {
            command: Command::Push,
            block_size: u64::from(code),
        });
        instructions.push(Instruction {
            command: Command::OutChar,
            block_size: 1,
        });
    }
    assemble(&instructions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::Blocks;

    fn chromatic_sizes(grid: CodelGrid) -> Vec<usize> {
        let blocks = Blocks::partition(grid);
        let mut sizes = blocks
            .blocks()
            .iter()
            .filter(|block| block.color().is_chromatic())
            .map(|block| block.size())
            .collect::<Vec<_>>();
        sizes.sort_unstable();
        sizes
    }

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
        assert_eq!(
            chromatic_sizes(linear_program("A").expect("a program")),
            [1, 3, 3, 65]
        );
        assert_eq!(
            chromatic_sizes(linear_program("AB").expect("a program")),
            [1, 1, 3, 3, 65, 66]
        );
    }

    #[test]
    fn an_empty_program_is_just_a_launcher_and_a_terminator() {
        assert_eq!(
            chromatic_sizes(linear_program("").expect("a program")).len(),
            2
        );
    }

    #[test]
    fn an_instruction_sequence_becomes_a_program() {
        let instructions = [
            Instruction {
                command: Command::Push,
                block_size: 4,
            },
            Instruction {
                command: Command::Duplicate,
                block_size: 1,
            },
            Instruction {
                command: Command::OutNumber,
                block_size: 1,
            },
        ];
        // Launcher, the four-codel push block, duplicate, out number, bar.
        assert_eq!(
            chromatic_sizes(assemble(&instructions).expect("a program")),
            [1, 1, 3, 3, 4]
        );
    }

    #[test]
    fn a_push_of_zero_is_refused() {
        let instructions = [Instruction {
            command: Command::Push,
            block_size: 0,
        }];
        assert!(matches!(assemble(&instructions), Err(Error::EmptyBlock)));
    }
}
