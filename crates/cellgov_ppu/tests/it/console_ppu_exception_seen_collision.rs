//! An invalid-form `lmw` or `lswi` whose RA lies in the load range, run in
//! a child thread, checked against the `ppu_exception_seen_lmw_ra`,
//! `ppu_exception_seen_lmw_ra_null` and `ppu_exception_seen_lswi_ra_null`
//! console captures.
//!
//! On the console, each probe ends the child thread at the instruction
//! and the main thread finishes, whether the words ahead of RA are mapped
//! or not. CellGov takes the illegal-instruction interrupt when they are
//! mapped and the data storage interrupt at the first unmapped byte when
//! they are not.

use crate::console_ppu_exception_seen::{payload, seen, Seen};
use cellgov_mem::{FaultContext, MemError};
use cellgov_ppu::differential::{execute_state_in, CaseOutcome, PpuStateSnapshot};
use cellgov_ppu::exec::{ExecuteVerdict, PpuFault};

/// `lmw r28,0(r30)`.
const LMW_R28_R30: u32 = 0xBB9E_0000;
/// `lswi r28,r30,16`.
const LSWI_R28_R30_16: u32 = 0x7F9E_84AA;

/// CellGov's verdict for `word` with r30 at the probe's access address.
/// A nonzero address holds the probe's 64-byte window of 0x10, 0x11, ...;
/// address 0 holds no byte.
fn cellgov(probe: &str, word: u32) -> Result<(), Box<CaseOutcome>> {
    let ea = u64::from(payload(probe)[11]);
    let mut s = PpuStateSnapshot::zero();
    s.gpr[30] = ea;
    let window = if ea == 0 {
        Vec::new()
    } else {
        (0x10..0x50).collect()
    };
    execute_state_in(word, &s, ea, window).map(|_| ())
}

#[test]
fn an_invalid_lmw_over_mapped_words_removes_the_child_and_cellgov_takes_the_illegal_fault() {
    let probe = "ppu_exception_seen_lmw_ra";
    assert_eq!(seen(probe), Seen::ThreadRemoved, "{probe}");
    assert_ne!(payload(probe)[11], 0, "{probe}: the access address");
    assert_eq!(
        cellgov(probe, LMW_R28_R30),
        Err(Box::new(CaseOutcome::UnexpectedVerdict(
            ExecuteVerdict::Fault(PpuFault::illegal("lmw"))
        ))),
        "{probe}"
    );
}

#[test]
fn an_invalid_lmw_or_lswi_over_unmapped_words_removes_the_child_and_cellgov_takes_the_data_storage_fault(
) {
    for (probe, word) in [
        ("ppu_exception_seen_lmw_ra_null", LMW_R28_R30),
        ("ppu_exception_seen_lswi_ra_null", LSWI_R28_R30_16),
    ] {
        assert_eq!(seen(probe), Seen::ThreadRemoved, "{probe}");
        assert_eq!(payload(probe)[11], 0, "{probe}: the access address");
        assert_eq!(
            cellgov(probe, word),
            Err(Box::new(CaseOutcome::UnexpectedVerdict(
                ExecuteVerdict::MemFault(MemError::Unmapped(FaultContext {
                    addr: 0,
                    nearest_below: None,
                    nearest_above: None,
                }))
            ))),
            "{probe}"
        );
    }
}
