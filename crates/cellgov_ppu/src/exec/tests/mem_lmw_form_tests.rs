//! lmw invalid forms: RA=0 or RA inside the RT..=31 load range.

use super::*;

/// Runs `lmw rt, 0(ra)` with rN = `0x100 + N` and every memory byte
/// 0xAA; returns the verdict, the state, and the effects.
fn lmw(rt: u8, ra: u8) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    for r in 1..32 {
        s.set_gpr(r, 0x100 + r as u64);
    }
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lmw { rt, ra, imm: 0 },
        &mut s,
        0,
        &[0xAAu8; 0x400],
        &mut effects,
    );
    (v, s, effects)
}

// [PPC-Book1 p:46 s:3.3.5] lmw is invalid when RA is 0 or in the load range.
// [CBE-Handbook p:254 s:9.5.9] The PPE takes an illegal-instruction interrupt for it.
#[test]
fn lmw_with_ra_in_the_load_range_or_zero_faults_without_loading() {
    for (rt, ra) in [(20, 26), (20, 20), (20, 31), (3, 0), (31, 31)] {
        let (v, s, effects) = lmw(rt, ra);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::InvalidForm("lmw")),
            "rt={rt} ra={ra}"
        );
        for r in 1..32 {
            assert_eq!(s.gpr[r], 0x100 + r as u64, "rt={rt} ra={ra} r{r}");
        }
        assert!(effects.is_empty(), "rt={rt} ra={ra}: {effects:?}");
    }
}

#[test]
fn only_an_invalid_lmw_encoding_admits_a_fault() {
    use crate::instruction::fuzz::PpuOutcomeClass;
    let outcomes = |rt: u32, ra: u32| {
        let raw = (46 << 26) | (rt << 21) | (ra << 16);
        crate::decode::decode(raw)
            .unwrap()
            .fuzz_descriptor(raw)
            .outcomes
    };
    assert_eq!(outcomes(20, 26), &[PpuOutcomeClass::Fault]);
    assert_eq!(outcomes(20, 0), &[PpuOutcomeClass::Fault]);
    assert!(!outcomes(20, 19).contains(&PpuOutcomeClass::Fault));
}

#[test]
fn lmw_with_ra_just_below_the_load_range_loads() {
    let (v, s, _) = lmw(20, 19);
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[20], 0xAAAA_AAAA);
    assert_eq!(s.gpr[31], 0xAAAA_AAAA);
    assert_eq!(s.gpr[19], 0x100 + 19);
}
