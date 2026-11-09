//! The direction pointer and the codel chooser.

/// The direction pointer. Clockwise is right, down, left, up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    Right,
    Down,
    Left,
    Up,
}

impl Direction {
    /// Every direction in clockwise order.
    pub const ALL: [Direction; 4] = [
        Direction::Right,
        Direction::Down,
        Direction::Left,
        Direction::Up,
    ];

    /// The next direction clockwise.
    pub const fn clockwise(self) -> Direction {
        match self {
            Direction::Right => Direction::Down,
            Direction::Down => Direction::Left,
            Direction::Left => Direction::Up,
            Direction::Up => Direction::Right,
        }
    }

    /// Rotates clockwise by `steps`, modulo four.
    pub const fn clockwise_by(self, steps: u32) -> Direction {
        let mut direction = self;
        let mut remaining = steps % 4;
        while remaining > 0 {
            direction = direction.clockwise();
            remaining -= 1;
        }
        direction
    }

    /// The codel delta in image coordinates.
    pub const fn delta(self) -> (i32, i32) {
        match self {
            Direction::Right => (1, 0),
            Direction::Down => (0, 1),
            Direction::Left => (-1, 0),
            Direction::Up => (0, -1),
        }
    }
}

/// The codel chooser, relative to the direction pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Chooser {
    Left,
    Right,
}

impl Chooser {
    /// The opposite side.
    pub const fn toggled(self) -> Chooser {
        match self {
            Chooser::Left => Chooser::Right,
            Chooser::Right => Chooser::Left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clockwise_walks_the_four_directions() {
        assert_eq!(Direction::Right.clockwise(), Direction::Down);
        assert_eq!(Direction::Down.clockwise(), Direction::Left);
        assert_eq!(Direction::Left.clockwise(), Direction::Up);
        assert_eq!(Direction::Up.clockwise(), Direction::Right);
    }

    #[test]
    fn clockwise_by_is_modulo_four() {
        assert_eq!(Direction::Right.clockwise_by(0), Direction::Right);
        assert_eq!(Direction::Right.clockwise_by(1), Direction::Down);
        assert_eq!(Direction::Right.clockwise_by(3), Direction::Up);
        assert_eq!(Direction::Right.clockwise_by(4), Direction::Right);
        assert_eq!(Direction::Up.clockwise_by(9), Direction::Right);
    }

    #[test]
    fn deltas_match_image_coordinates() {
        assert_eq!(Direction::Right.delta(), (1, 0));
        assert_eq!(Direction::Down.delta(), (0, 1));
        assert_eq!(Direction::Left.delta(), (-1, 0));
        assert_eq!(Direction::Up.delta(), (0, -1));
    }

    #[test]
    fn toggling_the_chooser_is_its_own_inverse() {
        assert_eq!(Chooser::Left.toggled(), Chooser::Right);
        assert_eq!(Chooser::Right.toggled(), Chooser::Left);
        assert_eq!(Chooser::Left.toggled().toggled(), Chooser::Left);
    }
}
