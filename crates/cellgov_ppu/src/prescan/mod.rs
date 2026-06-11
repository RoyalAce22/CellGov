//! Static pre-execution decode scan.
//!
//! Walks a slice of guest-text instruction words through [`decode`]
//! and accumulates the encodings the decoder rejects, deduped by
//! [`Locator`]. The result is the load-time gap report: every
//! named-but-unimplemented and every unrecognized encoding the scan
//! reaches, with the names Tables 1 and 2 carry.
//!
//! The scan is the early-warning half of Phase 40 layer 2; it sees
//! only code the caller hands it. The runtime [`PpuDecodeError`]
//! path is the co-equal backstop for everything the scan cannot
//! reach -- runtime PRX loads, computed-target jumps, self-modifying
//! writes, dead-on-boot segments. A scan that reports "no gaps"
//! means "no gaps in the slice I walked," not "no gaps."
//!
//! Determinism: the scan deduplicates with `BTreeMap`, never
//! `HashMap`, and reports in `GapKey` order so output is
//! byte-identical between runs.
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
