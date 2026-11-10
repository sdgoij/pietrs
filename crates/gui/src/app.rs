//! The egui view, with no window types.

use std::path::PathBuf;

use editor::{Analysis, Document};
use egui::{CornerRadius, TextureHandle, TextureOptions, ThemePreference};
use tessera::Start;

/// Which theme the editor asks egui for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

/// The editor's state and panels.
///
/// `ui_on` takes the root `Ui` eframe hands the app, and is also what a test
/// drives through `egui::Context::run_ui`; no window is involved.
pub struct EditorApp {
    document: Document,
    analysis: Analysis,
    path: Option<PathBuf>,
    texture: Option<TextureHandle>,
    texture_revision: u64,
    zoom: f32,
    pan: egui::Vec2,
    theme: ThemeChoice,
    fit_requested: bool,
}

impl EditorApp {
    pub fn new(document: Document, path: Option<PathBuf>) -> Result<Self, canvas::Error> {
        let analysis = Analysis::build(&document)?;
        Ok(Self {
            document,
            analysis,
            path,
            texture: None,
            texture_revision: u64::MAX,
            zoom: 8.0,
            pan: egui::Vec2::ZERO,
            theme: ThemeChoice::System,
            fit_requested: true,
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }

    pub fn theme(&self) -> ThemeChoice {
        self.theme
    }

    pub fn set_theme(&mut self, theme: ThemeChoice) {
        self.theme = theme;
    }

    /// Renders the panels and applies the theme.
    pub fn ui_on(&mut self, ui: &mut egui::Ui) {
        ui.ctx().set_theme(match self.theme {
            ThemeChoice::System => ThemePreference::System,
            ThemeChoice::Light => ThemePreference::Light,
            ThemeChoice::Dark => ThemePreference::Dark,
        });
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("report")
            .default_size(240.0)
            .show(ui, |ui| self.report(ui));
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("pietrs");
            ui.separator();
            ui.label(self.title());
            ui.separator();
            ui.label(format!(
                "{}x{} codels, codel size {}",
                self.document.width(),
                self.document.height(),
                self.document.codel_size()
            ));
            ui.separator();
            ui.label("Theme");
            ui.selectable_value(&mut self.theme, ThemeChoice::System, "System");
            ui.selectable_value(&mut self.theme, ThemeChoice::Light, "Light");
            ui.selectable_value(&mut self.theme, ThemeChoice::Dark, "Dark");
            ui.separator();
            if ui.button("Fit").clicked() {
                self.fit_requested = true;
            }
        });
    }

    fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("untitled")
            .to_owned()
    }

    fn report(&mut self, ui: &mut egui::Ui) {
        let report = self.analysis.report();
        ui.heading("Check");
        ui.label(format!("grid: {}x{}", report.width, report.height));
        ui.label(format!("blocks: {}", report.blocks));
        ui.label(format!("states: {}", report.states));
        ui.label(format!("edges: {}", report.edges));
        if report.can_terminate {
            ui.label("termination: possible");
        } else {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "termination: never (every state has a successor)",
            );
        }
        match report.start {
            Start::State(state) => {
                ui.label(format!(
                    "start: block {} dp {:?} cc {:?}",
                    state.block.0, state.dp, state.cc
                ));
            }
            Start::Terminate => {
                ui.label("start: terminates");
            }
        }
        if !report.unknown_colours.is_empty() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "{} non-standard colours are treated as white",
                    report.unknown_colours.len()
                ),
            );
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        self.refresh_texture(ui.ctx());
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());

        let background = if ui.visuals().dark_mode {
            egui::Color32::from_gray(24)
        } else {
            egui::Color32::from_gray(220)
        };
        painter.rect_filled(response.rect, CornerRadius::ZERO, background);

        if self.fit_requested {
            let available = response.rect.size();
            let zoom_x = (available.x - 24.0) / self.document.width() as f32;
            let zoom_y = (available.y - 24.0) / self.document.height() as f32;
            self.zoom = zoom_x.min(zoom_y).clamp(0.5, 64.0);
            self.pan = egui::Vec2::ZERO;
            self.fit_requested = false;
        }

        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if response.hovered() && scroll != 0.0 {
            self.zoom = (self.zoom * (scroll * 0.001).exp()).clamp(0.5, 64.0);
        }
        if response.dragged() {
            self.pan += response.drag_delta();
        }

        let size = egui::vec2(
            self.document.width() as f32 * self.zoom,
            self.document.height() as f32 * self.zoom,
        );
        let center = response.rect.center() + self.pan;
        let image_rect = egui::Rect::from_center_size(center, size);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    fn refresh_texture(&mut self, ctx: &egui::Context) {
        if self.texture.is_some() && self.texture_revision == self.document.revision() {
            return;
        }
        let size = [
            self.document.width() as usize,
            self.document.height() as usize,
        ];
        let pixels: Vec<egui::Color32> = self
            .document
            .codels()
            .iter()
            .map(|color| {
                let [red, green, blue] = color.rgb();
                egui::Color32::from_rgb(red, green, blue)
            })
            .collect();
        self.texture = Some(ctx.load_texture(
            "codels",
            egui::ColorImage::new(size, pixels),
            TextureOptions::NEAREST,
        ));
        self.texture_revision = self.document.revision();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Color, Hue, Lightness, Point};

    fn document() -> Document {
        let grid = CodelGrid::from_colors(
            2,
            1,
            vec![
                Color::chromatic(Hue::Red, Lightness::Normal),
                Color::chromatic(Hue::Green, Lightness::Normal),
            ],
        )
        .unwrap();
        Document::from_grid(&grid)
    }

    #[test]
    fn a_frame_renders_without_a_window() {
        let mut app = EditorApp::new(document(), None).unwrap();
        let ctx = egui::Context::default();

        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.ui_on(ui));
        output.textures_delta.clear();

        assert!(!output.shapes.is_empty());
        assert_eq!(app.analysis().report().blocks, 2);
        assert!(app.document().color(Point::new(0, 0)).is_some());
    }

    #[test]
    fn the_theme_choice_reaches_egui() {
        let mut app = EditorApp::new(document(), None).unwrap();
        app.set_theme(ThemeChoice::Light);
        assert_eq!(app.theme(), ThemeChoice::Light);

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.ui_on(ui));
        output.textures_delta.clear();
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }
}
