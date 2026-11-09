#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The reference interpreter: walk the tessellation flow and apply commands.

use canvas::Blocks;
use runtime::{Io, Stack, Value, floored_div, floored_mod, greater, not};
use tessera::{Chooser, Command, Direction, Exit, Start, State, start};

/// How the interpreter bounds a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub stack_capacity: usize,
    pub max_steps: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stack_capacity: 1 << 20,
            max_steps: None,
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

fn run_inner(
    blocks: &Blocks,
    io: &mut dyn Io,
    options: &Options,
    mut trace: Option<&mut Vec<Step>>,
) -> Result<Run, Error> {
    let Start::State(mut state) = start(blocks) else {
        return Ok(Run {
            reason: ExitReason::Terminated,
            steps: 0,
        });
    };
    let mut stack = Stack::new(options.stack_capacity);
    let mut steps = 0u64;

    loop {
        if let Some(limit) = options.max_steps
            && steps >= limit
        {
            return Ok(Run {
                reason: ExitReason::StepLimit,
                steps,
            });
        }
        match tessera::exit(blocks, state.block, state.dp, state.cc) {
            Exit::Terminate => {
                if let Some(trace) = trace.as_deref_mut() {
                    trace.push(Step {
                        block: state.block,
                        dp: state.dp,
                        cc: state.cc,
                        command: None,
                        target: None,
                    });
                }
                return Ok(Run {
                    reason: ExitReason::Terminated,
                    steps,
                });
            }
            Exit::Move {
                target,
                dp,
                cc,
                command,
            } => {
                if let Some(trace) = trace.as_deref_mut() {
                    trace.push(Step {
                        block: state.block,
                        dp: state.dp,
                        cc: state.cc,
                        command,
                        target: Some(target),
                    });
                }
                let block_size = blocks
                    .block(state.block)
                    .map_or(0, |block| block.size() as Value);
                let (dp, cc) = execute(command, block_size, dp, cc, &mut stack, io)?;
                state = State {
                    block: target,
                    dp,
                    cc,
                };
                steps += 1;
            }
        }
    }
}

fn execute(
    command: Option<Command>,
    block_size: Value,
    dp: Direction,
    cc: Chooser,
    stack: &mut Stack,
    io: &mut dyn Io,
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
        Command::Divide => binary(stack, floored_div)?,
        Command::Modulo => binary(stack, floored_mod)?,
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
    use runtime::BufferIo;

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
}
