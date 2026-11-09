#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The canvas: a Piet image decoded into a codel grid and partitioned into
//! 4-connected colour blocks.
//!
//! This crate owns the representation, not the semantics. A [`CodelGrid`] is the
//! picture after codel-size detection and palette quantisation; [`Blocks`] is
//! the 4-connected partition the movement rules walk.

mod blocks;
mod color;
mod decode;
mod error;
mod grid;

pub use blocks::{Block, BlockId, Blocks};
pub use color::{Color, Hue, Lightness};
pub use error::Error;
pub use grid::{CodelGrid, Point};
