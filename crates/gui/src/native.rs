//! The eframe shell: a window, the dialogs, and the process entry point.

use std::path::PathBuf;

use crate::{EditorApp, FileError, ThemeChoice};

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

    #[error(transparent)]
    File(#[from] FileError),

    #[error("cannot open the editor window: {0}")]
    Window(#[from] eframe::Error),
}

/// Opens the editor window and runs until it closes.
pub fn run(args: Args) -> Result<(), Error> {
    let mut app = EditorApp::new(editor::Document::new(32, 32)?, None)?;
    if let Some(path) = &args.image {
        app.open_file_with_codel_size(path, args.codel_size)?;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "pietrs",
        options,
        Box::new(move |cc| {
            let mut app = app;
            if let Some(storage) = cc.storage
                && let Some(name) = eframe::get_value::<String>(storage, "theme")
                && let Ok(theme) = name.parse::<ThemeChoice>()
            {
                app.set_theme(theme);
            }
            Ok(Box::new(NativeApp { app }))
        }),
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

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "theme", &self.app.theme().as_str());
    }
}
