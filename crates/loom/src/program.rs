//! The interpreted instruction set and the input stream it is read from.
//!
//! The interpreter eats a stream of numbers: an opcode, and for `push` the
//! block size to push. A zero opcode ends the program. The interpreted
//! program's own input is inlined at each `in number` or `in char`, because the
//! instruction stream and the data stream are one stream.

use canvas::Blocks;
use interp::Options;
use runtime::Io;
use tessera::Command;

use crate::error::Error;

/// The commands the interpreter can run, in opcode order.
///
/// `pointer` and `switch` are absent: they change the direction pointer of the
/// *interpreted* machine, which this interpreter does not model. A linear
/// program never runs them.
pub const OPCODES: [Command; 15] = [
    Command::Push,
    Command::Pop,
    Command::Add,
    Command::Subtract,
    Command::Multiply,
    Command::Divide,
    Command::Modulo,
    Command::Not,
    Command::Greater,
    Command::Duplicate,
    Command::Roll,
    Command::InNumber,
    Command::InChar,
    Command::OutNumber,
    Command::OutChar,
];

/// The opcode a command reads, or `None` when the interpreter cannot run it.
pub fn opcode(command: Command) -> Option<u8> {
    OPCODES
        .iter()
        .position(|candidate| *candidate == command)
        .and_then(|index| u8::try_from(index + 1).ok())
}

/// One instruction of an interpreted program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub command: Command,
    /// The block size a `push` pushes; meaningless for the other commands.
    pub block_size: u64,
}

/// The instructions a program runs, in order, read from its trace.
///
/// A run that terminates records a last step with no command; that step is not
/// an instruction. A run that slides through white has no instruction for the
/// slide either, and is refused because the interpreter cannot model it.
pub fn instructions(
    blocks: &Blocks,
    io: &mut dyn Io,
    options: &Options,
) -> Result<Vec<Instruction>, Error> {
    let (_, trace) = interp::run_traced(blocks, io, options)?;
    let mut instructions = Vec::new();
    for step in &trace {
        match (step.command, step.target) {
            (Some(command), Some(_)) => {
                let block_size = blocks
                    .block(step.block)
                    .map_or(0, |block| block.size() as u64);
                instructions.push(Instruction {
                    command,
                    block_size,
                });
            }
            (None, None) => {}
            (None, Some(_)) | (Some(_), None) => return Err(Error::NotLinear),
        }
    }
    Ok(instructions)
}

/// Encodes instructions as the stream the interpreter reads, inlining `input`
/// at every command that consumes it.
pub fn encode(instructions: &[Instruction], input: &str) -> Result<String, Error> {
    let mut feed = Feed::new(input);
    let mut stream = String::new();
    for instruction in instructions {
        match instruction.command {
            Command::Push => {
                stream.push_str("1 ");
                stream.push_str(&instruction.block_size.to_string());
                stream.push(' ');
            }
            Command::InNumber => {
                stream.push_str("12 ");
                let value = feed.number().ok_or(Error::InputExhausted)?;
                stream.push_str(&value.to_string());
                stream.push(' ');
            }
            Command::InChar => {
                // A separator, so a digit is not swallowed by the opcode.
                stream.push_str("13 ");
                let character = feed.character().ok_or(Error::InputExhausted)?;
                stream.push(character);
            }
            other => {
                let code = opcode(other).ok_or(Error::Unsupported(other))?;
                stream.push_str(&code.to_string());
                stream.push(' ');
            }
        }
    }
    stream.push('0');
    Ok(stream)
}

/// The interpreted program's input, consumed in the same order the runtime
/// would consume it.
struct Feed {
    characters: Vec<char>,
    at: usize,
}

impl Feed {
    fn new(text: &str) -> Self {
        Self {
            characters: text.chars().collect(),
            at: 0,
        }
    }

    fn character(&mut self) -> Option<char> {
        let character = self.characters.get(self.at).copied()?;
        self.at += 1;
        Some(character)
    }

    fn number(&mut self) -> Option<i64> {
        while self
            .characters
            .get(self.at)
            .is_some_and(|character| character.is_whitespace())
        {
            self.at += 1;
        }
        let mut text = String::new();
        if self
            .characters
            .get(self.at)
            .is_some_and(|character| *character == '+' || *character == '-')
        {
            text.push(self.characters.get(self.at).copied()?);
            self.at += 1;
        }
        while let Some(character) = self.characters.get(self.at).copied() {
            if !character.is_ascii_digit() {
                break;
            }
            text.push(character);
            self.at += 1;
        }
        text.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcodes_start_at_one_and_cover_every_interpreted_command() {
        assert_eq!(opcode(Command::Push), Some(1));
        assert_eq!(opcode(Command::Pop), Some(2));
        assert_eq!(opcode(Command::OutChar), Some(15));
        assert_eq!(opcode(Command::Pointer), None);
        assert_eq!(opcode(Command::Switch), None);
        let mut seen = Vec::new();
        for command in OPCODES {
            let code = opcode(command).expect("an opcode");
            assert!(!seen.contains(&code));
            seen.push(code);
        }
        assert_eq!(seen.len(), 15);
    }

    #[test]
    fn an_instruction_stream_ends_with_a_halt() {
        let instructions = [
            Instruction {
                command: Command::Push,
                block_size: 65,
            },
            Instruction {
                command: Command::OutChar,
                block_size: 1,
            },
        ];
        assert_eq!(encode(&instructions, "").expect("encodes"), "1 65 15 0");
    }

    #[test]
    fn input_is_inlined_at_the_command_that_reads_it() {
        let instructions = [
            Instruction {
                command: Command::InChar,
                block_size: 1,
            },
            Instruction {
                command: Command::OutChar,
                block_size: 1,
            },
        ];
        assert_eq!(encode(&instructions, "A").expect("encodes"), "13 A15 0");
    }

    #[test]
    fn a_number_reads_the_next_token() {
        let instructions = [Instruction {
            command: Command::InNumber,
            block_size: 1,
        }];
        assert_eq!(
            encode(&instructions, "  -42 ").expect("encodes"),
            "12 -42 0"
        );
    }

    #[test]
    fn a_digit_can_be_read_as_a_character() {
        let instructions = [Instruction {
            command: Command::InChar,
            block_size: 1,
        }];
        assert_eq!(encode(&instructions, "7").expect("encodes"), "13 70");
    }

    #[test]
    fn a_missing_input_is_reported() {
        let instructions = [Instruction {
            command: Command::InChar,
            block_size: 1,
        }];
        assert!(matches!(
            encode(&instructions, ""),
            Err(Error::InputExhausted)
        ));
    }

    #[test]
    fn a_control_flow_command_is_refused() {
        let instructions = [Instruction {
            command: Command::Pointer,
            block_size: 1,
        }];
        assert!(matches!(
            encode(&instructions, ""),
            Err(Error::Unsupported(Command::Pointer))
        ));
    }
}
