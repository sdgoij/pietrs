#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The egui editor and debugger.
//!
//! [`EditorApp`] is egui-only and has no window types, so a test can render a
//! frame through `egui::Context::run_ui`. The `native` feature adds the eframe
//! shell and `gui::run`, which is what `pietrs editor` calls; the `web` feature
//! adds the wasm entry that `index.html` starts. In both shells the app reaches
//! the dialogs and the filesystem through [`FileRequest`] and [`IoResult`].

mod app;
#[cfg(feature = "native")]
mod native;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web;

#[cfg(all(feature = "web", not(target_arch = "wasm32")))]
compile_error!(
    "the `web` feature builds for `wasm32-unknown-unknown`; pass --target wasm32-unknown-unknown"
);

pub use app::{EditorApp, FileRequest, IoResult, ThemeChoice};
#[cfg(feature = "native")]
pub use native::{Args, Error, FileError, run};
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub use web::WebHandle;
