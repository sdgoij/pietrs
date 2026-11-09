//! Input and output, with a deterministic in-memory implementation.

use std::collections::VecDeque;

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

#[cfg(test)]
mod tests {
    use super::*;

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
