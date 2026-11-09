//! The 17 commands and the colour-transition table.

use canvas::Color;

/// One Piet command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Push,
    Pop,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Not,
    Greater,
    Pointer,
    Switch,
    Duplicate,
    Roll,
    InNumber,
    InChar,
    OutNumber,
    OutChar,
}

/// The command a chromatic transition runs, if any.
pub fn command(from: Color, to: Color) -> Option<Command> {
    let (hue, lightness) = from.deltas(to)?;
    match (hue, lightness) {
        (0, 0) => None,
        (0, 1) => Some(Command::Push),
        (0, 2) => Some(Command::Pop),
        (1, 0) => Some(Command::Add),
        (1, 1) => Some(Command::Subtract),
        (1, 2) => Some(Command::Multiply),
        (2, 0) => Some(Command::Divide),
        (2, 1) => Some(Command::Modulo),
        (2, 2) => Some(Command::Not),
        (3, 0) => Some(Command::Greater),
        (3, 1) => Some(Command::Pointer),
        (3, 2) => Some(Command::Switch),
        (4, 0) => Some(Command::Duplicate),
        (4, 1) => Some(Command::Roll),
        (4, 2) => Some(Command::InNumber),
        (5, 0) => Some(Command::InChar),
        (5, 1) => Some(Command::OutNumber),
        (5, 2) => Some(Command::OutChar),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{Hue, Lightness};

    const TABLE: [[Option<Command>; 3]; 6] = [
        [None, Some(Command::Push), Some(Command::Pop)],
        [
            Some(Command::Add),
            Some(Command::Subtract),
            Some(Command::Multiply),
        ],
        [
            Some(Command::Divide),
            Some(Command::Modulo),
            Some(Command::Not),
        ],
        [
            Some(Command::Greater),
            Some(Command::Pointer),
            Some(Command::Switch),
        ],
        [
            Some(Command::Duplicate),
            Some(Command::Roll),
            Some(Command::InNumber),
        ],
        [
            Some(Command::InChar),
            Some(Command::OutNumber),
            Some(Command::OutChar),
        ],
    ];

    #[test]
    fn every_hue_and_lightness_pair_is_in_the_table() {
        for hue_delta in 0u8..6 {
            for lightness_delta in 0u8..3 {
                let hue = Hue::ALL[usize::from((Hue::Red.index() + hue_delta) % 6)];
                let lightness =
                    Lightness::ALL[usize::from((Lightness::Light.index() + lightness_delta) % 3)];
                let from = Color::chromatic(Hue::Red, Lightness::Light);
                let to = Color::chromatic(hue, lightness);
                assert_eq!(
                    command(from, to),
                    TABLE[usize::from(hue_delta)][usize::from(lightness_delta)],
                    "hue delta {hue_delta}, lightness delta {lightness_delta}"
                );
            }
        }
    }

    #[test]
    fn white_and_black_transitions_run_no_command() {
        let red = Color::chromatic(Hue::Red, Lightness::Normal);
        assert_eq!(command(Color::White, Color::White), None);
        assert_eq!(command(Color::Black, Color::White), None);
        assert_eq!(command(red, Color::White), None);
        assert_eq!(command(Color::Black, red), None);
    }
}
