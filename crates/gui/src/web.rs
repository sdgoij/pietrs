//! The browser shell: a canvas, the async dialogs, and the wasm entry.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use crate::{EditorApp, FileRequest, IoResult, ThemeChoice};

/// What `index.html` constructs and starts.
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
    results: Rc<RefCell<Vec<IoResult>>>,
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            runner: eframe::WebRunner::new(),
            results: Rc::default(),
        }
    }

    /// Starts the editor on `canvas`.
    ///
    /// # Errors
    /// Failing to create the WebGL context or the app.
    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        let results = Rc::clone(&self.results);
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    let mut app = EditorApp::new(editor::Document::new(32, 32)?, None)?;
                    if let Some(storage) = cc.storage
                        && let Some(name) = eframe::get_value::<String>(storage, "theme")
                        && let Ok(theme) = name.parse::<ThemeChoice>()
                    {
                        app.set_theme(theme);
                    }
                    Ok(Box::new(WebApp { app, results }))
                }),
            )
            .await
    }

    /// Stops the frame loop and releases the canvas.
    pub fn destroy(&self) {
        self.runner.destroy();
    }

    /// Whether the app has panicked since it started.
    pub fn has_panicked(&self) -> bool {
        self.runner.has_panicked()
    }

    /// The message of the first panic, if there was one.
    pub fn panic_message(&self) -> Option<String> {
        self.runner.panic_summary().map(|summary| summary.message())
    }
}

impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

struct WebApp {
    app: EditorApp,
    results: Rc<RefCell<Vec<IoResult>>>,
}

impl eframe::App for WebApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A dialog resolves on a later frame; apply what arrived before the
        // draw, so the frame after the dialog already shows the new document.
        for result in self.results.borrow_mut().drain(..) {
            self.app.apply_io(result);
        }

        self.app.ui_on(ui);

        let Some(request) = self.app.take_request() else {
            return;
        };
        let results = Rc::clone(&self.results);
        let ctx = ui.ctx().clone();
        wasm_bindgen_futures::spawn_local(async move {
            results.borrow_mut().push(service(request).await);
            ctx.request_repaint();
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "theme", &self.app.theme().as_str());
    }
}

/// Runs one file dialog and whatever follows it.
///
/// Always yields a result: a dismissed dialog is [`IoResult::Cancelled`], which
/// is what clears the app's `io_pending` flag.
async fn service(request: FileRequest) -> IoResult {
    match request {
        FileRequest::Open => {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("Piet image", &["png", "gif"])
                .pick_file()
                .await
            else {
                return IoResult::Cancelled;
            };
            let name = file.file_name();
            let bytes = file.read().await;
            // Browsers do not expose the file's path; the name is the document.
            IoResult::Opened {
                bytes,
                path: Some(PathBuf::from(name)),
            }
        }
        FileRequest::Save { path, bytes } => {
            let mut dialog = rfd::AsyncFileDialog::new();
            if let Some(name) = path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
            {
                dialog = dialog.set_file_name(name);
            }
            // On the web this resolves immediately; `write` is the download.
            let Some(file) = dialog.save_file().await else {
                return IoResult::Cancelled;
            };
            match file.write(&bytes).await {
                Ok(()) => IoResult::Saved {
                    path: Some(PathBuf::from(file.file_name())),
                },
                Err(error) => IoResult::Failed(error.to_string()),
            }
        }
    }
}
