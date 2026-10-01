//! The RPCS3 TTY log as a CGOV frame source.
//!
//! The test writes its CGOV frame via `sys_tty_write`, so the frame
//! sits in RPCS3's TTY log among other output. The frame format and its
//! parser are [`cellgov_observation::frame`]'s; this module reads the
//! log and states the parser's refusals as [`Rpcs3Error`]s.

use std::path::Path;

use cellgov_observation::frame::{parse_frame, FrameError};

use crate::observation::NamedMemoryRegion;

use super::config::TtyRegion;
use super::error::Rpcs3Error;

/// Magic tag that precedes the big-endian u32 length and payload bytes.
pub use cellgov_observation::frame::FRAME_MAGIC as TTY_MAGIC;

impl From<FrameError> for Rpcs3Error {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::MagicNotFound => Self::TtyMagicNotFound,
            FrameError::Ambiguous { first, second } => Self::TtyFrameAmbiguous { first, second },
            FrameError::PayloadTooSmall { expected, actual } => {
                Self::TtyPayloadTooSmall { expected, actual }
            }
            FrameError::OffsetOverflow {
                region_name,
                offset,
                size,
            } => Self::TtyOffsetOverflow {
                region_name,
                offset,
                size,
            },
        }
    }
}

/// Read the TTY log at `tty_path` and parse it with [`parse_tty_frame`].
///
/// # Errors
///
/// [`Rpcs3Error::TtyRead`] when the file cannot be read, and every
/// [`parse_tty_frame`] refusal.
pub fn parse_tty_log(
    tty_path: &Path,
    regions: &[TtyRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    let data = std::fs::read(tty_path).map_err(Rpcs3Error::TtyRead)?;
    parse_tty_frame(&data, regions)
}

/// Scan `data`, a TTY log or a bare frame already in memory, for the
/// `CGOV` frame and slice each declared region out of its payload at
/// that region's offset.
///
/// # Errors
///
/// Returns `Err` when the bytes carry no frame, carry a second frame
/// past the first one's payload, are truncated inside the header or
/// payload, or declare a region the payload cannot satisfy.
pub fn parse_tty_frame(
    data: &[u8],
    regions: &[TtyRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    Ok(parse_frame(data, regions)?)
}

#[cfg(test)]
#[path = "tests/tty_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/tty_frame_tests.rs"]
mod tty_frame_tests;
