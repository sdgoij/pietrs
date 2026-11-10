//! The eframe shell: a window, the dialogs, and the process entry point.

use std::path::PathBuf;

use crate::EditorApp;

/// What `pietrs editor` starts with.
#[derive(Debug, Clone)]
pub struct Args {
    pub image: Option<PathBuf>,
    pub codel_size: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Canvas(#[from] canvas::Error),

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot open the editor window: {0}")]
    Window(#[from] eframe::Error),
}

/// Opens the editor window and runs until it closes.
pub fn run(args: Args) -> Result<(), Error> {
    let (document, path) = match &args.image {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|source| Error::Read {
                path: path.clone(),
                source,
            })?;
            let grid = match args.codel_size {
                Some(size) => canvas::CodelGrid::from_image_with_codel_size(&bytes, size)?,
                None => canvas::CodelGrid::from_image(&bytes)?,
            };
            (editor::Document::from_grid(&grid), Some(path.clone()))
        }
        None => (editor::Document::new(32, 32)?, None),
    };
    let app = EditorApp::new(document, path)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "pietrs",
        options,
        Box::new(move |_cc| Ok(Box::new(NativeApp { app }))),
    )?;
    Ok(())
}

struct NativeApp {
    app: EditorApp,
}

impl eframe::App for NativeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui_on(ui);
    }
}
