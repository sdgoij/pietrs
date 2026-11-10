#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The egui editor and debugger.
//!
//! [`EditorApp`] is egui-only and has no window types, so a test can render a
//! frame through `egui::Context::run_ui`. The `native` feature adds the eframe
//! shell and `gui::run`, which is what `pietrs editor` calls; the shell owns
//! every file dialog and every filesystem access, and the app reaches them
//! through [`FileRequest`] and [`IoResult`].

mod app;
#[cfg(feature = "native")]
mod native;

pub use app::{EditorApp, FileRequest, IoResult, ThemeChoice};
#[cfg(feature = "native")]
pub use native::{Args, Error, FileError, run};
