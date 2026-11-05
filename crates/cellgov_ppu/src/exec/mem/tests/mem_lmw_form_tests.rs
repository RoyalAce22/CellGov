//! lmw invalid forms: RA=0 or RA inside the RT..=31 load range.

use super::*;

/// The word-aligned value rN holds before an `lmw`.
fn before(r: usize) -> u64 {
    0x100 + 4 * r as u64
}

/// Runs `lmw rt, 0(ra)` with rN = [`before`]`(N)` and every memory byte
/// 0xAA; returns the verdict, the state, and the effects.
fn lmw(rt: u8, ra: u8) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    for r in 1..32 {
        s.set_gpr(r, before(r));
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

/// [PPC-Book1 p:46 s:3.3.5] lmw is invalid when RA is 0 or in the load range.
/// [CBE-Handbook p:254 s:9.5.9] The PPE takes an illegal-instruction interrupt for it.
#[test]
fn lmw_with_ra_in_the_load_range_or_zero_faults_without_loading() {
    for (rt, ra) in [(20, 26), (20, 20), (20, 31), (3, 0), (31, 31)] {
        let (v, s, effects) = lmw(rt, ra);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::illegal("lmw")),
            "rt={rt} ra={ra}"
        );
        for r in 1..32 {
            assert_eq!(s.gpr[r], before(r), "rt={rt} ra={ra} r{r}");
        }
        assert!(effects.is_empty(), "rt={rt} ra={ra}: {effects:?}");
    }
}

#[test]
fn an_invalid_lmw_encoding_faults_only_as_illegal_and_a_valid_one_only_on_alignment() {
    use crate::instruction::fuzz::PpuOutcomeClass;
    use crate::PpuFaultClass as C;
    let descriptor = |rt: u32, ra: u32| {
        let raw = (46 << 26) | (rt << 21) | (ra << 16);
        crate::decode::decode(raw).unwrap().fuzz_descriptor(raw)
    };
    for (rt, ra) in [(20, 26), (20, 0)] {
        let invalid = descriptor(rt, ra);
        assert_eq!(
            invalid.outcomes,
            &[PpuOutcomeClass::Fault, PpuOutcomeClass::MemoryFault],
            "rt={rt} ra={ra}"
        );
        assert_eq!(
            invalid.faults,
            &[C::ProgramIllegal, C::Alignment, C::DataStorage],
            "rt={rt} ra={ra}"
        );
        let raw = (46 << 26) | (rt << 21) | (ra << 16);
        assert_eq!(
            crate::instruction::fuzz::generation_descriptor(raw)
                .unwrap()
                .sequence_flow,
            crate::instruction::fuzz::PpuSequenceFlow::Terminal,
            "rt={rt} ra={ra}"
        );
    }
    let valid = descriptor(20, 19);
    assert!(valid.outcomes.contains(&PpuOutcomeClass::Continue));
    assert_eq!(valid.faults, &[C::Alignment, C::DataStorage]);
}

#[test]
fn lmw_with_ra_just_below_the_load_range_loads() {
    let (v, s, _) = lmw(20, 19);
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[20], 0xAAAA_AAAA);
    assert_eq!(s.gpr[31], 0xAAAA_AAAA);
    assert_eq!(s.gpr[19], before(19));
}
