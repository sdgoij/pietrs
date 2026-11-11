//! The eighteen chromatic colours as a group, and the neutral filler search.
//!
//! A transition between two colours runs one command, and the command is the
//! forward `(hue, lightness)` delta between them. Because the palette is a
//! cycle in each coordinate, a command is an invertible translation: from a
//! known colour it fixes the next one, and from a pair of colours it fixes the
//! command.
//!
//! The interpreter's return paths need *filler*: commands that walk the colour
//! cycle without disturbing the value stack. Every filler here is a sequence of
//! [`NEUTRAL`] macros, each of which leaves the stack exactly as it found it.
//! [`neutral_filler`] searches those macros for the cheapest sequence whose
//! total colour delta reaches the colour the next block needs.

use std::collections::VecDeque;

use canvas::{Color, Hue, Lightness};
use tessera::Command;

use crate::error::Error;

/// A chromatic colour as `(hue, lightness)` cycle indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Shade {
    hue: u8,
    lightness: u8,
}

impl Shade {
    /// How many chromatic colours there are.
    pub(crate) const COUNT: usize = 18;

    /// The palette colour for a shade.
    pub(crate) const fn new(hue: u8, lightness: u8) -> Self {
        Self {
            hue: hue % 6,
            lightness: lightness % 3,
        }
    }

    /// Every shade, in palette order.
    pub(crate) const ALL: [Shade; Shade::COUNT] = all();

    /// The palette colour of this shade.
    pub(crate) fn color(self) -> Color {
        let hue = Hue::ALL[usize::from(self.hue)];
        let lightness = Lightness::ALL[usize::from(self.lightness)];
        Color::chromatic(hue, lightness)
    }

    /// The position of a shade in [`Shade::ALL`].
    fn index(self) -> usize {
        usize::from(self.hue) * 3 + usize::from(self.lightness)
    }

    fn from_index(index: usize) -> Self {
        Self::new((index / 3) as u8, (index % 3) as u8)
    }

    /// The colour a transition running `command` from this shade enters.
    pub(crate) fn advance(self, command: Command) -> Self {
        let (hue, lightness) = delta(command);
        Self::new(self.hue + hue, self.lightness + lightness)
    }

    /// The colour whose transition by `command` enters this shade.
    pub(crate) fn before(self, command: Command) -> Self {
        let (hue, lightness) = delta(command);
        Self::new(self.hue + 6 - hue, self.lightness + 3 - lightness)
    }
}

/// The forward `(hue, lightness)` deltas a command runs.
pub(crate) const fn delta(command: Command) -> (u8, u8) {
    use Command::{
        Add, Divide, Duplicate, Greater, InChar, InNumber, Modulo, Multiply, Not, OutChar,
        OutNumber, Pointer, Pop, Push, Roll, Subtract, Switch,
    };
    match command {
        Push => (0, 1),
        Pop => (0, 2),
        Add => (1, 0),
        Subtract => (1, 1),
        Multiply => (1, 2),
        Divide => (2, 0),
        Modulo => (2, 1),
        Not => (2, 2),
        Greater => (3, 0),
        Pointer => (3, 1),
        Switch => (3, 2),
        Duplicate => (4, 0),
        Roll => (4, 1),
        InNumber => (4, 2),
        InChar => (5, 0),
        OutNumber => (5, 1),
        OutChar => (5, 2),
    }
}

/// Command sequences that leave the value stack exactly as they found it.
///
/// `not` is unary and the arithmetic commands are binary, so the arithmetic
/// macros push their own operands and discard the result. `roll` with a depth
/// of one is an identity; `duplicate` copies the top and the trailing `pop`
/// removes the copy. `roll` pops its two operands and pushes nothing, so
/// pushing those two is enough to cancel it. An empty stack makes every one of
/// them a no-op.
const NEUTRAL: [&[Command]; 11] = [
    &[Command::Push, Command::Pop],
    &[Command::Duplicate, Command::Pop],
    &[Command::Push, Command::Not, Command::Pop],
    &[Command::Push, Command::Push, Command::Add, Command::Pop],
    &[
        Command::Push,
        Command::Push,
        Command::Subtract,
        Command::Pop,
    ],
    &[
        Command::Push,
        Command::Push,
        Command::Multiply,
        Command::Pop,
    ],
    &[Command::Push, Command::Push, Command::Divide, Command::Pop],
    &[Command::Push, Command::Push, Command::Modulo, Command::Pop],
    &[Command::Push, Command::Push, Command::Greater, Command::Pop],
    &[Command::Push, Command::Push, Command::Roll],
    &[
        Command::Push,
        Command::Not,
        Command::Not,
        Command::Not,
        Command::Pop,
    ],
];

/// Two neutral macros of length zero-delta: two and five commands.
const ZERO_SHORT: &[Command] = &[Command::Push, Command::Pop];
const ZERO_LONG: &[Command] = &[
    Command::Push,
    Command::Not,
    Command::Not,
    Command::Not,
    Command::Pop,
];

/// The shade reached by running `commands` from `shade`.
pub(crate) fn walk(mut shade: Shade, commands: &[Command]) -> Shade {
    for command in commands {
        shade = shade.advance(*command);
    }
    shade
}

/// A stack-neutral sequence of exactly `length` commands that runs from `from`
/// to `to`.
///
/// The search is over the eighteen shades; the padding at the end is a pair of
/// zero-delta macros of length two and five, so every length from the shortest
/// answer upwards is reachable except the next one and the next but two.
pub(crate) fn neutral_filler(from: Shade, to: Shade, length: usize) -> Result<Vec<Command>, Error> {
    let start = from.index();
    let target = to.index();
    let mut parent = [usize::MAX; Shade::COUNT];
    let mut through = [usize::MAX; Shade::COUNT];
    let mut seen = [false; Shade::COUNT];
    seen[start] = true;
    let mut queue = VecDeque::from([start]);

    while let Some(current) = queue.pop_front() {
        if current == target {
            break;
        }
        let shade = Shade::from_index(current);
        for (index, commands) in NEUTRAL.iter().enumerate() {
            let next = walk(shade, commands).index();
            if next == current || seen[next] {
                continue;
            }
            seen[next] = true;
            parent[next] = current;
            through[next] = index;
            queue.push_back(next);
        }
    }

    if !seen[target] {
        return Err(Error::NoFiller { length });
    }

    let mut macros = Vec::new();
    let mut node = target;
    while node != start {
        macros.push(through[node]);
        node = parent[node];
    }
    macros.reverse();

    let mut commands = Vec::new();
    for index in macros {
        if let Some(sequence) = NEUTRAL.get(index) {
            commands.extend_from_slice(sequence);
        }
    }
    if commands.len() > length || !pad(&mut commands, length) {
        return Err(Error::NoFiller { length });
    }
    Ok(commands)
}

/// Appends zero-delta macros until the sequence is `length` commands long.
fn pad(commands: &mut Vec<Command>, length: usize) -> bool {
    let need = length.saturating_sub(commands.len());
    let mut long = need / ZERO_LONG.len();
    loop {
        let rest = need - long * ZERO_LONG.len();
        if rest.is_multiple_of(ZERO_SHORT.len()) {
            for _ in 0..rest / ZERO_SHORT.len() {
                commands.extend_from_slice(ZERO_SHORT);
            }
            for _ in 0..long {
                commands.extend_from_slice(ZERO_LONG);
            }
            return true;
        }
        if long == 0 {
            return false;
        }
        long -= 1;
    }
}

const fn all() -> [Shade; Shade::COUNT] {
    let mut shades = [Shade {
        hue: 0,
        lightness: 0,
    }; Shade::COUNT];
    let mut index = 0;
    while index < Shade::COUNT {
        shades[index] = Shade {
            hue: (index / 3) as u8,
            lightness: (index % 3) as u8,
        };
        index += 1;
    }
    shades
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many values a command adds to the stack.
    const fn net_effect(command: Command) -> i32 {
        use Command::{
            Add, Divide, Duplicate, Greater, InChar, InNumber, Modulo, Multiply, Not, OutChar,
            OutNumber, Pointer, Pop, Push, Roll, Subtract, Switch,
        };
        match command {
            Push | Duplicate | InNumber | InChar => 1,
            Pop | OutNumber | OutChar | Pointer | Switch => -1,
            Add | Subtract | Multiply | Divide | Modulo | Greater => -1,
            Not => 0,
            Roll => -2,
        }
    }

    #[test]
    fn every_command_is_an_invertible_translation() {
        for shade in Shade::ALL {
            for command in [
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
            ] {
                assert_eq!(shade.advance(command).before(command), shade);
                assert_eq!(shade.before(command).advance(command), shade);
            }
        }
    }

    #[test]
    fn advancing_agrees_with_the_transition_table() {
        for shade in Shade::ALL {
            for command in [
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
            ] {
                let from = shade.color();
                let to = shade.advance(command).color();
                assert_eq!(tessera::command(from, to), Some(command));
            }
        }
    }

    #[test]
    fn every_neutral_macro_has_no_stack_effect() {
        for commands in NEUTRAL {
            let effect = commands
                .iter()
                .map(|command| net_effect(*command))
                .sum::<i32>();
            assert_eq!(effect, 0, "{commands:?} changes the stack depth");
        }
        let short = ZERO_SHORT
            .iter()
            .map(|command| net_effect(*command))
            .sum::<i32>();
        let long = ZERO_LONG
            .iter()
            .map(|command| net_effect(*command))
            .sum::<i32>();
        assert_eq!(short, 0);
        assert_eq!(long, 0);
    }

    #[test]
    fn the_zero_pads_return_to_their_colour() {
        assert_eq!(walk(Shade::new(0, 0), ZERO_SHORT), Shade::new(0, 0));
        assert_eq!(walk(Shade::new(0, 0), ZERO_LONG), Shade::new(0, 0));
    }

    #[test]
    fn a_filler_reaches_every_colour_at_the_requested_length() {
        for from in Shade::ALL {
            for to in Shade::ALL {
                let commands = neutral_filler(from, to, 24).expect("a filler exists");
                assert_eq!(commands.len(), 24);
                assert_eq!(walk(from, &commands), to);
            }
        }
    }

    #[test]
    fn a_filler_that_cannot_be_padded_is_reported() {
        assert!(neutral_filler(Shade::new(0, 0), Shade::new(5, 2), 1).is_err());
    }
}
