#![allow(clippy::unwrap_used)]
//! The AOT runner: load the appended payload and execute `piet_main`.
//!
//! `pietrs build` appends a payload and footer to the runner compiled from
//! `loader.rs`; `aot/build.rs` compiles that same file into the runner bytes
//! embedded in `pietrs`. The loader maps the payload, resolves the five host
//! imports, applies the relocations, and calls the entry symbol.

mod loader;

pub use loader::{execute, run_file, start};
