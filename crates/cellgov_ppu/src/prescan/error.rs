//! Typed errors surfaced by [`super::scan_elf_text`].

use crate::loader::LoadError;

/// Why the ELF-text walk could not run. Distinct from gap-report
/// content: an `Err` here means the input wasn't a parseable PPU ELF
/// at all (or its program-header table was malformed), so the scan
/// could not begin. A valid ELF with zero PF_X segments returns an
/// `Ok` empty report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrescanError {
    /// Loader could not parse the ELF program-header table; the
    /// inner error carries the specific reason.
    #[error("prescan: ELF parse failed: {0}")]
    Loader(#[from] LoadError),
    /// Section-header table is present (`e_shoff != 0`) but
    /// malformed: undersized entries, out-of-range slot, or a
    /// section whose `sh_offset + sh_size` exceeds the file. A
    /// stripped binary (`e_shoff == 0`) does NOT take this path --
    /// it routes through the segment-walk fallback.
    #[error("prescan: ELF section-header table malformed")]
    MalformedSectionTable,
}
