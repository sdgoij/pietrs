//! Composing the frontend into one image: the PPM parser feeding a reverse ring
//! builder.
//!
//! The two stages normally talk through the host: [`crate::ppm_codes`] prints the
//! grid codes and [`crate::ring_to_stream`] reads them back. This image removes
//! that hop. The parser leaves the codes on the stack and the ring builder reads
//! them with plain pops.
//!
//! The layout that makes it work is the parser's accumulated output:
//!
//! ```text
//!   [width, height, last pixel .. first pixel]
//! ```
//!
//! The parser keeps `[counter, width, height]` above the accumulator, so each
//! finished code is one fixed `roll(4, 1)` below the header. The ring builder
//! then walks the ring **backwards**, from the highest cell down to zero, popping
//! one code per interior cell. Because the ring's interior cells in descending
//! order are the pixels in descending row-major order, the pops are the reversed
//! pixel order the parser left, and the records come out highest-index-first —
//! the order the splitter reads. No FIFO tape and no dynamic-depth roll: the only
//! rolls are one constant header roll and the code pop.
//!
//! The ring builder streams each record to its output as it goes, so it needs no
//! output accumulator of its own.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options};
use runtime::BufferIo;
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};
use crate::ppm;

/// The ring builder's blocks, numbered from one.
const R_ENTRY: u32 = 1;
const R_BODY: u32 = 2;
const R_GUARD: u32 = 3;
const R_CODEL: u32 = 4;
const R_NEXT: u32 = 5;
const R_DONE: u32 = 6;
const R_DECREMENT: u32 = 7;

/// The code a guard cell carries: above every real colour.
const GUARD_CODE: i64 = 22;

/// The composed frontend image: PPM in, the ring stream out.
pub fn ppm_ring_grid() -> Result<CodelGrid, Error> {
    ppm_ring_program()?.image()
}

fn ppm_ring_program() -> Result<Program, Error> {
    Program::new(ppm_ring_blocks(), 1)
}

/// The ring stream a P3 PPM decodes to, built by the one image.
pub fn ppm_ring(source: &str) -> Result<String, Error> {
    /// A few thousand cells: the ring builder is one short lap per cell.
    const STEPS: u64 = 200_000_000;

    let image = Blocks::partition(ppm_ring_grid()?);
    let mut io = BufferIo::new(source);
    let options = Options {
        max_steps: Some(STEPS),
        ..Options::default()
    };
    let result = interp::run(&image, &mut io, &options)?;
    if result.reason != ExitReason::Terminated {
        return Err(Error::StepLimit(STEPS));
    }
    Ok(io.into_output())
}

/// The parser's blocks, then the ring builder's, with the parser's normal
/// completion handing over instead of halting.
fn ppm_ring_blocks() -> Vec<Block> {
    let mut all = ppm::blocks_with(ppm::Output::Stack);
    let offset = all.len() as u32;
    let entry = offset + R_ENTRY;
    all[(ppm::DONE - 1) as usize].end = End::Jump(entry);
    all.extend(
        ring_blocks()
            .into_iter()
            .map(|block| Block::new(block.nodes, offset_end(block.end, offset))),
    );
    all
}

/// Add `offset` to every block id an end names.
fn offset_end(end: End, offset: u32) -> End {
    match end {
        End::Jump(id) => End::Jump(id + offset),
        End::If { then, otherwise } => End::If {
            then: then + offset,
            otherwise: otherwise + offset,
        },
        End::Computed => End::Computed,
        End::Halt => End::Halt,
    }
}

/// The reverse ring builder.
///
/// On entry the stack is `[width, height, last pixel .. first pixel]`. The live
/// registers are `[i, w2, h2]` (the current ring index and the padded
/// dimensions) above the pixel stack, so the next pixel is always at depth four.
fn ring_blocks() -> Vec<Block> {
    let mut blocks = Vec::with_capacity(R_DECREMENT as usize);
    blocks.push(Block::new(entry(), End::Jump(R_BODY)));
    blocks.push(Block::new(
        body(),
        End::If {
            then: R_GUARD,
            otherwise: R_CODEL,
        },
    ));
    // A guard cell: its colour is `22 + i`.
    blocks.push(Block::new(
        {
            let mut nodes = drop_coordinates();
            nodes.extend([
                Node::DupLocal(1),
                Node::Const(GUARD_CODE),
                Node::Command(Command::Add),
            ]);
            nodes.extend(print_record());
            nodes
        },
        End::Jump(R_NEXT),
    ));
    // An interior cell: the top pixel is the cell's colour.
    blocks.push(Block::new(
        {
            let mut nodes = drop_coordinates();
            // [i, w2, h2, code ..] -> [code, i, w2, h2, ..]
            nodes.push(Node::Roll(4, 3));
            nodes.extend(print_record());
            nodes
        },
        End::Jump(R_NEXT),
    ));
    blocks.push(Block::new(
        vec![Node::DupLocal(1), Node::Command(Command::Not)],
        End::If {
            then: R_DONE,
            otherwise: R_DECREMENT,
        },
    ));
    blocks.push(Block::new(Vec::new(), End::Halt));
    blocks.push(Block::new(
        vec![Node::Const(1), Node::Command(Command::Subtract)],
        End::Jump(R_BODY),
    ));
    blocks
}

/// `[width, height, pixels ..]` to `[i, w2, h2, pixels ..]`, printing the cell
/// count first.
fn entry() -> Vec<Node> {
    vec![
        Node::Const(2),
        Node::Command(Command::Add), // w2
        Node::Roll(2, 1),
        Node::Const(2),
        Node::Command(Command::Add), // h2
        Node::Roll(2, 1),            // [w2, h2]
        Node::DupLocal(1),
        Node::DupLocal(3),
        Node::Command(Command::Multiply), // w2 * h2
        Node::Const(1),
        Node::Command(Command::Subtract), // i = cells - 1
        // the cell count
        Node::DupLocal(1),
        Node::Const(1),
        Node::Command(Command::Add),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
    ]
}

/// The coordinates and the guard test, leaving `[col, row, i, w2, h2, ..]`.
fn body() -> Vec<Node> {
    vec![
        // row = i / w2
        Node::DupLocal(1),
        Node::DupLocal(3),
        Node::Command(Command::Divide),
        // column = i % w2
        Node::DupLocal(2),
        Node::DupLocal(4),
        Node::Command(Command::Modulo),
        // g1 = row == 0
        Node::DupLocal(2),
        Node::Command(Command::Not),
        // g2 = row == h2 - 1
        Node::DupLocal(3),
        Node::DupLocal(7),
        Node::Const(1),
        Node::Command(Command::Subtract),
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
        // g3 = column == 0
        Node::DupLocal(3),
        Node::Command(Command::Not),
        // g4 = column == w2 - 1
        Node::DupLocal(4),
        Node::DupLocal(8),
        Node::Const(1),
        Node::Command(Command::Subtract),
        Node::Command(Command::Subtract),
        Node::Command(Command::Not),
        // any guard is a guard
        Node::Command(Command::Add),
        Node::Command(Command::Add),
        Node::Command(Command::Add),
        Node::Const(0),
        Node::Command(Command::Greater),
    ]
}

/// Drop the coordinate temporaries a branch inherits: `[col, row, i, ..]` to
/// `[i, ..]`.
fn drop_coordinates() -> Vec<Node> {
    vec![
        Node::Roll(2, 1),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]
}

/// Print `ring stride colour index` from `[colour, i, w2, h2, ..]`, then drop
/// the temporaries.
fn print_record() -> Vec<Node> {
    vec![
        // stride = w2 * 4
        Node::DupLocal(3),
        Node::Const(4),
        Node::Command(Command::Multiply),
        // ring = stride * h2
        Node::DupLocal(1),
        Node::DupLocal(6),
        Node::Command(Command::Multiply),
        // ring
        Node::DupLocal(1),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        // stride
        Node::DupLocal(2),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        // colour
        Node::DupLocal(3),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        // index
        Node::DupLocal(4),
        Node::Command(Command::OutNumber),
        Node::Const(32),
        Node::Command(Command::OutChar),
        // ring, stride, colour
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::Block;

    /// The run's output with whitespace normalised, so separators do not matter.
    fn tokens(text: &str) -> Vec<&str> {
        text.split_whitespace().collect()
    }

    #[test]
    fn the_composed_image_builds_the_ring() {
        let cases = [
            vec!["r"],
            vec!["rg"],
            vec!["rg", "gb"],
            vec!["r.b", "#g#", "bk."],
        ];
        for rows in cases {
            let grid = grid(&rows);
            let want = crate::ring_to_stream(&crate::grid_codes(&grid)).unwrap();
            let got = ppm_ring(&ppm_of(&grid)).unwrap();
            assert_eq!(tokens(&got), tokens(&want), "{rows:?}");
        }
    }

    #[test]
    fn the_canonical_hello_world_ring_is_the_host_s() {
        let png = include_bytes!("../fixtures/hello-world/program.png");
        let grid = CodelGrid::from_image(png).unwrap();
        let want = crate::ring_to_stream(&crate::grid_codes(&grid)).unwrap();
        let got = ppm_ring(&ppm_of(&grid)).unwrap();
        assert_eq!(tokens(&got), tokens(&want));
    }

    fn grid(rows: &[&str]) -> CodelGrid {
        use canvas::{Color, Hue, Lightness};
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
                    'b' => Color::chromatic(Hue::Blue, Lightness::Normal),
                    'k' => Color::chromatic(Hue::Red, Lightness::Dark),
                    other => panic!("no colour for {other:?}"),
                });
            }
        }
        CodelGrid::from_colors(width, height, codels).unwrap()
    }

    fn ppm_of(grid: &CodelGrid) -> String {
        let mut text = format!("P3\n{} {}\n255\n", grid.width(), grid.height());
        for color in grid.codels() {
            let [red, green, blue] = color.rgb();
            text.push_str(&format!("{red} {green} {blue}\n"));
        }
        text
    }

    #[test]
    fn the_parser_leaves_width_on_top() {
        // The parser's normal completion drops the counter, leaving width, then
        // height, then the codes.
        let source = ppm_of(&grid(&["rg"]));
        let mut blocks = ppm::blocks_with(ppm::Output::Stack);
        // The parser halts at DONE; make it visible by reading the stack there.
        blocks[(ppm::DONE - 1) as usize] = Block::new(
            vec![
                Node::Command(Command::Pop),
                Node::DupLocal(1),
                Node::Command(Command::OutNumber), // width
                Node::Const(32),
                Node::Command(Command::OutChar),
                Node::DupLocal(2),
                Node::Command(Command::OutNumber), // height
                Node::Const(32),
                Node::Command(Command::OutChar),
                Node::DupLocal(3),
                Node::Command(Command::OutNumber), // last code
            ],
            End::Halt,
        );
        let program = Program::new(blocks, 1).unwrap();
        let image = Blocks::partition(program.image().unwrap());
        let mut io = BufferIo::new(&source);
        let result = interp::run(
            &image,
            &mut io,
            &Options {
                max_steps: Some(1_000_000),
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(result.reason, ExitReason::Terminated);
        // 1x2 grid of red and green: width 2, height 1, the last pixel's green.
        assert_eq!(tokens(&io.into_output()), ["2", "1", "7"]);
    }
}
