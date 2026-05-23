//! Failure modes for the RPCS3 runner.

use std::io;

/// Why an RPCS3 run failed.
#[derive(Debug, thiserror::Error)]
pub enum Rpcs3Error {
    /// The RPCS3 process could not be started.
    #[error("rpcs3 launch: {0}")]
    Launch(#[source] io::Error),
    /// The memory dump file could not be read.
    #[error("rpcs3 dump read: {0}")]
    DumpRead(#[source] io::Error),
    /// A declared dump region extends past the file end. Carries the
    /// region's identity so the operator can fix the manifest without
    /// re-deriving which region failed from raw byte counts.
    #[error("rpcs3 dump too small for region {region_name:?} at 0x{guest_addr:016x}: expected >= {expected} bytes, got {actual}")]
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
    #[error(
        "rpcs3 dump offset/size overflow for region {region_name:?}: offset={offset}, size={size}"
    )]
    DumpOffsetOverflow {
        /// Region name from the dump manifest.
        region_name: String,
        /// File offset declared for the region.
        offset: u64,
        /// Region byte length declared in the manifest.
        size: u64,
    },
    /// The TTY log file could not be read.
    #[error("rpcs3 tty read: {0}")]
    TtyRead(#[source] io::Error),
    /// The TTY log does not contain the expected magic tag.
    #[error("rpcs3 tty: magic tag not found")]
    TtyMagicNotFound,
    /// The TTY payload is shorter than declared regions require.
    #[error("rpcs3 tty payload too small: expected >= {expected} bytes, got {actual}")]
    TtyPayloadTooSmall {
        /// Minimum payload size required by declared regions.
        expected: u64,
        /// Actual payload size.
        actual: u64,
    },
    /// TTY-manifest offset / size arithmetic overflows `u64`.
    #[error("rpcs3 tty offset/size overflow for region {region_name:?}: size={size}")]
    TtyOffsetOverflow {
        /// Region name from the TTY manifest.
        region_name: String,
        /// Region byte length declared in the manifest.
        size: u64,
    },
}
