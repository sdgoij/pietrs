//! The eframe shell: a window, the dialogs, the filesystem, and the process
//! entry point.

use std::path::{Path, PathBuf};

use crate::{EditorApp, FileRequest, IoResult, ThemeChoice};

/// What `pietrs editor` starts with.
#[derive(Debug, Clone)]
pub struct Args {
    pub image: Option<PathBuf>,
    pub codel_size: Option<u32>,
}

/// Why reading or writing a program failed.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error(transparent)]
    Canvas(#[from] canvas::Error),

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
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
        let bytes = read_path(path)?;
        match args.codel_size {
            Some(size) => app.open_bytes_with_codel_size(&bytes, size, Some(path.clone()))?,
            None => app.open_bytes(&bytes, Some(path.clone()))?,
        }
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

fn read_path(path: &Path) -> Result<Vec<u8>, FileError> {
    std::fs::read(path).map_err(|source| FileError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn write_path(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    std::fs::write(path, bytes).map_err(|source| FileError::Write {
        path: path.to_path_buf(),
        source,
    })
}

struct NativeApp {
    app: EditorApp,
}

impl NativeApp {
    /// Performs the file operation the app asked for, if any.
    ///
    /// The dialogs block, so a request completes inside the frame that made it
    /// and the app's `io_pending` window never spans a frame. Returns whether a
    /// result was applied, because it lands after the frame has drawn.
    fn service(&mut self) -> bool {
        match self.app.take_request() {
            Some(FileRequest::Open) => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Piet image", &["png", "gif"])
                    .pick_file()
                else {
                    self.app.apply_io(IoResult::Cancelled);
                    return true;
                };
                let result = match read_path(&path) {
                    Ok(bytes) => IoResult::Opened {
                        bytes,
                        path: Some(path),
                    },
                    Err(error) => IoResult::Failed(error.to_string()),
                };
                self.app.apply_io(result);
                true
            }
            Some(FileRequest::Save { path, bytes }) => {
                let Some(path) = path.or_else(|| self.save_dialog()) else {
                    self.app.apply_io(IoResult::Cancelled);
                    return true;
                };
                let mut path = path;
                if path.extension().is_none() {
                    path.set_extension("png");
                }
                let result = match write_path(&path, &bytes) {
                    Ok(()) => IoResult::Saved { path: Some(path) },
                    Err(error) => IoResult::Failed(error.to_string()),
                };
                self.app.apply_io(result);
                true
            }
            None => false,
        }
    }

    fn save_dialog(&self) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().add_filter("PNG", &["png"]);
        if let Some(name) = self.app.file_name() {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file()
    }
}

impl eframe::App for NativeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.app.ui_on(ui);
        if self.service() {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "theme", &self.app.theme().as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Color, Hue, Lightness, Point};
    use editor::Document;

    fn document(colours: &[Color]) -> Document {
        let width = colours.len() as u32;
        Document::from_grid(&CodelGrid::from_colors(width, 1, colours.to_vec()).unwrap())
    }

    fn red() -> Color {
        Color::chromatic(Hue::Red, Lightness::Normal)
    }

    fn green() -> Color {
        Color::chromatic(Hue::Green, Lightness::Normal)
    }

    #[test]
    fn a_saved_png_reopens_the_same_document() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("program.png");

        let mut app = EditorApp::new(
            document(&[red(), green()]),
            Some(PathBuf::from("start.png")),
        )
        .unwrap();
        app.set_colour(Color::Black);
        app.fill_at(Point::new(0, 0));
        assert!(app.is_dirty());

        // What the shell does with a `Save` request.
        let bytes = app.export_png().unwrap();
        write_path(&path, &bytes).unwrap();
        app.apply_io(IoResult::Saved {
            path: Some(path.clone()),
        });
        assert!(!app.is_dirty());

        // What the shell does with an `Open` result.
        let mut reopened = EditorApp::new(document(&[red(), red(), red()]), None).unwrap();
        reopened.apply_io(IoResult::Opened {
            bytes: read_path(&path).unwrap(),
            path: Some(path),
        });
        assert_eq!(reopened.document().width(), 2);
        assert_eq!(reopened.document().height(), 1);
        assert_eq!(
            reopened.document().color(Point::new(0, 0)),
            Some(Color::Black)
        );
        assert_eq!(reopened.document().color(Point::new(1, 0)), Some(green()));
        assert!(!reopened.is_dirty());
        assert!(reopened.session().is_none());
    }

    #[test]
    fn an_unreadable_image_is_reported_and_the_document_stays() {
        let mut app = EditorApp::new(document(&[red(), green()]), None).unwrap();
        let before = app.document().color(Point::new(0, 0));

        app.apply_io(IoResult::Opened {
            bytes: vec![0, 1, 2, 3],
            path: None,
        });
        assert_eq!(app.document().color(Point::new(0, 0)), before);
        assert_eq!(app.document().width(), 2);
        assert!(!app.io_pending());
    }
}
