//! The egui view, with no window types.

use std::path::{Path, PathBuf};

use canvas::{BlockId, CodelGrid, Color, Hue, Lightness, Point};
use editor::{Analysis, Breakpoints, Budget, Document, Session, Stop};
use egui::{CornerRadius, TextureHandle, TextureOptions, ThemePreference};
use interp::Options;
use tessera::{Exit, Start, State};

use crate::FileError;

#[cfg(feature = "codegen")]
use runtime::BufferIo;
#[cfg(feature = "codegen")]
use std::sync::Arc;
#[cfg(feature = "codegen")]
use std::sync::mpsc::{self, Receiver, TryRecvError};

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

impl ThemeChoice {
    /// The name persisted in `eframe`'s storage.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }
}

impl std::str::FromStr for ThemeChoice {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "system" => Ok(ThemeChoice::System),
            "light" => Ok(ThemeChoice::Light),
            "dark" => Ok(ThemeChoice::Dark),
            _ => Err(()),
        }
    }
}

enum DebugAction {
    Step,
    ToggleRun,
    Reset,
    ToggleBreak(BlockId),
    #[cfg(feature = "codegen")]
    Compare,
    #[cfg(feature = "codegen")]
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pencil,
    Rectangle,
    Fill,
    Eyedropper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    New,
    Open,
}

#[cfg(feature = "codegen")]
#[derive(Default)]
struct CompilerState {
    job: Option<Job>,
    message: Option<String>,
}

#[cfg(feature = "codegen")]
struct Job {
    receiver: Receiver<Result<String, String>>,
    kind: JobKind,
}

#[cfg(feature = "codegen")]
#[derive(Clone, Copy)]
enum JobKind {
    Compare,
    Build,
}

#[cfg(feature = "codegen")]
impl JobKind {
    fn label(self) -> &'static str {
        match self {
            JobKind::Compare => "JIT comparison",
            JobKind::Build => "build",
        }
    }
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
    tool: Tool,
    colour: Color,
    stroke_active: bool,
    stroke_changed: bool,
    last_paint: Option<Point>,
    rect_start: Option<Point>,
    rect_end: Option<Point>,
    dirty: bool,
    window_title: String,
    pending: Option<Pending>,
    new_open: bool,
    new_width: u32,
    new_height: u32,
    resize_width: u32,
    resize_height: u32,
    #[cfg(feature = "codegen")]
    compiler: CompilerState,
}

impl EditorApp {
    pub fn new(document: Document, path: Option<PathBuf>) -> Result<Self, canvas::Error> {
        let analysis = Analysis::build(&document)?;
        let dirty = path.is_none();
        let resize_width = document.width();
        let resize_height = document.height();
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
            tool: Tool::Pencil,
            colour: Color::chromatic(Hue::Red, Lightness::Normal),
            stroke_active: false,
            stroke_changed: false,
            last_paint: None,
            rect_start: None,
            rect_end: None,
            dirty,
            window_title: String::new(),
            pending: None,
            new_open: false,
            new_width: 32,
            new_height: 32,
            resize_width,
            resize_height,
            #[cfg(feature = "codegen")]
            compiler: CompilerState::default(),
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

    pub fn colour(&self) -> Color {
        self.colour
    }

    pub fn set_colour(&mut self, colour: Color) {
        self.colour = colour;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Starts a freehand stroke.
    pub fn begin_edit(&mut self) {
        self.document.begin_stroke();
        self.stroke_active = true;
        self.stroke_changed = false;
        self.last_paint = None;
    }

    /// Paints from the previous point to this one, so a fast drag leaves no
    /// gaps between sampled codels.
    pub fn paint_to(&mut self, point: Point) {
        if !self.stroke_active {
            self.begin_edit();
        }
        let from = self.last_paint.unwrap_or(point);
        for codel in line(from, point) {
            self.stroke_changed |= self.document.set(codel, self.colour);
        }
        self.last_paint = Some(point);
    }

    /// Ends a freehand stroke, recording it as one undo entry.
    pub fn end_edit(&mut self) {
        if !self.stroke_active {
            return;
        }
        self.stroke_active = false;
        self.last_paint = None;
        self.document.end_stroke();
        if std::mem::take(&mut self.stroke_changed) {
            self.after_edit();
        }
    }

    pub fn fill_at(&mut self, point: Point) {
        if self.document.fill(point, self.colour) {
            self.after_edit();
        }
    }

    pub fn pick_at(&mut self, point: Point) {
        if let Some(colour) = self.document.color(point) {
            self.colour = colour;
        }
    }

    /// Paints the inclusive rectangle in one undo entry.
    pub fn apply_rect(&mut self, from: Point, to: Point) {
        self.document.begin_stroke();
        let mut changed = false;
        for y in from.y.min(to.y)..=from.y.max(to.y) {
            for x in from.x.min(to.x)..=from.x.max(to.x) {
                changed |= self.document.set(Point::new(x, y), self.colour);
            }
        }
        self.document.end_stroke();
        if changed {
            self.after_edit();
        }
    }

    pub fn undo(&mut self) -> bool {
        if self.stroke_active {
            self.stroke_active = false;
            self.stroke_changed = false;
            self.last_paint = None;
            self.document.end_stroke();
        }
        if !self.document.undo() {
            return false;
        }
        self.after_edit();
        true
    }

    pub fn redo(&mut self) -> bool {
        if !self.document.redo() {
            return false;
        }
        self.after_edit();
        true
    }

    /// Replaces the document with a blank one at `width` x `height`.
    pub fn new_document(&mut self, width: u32, height: u32) {
        let document = match Document::new(width, height) {
            Ok(document) => document,
            Err(error) => {
                self.last_error = Some(error.to_string());
                return;
            }
        };
        let analysis = match Analysis::build(&document) {
            Ok(analysis) => analysis,
            Err(error) => {
                self.last_error = Some(error.to_string());
                return;
            }
        };
        self.document = document;
        self.analysis = analysis;
        self.path = None;
        self.dirty = true;
        self.reset_view();
    }

    pub fn open_bytes(&mut self, bytes: &[u8], path: Option<PathBuf>) -> Result<(), canvas::Error> {
        let grid = CodelGrid::from_image(bytes)?;
        self.adopt(grid, path)
    }

    pub fn open_bytes_with_codel_size(
        &mut self,
        bytes: &[u8],
        codel_size: u32,
        path: Option<PathBuf>,
    ) -> Result<(), canvas::Error> {
        let grid = CodelGrid::from_image_with_codel_size(bytes, codel_size)?;
        self.adopt(grid, path)
    }

    pub fn open_file(&mut self, path: &Path) -> Result<(), FileError> {
        self.open_file_with_codel_size(path, None)
    }

    pub fn open_file_with_codel_size(
        &mut self,
        path: &Path,
        codel_size: Option<u32>,
    ) -> Result<(), FileError> {
        let bytes = std::fs::read(path).map_err(|source| FileError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let owned = path.to_path_buf();
        match codel_size {
            Some(size) => self.open_bytes_with_codel_size(&bytes, size, Some(owned)),
            None => self.open_bytes(&bytes, Some(owned)),
        }?;
        Ok(())
    }

    pub fn export_png(&self) -> Result<Vec<u8>, canvas::Error> {
        self.document.to_grid()?.encode_png()
    }

    /// Writes the document as a PNG and makes `path` the current file.
    pub fn save_to(&mut self, path: &Path) -> Result<(), FileError> {
        let bytes = self.export_png()?;
        std::fs::write(path, bytes).map_err(|source| FileError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    fn adopt(&mut self, grid: CodelGrid, path: Option<PathBuf>) -> Result<(), canvas::Error> {
        let document = Document::from_grid(&grid);
        let analysis = Analysis::build(&document)?;
        self.document = document;
        self.analysis = analysis;
        self.path = path;
        self.dirty = false;
        self.reset_view();
        Ok(())
    }

    fn reset_view(&mut self) {
        self.texture = None;
        self.texture_revision = u64::MAX;
        self.overlay = None;
        self.overlay_state = None;
        self.session = None;
        self.running = false;
        self.selected = None;
        self.pan = egui::Vec2::ZERO;
        self.fit_requested = true;
        self.last_error = None;
        self.rect_start = None;
        self.rect_end = None;
        self.stroke_active = false;
        self.stroke_changed = false;
        self.last_paint = None;
        self.resize_width = self.document.width();
        self.resize_height = self.document.height();
    }

    /// Resizes the document, anchored at the upper-left. New codels are white.
    pub fn resize_document(&mut self, width: u32, height: u32) {
        match self.document.resize(width, height) {
            Ok(true) => self.after_edit(),
            Ok(false) => {}
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    fn request_new(&mut self) {
        if self.dirty {
            self.pending = Some(Pending::New);
        } else {
            self.new_open = true;
        }
    }

    fn request_open(&mut self) {
        if self.dirty {
            self.pending = Some(Pending::Open);
        } else {
            self.open_dialog();
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(path) => self.save_or_report(&path),
            None => self.save_dialog(),
        }
    }

    fn save_as(&mut self) {
        self.save_dialog();
    }

    fn save_or_report(&mut self, path: &Path) {
        match self.save_to(path) {
            Ok(()) => self.last_error = None,
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    #[cfg(feature = "native")]
    fn open_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Piet image", &["png", "gif"])
            .pick_file()
        else {
            return;
        };
        match self.open_file(&path) {
            Ok(()) => self.last_error = None,
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    #[cfg(not(feature = "native"))]
    fn open_dialog(&mut self) {
        self.last_error = Some("file dialogs need the native feature".to_owned());
    }

    #[cfg(feature = "native")]
    fn save_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter("PNG", &["png"]);
        if let Some(name) = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
        {
            dialog = dialog.set_file_name(name);
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("png");
        }
        self.save_or_report(&path);
    }

    #[cfg(not(feature = "native"))]
    fn save_dialog(&mut self) {
        self.last_error = Some("file dialogs need the native feature".to_owned());
    }

    fn confirm_discard(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending else {
            return;
        };
        let mut discard = false;
        let mut cancel = false;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("The current document has unsaved changes.");
                ui.horizontal(|ui| {
                    if ui.button("Discard").clicked() {
                        discard = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if discard {
            self.pending = None;
            match pending {
                Pending::New => self.new_open = true,
                Pending::Open => self.open_dialog(),
            }
        } else if cancel {
            self.pending = None;
        }
    }

    fn show_new_dialog(&mut self, ctx: &egui::Context) {
        if !self.new_open {
            return;
        }
        let mut create = false;
        let mut cancel = false;
        egui::Window::new("New document")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut self.new_width).range(1..=1024));
                });
                ui.horizontal(|ui| {
                    ui.label("Height");
                    ui.add(egui::DragValue::new(&mut self.new_height).range(1..=1024));
                });
                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() {
                        create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if create {
            self.new_open = false;
            self.new_document(self.new_width, self.new_height);
        } else if cancel {
            self.new_open = false;
        }
    }

    #[cfg(feature = "codegen")]
    fn poll_job(&mut self, ctx: &egui::Context) {
        let outcome = match &self.compiler.job {
            Some(job) => match job.receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint();
                    return;
                }
                Err(TryRecvError::Disconnected) => Err("the compiler worker exited".to_owned()),
            },
            None => return,
        };
        self.compiler.job = None;
        match outcome {
            Ok(message) => {
                self.last_error = None;
                self.compiler.message = Some(message);
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    #[cfg(feature = "codegen")]
    fn compare_with_jit(&mut self, ctx: &egui::Context) {
        let blocks = Arc::clone(self.analysis.blocks());
        let input = self.input.clone();
        let context = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(compare_jit(&blocks, &input));
            context.request_repaint();
        });
        self.compiler = CompilerState {
            job: Some(Job {
                receiver,
                kind: JobKind::Compare,
            }),
            message: None,
        };
    }

    #[cfg(feature = "codegen")]
    fn spawn_build(&mut self, ctx: &egui::Context, path: PathBuf) {
        let blocks = Arc::clone(self.analysis.blocks());
        let context = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let output = path.clone();
        std::thread::spawn(move || {
            let result = aot::build(
                &blocks,
                &output,
                None,
                runtime::Division::Floored,
                aot::HOST_TARGET,
            )
            .map(|()| format!("built {}", output.display()))
            .map_err(|error| error.to_string());
            let _ = sender.send(result);
            context.request_repaint();
        });
        self.compiler = CompilerState {
            job: Some(Job {
                receiver,
                kind: JobKind::Build,
            }),
            message: None,
        };
    }

    #[cfg(all(feature = "native", feature = "codegen"))]
    fn build_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new().set_file_name("program").save_file() else {
            return;
        };
        let path = if path.extension().is_none() {
            let mut name = path.into_os_string();
            name.push(std::env::consts::EXE_SUFFIX);
            PathBuf::from(name)
        } else {
            path
        };
        self.spawn_build(ctx, path);
    }

    #[cfg(all(not(feature = "native"), feature = "codegen"))]
    fn build_dialog(&mut self, _ctx: &egui::Context) {
        self.last_error = Some("building an executable needs the native feature".to_owned());
    }

    fn after_edit(&mut self) {
        self.dirty = true;
        self.debug_reset();
        self.rect_start = None;
        self.rect_end = None;
        self.resize_width = self.document.width();
        self.resize_height = self.document.height();
        if let Err(error) = self.analysis.refresh(&self.document) {
            self.last_error = Some(error.to_string());
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!(
            "pietrs - {}{}",
            self.title(),
            if self.dirty { " *" } else { "" }
        );
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    /// Renders the panels and applies the theme.
    pub fn ui_on(&mut self, ui: &mut egui::Ui) {
        ui.ctx().set_theme(match self.theme {
            ThemeChoice::System => ThemePreference::System,
            ThemeChoice::Light => ThemePreference::Light,
            ThemeChoice::Dark => ThemePreference::Dark,
        });
        ui.ctx()
            .all_styles_mut(|style| style.interaction.selectable_labels = false);
        self.update_title(ui.ctx());
        #[cfg(feature = "codegen")]
        self.poll_job(ui.ctx());

        if !ui.ctx().text_edit_focused() {
            let (undo, redo) = ui.input(|input| {
                let command = input.modifiers.command;
                (
                    command && !input.modifiers.shift && input.key_pressed(egui::Key::Z),
                    command
                        && (input.key_pressed(egui::Key::Y)
                            || (input.modifiers.shift && input.key_pressed(egui::Key::Z))),
                )
            });
            if undo {
                self.undo();
            }
            if redo {
                self.redo();
            }
        }

        if self.running {
            self.debug_run(ui.ctx());
        }

        egui::Panel::left("tools")
            .default_size(190.0)
            .show(ui, |ui| self.tools(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("report")
            .default_size(280.0)
            .show(ui, |ui| self.right_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));

        if self.running {
            ui.ctx().request_repaint();
        }
        self.confirm_discard(ui.ctx());
        self.show_new_dialog(ui.ctx());
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
        let mut new = false;
        let mut open = false;
        let mut save = false;
        let mut save_as = false;
        ui.horizontal(|ui| {
            ui.heading("pietrs");
            ui.separator();
            ui.label(self.title());
            ui.separator();
            if ui.button("New").clicked() {
                new = true;
            }
            if ui.button("Open").clicked() {
                open = true;
            }
            if ui.button("Save").clicked() {
                save = true;
            }
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
        if !ui.ctx().text_edit_focused() {
            let (key_new, key_open, key_save, key_save_as) = ui.input(|input| {
                let command = input.modifiers.command;
                (
                    command && input.key_pressed(egui::Key::N),
                    command && input.key_pressed(egui::Key::O),
                    command && !input.modifiers.shift && input.key_pressed(egui::Key::S),
                    command && input.modifiers.shift && input.key_pressed(egui::Key::S),
                )
            });
            new |= key_new;
            open |= key_open;
            save |= key_save;
            save_as |= key_save_as;
        }

        if new {
            self.request_new();
        }
        if open {
            self.request_open();
        }
        if save {
            self.save();
        }
        if save_as {
            self.save_as();
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

    fn tools(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.tool, Tool::Pencil, "Pencil");
            ui.selectable_value(&mut self.tool, Tool::Rectangle, "Rect");
            ui.selectable_value(&mut self.tool, Tool::Fill, "Fill");
            ui.selectable_value(&mut self.tool, Tool::Eyedropper, "Pick");
        });

        ui.separator();
        ui.label("Colour");
        self.palette(ui);

        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.document.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(self.document.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.redo();
            }
        });

        ui.separator();
        ui.label(format!(
            "{}x{} codels, codel size {}",
            self.document.width(),
            self.document.height(),
            self.document.codel_size()
        ));
        ui.horizontal(|ui| {
            ui.label("W");
            ui.add(egui::DragValue::new(&mut self.resize_width).range(1..=1024));
            ui.label("H");
            ui.add(egui::DragValue::new(&mut self.resize_height).range(1..=1024));
        });
        if ui.button("Resize").clicked() {
            self.resize_document(self.resize_width, self.resize_height);
        }
        if let Some(block) = self
            .selected
            .and_then(|block| self.analysis.blocks().block(block))
        {
            ui.label(format!(
                "selected: block {} ({:?})",
                block.id().0,
                block.color()
            ));
        }
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        let size = egui::vec2(24.0, 24.0);
        for row in 0..6 {
            ui.horizontal(|ui| {
                for column in 0..3 {
                    let index = row * 3 + column;
                    if let Some(colour) = Color::ALL.get(index).copied() {
                        self.swatch(ui, colour, size);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            self.swatch(ui, Color::White, size);
            self.swatch(ui, Color::Black, size);
        });
    }

    fn swatch(&mut self, ui: &mut egui::Ui, colour: Color, size: egui::Vec2) {
        let [red, green, blue] = colour.rgb();
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::ZERO,
            egui::Color32::from_rgb(red, green, blue),
        );
        if colour == self.colour {
            outline(
                painter,
                rect,
                egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
            );
        }
        if response.clicked() {
            self.colour = colour;
        }
        response.on_hover_text(format!("{colour:?}"));
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

        #[cfg(feature = "codegen")]
        {
            ui.separator();
            ui.label("Compiler");
            let busy = self.compiler.job.is_some();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Compare with JIT"))
                    .clicked()
                {
                    action = Some(DebugAction::Compare);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Build executable"))
                    .clicked()
                {
                    action = Some(DebugAction::Build);
                }
            });
            if let Some(job) = &self.compiler.job {
                ui.label(format!("{} running", job.kind.label()));
            }
            if let Some(message) = &self.compiler.message {
                ui.label(message);
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
            #[cfg(feature = "codegen")]
            Some(DebugAction::Compare) => self.compare_with_jit(ui.ctx()),
            #[cfg(feature = "codegen")]
            Some(DebugAction::Build) => self.build_dialog(ui.ctx()),
            None => {}
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
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

        let space = ui.input(|input| input.key_down(egui::Key::Space));
        let panning = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || (space && response.dragged_by(egui::PointerButton::Primary));
        if panning {
            self.pan += response.drag_delta();
        }

        let size = egui::vec2(
            self.document.width() as f32 * self.zoom,
            self.document.height() as f32 * self.zoom,
        );
        let center = response.rect.center() + self.pan;
        let image_rect = egui::Rect::from_center_size(center, size);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

        let pointer = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos());
        let hovered = pointer.and_then(|pointer| {
            codel_at(
                image_rect,
                pointer,
                self.zoom,
                self.document.width(),
                self.document.height(),
            )
        });

        if let Some(point) = hovered
            && (response.clicked() || response.drag_started_by(egui::PointerButton::Primary))
        {
            self.selected = self.analysis.blocks().block_id_at(point);
        }
        if !panning {
            self.handle_tool(&response, hovered);
        }

        // An edit changed the document this frame; rebuild before painting.
        self.refresh_texture(ui.ctx());
        self.refresh_overlay(ui.ctx());

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
            outline(
                &painter,
                block_rect(image_rect, block.codels(), self.zoom),
                egui::Stroke::new(1.5, ui.visuals().selection.bg_fill),
            );
        }

        if self.tool == Tool::Rectangle
            && let (Some(from), Some(to)) = (self.rect_start, self.rect_end)
        {
            outline(
                &painter,
                codel_rect(image_rect, from, to, self.zoom),
                egui::Stroke::new(1.5, egui::Color32::WHITE),
            );
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

    fn handle_tool(&mut self, response: &egui::Response, hovered: Option<Point>) {
        let primary = egui::PointerButton::Primary;
        match self.tool {
            Tool::Pencil => {
                if response.drag_started_by(primary) {
                    self.begin_edit();
                    if let Some(point) = hovered {
                        self.paint_to(point);
                    }
                } else if response.dragged_by(primary)
                    && self.stroke_active
                    && let Some(point) = hovered
                {
                    self.paint_to(point);
                }
                if response.drag_stopped() && self.stroke_active {
                    self.end_edit();
                }
                if response.clicked()
                    && let Some(point) = hovered
                {
                    self.begin_edit();
                    self.paint_to(point);
                    self.end_edit();
                }
            }
            Tool::Rectangle => {
                if response.drag_started_by(primary) {
                    self.rect_start = hovered;
                    self.rect_end = hovered;
                } else if response.dragged_by(primary)
                    && self.rect_start.is_some()
                    && let Some(point) = hovered
                {
                    self.rect_end = Some(point);
                }
                if response.drag_stopped() {
                    let start = self.rect_start.take();
                    let end = self.rect_end.take();
                    if let (Some(from), Some(to)) = (start, end) {
                        self.apply_rect(from, to);
                    }
                }
                if response.clicked()
                    && let Some(point) = hovered
                {
                    self.apply_rect(point, point);
                }
            }
            Tool::Fill => {
                if response.clicked()
                    && let Some(point) = hovered
                {
                    self.fill_at(point);
                }
            }
            Tool::Eyedropper => {
                if response.clicked()
                    && let Some(point) = hovered
                {
                    self.pick_at(point);
                }
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

#[cfg(feature = "codegen")]
const COMPARE_LIMIT: u64 = 1_000_000;

/// Runs the interpreter and the JIT with the same input and compares them.
#[cfg(feature = "codegen")]
fn compare_jit(blocks: &canvas::Blocks, input: &str) -> Result<String, String> {
    let mut interp_io = BufferIo::new(input);
    let interp_options = interp::Options {
        max_steps: Some(COMPARE_LIMIT),
        ..interp::Options::default()
    };
    let interp_run =
        interp::run(blocks, &mut interp_io, &interp_options).map_err(|error| error.to_string())?;

    let mut jit_io = BufferIo::new(input);
    let jit_options = codegen::Options {
        max_steps: Some(COMPARE_LIMIT),
        ..codegen::Options::default()
    };
    let jit_run =
        codegen::run(blocks, &mut jit_io, &jit_options).map_err(|error| error.to_string())?;

    match (interp_run.reason, jit_run.reason) {
        (interp::ExitReason::Terminated, codegen::ExitReason::Terminated) => {
            if interp_io.output() == jit_io.output() {
                Ok(format!(
                    "JIT agrees: {} steps, {} output bytes",
                    interp_run.steps,
                    interp_io.output().len()
                ))
            } else {
                Err(format!(
                    "JIT differs: interpreter output {:?}, JIT output {:?}",
                    interp_io.output(),
                    jit_io.output()
                ))
            }
        }
        (interp::ExitReason::StepLimit, _) | (_, codegen::ExitReason::StepLimit) => Ok(format!(
            "JIT matched the interpreter up to the {COMPARE_LIMIT}-step limit"
        )),
        (interp_reason, jit_reason) => Err(format!(
            "JIT differs: interpreter {interp_reason:?}, JIT {jit_reason:?}"
        )),
    }
}

fn codel_at(
    image_rect: egui::Rect,
    pointer: egui::Pos2,
    zoom: f32,
    width: u32,
    height: u32,
) -> Option<Point> {
    if !image_rect.contains(pointer) {
        return None;
    }
    let x = (pointer.x - image_rect.min.x) / zoom;
    let y = (pointer.y - image_rect.min.y) / zoom;
    if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
        return None;
    }
    Some(Point::new(x as u32, y as u32))
}

fn codel_rect(image_rect: egui::Rect, from: Point, to: Point, zoom: f32) -> egui::Rect {
    let min_x = from.x.min(to.x);
    let min_y = from.y.min(to.y);
    let max_x = from.x.max(to.x);
    let max_y = from.y.max(to.y);
    egui::Rect::from_min_max(
        image_rect.min + egui::vec2(min_x as f32 * zoom, min_y as f32 * zoom),
        image_rect.min + egui::vec2((max_x + 1) as f32 * zoom, (max_y + 1) as f32 * zoom),
    )
}

fn outline(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    painter.line_segment([rect.left_top(), rect.right_top()], stroke);
    painter.line_segment([rect.right_top(), rect.right_bottom()], stroke);
    painter.line_segment([rect.right_bottom(), rect.left_bottom()], stroke);
    painter.line_segment([rect.left_bottom(), rect.left_top()], stroke);
}

/// Every codel on the inclusive line, so a fast drag does not leave gaps.
fn line(from: Point, to: Point) -> Vec<Point> {
    let mut points = Vec::new();
    let mut x = from.x as i64;
    let mut y = from.y as i64;
    let end_x = to.x as i64;
    let end_y = to.y as i64;
    let dx = (end_x - x).abs();
    let dy = -(end_y - y).abs();
    let step_x = if x < end_x { 1 } else { -1 };
    let step_y = if y < end_y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        points.push(Point::new(x as u32, y as u32));
        if x == end_x && y == end_y {
            break;
        }
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            x += step_x;
        }
        if twice <= dx {
            error += dx;
            y += step_y;
        }
    }
    points
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

    fn red() -> Color {
        Color::chromatic(Hue::Red, Lightness::Normal)
    }

    fn green() -> Color {
        Color::chromatic(Hue::Green, Lightness::Normal)
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
    fn labels_are_not_selectable() {
        let mut app = EditorApp::new(small_document(&["rg"]), None).unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.ui_on(ui));
        output.textures_delta.clear();
        assert!(!ctx.global_style().interaction.selectable_labels);
    }

    #[test]
    fn theme_names_round_trip() {
        for theme in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
            assert_eq!(theme.as_str().parse::<ThemeChoice>().ok(), Some(theme));
        }
        assert!("nope".parse::<ThemeChoice>().is_err());
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

    #[test]
    fn the_pencil_paints_a_line_and_undoes_as_one_stroke() {
        let mut app =
            EditorApp::new(small_document(&["rrr"]), Some(PathBuf::from("x.png"))).unwrap();
        assert!(!app.is_dirty());
        app.set_colour(Color::Black);

        app.begin_edit();
        app.paint_to(Point::new(0, 0));
        app.paint_to(Point::new(2, 0));
        app.end_edit();

        assert_eq!(app.document().color(Point::new(1, 0)), Some(Color::Black));
        assert!(app.is_dirty());
        assert!(app.undo());
        assert_eq!(app.document().color(Point::new(1, 0)), Some(red()));
        assert!(!app.document().can_undo());
    }

    #[test]
    fn the_rectangle_fills_its_span_as_one_edit() {
        let mut app = EditorApp::new(small_document(&["rrr", "rrr"]), None).unwrap();
        app.set_colour(Color::Black);
        app.apply_rect(Point::new(0, 0), Point::new(1, 1));

        assert_eq!(app.document().color(Point::new(1, 1)), Some(Color::Black));
        assert_eq!(app.document().color(Point::new(2, 0)), Some(red()));
        assert!(app.undo());
        assert_eq!(app.document().color(Point::new(1, 1)), Some(red()));
    }

    #[test]
    fn the_fill_replaces_the_4_connected_region() {
        let mut app = EditorApp::new(small_document(&["rrr", "rgr"]), None).unwrap();
        app.set_colour(Color::Black);
        app.fill_at(Point::new(0, 0));

        assert_eq!(app.document().color(Point::new(2, 0)), Some(Color::Black));
        assert_eq!(app.document().color(Point::new(2, 1)), Some(Color::Black));
        assert_eq!(app.document().color(Point::new(1, 1)), Some(green()));
    }

    #[test]
    fn the_eyedropper_adopts_the_codel_colour() {
        let mut app = EditorApp::new(small_document(&["rg"]), None).unwrap();
        assert_eq!(app.colour(), red());
        app.pick_at(Point::new(1, 0));
        assert_eq!(app.colour(), green());
    }

    #[test]
    fn an_edit_invalidates_the_debug_session() {
        let mut app = EditorApp::new(small_document(&["rg", "gg"]), None).unwrap();
        app.debug_step().unwrap();
        assert!(app.session().is_some());

        app.set_colour(Color::Black);
        app.begin_edit();
        app.paint_to(Point::new(0, 0));
        app.end_edit();

        assert!(app.session().is_none());
    }

    #[cfg(feature = "codegen")]
    #[test]
    fn the_jit_comparison_agrees_on_a_terminating_program() {
        let app = EditorApp::new(small_document(&["rg", "gg"]), None).unwrap();
        let result = compare_jit(app.analysis().blocks().as_ref(), "");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn a_saved_png_reopens_the_same_document() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("program.png");

        let mut app =
            EditorApp::new(small_document(&["rg"]), Some(PathBuf::from("start.png"))).unwrap();
        app.set_colour(Color::Black);
        app.fill_at(Point::new(0, 0));
        assert!(app.is_dirty());
        app.save_to(&path).unwrap();
        assert!(!app.is_dirty());

        let mut reopened = EditorApp::new(small_document(&["rrr"]), None).unwrap();
        reopened.open_file(&path).unwrap();
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
    fn a_failed_open_keeps_the_current_document() {
        let mut app = EditorApp::new(small_document(&["rg"]), None).unwrap();
        let before = app.document().color(Point::new(0, 0));

        assert!(app.open_bytes(&[0, 1, 2, 3], None).is_err());
        assert_eq!(app.document().color(Point::new(0, 0)), before);
        assert_eq!(app.document().width(), 2);
    }

    #[test]
    fn new_document_replaces_the_image_and_the_session() {
        let mut app =
            EditorApp::new(small_document(&["rg"]), Some(PathBuf::from("x.png"))).unwrap();
        app.debug_step().unwrap();
        assert!(app.session().is_some());

        app.new_document(4, 3);
        assert_eq!(app.document().width(), 4);
        assert_eq!(app.document().height(), 3);
        assert!(app.is_dirty());
        assert!(app.session().is_none());
    }

    #[test]
    fn resizing_refreshes_the_analysis_and_invalidates_the_session() {
        let mut app = EditorApp::new(small_document(&["rg"]), None).unwrap();
        app.debug_step().unwrap();
        assert!(app.session().is_some());

        app.resize_document(3, 2);
        assert_eq!(app.document().width(), 3);
        assert_eq!(app.document().height(), 2);
        assert_eq!(app.document().color(Point::new(2, 1)), Some(Color::White));
        assert_eq!(app.analysis().report().width, 3);
        assert!(app.session().is_none());

        assert!(app.undo());
        assert_eq!(app.document().width(), 2);
        assert_eq!(app.document().height(), 1);
    }

    #[test]
    fn a_resize_to_the_same_size_is_not_an_edit() {
        let mut app =
            EditorApp::new(small_document(&["rg"]), Some(PathBuf::from("x.png"))).unwrap();
        let revision = app.document().revision();

        app.resize_document(2, 1);
        assert_eq!(app.document().revision(), revision);
        assert!(!app.is_dirty());
        assert!(!app.document().can_undo());
    }
}
