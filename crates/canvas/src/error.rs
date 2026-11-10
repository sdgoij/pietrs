use thiserror::Error;

/// Why a canvas could not be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    #[error("the image is empty")]
    EmptyImage,

    #[error("the image is too large to decode")]
    ImageTooLarge,

    #[error("the RGBA buffer is {actual} bytes, expected {expected}")]
    BufferSize { expected: usize, actual: usize },

    #[error("the codel grid needs {expected} codels, got {actual}")]
    CodelCount { expected: usize, actual: usize },

    #[error("codel size {codel_size} does not tile a {width}x{height} image")]
    CodelSize {
        width: u32,
        height: u32,
        codel_size: u32,
    },

    #[error("codel ({x}, {y}) is not a single colour")]
    NonMonochromeCodel { x: u32, y: u32 },

    #[error("PNG decoding failed: {0}")]
    Png(String),

    #[error("GIF decoding failed: {0}")]
    Gif(String),

    #[error("the image is neither PNG nor GIF")]
    UnknownFormat,

    #[error("PNG bit depth {0} is not supported after normalisation")]
    UnsupportedBitDepth(String),

    #[error("indexed PNGs were not expanded by the decoder")]
    IndexedNotExpanded,
}
