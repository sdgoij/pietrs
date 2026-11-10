//! The golden trace format: one line per transition.

use std::fmt::Write as _;

use tessera::{Chooser, Command, Direction};

/// One parsed trace line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceLine {
    pub block: u32,
    pub dp: Direction,
    pub cc: Chooser,
    pub command: Option<Command>,
    pub target: Option<u32>,
}

/// Parses the line-oriented trace format.
pub fn parse_trace(text: &str) -> Result<Vec<TraceLine>, String> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(parse_line)
        .collect()
}

/// Formats parsed lines back into the trace format, with one newline per line.
pub fn format_trace(lines: &[TraceLine]) -> String {
    let mut text = String::new();
    for line in lines {
        write!(
            text,
            "block={} dp={:?} cc={:?} command=",
            line.block, line.dp, line.cc
        )
        .unwrap();
        match line.command {
            Some(command) => write!(text, "{command:?}").unwrap(),
            None => text.push_str("none"),
        }
        text.push_str(" target=");
        match line.target {
            Some(target) => write!(text, "{target}").unwrap(),
            None => text.push_str("none"),
        }
        text.push('\n');
    }
    text
}

fn parse_line(line: &str) -> Result<TraceLine, String> {
    let mut fields = line.split_whitespace();
    let block = field(&mut fields, "block", line)?
        .parse()
        .map_err(|error| format!("invalid block in {line:?}: {error}"))?;
    let dp = match field(&mut fields, "dp", line)? {
        "Right" => Direction::Right,
        "Down" => Direction::Down,
        "Left" => Direction::Left,
        "Up" => Direction::Up,
        other => return Err(format!("unknown dp {other:?} in {line:?}")),
    };
    let cc = match field(&mut fields, "cc", line)? {
        "Left" => Chooser::Left,
        "Right" => Chooser::Right,
        other => return Err(format!("unknown cc {other:?} in {line:?}")),
    };
    let command = match field(&mut fields, "command", line)? {
        "none" => None,
        name => Some(
            command_from_name(name)
                .ok_or_else(|| format!("unknown command {name:?} in {line:?}"))?,
        ),
    };
    let target = match field(&mut fields, "target", line)? {
        "none" => None,
        text => Some(
            text.parse()
                .map_err(|error| format!("invalid target in {line:?}: {error}"))?,
        ),
    };
    if fields.next().is_some() {
        return Err(format!("trailing fields in {line:?}"));
    }
    Ok(TraceLine {
        block,
        dp,
        cc,
        command,
        target,
    })
}

fn field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    key: &str,
    line: &str,
) -> Result<&'a str, String> {
    let token = fields
        .next()
        .ok_or_else(|| format!("missing {key} in {line:?}"))?;
    token
        .strip_prefix(key)
        .and_then(|rest| rest.strip_prefix('='))
        .ok_or_else(|| format!("expected {key}=... in {line:?}"))
}

fn command_from_name(name: &str) -> Option<Command> {
    Some(match name {
        "Push" => Command::Push,
        "Pop" => Command::Pop,
        "Add" => Command::Add,
        "Subtract" => Command::Subtract,
        "Multiply" => Command::Multiply,
        "Divide" => Command::Divide,
        "Modulo" => Command::Modulo,
        "Not" => Command::Not,
        "Greater" => Command::Greater,
        "Pointer" => Command::Pointer,
        "Switch" => Command::Switch,
        "Duplicate" => Command::Duplicate,
        "Roll" => Command::Roll,
        "InNumber" => Command::InNumber,
        "InChar" => Command::InChar,
        "OutNumber" => Command::OutNumber,
        "OutChar" => Command::OutChar,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_round_trips() {
        let text = concat!(
            "block=0 dp=Right cc=Left command=Push target=1\n",
            "block=1 dp=Down cc=Right command=none target=none\n",
        );
        let lines = parse_trace(text).unwrap();
        assert_eq!(
            lines[0],
            TraceLine {
                block: 0,
                dp: Direction::Right,
                cc: Chooser::Left,
                command: Some(Command::Push),
                target: Some(1),
            }
        );
        assert_eq!(format_trace(&lines), text);
    }

    #[test]
    fn an_empty_trace_round_trips() {
        assert!(parse_trace("").unwrap().is_empty());
        assert_eq!(format_trace(&[]), "");
    }

    #[test]
    fn a_bad_line_is_a_diagnostic() {
        assert!(parse_trace("block=0 dp=Sideways cc=Left command=none target=none").is_err());
        assert!(parse_trace("block=0 dp=Right cc=Left command=Fly target=none").is_err());
        assert!(parse_trace("block=0 dp=Right cc=Left command=none").is_err());
        assert!(parse_trace("block=0 dp=Right cc=Left command=none target=none extra").is_err());
    }
}
