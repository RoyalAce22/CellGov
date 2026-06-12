//! RPCS3 runner configuration types: installation, per-test settings,
//! and region descriptors.

use std::path::PathBuf;
use std::time::Duration;

/// RPCS3 installation and global settings.
#[derive(Debug, Clone)]
pub struct Rpcs3Config {
    /// Path to the rpcs3 executable.
    pub executable: PathBuf,
    /// Decoder mode for the test run.
    pub decoder: Rpcs3Decoder,
}

/// Which RPCS3 decoder combination to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
pub enum Rpcs3Decoder {
    /// PPU Interpreter + SPU Interpreter.
    Interpreter,
    /// PPU LLVM + SPU LLVM.
    Llvm,
}

impl Rpcs3Decoder {
    /// Runner-name fragment used in `Observation::metadata.runner`.
    pub fn as_runner_str(self) -> &'static str {
        match self {
            Self::Interpreter => "rpcs3-interpreter",
            Self::Llvm => "rpcs3-llvm",
        }
    }
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;

/// How to extract the result buffer from RPCS3 after a test run.
#[derive(Debug, Clone)]
pub enum ExtractionMethod {
    /// Read regions at byte offsets within a binary memory dump file.
    DumpFile {
        /// Path to the dump file.
        path: PathBuf,
        /// Regions to extract from the dump.
        regions: Vec<DumpRegion>,
    },
    /// Scan the TTY log for the `CGOV` frame and slice regions from its
    /// payload in declaration order.
    TtyLog {
        /// Path to RPCS3's TTY.log.
        path: PathBuf,
        /// Regions to extract from the CGOV payload.
        regions: Vec<TtyRegion>,
    },
}

/// Per-test configuration for an RPCS3 run.
#[derive(Debug, Clone)]
pub struct Rpcs3TestConfig {
    /// Path to the ELF binary to execute.
    pub binary: PathBuf,
    /// Wall-clock timeout for the RPCS3 process.
    pub timeout: Duration,
    /// How to extract the result buffer after the run.
    pub extraction: ExtractionMethod,
}

/// A region within a binary memory dump file.
#[derive(Debug, Clone)]
pub struct DumpRegion {
    /// Region name.
    pub name: String,
    /// Byte offset within the dump file.
    pub offset: u64,
    /// Number of bytes to read.
    pub size: u64,
    /// Guest address to report in the observation.
    pub guest_addr: u64,
}

/// A region to extract from the TTY payload; regions are packed
/// contiguously in declaration order.
#[derive(Debug, Clone)]
pub struct TtyRegion {
    /// Region name.
    pub name: String,
    /// Number of bytes for this region within the payload.
    pub size: u64,
    /// Guest address to report in the observation.
    pub guest_addr: u64,
}
