//! Input and output, with a deterministic in-memory implementation and a
//! streaming one over the process's standard streams.

use std::collections::VecDeque;
use std::io::{Read, Write};

use crate::Value;

/// The input side of a Piet program.
pub trait Input {
    /// Reads one character, or `None` when the input is exhausted.
    fn read_char(&mut self) -> Option<char>;

    /// Reads one integer, or `None` when the next token is not an integer.
    fn read_number(&mut self) -> Option<Value>;
}

/// The output side of a Piet program.
pub trait Output {
    fn write_char(&mut self, c: char);
    fn write_number(&mut self, value: Value);
}

/// A complete program I/O surface.
pub trait Io: Input + Output {}

impl<T: Input + Output> Io for T {}

/// A deterministic buffer implementation, used by `run` and by tests.
#[derive(Debug, Clone, Default)]
pub struct BufferIo {
    input: VecDeque<char>,
    output: String,
}

impl BufferIo {
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into().chars().collect(),
            output: String::new(),
        }
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn into_output(self) -> String {
        self.output
    }
}

impl Input for BufferIo {
    fn read_char(&mut self) -> Option<char> {
        self.input.pop_front()
    }

    fn read_number(&mut self) -> Option<Value> {
        while self.input.front().is_some_and(|c| c.is_whitespace()) {
            self.input.pop_front();
        }
        let mut text = String::new();
        if let Some(&c) = self.input.front()
            && (c == '+' || c == '-')
        {
            text.push(c);
            self.input.pop_front();
        }
        while let Some(&c) = self.input.front() {
            if c.is_ascii_digit() {
                text.push(c);
                self.input.pop_front();
            } else {
                break;
            }
        }
        if text.is_empty() || text == "+" || text == "-" {
            for c in text.chars().rev() {
                self.input.push_front(c);
            }
            return None;
        }
        text.parse().ok()
    }
}

impl Output for BufferIo {
    fn write_char(&mut self, c: char) {
        self.output.push(c);
    }

    fn write_number(&mut self, value: Value) {
        self.output.push_str(&value.to_string());
    }
}

/// Input and output over arbitrary byte streams, used for the process's stdio.
///
/// Reads block until bytes arrive, so an interactive program waits for input
/// instead of seeing an exhausted stream. The pushback buffer keeps the byte
/// that ends a number for the next read, exactly as [`BufferIo`] keeps its
/// characters; characters are decoded as UTF-8, as in [`BufferIo`].
pub struct StreamIo<R, W> {
    input: R,
    pushback: VecDeque<u8>,
    output: W,
}

impl StreamIo<std::io::Stdin, std::io::Stdout> {
    /// The process's standard input and output.
    pub fn new() -> Self {
        Self::over(std::io::stdin(), std::io::stdout())
    }
}

impl Default for StreamIo<std::io::Stdin, std::io::Stdout> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R, W> StreamIo<R, W> {
    /// A stream reader and writer, for tests and for the CLI's stdio.
    pub fn over(input: R, output: W) -> Self {
        Self {
            input,
            pushback: VecDeque::new(),
            output,
        }
    }
}

impl<R: Read, W> StreamIo<R, W> {
    /// The next byte, or `None` at end of stream or on a read error.
    fn byte(&mut self) -> Option<u8> {
        if let Some(byte) = self.pushback.pop_front() {
            return Some(byte);
        }
        let mut buffer = [0u8; 1];
        match self.input.read(&mut buffer) {
            Ok(1) => Some(buffer[0]),
            _ => None,
        }
    }

    fn unread(&mut self, byte: u8) {
        self.pushback.push_front(byte);
    }
}

impl<R: Read, W> Input for StreamIo<R, W> {
    fn read_char(&mut self) -> Option<char> {
        let first = self.byte()?;
        let width = match first {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return None,
        };
        let mut buffer = [0u8; 4];
        buffer[0] = first;
        for slot in buffer.iter_mut().take(width).skip(1) {
            *slot = self.byte()?;
        }
        std::str::from_utf8(&buffer[..width]).ok()?.chars().next()
    }

    fn read_number(&mut self) -> Option<Value> {
        let mut byte = loop {
            let byte = self.byte()?;
            if !byte.is_ascii_whitespace() {
                break byte;
            }
        };
        let mut text = String::new();
        if byte == b'+' || byte == b'-' {
            text.push(char::from(byte));
            let Some(next) = self.byte() else {
                self.unread(byte);
                return None;
            };
            byte = next;
        }
        let mut digits = 0;
        loop {
            if byte.is_ascii_digit() {
                text.push(char::from(byte));
                digits += 1;
                let Some(next) = self.byte() else {
                    break;
                };
                byte = next;
            } else {
                self.unread(byte);
                break;
            }
        }
        if digits == 0 {
            if let Some(sign) = text.chars().next() {
                self.unread(sign as u8);
            }
            return None;
        }
        text.parse().ok()
    }
}

impl<R, W: Write> Output for StreamIo<R, W> {
    fn write_char(&mut self, c: char) {
        let mut bytes = [0u8; 4];
        let _ = self.output.write_all(c.encode_utf8(&mut bytes).as_bytes());
        let _ = self.output.flush();
    }

    fn write_number(&mut self, value: Value) {
        let _ = write!(self.output, "{value}");
        let _ = self.output.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_stream_reads_a_number_and_leaves_the_delimiter() {
        let mut io = StreamIo::over(Cursor::new(b"  -42 7x".to_vec()), Vec::<u8>::new());
        assert_eq!(io.read_number(), Some(-42));
        assert_eq!(io.read_number(), Some(7));
        assert_eq!(io.read_char(), Some('x'));
        assert_eq!(io.read_char(), None);
    }

    #[test]
    fn a_stream_decodes_utf8_characters() {
        let mut io = StreamIo::over(Cursor::new("é!".as_bytes().to_vec()), Vec::<u8>::new());
        assert_eq!(io.read_char(), Some('é'));
        assert_eq!(io.read_char(), Some('!'));
        assert_eq!(io.read_char(), None);
    }

    #[test]
    fn a_stream_writes_and_flushes() {
        let mut io = StreamIo::over(Cursor::new(Vec::<u8>::new()), Vec::<u8>::new());
        io.write_char('A');
        io.write_number(-7);
        assert_eq!(io.output, b"A-7");
    }

    #[test]
    fn characters_are_read_in_order() {
        let mut io = BufferIo::new("ab");
        assert_eq!(io.read_char(), Some('a'));
        assert_eq!(io.read_char(), Some('b'));
        assert_eq!(io.read_char(), None);
    }

    #[test]
    fn numbers_skip_whitespace_and_keep_the_rest() {
        let mut io = BufferIo::new("  -42 7x");
        assert_eq!(io.read_number(), Some(-42));
        assert_eq!(io.read_number(), Some(7));
        assert_eq!(io.read_char(), Some('x'));
    }

    #[test]
    fn a_non_number_is_not_consumed() {
        let mut io = BufferIo::new("abc");
        assert_eq!(io.read_number(), None);
        assert_eq!(io.read_char(), Some('a'));
    }

    #[test]
    fn a_lone_sign_is_restored() {
        let mut io = BufferIo::new("+");
        assert_eq!(io.read_number(), None);
        assert_eq!(io.read_char(), Some('+'));
    }

    #[test]
    fn output_formats_characters_and_numbers() {
        let mut io = BufferIo::new("");
        io.write_char('A');
        io.write_number(-42);
        io.write_char('\n');
        assert_eq!(io.output(), "A-42\n");
    }
}
