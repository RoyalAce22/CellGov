//! The store pairs at the end of the region: each refuses inside its
//! second half and still meets the fused-pair property.

use super::proptests::check_pair;
use crate::exec::ExecuteVerdict;
use crate::instruction::PpuInstruction;
use crate::shadow::semantics_support::{StateSeed, MEM_BASE, MEM_LEN};

/// The distance from `r19` to the region's end.
const TO_END: i16 = 192;

/// Every GPR at the region's base, and `r19` [`TO_END`] bytes short of
/// its end.
fn region_end_seed() -> StateSeed {
    let mut gpr = [MEM_BASE; 32];
    gpr[19] = MEM_BASE + MEM_LEN as u64 - TO_END as u64;
    StateSeed::with_gprs(gpr)
}

#[test]
fn each_store_pair_refused_in_its_second_half_meets_its_halves() {
    let seed = region_end_seed();
    for (kind, a, b) in [
        // The second doubleword runs four bytes past the region.
        (
            "std; std",
            PpuInstruction::Std {
                rs: 0,
                ra: 19,
                imm: TO_END - 12,
            },
            PpuInstruction::Std {
                rs: 0,
                ra: 19,
                imm: TO_END - 4,
            },
        ),
        (
            "mflr; std",
            PpuInstruction::Mflr { rt: 0 },
            PpuInstruction::Std {
                rs: 0,
                ra: 19,
                imm: TO_END,
            },
        ),
        (
            "std; lwz",
            PpuInstruction::Std {
                rs: 0,
                ra: 1,
                imm: 0,
            },
            PpuInstruction::Lwz {
                rt: 3,
                ra: 19,
                imm: TO_END,
            },
        ),
    ] {
        let verdict = check_pair(a, b, &seed, None).unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert!(
            !matches!(verdict, ExecuteVerdict::Continue | ExecuteVerdict::Branch),
            "precondition: {kind} refuses, got {verdict:?}"
        );
    }
}
