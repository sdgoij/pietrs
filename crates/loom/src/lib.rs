#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! A Piet program that interprets Piet programs.
//!
//! [`interpreter`] weaves an image that reads a program from its input and
//! runs it. The interpreted program is Piet's straight-line fragment — the
//! fifteen commands that leave the direction pointer alone — encoded as a
//! stream of opcodes. [`linear_program`] builds a sample program in that
//! fragment, and [`instructions`] reads a program's instruction sequence back
//! out of its trace, so the two images can be checked against each other.
//!
//! The interpreter image is generated rather than drawn: every command in Piet
//! is a colour transition, so a program is a colour walk over a grid, and a
//! layout is a path with a colour for each cell. The path is fixed first, then
//! the filler between real commands is chosen by a search over the eighteen
//! chromatic colours; [`interpreter`] retries the free colours until the
//! partition has exactly the blocks the path needs.
//!
//! ```
//! use canvas::Blocks;
//! use interp::Options;
//! use runtime::BufferIo;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let image = loom::interpreter()?;
//! let blocks = Blocks::partition(image);
//! // push 65, out char, halt
//! let mut io = BufferIo::new("1 65 15 0");
//! interp::run(&blocks, &mut io, &Options::default())?;
//! assert_eq!(io.output(), "A");
//! # Ok(())
//! # }
//! ```

mod error;
mod interpreter;
mod linear;
mod paint;
mod program;
mod shade;

pub use error::Error;
pub use interpreter::interpreter;
pub use linear::linear_program;
pub use program::{Instruction, OPCODES, encode, instructions, opcode};
