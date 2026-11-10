//! The egui view, with no window types.

use std::path::PathBuf;

use canvas::{BlockId, Point};
use editor::{Analysis, Breakpoints, Budget, Document, Session, Stop};
use egui::{CornerRadius, TextureHandle, TextureOptions, ThemePreference};
use interp::Options;
use tessera::{Exit, Start, State};

/// Stack rows shown before the rest is collapsed.
const MAX_STACK_ROWS: usize = 16;
/// Output characters shown; the count is always complete.
const MAX_OUTPUT_CHARS: usize = 2_000;
/// Trail points drawn; older points are still recorded for the session.
const MAX_TRAIL_POINTS: usize = 512;

/// Which theme the editor asks egui for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

enum DebugAction {
    Step,
    ToggleRun,
    Reset,
    ToggleBreak(BlockId),
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
    session: Option<Session>,
    breakpoints: Breakpoints,
    input: String,
    running: bool,
    selected: Option<BlockId>,
    overlay: Option<TextureHandle>,
    overlay_state: Option<State>,
    overlay_revision: u64,
    last_error: Option<String>,
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
            session: None,
            breakpoints: Breakpoints::default(),
            input: String::new(),
            running: false,
            selected: None,
            overlay: None,
            overlay_state: None,
            overlay_revision: 0,
            last_error: None,
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

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn selected(&self) -> Option<BlockId> {
        self.selected
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn set_input(&mut self, input: impl Into<String>) {
        self.input = input.into();
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    pub fn breakpoints(&self) -> &Breakpoints {
        &self.breakpoints
    }

    pub fn toggle_breakpoint(&mut self, block: BlockId) {
        self.breakpoints.toggle_block(block);
    }

    /// One transition, and pause.
    pub fn debug_step(&mut self) -> Result<(), interp::Error> {
        self.running = false;
        if !self.ensure_session() {
            return Ok(());
        }
        let result = match &mut self.session {
            Some(session) => session.step(),
            None => return Ok(()),
        };
        match result {
            Ok(_) => {
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Runs until a breakpoint, a final stop, or the frame budget.
    pub fn debug_run(&mut self, ctx: &egui::Context) -> Option<Stop> {
        if !self.running {
            return None;
        }
        if !self.ensure_session() {
            self.running = false;
            return None;
        }
        let result = match &mut self.session {
            Some(session) => session.run_until(&self.breakpoints, Budget::default()),
            None => {
                self.running = false;
                return None;
            }
        };
        match result {
            Ok(Stop::Budget) => {
                ctx.request_repaint();
                None
            }
            Ok(stop) => {
                self.running = false;
                Some(stop)
            }
            Err(error) => {
                self.running = false;
                self.last_error = Some(error.to_string());
                None
            }
        }
    }

    pub fn debug_reset(&mut self) {
        self.session = None;
        self.running = false;
        self.overlay = None;
        self.overlay_state = None;
        self.last_error = None;
    }

    /// Renders the panels and applies the theme.
    pub fn ui_on(&mut self, ui: &mut egui::Ui) {
        ui.ctx().set_theme(match self.theme {
            ThemeChoice::System => ThemePreference::System,
            ThemeChoice::Light => ThemePreference::Light,
            ThemeChoice::Dark => ThemePreference::Dark,
        });
        if self.running {
            self.debug_run(ui.ctx());
        }

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("report")
            .default_size(280.0)
            .show(ui, |ui| self.right_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));

        if self.running {
            ui.ctx().request_repaint();
        }
    }

    fn ensure_session(&mut self) -> bool {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.is_valid_for(&self.document))
        {
            return true;
        }
        if let Err(error) = self.analysis.refresh(&self.document) {
            self.last_error = Some(error.to_string());
            return false;
        }
        self.session = Some(Session::new(
            &self.analysis,
            self.input.clone(),
            Options::default(),
        ));
        self.overlay = None;
        self.overlay_state = None;
        true
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let mut step = false;
        let mut toggle_run = false;
        let mut reset = false;
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
            if ui.button("Step").clicked() {
                step = true;
            }
            if ui
                .button(if self.running { "Pause" } else { "Run" })
                .clicked()
            {
                toggle_run = true;
            }
            if ui.button("Reset").clicked() {
                reset = true;
            }
            if let Some(session) = &self.session {
                ui.label(format!("step {}", session.steps()));
            }
            ui.separator();
            if ui.button("Fit").clicked() {
                self.fit_requested = true;
            }
        });

        if ui.input(|input| input.key_pressed(egui::Key::F10)) {
            step = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::F5)) {
            toggle_run = true;
        }

        if step && let Err(error) = self.debug_step() {
            self.last_error = Some(error.to_string());
        }
        if toggle_run {
            self.running = !self.running;
        }
        if reset {
            self.debug_reset();
        }
        if let Some(error) = &self.last_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("untitled")
            .to_owned()
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.report(ui);
            ui.separator();
            self.debugger(ui);
        });
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

    fn debugger(&mut self, ui: &mut egui::Ui) {
        ui.heading("Debugger");

        ui.horizontal(|ui| {
            ui.label("Input");
            ui.add(egui::TextEdit::singleline(&mut self.input).desired_width(150.0));
        });
        if ui.button("Apply input and reset").clicked() {
            self.debug_reset();
        }

        let mut action: Option<DebugAction> = None;

        if let Some(session) = &self.session {
            ui.label(format!("steps: {}", session.steps()));
            match session.stop() {
                Some(Stop::Terminated) => {
                    ui.label("terminated");
                }
                Some(Stop::StepLimit) => {
                    ui.colored_label(ui.visuals().warn_fg_color, "step limit reached");
                }
                Some(Stop::StackOverflow) => {
                    ui.colored_label(ui.visuals().error_fg_color, "stack overflow");
                }
                Some(Stop::Breakpoint) => {
                    ui.colored_label(ui.visuals().selection.bg_fill, "paused at a breakpoint");
                }
                Some(Stop::Budget) => {
                    ui.label("paused");
                }
                None => {
                    ui.label(if self.running { "running" } else { "ready" });
                }
            }

            match session.state() {
                Some(state) => {
                    let block = session.blocks().block(state.block);
                    ui.label(format!(
                        "block {} (size {})",
                        state.block.0,
                        block.map_or(0, |block| block.size())
                    ));
                    if let Some(block) = block {
                        ui.label(format!("colour: {:?}", block.color()));
                    }
                    ui.label(format!("dp {:?}  cc {:?}", state.dp, state.cc));
                    match tessera::exit(session.blocks(), state.block, state.dp, state.cc) {
                        Exit::Move {
                            command, target, ..
                        } => {
                            let name = command.map_or_else(
                                || "none".to_owned(),
                                |command| format!("{command:?}"),
                            );
                            ui.label(format!("next: {name} -> block {}", target.0));
                        }
                        Exit::Terminate => {
                            ui.label("next: terminate");
                        }
                    }

                    ui.horizontal(|ui| {
                        if ui.button("Step").clicked() {
                            action = Some(DebugAction::Step);
                        }
                        if ui
                            .button(if self.running { "Pause" } else { "Run" })
                            .clicked()
                        {
                            action = Some(DebugAction::ToggleRun);
                        }
                        if ui.button("Reset").clicked() {
                            action = Some(DebugAction::Reset);
                        }
                    });

                    let active = self.breakpoints.blocks.contains(&state.block);
                    let label = if active {
                        "Clear breakpoint here"
                    } else {
                        "Break here"
                    };
                    if ui.button(label).clicked() {
                        action = Some(DebugAction::ToggleBreak(state.block));
                    }
                }
                None => {
                    ui.label("state: finished");
                    if ui.button("Reset").clicked() {
                        action = Some(DebugAction::Reset);
                    }
                }
            }

            ui.separator();
            ui.label("Stack");
            let values = session.stack().as_slice();
            let depth = values.len();
            if depth == 0 {
                ui.label("empty");
            } else {
                for (offset, value) in values.iter().rev().take(MAX_STACK_ROWS).enumerate() {
                    ui.label(
                        egui::RichText::new(format!("[{}] {value}", depth - 1 - offset))
                            .monospace(),
                    );
                }
                if depth > MAX_STACK_ROWS {
                    ui.label(format!("... {} more", depth - MAX_STACK_ROWS));
                }
            }

            ui.separator();
            ui.label("Output");
            let output = session.output();
            ui.label(format!("{} characters", output.chars().count()));
            if !output.is_empty() {
                let mut tail: Vec<char> = output.chars().rev().take(MAX_OUTPUT_CHARS).collect();
                tail.reverse();
                let tail: String = tail.into_iter().collect();
                ui.label(egui::RichText::new(tail).monospace());
            }
            if session.trace_truncated() {
                ui.colored_label(ui.visuals().warn_fg_color, "trace history truncated");
            }
        } else {
            ui.label("not started");
            if ui.button("Start").clicked() {
                action = Some(DebugAction::Step);
            }
        }

        ui.separator();
        ui.label("Breakpoints");
        let blocks = self.breakpoints.sorted_blocks();
        if blocks.is_empty() {
            ui.label("none");
        }
        for block in blocks {
            ui.horizontal(|ui| {
                ui.label(format!("block {}", block.0));
                if ui.small_button("x").clicked() {
                    action = Some(DebugAction::ToggleBreak(block));
                }
            });
        }
        if let Some(block) = self.selected {
            let active = self.breakpoints.blocks.contains(&block);
            let label = if active {
                "Clear the breakpoint on the selected block"
            } else {
                "Break on the selected block"
            };
            if ui.button(label).clicked() {
                action = Some(DebugAction::ToggleBreak(block));
            }
        }

        match action {
            Some(DebugAction::Step) => {
                if let Err(error) = self.debug_step() {
                    self.last_error = Some(error.to_string());
                }
            }
            Some(DebugAction::ToggleRun) => {
                self.running = !self.running;
            }
            Some(DebugAction::Reset) => {
                self.debug_reset();
            }
            Some(DebugAction::ToggleBreak(block)) => {
                self.toggle_breakpoint(block);
            }
            None => {}
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        self.refresh_texture(ui.ctx());
        self.refresh_overlay(ui.ctx());

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
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
            && image_rect.contains(pointer)
        {
            let x = (pointer.x - image_rect.min.x) / self.zoom;
            let y = (pointer.y - image_rect.min.y) / self.zoom;
            if x >= 0.0
                && y >= 0.0
                && x < self.document.width() as f32
                && y < self.document.height() as f32
            {
                let point = Point::new(x as u32, y as u32);
                self.selected = self.analysis.blocks().block_id_at(point);
            }
        }

        if let Some(texture) = &self.texture {
            painter.image(texture.id(), image_rect, uv, egui::Color32::WHITE);
        }
        if let Some(overlay) = &self.overlay {
            painter.image(overlay.id(), image_rect, uv, egui::Color32::WHITE);
        }

        if let Some(block) = self
            .selected
            .and_then(|block| self.analysis.blocks().block(block))
        {
            let outline = block_rect(image_rect, block.codels(), self.zoom);
            let stroke = egui::Stroke::new(1.5, ui.visuals().selection.bg_fill);
            painter.line_segment([outline.left_top(), outline.right_top()], stroke);
            painter.line_segment([outline.right_top(), outline.right_bottom()], stroke);
            painter.line_segment([outline.right_bottom(), outline.left_bottom()], stroke);
            painter.line_segment([outline.left_bottom(), outline.left_top()], stroke);
        }

        if let Some(session) = &self.session {
            let trail = session.trail();
            let start = trail.len().saturating_sub(MAX_TRAIL_POINTS);
            let stroke = egui::Stroke::new(1.0, ui.visuals().selection.bg_fill);
            let mut previous: Option<egui::Pos2> = None;
            for point in &trail[start..] {
                let current = codel_center(image_rect, *point, self.zoom);
                if let Some(previous) = previous {
                    painter.line_segment([previous, current], stroke);
                }
                previous = Some(current);
            }
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

    fn refresh_overlay(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.session else {
            self.overlay = None;
            return;
        };
        let Some(state) = session.state() else {
            self.overlay = None;
            return;
        };
        if self.overlay.is_some()
            && self.overlay_state == Some(state)
            && self.overlay_revision == session.revision()
        {
            return;
        }
        let Some(block) = session.blocks().block(state.block) else {
            self.overlay = None;
            return;
        };
        let width = session.blocks().grid().width() as usize;
        let height = session.blocks().grid().height() as usize;
        let mut pixels = vec![egui::Color32::TRANSPARENT; width * height];
        for point in block.codels() {
            let index = (point.y as usize) * width + point.x as usize;
            if let Some(pixel) = pixels.get_mut(index) {
                *pixel = egui::Color32::from_rgba_unmultiplied(255, 220, 0, 110);
            }
        }
        self.overlay = Some(ctx.load_texture(
            "current-block",
            egui::ColorImage::new([width, height], pixels),
            TextureOptions::NEAREST,
        ));
        self.overlay_state = Some(state);
        self.overlay_revision = session.revision();
    }
}

fn codel_center(image_rect: egui::Rect, point: Point, zoom: f32) -> egui::Pos2 {
    image_rect.min + egui::vec2((point.x as f32 + 0.5) * zoom, (point.y as f32 + 0.5) * zoom)
}

fn block_rect(image_rect: egui::Rect, codels: &[Point], zoom: f32) -> egui::Rect {
    let mut min = (u32::MAX, u32::MAX);
    let mut max = (0, 0);
    for point in codels {
        min = (min.0.min(point.x), min.1.min(point.y));
        max = (max.0.max(point.x), max.1.max(point.y));
    }
    egui::Rect::from_min_max(
        image_rect.min + egui::vec2(min.0 as f32 * zoom, min.1 as f32 * zoom),
        image_rect.min + egui::vec2((max.0 + 1) as f32 * zoom, (max.1 + 1) as f32 * zoom),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Color, Hue, Lightness};

    fn small_document(rows: &[&str]) -> Document {
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, |row| row.len() as u32);
        let mut codels = Vec::with_capacity((width as usize) * (height as usize));
        for row in rows {
            for cell in row.chars() {
                codels.push(match cell {
                    '.' => Color::White,
                    '#' => Color::Black,
                    'r' => Color::chromatic(Hue::Red, Lightness::Normal),
                    'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        Document::from_grid(&CodelGrid::from_colors(width, height, codels).unwrap())
    }

    fn print_a_document() -> Document {
        let mut codels = vec![Color::chromatic(Hue::Red, Lightness::Normal); 65];
        codels.push(Color::chromatic(Hue::Red, Lightness::Dark));
        codels.push(Color::chromatic(Hue::Magenta, Lightness::Normal));
        Document::from_grid(&CodelGrid::from_colors(67, 1, codels).unwrap())
    }

    fn render(app: &mut EditorApp) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.ui_on(ui));
        output.textures_delta.clear();
        output
    }

    #[test]
    fn a_frame_renders_without_a_window() {
        let mut app = EditorApp::new(small_document(&["rg"]), None).unwrap();
        let output = render(&mut app);

        assert!(!output.shapes.is_empty());
        assert_eq!(app.analysis().report().blocks, 2);
        assert!(app.document().color(Point::new(0, 0)).is_some());
    }

    #[test]
    fn the_theme_choice_reaches_egui() {
        let mut app = EditorApp::new(small_document(&["rg"]), None).unwrap();
        app.set_theme(ThemeChoice::Light);
        assert_eq!(app.theme(), ThemeChoice::Light);

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.ui_on(ui));
        output.textures_delta.clear();
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn stepping_advances_the_session() {
        let mut app = EditorApp::new(print_a_document(), None).unwrap();
        app.debug_step().unwrap();
        app.debug_step().unwrap();

        let session = app.session().unwrap();
        assert_eq!(session.steps(), 2);
        assert_eq!(session.output(), "A");
        assert!(session.stack().is_empty());
    }

    #[test]
    fn running_a_terminating_program_stops() {
        let mut app = EditorApp::new(small_document(&["rg", "gg"]), None).unwrap();
        app.set_running(true);

        let ctx = egui::Context::default();
        assert_eq!(app.debug_run(&ctx), Some(Stop::Terminated));
        assert!(!app.running());
    }

    #[test]
    fn a_block_breakpoint_pauses_a_run() {
        let mut app = EditorApp::new(small_document(&["rg", "gg"]), None).unwrap();
        let green = app
            .analysis()
            .blocks()
            .block_id_at(Point::new(1, 0))
            .unwrap();
        app.toggle_breakpoint(green);
        app.set_running(true);

        let ctx = egui::Context::default();
        assert_eq!(app.debug_run(&ctx), Some(Stop::Breakpoint));
        assert!(!app.running());
        assert_eq!(app.session().unwrap().state().unwrap().block, green);
        assert_eq!(app.session().unwrap().steps(), 1);
    }

    #[test]
    fn reset_clears_the_session() {
        let mut app = EditorApp::new(print_a_document(), None).unwrap();
        app.debug_step().unwrap();
        assert!(app.session().is_some());

        app.debug_reset();
        assert!(app.session().is_none());
        assert!(!app.running());
    }

    #[test]
    fn a_frame_renders_with_a_session() {
        let mut app = EditorApp::new(small_document(&["rg", "gg"]), None).unwrap();
        app.debug_step().unwrap();
        let output = render(&mut app);
        assert!(!output.shapes.is_empty());
        assert!(app.session().unwrap().steps() >= 1);
    }
}
