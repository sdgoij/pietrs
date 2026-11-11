#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! A Piet program that interprets Piet programs.
//!
//! [`interpreter`] weaves an image that reads a program from its input and
//! runs it. The interpreted program is Piet's straight-line fragment — the
//! fifteen commands that leave the direction pointer alone — encoded as a
//! stream of opcodes. [`assemble`] builds an image for any instruction
//! sequence in that fragment, [`linear_program`] builds one that prints text,
//! and [`instructions`] reads a program's instruction sequence back out of its
//! trace, so the two images can be checked against each other.
//!
//! The interpreter image is generated rather than drawn: every command in Piet
//! is a colour transition, so a program is a colour walk over a grid, and a
//! layout is a path with a colour for each cell. The path is fixed first, then
//! the filler between real commands is chosen by a search over the eighteen
//! chromatic colours; [`interpreter`] retries the free colours until the
//! partition has exactly the blocks the path needs.
//!
//! [`Program`] is the way past straight lines. It is a structured program —
//! basic blocks that compute their successor id and hand it to a shared
//! dispatcher — so loops and branches can be generated without ever laying out
//! a branch. A block that needs two successors selects one arithmetically.
//!
//! [`compile`] is the way past a trace. It turns a program's tessellation graph
//! into a [`Program`]: one block per state, jumps between them, and a `pointer`
//! or `switch` state selecting its successor from the value stack at run time.
//!
//! [`splitter`] is the way past the host's partition: it reads a grid's colours
//! and reports the block each ring cell belongs to, so the first half of the
//! tessellation happens in the image rather than before it.
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
mod flow;
mod graph;
mod grid;
mod interpreter;
mod linear;
mod paint;
mod program;
mod shade;
mod walker;

pub use error::Error;
pub use flow::{Block, End, Node, Program};
pub use graph::compile;
pub use grid::{block_edge, exit_state, splitter, start_state, step_state};
pub use interpreter::interpreter;
pub use linear::{assemble, linear_program};
pub use program::{Instruction, OPCODES, encode, instructions, opcode};
pub use walker::{COMMAND_DEPTH, RECORD, graph_walker, skeleton, walker};
