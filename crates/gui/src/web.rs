//! The browser shell: a canvas, the async dialogs, and the wasm entry.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::{EditorApp, FileRequest, IoResult, ThemeChoice};

/// The program the demo opens with, so the page shows a running image instead
/// of a blank grid. This is the canonical Piet "Hello world!" (30x29 codels),
/// from the specification page linked in the footer.
const DEMO_IMAGE: &[u8] = include_bytes!("../web/hello.png");
const DEMO_NAME: &str = "hello.png";

/// What `index.html` constructs and starts.
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
    results: Rc<Cell<Vec<IoResult>>>,
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
                    app.open_bytes(DEMO_IMAGE, Some(PathBuf::from(DEMO_NAME)))?;
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

/// The queue between the dialog futures and the next frame.
///
/// A `Cell` rather than a `RefCell` on purpose: `take` and `set` cannot fail,
/// so a future cannot hold a borrow across an `.await` and abort the module
/// when the next frame tries to read the queue.
struct WebApp {
    app: EditorApp,
    results: Rc<Cell<Vec<IoResult>>>,
}

impl eframe::App for WebApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A dialog resolves on a later frame; apply what arrived before the
        // draw, so the frame after the dialog already shows the new document.
        for result in self.results.take() {
            self.app.apply_io(result);
        }

        self.app.ui_on(ui);

        let Some(request) = self.app.take_request() else {
            return;
        };
        let results = Rc::clone(&self.results);
        let ctx = ui.ctx().clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = service(request).await;
            let mut queued = results.take();
            queued.push(result);
            results.set(queued);
            ctx.request_repaint();
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "theme", &self.app.theme().as_str());
    }
}

/// Runs one file operation.
///
/// Always yields a result: a dismissed picker is [`IoResult::Cancelled`], which
/// is what clears the app's `io_pending` flag.
async fn service(request: FileRequest) -> IoResult {
    match request {
        FileRequest::Open => {
            // rfd renders an `<input type=file>` and clicks it, which opens the
            // browser's picker while the click's user activation is still live.
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
            // No dialog: a Blob URL with `download` is the browser's save, and
            // it completes without waiting for another click.
            let name = path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("program.png")
                .to_owned();
            match download(&name, &bytes) {
                Ok(()) => IoResult::Saved {
                    path: Some(PathBuf::from(name)),
                },
                Err(message) => IoResult::Failed(message),
            }
        }
    }
}

/// Hands `bytes` to the browser as a download called `name`.
fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;

    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::new_from_slice(bytes).buffer());

    let options = web_sys::BlobPropertyBag::new();
    options.set_type("image/png");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|_| "cannot build the download".to_owned())?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|_| "cannot create the download URL".to_owned())?;

    let anchor: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|_| "cannot create the download link".to_owned())?
        .dyn_into()
        .map_err(|_| "cannot create the download link".to_owned())?;
    anchor.set_href(&url);
    anchor.set_download(name);
    body.append_child(&anchor)
        .map_err(|_| "cannot attach the download link".to_owned())?;
    anchor.click();
    anchor.remove();
    web_sys::Url::revoke_object_url(&url)
        .map_err(|_| "cannot release the download URL".to_owned())?;

    Ok(())
}
