#![allow(clippy::unwrap_used)]
//! Test scaffolding: builders, PNG fixtures, trace parsing, and verdicts.
//!
//! No shipped crate depends on this crate. `canvas` and `tessera` keep local
//! helpers because depending on `testkit` would close a dependency cycle;
//! `xtask` is the only non-test consumer.

mod blocks;
mod checks;
mod colors;
mod png;
mod trace;

pub use blocks::{blocks, id_at, row};
pub use checks::Checks;
pub use colors::{
    BLUE, BLUE_DARK, BLUE_LIGHT, CYAN_LIGHT, GREEN, GREEN_DARK, GREEN_NORMAL, MAGENTA, RED,
    RED_DARK, RED_LIGHT, WHITE,
};
pub use png::{png_bytes, write_png};
pub use trace::{TraceLine, format_trace, parse_trace};
