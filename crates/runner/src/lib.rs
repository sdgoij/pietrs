#![allow(clippy::unwrap_used)]
//! The AOT runner: load the appended payload and execute it.
//!
//! `pietrs build` appends a payload and footer to the runner compiled from
//! `loader.rs`; `aot/build.rs` compiles that same file into the runner bytes
//! embedded in `pietrs`. For an image small enough to compile, the loader maps
//! the payload, resolves the five host imports, applies the relocations, and
//! calls `piet_main`. A larger image carries its state graph as data instead;
//! the loader recognizes that payload and walks it with a table lookup per step.

mod loader;

pub use loader::{execute, run_file, start};
