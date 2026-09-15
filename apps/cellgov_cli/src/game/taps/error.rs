//! Why a debug watch the environment asks for cannot start.

use std::num::ParseIntError;
use std::path::PathBuf;

/// Why a debug watch the environment asks for cannot start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TapError {
    /// A variable holds a value that is not Unicode.
    #[error("{var} is set to a value that is not Unicode")]
    NotUnicode { var: &'static str },
    /// A number in a variable does not parse.
    #[error("{var}: cannot parse {token:?}: {source}")]
    BadNumber {
        var: &'static str,
        token: String,
        #[source]
        source: ParseIntError,
    },
    /// A variable's value does not have the shape the watch reads.
    #[error("{var}: expected {expected}, got {got:?}")]
    BadShape {
        var: &'static str,
        expected: &'static str,
        got: String,
    },
    /// A parsed number is outside the range the watch accepts.
    #[error("{var}: 0x{value:x} is outside {range}")]
    OutOfRange {
        var: &'static str,
        value: u64,
        range: &'static str,
    },
    /// One variable of a pair is set without the other.
    #[error("{set} is set but {missing} is not")]
    Unpaired {
        set: &'static str,
        missing: &'static str,
    },
    /// A raw-PC watch's on-wire ID equals a watched NID.
    #[error(
        "CELLGOV_HLE_RETURN_WATCH_PCS: PC 0x{pc:08x} takes on-wire ID 0x{id:08x}, \
         which CELLGOV_HLE_RETURN_WATCH also names as a NID"
    )]
    RawPcCollides {
        pc: u32,
        /// The synthetic on-wire ID of `pc`.
        id: u32,
    },
    /// Two raw-PC watches take one on-wire ID.
    #[error(
        "CELLGOV_HLE_RETURN_WATCH_PCS: PCs 0x{first:08x} and 0x{second:08x} both take \
         on-wire ID 0x{id:08x}"
    )]
    RawPcsCollide { first: u32, second: u32, id: u32 },
    /// Two watches name one capture file.
    #[error(
        "{first} and {second} both write {}; each capture needs its own file",
        path.display()
    )]
    SharedCapture {
        first: &'static str,
        second: &'static str,
        path: PathBuf,
    },
    /// The host refused to create the capture file or to write its header.
    #[error("{label}: cannot write {}: {source}", path.display())]
    Capture {
        /// The watch that owns the capture.
        label: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
