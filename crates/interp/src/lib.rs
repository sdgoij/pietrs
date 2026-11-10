#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The reference interpreter: walk the tessellation flow and apply commands.

use std::fmt;

use canvas::Blocks;
use runtime::{Division, Io, Stack, Value, greater, not};
use tessera::{Chooser, Command, Direction, Exit, Start, State, start};

/// How the interpreter bounds a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub stack_capacity: usize,
    pub max_steps: Option<u64>,
    pub division: Division,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stack_capacity: 1 << 20,
            max_steps: None,
            division: Division::Floored,
        }
    }
}

/// Why a run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Terminated,
    StepLimit,
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub reason: ExitReason,
    pub steps: u64,
}

/// One transition, recorded before the command runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub block: canvas::BlockId,
    pub dp: Direction,
    pub cc: Chooser,
    pub command: Option<Command>,
    pub target: Option<canvas::BlockId>,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "block={} dp={:?} cc={:?} command=",
            self.block.0, self.dp, self.cc
        )?;
        match self.command {
            Some(command) => write!(f, "{command:?}")?,
            None => write!(f, "none")?,
        }
        write!(f, " target=")?;
        match self.target {
            Some(target) => write!(f, "{}", target.0),
            None => write!(f, "none"),
        }
    }
}

/// A runtime failure that is not ignored by the specification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Stack(#[from] runtime::Error),
}

/// Runs the program to termination or the step limit.
pub fn run(blocks: &Blocks, io: &mut dyn Io, options: &Options) -> Result<Run, Error> {
    run_inner(blocks, io, options, None)
}

/// Runs the program and records every transition.
pub fn run_traced(
    blocks: &Blocks,
    io: &mut dyn Io,
    options: &Options,
) -> Result<(Run, Vec<Step>), Error> {
    let mut trace = Vec::new();
    let result = run_inner(blocks, io, options, Some(&mut trace))?;
    Ok((result, trace))
}

/// One step of a [`Machine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The machine executed a transition.
    Moved {
        from: State,
        to: State,
        command: Option<Command>,
        target: canvas::BlockId,
    },
    /// The program stopped. `from` is `None` when it could not start at all,
    /// which records no step.
    Terminated { from: Option<State> },
    /// The step limit was reached before a transition.
    StepLimit,
}

impl Progress {
    /// The transition the trace records, if any.
    pub fn step(&self) -> Option<Step> {
        match self {
            Self::Moved {
                from,
                command,
                target,
                ..
            } => Some(Step {
                block: from.block,
                dp: from.dp,
                cc: from.cc,
                command: *command,
                target: Some(*target),
            }),
            Self::Terminated { from: Some(from) } => Some(Step {
                block: from.block,
                dp: from.dp,
                cc: from.cc,
                command: None,
                target: None,
            }),
            Self::Terminated { from: None } | Self::StepLimit => None,
        }
    }
}

/// The execution state, independent of the blocks it walks.
///
/// `run` and `run_traced` drive a machine; so does the debugger. There is one
/// step implementation.
#[derive(Debug)]
pub struct Machine {
    state: Option<State>,
    stack: Stack,
    steps: u64,
    max_steps: Option<u64>,
    division: Division,
    finished: bool,
}

impl Machine {
    pub fn new(blocks: &Blocks, options: Options) -> Self {
        let state = match start(blocks) {
            Start::State(state) => Some(state),
            Start::Terminate => None,
        };
        Self {
            state,
            stack: Stack::new(options.stack_capacity),
            steps: 0,
            max_steps: options.max_steps,
            division: options.division,
            finished: false,
        }
    }

    /// Runs one transition.
    ///
    /// `blocks` must be the partition the machine was created from. `Ok(None)`
    /// means the machine had already finished and nothing changed.
    pub fn step(&mut self, blocks: &Blocks, io: &mut dyn Io) -> Result<Option<Progress>, Error> {
        if self.finished {
            return Ok(None);
        }
        if let Some(limit) = self.max_steps
            && self.steps >= limit
        {
            self.finished = true;
            return Ok(Some(Progress::StepLimit));
        }
        let Some(current) = self.state else {
            self.finished = true;
            return Ok(Some(Progress::Terminated { from: None }));
        };
        match tessera::exit(blocks, current.block, current.dp, current.cc) {
            Exit::Terminate => {
                self.finished = true;
                self.state = None;
                Ok(Some(Progress::Terminated {
                    from: Some(current),
                }))
            }
            Exit::Move {
                target,
                dp,
                cc,
                command,
            } => {
                let block_size = blocks
                    .block(current.block)
                    .map_or(0, |block| block.size() as Value);
                let (dp, cc) = match execute_with(
                    command,
                    block_size,
                    dp,
                    cc,
                    &mut self.stack,
                    io,
                    self.division,
                ) {
                    Ok(result) => result,
                    Err(error) => {
                        self.finished = true;
                        return Err(error);
                    }
                };
                let to = State {
                    block: target,
                    dp,
                    cc,
                };
                self.state = Some(to);
                self.steps += 1;
                Ok(Some(Progress::Moved {
                    from: current,
                    to,
                    command,
                    target,
                }))
            }
        }
    }

    /// The state the next step starts from, or `None` when the machine is
    /// finished.
    pub fn state(&self) -> Option<State> {
        self.state
    }

    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

fn run_inner(
    blocks: &Blocks,
    io: &mut dyn Io,
    options: &Options,
    mut trace: Option<&mut Vec<Step>>,
) -> Result<Run, Error> {
    let mut machine = Machine::new(blocks, *options);
    loop {
        let Some(progress) = machine.step(blocks, io)? else {
            return Ok(Run {
                reason: ExitReason::Terminated,
                steps: machine.steps(),
            });
        };
        if let Some(trace) = trace.as_deref_mut()
            && let Some(step) = progress.step()
        {
            trace.push(step);
        }
        match progress {
            Progress::Moved { .. } => {}
            Progress::Terminated { .. } => {
                return Ok(Run {
                    reason: ExitReason::Terminated,
                    steps: machine.steps(),
                });
            }
            Progress::StepLimit => {
                return Ok(Run {
                    reason: ExitReason::StepLimit,
                    steps: machine.steps(),
                });
            }
        }
    }
}

fn execute_with(
    command: Option<Command>,
    block_size: Value,
    dp: Direction,
    cc: Chooser,
    stack: &mut Stack,
    io: &mut dyn Io,
    division: Division,
) -> Result<(Direction, Chooser), Error> {
    let Some(command) = command else {
        return Ok((dp, cc));
    };
    match command {
        Command::Push => stack.push(block_size)?,
        Command::Pop => {
            stack.pop();
        }
        Command::Add => binary(stack, |a, b| Some(a.wrapping_add(b)))?,
        Command::Subtract => binary(stack, |a, b| Some(a.wrapping_sub(b)))?,
        Command::Multiply => binary(stack, |a, b| Some(a.wrapping_mul(b)))?,
        Command::Divide => binary(stack, |a, b| division.divide(a, b))?,
        Command::Modulo => binary(stack, |a, b| division.modulo(a, b))?,
        Command::Not => {
            if let Some(value) = stack.pop() {
                stack.push(not(value))?;
            }
        }
        Command::Greater => binary(stack, |a, b| Some(greater(a, b)))?,
        Command::Pointer => {
            let turns = stack.pop().unwrap_or(0).rem_euclid(4) as u32;
            return Ok((dp.clockwise_by(turns), cc));
        }
        Command::Switch => {
            let toggles = stack.pop().unwrap_or(0).rem_euclid(2);
            return Ok((dp, if toggles == 1 { cc.toggled() } else { cc }));
        }
        Command::Duplicate => stack.duplicate()?,
        Command::Roll => {
            if let Some((depth, rolls)) = stack.pop_two() {
                match stack.roll(depth, rolls) {
                    Ok(()) | Err(runtime::Error::InvalidDepth(_)) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Command::InNumber => {
            if let Some(value) = io.read_number() {
                stack.push(value)?;
            }
        }
        Command::InChar => {
            if let Some(c) = io.read_char() {
                stack.push(Value::from(c as u32))?;
            }
        }
        Command::OutNumber => {
            if let Some(value) = stack.pop() {
                io.write_number(value);
            }
        }
        Command::OutChar => {
            if let Some(value) = stack.pop()
                && let Ok(code) = u32::try_from(value)
                && let Some(c) = char::from_u32(code)
            {
                io.write_char(c);
            }
        }
    }
    Ok((dp, cc))
}

#[cfg(test)]
fn execute(
    command: Option<Command>,
    block_size: Value,
    dp: Direction,
    cc: Chooser,
    stack: &mut Stack,
    io: &mut dyn Io,
) -> Result<(Direction, Chooser), Error> {
    execute_with(command, block_size, dp, cc, stack, io, Division::Floored)
}

fn binary(
    stack: &mut Stack,
    operation: impl FnOnce(Value, Value) -> Option<Value>,
) -> Result<(), Error> {
    let Some((a, b)) = stack.pop_two() else {
        return Ok(());
    };
    if let Some(result) = operation(a, b) {
        stack.push(result)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{CodelGrid, Color, Hue, Lightness};
    use runtime::BufferIo;

    fn blocks(rows: &[&str]) -> Blocks {
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, |row| row.len() as u32);
        let mut codels = Vec::with_capacity((width as usize) * (height as usize));
        for row in rows {
            assert_eq!(row.len() as u32, width, "rows must be rectangular");
            for cell in row.chars() {
                codels.push(match cell {
                    '.' => Color::White,
                    '#' => Color::Black,
                    'r' => Color::chromatic(Hue::Red, Lightness::Normal),
                    'g' => Color::chromatic(Hue::Green, Lightness::Normal),
                    'd' => Color::chromatic(Hue::Red, Lightness::Dark),
                    other => panic!("unknown cell {other:?}"),
                });
            }
        }
        Blocks::partition(CodelGrid::from_colors(width, height, codels).unwrap())
    }

    fn context() -> (Stack, BufferIo) {
        (Stack::new(8), BufferIo::new(""))
    }

    #[test]
    fn pointer_rotates_by_the_popped_value() {
        let (mut stack, mut io) = context();
        stack.push(1).unwrap();
        let (dp, cc) = execute(
            Some(Command::Pointer),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!((dp, cc), (Direction::Down, Chooser::Left));
        assert!(stack.is_empty());
    }

    #[test]
    fn a_negative_pointer_turns_anticlockwise() {
        let (mut stack, mut io) = context();
        stack.push(-1).unwrap();
        let (dp, _) = execute(
            Some(Command::Pointer),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(dp, Direction::Up);
    }

    #[test]
    fn an_empty_stack_leaves_the_pointer_alone() {
        let (mut stack, mut io) = context();
        let (dp, cc) = execute(
            Some(Command::Pointer),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!((dp, cc), (Direction::Right, Chooser::Left));
    }

    #[test]
    fn switch_toggles_once_for_an_odd_value() {
        let (mut stack, mut io) = context();
        stack.push(1).unwrap();
        let (dp, cc) = execute(
            Some(Command::Switch),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!((dp, cc), (Direction::Right, Chooser::Right));
    }

    #[test]
    fn switch_does_nothing_for_an_even_value() {
        let (mut stack, mut io) = context();
        stack.push(2).unwrap();
        let (dp, cc) = execute(
            Some(Command::Switch),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!((dp, cc), (Direction::Right, Chooser::Left));
    }

    #[test]
    fn a_binary_operation_with_one_value_is_ignored() {
        let (mut stack, mut io) = context();
        stack.push(7).unwrap();
        execute(
            Some(Command::Add),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[7]);
    }

    #[test]
    fn add_wraps_on_overflow() {
        let (mut stack, mut io) = context();
        stack.push(Value::MAX).unwrap();
        stack.push(1).unwrap();
        execute(
            Some(Command::Add),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[Value::MIN]);
    }

    #[test]
    fn divide_by_zero_is_ignored() {
        let (mut stack, mut io) = context();
        stack.push(7).unwrap();
        stack.push(0).unwrap();
        execute(
            Some(Command::Divide),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert!(stack.is_empty());
    }

    #[test]
    fn out_char_prints_the_codepoint() {
        let (mut stack, mut io) = context();
        stack.push(65).unwrap();
        execute(
            Some(Command::OutChar),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(io.output(), "A");
    }

    #[test]
    fn out_number_writes_the_value() {
        let (mut stack, mut io) = context();
        stack.push(-42).unwrap();
        execute(
            Some(Command::OutNumber),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(io.output(), "-42");
    }

    #[test]
    fn in_char_pushes_the_codepoint() {
        let mut stack = Stack::new(8);
        let mut io = BufferIo::new("Z");
        execute(
            Some(Command::InChar),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[90]);
    }

    #[test]
    fn in_number_pushes_the_parsed_value() {
        let mut stack = Stack::new(8);
        let mut io = BufferIo::new("42");
        execute(
            Some(Command::InNumber),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[42]);
    }

    #[test]
    fn truncating_division_is_selectable() {
        let (mut stack, mut io) = context();
        stack.push(-7).unwrap();
        stack.push(3).unwrap();
        execute_with(
            Some(Command::Divide),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
            Division::Truncating,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[-2]);
    }

    #[test]
    fn roll_uses_the_top_two_values() {
        let (mut stack, mut io) = context();
        for value in [1, 2, 3, 3, 1] {
            stack.push(value).unwrap();
        }
        execute(
            Some(Command::Roll),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[3, 1, 2]);
    }

    #[test]
    fn an_invalid_roll_is_ignored() {
        let (mut stack, mut io) = context();
        for value in [1, -1, 1] {
            stack.push(value).unwrap();
        }
        execute(
            Some(Command::Roll),
            0,
            Direction::Right,
            Chooser::Left,
            &mut stack,
            &mut io,
        )
        .unwrap();
        assert_eq!(stack.as_slice(), &[1]);
    }

    #[test]
    fn a_step_displays_the_transition() {
        let step = Step {
            block: canvas::BlockId(3),
            dp: Direction::Down,
            cc: Chooser::Right,
            command: Some(Command::OutChar),
            target: Some(canvas::BlockId(7)),
        };
        assert_eq!(
            step.to_string(),
            "block=3 dp=Down cc=Right command=OutChar target=7"
        );

        let terminal = Step {
            block: canvas::BlockId(1),
            dp: Direction::Up,
            cc: Chooser::Left,
            command: None,
            target: None,
        };
        assert_eq!(
            terminal.to_string(),
            "block=1 dp=Up cc=Left command=none target=none"
        );
    }

    #[test]
    fn stepping_matches_the_recorded_trace() {
        let blocks = blocks(&["rg", "gg"]);
        let options = Options::default();
        let (result, trace) = run_traced(&blocks, &mut BufferIo::new(""), &options).unwrap();

        let mut machine = Machine::new(&blocks, options);
        let mut io = BufferIo::new("");
        let mut stepped = Vec::new();
        while let Some(progress) = machine.step(&blocks, &mut io).unwrap() {
            if let Some(step) = progress.step() {
                stepped.push(step);
            }
            if matches!(progress, Progress::Terminated { .. } | Progress::StepLimit) {
                break;
            }
        }

        assert_eq!(stepped, trace);
        assert_eq!(machine.steps(), result.steps);
        assert!(machine.is_finished());
    }

    #[test]
    fn a_step_limit_is_checked_before_a_transition() {
        let blocks = blocks(&["rg"]);
        let options = Options {
            max_steps: Some(0),
            ..Options::default()
        };
        let mut machine = Machine::new(&blocks, options);
        let mut io = BufferIo::new("");
        assert_eq!(
            machine.step(&blocks, &mut io).unwrap(),
            Some(Progress::StepLimit)
        );
        assert_eq!(machine.steps(), 0);
        assert!(machine.is_finished());
        assert_eq!(machine.step(&blocks, &mut io).unwrap(), None);
    }

    #[test]
    fn a_program_that_cannot_start_terminates_without_a_step() {
        let blocks = blocks(&["#"]);
        let mut machine = Machine::new(&blocks, Options::default());
        assert_eq!(machine.state(), None);

        let mut io = BufferIo::new("");
        assert_eq!(
            machine.step(&blocks, &mut io).unwrap(),
            Some(Progress::Terminated { from: None })
        );
        assert_eq!(machine.steps(), 0);
        assert!(machine.is_finished());
    }

    #[test]
    fn the_machine_exposes_the_stack_between_transitions() {
        // Red normal to red dark is `push`, and the red block is one codel.
        let blocks = blocks(&["rd"]);
        let mut machine = Machine::new(&blocks, Options::default());
        assert_eq!(machine.stack().as_slice(), &[]);

        let mut io = BufferIo::new("");
        machine.step(&blocks, &mut io).unwrap();
        assert_eq!(machine.stack().as_slice(), &[1]);
    }

    #[test]
    fn an_overflow_finishes_the_machine() {
        let blocks = blocks(&["rd"]);
        let options = Options {
            stack_capacity: 0,
            ..Options::default()
        };
        let mut machine = Machine::new(&blocks, options);
        let mut io = BufferIo::new("");
        assert!(machine.step(&blocks, &mut io).is_err());
        assert!(machine.is_finished());
        assert_eq!(machine.step(&blocks, &mut io).unwrap(), None);
    }
}
