//! Classifier for instructions whose outcome the case snapshot
//! cannot faithfully reproduce.

/// Whether an instruction's outcome depends on PPU thread state
/// the [`InstructionCase`](super::InstructionCase) snapshot does not
/// faithfully reproduce.
///
/// Returns `true` for:
///
/// - Atomic load-reserve / store-conditional (`lwarx`, `ldarx`,
///   `stwcx`, `stdcx`). The
///   [`PpuStateSnapshot::reservation`](super::PpuStateSnapshot::reservation)
///   field carries the line address but not the full reservation
///   context (RPCS3 also tracks `rtime` and cross-thread
///   invalidation state).
/// - Time-base reads (`mftb`, `mftbu`). CellGov advances TB per
///   retired instruction; RPCS3 tracks wall clock.
pub fn is_context_dependent(raw: u32) -> bool {
    let primary = (raw >> 26) & 0x3f;
    if primary != 31 {
        return false;
    }
    let xo_10 = (raw >> 1) & 0x3ff;
    match xo_10 {
        // [PPC-Book2 p:24 s:3.3] lwarx / ldarx: set reservation;
        // their effect on PPU state is local but the matching
        // stwcx / stdcx's success depends on cross-thread
        // invalidation state.
        20 | 84 => true, // lwarx / ldarx
        // [PPC-Book2 p:25 s:3.3.2] stwcx / stdcx: success bit
        // depends on full reservation context.
        150 | 214 => true, // stwcx. / stdcx.
        // [PPC-Book2 p:30 s:6.2] mftb / mftbu: returns the TB
        // register, which the two emulators model differently.
        371 => {
            // mftb (TBR=268) and mftbu (TBR=269) live under the
            // same XO; both are context-dependent.
            let ra = ((raw >> 16) & 0x1f) as u16;
            let rb = ((raw >> 11) & 0x1f) as u16;
            let tbr = (rb << 5) | ra;
            tbr == 268 || tbr == 269
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "tests/context_tests.rs"]
mod tests;
