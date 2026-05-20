//! Named memory region snapshots taken at end of run.

use serde::{Deserialize, Serialize};

/// Region name for the title's `code` PT_LOAD. Single source of
/// truth for callers that look up the ELF-bearing region.
pub const CODE_REGION_NAME: &str = "code";

/// A named memory region snapshot taken at end of run.
///
/// All observed regions must be test-owned and write-complete: the test
/// allocates the region, fully initializes it to a known value, and
/// writes its result before terminating. Comparison must not depend on
/// uninitialized or partially-written memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedMemoryRegion {
    /// Region name from the manifest.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Raw bytes captured at end of run.
    pub data: Vec<u8>,
}
