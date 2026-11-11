#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The tessellation flow: commands, movement, white slides, and start states.
//!
//! The state is a coloured block plus the direction pointer and codel chooser.
//! [`exit`] resolves one step of the movement rules against the static image;
//! [`start`] resolves the upper-left codel into the first state.

mod command;
mod direction;
mod exit;
mod graph;

pub use command::{Command, command};
pub use direction::{Chooser, Direction};
pub use exit::{Exit, Start, State, block_edge, edge_codel, exit, start};
pub use graph::{Branch, Edge, StateGraph};
