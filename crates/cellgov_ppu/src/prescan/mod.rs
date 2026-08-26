//! Static pre-execution decode scan.
//!
//! Walks a slice of guest-text instruction words through [`decode`]
//! and accumulates the encodings the decoder rejects, deduped by
//! [`Locator`] and reported in `GapKey` order. It sees only the code
//! the caller hands it; the runtime [`PpuDecodeError`] path covers
//! what the scan cannot reach (runtime PRX loads, computed-target
//! jumps, self-modifying writes).
//!
//! [`decode`]: crate::decode::decode
//! [`Locator`]: crate::instruction::Locator
//! [`PpuDecodeError`]: crate::instruction::PpuDecodeError

mod error;
mod scan;
mod sections;

pub use error::PrescanError;
pub use scan::{
    scan_be_bytes, scan_elf_text, scan_words, CoverageMode, ElfTextCoverage, PrescanGap,
    PrescanReport,
};
pub use sections::executable_progbits_ranges;
