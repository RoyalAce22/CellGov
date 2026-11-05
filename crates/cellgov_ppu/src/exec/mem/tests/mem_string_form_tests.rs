//! lswi and lswx invalid forms: RA or RB inside the load range, or RT
//! equal to RA or RB.

use super::*;

/// Runs `insn` with rN = `0x10 + N`, XER byte count `n` and every memory
/// byte 0xAA; returns the verdict, the state, and the effects.
fn run(insn: PpuInstruction, n: u64) -> (ExecuteVerdict, PpuState, Vec<Effect>) {
    let mut s = PpuState::new();
    for r in 0..32 {
        s.set_gpr(r, 0x10 + r as u64);
    }
    s.set_xer(n);
    let mut effects = Vec::new();
    let v = exec_with_mem(&insn, &mut s, 0, &[0xAAu8; 0x100], &mut effects);
    (v, s, effects)
}

/// [PPC-Book1 p:48 s:3.3.6] RA in the range, RA=0 included, is invalid for both; so is RB in the range, or RT=RA or RT=RB, for lswx.
/// [CBE-Handbook p:254 s:9.5.9] The PPE takes an illegal-instruction interrupt for it.
#[test]
fn a_load_string_with_an_address_register_in_range_faults_without_loading() {
    let lswi = |rt, ra, nb| (PpuInstruction::Lswi { rt, ra, nb }, 0, "lswi");
    let lswx = |rt, ra, rb, n| (PpuInstruction::Lswx { rt, ra, rb }, n, "lswx");
    for (insn, n, name) in [
        lswi(4, 5, 8),
        lswi(30, 0, 12),
        lswi(30, 5, 0),
        lswx(4, 5, 6, 8),
        lswx(4, 0, 5, 8),
        lswx(31, 0, 7, 8),
        lswx(4, 4, 6, 0),
        lswx(4, 6, 4, 0),
    ] {
        let (v, s, effects) = run(insn, n);
        assert_eq!(
            v,
            ExecuteVerdict::Fault(PpuFault::illegal(name)),
            "{insn:?}"
        );
        for r in 0..32 {
            assert_eq!(s.gpr[r], 0x10 + r as u64, "{insn:?} r{r}");
        }
        assert!(effects.is_empty(), "{insn:?}: {effects:?}");
    }
}

#[test]
fn a_load_string_with_its_address_registers_outside_the_range_loads() {
    for (insn, n) in [
        (
            PpuInstruction::Lswi {
                rt: 4,
                ra: 6,
                nb: 8,
            },
            0,
        ),
        (
            PpuInstruction::Lswi {
                rt: 4,
                ra: 3,
                nb: 8,
            },
            0,
        ),
        (
            PpuInstruction::Lswx {
                rt: 4,
                ra: 0,
                rb: 6,
            },
            8,
        ),
        (
            PpuInstruction::Lswx {
                rt: 4,
                ra: 3,
                rb: 6,
            },
            8,
        ),
    ] {
        let (v, s, _) = run(insn, n);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
        assert_eq!(s.gpr[4], 0xAAAA_AAAA, "{insn:?}");
        assert_eq!(s.gpr[5], 0xAAAA_AAAA, "{insn:?}");
    }
}

#[test]
fn only_an_invalid_load_string_encoding_admits_a_fault() {
    use crate::instruction::fuzz::PpuOutcomeClass;
    let outcomes = |xo: u32, rt: u32, ra: u32, rb: u32| {
        let raw = (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (xo << 1);
        crate::decode::decode(raw)
            .unwrap()
            .fuzz_descriptor(raw)
            .outcomes
    };
    // lswi: NB from the word decides the range.
    assert_eq!(outcomes(597, 4, 5, 8), &[PpuOutcomeClass::Fault]);
    assert!(!outcomes(597, 4, 6, 8).contains(&PpuOutcomeClass::Fault));
    assert!(!outcomes(597, 4, 0, 8).contains(&PpuOutcomeClass::Fault));
    // lswx: RT=RA always faults; the XER count decides the rest.
    assert_eq!(outcomes(533, 4, 4, 6), &[PpuOutcomeClass::Fault]);
    let open = outcomes(533, 4, 5, 6);
    assert!(open.contains(&PpuOutcomeClass::Fault), "{open:?}");
    assert!(open.contains(&PpuOutcomeClass::Continue), "{open:?}");
}
