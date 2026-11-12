//! Reading a PPM image in the image.
//!
//! [`ppm_grid`] weaves a program that reads a P3 PPM from its input and prints
//! the grid it decodes to: `<width> <height>`, then one colour code per codel in
//! row-major order, each followed by a space, then a final newline. The codes are
//! indices into [`canvas::Color::ALL`], so 18 is white and 19 is black — the
//! same codes the splitter and the movement stages already speak.
//!
//! This is the last piece the host still does for a whole run: `canvas` decodes
//! a PNG, detects the codel size, and quantises the pixels, and this does the
//! same for the one container format simple enough to parse with `in number` and
//! `in char`. The host is left with the bookkeeping the ring needs — block sizes
//! and the label-to-block map — and with feeding this output to the next stage.
//!
//! The parser is deliberately small. It accepts `P3`, a maxval of 255, and no
//! comments, because those are the three things a PPM written from a Piet image
//! has; anything else prints `-1` and stops rather than decoding to a grid that
//! is silently wrong. A `P6` is refused outright: its bytes are not a UTF-8
//! stream, and the runtime reads input as code points.
//!
//! # How it is built
//!
//! A pixel's colour is a function of its three components, each of which is
//! `0`, `192` or `255` in the palette and something else otherwise. Every
//! component becomes a digit — `0`, `1`, `2`, or `3` for "not a palette value" —
//! and the three digits index a 64-entry table of colour codes. Anything holding
//! a `3`, and any of the six colour-space cells the palette leaves empty, is
//! white, which is what the specification says a non-standard colour means.
//!
//! The digits are classified with three comparisons per channel rather than
//! arithmetic: division and remainder cannot tell `0` from `64`, and a
//! non-standard colour has to be recognised, not approximated.

use canvas::{Blocks, CodelGrid, Color};
use interp::{ExitReason, Options};
use runtime::BufferIo;
use tessera::Command;

use crate::error::Error;
use crate::flow::{Block, End, Node, Program};

/// The code `canvas::Color::ALL` gives white, and black.
const WHITE: i64 = 18;
const BLACK: i64 = 19;

/// The three components of a pixel, in the order a PPM writes them.
const RED: u32 = 0;
const GREEN: u32 = 1;
const BLUE: u32 = 2;

/// The lengths and blocks of the layout, in the order they are emitted.
///
/// Block ids are the program's index plus one, so each group's first id is the
/// count of everything before it. The channels and the table are contiguous
/// because the channel blocks jump to the next channel by arithmetic and the
/// table is reached by `Computed`.
const ENTRY: u32 = 1;
const MAGIC: u32 = 2;
const WIDTH: u32 = 3;
const HEIGHT: u32 = 4;
const MAXVAL: u32 = 5;
const LOOP: u32 = 6;
pub(crate) const DONE: u32 = 7;
/// The red channel's read block; a channel owns [`PER_CHANNEL`] blocks.
const CHANNEL: u32 = 8;
const PER_CHANNEL: u32 = 8;
const COMBINE: u32 = CHANNEL + PER_CHANNEL * 3;
const TABLE: u32 = COMBINE + 1;
const TABLE_LEN: u32 = 4 * 4 * 4;
const EMIT: u32 = TABLE + TABLE_LEN;
const BAD: u32 = EMIT + 1;

/// The program that reads a P3 PPM and prints the grid's colour codes.
pub fn ppm_program() -> Result<Program, Error> {
    Program::new(blocks(), ENTRY)
}

/// Where the parser's output goes.
#[derive(Clone, Copy)]
pub(crate) enum Output {
    /// Print the grid codes: `<width> <height>`, then one code per codel.
    Print,
    /// Leave the grid codes on the stack for the next stage: `width` on top,
    /// then `height`, then the codes with the last pixel on top. That order is
    /// what a reverse ring builder reads with plain pops.
    Stack,
}

/// The same program, laid out as an image.
pub fn ppm_grid() -> Result<CodelGrid, Error> {
    ppm_program()?.image()
}

/// The grid a P3 PPM decodes to, decoded by [`ppm_grid`].
///
/// This is the whole of the frontend's host side: run the image, read the codes
/// back, and package them as a grid. Everything the image cannot print — the
/// block partition and the walker's records — belongs to [`crate::grid_graph`]
/// and [`crate::graph_walker`], which take a grid.
pub fn ppm_to_grid(source: &str) -> Result<CodelGrid, Error> {
    let output = ppm_codes(source)?;
    let bad = || Error::BadOutput {
        output: output.clone(),
    };
    let mut tokens = output.split_whitespace();
    let width: u32 = tokens.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let height: u32 = tokens.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let mut codels = Vec::with_capacity((width as usize) * (height as usize));
    for token in tokens {
        let code: usize = token.parse().map_err(|_| bad())?;
        codels.push(*Color::ALL.get(code).ok_or_else(bad)?);
    }
    CodelGrid::from_colors(width, height, codels).map_err(Error::Canvas)
}

/// The colour codes [`ppm_grid`] prints for a P3 PPM.
///
/// This is the text the ring image takes, so a picture's whole frontend is
/// images and the host only carries the characters:
/// [`ppm_codes`] -> [`crate::ring_to_stream`] -> [`crate::splitter`].
pub fn ppm_codes(source: &str) -> Result<String, Error> {
    /// The image reads three numbers per codel and prints one; this is ample.
    const STEPS: u64 = 20_000_000;

    let image = ppm_grid()?;
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

/// The colour code for three digits, as `canvas::Color::ALL` numbers them.
fn palette_code(red: u32, green: u32, blue: u32) -> i64 {
    match (red, green, blue) {
        // A `3` means a component that is not a palette value.
        (0..=2, 0..=2, 0..=2) => match (red, green, blue) {
            (2, 2, 2) => WHITE,
            (0, 0, 0) => BLACK,
            (2, 1, 1) => 0,
            (2, 0, 0) => 1,
            (1, 0, 0) => 2,
            (2, 2, 1) => 3,
            (2, 2, 0) => 4,
            (1, 1, 0) => 5,
            (1, 2, 1) => 6,
            (0, 2, 0) => 7,
            (0, 1, 0) => 8,
            (1, 2, 2) => 9,
            (0, 2, 2) => 10,
            (0, 1, 1) => 11,
            (1, 1, 2) => 12,
            (0, 0, 2) => 13,
            (0, 0, 1) => 14,
            (2, 1, 2) => 15,
            (2, 0, 2) => 16,
            (1, 0, 1) => 17,
            // A colour the palette leaves empty is white, like a non-standard one.
            _ => WHITE,
        },
        _ => WHITE,
    }
}

fn blocks() -> Vec<Block> {
    blocks_with(Output::Print)
}

/// The parser's blocks, with the output either printed or left on the stack.
pub(crate) fn blocks_with(output: Output) -> Vec<Block> {
    let not = Node::Command(Command::Not);
    let subtract = Node::Command(Command::Subtract);
    let pop = Node::Command(Command::Pop);
    let bytes = |text: u8| Node::Const(i64::from(text));

    let mut blocks: Vec<Block> = Vec::with_capacity(BAD as usize);

    // The magic. `in char` leaves the code point, and subtracting the expected
    // one consumes it, so both branches start with the stack untouched.
    blocks.push(Block::new(
        vec![Node::Command(Command::InChar), bytes(b'P'), subtract, not],
        End::If {
            then: MAGIC,
            otherwise: BAD,
        },
    ));
    blocks.push(Block::new(
        vec![Node::Command(Command::InChar), bytes(b'3'), subtract, not],
        End::If {
            then: WIDTH,
            otherwise: BAD,
        },
    ));

    // The width. The printed form prints it as it arrives; the stack form keeps
    // it for the header rearrangement.
    let width = match output {
        Output::Print => vec![
            Node::Command(Command::InNumber),
            Node::Dup,
            Node::Command(Command::OutNumber),
            bytes(b' '),
            Node::Command(Command::OutChar),
        ],
        Output::Stack => vec![Node::Command(Command::InNumber)],
    };
    blocks.push(Block::new(width, End::Jump(HEIGHT)));

    // The height, and the pixel counter. The stack form leaves
    // `[counter, width, height]` so the channels and the accumulator sit under
    // the counter and the final `pop` leaves `[width, height, codes ..]`.
    let height = match output {
        Output::Print => vec![
            Node::Command(Command::InNumber),
            Node::Dup,
            Node::Command(Command::OutNumber),
            bytes(b'\n'),
            Node::Command(Command::OutChar),
            Node::Command(Command::Multiply),
        ],
        Output::Stack => vec![
            Node::Command(Command::InNumber),
            Node::Roll(2, 1),
            Node::Dup,
            Node::DupLocal(3),
            Node::Command(Command::Multiply),
        ],
    };
    blocks.push(Block::new(height, End::Jump(MAXVAL)));
    // The values are unscaled, so anything but 255 would decode to nonsense.
    blocks.push(Block::new(
        vec![
            Node::Command(Command::InNumber),
            Node::Const(255),
            subtract,
            not,
        ],
        End::If {
            then: LOOP,
            otherwise: BAD,
        },
    ));

    // One pixel per lap; the counter lives alone under each pixel's values.
    blocks.push(Block::new(
        vec![Node::Dup, not],
        End::If {
            then: DONE,
            otherwise: CHANNEL,
        },
    ));
    // Done: printed as a newline; on the stack the counter goes and the codes
    // are left for the ring builder.
    let done = match output {
        Output::Print => Block::new(
            vec![bytes(b'\n'), Node::Command(Command::OutChar)],
            End::Halt,
        ),
        Output::Stack => Block::new(vec![pop], End::Halt),
    };
    blocks.push(done);

    // Each channel: read one component, classify it as 0, 1, 2, or 3, and push
    // the digit. The classification chains through three equality tests; the
    // value is under the temporary so `Pop` is what discards it.
    for channel in [RED, GREEN, BLUE] {
        let base = CHANNEL + PER_CHANNEL * channel;
        let next = if channel == BLUE {
            COMBINE
        } else {
            base + PER_CHANNEL
        };
        blocks.push(Block::new(
            vec![Node::Command(Command::InNumber)],
            End::Jump(base + 1),
        ));
        blocks.push(Block::new(
            vec![Node::Dup, Node::Const(0), subtract, not],
            End::If {
                then: base + 2,
                otherwise: base + 3,
            },
        ));
        blocks.push(Block::new(vec![pop, Node::Const(0)], End::Jump(next)));
        blocks.push(Block::new(
            vec![Node::Dup, Node::Const(192), subtract, not],
            End::If {
                then: base + 4,
                otherwise: base + 5,
            },
        ));
        blocks.push(Block::new(vec![pop, Node::Const(1)], End::Jump(next)));
        blocks.push(Block::new(
            vec![Node::Dup, Node::Const(255), subtract, not],
            End::If {
                then: base + 6,
                otherwise: base + 7,
            },
        ));
        blocks.push(Block::new(vec![pop, Node::Const(2)], End::Jump(next)));
        blocks.push(Block::new(vec![pop, Node::Const(3)], End::Jump(next)));
    }

    // The three digits become `16 * red + 4 * green + blue`. The base is four,
    // not the palette's three, so that a `3` digit stays unambiguous.
    blocks.push(Block::new(
        vec![
            Node::Roll(3, 2),
            Node::Const(4),
            Node::Command(Command::Multiply),
            Node::Roll(3, 2),
            Node::Command(Command::Add),
            Node::Const(4),
            Node::Command(Command::Multiply),
            Node::Command(Command::Add),
            Node::Base(TABLE),
            Node::Command(Command::Add),
        ],
        End::Computed,
    ));

    // The table. Each cell pushes its colour code and hands over to the print.
    for index in 0..TABLE_LEN {
        blocks.push(Block::new(
            vec![Node::Const(palette_code(
                index / 16,
                (index / 4) % 4,
                index % 4,
            ))],
            End::Jump(EMIT),
        ));
    }

    // The code. Printed with its separator; on the stack it goes under the
    // header (`counter, width, height`) and above the codes already collected.
    let emit = match output {
        Output::Print => vec![
            Node::Command(Command::OutNumber),
            bytes(b' '),
            Node::Command(Command::OutChar),
            Node::Const(1),
            subtract,
        ],
        Output::Stack => vec![Node::Roll(4, 1), Node::Const(1), subtract],
    };
    blocks.push(Block::new(emit, End::Jump(LOOP)));

    // What the header refusals print: a value no colour code can be.
    blocks.push(Block::new(
        vec![Node::Const(-1), Node::Command(Command::OutNumber)],
        End::Halt,
    ));

    assert_eq!(blocks.len() as u32, BAD, "the layout constants must match");
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_matches_the_palette() {
        for color in canvas::Color::ALL {
            let [red, green, blue] = color.rgb();
            let digit = |component: u8| -> u32 {
                match component {
                    0 => 0,
                    192 => 1,
                    _ => 2,
                }
            };
            let code = palette_code(digit(red), digit(green), digit(blue));
            let expected = canvas::Color::ALL
                .iter()
                .position(|candidate| *candidate == color)
                .unwrap() as i64;
            assert_eq!(code, expected, "{color:?}");
        }
    }

    #[test]
    fn a_component_off_the_palette_is_white() {
        assert_eq!(palette_code(3, 0, 0), WHITE);
        assert_eq!(palette_code(0, 3, 2), WHITE);
    }

    #[test]
    fn a_colour_the_palette_leaves_empty_is_white() {
        // (255, 0, 192) is not one of the twenty.
        assert_eq!(palette_code(2, 0, 1), WHITE);
    }
}
