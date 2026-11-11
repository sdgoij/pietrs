//! A structured backend: basic blocks, arithmetic, and a dispatcher.
//!
//! A Piet program has no jumps. What it has is the direction pointer and a
//! codel chooser, so every branch is geometry: a block that can be left in more
//! than one direction, and corridors that lead where you want to go. Writing
//! that by hand does not scale, so this module compiles a small structured
//! language onto the same ladder the generated interpreter uses.
//!
//! The trick that keeps the layout simple is that **no block branches**. A block
//! computes the id of its successor and hands it to one shared dispatcher, which
//! is a linear chain of equality tests ending in the matching lane. `if` is not
//! a jump either: the condition is normalised to `0` or `1` and the successor is
//! selected arithmetically,
//!
//! ```text
//! next = otherwise + condition * (then - otherwise)
//! ```
//!
//! so the only control-flow geometry in the whole image is the dispatcher, which
//! already exists and is already tested.
//!
//! ```text
//!   S [entry id] H [pop][test 1]…[test n] [halt]
//!   |            |    |          |
//!   |            |    +- lane 1 --+     one lane per block, each a
//!   |            |    +- lane 2 --+     straight descent to the lane row
//!   |            |    …
//!   |            +------ ascent ------+ the shared return path
//!   +------------------- lane ---------+
//! ```
//!
//! Every block is one lane. The lane pops the zero the dispatcher left behind,
//! runs the block, pushes the successor id, and returns to the head.

use canvas::{Blocks, CodelGrid, Color, Point};
use tessera::Command;

use crate::error::Error;
use crate::paint::Canvas;
use crate::shade::{Shade, neutral_filler};

/// Padding between the last test and the halt branch.
const HALT_FILL: u32 = 13;
/// Extra lane rows tried beyond the longest block, for the filler's length.
const LANE_SLACK: u32 = 8;
/// The shortest lane row that still leaves room for the halt descent.
const MIN_LANE: u32 = 8;

/// One stack operation in a basic block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// Push a constant.
    Const(i64),
    /// Run a command on the stack.
    Command(Command),
    /// Duplicate the top value.
    Dup,
    /// Swap the top two values.
    Swap,
    /// Rotate the top `depth` values right by `rolls`.
    Roll(u32, u32),
    /// Duplicate the value `depth` slots down, counting the top as one.
    DupLocal(u32),
}

/// How a block leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Push the target's id and dispatch to it.
    Jump(u32),
    /// The block has already left the successor id on the stack; dispatch to it.
    ///
    /// This is what lets one block serve many successors: a handler table laid
    /// out contiguously can be selected arithmetically from the command code.
    Computed,
    /// Pop a condition and dispatch to `then` when it is non-zero.
    If { then: u32, otherwise: u32 },
    /// Push an id no block has, which falls through the dispatcher and stops.
    Halt,
}

/// A basic block: straight-line nodes and one exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub nodes: Vec<Node>,
    pub end: End,
}

impl Block {
    pub fn new(nodes: impl Into<Vec<Node>>, end: End) -> Self {
        Self {
            nodes: nodes.into(),
            end,
        }
    }
}

/// A program: blocks numbered from one, and the block to start at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    blocks: Vec<Block>,
    entry: u32,
}

impl Program {
    pub fn new(blocks: Vec<Block>, entry: u32) -> Result<Self, Error> {
        let program = Self { blocks, entry };
        program.check()?;
        Ok(program)
    }

    /// The block ids this program may name.
    pub fn count(&self) -> u32 {
        self.blocks.len() as u32
    }

    fn check(&self) -> Result<(), Error> {
        let count = self.count();
        if count == 0 {
            return Err(Error::EmptyProgram);
        }
        let check = |id: u32| {
            if id == 0 || id > count {
                return Err(Error::BlockId { id, count });
            }
            Ok(())
        };
        check(self.entry)?;
        for block in &self.blocks {
            match block.end {
                End::Jump(id) => check(id)?,
                End::Computed => {}
                End::If { then, otherwise } => {
                    check(then)?;
                    check(otherwise)?;
                }
                End::Halt => {}
            }
            for node in &block.nodes {
                if let Node::DupLocal(depth) = node
                    && *depth == 0
                {
                    return Err(Error::LocalDepth);
                }
            }
        }
        Ok(())
    }

    /// Lays the program out as an image.
    pub fn image(&self) -> Result<CodelGrid, Error> {
        let bodies = self.bodies();
        let bus = self.bus()?;
        let prologue = constant(i64::from(self.entry));
        let geometry = Geometry::new(prologue.len() as u32, bodies.len() as u32);
        let longest = bodies.iter().map(Vec::len).max().unwrap_or(0) as u32;
        let shortest = MIN_LANE.max(longest + 2);

        for start in Shade::ALL {
            for lane in Shade::ALL {
                for lane_row in shortest..=shortest + LANE_SLACK {
                    if let Ok(grid) =
                        self.build(&bodies, &bus, &prologue, &geometry, start, lane, lane_row)
                        && geometry.is_clean(&grid, lane_row)
                    {
                        return Ok(grid);
                    }
                }
            }
        }
        Err(Error::NoLayout)
    }

    /// One command list per block: pop the dispatcher's zero, run the block,
    /// push the successor.
    fn bodies(&self) -> Vec<Vec<Command>> {
        self.blocks
            .iter()
            .map(|block| {
                let mut commands = vec![Command::Pop];
                for node in &block.nodes {
                    node_commands(*node, &mut commands);
                }
                end_commands(block.end, &mut commands);
                commands
            })
            .collect()
    }

    /// The dispatcher: the leading pop, one five-cell test per block, the
    /// fall-through filler, and the halt branch.
    fn bus(&self) -> Result<Vec<Command>, Error> {
        let mut bus = vec![Command::Pop];
        for _ in &self.blocks {
            bus.extend([
                Command::Push,
                Command::Subtract,
                Command::Duplicate,
                Command::Not,
                Command::Pointer,
            ]);
        }
        bus.extend(neutral_filler(
            Shade::new(0, 0),
            Shade::new(0, 0),
            HALT_FILL as usize,
        )?);
        bus.extend([Command::Push, Command::Pointer]);
        Ok(bus)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        bodies: &[Vec<Command>],
        bus: &[Command],
        prologue: &[Command],
        geometry: &Geometry,
        start: Shade,
        lane: Shade,
        lane_row: u32,
    ) -> Result<CodelGrid, Error> {
        // The entry id grows on the way to the head; the head is entered with a
        // pushed one that the dispatcher's first pop removes.
        let mut shade = start;
        let mut prologue_shades = Vec::with_capacity(prologue.len());
        for command in prologue {
            shade = shade.advance(*command);
            prologue_shades.push(shade);
        }
        let last = shade;
        let head = last.advance(Command::Push);

        let mut shade = head;
        let mut bus_shades = Vec::with_capacity(bus.len());
        for command in bus {
            shade = shade.advance(*command);
            bus_shades.push(shade);
        }

        // The return path mirrors the entry: one pushed value cancelled by one
        // pop, then neutral filler to the colour the head is entered from.
        let above_lane = lane.before(Command::Push);
        let first_ascent = lane.advance(Command::Pop);
        let ascent = neutral_filler(first_ascent, last, (lane_row - 1) as usize)?;

        let mut canvas = Canvas::new(geometry.width, lane_row + 1)?;
        canvas.put(0, 0, start.color())?;
        for (offset, shade) in prologue_shades.iter().enumerate() {
            canvas.put(1 + offset as u32, 0, shade.color())?;
        }
        canvas.put(geometry.head_x, 0, head.color())?;
        for (offset, shade) in bus_shades.iter().enumerate() {
            canvas.put(geometry.head_x + 1 + offset as u32, 0, shade.color())?;
        }

        for (index, body) in bodies.iter().enumerate() {
            let block = index as u32 + 1;
            let column = geometry.pointer_x(block);
            let mut shade = bus_shades[geometry.pointer_index(block)];
            for (offset, command) in body.iter().enumerate() {
                shade = shade.advance(*command);
                canvas.put(column, 1 + offset as u32, shade.color())?;
            }
            let filler = neutral_filler(
                shade,
                above_lane,
                (lane_row - 1 - body.len() as u32) as usize,
            )?;
            for (offset, command) in filler.iter().enumerate() {
                shade = shade.advance(*command);
                canvas.put(column, 1 + body.len() as u32 + offset as u32, shade.color())?;
            }
        }

        canvas.put_block(
            geometry.head_x + 1,
            lane_row,
            geometry.width - geometry.head_x - 1,
            1,
            lane.color(),
        )?;
        canvas.put(geometry.head_x, lane_row, first_ascent.color())?;
        let mut shade = first_ascent;
        for (offset, command) in ascent.iter().enumerate() {
            shade = shade.advance(*command);
            canvas.put(geometry.head_x, lane_row - 1 - offset as u32, shade.color())?;
        }

        let mut shade = bus_shades[geometry.halt_pointer_index];
        shade = shade.advance(Command::Pop);
        canvas.put(geometry.halt_pointer_x, 1, shade.color())?;
        let filler = neutral_filler(shade, shade, (lane_row - 4) as usize)?;
        for (offset, command) in filler.iter().enumerate() {
            shade = shade.advance(*command);
            canvas.put(geometry.halt_pointer_x, 2 + offset as u32, shade.color())?;
        }
        let bar = shade.advance(Command::Push);
        canvas.put_block(geometry.halt_pointer_x - 1, lane_row - 2, 3, 1, bar.color())?;

        canvas.into_grid()
    }
}

/// The geometry of one layout: every column the image needs.
struct Geometry {
    head_x: u32,
    halt_pointer_index: usize,
    halt_pointer_x: u32,
    width: u32,
}

impl Geometry {
    fn new(prologue: u32, blocks: u32) -> Self {
        let head_x = 1 + prologue;
        // Cell i after the head is at `head_x + 1 + i`: the leading pop, then
        // five cells per block.
        let halt_pointer_index = (5 * blocks + HALT_FILL + 2) as usize;
        let halt_pointer_x = head_x + 1 + halt_pointer_index as u32;
        Self {
            head_x,
            halt_pointer_index,
            halt_pointer_x,
            width: halt_pointer_x + 4,
        }
    }

    /// The x of a block's `pointer` cell, which is also its lane's column.
    fn pointer_x(&self, block: u32) -> u32 {
        self.head_x + 1 + 5 * block
    }

    /// The bus index of a block's `pointer` cell.
    fn pointer_index(&self, block: u32) -> usize {
        (5 * block) as usize
    }

    /// Whether every painted block is the size the layout expects: one codel
    /// everywhere, except the shared lane and the terminator bar.
    fn is_clean(&self, grid: &CodelGrid, lane_row: u32) -> bool {
        let blocks = Blocks::partition(grid.clone());
        for y in 0..grid.height() {
            for x in 0..grid.width() {
                let point = Point::new(x, y);
                let Some(color) = grid.get(point) else {
                    continue;
                };
                if color == Color::Black {
                    continue;
                }
                let expected = if y == lane_row && x > self.head_x {
                    (self.width - self.head_x - 1) as usize
                } else if y + 2 == lane_row
                    && (self.halt_pointer_x - 1..=self.halt_pointer_x + 1).contains(&x)
                {
                    3
                } else {
                    1
                };
                let Some(block) = blocks.block_at(point) else {
                    return false;
                };
                if block.size() != expected {
                    return false;
                }
            }
        }
        true
    }
}

/// The commands that build a node's value or effect.
fn node_commands(node: Node, commands: &mut Vec<Command>) {
    match node {
        Node::Const(value) => commands.extend(constant(value)),
        Node::Command(command) => commands.push(command),
        Node::Dup => commands.push(Command::Duplicate),
        Node::Swap => commands.extend(roll(2, 1)),
        Node::Roll(depth, rolls) => commands.extend(roll(depth, rolls)),
        // Reading the top is the one case that needs no roll at all.
        Node::DupLocal(1) => commands.push(Command::Duplicate),
        Node::DupLocal(depth) => {
            commands.extend(roll(depth, depth - 1));
            commands.push(Command::Duplicate);
            commands.extend(roll(depth + 1, 1));
        }
    }
}

/// The commands that leave the successor id on the stack.
fn end_commands(end: End, commands: &mut Vec<Command>) {
    match end {
        End::Jump(id) => commands.extend(constant(i64::from(id))),
        End::Computed => {}
        End::Halt => commands.extend(constant(0)),
        End::If { then, otherwise } => {
            // m is one when the condition is non-zero. The difference is always
            // positive, so no constant has to be negative.
            commands.extend([Command::Not, Command::Not]);
            if then >= otherwise {
                commands.extend(constant(i64::from(then - otherwise)));
                commands.extend([Command::Multiply]);
                commands.extend(constant(i64::from(otherwise)));
            } else {
                commands.push(Command::Not);
                commands.extend(constant(i64::from(otherwise - then)));
                commands.extend([Command::Multiply]);
                commands.extend(constant(i64::from(then)));
            }
            commands.push(Command::Add);
        }
    }
}

/// `roll` with both operands built for it.
fn roll(depth: u32, rolls: u32) -> Vec<Command> {
    let mut commands = constant(i64::from(depth));
    commands.extend(constant(i64::from(rolls)));
    commands.push(Command::Roll);
    commands
}

/// A constant, built by doubling: one push, then a double and a conditional
/// add per remaining bit.
fn constant(value: i64) -> Vec<Command> {
    let mut commands = Vec::new();
    if value == 0 {
        commands.extend([Command::Push, Command::Not]);
        return commands;
    }
    let magnitude = value.unsigned_abs();
    let bits = u64::BITS - magnitude.leading_zeros();
    commands.push(Command::Push);
    for bit in (0..bits - 1).rev() {
        commands.extend([Command::Duplicate, Command::Add]);
        if magnitude >> bit & 1 == 1 {
            commands.extend([Command::Push, Command::Add]);
        }
    }
    if value < 0 {
        // 0 - x needs the zero underneath: push it, swap, subtract.
        commands.extend(constant(0));
        commands.extend(roll(2, 1));
        commands.push(Command::Subtract);
    }
    commands
}
