#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The stack, arithmetic, and I/O shared by the interpreter and compiled code.

mod arithmetic;
mod io;
mod stack;

/// The value type the interpreter and compiled code use.
pub type Value = i64;

pub use arithmetic::{
    Division, floored_div, floored_mod, greater, not, truncating_div, truncating_mod,
};
pub use io::{BufferIo, Input, Io, Output};
pub use stack::Stack;

use thiserror::Error;

/// A runtime failure the specification does not ignore.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    #[error("the stack overflowed")]
    Overflow,
    #[error("the roll depth {0} is invalid")]
    InvalidDepth(Value),
}
