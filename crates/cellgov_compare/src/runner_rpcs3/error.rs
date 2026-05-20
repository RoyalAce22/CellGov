//! Failure modes for the RPCS3 runner.

use std::io;

/// Why an RPCS3 run failed.
#[derive(Debug)]
pub enum Rpcs3Error {
    /// The RPCS3 process could not be started.
    Launch(io::Error),
    /// The memory dump file could not be read.
    DumpRead(io::Error),
    /// A declared dump region extends past the file end. Carries the
    /// region's identity so the operator can fix the manifest without
    /// re-deriving which region failed from raw byte counts.
    DumpTooSmall {
        /// Region name from the dump manifest.
        region_name: String,
        /// Guest address the region was declared at.
        guest_addr: u64,
        /// Minimum file size required to satisfy this region.
        expected: u64,
        /// Actual file size.
        actual: u64,
    },
    /// Dump-manifest offset / size arithmetic overflows `u64`. Reaches
    /// here only on hand-crafted manifests; real dumps are well below
    /// `u64::MAX`.
    DumpOffsetOverflow {
        region_name: String,
        offset: u64,
        size: u64,
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
    /// TTY-manifest offset / size arithmetic overflows `u64`.
    TtyOffsetOverflow { region_name: String, size: u64 },
}

impl std::fmt::Display for Rpcs3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Launch(e) => write!(f, "rpcs3 launch: {e}"),
            Self::DumpRead(e) => write!(f, "rpcs3 dump read: {e}"),
            Self::DumpTooSmall {
                region_name,
                guest_addr,
                expected,
                actual,
            } => write!(
                f,
                "rpcs3 dump too small for region {region_name:?} at 0x{guest_addr:016x}: expected >= {expected} bytes, got {actual}"
            ),
            Self::DumpOffsetOverflow {
                region_name,
                offset,
                size,
            } => write!(
                f,
                "rpcs3 dump offset/size overflow for region {region_name:?}: offset={offset}, size={size}"
            ),
            Self::TtyRead(e) => write!(f, "rpcs3 tty read: {e}"),
            Self::TtyMagicNotFound => f.write_str("rpcs3 tty: magic tag not found"),
            Self::TtyPayloadTooSmall { expected, actual } => write!(
                f,
                "rpcs3 tty payload too small: expected >= {expected} bytes, got {actual}"
            ),
            Self::TtyOffsetOverflow { region_name, size } => write!(
                f,
                "rpcs3 tty offset/size overflow for region {region_name:?}: size={size}"
            ),
        }
    }
}

impl std::error::Error for Rpcs3Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Launch(e) | Self::DumpRead(e) | Self::TtyRead(e) => Some(e),
            _ => None,
        }
    }
}
