//! Archive context for the syscalls a run reached and the model does
//! not answer.

use cellgov_ps3_abi::lv2::census::{lookup, PupCensusClass};

use super::caller::CallerRow;
use super::census::GateRow;
use super::name::NameRow;

/// One unsupported syscall, joined with what the archive holds for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmodelledSyscall<'a> {
    /// The syscall ordinal.
    pub ordinal: u64,
    /// How many times the run reached it.
    pub hits: u64,
    /// Every name a source gives the whole ordinal.
    pub names: Vec<&'a NameRow>,
    /// The kernel census class for the run's PUP.
    pub census: PupCensusClass,
    /// The capability-gate row the archive holds for the run's PUP, if any.
    pub gate: Option<&'a GateRow>,
    /// The firmware modules whose sites load the ordinal.
    /// [`unmodelled_syscalls`] fills this only when the run's PUP has no
    /// kernel census; the rows are then other evidence that the firmware
    /// uses the ordinal.
    pub callers: Vec<&'a CallerRow>,
}

/// Join each `(ordinal, hits)` pair with the archive's name, gate,
/// census and caller rows for the run's PUP.
///
/// With no PUP identity, the census class is
/// [`PupCensusClass::NotExtracted`] and no gate or caller row matches.
pub fn unmodelled_syscalls<'a>(
    unsupported: impl IntoIterator<Item = (u64, u64)>,
    pup_sha256: Option<&[u8; 32]>,
    names: &'a [NameRow],
    gates: &'a [GateRow],
    callers: &'a [CallerRow],
) -> Vec<UnmodelledSyscall<'a>> {
    let pup = pup_sha256.map(pup_hex);
    unsupported
        .into_iter()
        .map(|(ordinal, hits)| {
            let ordinal_index = usize::try_from(ordinal).ok();
            let census = pup_sha256.map_or(PupCensusClass::NotExtracted, |pup_sha256| {
                ordinal_index.map_or(PupCensusClass::OutOfRange, |index| {
                    lookup(pup_sha256, index)
                })
            });
            let gate = gates.iter().find(|row| {
                pup.as_deref().is_some_and(|pup| row.pup_sha256 == pup)
                    && Some(row.ordinal) == ordinal_index
            });
            let callers = match (&pup, census) {
                (Some(pup), PupCensusClass::NotExtracted) => callers
                    .iter()
                    .filter(|row| row.pup_sha256 == *pup && Some(row.ordinal) == ordinal_index)
                    .collect(),
                _ => Vec::new(),
            };
            UnmodelledSyscall {
                ordinal,
                hits,
                names: names
                    .iter()
                    .filter(|row| row.ordinal == ordinal && row.packet.is_none())
                    .collect(),
                census,
                gate,
                callers,
            }
        })
        .collect()
}

/// Whether [`unmodelled_syscalls`] reads caller rows for a run under
/// `pup_sha256`: only a PUP with no kernel census takes caller
/// evidence, so a caller may skip loading `caller.tsv` otherwise.
pub fn takes_caller_evidence(pup_sha256: Option<&[u8; 32]>) -> bool {
    pup_sha256.is_some_and(|pup_sha256| lookup(pup_sha256, 0) == PupCensusClass::NotExtracted)
}

/// The archive's label for a kernel census class.
pub fn census_class_label(class: PupCensusClass) -> &'static str {
    match class {
        PupCensusClass::Implemented => "implemented",
        PupCensusClass::Stub => "stub",
        PupCensusClass::Absent => "absent",
        PupCensusClass::NotExtracted => "not_extracted",
        PupCensusClass::OutOfRange => "out_of_range",
    }
}

/// A PUP digest in the archive's key spelling: 64 lowercase hex digits.
fn pup_hex(pup_sha256: &[u8; 32]) -> String {
    pup_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "tests/unmodelled_tests.rs"]
mod tests;
