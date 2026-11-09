//! The fixed-capacity value stack.

use crate::{Error, Value};

/// A fixed-capacity Piet value stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    values: Vec<Value>,
    capacity: usize,
}

impl Stack {
    /// A stack that overflows after `capacity` values.
    pub fn new(capacity: usize) -> Self {
        Self {
            values: Vec::new(),
            capacity,
        }
    }

    /// The capacity the interpreter uses unless told otherwise.
    pub fn with_default_capacity() -> Self {
        Self::new(1 << 20)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn as_slice(&self) -> &[Value] {
        &self.values
    }

    pub fn push(&mut self, value: Value) -> Result<(), Error> {
        if self.values.len() >= self.capacity {
            return Err(Error::Overflow);
        }
        self.values.push(value);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<Value> {
        self.values.pop()
    }

    pub fn peek(&self) -> Option<Value> {
        self.values.last().copied()
    }

    /// Pops the top two values as `(second, top)`, or `None` if there are fewer.
    pub fn pop_two(&mut self) -> Option<(Value, Value)> {
        if self.values.len() < 2 {
            return None;
        }
        let top = self.values.pop()?;
        let second = self.values.pop()?;
        Some((second, top))
    }

    /// Duplicates the top value; an empty stack is left alone.
    pub fn duplicate(&mut self) -> Result<(), Error> {
        if let Some(value) = self.peek() {
            self.push(value)?;
        }
        Ok(())
    }

    /// Rolls the top `depth` values by `rolls` places.
    pub fn roll(&mut self, depth: Value, rolls: Value) -> Result<(), Error> {
        let original = depth;
        if depth < 0 {
            return Err(Error::InvalidDepth(original));
        }
        let Ok(depth) = usize::try_from(depth) else {
            return Err(Error::InvalidDepth(original));
        };
        if depth > self.values.len() {
            return Err(Error::InvalidDepth(original));
        }
        if depth <= 1 {
            return Ok(());
        }
        let start = self.values.len() - depth;
        let shift = rolls.rem_euclid(depth as Value) as usize;
        self.values[start..].rotate_right(shift);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(values: &[Value]) -> Stack {
        let mut stack = Stack::new(64);
        for &value in values {
            stack.push(value).unwrap();
        }
        stack
    }

    #[test]
    fn push_and_pop_walk_the_stack() {
        let mut stack = Stack::new(2);
        assert_eq!(stack.pop(), None);
        stack.push(1).unwrap();
        stack.push(2).unwrap();
        assert_eq!(stack.peek(), Some(2));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn a_full_stack_overflows() {
        let mut stack = Stack::new(1);
        stack.push(1).unwrap();
        assert_eq!(stack.push(2), Err(Error::Overflow));
        assert_eq!(stack.as_slice(), &[1]);
    }

    #[test]
    fn pop_two_returns_second_then_top() {
        let mut stack = stack(&[1, 2, 3]);
        assert_eq!(stack.pop_two(), Some((2, 3)));
        assert_eq!(stack.as_slice(), &[1]);
        assert_eq!(stack.pop_two(), None);
    }

    #[test]
    fn duplicate_leaves_an_empty_stack_alone() {
        let mut stack = Stack::new(2);
        stack.duplicate().unwrap();
        assert!(stack.is_empty());
        stack.push(7).unwrap();
        stack.duplicate().unwrap();
        assert_eq!(stack.as_slice(), &[7, 7]);
    }

    #[test]
    fn roll_matches_the_specification_example() {
        let mut stack = stack(&[1, 2, 3]);
        stack.roll(3, 1).unwrap();
        assert_eq!(stack.as_slice(), &[3, 1, 2]);
    }

    #[test]
    fn a_negative_roll_goes_the_other_way() {
        let mut stack = stack(&[1, 2, 3]);
        stack.roll(3, -1).unwrap();
        assert_eq!(stack.as_slice(), &[2, 3, 1]);
    }

    #[test]
    fn rolls_wrap_modulo_the_depth() {
        let mut stack = stack(&[1, 2, 3]);
        stack.roll(3, 4).unwrap();
        assert_eq!(stack.as_slice(), &[3, 1, 2]);
    }

    #[test]
    fn a_depth_of_zero_or_one_is_an_identity() {
        let mut stack = stack(&[1, 2, 3]);
        stack.roll(0, 5).unwrap();
        stack.roll(1, -3).unwrap();
        assert_eq!(stack.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn an_invalid_depth_is_reported() {
        let mut stack = stack(&[1, 2]);
        assert_eq!(stack.roll(-1, 1), Err(Error::InvalidDepth(-1)));
        assert_eq!(stack.roll(3, 1), Err(Error::InvalidDepth(3)));
    }
}
