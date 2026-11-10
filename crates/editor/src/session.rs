//! One interpreter run, stepped by the debugger.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas::{BlockId, Blocks, Point};
use interp::{Machine, Options, Progress, Step};
use runtime::{BufferIo, Stack};
use tessera::{Command, Exit, State};

use crate::{Analysis, Document};

/// The number of visited codels kept for the trail.
const MAX_TRAIL: usize = 16_384;
/// The number of steps kept in the history and the trace export.
const MAX_TRACE: usize = 100_000;

/// Why a run stopped, or paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Terminated,
    StepLimit,
    StackOverflow,
    /// A breakpoint fired; the run can continue.
    Breakpoint,
    /// The frame's step or time budget ran out; the run can continue.
    Budget,
}

impl Stop {
    /// Whether the run cannot continue.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            Stop::Terminated | Stop::StepLimit | Stop::StackOverflow
        )
    }
}

/// How much one `run_until` call may execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub steps: u64,
    pub time: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            steps: 100_000,
            time: Duration::from_millis(4),
        }
    }
}

/// Where a run should pause before stepping.
#[derive(Debug, Clone, Default)]
pub struct Breakpoints {
    pub blocks: HashSet<BlockId>,
    pub states: HashSet<State>,
    pub commands: HashSet<Command>,
    /// Pause before any `in` or `out` command.
    pub pause_on_io: bool,
}

impl Breakpoints {
    /// Adds the block if it is absent, removes it if it is present.
    pub fn toggle_block(&mut self, block: BlockId) {
        if !self.blocks.remove(&block) {
            self.blocks.insert(block);
        }
    }

    /// The block breakpoints in id order, for a stable list.
    pub fn sorted_blocks(&self) -> Vec<BlockId> {
        let mut blocks: Vec<BlockId> = self.blocks.iter().copied().collect();
        blocks.sort();
        blocks
    }

    pub fn matches(&self, blocks: &Blocks, state: Option<State>) -> bool {
        let Some(state) = state else {
            return false;
        };
        if self.blocks.contains(&state.block) || self.states.contains(&state) {
            return true;
        }
        let Exit::Move {
            command: Some(command),
            ..
        } = tessera::exit(blocks, state.block, state.dp, state.cc)
        else {
            return false;
        };
        self.commands.contains(&command)
            || (self.pause_on_io
                && matches!(
                    command,
                    Command::InNumber | Command::InChar | Command::OutNumber | Command::OutChar
                ))
    }
}

/// A debug session over one revision of a document.
#[derive(Debug)]
pub struct Session {
    blocks: Arc<Blocks>,
    machine: Machine,
    io: BufferIo,
    input: String,
    options: Options,
    trace: Vec<Step>,
    trace_truncated: bool,
    trail: Vec<Point>,
    stop: Option<Stop>,
    revision: u64,
    at_breakpoint: bool,
}

impl Session {
    pub fn new(analysis: &Analysis, input: String, options: Options) -> Self {
        let blocks = Arc::clone(analysis.blocks());
        let machine = Machine::new(&blocks, options);
        Self {
            io: BufferIo::new(&input),
            input,
            options,
            machine,
            blocks,
            trace: Vec::new(),
            trace_truncated: false,
            trail: Vec::new(),
            stop: None,
            revision: analysis.revision(),
            at_breakpoint: false,
        }
    }

    /// Whether this session still describes the document's current revision.
    pub fn is_valid_for(&self, document: &Document) -> bool {
        self.revision == document.revision()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn blocks(&self) -> &Blocks {
        &self.blocks
    }

    /// Runs exactly one transition, ignoring breakpoints.
    pub fn step(&mut self) -> Result<Option<Progress>, interp::Error> {
        if self.is_finished() {
            return Ok(None);
        }
        self.stop = None;
        self.at_breakpoint = false;
        self.step_once()
    }

    /// Runs until a breakpoint, a final stop, or the budget.
    ///
    /// A breakpoint that fired on the previous call is stepped over rather than
    /// firing again on the same state.
    pub fn run_until(
        &mut self,
        breakpoints: &Breakpoints,
        budget: Budget,
    ) -> Result<Stop, interp::Error> {
        if let Some(stop) = self.stop
            && stop.is_final()
        {
            return Ok(stop);
        }
        let started = Instant::now();
        let mut steps = 0u64;
        let mut skip_check = self.at_breakpoint;
        self.at_breakpoint = false;
        self.stop = None;

        loop {
            if !skip_check && breakpoints.matches(&self.blocks, self.machine.state()) {
                self.at_breakpoint = true;
                self.stop = Some(Stop::Breakpoint);
                return Ok(Stop::Breakpoint);
            }
            skip_check = false;
            if steps >= budget.steps || started.elapsed() >= budget.time {
                return Ok(Stop::Budget);
            }
            match self.step_once()? {
                Some(Progress::Moved { .. }) => steps += 1,
                Some(Progress::Terminated { .. }) | None => return Ok(Stop::Terminated),
                Some(Progress::StepLimit) => return Ok(Stop::StepLimit),
            }
        }
    }

    /// Replaces the input and restarts from the beginning.
    pub fn reset(&mut self) {
        self.machine = Machine::new(&self.blocks, self.options);
        self.io = BufferIo::new(&self.input);
        self.trace.clear();
        self.trace_truncated = false;
        self.trail.clear();
        self.stop = None;
        self.at_breakpoint = false;
    }

    pub fn set_input(&mut self, input: String) {
        self.input = input;
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn state(&self) -> Option<State> {
        self.machine.state()
    }

    pub fn stack(&self) -> &Stack {
        self.machine.stack()
    }

    pub fn steps(&self) -> u64 {
        self.machine.steps()
    }

    pub fn trace(&self) -> &[Step] {
        &self.trace
    }

    /// Whether the history dropped steps; the run was longer than the cap.
    pub fn trace_truncated(&self) -> bool {
        self.trace_truncated
    }

    pub fn trail(&self) -> &[Point] {
        &self.trail
    }

    pub fn output(&self) -> &str {
        self.io.output()
    }

    pub fn stop(&self) -> Option<Stop> {
        self.stop
    }

    pub fn is_finished(&self) -> bool {
        self.stop.is_some_and(Stop::is_final)
    }

    /// The recorded trace in `pietrs trace`'s line format.
    pub fn export_trace(&self) -> String {
        let mut out = String::new();
        for step in &self.trace {
            out.push_str(&step.to_string());
            out.push('\n');
        }
        out
    }

    fn step_once(&mut self) -> Result<Option<Progress>, interp::Error> {
        let progress = match self.machine.step(&self.blocks, &mut self.io) {
            Ok(progress) => progress,
            Err(error) => {
                self.stop = Some(Stop::StackOverflow);
                return Err(error);
            }
        };
        let Some(progress) = progress else {
            self.stop = Some(Stop::Terminated);
            return Ok(None);
        };
        if let Some(step) = progress.step() {
            self.record(step);
        }
        match progress {
            Progress::Moved { from, .. } => {
                if let Some(point) = tessera::edge_codel(&self.blocks, from.block, from.dp, from.cc)
                {
                    self.record_trail(point);
                }
            }
            Progress::Terminated { .. } => self.stop = Some(Stop::Terminated),
            Progress::StepLimit => self.stop = Some(Stop::StepLimit),
        }
        Ok(Some(progress))
    }

    fn record(&mut self, step: Step) {
        if self.trace.len() == MAX_TRACE {
            self.trace_truncated = true;
            return;
        }
        self.trace.push(step);
    }

    fn record_trail(&mut self, point: Point) {
        if self.trail.len() == MAX_TRAIL {
            self.trail.drain(..MAX_TRAIL / 2);
        }
        self.trail.push(point);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Color, Hue, Lightness};
    use interp::run_traced;
    use runtime::BufferIo;

    fn document(codels: Vec<Color>, width: u32) -> Document {
        let height = (codels.len() as u32) / width;
        Document::from_grid(&CodelGrid::from_colors(width, height, codels).unwrap())
    }

    fn print_a_document() -> Document {
        let mut codels = vec![Color::chromatic(Hue::Red, Lightness::Normal); 65];
        codels.push(Color::chromatic(Hue::Red, Lightness::Dark));
        codels.push(Color::chromatic(Hue::Magenta, Lightness::Normal));
        document(codels, 67)
    }

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
        document(codels, width)
    }

    #[test]
    fn a_session_matches_a_traced_run() {
        let document = print_a_document();
        let analysis = Analysis::build(&document).unwrap();
        let options = Options {
            max_steps: Some(2),
            ..Options::default()
        };
        let mut session = Session::new(&analysis, String::new(), options);
        let stop = session
            .run_until(&Breakpoints::default(), Budget::default())
            .unwrap();
        assert_eq!(stop, Stop::StepLimit);
        assert_eq!(session.steps(), 2);
        assert_eq!(session.output(), "A");

        let mut io = BufferIo::new("");
        let (result, trace) = run_traced(analysis.blocks().as_ref(), &mut io, &options).unwrap();
        assert_eq!(session.trace(), trace.as_slice());
        assert_eq!(session.steps(), result.steps);
        assert_eq!(session.output(), io.output());
    }

    #[test]
    fn a_block_breakpoint_pauses_before_the_step() {
        let document = small_document(&["rg", "gg"]);
        let analysis = Analysis::build(&document).unwrap();
        let green = analysis.blocks().block_id_at(Point::new(1, 0)).unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.blocks.insert(green);

        let mut session = Session::new(&analysis, String::new(), Options::default());
        let stop = session.run_until(&breakpoints, Budget::default()).unwrap();
        assert_eq!(stop, Stop::Breakpoint);
        assert_eq!(session.steps(), 1);
        assert_eq!(session.state().unwrap().block, green);

        // Resuming steps over the breakpoint and terminates.
        let stop = session.run_until(&breakpoints, Budget::default()).unwrap();
        assert_eq!(stop, Stop::Terminated);
    }

    #[test]
    fn a_breakpoint_on_the_start_state_pauses_at_step_zero() {
        let document = small_document(&["rg", "gg"]);
        let analysis = Analysis::build(&document).unwrap();
        let start = analysis.blocks().block_id_at(Point::new(0, 0)).unwrap();
        let mut breakpoints = Breakpoints::default();
        breakpoints.blocks.insert(start);

        let mut session = Session::new(&analysis, String::new(), Options::default());
        let stop = session.run_until(&breakpoints, Budget::default()).unwrap();
        assert_eq!(stop, Stop::Breakpoint);
        assert_eq!(session.steps(), 0);
    }

    #[test]
    fn a_command_breakpoint_pauses_before_the_command_runs() {
        let document = print_a_document();
        let analysis = Analysis::build(&document).unwrap();
        let options = Options {
            max_steps: Some(2),
            ..Options::default()
        };
        let mut breakpoints = Breakpoints::default();
        breakpoints.commands.insert(Command::OutChar);

        let mut session = Session::new(&analysis, String::new(), options);
        let stop = session.run_until(&breakpoints, Budget::default()).unwrap();
        assert_eq!(stop, Stop::Breakpoint);
        assert_eq!(session.steps(), 1);
        assert_eq!(session.stack().as_slice(), &[65]);
        assert_eq!(session.output(), "");

        let stop = session.run_until(&breakpoints, Budget::default()).unwrap();
        assert_eq!(stop, Stop::StepLimit);
        assert_eq!(session.output(), "A");
    }

    #[test]
    fn reset_returns_to_the_start() {
        let document = print_a_document();
        let analysis = Analysis::build(&document).unwrap();
        let options = Options {
            max_steps: Some(1),
            ..Options::default()
        };
        let mut session = Session::new(&analysis, String::new(), options);
        session.step().unwrap();
        assert_eq!(session.steps(), 1);

        session.reset();
        assert_eq!(session.steps(), 0);
        assert!(session.trace().is_empty());
        assert!(session.trail().is_empty());
        assert_eq!(session.output(), "");
        assert_eq!(session.stop(), None);
        assert_eq!(session.state(), analysis.graph().start());
    }

    #[test]
    fn a_step_limit_is_final() {
        let document = small_document(&["rg"]);
        let analysis = Analysis::build(&document).unwrap();
        let options = Options {
            max_steps: Some(0),
            ..Options::default()
        };
        let mut session = Session::new(&analysis, String::new(), options);
        let stop = session
            .run_until(&Breakpoints::default(), Budget::default())
            .unwrap();
        assert_eq!(stop, Stop::StepLimit);
        assert_eq!(session.steps(), 0);
        assert!(session.is_finished());
        assert_eq!(session.stop(), Some(Stop::StepLimit));
    }

    #[test]
    fn the_exported_trace_is_the_line_format() {
        let document = print_a_document();
        let analysis = Analysis::build(&document).unwrap();
        let options = Options {
            max_steps: Some(2),
            ..Options::default()
        };
        let mut session = Session::new(&analysis, String::new(), options);
        session
            .run_until(&Breakpoints::default(), Budget::default())
            .unwrap();

        let expected: String = session
            .trace()
            .iter()
            .map(|step| format!("{step}\n"))
            .collect();
        assert_eq!(session.export_trace(), expected);
    }

    #[test]
    fn an_edit_invalidates_the_session() {
        let mut document = small_document(&["rg"]);
        let analysis = Analysis::build(&document).unwrap();
        let session = Session::new(&analysis, String::new(), Options::default());
        assert!(session.is_valid_for(&document));

        document.set(Point::new(0, 0), Color::Black);
        assert!(!session.is_valid_for(&document));
    }

    #[test]
    fn block_breakpoints_toggle_and_sort() {
        let mut breakpoints = Breakpoints::default();
        breakpoints.toggle_block(BlockId(3));
        breakpoints.toggle_block(BlockId(1));
        assert_eq!(breakpoints.sorted_blocks(), vec![BlockId(1), BlockId(3)]);

        breakpoints.toggle_block(BlockId(3));
        assert_eq!(breakpoints.sorted_blocks(), vec![BlockId(1)]);
    }
}
