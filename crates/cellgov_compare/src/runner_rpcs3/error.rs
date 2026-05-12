//! Failure modes for the RPCS3 runner.

use std::io;

/// Why an RPCS3 run failed.
#[derive(Debug)]
pub enum Rpcs3Error {
    /// The RPCS3 process could not be started.
    Launch(io::Error),
    /// The RPCS3 process exceeded the wall-clock timeout.
    Timeout,
    /// The memory dump file could not be read.
    DumpRead(io::Error),
    /// The dump file is too small for the declared regions.
    DumpTooSmall {
        /// Minimum size required by declared regions.
        expected: u64,
        /// Actual file size.
        actual: u64,
    },
    /// The TTY log file could not be read.
    TtyRead(io::Error),
    /// The TTY log does not contain the expected magic tag.
    TtyMagicNotFound,
    /// The TTY payload is shorter than declared regions require.
    TtyPayloadTooSmall {
        /// Minimum payload size required by declared regions.
        expected: u64,
        /// Actual payload size.
        actual: u64,
    },
}
