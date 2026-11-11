//! What the weaver can refuse to build.

use tessera::Command;

/// Why an image could not be woven.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cell ({x}, {y}) is outside the {width}x{height} canvas")]
    OutOfBounds {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },

    #[error("cell ({x}, {y}) was already painted a different colour")]
    Overlap { x: u32, y: u32 },

    #[error("the canvas is too large")]
    TooLarge,

    #[error("a block cannot have zero codels")]
    EmptyBlock,

    #[error("a push operand of {operand} does not fit in a block")]
    BlockTooLarge { operand: u64 },

    #[error("no stack-neutral filler of {length} commands reaches the target colour")]
    NoFiller { length: usize },

    #[error("the command {0:?} changes control flow and cannot be interpreted")]
    Unsupported(Command),

    #[error("the program is not linear: it runs through white")]
    NotLinear,

    #[error("the interpreted program needs input the stream does not carry")]
    InputExhausted,

    #[error("no layout keeps every block separate")]
    NoLayout,

    #[error(transparent)]
    Canvas(#[from] canvas::Error),

    #[error(transparent)]
    Interp(#[from] interp::Error),
}
