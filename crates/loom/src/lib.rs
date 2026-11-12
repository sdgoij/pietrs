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
//! tessellation happens in the image rather than before it. [`grid_graph`]
//! finishes that path: it resolves the state graph with the image's start and exit
//! stages and the partition with the splitter, so the host only assembles the
//! walker's records.
//!
//! [`machine`] is the interpreter's control loop: the movement rule applied again
//! and again to the state it is in, printing the trace. It is the first slice of
//! one image that runs a program from its pixels, and the note's S10 says what the
//! rest of that image needs.
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

mod compose;
mod error;
mod flow;
mod graph;
mod grid;
mod grid_graph;
mod interpreter;
mod linear;
mod paint;
mod ppm;
mod program;
mod ring;
mod shade;
mod walker;

pub use compose::{ppm_ring, ppm_ring_grid, ppm_ring_splitter, ppm_ring_splitter_grid};
pub use error::Error;
pub use flow::{Block, End, Node, Program};
pub use graph::compile;
pub use grid::{
    Partition, block_edge, exit_state, grid_partition, grid_ring, machine, machine_grid,
    ring_stream, run, run_grid, splitter, splitter_image, start_state, step_state, with_exit_query,
    with_step_query,
};
pub use grid_graph::grid_graph;
pub use interpreter::interpreter;
pub use linear::{assemble, linear_program};
pub use ppm::{ppm_codes, ppm_grid, ppm_program, ppm_to_grid};
pub use program::{Instruction, OPCODES, encode, instructions, opcode};
pub use ring::{grid_codes, ring_grid, ring_program, ring_to_stream};
pub use walker::{
    COMMAND_DEPTH, RECORD, Sizes, graph_walker, record_code, record_command, skeleton, walker,
};
