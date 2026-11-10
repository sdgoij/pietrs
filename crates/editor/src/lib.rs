#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The editable document and the debug session, with no UI.
//!
//! [`Document`] is the codel grid and its undo history; [`Analysis`] is the
//! block partition and state graph for a revision of it; [`Session`] is one
//! interpreter run, stepped by the debugger. `gui` renders these and nothing
//! else.

mod analysis;
mod document;
mod session;

pub use analysis::{Analysis, Report};
pub use document::Document;
pub use session::{Breakpoints, Budget, Session, Stop};
