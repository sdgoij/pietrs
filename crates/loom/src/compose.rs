//! Composing the frontend into one image: the PPM parser, a reverse ring
//! builder, and (optionally) the splitter.
//!
//! The stages normally talk through the host: [`crate::ppm_codes`] prints the
//! grid codes and [`crate::ring_to_stream`] reads them back, then the splitter
//! reads the ring stream. This image removes those hops. The parser leaves the
//! codes on the stack, the ring builder reads them with plain pops, and the
//! splitter reads the records the ring builder leaves.
//!
//! # PPM to ring
//!
//! The parser's accumulated output is:
//!
//! ```text
//!   [width, height, last pixel .. first pixel]
//! ```
//!
//! It keeps `[counter, width, height]` above the accumulator, so each finished
//! code is one fixed `roll(4, 1)` below the header. The ring builder walks the
//! ring **backwards**, from the highest cell down to zero, popping one code per
//! interior cell. The interior cells in descending order are the pixels in
//! descending row-major order, so the pops are the reversed pixel order the
//! parser left, and the records come out highest-index-first.
//!
//! # Ring to splitter
//!
//! When the ring builder is composed with the splitter it pushes each record
//! `[index, colour, stride, ring]` **below** its three registers `[i, w2, h2]`,
//! with two fixed rolls. The registers stay on top, so the next lap still finds
//! `i` at depth one. At the end it pushes the cell count and rolls the registers
//! off, leaving `[count, records ..]` — exactly what the splitter's loader
//! leaves after its read loop, so the builder enters the splitter at the block
//! after that loop. The registers were below the ring all along, so for a moment
//! there is nothing else to do: once the splitter and the movement start, they
//! are inert values under the ring, the same place the value stack lives.
//!
//! No FIFO tape and no dynamic-depth roll: every roll above is a constant.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options};
use runtime::BufferIo;
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};
use crate::grid;
use crate::ppm;

/// The ring builder's blocks, numbered from one.
const R_ENTRY: u32 = 1;
const R_BODY: u32 = 2;
const R_GUARD: u32 = 3;
const R_CODEL: u32 = 4;
const R_NEXT: u32 = 5;
const R_DONE: u32 = 6;
const R_DECREMENT: u32 = 7;
/// How many blocks [`ring_blocks`] returns.
const R_COUNT: u32 = R_DECREMENT;

/// The code a guard cell carries: above every real colour.
const GUARD_CODE: i64 = 22;

/// What the ring builder does with each record.
#[derive(Clone, Copy)]
enum Output {
    /// Print `ring stride colour index` as each record is built, then halt.
    Print,
    /// Leave the records on the stack and hand over to `handover` when done.
    Records { handover: u32 },
}

/// The composed frontend image: PPM in, the ring stream out.
pub fn ppm_ring_grid() -> Result<CodelGrid, Error> {
    ppm_ring_program()?.image()
}

fn ppm_ring_program() -> Result<Program, Error> {
    Program::new(ppm_ring_blocks(), 1)
}

/// The ring stream a P3 PPM decodes to, built by the one image.
pub fn ppm_ring(source: &str) -> Result<String, Error> {
    run(ppm_ring_grid()?, source)
}

/// The composed frontend plus the splitter: PPM in, the ring's labels out.
pub fn ppm_ring_splitter_grid() -> Result<CodelGrid, Error> {
    ppm_ring_splitter_program()?.image()
}

fn ppm_ring_splitter_program() -> Result<Program, Error> {
    Program::new(ppm_ring_splitter_blocks(), 1)
}

/// The labels a P3 PPM's ring partitions to, built by the one image.
pub fn ppm_ring_splitter(source: &str) -> Result<String, Error> {
    run(ppm_ring_splitter_grid()?, source)
}

fn run(image: CodelGrid, source: &str) -> Result<String, Error> {
    /// A few thousand cells: the partition is the slow part.
    const STEPS: u64 = 400_000_000;

    let blocks = Blocks::partition(image);
    let mut io = BufferIo::new(source);
    let options = Options {
        max_steps: Some(STEPS),
        ..Options::default()
    };
    let result = interp::run(&blocks, &mut io, &options)?;
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
    all[(ppm::DONE - 1) as usize].end = End::Jump(offset + R_ENTRY);
    append(&mut all, ring_blocks(Output::Print), offset);
    all
}

/// The same, then the splitter's blocks, with the ring builder handing over to
/// the splitter's post-load block.
fn ppm_ring_splitter_blocks() -> Vec<Block> {
    let mut all = ppm::blocks_with(ppm::Output::Stack);
    let parser_len = all.len() as u32;
    let splitter_offset = parser_len + R_COUNT;
    // The splitter's local id, made local to the ring builder; `append` adds
    // `parser_len`, so the jump lands on the splitter's own block.
    let handover = R_COUNT + grid::HANDOVER;
    all[(ppm::DONE - 1) as usize].end = End::Jump(parser_len + R_ENTRY);
    append(
        &mut all,
        ring_blocks(Output::Records { handover }),
        parser_len,
    );
    append(&mut all, grid::blocks(), splitter_offset);
    all
}

/// Append `blocks` after `all`, offsetting their ids by `offset`.
fn append(all: &mut Vec<Block>, blocks: Vec<Block>, offset: u32) {
    for block in blocks {
        all.push(Block::new(block.nodes, offset_end(block.end, offset)));
    }
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
fn ring_blocks(output: Output) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(R_COUNT as usize);
    blocks.push(Block::new(entry(output), End::Jump(R_BODY)));
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
            nodes.extend(emit_record(output));
            nodes
        },
        End::Jump(R_NEXT),
    ));
    // An interior cell: bring the next pixel to the top.
    blocks.push(Block::new(
        {
            let mut nodes = drop_coordinates();
            nodes.extend(pop_pixel(output));
            nodes.extend(emit_record(output));
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
    blocks.push(match output {
        Output::Print => Block::new(Vec::new(), End::Halt),
        Output::Records { handover } => Block::new(finish_records(), End::Jump(handover)),
    });
    blocks.push(Block::new(
        vec![Node::Const(1), Node::Command(Command::Subtract)],
        End::Jump(R_BODY),
    ));
    blocks
}

/// `[width, height, pixels ..]` to `[i, w2, h2, pixels ..]`, printing the cell
/// count first when the records are printed.
fn entry(output: Output) -> Vec<Node> {
    let mut nodes = vec![
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
    ];
    if matches!(output, Output::Print) {
        nodes.extend([
            Node::DupLocal(1),
            Node::Const(1),
            Node::Command(Command::Add),
            Node::Command(Command::OutNumber),
            Node::Const(32),
            Node::Command(Command::OutChar),
        ]);
    }
    nodes
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

/// Emit one record from `[colour, i, w2, h2, ..]`, then restore the registers.
fn emit_record(output: Output) -> Vec<Node> {
    match output {
        Output::Print => print_record(),
        Output::Records { .. } => records_record(),
    }
}

/// Bring the next pixel to the top, leaving the rest in order.
///
/// Printed records leave no output on the stack, so the pixel is at the fixed
/// depth four. Collected records sit between the registers and the pixels, so
/// the pixel's depth depends on how many records have been written. One
/// `roll(depth, depth - 1)` is a right rotation by one: it lifts the pixel and
/// leaves the registers and the records in their original order, which is why
/// no put-back is needed.
fn pop_pixel(output: Output) -> Vec<Node> {
    match output {
        Output::Print => vec![Node::Roll(4, 3)],
        Output::Records { .. } => vec![
            // records = (w2 * h2 - 1) - i
            Node::DupLocal(2),
            Node::DupLocal(4),
            Node::Command(Command::Multiply),
            Node::Const(1),
            Node::Command(Command::Subtract),
            Node::DupLocal(2),
            Node::Command(Command::Subtract),
            // depth = 4 + 4 * records
            Node::Const(4),
            Node::Command(Command::Multiply),
            Node::Const(4),
            Node::Command(Command::Add),
            // rolls = depth - 1
            Node::DupLocal(1),
            Node::Const(1),
            Node::Command(Command::Subtract),
            Node::Command(Command::Roll),
        ],
    }
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

/// Push `[index, colour, stride, ring]` **below** the registers, from
/// `[colour, i, w2, h2, ..]`.
fn records_record() -> Vec<Node> {
    vec![
        // ring = w2 * 4 * h2
        Node::DupLocal(3),
        Node::Const(4),
        Node::Command(Command::Multiply),
        Node::DupLocal(5),
        Node::Command(Command::Multiply),
        // stride = w2 * 4
        Node::DupLocal(4),
        Node::Const(4),
        Node::Command(Command::Multiply),
        // the record, index on top
        Node::DupLocal(3), // colour
        Node::DupLocal(5), // index
        // the original colour is consumed; drop it
        Node::Roll(5, 4),
        Node::Command(Command::Pop),
        // [record .., i, w2, h2] -> [i, w2, h2, record ..]
        Node::Roll(7, 4),
    ]
}

/// Leave `[count, records ..]` and the registers gone.
fn finish_records() -> Vec<Node> {
    vec![
        Node::Const(0),
        // [0, i, w2, h2] -> [i, w2, h2, 0]
        Node::Roll(4, 1),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
        Node::Command(Command::Pop),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_composed_image_partitions_the_ring() {
        let cases = [
            vec!["r"],
            vec!["rg"],
            vec!["rg", "gb"],
            vec!["r.b", "#g#", "bk."],
            vec!["rrr", "r.r", "rrr"],
        ];
        for rows in cases {
            let grid = grid(&rows);
            let want = splitter_labels(&grid);
            let got = ppm_ring_splitter(&ppm_of(&grid)).unwrap();
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

    #[test]
    fn the_canonical_hello_world_partitions_in_the_one_image() {
        let png = include_bytes!("../fixtures/hello-world/program.png");
        let grid = CodelGrid::from_image(png).unwrap();
        let want = splitter_labels(&grid);
        let got = ppm_ring_splitter(&ppm_of(&grid)).unwrap();
        assert_eq!(tokens(&got), tokens(&want));
    }

    /// The label per ring cell, from the standalone splitter image.
    fn splitter_labels(grid: &CodelGrid) -> String {
        let (image, stream) = crate::splitter(grid).unwrap();
        run(image, &stream).unwrap()
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
